use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

const BINARY: &str = "tailscale";
const TIMEOUT: Duration = Duration::from_secs(15);
/// How long a login link stays worth waiting on
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
/// Signing in is asynchronous: the link shows up in the status a moment
/// after asking to connect
const LINK_WAIT: Duration = Duration::from_secs(8);

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    NotInstalled,
    /// Installed, but its background service isn't running
    ServiceDown,
    NeedsLogin,
    /// Signed in, waiting for an admin of the network to approve the device
    NeedsApproval,
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Serialize, Clone, Debug)]
pub struct Status {
    pub state: State,
    /// Where to sign in, while a login is under way
    pub login_url: Option<String>,
    /// e.g. `ystreamer.tail1234.ts.net`
    pub name: Option<String>,
    pub addresses: Vec<String>,
    pub network: Option<String>,
    /// Devices exchanging traffic with this one right now
    pub peers: Vec<Peer>,
    pub problems: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub name: String,
    /// Straight to the other device, as opposed to through a relay server,
    /// which adds delay
    pub direct: bool,
    pub relay: Option<String>,
}

impl Status {
    fn only(state: State) -> Self {
        Self {
            state,
            login_url: None,
            name: None,
            addresses: Vec::new(),
            network: None,
            peers: Vec::new(),
            problems: Vec::new(),
        }
    }
}

// `tailscale status --json`, the parts used here
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct Raw {
    backend_state: String,
    #[serde(rename = "AuthURL")]
    auth_url: String,
    #[serde(rename = "Self")]
    this: Option<RawNode>,
    current_tailnet: Option<RawTailnet>,
    peer: Option<std::collections::HashMap<String, RawNode>>,
    health: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct RawNode {
    host_name: String,
    #[serde(rename = "DNSName")]
    dns_name: String,
    #[serde(rename = "TailscaleIPs")]
    tailscale_ips: Option<Vec<String>>,
    cur_addr: String,
    relay: String,
    active: bool,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct RawTailnet {
    name: String,
}

enum Outcome {
    Done {
        ok: bool,
        stdout: String,
        stderr: String,
    },
    Missing,
    Failed(String),
}

async fn tailscale(args: &[&str]) -> Outcome {
    let run = Command::new(BINARY).args(args).kill_on_drop(true).output();
    match tokio::time::timeout(TIMEOUT, run).await {
        Ok(Ok(out)) => Outcome::Done {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        },
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Outcome::Missing,
        Ok(Err(e)) => Outcome::Failed(e.to_string()),
        Err(_) => Outcome::Failed("Tailscale didn't answer".into()),
    }
}

async fn command(args: &[&str]) -> Result<(), String> {
    match tailscale(args).await {
        Outcome::Done { ok: true, .. } => Ok(()),
        Outcome::Done { stderr, .. } => Err(first_line(&stderr)),
        Outcome::Missing => Err("Tailscale isn't installed".into()),
        Outcome::Failed(e) => Err(e),
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Tailscale reported an error")
        .to_string()
}

pub async fn status() -> Status {
    match tailscale(&["status", "--json"]).await {
        Outcome::Missing => Status::only(State::NotInstalled),
        Outcome::Failed(e) => Status {
            problems: vec![e],
            ..Status::only(State::ServiceDown)
        },
        // Signed out or stopped still prints the status, with a failing exit
        Outcome::Done { stdout, stderr, .. } => match serde_json::from_str::<Raw>(&stdout) {
            Ok(raw) => interpret(raw),
            Err(_) => Status {
                problems: vec![first_line(&stderr)],
                ..Status::only(State::ServiceDown)
            },
        },
    }
}

fn interpret(raw: Raw) -> Status {
    let state = match raw.backend_state.as_str() {
        "Running" => State::Connected,
        "Starting" => State::Connecting,
        "NeedsMachineAuth" => State::NeedsApproval,
        "Stopped" => State::Disconnected,
        _ => State::NeedsLogin,
    };
    let signed_in = !matches!(state, State::NeedsLogin);
    let this = raw.this.unwrap_or_default();
    let name = |node: &RawNode| {
        let dns = node.dns_name.trim_end_matches('.');
        if dns.is_empty() {
            node.host_name.clone()
        } else {
            dns.to_string()
        }
    };
    let mut peers: Vec<Peer> = raw
        .peer
        .unwrap_or_default()
        .values()
        .filter(|p| p.active)
        .map(|p| Peer {
            // The network's suffix is the same for everyone: just the device
            name: name(p).split('.').next().unwrap_or_default().to_string(),
            direct: !p.cur_addr.is_empty(),
            relay: Some(p.relay.clone()).filter(|r| !r.is_empty()),
        })
        .collect();
    peers.sort_by(|a, b| a.name.cmp(&b.name));
    Status {
        state,
        login_url: Some(raw.auth_url).filter(|u| !u.is_empty() && !signed_in),
        name: Some(name(&this)).filter(|n| !n.is_empty() && signed_in),
        addresses: if state == State::Connected {
            this.tailscale_ips.unwrap_or_default()
        } else {
            Vec::new()
        },
        network: raw
            .current_tailnet
            .map(|t| t.name)
            .filter(|n| !n.is_empty() && signed_in),
        peers: if state == State::Connected {
            peers
        } else {
            Vec::new()
        },
        problems: raw.health.unwrap_or_default(),
    }
}

pub async fn connect(hostname: &str) -> Result<Status, String> {
    let before = status().await;
    match before.state {
        State::NotInstalled => return Err("Tailscale isn't installed".into()),
        State::ServiceDown => return Err("Tailscale's service isn't running".into()),
        State::Disconnected => {
            command(&["up"]).await?;
            return Ok(status().await);
        }
        State::Connected | State::Connecting => return Ok(before),
        State::NeedsLogin | State::NeedsApproval => {}
    }

    // Returns only once signed in, so it's left waiting in the background
    let mut child = Command::new(BINARY)
        .args(["up", "--reset", &format!("--hostname={hostname}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Couldn't start Tailscale: {e}"))?;
    tokio::spawn(async move {
        let _ = tokio::time::timeout(LOGIN_TIMEOUT, child.wait()).await;
    });

    let deadline = tokio::time::Instant::now() + LINK_WAIT;
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let now = status().await;
        let waiting = now.state == State::NeedsLogin && now.login_url.is_none();
        if !waiting || tokio::time::Instant::now() >= deadline {
            return Ok(now);
        }
    }
}

pub async fn disconnect() -> Result<Status, String> {
    command(&["down"]).await?;
    Ok(status().await)
}

pub async fn logout() -> Result<Status, String> {
    command(&["logout"]).await?;
    Ok(status().await)
}

#[cfg(test)]
#[path = "tailscale.test.rs"]
mod tests;
