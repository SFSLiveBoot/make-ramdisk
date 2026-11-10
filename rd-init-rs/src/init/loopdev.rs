//! Loop device helpers for attaching backing files, like busybox does.

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
    pub(crate) fn get_free_loop_dev(&self) -> Result<String> {
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

    pub(crate) fn mount_loop(&self, file: &str, target: &str, fstype: &str) -> Result<()> {
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
}
