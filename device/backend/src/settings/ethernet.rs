use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EthernetMode {
    #[default]
    System,
    Auto,
    Static,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct EthernetSettings {
    pub mode: EthernetMode,
    pub address: String,
    /// Empty for a network without a router
    pub gateway: String,
    /// Comma or space separated; empty for none
    pub dns: String,
}

impl EthernetSettings {
    pub fn dns_list(&self) -> Vec<String> {
        self.dns
            .split([',', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.mode != EthernetMode::Static {
            return Ok(());
        }
        let (ip, prefix) = self
            .address
            .trim()
            .split_once('/')
            .ok_or("Ethernet address needs a prefix, e.g. 192.168.1.50/24")?;
        let ip: std::net::Ipv4Addr = ip
            .parse()
            .map_err(|_| format!("\"{ip}\" isn't an IPv4 address"))?;
        let prefix: u8 = prefix
            .parse()
            .ok()
            .filter(|p| (1..=32).contains(p))
            .ok_or("Ethernet prefix must be 1-32, e.g. /24")?;
        if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() || ip.is_loopback() {
            return Err(format!("{ip} can't be used as the device's address"));
        }
        if !self.gateway.is_empty() {
            let gw: std::net::Ipv4Addr = self
                .gateway
                .trim()
                .parse()
                .map_err(|_| format!("Gateway \"{}\" isn't an IPv4 address", self.gateway))?;
            let mask = u32::MAX.checked_shl(32 - prefix as u32).unwrap_or(0);
            if u32::from(gw) & mask != u32::from(ip) & mask {
                return Err(format!("Gateway {gw} isn't on the {ip}/{prefix} network"));
            }
        }
        for dns in self.dns_list() {
            dns.parse::<std::net::Ipv4Addr>()
                .map_err(|_| format!("DNS server \"{dns}\" isn't an IPv4 address"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "ethernet.test.rs"]
mod tests;
