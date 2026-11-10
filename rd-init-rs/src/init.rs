use anyhow::{Context, Result};
use std::env;
use std::ffi::CString;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::IntoRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const DEFAULT_DRIVERS: &[&str] = &[
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
            _ if token.starts_with("break=") => self.brk = Some(token[6..].to_string()),
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

    fn init_ramdisk(&self) -> Result<()> {
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
            let reader = io::BufReader::new(file);
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

    fn init_env(&mut self) -> Result<()> {
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

    fn init_udev(&self) -> Result<()> {
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
            self.run_command("/bin/mknod", &[path, "c", &major.to_string(), &minor.to_string()])?;
        }
        Ok(())
    }

    fn init_modprobe(&self) -> Result<()> {
        self.log(&format!("loading drivers: {}", self.drivers.join(",")));
        for module in &self.drivers {
            self.run_command("/sbin/modprobe", &[module]).ok();
        }
        if Path::new("/etc/modules.preload").exists() {
            let contents = fs::read_to_string("/etc/modules.preload")?;
            for module in contents.lines().map(str::trim).filter(|l| !l.is_empty()) {
                self.run_command("/sbin/modprobe", &[module]).ok();
            }
        }
        self.enter_stage("pci")?;
        if Path::new("/sys/bus/pci/devices").exists() {
            for entry in fs::read_dir("/sys/bus/pci/devices")? {
                let entry = entry?;
                let modalias = entry.path().join("modalias");
                if modalias.exists() {
                    let alias = fs::read_to_string(modalias)?.trim().to_string();
                    self.run_command("/sbin/modprobe", &[&alias]).ok();
                }
            }
        }
        Ok(())
    }

    fn source_load_modules(&self) -> Result<()> {
        let contents = fs::read_to_string("/load-modules")?;
        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with("modprobe") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 1 {
                    self.run_command("/sbin/modprobe", &[parts[1]]).ok();
                }
            }
        }
        Ok(())
    }

    fn init_wifi(&self) -> Result<()> {
        let wpa_conf = self.wpa_conf.as_ref().unwrap();
        if !Path::new(wpa_conf).exists() {
            return Ok(());
        }
        let iface = self.find_wifi_iface()?;
        self.log(&format!("running wpa_supplicant on {} using {}", iface, wpa_conf));
        self.run_command("wpa_supplicant", &["-B", "-Dnl80211", "-i", &iface, "-c", wpa_conf])
    }

    fn find_wifi_iface(&self) -> Result<String> {
        let mut result = None;
        for entry in fs::read_dir("/sys/class/net")? {
            let entry = entry?;
            let path = entry.path();
            if path.join("phy80211").exists() {
                result = Some(entry.file_name().to_string_lossy().into_owned());
                break;
            }
        }
        result.context("80211 interface not found")
    }

    fn init_net(&self, ip: &str) -> Result<()> {
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

    fn auto_root(&mut self) -> Result<()> {
        for dev in self.list_partitions()? {
            if self.fs_type(&dev)? == "crypto_LUKS" {
                if self.prepare_dev(&dev, Some(5)).is_ok() && self.prepare_dev("@ROOT", Some(5)).is_ok() {
                    self.root = Some(self.prepared_device()?);
                    return Ok(());
                }
            }
        }
        if self.prepare_dev("@ROOT", Some(5)).is_ok() {
            self.root = Some(self.prepared_device()?);
        }
        Ok(())
    }

    fn list_partitions(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let contents = fs::read_to_string("/proc/partitions")?;
        for line in contents.lines().skip(2) {
            let mut parts = line.split_whitespace();
            if let (Some(_major), Some(_minor), Some(_blocks), Some(name)) = (parts.next(), parts.next(), parts.next(), parts.next()) {
                let device = format!("/dev/{}", name);
                if Path::new(&device).exists() {
                    out.push(device);
                }
            }
        }
        Ok(out)
    }

    fn fs_type(&self, device: &str) -> Result<String> {
        let output = Command::new("/sbin/blkid")
            .args(["-o", "value", "-s", "TYPE", device])
            .output()
            .with_context(|| format!("running blkid on {}", device))?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn prepare_dev(&mut self, dev: &str, _timeout: Option<u64>) -> Result<()> {
        let resolved = if dev.starts_with("LABEL=") {
            format!("/dev/disk/by-label/{}", &dev[6..])
        } else if dev.starts_with("UUID=") {
            format!("/dev/disk/by-uuid/{}", &dev[5..])
        } else {
            dev.to_string()
        };
        if !Path::new(&resolved).exists() {
            anyhow::bail!("device not found: {}", resolved);
        }
        self.prepared_device = Some(resolved);
        Ok(())
    }

    fn prepared_device(&self) -> Result<String> {
        self.prepared_device.clone().context("prepared device not available")
    }

    fn refresh_lvm(&self) -> Result<()> {
        let _ = fs::remove_file("/etc/lvm/cache/.cache");
        self.run_command("lvm", &["vgscan", "--mknodes"]).ok();
        self.run_command("lvm", &["vgchange", "-a", "y"]).ok();
        Ok(())
    }

    fn mount_root(&mut self, root: &str) -> Result<()> {
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

    fn finalize_boot(&self) -> Result<()> {
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
            if Path::new(&format!("/realroot{}", self.init_path)).exists() {
                println!("/sbin/init present: {}", self.init_path);
            }
        }
        if !Path::new(&format!("/realroot/{}", self.dev_console)).exists() {
            let _ = Command::new("/bin/mknod").args([&format!("/realroot/{}", self.dev_console), "c", "5", "1"]).status();
        }
        if !Path::new(&format!("/realroot{}", self.init_path)).exists() {
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

    fn killall(&self, progname: &str) {
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

    fn is_executable(&self, path: &Path) -> bool {
        path.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }

    fn executable_exists(&self, program: &str) -> bool {
        Path::new(program).exists()
    }

    fn run_command(&self, program: &str, args: &[&str]) -> Result<()> {
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
}
