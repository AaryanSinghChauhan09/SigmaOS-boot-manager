// SPDX-License-Identifier: Apache-2.0
//! SigmaOS-boot-manager — boot entry management, systemd-boot/GRUB-inspired.
//!
//! Models boot entries: each entry names a kernel, optional initrd(s), and
//! kernel command line. Validation is fail-fast — an entry without a kernel
//! or a `root=` parameter can't boot and is rejected.

pub mod esp;

use std::fmt;

/// Errors validating a boot entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootError {
    /// Entry title is empty.
    EmptyTitle,
    /// Kernel path missing or empty.
    MissingKernel,
    /// Kernel command line lacks a `root=` parameter.
    MissingRootParam,
    /// Default index out of range.
    InvalidIndex,
}

impl fmt::Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTitle => write!(f, "boot entry title is empty"),
            Self::MissingKernel => write!(f, "boot entry has no kernel"),
            Self::MissingRootParam => write!(f, "kernel cmdline lacks root= parameter"),
            Self::InvalidIndex => write!(f, "boot entry index out of range"),
        }
    }
}

impl std::error::Error for BootError {}

/// A single boot entry (one menu item).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootEntry {
    /// Display title shown in the boot menu.
    pub title: String,
    /// Path to the kernel image (e.g. `/vmlinuz-linux`).
    pub kernel: String,
    /// Initrd image paths, in load order.
    pub initrds: Vec<String>,
    /// Kernel command line options.
    pub options: String,
}

impl BootEntry {
    /// Create and validate an entry. Fails fast on unbootable configs.
    pub fn new(
        title: impl Into<String>,
        kernel: impl Into<String>,
        initrds: Vec<String>,
        options: impl Into<String>,
    ) -> Result<Self, BootError> {
        let entry = Self {
            title: title.into(),
            kernel: kernel.into(),
            initrds,
            options: options.into(),
        };
        entry.validate()?;
        Ok(entry)
    }

    /// Validate: non-empty title/kernel, `root=` present in options.
    pub fn validate(&self) -> Result<(), BootError> {
        if self.title.trim().is_empty() {
            return Err(BootError::EmptyTitle);
        }
        if self.kernel.trim().is_empty() {
            return Err(BootError::MissingKernel);
        }
        let has_root = self
            .options
            .split_whitespace()
            .any(|t| t.starts_with("root="));
        if !has_root {
            return Err(BootError::MissingRootParam);
        }
        Ok(())
    }

    /// Render as a systemd-boot-style `.conf` snippet.
    pub fn to_conf(&self) -> String {
        let mut s = format!("title {}\nlinux {}\n", self.title, self.kernel);
        for i in &self.initrds {
            s.push_str(&format!("initrd {i}\n"));
        }
        s.push_str(&format!("options {}\n", self.options));
        s
    }
}

/// A collection of boot entries with a default selection.
#[derive(Debug, Default)]
pub struct BootManager {
    entries: Vec<BootEntry>,
    default: Option<usize>,
}

impl BootManager {
    /// Empty manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a validated entry; returns its index.
    pub fn add(&mut self, entry: BootEntry) -> usize {
        self.entries.push(entry);
        self.entries.len() - 1
    }

    /// Set the default entry index; fails fast if out of range.
    pub fn set_default(&mut self, index: usize) -> Result<(), BootError> {
        if index >= self.entries.len() {
            return Err(BootError::InvalidIndex);
        }
        self.default = Some(index);
        Ok(())
    }

    /// The default entry, if set and valid.
    pub fn default_entry(&self) -> Option<&BootEntry> {
        self.default.and_then(|i| self.entries.get(i))
    }

    /// All entries.
    pub fn entries(&self) -> &[BootEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> BootEntry {
        BootEntry::new(
            "SigmaOS",
            "/vmlinuz-linux",
            vec!["/initramfs-linux.img".into()],
            "root=/dev/sda2 rw quiet",
        )
        .unwrap()
    }

    #[test]
    fn valid_entry_ok() {
        assert!(entry().validate().is_ok());
    }

    #[test]
    fn empty_title_fails() {
        assert_eq!(
            BootEntry::new(" ", "/vmlinuz", vec![], "root=/dev/sda2"),
            Err(BootError::EmptyTitle)
        );
    }

    #[test]
    fn missing_kernel_fails() {
        assert_eq!(
            BootEntry::new("t", " ", vec![], "root=/dev/sda2"),
            Err(BootError::MissingKernel)
        );
    }

    #[test]
    fn missing_root_param_fails() {
        assert_eq!(
            BootEntry::new("t", "/vmlinuz", vec![], "rw quiet"),
            Err(BootError::MissingRootParam)
        );
    }

    #[test]
    fn conf_render() {
        let e = entry();
        let conf = e.to_conf();
        assert!(conf.contains("title SigmaOS"));
        assert!(conf.contains("linux /vmlinuz-linux"));
        assert!(conf.contains("initrd /initramfs-linux.img"));
        assert!(conf.contains("options root=/dev/sda2 rw quiet"));
    }

    #[test]
    fn manager_default() {
        let mut m = BootManager::new();
        let i = m.add(entry());
        m.set_default(i).unwrap();
        assert_eq!(m.default_entry().unwrap().title, "SigmaOS");
    }
}
