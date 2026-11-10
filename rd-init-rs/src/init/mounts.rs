//! Mount helpers: loop devices, tmpfs and overlay root mounts.

use anyhow::{Context, Result};
use std::ffi::CString;
use std::fs;
use std::os::unix::io::IntoRawFd;
use std::path::Path;

use super::Config;

/// `struct loop_info64` from <linux/loop.h>, used by the loop device.
#[repr(C)]
struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdevice: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; 64],
    lo_crypt_name: [u8; 64],
    lo_encrypt_key: [u8; 32],
    lo_init: [u64; 2],
}

/// `struct loop_config` from <linux/loop.h>, used by the `LOOP_CONFIGURE` ioctl.
#[repr(C)]
struct LoopConfig {
    fd: u32,
    block_size: u32,
    info: LoopInfo64,
    reserved: [u64; 8],
}

// ioctl(LOOP_CTL_GET_FREE) on /dev/loop-control to obtain a free loop device.
ioctl_none_bad!(loop_ctl_get_free, 0x4C82u32);
// ioctl(LOOP_GET_STATUS) on a loop device to inspect it.
ioctl_read_bad!(loop_get_status, 0x4C03u32, LoopInfo64);
// ioctl(LOOP_CONFIGURE) on the loop device to attach a backing file to it.
ioctl_write_ptr_bad!(loop_configure, 0x4C0Au32, LoopConfig);

/// Build the loop device information for attaching a backing file, like busybox does.
fn loop_info64(file: &str) -> LoopInfo64 {
    let mut file_name = [0u8; 64];
    let bytes = file.as_bytes();
    let n = bytes.len().min(file_name.len());
    file_name[..n].copy_from_slice(&bytes[..n]);
    LoopInfo64 {
        lo_device: 0,
        lo_inode: 0,
        lo_rdevice: 0,
        lo_offset: 0,
        lo_sizelimit: 0,
        lo_number: 0,
        lo_encrypt_type: 0,
        lo_encrypt_key_size: 0,
        lo_flags: 4, // LO_FLAGS_AUTOCLEAR
        lo_file_name: file_name,
        lo_crypt_name: [0; 64],
        lo_encrypt_key: [0; 32],
        lo_init: [0; 2],
    }
}

impl Config {
    fn mount_tmpfs(&self, target: &str) -> Result<()> {
        nix::mount::mount(
            Some("tmpfs"),
            target,
            Some("tmpfs"),
            nix::mount::MsFlags::empty(),
            Some("mode=0755"),
        )
        .with_context(|| format!("mounting tmpfs on {}", target))
    }

    fn get_free_loop_dev(&self) -> Result<String> {
        // Ask /dev/loop-control for a free loop device, like /sbin/mount does.
        if let Ok(ctl) = fs::OpenOptions::new().read(true).write(true).open("/dev/loop-control") {
            let ctl_fd = ctl.into_raw_fd();
            let loop_dev = unsafe { loop_ctl_get_free(ctl_fd) }?;
            unsafe { nix::libc::close(ctl_fd) };
            if loop_dev >= 0 {
                let dev = format!("/dev/loop{}", loop_dev);
                if !Path::new(&dev).exists() {
                    // The loop device node might not exist yet; create it, like busybox does.
                    let cdev = CString::new(dev.as_str())?;
                    let rc = unsafe { nix::libc::mknod(cdev.as_ptr(), nix::libc::S_IFBLK | 0o644, nix::libc::makedev(7, loop_dev as u32)) };
                    if rc != 0 {
                        anyhow::bail!("failed to create loop device {}", dev);
                    }
                }
                return Ok(dev);
            }
        }
        // /dev/loop-control is unavailable; scan for a loop device, like busybox does.
        for i in 0..100000u32 {
            let dev = format!("/dev/loop{}", i);
            if !Path::new(&dev).exists() {
                let cdev = CString::new(dev.as_str())?;
                let rc = unsafe { nix::libc::mknod(cdev.as_ptr(), nix::libc::S_IFBLK | 0o644, nix::libc::makedev(7, i as u32)) };
                if rc != 0 {
                    anyhow::bail!("failed to create loop device {}", dev);
                }
            }
            return Ok(dev);
        }
        anyhow::bail!("no free loop device available")
    }

