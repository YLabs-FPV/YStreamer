use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::settings::{EthernetMode, EthernetSettings, WifiMode, WifiSettings};

const IFACE: &str = "wlan0";
const AP_CON: &str = "ystreamer-ap";
const CLIENT_CON: &str = "ystreamer-wifi";
const ETH_CON: &str = "ystreamer-eth";
/// How long client mode may go without joining before the fallback AP
const JOIN_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the device may be unreachable on every interface. Short: an
/// unplugged cable won't come back by waiting
const STRANDED_TIMEOUT: Duration = Duration::from_secs(15);
const WATCH_TICK: Duration = Duration::from_secs(5);
/// How long the fallback AP must sit unused, with Ethernet back, before it
/// goes. Never while someone is connected to it
const AP_IDLE_OFF: Duration = Duration::from_secs(30);
const AP_START_GRACE: Duration = Duration::from_secs(10);
/// What a phone sharing its connection over USB shows up as: Android
/// (RNDIS, NCM, ECM) and iPhone. Also type "ethernet" to NetworkManager,
/// so these must never be mistaken for the Ethernet port
const TETHER_DRIVERS: [&str; 4] = ["rndis_host", "cdc_ncm", "cdc_ether", "ipheth"];

#[derive(Default)]
pub struct Network {
    /// Serialises applies: two nmcli sequences interleaving would leave a mess
    apply_lock: Mutex<()>,
    /// Bumped on every WiFi apply so the watchdog starts counting afresh
    generation: AtomicU64,
    wifi: std::sync::Mutex<WifiSettings>,
    /// Why the fallback AP is up, None while it isn't
    fallback: std::sync::Mutex<Option<Fallback>>,
    applying: AtomicBool,
    last_error: std::sync::Mutex<Option<String>>,
}

#[derive(Clone)]
enum Fallback {
    /// Couldn't join this network
    Join(String),
    /// No Ethernet and no WiFi
    Stranded,
}

#[derive(Serialize)]
pub struct NetworkStatus {
    pub wifi: InterfaceStatus,
    pub ethernet: Option<InterfaceStatus>,
    /// A phone sharing its connection over USB
    pub tether: Option<InterfaceStatus>,
    /// Interface holding the default route, i.e. where streams to the
    /// internet leave
    pub internet_via: Option<String>,
    /// iPhones need usbmuxd for the "Trust this computer" pairing
    pub iphone_support: bool,
    /// Running from the ready-made image: no "System" network modes
    pub image: bool,
    pub fallback_active: bool,
    /// Why the fallback AP is on, phrased for right now
    pub fallback_reason: Option<String>,
    /// What the AP made up for is fine again; it stays on regardless
    pub fallback_resolved: bool,
    pub applying: bool,
    pub last_error: Option<String>,
}

#[derive(Serialize, Default)]
pub struct InterfaceStatus {
    pub device: String,
    /// NetworkManager state text, e.g. "connected", "disconnected"
    pub state: String,
    pub connection: Option<String>,
    pub addresses: Vec<String>,
    /// "ap" or "client" when a wifi connection is active
    pub role: Option<String>,
    pub ssid: Option<String>,
    /// USB product name, for a tethered phone
    pub product: Option<String>,
    /// How the active connection gets its address: "auto" (DHCP),
    /// "manual" (static), "shared" (our AP)
    pub method: Option<String>,
    pub gateway: Option<String>,
}

#[derive(Serialize)]
pub struct ScanResult {
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub channel: u32,
}

impl Network {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Boot-time hook: start the watchdog that keeps the device reachable.
    /// The profiles themselves persist in NetworkManager, so nothing is rebuilt
    pub fn start(self: &Arc<Self>, cfg: WifiSettings) {
        *self.wifi.lock().unwrap() = cfg;
        let this = Arc::clone(self);
        tokio::spawn(async move { this.watchdog().await });
    }

    pub fn fallback_active(&self) -> bool {
        self.fallback.lock().unwrap().is_some()
    }

