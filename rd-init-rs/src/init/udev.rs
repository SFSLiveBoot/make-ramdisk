//! Udev stage initialization.

use anyhow::Result;
use std::ffi::CString;
use std::fs;
use std::path::Path;

use super::Config;

impl Config {
    pub(crate) fn init_udev(&self) -> Result<()> {
        self.log("udev");
        if Path::new("/run").exists() {
            fs::create_dir_all("/run/udev")?;
        } else {
            fs::create_dir_all("/dev/.udev/db")?;
            fs::create_dir_all("/dev/.udev/queue")?;
        }
        self.mknod_if_missing("/dev/null", 1, 3)?;
        self.mknod_if_missing("/dev/console", 5, 1)?;
        self.mknod_if_missing("/dev/tty", 5, 0)?;
        if !self.no_volid {
            fs::write("/etc/udev/rules.d/vol_id.rules", "# Disabled with no_volid\n")?;
        }
        self.run_command("udevd", &["--daemon"]).ok();
        self.run_command("udevadm", &["trigger"]).ok();
        self.run_command("udevadm", &["settle"]).ok();
        Ok(())
    }

    fn mknod_if_missing(&self, path: &str, major: u32, minor: u32) -> Result<()> {
        if !Path::new(path).exists() {
            let cdev = CString::new(path)?;
            let rc = unsafe { nix::libc::mknod(cdev.as_ptr(), nix::libc::S_IFCHR | 0o644, nix::libc::makedev(major, minor)) };
            if rc != 0 {
                anyhow::bail!("failed to create device node {}: mknod failed", path);
            }
        }
        Ok(())
    }
}
