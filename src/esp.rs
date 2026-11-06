//! ESP installation — write boot entries into a systemd-boot layout.
//!
//! Produces the on-disk structure the bootloader reads:
//! ```text
//! <esp>/loader/loader.conf          # default + timeout
//! <esp>/loader/entries/<slug>.conf  # one file per BootEntry
//! ```
//! Kernel/initrd paths inside entries are ESP-relative (`\EFI\...` or
//! `/efi/...` depending on firmware), matching systemd-boot syntax.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{BootEntry, BootManager};

/// Errors installing entries to disk — fail-fast, no partial silence.
#[derive(Debug)]
pub enum EspError {
    Io(io::Error),
    EmptyManager,
    /// Title slugified to empty (all symbols).
    UnspeakableTitle(String),
}

impl std::fmt::Display for EspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EspError::Io(e) => write!(f, "esp i/o error: {e}"),
            EspError::EmptyManager => write!(f, "no boot entries to install"),
            EspError::UnspeakableTitle(t) => write!(f, "title slugifies to empty: {t:?}"),
        }
    }
}
impl std::error::Error for EspError {}
impl From<io::Error> for EspError {
    fn from(e: io::Error) -> Self {
        EspError::Io(e)
    }
}

/// systemd-boot loader.conf knobs.
#[derive(Debug, Clone)]
pub struct LoaderConfig {
    /// Entry slug selected at boot (without `.conf`).
    pub default_slug: String,
    /// Menu timeout in seconds; 0 = boot default immediately.
    pub timeout: u32,
}

impl LoaderConfig {
    pub fn render(&self) -> String {
        format!("default {}\ntimeout {}\neditor no\n", self.default_slug, self.timeout)
    }
}

/// Slugify an entry title into a filename-safe identifier:
/// lowercase, non-alphanumeric runs → `-`, trimmed.
pub fn slugify(title: &str) -> Result<String, EspError> {
    let mut out = String::new();
    let mut dash = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return Err(EspError::UnspeakableTitle(title.to_string()));
    }
    Ok(out)
}

/// What was written — for callers that need paths back.
#[derive(Debug)]
pub struct Installed {
    pub loader_conf: PathBuf,
    pub entries_dir: PathBuf,
    pub entry_files: Vec<PathBuf>,
    pub default_slug: String,
}

/// Install all entries of `mgr` under `esp_root`.
///
/// `esp_root` is the mounted ESP (e.g. `/boot` or `/efi`). Entries are
/// rewritten each call — idempotent, matching `bootctl install` semantics.
pub fn install(mgr: &BootManager, esp_root: &Path, timeout: u32) -> Result<Installed, EspError> {
    let entries = mgr.entries();
    if entries.is_empty() {
        return Err(EspError::EmptyManager);
    }
    let default = mgr.default_entry().ok_or(EspError::EmptyManager)?;
    let default_slug = slugify(&default.title)?;

    let loader_dir = esp_root.join("loader");
    let entries_dir = loader_dir.join("entries");
    fs::create_dir_all(&entries_dir)?;

    // Remove stale .conf files so retired entries don't linger.
    for f in fs::read_dir(&entries_dir)? {
        let p = f?.path();
        if p.extension().map(|e| e == "conf").unwrap_or(false) {
            fs::remove_file(p)?;
        }
    }

    let mut entry_files = Vec::new();
    let mut used_slugs = std::collections::BTreeSet::new();
    for e in entries {
        let mut slug = slugify(&e.title)?;
        // Disambiguate duplicate titles: name, name-2, name-3...
        let mut n = 2;
        while !used_slugs.insert(slug.clone()) {
            slug = format!("{}-{}", slugify(&e.title)?, n);
            n += 1;
        }
        let path = entries_dir.join(format!("{slug}.conf"));
        fs::write(&path, e.to_conf())?;
        entry_files.push(path);
    }

    let conf = LoaderConfig { default_slug: default_slug.clone(), timeout };
    let loader_conf = loader_dir.join("loader.conf");
    fs::write(&loader_conf, conf.render())?;

    Ok(Installed { loader_conf, entries_dir, entry_files, default_slug })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BootEntry;

    fn mgr() -> BootManager {
        let mut m = BootManager::new();
        m.add(BootEntry::new(
            "SigmaOS",
            "/efi/sigmaos/vmlinuz",
            vec!["/efi/sigmaos/initramfs.img".to_string()],
            "root=/dev/sda1 rw",
        ).unwrap());
        m.add(BootEntry::new(
            "SigmaOS LTS",
            "/efi/sigmaos/vmlinuz-lts",
            vec![],
            "root=/dev/sda1 ro",
        ).unwrap());
        m.set_default(0).unwrap();
        m
    }

    #[test]
    fn slugify_titles() {
        assert_eq!(slugify("SigmaOS LTS").unwrap(), "sigmaos-lts");
        assert_eq!(slugify("  Weird--Title!! ").unwrap(), "weird-title");
        assert!(slugify("!!!").is_err());
    }

    #[test]
    fn install_writes_loader_conf_and_entries() {
        let dir = std::env::temp_dir().join(format!("esp-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let got = install(&mgr(), &dir, 5).unwrap();
        assert!(got.loader_conf.exists());
        assert_eq!(got.entry_files.len(), 2);
        let conf = fs::read_to_string(&got.loader_conf).unwrap();
        assert!(conf.contains("default sigmaos"));
        assert!(conf.contains("timeout 5"));
        let entry = fs::read_to_string(&got.entry_files[0]).unwrap();
        assert!(entry.contains("title SigmaOS"));
        assert!(entry.contains("initrd /efi/sigmaos/initramfs.img"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reinstall_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("esp-test2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        install(&mgr(), &dir, 3).unwrap();
        let got = install(&mgr(), &dir, 3).unwrap();
        assert_eq!(got.entry_files.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_manager_fails() {
        let dir = std::env::temp_dir().join(format!("esp-test3-{}", std::process::id()));
        assert!(matches!(install(&BootManager::new(), &dir, 0), Err(EspError::EmptyManager)));
    }
}