    /// Apply in the background. Switching WiFi mode usually drops the browser
    /// that asked for it, so callers respond first and let this run on
    pub fn apply_later(self: &Arc<Self>, cfg: WifiSettings) {
        let this = Arc::clone(self);
        self.applying.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(750)).await;
            let result = this.apply(&cfg).await;
            *this.last_error.lock().unwrap() = result.err();
            this.applying.store(false, Ordering::SeqCst);
        });
    }

    async fn apply(self: &Arc<Self>, cfg: &WifiSettings) -> Result<(), String> {
        let _guard = self.apply_lock.lock().await;
        prepare_radio(cfg).await;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.fallback.lock().unwrap() = None;
        *self.wifi.lock().unwrap() = cfg.clone();

        match cfg.mode {
            WifiMode::Unmanaged => {
                for con in [AP_CON, CLIENT_CON] {
                    let _ = run(
                        "nmcli",
                        &["con", "modify", con, "connection.autoconnect", "no"],
                    )
                    .await;
                    let _ = run("nmcli", &["con", "down", con]).await;
                }
                Ok(())
            }
            WifiMode::Ap => {
                delete_con(CLIENT_CON).await;
                write_ap_profile(cfg, true).await?;
                run("nmcli", &["--wait", "30", "con", "up", AP_CON])
                    .await
                    .map(|_| ())
            }
            WifiMode::Client => {
                // Keep an AP profile around (not auto-connecting) for fallback
                if cfg.fallback_ap {
                    write_ap_profile(cfg, false).await?;
                    let _ = run("nmcli", &["con", "down", AP_CON]).await;
                } else {
                    delete_con(AP_CON).await;
                }
                write_client_profile(cfg).await?;
                let up = run("nmcli", &["--wait", "30", "con", "up", CLIENT_CON]).await;
                if up.is_err() && cfg.fallback_ap {
                    // Don't wait out the full timeout: we already know it failed
                    let why = Fallback::Join(cfg.client.ssid.clone());
                    self.bring_up_fallback(generation, cfg, why).await;
                }
                up.map(|_| ())
                    .map_err(|e| format!("Couldn't join \"{}\": {e}", cfg.client.ssid))
            }
        }
    }

    /// Keep the device reachable: when the configured way in stays down, start
    /// the access point. Joining WiFi gets a minute; with WiFi left to the
    /// system, having neither WiFi nor Ethernet (e.g. unplugged in the field)
    /// only a few seconds. Once Ethernet is back the AP goes again, but only
    /// after nobody has used it for a while
    async fn watchdog(self: Arc<Self>) {
        self.adopt_fallback().await;
        let mut offline = Duration::ZERO;
        let mut idle = Duration::ZERO;
        let mut generation = self.generation.load(Ordering::SeqCst);
        loop {
            tokio::time::sleep(WATCH_TICK).await;
            let current = self.generation.load(Ordering::SeqCst);
            if current != generation {
                generation = current;
                offline = Duration::ZERO;
                idle = Duration::ZERO;
            }
            let cfg = self.wifi.lock().unwrap().clone();
            if cfg.mode == WifiMode::Ap {
                offline = Duration::ZERO;
                continue;
            }

            let fallback = self.fallback.lock().unwrap().clone();
            if let Some(why) = fallback {
                // A failed join can't recover while the AP holds wlan0
                let unneeded =
                    matches!(why, Fallback::Stranded) && ethernet_up().await && ap_unused().await;
                idle = if unneeded {
                    idle + WATCH_TICK
                } else {
                    Duration::ZERO
                };
                if idle >= AP_IDLE_OFF {
                    self.take_down_fallback(generation).await;
                    idle = Duration::ZERO;
                }
                offline = Duration::ZERO;
                continue;
            }
            // Turned off on purpose, warning and all
            if !cfg.fallback_ap {
                offline = Duration::ZERO;
                continue;
            }

            let wifi_up = device_state(IFACE).await.as_deref() == Some("connected");
            let (stranded, limit, why) = match cfg.mode {
                WifiMode::Client => (
                    !wifi_up,
                    JOIN_TIMEOUT,
                    Fallback::Join(cfg.client.ssid.clone()),
                ),
                _ => (
                    !wifi_up && !ethernet_up().await,
                    STRANDED_TIMEOUT,
                    Fallback::Stranded,
                ),
            };
            if !stranded {
                offline = Duration::ZERO;
                continue;
            }
            offline += WATCH_TICK;
            if offline >= limit {
                self.bring_up_fallback(generation, &cfg, why).await;
                offline = Duration::ZERO;
            }
        }
    }

    /// NetworkManager keeps the AP up across our restarts; without this it
    /// would run on unseen and never be taken down
    async fn adopt_fallback(&self) {
        let cfg = self.wifi.lock().unwrap().clone();
        if cfg.mode == WifiMode::Ap {
            return;
        }
        if !ap_active().await {
            return;
        }
        eprintln!("[net] fallback access point still up from before, keeping track of it");
        *self.fallback.lock().unwrap() = Some(match cfg.mode {
            WifiMode::Client => Fallback::Join(cfg.client.ssid.clone()),
            _ => Fallback::Stranded,
        });
    }

    async fn take_down_fallback(&self, generation: u64) {
        let _guard = self.apply_lock.lock().await;
        if self.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        eprintln!("[net] Ethernet is back and nobody uses the fallback access point, stopping it");
        let _ = run("nmcli", &["con", "down", AP_CON]).await;
        *self.fallback.lock().unwrap() = None;
    }

    async fn bring_up_fallback(&self, generation: u64, cfg: &WifiSettings, why: Fallback) {
        if self.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        match &why {
            Fallback::Join(ssid) => {
                eprintln!("[net] couldn't join \"{ssid}\", starting fallback access point")
            }
            Fallback::Stranded => {
                eprintln!("[net] no Ethernet or WiFi, starting fallback access point")
            }
        }
        let _ = run("nmcli", &["con", "down", CLIENT_CON]).await;
        // System mode never wrote one
        if let Err(e) = write_ap_profile(cfg, false).await {
            eprintln!("[net] fallback AP failed: {e}");
            return;
        }
        match run("nmcli", &["--wait", "30", "con", "up", AP_CON]).await {
            Ok(_) => *self.fallback.lock().unwrap() = Some(why),
            Err(e) => {
                eprintln!("[net] fallback AP failed: {e}");
                *self.last_error.lock().unwrap() =
                    Some(format!("Fallback access point failed: {e}"));
            }
        }
    }

    /// Make what's running match the settings at startup. The settings can
    /// be ahead of NetworkManager: a fresh image has no profiles at all, and
    /// an AP profile left behind by the fallback doesn't autoconnect
    pub async fn ensure_applied(self: &Arc<Self>, store: &crate::settings::SettingsStore) {
        let s = store.get();
        prepare_radio(&s.wifi).await;
        let Ok(out) = run("nmcli", &["-t", "-f", "NAME", "con", "show"]).await else {
            return;
        };
        let has = |name: &str| out.lines().any(|l| l == name);

        match s.wifi.mode {
            WifiMode::Ap => {
                let this = Arc::clone(self);
                tokio::spawn(async move {
                    // Give NetworkManager's own autoconnect a chance at boot
                    for _ in 0..2 {
                        if ap_active().await {
                            return;
                        }
                        tokio::time::sleep(AP_START_GRACE).await;
                    }
                    eprintln!("[net] access point should be on but isn't, starting it");
                    this.apply_later(s.wifi);
                });
            }
            WifiMode::Client if !has(CLIENT_CON) => {
                eprintln!("[net] setting up WiFi for the first time");
                self.apply_later(s.wifi.clone());
            }
            _ => {}
        }

        if s.ethernet.mode == EthernetMode::System || has(ETH_CON) {
            return;
        }
        if crate::settings::is_image() {
            eprintln!("[net] setting up Ethernet for the first time");
            self.apply_ethernet_later(s.ethernet.clone());
        } else {
            // Written out with defaults when another section was saved, never
            // applied: what really runs is the OS's own setup
            eprintln!("[net] Ethernet settings were never applied, showing them as System");
            if let Err(e) = store.update(|s| s.ethernet.mode = EthernetMode::System) {
                eprintln!("[net] {e}");
            }
        }
    }

    /// Like apply_later, for the wired side
    pub fn apply_ethernet_later(self: &Arc<Self>, cfg: EthernetSettings) {
        let this = Arc::clone(self);
        self.applying.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(750)).await;
            let result = this.apply_ethernet(&cfg).await;
            *this.last_error.lock().unwrap() = result.err();
            this.applying.store(false, Ordering::SeqCst);
        });
    }

    async fn apply_ethernet(&self, cfg: &EthernetSettings) -> Result<(), String> {
        let _guard = self.apply_lock.lock().await;
        let dev = wired_port()
            .await
            .ok_or("This device has no Ethernet port")?;
        delete_con(ETH_CON).await;
        if cfg.mode == EthernetMode::System {
            // Let NetworkManager pick the OS's own profile again
            let _ = run("nmcli", &["dev", "connect", &dev]).await;
            return Ok(());
        }
        let mut args = vec![
            "con",
            "add",
            "type",
            "ethernet",
            "ifname",
            &dev,
            "con-name",
            ETH_CON,
            "autoconnect",
            "yes",
            // Above NetworkManager's auto-created "Wired connection 1"
            "connection.autoconnect-priority",
            "100",
        ];
        let dns = cfg.dns_list().join(" ");
        match cfg.mode {
            EthernetMode::System => unreachable!("handled above"),
            EthernetMode::Auto => args.extend(["ipv4.method", "auto"]),
            EthernetMode::Static => {
                args.extend(["ipv4.method", "manual", "ipv4.addresses", &cfg.address]);
                if !cfg.gateway.is_empty() {
                    args.extend(["ipv4.gateway", &cfg.gateway]);
                }
                if !dns.is_empty() {
                    args.extend(["ipv4.dns", &dns]);
                }
            }
        }
        run("nmcli", &args).await?;
        match run("nmcli", &["--wait", "20", "con", "up", ETH_CON]).await {
            Ok(_) => Ok(()),
            // Saved anyway: it connects by itself once a cable is plugged in
            Err(_) if !ethernet_has_carrier(&dev).await => Ok(()),
            Err(e) => Err(format!("Ethernet: {e}")),
        }
    }

    pub async fn status(&self) -> NetworkStatus {
        let wifi = interface_status(IFACE).await;
        let ethernet = match wired_port().await {
            Some(dev) => Some(interface_status(&dev).await),
            None => None,
        };
        let tether = match tethered_phone().await {
            Some(dev) => {
                let mut st = interface_status(&dev).await;
                st.product = usb_product(&dev).await;
                Some(st)
            }
            None => None,
        };
        let fallback = self.fallback.lock().unwrap().clone();
        // The AP holds wlan0, so a failed join can't fix itself; Ethernet can
        let fallback_resolved = matches!(fallback, Some(Fallback::Stranded))
            && ethernet.as_ref().is_some_and(|e| e.state == "connected");
        let fallback_reason = fallback.map(|f| match f {
            Fallback::Join(ssid) => {
                format!("Couldn't join \u{201c}{ssid}\u{201d}, so the access point started.")
            }
            Fallback::Stranded if fallback_resolved => "Ethernet is back. The access point \
                 turns off once nobody has used it for a moment."
                .to_string(),
            Fallback::Stranded => "No Ethernet or WiFi, so the access point started.".to_string(),
        });
        NetworkStatus {
            wifi,
            ethernet,
            tether,
            internet_via: default_route_device().await,
            iphone_support: run("which", &["usbmuxd"]).await.is_ok(),
            image: crate::settings::is_image(),
            fallback_active: self.fallback_active(),
            fallback_reason,
            fallback_resolved,
            applying: self.applying.load(Ordering::SeqCst),
            last_error: self.last_error.lock().unwrap().clone(),
        }
    }
}