    fn mount_loop(&self, file: &str, target: &str, fstype: &str) -> Result<()> {
        // Set up a loop device for the backing file before mounting, like /sbin/mount does.
        let backing = fs::OpenOptions::new().read(true).write(true).open(file)?;
        let backing_fd = backing.into_raw_fd();
        let loop_dev = self.get_free_loop_dev()?;
        let lfd = fs::OpenOptions::new().read(true).write(true).open(&loop_dev)?.into_raw_fd();

        let mut info = loop_info64(file);
        let rc = match unsafe { loop_get_status(lfd, &mut info) } {
            Ok(rc) => rc,
            Err(nix::errno::Errno::ENXIO) => -1, // ENXIO means the loop device is free
            Err(e) => return Err(e.into()),
        };
        if rc == 0 {
            anyhow::bail!("loop device {} is busy", loop_dev);
        }

        let mut lconfig = LoopConfig {
            fd: backing_fd as u32,
            block_size: 0,
            info,
            reserved: [0; 8],
        };
        unsafe {
            loop_configure(lfd, &mut lconfig as *mut LoopConfig as *const LoopConfig)
                .with_context(|| format!("configuring loop device {}", loop_dev))?;
        }

        let result = nix::mount::mount(
            Some(loop_dev.as_str()),
            target,
            Some(fstype),
            nix::mount::MsFlags::empty(),
            None::<&str>,
        )
        .with_context(|| format!("loop-mounting {} from {} on {}", fstype, file, target));

        unsafe { nix::libc::close(backing_fd) };
        unsafe { nix::libc::close(lfd) };
        result
    }

    fn mount_source(&self, source: &str, target: &str) -> Result<()> {
        let fs: &str = if source == "mem" { "tmpfs" } else { &self.fs_type(source)? };
        nix::mount::mount(Some(source), target, Some(fs), nix::mount::MsFlags::empty(), None::<&str>)
            .with_context(|| format!("mounting {} on {}", source, target))
    }

