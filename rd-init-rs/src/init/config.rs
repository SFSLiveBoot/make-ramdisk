//! Kernel command line and environment based configuration.

use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::io::Read;
use std::os::unix::io::IntoRawFd;

use super::Config;

/// Default kernel drivers to load automatically.
pub(crate) const DEFAULT_DRIVERS: &[&str] = &[
    "sd-mod",
    "sr-mod",
    "ide-disk",
    "ide-cd",
    "usb-storage",
    "uas",
    "dm-mod",
    "usbhid",
    "hid-generic",
    "virtio-blk",
    "virtio-net",
    "rtsx-pci-sdmmc",
    "mmc-block",
    "sdhci-acpi",
    "virtio-pci",
];

impl Config {
    pub fn load_cmdline(&mut self, path: &str) -> Result<()> {
        let mut contents = String::new();
        fs::File::open(path)
            .with_context(|| format!("opening cmdline file {}", path))?
            .read_to_string(&mut contents)
            .with_context(|| format!("reading cmdline file {}", path))?;
        for token in contents.split_whitespace() {
            self.parse_cmdline_token(token);
        }
        Ok(())
    }

    fn parse_cmdline_token(&mut self, token: &str) {
        match token {
            "no_volid" => self.no_volid = true,
            "quiet" => {
                self.quiet = true;
                self.qswitch = "-q";
            }
            "verbose" => {
                self.qswitch = "-v";
                self.verbose = true;
            }
            "break" => self.brk = Some("finalize".to_string()),
            "debug" => self.debug = true,
            _ if token.starts_with("stderr=") => {
                let file = &token[7..];
                let _ = self.redirect_fd(2, file);
            }
            _ if token.starts_with("stdout=") => {
                let file = &token[7..];
                let _ = self.redirect_fd(1, file);
            }
            _ if token.starts_with("quiet=") => {
                self.quiet = token[6..].to_string() != "0";
                if self.quiet {
                    self.qswitch = "-q";
                }
            }
            _ if token.starts_with("verbose=") => {
                self.verbose = true;
                self.qswitch = "-v";
            }
            _ if token.starts_with("break=") => {
                self.brk = Some(token[6..].to_string())
            }
            _ if token.contains('=') => {
                let mut split = token.splitn(2, '=');
                if let (Some(name), Some(value)) = (split.next(), split.next()) {
                    if !name.contains('.') {
                        match name {
                            "root" => self.root = Some(value.to_string()),
                            "ip" => self.ip = Some(value.to_string()),
                            "wpa_conf" => self.wpa_conf = Some(value.to_string()),
                            "prepdev" => self.prepdev = Some(value.to_string()),
                            "sshkey" => self.sshkey = Some(value.to_string()),
                            "sshkey_file" => self.sshkey_file = Some(value.to_string()),
                            "root_backend" => self.root_backend = Some(value.to_string()),
                            "running_config" => self.running_config = value.to_string(),
                            "dev_console" => self.dev_console = value.to_string(),
                            _ => { env::set_var(name, value); }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn redirect_fd(&self, fd: i32, path: &str) -> Result<()> {
        let file = fs::OpenOptions::new().create(true).append(true).open(path)?;
        let raw = file.into_raw_fd();
        nix::unistd::dup2(raw, fd)?;
        Ok(())
    }
}