async fn write_ap_profile(cfg: &WifiSettings, autoconnect: bool) -> Result<(), String> {
    delete_con(AP_CON).await;
    let channel = cfg.ap.channel.to_string();
    let address = format!("{}/24", cfg.ap.address);
    let autoconnect = if autoconnect { "yes" } else { "no" };
    run(
        "nmcli",
        &[
            "con",
            "add",
            "type",
            "wifi",
            "ifname",
            IFACE,
            "con-name",
            AP_CON,
            "autoconnect",
            autoconnect,
            "connection.autoconnect-priority",
            "100",
            "ssid",
            &cfg.ap.ssid,
            "802-11-wireless.mode",
            "ap",
            // bg = 2.4 GHz only
            "802-11-wireless.band",
            "bg",
            "802-11-wireless.channel",
            &channel,
            // 2 = disable power save; it adds latency spikes to the video
            "802-11-wireless.powersave",
            "2",
            // shared = NetworkManager runs DHCP (dnsmasq) + NAT for clients
            "ipv4.method",
            "shared",
            "ipv4.addresses",
            &address,
            "ipv6.method",
            "disabled",
            "wifi-sec.key-mgmt",
            "wpa-psk",
            // WPA2-only CCMP: the Pi's brcmfmac AP is flaky with TKIP/WPA1
            "wifi-sec.proto",
            "rsn",
            "wifi-sec.pairwise",
            "ccmp",
            "wifi-sec.group",
            "ccmp",
            "wifi-sec.psk",
            &cfg.ap.password,
        ],
    )
    .await
    .map(|_| ())
}

