//! Mount helpers: tmpfs, source and overlay root mounts.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use super::Config;

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
