pub mod button;
pub mod drives;
pub mod metrics;
pub mod reset;
pub mod ssh;
pub mod update;

use std::fs;
use std::time::Duration;

use serde::Serialize;

use crate::net::network::run;

#[derive(Serialize)]
pub struct SystemInfo {
    pub version: &'static str,
    pub hostname: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub uptime_secs: Option<u64>,
    pub load: Option<[f32; 3]>,
    pub cpu_temp_c: Option<f32>,
    pub mem_total_kb: Option<u64>,
    pub mem_available_kb: Option<u64>,
    pub disk_total_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
    pub throttling: Option<Throttling>,
    pub settings_path: String,
    /// Running as a systemd unit - restart and logs are only available then
    pub systemd: bool,
    pub unit: String,
}

/// Decoded `vcgencmd get_throttled`. "now" = currently, "seen" = since boot
#[derive(Serialize)]
pub struct Throttling {
    pub raw: String,
    pub under_voltage_now: bool,
    pub throttled_now: bool,
    pub under_voltage_seen: bool,
    pub throttled_seen: bool,
}

pub fn unit() -> String {
    std::env::var("YSTREAMER_UNIT").unwrap_or_else(|_| "ystreamer.service".into())
}

/// systemd sets INVOCATION_ID for every process it starts
pub fn under_systemd() -> bool {
    std::env::var_os("INVOCATION_ID").is_some()
}