async fn write_client_profile(cfg: &WifiSettings) -> Result<(), String> {
    delete_con(CLIENT_CON).await;
    let mut args = vec![
        "con",
        "add",
        "type",
        "wifi",
        "ifname",
        IFACE,
        "con-name",
        CLIENT_CON,
        "autoconnect",
        "yes",
        "connection.autoconnect-priority",
        "50",
        "ssid",
        cfg.client.ssid.as_str(),
        "802-11-wireless.powersave",
        "2",
        // 5 GHz is where the goggles' video link lives
        "802-11-wireless.band",
        "bg",
    ];
    if !cfg.client.password.is_empty() {
        args.extend([
            "wifi-sec.key-mgmt",
            "wpa-psk",
            "wifi-sec.psk",
            cfg.client.password.as_str(),
        ]);
    }
    run("nmcli", &args).await.map(|_| ())
}

/// The radio's legal limits depend on the country, and Raspberry Pi OS keeps
/// it switched off until one is set. On the image nothing else will ever
/// switch it on
async fn prepare_radio(cfg: &WifiSettings) {
    if !cfg.country.is_empty()
        && let Err(e) = run("iw", &["reg", "set", &cfg.country]).await
    {
        eprintln!("[network] couldn't set the WiFi country: {e}");
    }
    if (crate::settings::is_image() || cfg.mode != WifiMode::Unmanaged)
        && let Err(e) = run("nmcli", &["radio", "wifi", "on"]).await
    {
        eprintln!("[network] couldn't switch the WiFi radio on: {e}");
    }
}

