use std::fmt::Write;

use base64::Engine;

use crate::input::goggles::LinkState;
use crate::net::network::{InterfaceStatus, Network, NetworkStatus};
use crate::settings::{InputMode, Scaling, SettingsStore, WifiMode, WifiSettings};

pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;

const LOGO: &[u8] = include_bytes!("../../../assets/logo.svg");
const LOGO_MIME: &str = "image/svg+xml";

const BASE: &str = "#0b101a";
const LINE: &str = "#273142";
const FG: &str = "#f1f5f9";
const MUTED: &str = "#94a3b8";
const FAINT: &str = "#64748b";
const OK_SOFT: &str = "#86efac";
const WARN_SOFT: &str = "#fdba74";
const WARN: &str = "#f97316";
const DOWN: &str = "#f87171";
const QR_LIGHT: &str = "#ffffff";

const SANS: &str = "Inter, Nimbus Sans, DejaVu Sans, sans-serif";
const MONO: &str = "DejaVu Sans Mono, monospace";

pub struct Screen {
    source: InputMode,
    goggles: LinkState,
    goggles_name: Option<String>,
    join_wifi: Option<String>,
    notice: Option<(String, &'static str)>,
    caption: &'static str,
    links: Vec<Link>,
}

struct Link {
    label: &'static str,
    value: String,
    tone: Tone,
    details: Vec<(&'static str, String)>,
}

#[derive(PartialEq)]
enum Tone {
    Address,
    Pending,
    Down,
}

impl Link {
    fn address(label: &'static str, value: String) -> Self {
        Self {
            label,
            value,
            tone: Tone::Address,
            details: Vec::new(),
        }
    }

