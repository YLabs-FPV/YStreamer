use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io::Write};

use serde::Serialize;

use crate::net::network::run;

/// Both NetworkManager's profile and the network interface: the import
/// names them after the file
const NAME: &str = "ystreamer-wg";
const MAX_CONFIG_BYTES: usize = 64 * 1024;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    None,
    Off,
    On,
}

#[derive(Serialize, Clone, Debug)]
pub struct Status {
    pub state: State,
    /// This device's address inside the tunnel
    pub address: Option<String>,
    pub peers: Vec<Peer>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub endpoint: Option<String>,
    pub allowed_ips: Vec<String>,
    /// Seconds since the server last answered; None if it never has. The
    /// tunnel counts as "up" either way: WireGuard has no connection as such
    pub handshake_age_secs: Option<u64>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    /// Without it, a server can't reach this device once the line's been
    /// quiet, since the device sits behind a router
    pub keepalive: bool,
}

pub async fn status() -> Status {
    let shown = run(
        "nmcli",
        &[
            "-t",
            "-f",
            "GENERAL.STATE,IP4.ADDRESS",
            "connection",
            "show",
            NAME,
        ],
    )
    .await;
    let Ok(shown) = shown else {
        return Status {
            state: State::None,
            address: None,
            peers: Vec::new(),
        };
    };
    let field = |name: &str| {
        shown.lines().find_map(|l| {
            l.strip_prefix(name)?
                .split_once(':')
                .map(|(_, v)| v.to_string())
        })
    };
    if field("GENERAL.STATE").as_deref() != Some("activated") {
        return Status {
            state: State::Off,
            address: None,
            peers: Vec::new(),
        };
    }
    // Needs the wireguard tools; without them the tunnel still works
    let peers = match run("wg", &["show", NAME, "dump"]).await {
        Ok(dump) => parse_dump(&dump, now()),
        Err(_) => Vec::new(),
    };
    Status {
        state: State::On,
        address: field("IP4.ADDRESS"),
        peers,
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `wg show <interface> dump`: the interface's line, then one per peer with
/// public key, preshared key, endpoint, allowed IPs, latest handshake,
/// received, sent, keepalive
fn parse_dump(dump: &str, now: u64) -> Vec<Peer> {
    dump.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 8 {
                return None;
            }
            let handshake: u64 = f[4].parse().unwrap_or(0);
            Some(Peer {
                endpoint: Some(f[2].to_string()).filter(|e| e != "(none)"),
                allowed_ips: f[3]
                    .split(',')
                    .filter(|a| !a.is_empty() && *a != "(none)")
                    .map(str::to_string)
                    .collect(),
                handshake_age_secs: (handshake != 0).then(|| now.saturating_sub(handshake)),
                rx_bytes: f[5].parse().unwrap_or(0),
                tx_bytes: f[6].parse().unwrap_or(0),
                keepalive: f[7] != "off",
            })
        })
        .collect()
}

/// Catches a wrong file before NetworkManager's less friendly refusal
fn check(config: &str) -> Result<(), String> {
    let has = |needle: &str| {
        config
            .lines()
            .any(|l| l.trim().to_ascii_lowercase().starts_with(needle))
    };
    if !has("[interface]") || !has("privatekey") {
        return Err(
            "That isn't a WireGuard configuration: it has no [Interface] with a PrivateKey".into(),
        );
    }
    if !has("[peer]") || !has("publickey") {
        return Err("The configuration has no [Peer], so there's nothing to connect to".into());
    }
    Ok(())
}

/// Replace whatever was imported before with this `.conf`, and bring it up
pub async fn import(config: &[u8]) -> Result<Status, String> {
    if config.len() > MAX_CONFIG_BYTES {
        return Err("That file is too large for a WireGuard configuration".into());
    }
    let text = std::str::from_utf8(config).map_err(|_| "That file isn't text".to_string())?;
    check(text)?;

    // It holds a private key: a directory only we can enter, gone right after
    let dir = std::env::temp_dir().join(format!("ystreamer-wg-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| e.to_string())?;
    let path = dir.join(format!("{NAME}.conf"));
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut f| f.write_all(config));
    let result = match written {
        Ok(()) => {
            let _ = run("nmcli", &["connection", "delete", NAME]).await;
            let path = path.to_string_lossy();
            run(
                "nmcli",
                &["connection", "import", "type", "wireguard", "file", &path],
            )
            .await
        }
        Err(e) => Err(e.to_string()),
    };
    let _ = fs::remove_dir_all(&dir);
    result.map_err(|e| format!("Couldn't use that configuration: {e}"))?;

    // The import usually brings it up by itself
    let _ = run("nmcli", &["connection", "up", NAME]).await;
    Ok(status().await)
}

pub async fn connect() -> Result<Status, String> {
    run(
        "nmcli",
        &[
            "connection",
            "modify",
            NAME,
            "connection.autoconnect",
            "yes",
        ],
    )
    .await?;
    run("nmcli", &["connection", "up", NAME]).await?;
    Ok(status().await)
}

/// Stays off across reboots too, until connected again
pub async fn disconnect() -> Result<Status, String> {
    run(
        "nmcli",
        &["connection", "modify", NAME, "connection.autoconnect", "no"],
    )
    .await?;
    let _ = run("nmcli", &["connection", "down", NAME]).await;
    Ok(status().await)
}

pub async fn remove() -> Result<Status, String> {
    run("nmcli", &["connection", "delete", NAME]).await?;
    Ok(status().await)
}

#[cfg(test)]
#[path = "wireguard.test.rs"]
mod tests;
