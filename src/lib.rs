// SPDX-License-Identifier: Apache-2.0
//! SigmaOS-boot-manager — systemd-boot/GRUB-style boot entry manager.
//!
//! Models boot entries (title, EFI/linux path, initrd, kernel cmdline,
//! OS type, options), default selection, boot counting for A/B updates,
//! and EFI variable persistence. A real backend writes loader entries;
//! this is the tested entry/policy core.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OsKind { Linux, SigmaOS, Recovery, Windows, Other }
impl OsKind {
    pub fn label(self) -> &'static str {
        match self { Self::Linux=>"linux", Self::SigmaOS=>"sigmaos", Self::Recovery=>"recovery", Self::Windows=>"windows", Self::Other=>"other" }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootEntry {
    pub id: String,
    pub title: String,
    pub os: OsKind,
    pub kernel: String,            // path to kernel image (vmlinuz / bzImage)
    pub initrd: Option<String>,
    pub cmdline: String,           // kernel command line
    pub efi_path: Option<String>,  // .efi for chainload entries
    pub sort_order: i32,
    pub hidden: bool,
    pub boot_count: u32,           // tries left (boot counting)
    pub boot_success: Option<bool>,// last boot result (EFI var)
}

impl BootEntry {
    pub fn linux(id: impl Into<String>, title: impl Into<String>, kernel: impl Into<String>) -> Self {
        Self {
            id: id.into(), title: title.into(), os: OsKind::Linux,
            kernel: kernel.into(), initrd: None, cmdline: String::new(),
            efi_path: None, sort_order: 0, hidden: false,
            boot_count: 0, boot_success: None,
        }
    }

    pub fn sigmaos(id: impl Into<String>, title: impl Into<String>, kernel: impl Into<String>) -> Self {
        let mut e = Self::linux(id, title, kernel);
        e.os = OsKind::SigmaOS;
        e
    }

    pub fn recovery(id: impl Into<String>, title: impl Into<String>, efi: impl Into<String>) -> Self {
        let mut e = Self::linux(id, title, "");
        e.os = OsKind::Recovery;
        e.efi_path = Some(efi.into());
        e
    }

    pub fn with_cmdline(mut self, cmd: impl Into<String>) -> Self { self.cmdline = cmd.into(); self }
    pub fn with_initrd(mut self, i: impl Into<String>) -> Self { self.initrd = Some(i.into()); self }
    pub fn with_order(mut self, o: i32) -> Self { self.sort_order = o; self }
    pub fn with_boot_count(mut self, n: u32) -> Self { self.boot_count = n; self }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootError {
    NoSuchEntry(String),
    DuplicateId(String),
    NoBootableEntry,
    BadCmdline,
}
impl fmt::Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchEntry(id) => write!(f, "no boot entry {id}"),
            Self::DuplicateId(id) => write!(f, "duplicate entry id {id}"),
            Self::NoBootableEntry => write!(f, "no bootable entries"),
            Self::BadCmdline => write!(f, "invalid kernel cmdline"),
        }
    }
}
impl std::error::Error for BootError {}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BootConfig {
    pub timeout_secs: u32,          // menu timeout (0 = no menu)
    pub default_entry: Option<String>,
    pub console_mode: String,       // "gfxterm", "text"
    pub theme: String,
}
impl Default for BootConfig {
    fn default() -> Self {
        Self { timeout_secs: 5, default_entry: None, console_mode: "gfxterm".into(), theme: "sigma".into() }
    }
}

pub struct SigmaOSBootManager {
    entries: BTreeMap<String, BootEntry>,
    cfg: BootConfig,
    booted_entry: Option<String>,   // last booted (from EFI var)
}

impl SigmaOSBootManager {
    pub fn new() -> Self {
        Self { entries: BTreeMap::new(), cfg: BootConfig::default(), booted_entry: None }
    }

    pub fn config(&self) -> &BootConfig { &self.cfg }
    pub fn config_mut(&mut self) -> &mut BootConfig { &mut self.cfg }
    pub fn booted_entry(&self) -> Option<&str> { self.booted_entry.as_deref() }