pub async fn info(settings_path: &std::path::Path) -> SystemInfo {
    let read = |p: &str| fs::read_to_string(p).ok();
    // Device-tree strings are NUL-terminated
    let dt = |p: &str| read(p).map(|s| s.trim_end_matches('\0').trim().to_string());

    let os = read("/etc/os-release").and_then(|s| {
        s.lines()
            .find_map(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string())
    });

    let meminfo = read("/proc/meminfo").unwrap_or_default();
    let mem = |key: &str| {
        meminfo
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
    };

    let load = read("/proc/loadavg").and_then(|s| {
        let v: Vec<f32> = s
            .split_whitespace()
            .take(3)
            .filter_map(|x| x.parse().ok())
            .collect();
        (v.len() == 3).then(|| [v[0], v[1], v[2]])
    });

    let (disk_total_bytes, disk_free_bytes) = match disk_usage("/") {
        Some((t, f)) => (Some(t), Some(f)),
        None => (None, None),
    };

    SystemInfo {
        version: env!("CARGO_PKG_VERSION"),
        hostname: crate::settings::current_hostname(),
        model: dt("/proc/device-tree/model"),
        serial: dt("/proc/device-tree/serial-number"),
        os,
        kernel: read("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()),
        uptime_secs: read("/proc/uptime")
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            .map(|v| v as u64),
        load,
        cpu_temp_c: read("/sys/class/thermal/thermal_zone0/temp")
            .and_then(|s| s.trim().parse::<f32>().ok())
            .map(|m| m / 1000.0),
        mem_total_kb: mem("MemTotal:"),
        mem_available_kb: mem("MemAvailable:"),
        disk_total_bytes,
        disk_free_bytes,
        throttling: throttling().await,
        settings_path: settings_path.display().to_string(),
        systemd: under_systemd(),
        unit: unit(),
    }
}

async fn throttling() -> Option<Throttling> {
    let out = run("vcgencmd", &["get_throttled"]).await.ok()?;
    let raw = out.trim().strip_prefix("throttled=")?.to_string();
    let bits = u32::from_str_radix(raw.trim_start_matches("0x"), 16).ok()?;
    Some(Throttling {
        under_voltage_now: bits & 0x1 != 0,
        throttled_now: bits & 0x4 != 0,
        under_voltage_seen: bits & 0x10000 != 0,
        throttled_seen: bits & 0x40000 != 0,
        raw,
    })
}

fn disk_usage(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frsize = st.f_frsize as u64;
    Some((st.f_blocks as u64 * frsize, st.f_bavail as u64 * frsize))
}

const CLOUD_INIT_DIR: &str = "/etc/cloud/cloud.cfg.d";
const CLOUD_INIT_KEEP: &str = "/etc/cloud/cloud.cfg.d/99-ystreamer-hostname.cfg";
const HOSTS: &str = "/etc/hosts";

pub async fn set_hostname(name: &str) -> Result<(), String> {
    if crate::settings::current_hostname() != name {
        run("hostnamectl", &["set-hostname", name]).await?;
    }
    keep_hostname(name);
    Ok(())
}

/// Something else may have renamed the system since the name was saved:
/// images flashed with Raspberry Pi Imager have cloud-init put its own
/// name back on every boot
pub async fn ensure_hostname(saved: &str) {
    if crate::settings::validate_hostname(saved).is_err() {
        return;
    }
    let was = crate::settings::current_hostname();
    match set_hostname(saved).await {
        Ok(()) if was != saved => println!("[system] renamed {was} back to {saved}"),
        Ok(()) => {}
        Err(e) => eprintln!("[system] couldn't rename {was} to {saved}: {e}"),
    }
}

pub async fn set_timezone(tz: &str) -> Result<(), String> {
    if crate::settings::current_timezone() != tz {
        run("timedatectl", &["set-timezone", tz]).await?;
    }
    reload_timezone();
    Ok(())
}

// In the C library on every system, but not in the libc crate's Linux list
unsafe extern "C" {
    fn tzset();
}

/// glibc keeps the zone it read first; this makes it read /etc/localtime
/// again, so local times follow a zone changed while running
pub fn reload_timezone() {
    unsafe { tzset() };
}

/// The zone can also be set outside YStreamer: raspi-config, timedatectl,
/// or Raspberry Pi Imager's options on the first boot. The system's is the
/// one that counts, so the settings follow it
pub fn adopt_timezone(store: &crate::settings::SettingsStore) {
    let system = crate::settings::current_timezone();
    let saved = store.get().device.timezone;
    if saved == system {
        return;
    }
    match store.update(|s| s.device.timezone = system.clone()) {
        Ok(_) => println!("[system] timezone is {system}, not {saved}"),
        Err(e) => eprintln!("[system] couldn't save the timezone {system}: {e}"),
    }
}

// Neither is worth failing a rename over
fn keep_hostname(name: &str) {
    if std::path::Path::new(CLOUD_INIT_DIR).is_dir()
        && !std::path::Path::new(CLOUD_INIT_KEEP).exists()
        && let Err(e) = fs::write(CLOUD_INIT_KEEP, "preserve_hostname: true\n")
    {
        eprintln!("[system] {CLOUD_INIT_KEEP}: {e}");
    }
    // hostnamectl leaves this alone, and sudo complains about a name that
    // doesn't resolve
    if let Ok(hosts) = fs::read_to_string(HOSTS) {
        let renamed = with_local_name(&hosts, name);
        if renamed != hosts
            && let Err(e) = fs::write(HOSTS, renamed)
        {
            eprintln!("[system] {HOSTS}: {e}");
        }
    }
}

/// /etc/hosts with the 127.0.1.1 line naming this machine
fn with_local_name(hosts: &str, name: &str) -> String {
    let entry = format!("127.0.1.1 {name}");
    let is_local = |line: &str| line.split_whitespace().next() == Some("127.0.1.1");
    let mut lines: Vec<&str> = hosts.lines().collect();
    match lines.iter().position(|l| is_local(l)) {
        Some(i) => lines[i] = &entry,
        None => lines.push(&entry),
    }
    lines.join("\n") + "\n"
}

#[derive(Clone, Copy)]
pub enum Action {
    Restart,
    Reboot,
    Poweroff,
}

impl Action {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "restart" => Self::Restart,
            "reboot" => Self::Reboot,
            "poweroff" => Self::Poweroff,
            _ => return None,
        })
    }
}

/// Check an action can run, then run it shortly after, so the HTTP response
/// makes it out before this process (or the whole device) goes away
pub fn schedule(action: Action) -> Result<(), String> {
    let args: Vec<String> = match action {
        Action::Restart if !under_systemd() => {
            return Err("Not running under systemd; restart the process manually".into());
        }
        Action::Restart => vec!["restart".into(), "--no-block".into(), unit()],
        Action::Reboot => vec!["reboot".into()],
        Action::Poweroff => vec!["poweroff".into()],
    };
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Err(e) = run("systemctl", &args).await {
            eprintln!("[system] systemctl {}: {e}", args.join(" "));
        }
    });
    Ok(())
}

pub async fn logs(lines: u32) -> Result<String, String> {
    if !under_systemd() {
        return Err("Logs are available once YStreamer runs as a systemd service".into());
    }
    let unit = unit();
    let n = lines.clamp(10, 2000).to_string();
    run(
        "journalctl",
        &["-u", &unit, "-n", &n, "--no-pager", "-o", "short-iso"],
    )
    .await
}

#[cfg(test)]
#[path = "system.test.rs"]
mod tests;