#[derive(Serialize)]
pub struct Countries {
    /// What the radio follows right now, if any country at all
    current: Option<String>,
    countries: Vec<Country>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Country {
    code: String,
    name: String,
}

pub async fn countries() -> Countries {
    let current = run("iw", &["reg", "get"])
        .await
        .ok()
        .and_then(|out| current_country(&out));
    let list = std::fs::read_to_string("/usr/share/zoneinfo/iso3166.tab").unwrap_or_default();
    Countries {
        current,
        countries: parse_countries(&list),
    }
}

/// From `iw reg get`, whose first block is the one in force; "00" is the
/// worldwide fallback rather than a country
fn current_country(reg: &str) -> Option<String> {
    let line = reg.lines().find(|l| l.starts_with("country "))?;
    let code = line.strip_prefix("country ")?.split(':').next()?.trim();
    (code != "00" && code.len() == 2).then(|| code.to_string())
}

/// tzdata's ISO 3166 table: `code<TAB>name`, with `#` comments
fn parse_countries(table: &str) -> Vec<Country> {
    let mut countries: Vec<Country> = table
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(code, name)| Country {
            code: code.to_string(),
            name: name.trim().to_string(),
        })
        .collect();
    countries.sort_by(|a, b| a.name.cmp(&b.name));
    countries
}

