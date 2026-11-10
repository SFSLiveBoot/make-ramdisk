//! Wifi stage initialization.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use super::Config;

impl Config {
    pub(crate) fn init_wifi(&self) -> Result<()> {
        let wpa_conf = self.wpa_conf.as_ref().unwrap();
        if !Path::new(wpa_conf).exists() {
            return Ok(());
        }
        let iface = self.find_wifi_iface()?;
        self.log(&format!("running wpa_supplicant on {} using {}", iface, wpa_conf));
        self.run_command("wpa_supplicant", &["-B", "-Dnl80211", "-i", &iface, "-c", wpa_conf])
    }

    fn find_wifi_iface(&self) -> Result<String> {
        let mut result = None;
        for entry in fs::read_dir("/sys/class/net")? {
            let entry = entry?;
            let path = entry.path();
            if path.join("phy80211").exists() {
                result = Some(entry.file_name().to_string_lossy().into_owned());
                break;
            }
        }
        result.context("80211 interface not found")
    }
}
