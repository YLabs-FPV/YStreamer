use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WifiMode {
    #[default]
    Unmanaged,
    Client,
    Ap,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct WifiSettings {
    pub mode: WifiMode,
    pub client: WifiClientSettings,
    pub ap: WifiApSettings,
    pub fallback_ap: bool,
    /// Two-letter code whose radio rules apply; empty leaves it to the system
    pub country: String,
}

impl Default for WifiSettings {
    fn default() -> Self {
        Self {
            mode: WifiMode::Unmanaged,
            client: WifiClientSettings::default(),
            ap: WifiApSettings::default(),
            fallback_ap: true,
            country: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct WifiClientSettings {
    pub ssid: String,
    /// Empty for open networks
    pub password: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct WifiApSettings {
    pub ssid: String,
    pub password: String,
    /// 2.4 GHz channel, 1-11
    pub channel: u8,
    /// Address of the device on the AP network (/24; clients get DHCP)
    pub address: String,
}

impl Default for WifiApSettings {
    fn default() -> Self {
        Self {
            ssid: "YStreamer".into(),
            password: "ystreamer".into(),
            channel: 6,
            address: "10.0.0.1".into(),
        }
    }
}

impl WifiSettings {
    pub fn validate(&self) -> Result<(), String> {
        let code = self.country.as_bytes();
        if !(code.is_empty() || code.len() == 2 && code.iter().all(u8::is_ascii_uppercase)) {
            return Err("WiFi country must be a two-letter code like DE".into());
        }
        if self.mode == WifiMode::Client {
            check_ssid(&self.client.ssid, "Network name")?;
            if !self.client.password.is_empty() {
                check_psk(&self.client.password)?;
            }
        }
        // Also checked when it's only the fallback: a bad value would surface
        // just when the AP is needed, e.g. in the field
        if self.mode != WifiMode::Ap && !self.fallback_ap {
            return Ok(());
        }
        check_ssid(&self.ap.ssid, "Access point name")?;
        check_psk(&self.ap.password)?;
        if !(1..=11).contains(&self.ap.channel) {
            return Err("Channel must be between 1 and 11".into());
        }
        let ip: std::net::Ipv4Addr = self.ap.address.parse().map_err(|_| {
            format!(
                "Access point address \"{}\" isn't an IPv4 address like 10.0.0.1",
                self.ap.address
            )
        })?;
        if !ip.is_private() {
            return Err(
                "Access point address must be private: 10.x.x.x, 172.16-31.x.x or 192.168.x.x"
                    .into(),
            );
        }
        if matches!(ip.octets()[3], 0 | 255) {
            return Err(format!(
                "Access point address can't end in .0 or .255, e.g. {}.1",
                ip.octets()[..3]
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            ));
        }
        Ok(())
    }
}

fn check_ssid(s: &str, what: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > 32 {
        return Err(format!("{what} must be 1-32 bytes"));
    }
    Ok(())
}

fn check_psk(p: &str) -> Result<(), String> {
    if p.len() < 8 || p.len() > 63 || !p.is_ascii() {
        return Err("WiFi password must be 8-63 ASCII characters".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "wifi.test.rs"]
mod tests;