    fn detail(mut self, label: &'static str, value: impl Into<String>) -> Self {
        self.details.push((label, value.into()));
        self
    }
}

pub async fn gather(
    web_port: u16,
    store: &SettingsStore,
    network: &Network,
    source: InputMode,
    goggles: LinkState,
    goggles_name: Option<String>,
) -> Screen {
    let settings = store.get();
    let mut screen = describe(
        web_port,
        goggles,
        &settings.wifi,
        &network.status().await,
        settings.splash.show_ap_password,
    );
    if let Some(notice) = crate::system::reset::notice() {
        screen.notice = Some((notice.to_string(), WARN));
    }
    screen.source = source;
    screen.goggles_name = goggles_name;
    screen
}

/// Tell a first-time user how to reach the web interface from where the
/// device currently is on the network
fn describe(
    web_port: u16,
    goggles: LinkState,
    wifi: &WifiSettings,
    status: &NetworkStatus,
    show_ap_password: bool,
) -> Screen {
    let host = |host: &str| match web_port {
        80 => host.to_string(),
        port => format!("{host}:{port}"),
    };
    let mut links = Vec::new();
    let mut notice = None;
    let mut join_wifi = None;

    let wifi_ip = ipv4(&status.wifi);
    let eth_ip = status.ethernet.as_ref().and_then(ipv4);
    let tether_ip = status.tether.as_ref().and_then(ipv4);

    if status.wifi.role.as_deref() == Some("ap") {
        if let Some(reason) = &status.fallback_reason {
            let tone = if status.fallback_resolved {
                MUTED
            } else {
                WARN
            };
            notice = Some((reason.clone(), tone));
        }
        let ssid = status
            .wifi
            .ssid
            .clone()
            .unwrap_or_else(|| wifi.ap.ssid.clone());
        let mut link = match &wifi_ip {
            Some(ip) => Link::address("Wi-Fi access point", host(ip)),
            None => Link {
                tone: Tone::Pending,
                ..Link::address("Wi-Fi access point", "Starting".into())
            },
        }
        .detail("Network", ssid.clone());
        // Only our own AP profile's password is known; an OS-configured one isn't
        let ours = wifi.mode == WifiMode::Ap || status.fallback_active;
        // The QR code holds the password too
        let shown = ours && show_ap_password;
        if shown {
            link = link.detail("Password", &wifi.ap.password);
        }
        links.push(link);
        if shown {
            join_wifi = Some(wifi_qr_payload(&ssid, &wifi.ap.password));
        }
    } else if let Some(ip) = &wifi_ip {
        let mut link = Link::address("Wi-Fi", host(ip));
        if let Some(ssid) = &status.wifi.ssid {
            link = link.detail("Network", ssid);
        }
        links.push(link);
    }

    if let Some(ip) = &eth_ip {
        links.push(Link::address("Ethernet", host(ip)));
    }

    // The phone sharing its connection can open this address itself
    if let Some(ip) = &tether_ip {
        let mut link = Link::address("Phone over USB", host(ip));
        if let Some(product) = status.tether.as_ref().and_then(|t| t.product.clone()) {
            link = link.detail("", product);
        }
        links.push(link);
    }

    if wifi_ip.is_some() || eth_ip.is_some() || tether_ip.is_some() {
        let name = format!("{}.local", crate::settings::current_hostname());
        links.push(Link::address("Local name", host(&name)));
        return Screen {
            source: InputMode::DjiFpv,
            goggles,
            goggles_name: None,
            join_wifi,
            notice,
            caption: "Open in a browser",
            links,
        };
    }

    let link = if wifi.mode == WifiMode::Client {
        let link = Link {
            tone: Tone::Pending,
            ..Link::address("Wi-Fi", "Connecting".into())
        }
        .detail("Network", &wifi.client.ssid);
        if wifi.fallback_ap {
            link.detail("", "Access point starts in a minute if this fails")
        } else {
            link
        }
    } else {
        let link = Link {
            tone: Tone::Down,
            ..Link::address("Wi-Fi and Ethernet", "Not connected".into())
        };
        if wifi.fallback_ap {
            link.detail("", "Access point starts in a few seconds")
        } else {
            link.detail("", "Plug in an Ethernet cable to set up")
        }
    };
    Screen {
        source: InputMode::DjiFpv,
        goggles,
        goggles_name: None,
        join_wifi,
        notice,
        caption: "Network",
        links: vec![link],
    }
}

fn wifi_qr_payload(ssid: &str, password: &str) -> String {
    let esc = |s: &str| {
        s.chars().fold(String::new(), |mut out, c| {
            if matches!(c, '\\' | ';' | ',' | ':' | '"') {
                out.push('\\');
            }
            out.push(c);
            out
        })
    };
    format!("WIFI:T:WPA;S:{};P:{};;", esc(ssid), esc(password))
}

fn qr_card(data: &str, x: u32, y: u32, size: u32) -> Option<String> {
    const QUIET: usize = 2;
    let code = qrcode::QrCode::with_error_correction_level(data, qrcode::EcLevel::M).ok()?;
    let n = code.width();
    let module = size as f64 / (n + 2 * QUIET) as f64;
    let mut path = String::new();
    for (i, color) in code.to_colors().iter().enumerate() {
        if *color == qrcode::Color::Dark {
            let _ = write!(path, "M{} {}h1v1h-1z", i % n + QUIET, i / n + QUIET);
        }
    }
    Some(format!(
        r#"<rect x="{x}" y="{y}" width="{size}" height="{size}" rx="16" fill="{QR_LIGHT}"/>
<path transform="translate({x} {y}) scale({module:.4})" d="{path}" fill="{BASE}" shape-rendering="crispEdges"/>
"#
    ))
}

fn ipv4(iface: &InterfaceStatus) -> Option<String> {
    let cidr = iface.addresses.first()?;
    Some(cidr.split('/').next().unwrap_or(cidr).to_string())
}

impl Screen {
    fn status(&self) -> (&'static str, String) {
        match (self.source, self.goggles) {
            (InputMode::DjiFpv, LinkState::Unplugged) => {
                (MUTED, "Connect the goggles to the USB-C port".to_string())
            }
            (InputMode::DjiFpv, LinkState::Connecting) => {
                (WARN_SOFT, "Goggles found, connecting".to_string())
            }
            (InputMode::DjiFpvLegacy, LinkState::Unplugged) => {
                (MUTED, "Connect the goggles to a USB-A port".to_string())
            }
            (InputMode::DjiFpvLegacy, LinkState::Connecting) => {
                (WARN_SOFT, "Goggles found, connecting".to_string())
            }
            (InputMode::Uvc, LinkState::Unplugged) => {
                (MUTED, "Connect a USB camera or receiver".to_string())
            }
            (InputMode::Uvc, LinkState::Connecting) => (
                WARN_SOFT,
                format!(
                    "{} found, starting",
                    self.goggles_name.as_deref().unwrap_or("USB camera")
                ),
            ),
            (source, LinkState::Live) => (
                OK_SOFT,
                format!(
                    "{} connected, waiting for video",
                    self.goggles_name.as_deref().unwrap_or(match source {
                        InputMode::DjiFpv | InputMode::DjiFpvLegacy => "Goggles",
                        InputMode::Uvc => "USB camera",
                    })
                ),
            ),
        }
    }

