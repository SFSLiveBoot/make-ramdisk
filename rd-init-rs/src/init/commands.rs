//! Helpers for running external commands and scripts.

use anyhow::{Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use super::Config;

impl Config {
    pub(crate) fn run_command(&self, program: &str, args: &[&str]) -> Result<()> {
        let status = Command::new(program)
            .args(args)
            .status()
            .with_context(|| format!("running {}", program))?;
        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("command {} failed with status {}", program, status);
        }
    }

    pub(crate) fn is_executable(&self, path: &Path) -> bool {
        path.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }

    pub(crate) fn executable_exists(&self, program: &str) -> bool {
        Path::new(program).exists()
    }

    pub(crate) fn killall(&self, progname: &str) {
        if let Ok(pids) = self.getpids(progname) {
            for pid in pids {
                let _ = Command::new("kill").arg(pid.to_string()).status();
            }
        }
    }

    fn getpids(&self, progname: &str) -> Result<Vec<u32>> {
        let mut result = Vec::new();
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() {
                let stat_path = entry.path().join("stat");
                if let Ok(contents) = fs::read_to_string(stat_path) {
                    if let Some(cmd) = contents.split_whitespace().nth(1) {
                        if cmd == format!("({})", progname) {
                            result.push(pid);
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}