    pub(crate) fn mount_root(&mut self, root: &str) -> Result<()> {
        fs::create_dir_all("/realroot")?;
        fs::create_dir_all("/live")?;

        let parts: Vec<String> = root
            .split('+')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();

        // If the rootfs is prefixed with `mem:`, then `/live` should be tmpfs and all sfs component files should be copied there before loop-mounting to /.parts.
        let copy_up = root.starts_with("mem:");
        let mut live = "/live".to_string();
        let mut live_mounted = false;
        let mut live_mounts: Vec<String> = Vec::new();
        let mut mounted: Vec<(String, String)> = Vec::new();
        let mut content_mounts: Vec<String> = Vec::new();
        let mut storage_mount: Option<String> = None;

        for part in parts {
            let part = part.trim().to_string();
            if part.is_empty() {
                continue;
            }

            if part.contains(':') {
                let idx = part.find(':').unwrap();
                let before = part[..idx].trim().to_string();
                let after = part[idx + 1..].trim().to_string();

                let source_mnt = if before.is_empty() {
                    live.clone()
                } else {
                    let source_mnt = if live_mounted {
                        format!("/live{}", live_mounts.len())
                    } else {
                        "/live".to_string()
                    };
                    live_mounted = true;
                    live_mounts.push(source_mnt.clone());
                    self.mount_source(&before, &source_mnt)?;
                    source_mnt
                };
                live = source_mnt.clone();

                let pattern = format!(
                    "{}/{}",
                    source_mnt.trim_end_matches('/'),
                    after.trim_start_matches('/')
                );
                let mut files: Vec<String> = Vec::new();
                if copy_up {
                    // The rootfs is prefixed with `mem:`, so the component files should be copied into the tmpfs-backed /live before mounting.
                    for entry in glob::glob(&pattern)? {
                        let entry = entry?;
                        let file_name = entry.file_name().unwrap().to_string_lossy().into_owned();
                        let dst = format!("{}/{}", source_mnt.trim_end_matches('/'), file_name);
                        fs::copy(&entry, &dst)?;
                        files.push(dst);
                    }
                } else if after.contains('*') || after.contains('[') {
                    // The component may be designated by a glob pattern (e.g. `*.sfs`), all matching files form stacked content layers.
                    for entry in glob::glob(&pattern)? {
                        let entry = entry?;
                        files.push(entry.to_string_lossy().into_owned());
                    }
                } else {
                    files.push(pattern.clone());
                }
                if files.is_empty() {
                    anyhow::bail!("no files found matching pattern {}", pattern);
                }

                for file in files {
                    let file_name = file.rsplit('/').next().unwrap().to_string();
                    let target = format!("/.parts/{}", file_name);
                    if let Some(parent) = Path::new(&target).parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::create_dir_all(&target)?;
                    let fstype = if file.ends_with(".sfs") {
                        "squashfs".to_string()
                    } else {
                        self.fs_type(&file)?
                    };
                    self.mount_loop(&file, &target, &fstype)?;
                    content_mounts.push(target.clone());
                    storage_mount = Some(source_mnt.clone());
                    mounted.push((part.clone(), target));
                }
            } else {
                let target = format!("/.parts/{}", part);
                fs::create_dir_all(&target)?;
                if part == "mem" {
                    self.mount_tmpfs(&target)?;
                    storage_mount = Some(target.clone());
                } else if part.ends_with(".sfs") {
                    self.mount_loop(&part, &target, "squashfs")?;
                    content_mounts.push(target.clone());
                } else {
                    self.mount_source(&part, &target)?;
                    storage_mount = Some(target.clone());
                }
                mounted.push((part.clone(), target));
            }
        }

        // Finally mount them all together as overlayfs on /realroot, with all content parts stacked as lower layers, the last mounted part on top of the earlier ones.
        let lower = if content_mounts.is_empty() {
            mounted.last().map(|m| m.1.clone()).unwrap_or_default()
        } else {
            content_mounts
                .iter()
                .rev()
                .map(|s| s.as_str())
                .collect::<Vec<&str>>()
                .join(":")
        };
        fs::create_dir_all("/realroot/.parts")?;
        let (upper, work) = if let Some(storage) = storage_mount.clone() {
            (
                format!("{}/upper", storage),
                format!("{}/work", storage),
            )
        } else if !lower.is_empty() {
            (
                format!("{}/upper", lower),
                format!("{}/work", lower),
            )
        } else {
            (
                "/realroot/.parts".to_string(),
                "/realroot/.parts/.work".to_string(),
            )
        };
        fs::create_dir_all(&upper)?;
        fs::create_dir_all(&work)?;
        let data = format!("lowerdir={},upperdir={},workdir={}", lower, upper, work);
        let data_str: &str = &data;
        nix::mount::mount(
            Some("overlay"),
            "/realroot",
            Some("overlay"),
            nix::mount::MsFlags::empty(),
            Some(data_str),
        )
        .with_context(|| format!("mounting overlay root on /realroot with lowerdir {}", lower))?;

        // After mounting as overlay, create /realroot/.parts and move each individual part from /.parts/XXX to /realroot/.parts/XXX.
        for (_, target) in mounted {
            let dst = format!("/realroot{}", target);
            fs::create_dir_all(&dst)?;
            nix::mount::mount(
                Some(target.as_str()),
                dst.as_str(),
                None::<&str>,
                nix::mount::MsFlags::MS_MOVE,
                None::<&str>,
            )?;
        }
        for mnt in live_mounts {
            let dst = format!("/realroot{}", mnt);
            fs::create_dir_all(&dst)?;
            nix::mount::mount(
                Some(mnt.as_str()),
                dst.as_str(),
                None::<&str>,
                nix::mount::MsFlags::MS_MOVE,
                None::<&str>,
            )?;
        }

        self.root_mounted = true;
        Ok(())
    }
}