async fn delete_con(name: &str) {
    let _ = run("nmcli", &["con", "delete", name]).await;
}

pub async fn scan() -> Result<Vec<ScanResult>, String> {
    let out = run(
        "nmcli",
        &[
            "-t",
            "-f",
            "SSID,SIGNAL,SECURITY,CHAN,FREQ",
            "dev",
            "wifi",
            "list",
            "ifname",
            IFACE,
            "--rescan",
            "yes",
        ],
    )
    .await?;
    let mut results: Vec<ScanResult> = Vec::new();
    for line in out.lines() {
        let f = split_terse(line);
        if f.len() < 5 || f[0].is_empty() {
            continue; // hidden networks
        }
        let freq: u32 = f[4]
            .split_whitespace()
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        // Never joined on 5 GHz, so those aren't offered
        if freq >= 5000 {
            continue;
        }
        let r = ScanResult {
            ssid: f[0].clone(),
            signal: f[1].parse().unwrap_or(0),
            security: f[2].clone(),
            channel: f[3].parse().unwrap_or(0),
        };
        // One row per SSID: keep the strongest BSS
        match results.iter_mut().find(|x| x.ssid == r.ssid) {
            Some(existing) if existing.signal >= r.signal => {}
            Some(existing) => *existing = r,
            None => results.push(r),
        }
    }
    results.sort_by_key(|r| std::cmp::Reverse(r.signal));
    Ok(results)
}

async fn device_state(dev: &str) -> Option<String> {
    let out = run("nmcli", &["-g", "GENERAL.STATE", "dev", "show", dev])
        .await
        .ok()?;
    // "100 (connected)" -> "connected"
    let s = out.trim();
    Some(
        s.split_once('(')
            .map(|(_, rest)| rest.trim_end_matches(')').to_string())
            .unwrap_or_else(|| s.to_string()),
    )
}

/// A phone counts on purpose: it reaches this page, but nothing else does,
/// so the fallback AP still comes up for laptops and other phones
async fn ethernet_up() -> bool {
    match wired_port().await {
        Some(dev) => device_state(&dev).await.as_deref() == Some("connected"),
        None => false,
    }
}

async fn ap_active() -> bool {
    run("nmcli", &["-g", "GENERAL.CONNECTION", "dev", "show", IFACE])
        .await
        .is_ok_and(|c| c.trim() == AP_CON)
}

async fn ap_unused() -> bool {
    run("iw", &["dev", IFACE, "station", "dump"])
        .await
        .is_ok_and(|out| out.trim().is_empty())
}

async fn ethernet_has_carrier(dev: &str) -> bool {
    tokio::fs::read_to_string(format!("/sys/class/net/{dev}/carrier"))
        .await
        .is_ok_and(|c| c.trim() == "1")
}

async fn ethernet_devices() -> Vec<String> {
    let Ok(out) = run("nmcli", &["-t", "-f", "DEVICE,TYPE", "dev"]).await else {
        return Vec::new();
    };
    out.lines()
        .filter_map(|l| {
            let f = split_terse(l);
            (f.len() >= 2 && f[1] == "ethernet").then(|| f[0].clone())
        })
        .collect()
}