    pub fn add(&mut self, e: BootEntry) -> Result<(), BootError> {
        if self.entries.contains_key(&e.id) { return Err(BootError::DuplicateId(e.id.clone())); }
        self.entries.insert(e.id.clone(), e);
        if self.cfg.default_entry.is_none() {
            self.cfg.default_entry = self.entries.keys().next().cloned();
        }
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<BootEntry, BootError> {
        let e = self.entries.remove(id).ok_or_else(|| BootError::NoSuchEntry(id.into()))?;
        if self.cfg.default_entry.as_deref() == Some(id) {
            self.cfg.default_entry = self.entries.keys().next().cloned();
        }
        Ok(e)
    }

    pub fn get(&self, id: &str) -> Option<&BootEntry> { self.entries.get(id) }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut BootEntry> { self.entries.get_mut(id) }
    pub fn entries(&self) -> impl Iterator<Item = &BootEntry> {
        let mut v: Vec<&BootEntry> = self.entries.values().filter(|e| !e.hidden).collect();
        v.sort_by_key(|e| e.sort_order);
        v.into_iter()
    }
    pub fn all_entries(&self) -> impl Iterator<Item = &BootEntry> { self.entries.values() }
    pub fn count(&self) -> usize { self.entries.len() }

    pub fn set_default(&mut self, id: &str) -> Result<(), BootError> {
        if !self.entries.contains_key(id) { return Err(BootError::NoSuchEntry(id.into())); }
        self.cfg.default_entry = Some(id.into()); Ok(())
    }

    /// Boot counting: decrement tries; if exhausted, mark failed.
    pub fn boot_attempt(&mut self, id: &str) -> Result<u32, BootError> {
        let e = self.entries.get_mut(id).ok_or_else(|| BootError::NoSuchEntry(id.into()))?;
        if e.boot_count > 0 {
            e.boot_count -= 1;
            if e.boot_count == 0 { e.boot_success = Some(false); }
        }
        self.booted_entry = Some(id.into());
        Ok(e.boot_count)
    }

    /// Called from userspace on successful boot (systemd-boot service).
    pub fn mark_boot_success(&mut self, id: &str) -> Result<(), BootError> {
        let e = self.entries.get_mut(id).ok_or_else(|| BootError::NoSuchEntry(id.into()))?;
        e.boot_success = Some(true);
        e.boot_count = 0;
        Ok(())
    }

    /// Entries with boot counting enabled and tries remaining.
    pub fn counting_entries(&self) -> Vec<&BootEntry> {
        self.entries.values().filter(|e| e.boot_count > 0).collect()
    }

    /// Resolve which entry boots next: default (if visible) > first
    /// non-hidden entry by sort order.
    pub fn next_boot(&self) -> Result<&BootEntry, BootError> {
        if let Some(id) = self.cfg.default_entry.as_deref() {
            if let Some(e) = self.entries.get(id) {
                if !e.hidden { return Ok(e); }
            }
        }
        self.entries.values()
            .filter(|e| !e.hidden)
            .min_by_key(|e| e.sort_order)
            .ok_or(BootError::NoBootableEntry)
    }

    /// Validate an entry can actually boot (has kernel or efi).
    pub fn is_bootable(&self, id: &str) -> bool {
        self.entries.get(id).map(|e| !e.kernel.is_empty() || e.efi_path.is_some()).unwrap_or(false)
    }

    /// Render loader entries.conf-style config text.
    pub fn render_entries(&self) -> String {
        let mut out = String::new();
        for e in self.entries.values() {
            if e.hidden { continue; }
            out.push_str(&format!("title {}\n", e.title));
            out.push_str(&format!("  os {}\n", e.os.label()));
            if !e.kernel.is_empty() { out.push_str(&format!("  linux {}\n", e.kernel)); }
            if let Some(i) = &e.initrd { out.push_str(&format!("  initrd {}\n", i)); }
            if !e.cmdline.is_empty() { out.push_str(&format!("  options {}\n", e.cmdline)); }
            if let Some(efi) = &e.efi_path { out.push_str(&format!("  efi {}\n", efi)); }
            if e.boot_count > 0 { out.push_str(&format!("  tries {}\n", e.boot_count)); }
            out.push('\n');
        }
        out
    }

    /// Persist entries + config + last-booted to JSON (models EFI vars).
    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Disk { entries: Vec<BootEntry>, cfg: BootConfig, booted: Option<String> }
        let d = Disk { entries: self.entries.values().cloned().collect(),
                       cfg: self.cfg.clone(), booted: self.booted_entry.clone() };
        let j = serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?;
        if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        std::fs::write(path, j).map_err(|e| e.to_string())
    }

    pub fn load_from(path: &std::path::Path) -> Result<Self, String> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Disk { entries: Vec<BootEntry>, cfg: BootConfig, booted: Option<String> }
        let txt = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let d: Disk = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let mut m = Self { entries: BTreeMap::new(), cfg: d.cfg, booted_entry: d.booted };
        for e in d.entries { m.entries.insert(e.id.clone(), e); }
        Ok(m)
    }

    pub fn state_path() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("SIGMAOS_BOOT_FILE") { return std::path::PathBuf::from(p); }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        std::path::PathBuf::from(home).join(".config/sigmaos/boot.json")
    }

    /// Seed sensible defaults for a SigmaOS install.
    pub fn seeded() -> Self {
        let mut m = Self::new();
        let _ = m.add(BootEntry::sigmaos("sigmaos", "SigmaOS", "/boot/vmlinuz-sigmaos")
            .with_cmdline("root=/dev/mapper/sigmaos-root rw quiet")
            .with_initrd("/boot/initrd-sigmaos.img")
            .with_order(0));
        let _ = m.add(BootEntry::sigmaos("sigmaos-fallback", "SigmaOS (fallback)", "/boot/vmlinuz-sigmaos-lts")
            .with_cmdline("root=/dev/mapper/sigmaos-root rw nomodeset")
            .with_initrd("/boot/initrd-sigmaos.img")
            .with_order(1)
            .with_boot_count(3));
        let _ = m.add(BootEntry::recovery("recovery", "SigmaOS Recovery", "/EFI/sigmaos/recovery.efi")
            .with_order(90));
        m
    }

    pub fn status_line(&self) -> String {
        let next = self.next_boot().map(|e| e.title.clone()).unwrap_or_else(|_| "<none>".into());
        format!("boot: {} entries, default='{}' (next: {}), timeout {}s",
            self.count(), self.cfg.default_entry.as_deref().unwrap_or("<unset>"), next, self.cfg.timeout_secs)
    }
}

