//! Boot finalization: migrate pseudo filesystems and hand off to the real init.

use anyhow::Result;
use std::env;
use std::ffi::CString;
use std::fs;
use std::path::Path;

use super::Config;

impl Config {
    pub(crate) fn finalize_boot(&self) -> Result<()> {
        self.killall("udevd");
        let mounts = fs::read_to_string("/proc/mounts")?;
        let mut migrate_list = Vec::new();
        for line in mounts.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 2 { continue; }
            let mpt = fields[1];
            if mpt.starts_with("/mnt") || mpt.starts_with("/parts") || mpt == "/dev" || mpt == "/sys" || mpt.starts_with("/live") || mpt == "/proc" || mpt == "/run" {
                if let Err(_) = fs::create_dir_all(format!("/realroot{}", mpt)) {};
                migrate_list.push(mpt.to_string());
            }
        }
        for mpt in migrate_list {
            let dst = format!("/realroot{}", mpt);
            let _ = nix::mount::mount(
                Some(mpt.as_str()),
                dst.as_str(),
                None::<&str>,
                nix::mount::MsFlags::MS_MOVE,
                None::<&str>,
            );
        }
        // Make sure the root filesystem has all necessary files before pivot.
        if self.debug {
            let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
            for line in mounts.lines() {
                println!("{}", line);
            }
            if let Ok(entries) = fs::read_dir("/realroot/dev") {
                for entry in entries.flatten() {
                    println!("/dev/{}", entry.file_name().to_string_lossy());
                }
            }
            if Path::new(&format!("/realroot/{}", self.init_path)).exists() {
                println!("/sbin/init present: {}", self.init_path);
            }
        }
        if !Path::new(&format!("/realroot/{}", self.dev_console)).exists() {
            let dev = format!("/realroot/{}", self.dev_console);
            let cdev = CString::new(dev.as_str())?;
            let rc = unsafe { nix::libc::mknod(cdev.as_ptr(), nix::libc::S_IFCHR | 0o644, nix::libc::makedev(5, 1)) };
            let _ = rc;
        }
        if !Path::new(&format!("/realroot/{}", self.init_path)).exists() {
            anyhow::bail!("root filesystem {} does not have the init before pivot", self.init_path);
        }
        self.log(&format!("End of ramdisk: run-init /realroot {}", self.init_path));
        if self.brk.is_none() {
            env::set_var("RUST_BACKTRACE", "1");
            let args: Vec<CString> = vec![
                CString::new("/bin/run-init").unwrap(),
                CString::new("-c").unwrap(),
                CString::new(self.dev_console.as_str()).unwrap(),
                CString::new("/realroot").unwrap(),
                CString::new(self.init_path.as_str()).unwrap(),
            ];
            nix::unistd::execv(&CString::new("/bin/run-init").unwrap(), &args)?;
        }
        Ok(())
    }
}
