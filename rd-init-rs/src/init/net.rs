//! Network configuration stage.

use anyhow::Result;
use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

use super::Config;

impl Config {
    pub(crate) fn init_net(&self, ip: &str) -> Result<()> {
        self.log("net");
        self.ip_link_up("lo")?;
        match ip {
            s if s.contains(":dhcp") => {
                let iface = s.trim_end_matches(":dhcp");
                self.wait_ifup(iface)?;
                self.get_dhcp_lease(Some(iface))?;
            }
            "dhcp" => {
                let iface = self.wait_ifup("all")?;
                self.log("DHCP..");
                self.get_dhcp_lease(Some(&iface))?;
            }
            _ => {
                self.run_command("ipconfig", &["-o", "-d", ip])?;
            }
        }
        Ok(())
    }

    fn ip_link_up(&self, iface: &str) -> Result<()> {
        if self.executable_exists("ip") {
            self.run_command("ip", &["link", "set", "up", "dev", iface])
        } else {
            self.run_command("ipconfig", &["-d", "169.254.1.1:::255.255.0.0:", iface, "off"])
        }
    }

    fn wait_ifup(&self, name: &str) -> Result<String> {
        let start = Instant::now();
        let timeout = Duration::from_secs(10);
        let ifaces = self.find_ifaces(name);
        if ifaces.is_empty() {
            return Ok(String::new());
        }
        println!("Waiting for {} to come up..", ifaces.join(" "));
        for iface in &ifaces {
            let _ = self.ip_link_up(iface);
        }
        loop {
            for iface in &ifaces {
                if self.is_iface_up(iface) {
                    println!("Ok ({})", iface);
                    return Ok(iface.to_string());
                }
            }
            if start.elapsed() > timeout {
                anyhow::bail!("No interface ({}) become up in {} seconds", ifaces.join(" "), timeout.as_secs());
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    fn get_dhcp_lease(&self, iface: Option<&str>) -> Result<()> {
        if self.executable_exists("udhcpc") {
            let mut args = vec!["-n", "-q"];
            if let Some(iface) = iface {
                args.push("-i");
                args.push(iface);
            }
            self.run_command("udhcpc", &args)
        } else {
            let mut args = vec!["-o"];
            if let Some(iface) = iface {
                args.push(iface);
            }
            self.run_command("ipconfig", &args)
        }
    }

    fn find_ifaces(&self, name: &str) -> Vec<String> {
        let mut result = Vec::new();
        if let Ok(entries) = fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                let iface = entry.file_name().to_string_lossy().into_owned();
                if iface == "lo" {
                    continue;
                }
                if name == "all" || iface == name {
                    result.push(iface);
                }
            }
        }
        result
    }

    fn is_iface_up(&self, iface: &str) -> bool {
        if self.executable_exists("ip") {
            if let Ok(output) = Command::new("ip").args(["link", "show", "dev", iface]).output() {
                if let Ok(text) = String::from_utf8(output.stdout) {
                    return text.contains("UP") && !text.contains("NO-CARRIER");
                }
            }
            false
        } else {
            let file = format!("/sys/class/net/{}/operstate", iface);
            if let Ok(contents) = fs::read_to_string(file) {
                return contents.trim() == "up";
            }
            false
        }
    }
}