    /// The same information as `svg`, as a band along the bottom of
    /// someone's own picture
    pub fn over_image(&self, image: &str, scaling: Scaling) -> String {
        const DETAIL_STEP: u32 = 32;
        const COLUMN_MAX: u32 = 520;
        const MARGIN: u32 = 64;

        let cx = WIDTH / 2;
        let (tone, status) = self.status();
        let aspect = match scaling {
            Scaling::Fit => "xMidYMid meet",
            Scaling::Fill => "xMidYMid slice",
        };
        let notice_h = if self.notice.is_some() { 36 } else { 0 };
        let details = self
            .links
            .iter()
            .map(|l| l.details.len())
            .max()
            .unwrap_or(0) as u32;
        let band_h = 168 + notice_h + DETAIL_STEP * details;
        let top = HEIGHT - band_h;

        let mut s = String::new();
        let _ = write!(
            s,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}">
<rect width="100%" height="100%" fill="black"/>
<image width="{WIDTH}" height="{HEIGHT}" preserveAspectRatio="{aspect}" href="{image}"/>
<rect y="{top}" width="{WIDTH}" height="{band_h}" fill="{BASE}" fill-opacity="0.78"/>
<text x="{cx}" y="{status_y}" text-anchor="middle" font-family="{SANS}" font-size="28" fill="{tone}">{status}</text>
"#,
            status_y = top + 48,
            status = escape(&status),
        );
        if let Some((notice, color)) = &self.notice {
            let _ = writeln!(
                s,
                r#"<text x="{cx}" y="{}" text-anchor="middle" font-family="{SANS}" font-size="22" fill="{color}">{}</text>"#,
                top + 84,
                escape(notice),
            );
        }

        let n = self.links.len().max(1) as u32;
        let column_w = COLUMN_MAX.min((WIDTH - 2 * MARGIN) / n);
        let left = (WIDTH - column_w * n) / 2;
        let label_y = top + 88 + notice_h;
        for (i, link) in self.links.iter().enumerate() {
            let x = left + column_w * i as u32 + column_w / 2;
            let (family, color) = match link.tone {
                Tone::Address => (MONO, FG),
                Tone::Pending => (SANS, WARN),
                Tone::Down => (SANS, DOWN),
            };
            let _ = write!(
                s,
                r#"<text x="{x}" y="{label_y}" text-anchor="middle" font-family="{SANS}" font-size="16" font-weight="600" letter-spacing="3" fill="{MUTED}">{label}</text>
<text x="{x}" y="{value_y}" text-anchor="middle" font-family="{family}" font-size="30" fill="{color}">{value}</text>
"#,
                value_y = label_y + 40,
                label = escape(&link.label.to_uppercase()),
                value = escape(&link.value),
            );
            for (j, (label, value)) in link.details.iter().enumerate() {
                let y = label_y + 76 + DETAIL_STEP * j as u32;
                let _ = write!(
                    s,
                    r#"<text x="{x}" y="{y}" text-anchor="middle" font-family="{SANS}" font-size="22" fill="{MUTED}">"#
                );
                if label.is_empty() {
                    let _ = write!(s, "{}", escape(value));
                } else {
                    let _ = write!(
                        s,
                        r#"{label}  <tspan font-family="{MONO}" fill="{FG}">{}</tspan>"#,
                        escape(value),
                    );
                }
                s.push_str("</text>\n");
            }
        }
        s.push_str("</svg>");
        s
    }

    pub fn svg(&self) -> String {
        const MARGIN: u32 = 120;
        const LOGO_SIZE: u32 = 160;
        const LOGO_Y: u32 = 176;
        const RULE_Y: u32 = 752;
        const COLUMN_MAX: u32 = 560;
        const STATUS_Y: u32 = LOGO_Y + LOGO_SIZE + 216;
        const QR_SIZE: u32 = 280;

        let cx = WIDTH / 2;
        let (tone, status) = self.status();
        let logo = base64::engine::general_purpose::STANDARD.encode(LOGO);

        let mut s = String::new();
        let _ = write!(
            s,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}">
<rect width="100%" height="100%" fill="{BASE}"/>
<image x="{logo_x}" y="{LOGO_Y}" width="{LOGO_SIZE}" height="{LOGO_SIZE}" href="data:{LOGO_MIME};base64,{logo}"/>
<text x="{cx}" y="{wordmark_y}" text-anchor="middle" font-family="{SANS}" font-size="92" font-weight="600" letter-spacing="-2" fill="{FG}">YStreamer</text>
<text x="{cx}" y="{STATUS_Y}" text-anchor="middle" font-family="{SANS}" font-size="32" fill="{tone}">{status}</text>
"#,
            logo_x = cx - LOGO_SIZE / 2,
            wordmark_y = LOGO_Y + LOGO_SIZE + 124,
            status = escape(&status),
        );

