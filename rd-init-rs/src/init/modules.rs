//! Kernel module loading stage.

use anyhow::Result;
use std::fs;
use std::path::Path;

use super::Config;

impl Config {
    pub(crate) fn init_modprobe(&self) -> Result<()> {
        self.log(&format!("loading drivers: {}", self.drivers.join(",")));
        for module in &self.drivers {
            self.run_command("/sbin/modprobe", &[module]).ok();
        }
        if Path::new("/etc/modules.preload").exists() {
            let contents = fs::read_to_string("/etc/modules.preload")?;
            for module in contents.lines().map(str::trim).filter(|l| !l.is_empty()) {
                self.run_command("/sbin/modprobe", &[module]).ok();
            }
        }
        self.enter_stage("pci")?;
        if Path::new("/sys/bus/pci/devices").exists() {
            for entry in fs::read_dir("/sys/bus/pci/devices")? {
                let entry = entry?;
                let modalias = entry.path().join("modalias");
                if modalias.exists() {
                    let alias = fs::read_to_string(modalias)?.trim().to_string();
                    self.run_command("/sbin/modprobe", &[&alias]).ok();
                }
            }
        }
        Ok(())
    }

    pub(crate) fn source_load_modules(&self) -> Result<()> {
        let contents = fs::read_to_string("/load-modules")?;
        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with("modprobe") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 1 {
                    self.run_command("/sbin/modprobe", &[parts[1]]).ok();
                }
            }
        }
        Ok(())
    }
}