async fn is_tether(dev: &str) -> bool {
    tokio::fs::read_link(format!("/sys/class/net/{dev}/device/driver"))
        .await
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .is_some_and(|d| TETHER_DRIVERS.contains(&d.as_str()))
}

/// The built-in port (or a USB Ethernet adapter), never a phone
async fn wired_port() -> Option<String> {
    for dev in ethernet_devices().await {
        if !is_tether(&dev).await {
            return Some(dev);
        }
    }
    None
}

async fn tethered_phone() -> Option<String> {
    for dev in ethernet_devices().await {
        if is_tether(&dev).await {
            return Some(dev);
        }
    }
    None
}

/// The interface's sysfs device is the USB interface; the name lives on
/// its parent, the USB device
async fn usb_product(dev: &str) -> Option<String> {
    let p = tokio::fs::read_to_string(format!("/sys/class/net/{dev}/device/../product"))
        .await
        .ok()?;
    Some(p.trim().to_string()).filter(|p| !p.is_empty())
}

async fn default_route_device() -> Option<String> {
    let out = run("ip", &["-j", "route", "show", "default"]).await.ok()?;
    let routes: Vec<serde_json::Value> = serde_json::from_str(&out).ok()?;
    routes
        .iter()
        .min_by_key(|r| r["metric"].as_u64().unwrap_or(0))
        .and_then(|r| r["dev"].as_str())
        .map(str::to_string)
}

async fn interface_status(dev: &str) -> InterfaceStatus {
    let mut st = InterfaceStatus {
        device: dev.to_string(),
        state: device_state(dev)
            .await
            .unwrap_or_else(|| "unavailable".into()),
        ..Default::default()
    };
    if let Ok(out) = run(
        "nmcli",
        &[
            "-t",
            "-f",
            "GENERAL.CONNECTION,IP4.ADDRESS,IP4.GATEWAY",
            "dev",
            "show",
            dev,
        ],
    )
    .await
    {
        for line in out.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            if value.is_empty() || value == "--" {
                continue;
            }
            if key == "GENERAL.CONNECTION" {
                st.connection = Some(value.to_string());
            } else if key == "IP4.GATEWAY" {
                st.gateway = Some(value.to_string());
            } else if key.starts_with("IP4.ADDRESS") {
                st.addresses.push(value.to_string());
            }
        }
    }
    if let Some(con) = &st.connection
        && let Ok(out) = run("nmcli", &["-g", "ipv4.method", "con", "show", con]).await
    {
        st.method = Some(out.trim().to_string()).filter(|m| !m.is_empty());
    }
    if let Some(con) = st.connection.clone()
        && let Ok(out) = run(
            "nmcli",
            &[
                "-g",
                "802-11-wireless.mode,802-11-wireless.ssid",
                "con",
                "show",
                &con,
            ],
        )
        .await
    {
        let mut lines = out.lines();
        st.role = lines.next().map(|m| {
            if m == "ap" {
                "ap".into()
            } else {
                "client".into()
            }
        });
        st.ssid = lines.next().filter(|s| !s.is_empty()).map(str::to_string);
    }
    st
}

/// Split one line of `nmcli -t` output, honouring `\:` escapes
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    fields.last_mut().unwrap().push(n);
                }
            }
            ':' => fields.push(String::new()),
            _ => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// Run a command, returning stdout, or a one-line error built from stderr
pub async fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let fut = Command::new(program).args(args).kill_on_drop(true).output();
    let out = match tokio::time::timeout(Duration::from_secs(45), fut).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("{program}: {e}")),
        Err(_) => return Err(format!("{program} timed out")),
    };
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = stderr
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("failed")
            .trim_start_matches("Error: ")
            .to_string();
        Err(msg)
    }
}

#[cfg(test)]
#[path = "network.test.rs"]
mod tests;
