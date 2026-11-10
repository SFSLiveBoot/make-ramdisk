//! Boot sequence orchestration for the ramdisk init.

mod commands;
mod config;
mod finalize;
mod loopdev;
mod mounts;
mod net;
mod modules;
mod ramfs;
mod storage;
mod udev;
mod wifi;

use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use self::config::DEFAULT_DRIVERS;

pub struct Config {
    quiet: bool,
    qswitch: &'static str,
    verbose: bool,
    brk: Option<String>,
    no_volid: bool,
    debug: bool,
    wpa_conf: Option<String>,
    ip: Option<String>,
    prepdev: Option<String>,
    sshkey: Option<String>,
    sshkey_file: Option<String>,
    root: Option<String>,
    root_backend: Option<String>,
    running_config: String,
    init_path: String,
    dev_console: String,
    drivers: Vec<String>,
    prepared_device: Option<String>,
    root_mounted: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            quiet: false,
            qswitch: "",
            verbose: false,
            brk: None,
            no_volid: false,
            debug: false,
            wpa_conf: Some("/etc/wpa.conf".to_string()),
            ip: None,
            prepdev: None,
            sshkey: None,
            sshkey_file: None,
            root: None,
            root_backend: None,
            running_config: "/run/running-config".to_string(),
            init_path: "/sbin/init".to_string(),
            dev_console: "dev/console".to_string(),
            drivers: DEFAULT_DRIVERS.iter().map(|s| s.to_string()).collect(),
            prepared_device: None,
            root_mounted: false,
        }
    }
}

impl Config {
    pub fn run(&mut self) -> Result<()> {
        self.enter_stage("init")?;
        self.init_ramdisk()?;
        self.init_env()?;

        self.enter_stage("udev")?;
        self.init_udev()?;

        self.enter_stage("modprobe")?;
        if self.executable_exists("/sbin/modprobe") && Path::new("/lib/modules").exists() {
            self.init_modprobe()?;
        } else if Path::new("/load-modules").exists() {
            self.source_load_modules()?;
        }

        self.enter_stage("wifi")?;
        if let Some(ref wpa_conf) = self.wpa_conf {
            if !wpa_conf.is_empty() {
                self.init_wifi()?;
            }
        }

        self.enter_stage("netconfig")?;
        if let Some(ref ip) = self.ip {
            if !ip.is_empty() {
                self.init_net(ip)?;
            }
        }

        if let Some(ref prepdev) = self.prepdev {
            let prepdev = prepdev.to_string();
            self.enter_stage("prepfs")?;
            for dev in prepdev.split(',') {
                self.prepare_dev(dev, Some(0))?;
            }
        }

        if let Some(ref sshkey) = self.sshkey {
            self.add_ssh_key(sshkey.as_bytes())?;
        }
        if let Some(ref sshkey_file) = self.sshkey_file {
            let path = self.prepare_file(sshkey_file)?;
            let contents = fs::read(path)?;
            self.add_ssh_key(&contents)?;
        }

        self.enter_stage("rootfs")?;
        if self.root.is_none() {
            self.auto_root()?;
        }

        if Path::new("/sbin/lvm").exists() {
            self.refresh_lvm()?;
        }

        if Path::new("/usr/lib/uswsusp/resume").exists() && Path::new("/etc/uswsusp.conf").exists() {
            self.enter_stage("resume")?;
            let _ = self.run_command("/usr/lib/uswsusp/resume", &[]);
        }

        if let Some(ref root) = self.root {
            if root != "none" {
                let root = root.to_string();
                self.log(&format!("Mounting root {}..", root));
                self.enter_stage("mountroot")?;
                self.mount_root(&root)?;
                self.enter_stage("finalize")?;
                if self.root_mounted {
                    self.finalize_boot()?;
                }
            }
        }

        self.enter_stage("finish")?;
        self.log("Init finished.");

        if let Some(post_init) = env::var_os("post_init") {
            self.enter_stage("post_init")?;
            let post_init_file = self.prepare_file(&post_init.to_string_lossy())?;
            self.run_script(post_init_file.to_str().context("post_init file path")?, &[])?;
        }

        Ok(())
    }

    fn log(&self, message: &str) {
        if !self.quiet {
            println!("{}", message);
        }
    }

    fn enter_stage(&self, stage: &str) -> Result<()> {
        let script = Path::new("/scripts").join(stage);
        if script.exists() {
            self.run_script(script.to_string_lossy().as_ref(), &[])?;
        }
        let stage_d = Path::new("/scripts").join(format!("{}.d", stage));
        if stage_d.exists() {
            self.run_parts(&stage_d)?;
        }
        Ok(())
    }

    fn run_script(&self, path: &str, args: &[&str]) -> Result<()> {
        let path = Path::new(path);
        if !path.exists() {
            return Ok(());
        }
        if path.extension().and_then(|s| s.to_str()) == Some("sh") {
            self.run_command("/bin/sh", &[path.to_string_lossy().as_ref()])?;
        } else if self.is_executable(path) {
            self.run_command(path.to_string_lossy().as_ref(), args)?;
        }
        Ok(())
    }

    fn run_parts(&self, dir: &Path) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .with_context(|| format!("reading directory {}", dir.display()))?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_file() {
                self.run_script(path.to_string_lossy().as_ref(), &[])?;
            }
        }
        Ok(())
    }

    fn add_ssh_key(&self, keys: &[u8]) -> Result<()> {
        fs::create_dir_all("/.ssh")?;
        let mapped: Vec<u8> = keys.iter().map(|&c| if c == b'_' { b' ' } else { c }).collect();
        let mut file = fs::OpenOptions::new().create(true).append(true).open("/.ssh/authorized_keys")?;
        file.write_all(&mapped)?;
        Ok(())
    }

    fn prepare_file(&self, source: &str) -> Result<PathBuf> {
        if source.starts_with("/") {
            Ok(PathBuf::from(source))
        } else {
            anyhow::bail!("prepare_file only supports absolute paths in this rewrite: {}", source)
        }
    }
}