impl Default for SigmaOSBootManager {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, kernel: &str) -> BootEntry {
        BootEntry::sigmaos(id, format!("SigmaOS {id}"), kernel)
            .with_cmdline("root=/dev/sda1 rw")
            .with_initrd("/boot/initrd.img")
    }

    #[test]
    fn add_remove_default() {
        let mut m = SigmaOSBootManager::new();
        m.add(entry("sigmaos", "/boot/vmlinuz-sigmaos")).unwrap();
        m.add(entry("linux", "/boot/vmlinuz-linux")).unwrap();
        assert_eq!(m.count(), 2);
        assert_eq!(m.config().default_entry.as_deref(), Some("sigmaos")); // first added becomes default
        m.set_default("sigmaos").unwrap();
        assert_eq!(m.next_boot().unwrap().id, "sigmaos");
        m.remove("sigmaos").unwrap();
        assert_eq!(m.next_boot().unwrap().id, "linux");
        assert!(m.remove("nope").is_err());
    }

    #[test]
    fn duplicate_rejected() {
        let mut m = SigmaOSBootManager::new();
        m.add(entry("a", "/k")).unwrap();
        assert!(matches!(m.add(entry("a", "/k2")), Err(BootError::DuplicateId(_))));
    }

    #[test]
    fn boot_counting() {
        let mut m = SigmaOSBootManager::new();
        m.add(entry("sigmaos", "/k").with_boot_count(3)).unwrap();
        assert_eq!(m.boot_attempt("sigmaos").unwrap(), 2);
        assert_eq!(m.boot_attempt("sigmaos").unwrap(), 1);
        assert_eq!(m.boot_attempt("sigmaos").unwrap(), 0);
        let e = m.get("sigmaos").unwrap();
        assert_eq!(e.boot_success, Some(false)); // exhausted = failed
        m.mark_boot_success("sigmaos").unwrap();
        assert_eq!(m.get("sigmaos").unwrap().boot_success, Some(true));
        assert_eq!(m.get("sigmaos").unwrap().boot_count, 0);
    }

    #[test]
    fn hidden_and_order() {
        let mut m = SigmaOSBootManager::new();
        let mut a = entry("a", "/ka"); a.hidden = true;
        m.add(a).unwrap();
        m.add(entry("b", "/kb").with_order(5)).unwrap();
        m.add(entry("c", "/kc").with_order(1)).unwrap();
        let visible: Vec<&str> = m.entries().map(|e| e.id.as_str()).collect();
        assert_eq!(visible, vec!["c", "b"]); // sorted by order, hidden excluded
        // default should skip hidden → first by order
        assert_eq!(m.next_boot().unwrap().id, "c");
    }

    #[test]
    fn bootable_check_and_recovery() {
        let mut m = SigmaOSBootManager::new();
        m.add(BootEntry::recovery("rec", "Recovery", "/EFI/BOOT/BOOTX64.EFI")).unwrap();
        assert!(m.is_bootable("rec"));
        assert_eq!(m.get("rec").unwrap().os, OsKind::Recovery);
        let mut bad = BootEntry::linux("bad", "Bad", "");
        bad.initrd = None;
        m.add(bad).unwrap();
        assert!(!m.is_bootable("bad"));
    }

    #[test]
    fn render_config() {
        let mut m = SigmaOSBootManager::new();
        m.add(entry("sigmaos", "/boot/vmlinuz")).unwrap();
        let cfg = m.render_entries();
        assert!(cfg.contains("title SigmaOS sigmaos"));
        assert!(cfg.contains("linux /boot/vmlinuz"));
        assert!(cfg.contains("options root=/dev/sda1 rw"));
        assert!(cfg.contains("initrd /boot/initrd.img"));
    }

    #[test]
    fn status_and_empty() {
        let m = SigmaOSBootManager::new();
        assert!(m.next_boot().is_err());
        assert!(m.status_line().contains("0 entries"));
    }
}