        if let Some((notice, color)) = &self.notice {
            let _ = writeln!(
                s,
                r#"<text x="{cx}" y="{}" text-anchor="middle" font-family="{SANS}" font-size="28" fill="{color}">{}</text>"#,
                STATUS_Y + 64,
                escape(notice),
            );
        }

        if let Some(card) = self
            .join_wifi
            .as_deref()
            .and_then(|data| qr_card(data, WIDTH - MARGIN - QR_SIZE, LOGO_Y, QR_SIZE))
        {
            s.push_str(&card);
            let _ = writeln!(
                s,
                r#"<text x="{}" y="{}" text-anchor="middle" font-family="{SANS}" font-size="20" font-weight="600" letter-spacing="3" fill="{FAINT}">SCAN TO JOIN WI-FI</text>"#,
                WIDTH - MARGIN - QR_SIZE / 2,
                LOGO_Y + QR_SIZE + 48,
            );
        }

        // A rule across the screen, broken in the middle by the caption
        let caption = self.caption.to_uppercase();
        let gap = caption.chars().count() as u32 * 17 + 72;
        let _ = write!(
            s,
            r#"<g stroke="{LINE}" stroke-width="2">
  <line x1="{MARGIN}" y1="{RULE_Y}" x2="{left_end}" y2="{RULE_Y}"/>
  <line x1="{right_start}" y1="{RULE_Y}" x2="{right_end}" y2="{RULE_Y}"/>
</g>
<text x="{cx}" y="{caption_y}" text-anchor="middle" font-family="{SANS}" font-size="20" font-weight="600" letter-spacing="4" fill="{FAINT}">{caption}</text>
"#,
            left_end = cx - gap / 2,
            right_start = cx + gap / 2,
            right_end = WIDTH - MARGIN,
            caption_y = RULE_Y + 7,
        );

        let n = self.links.len().max(1) as u32;
        let column_w = COLUMN_MAX.min((WIDTH - 2 * MARGIN) / n);
        let left = (WIDTH - column_w * n) / 2;
        for (i, link) in self.links.iter().enumerate() {
            let i = i as u32;
            let x = left + column_w * i + column_w / 2;
            if i > 0 {
                let divider_x = left + column_w * i;
                let _ = writeln!(
                    s,
                    r#"<line x1="{divider_x}" y1="{}" x2="{divider_x}" y2="{}" stroke="{LINE}" stroke-width="2"/>"#,
                    RULE_Y + 56,
                    RULE_Y + 200,
                );
            }
            let (family, size, color) = match link.tone {
                Tone::Address => (MONO, 40, FG),
                Tone::Pending => (SANS, 42, WARN),
                Tone::Down => (SANS, 42, DOWN),
            };
            let _ = write!(
                s,
                r#"<text x="{x}" y="{label_y}" text-anchor="middle" font-family="{SANS}" font-size="20" font-weight="600" letter-spacing="3" fill="{FAINT}">{label}</text>
<text x="{x}" y="{value_y}" text-anchor="middle" font-family="{family}" font-size="{size}" fill="{color}">{value}</text>
"#,
                label_y = RULE_Y + 80,
                value_y = RULE_Y + 136,
                label = escape(&link.label.to_uppercase()),
                value = escape(&link.value),
            );
            for (j, (label, value)) in link.details.iter().enumerate() {
                let y = RULE_Y + 184 + 38 * j as u32;
                let _ = write!(
                    s,
                    r#"<text x="{x}" y="{y}" text-anchor="middle" font-family="{SANS}" font-size="26" fill="{MUTED}">"#
                );
                if label.is_empty() {
                    let _ = write!(s, "{}", escape(value));
                } else {
                    let _ = write!(
                        s,
                        r#"<tspan fill="{FAINT}">{label}</tspan>  <tspan font-family="{MONO}" fill="{FG}">{}</tspan>"#,
                        escape(value),
                    );
                }
                s.push_str("</text>\n");
            }
        }

        let _ = write!(
            s,
            r#"<text x="{}" y="{}" text-anchor="end" font-family="{SANS}" font-size="20" fill="{FAINT}">v{}</text>
</svg>"#,
            WIDTH - 48,
            HEIGHT - 40,
            env!("CARGO_PKG_VERSION"),
        );
        s
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
#[path = "screen.test.rs"]
mod tests;
