//! Storage discovery and preparation: partitions, filesystems, LVM.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

use super::Config;

impl Config {
    pub(crate) fn refresh_lvm(&self) -> Result<()> {
        let _ = fs::remove_file("/etc/lvm/cache/.cache");
        self.run_command("lvm", &["vgscan", "--mknodes"]).ok();
        self.run_command("lvm", &["vgchange", "-a", "y"]).ok();
        Ok(())
    }

    pub(crate) fn list_partitions(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let contents = fs::read_to_string("/proc/partitions")?;
        for line in contents.lines().skip(2) {
            let mut parts = line.split_whitespace();
            if let (Some(_major), Some(_minor), Some(_blocks), Some(name)) = (parts.next(), parts.next(), parts.next(), parts.next()) {
                let device = format!("/dev/{}", name);
                if Path::new(&device).exists() {
                    out.push(device);
                }
            }
        }
        Ok(out)
    }

    pub(crate) fn fs_type(&self, device: &str) -> Result<String> {
        let output = Command::new("/sbin/blkid")
            .args(["-o", "value", "-s", "TYPE", device])
            .output()
            .with_context(|| format!("running blkid on {}", device))?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub(crate) fn prepare_dev(&mut self, dev: &str, _timeout: Option<u64>) -> Result<()> {
        let resolved = if dev.starts_with("LABEL=") {
            format!("/dev/disk/by-label/{}", &dev[6..])
        } else if dev.starts_with("UUID=") {
            format!("/dev/disk/by-uuid/{}", &dev[5..])
        } else {
            dev.to_string()
        };
        if !Path::new(&resolved).exists() {
            anyhow::bail!("device not found: {}", resolved);
        }
        self.prepared_device = Some(resolved);
        Ok(())
    }

    pub(crate) fn prepared_device(&self) -> Result<String> {
        self.prepared_device.clone().context("prepared device not available")
    }

    pub(crate) fn auto_root(&mut self) -> Result<()> {
        for dev in self.list_partitions()? {
            if self.fs_type(&dev)? == "crypto_LUKS" {
                if self.prepare_dev(&dev, Some(5)).is_ok() && self.prepare_dev("@ROOT", Some(5)).is_ok() {
                    self.root = Some(self.prepared_device()?);
                    return Ok(());
                }
            }
        }
        if self.prepare_dev("@ROOT", Some(5)).is_ok() {
            self.root = Some(self.prepared_device()?);
        }
        Ok(())
    }
}
