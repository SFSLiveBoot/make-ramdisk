//! Ramdisk bootstrap: virtual filesystem setup and initial environment.

use anyhow::Result;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::BufRead;
use std::path::Path;

use super::Config;

impl Config {
    pub(crate) fn init_ramdisk(&self) -> Result<()> {
        for p in &["/sys", "/proc", "/tmp", "/realroot", "/etc/udev", "/var/log", "/mnt", "/parts", "/usr/sbin", "/usr/bin", "/sbin", "/dev/pts"] {
            fs::create_dir_all(p)?;
        }
        if !Path::new("/run").exists() {
            fs::create_dir_all("/run")?;
            nix::mount::mount(
                Some("shmfs"),
                "/run",
                Some("tmpfs"),
                nix::mount::MsFlags::MS_NOSUID | nix::mount::MsFlags::MS_NOEXEC,
                Some("mode=0755"),
            )?;
        }
        fs::OpenOptions::new().append(true).create(true).open("/etc/fstab")?;
        if !Path::new("/etc/mtab").exists() {
            let _ = std::os::unix::fs::symlink("/proc/mounts", "/etc/mtab");
        }
        if !self.is_mounted("/proc") {
            nix::mount::mount(Some("none"), "/proc", Some("proc"), nix::mount::MsFlags::empty(), None::<&str>)?;
        }
        if !self.is_mounted("/sys") {
            nix::mount::mount(Some("none"), "/sys", Some("sysfs"), nix::mount::MsFlags::empty(), None::<&str>)?;
        }
        if !self.is_mounted("/dev") {
            nix::mount::mount(Some("devtmpfs"), "/dev", Some("devtmpfs"), nix::mount::MsFlags::empty(), None::<&str>).ok();
        }
        if Path::new("/bin/busybox").exists() {
            self.run_command("/bin/busybox", &["--install", "-s"]).ok();
        }
        Ok(())
    }

    fn is_mounted(&self, target: &str) -> bool {
        if let Ok(file) = fs::File::open("/proc/mounts") {
            let reader = std::io::BufReader::new(file);
            for line in reader.lines().flatten() {
                if let Some(field) = line.split_whitespace().nth(1) {
                    if field == target {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub(crate) fn init_env(&mut self) -> Result<()> {
        self.load_cmdline("/proc/cmdline")?;
        self.update_ld_library_path();
        Ok(())
    }

    fn update_ld_library_path(&self) {
        let mut libs = env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
        let nss_entries = glob::glob("/lib*/*-*/libnss_*");
        if let Ok(entries) = nss_entries {
            for entry in entries.flatten() {
                if let Some(parent) = entry.parent() {
                    let parent_str = parent.to_string_lossy().into_owned();
                    let current = env::var_os("LD_LIBRARY_PATH").unwrap_or_default().to_string_lossy().into_owned();
                    if !current.contains(&parent_str[..]) {
                        libs = if libs.is_empty() {
                            OsString::from(&*parent_str)
                        } else {
                            let mut v = libs.clone();
                            v.push(OsString::from(":"));
                            v.push(OsString::from(&*parent_str));
                            v
                        };
                    }
                }
            }
        }
        env::set_var("LD_LIBRARY_PATH", libs);
    }
}
