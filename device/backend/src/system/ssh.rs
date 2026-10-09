use std::path::Path;
use std::process::Stdio;

use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const UNIT: &str = "ssh.service";
const SOCKET: &str = "ssh.socket";
/// The account the image is made with, and its password until it's changed
const IMAGE_USER: &str = "ystreamer";
const MIN_PASSWORD: usize = 8;

#[derive(Serialize)]
pub struct SshStatus {
    pub available: bool,
    pub enabled: bool,
    pub user: Option<String>,
    pub default_password: bool,
}

/// stdout of a command whatever its exit code: `is-enabled` answers
/// "disabled" with a failure
async fn output(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// The first ordinary account, which is the one people log in with
fn login_user(passwd: &str) -> Option<String> {
    passwd.lines().find_map(|line| {
        let mut f = line.split(':');
        let (name, _, uid) = (f.next()?, f.next()?, f.next()?);
        (uid == "1000").then(|| name.to_string())
    })
}

fn changed_marker(settings: &Path) -> std::path::PathBuf {
    settings.with_file_name("ssh-password-changed")
}

pub async fn status(settings: &Path) -> SshStatus {
    let state = output("systemctl", &["is-enabled", UNIT]).await;
    let user = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|p| login_user(&p));
    SshStatus {
        available: crate::settings::is_image() && !state.is_empty() && state != "not-found",
        // Off only counts once it has stopped too
        enabled: state == "enabled" || output("systemctl", &["is-active", UNIT]).await == "active",
        default_password: user.as_deref() == Some(IMAGE_USER) && !changed_marker(settings).exists(),
        user,
    }
}

pub async fn set_enabled(on: bool) -> Result<(), String> {
    use crate::net::network::run;

    let action: &[&str] = if on {
        &["enable", "start"]
    } else {
        &["disable", "stop"]
    };
    let mut complaint = None;
    for verb in action {
        if let Err(e) = run("systemctl", &[verb, UNIT]).await {
            eprintln!("[ssh] systemctl {verb} {UNIT}: {e}");
            complaint.get_or_insert(e);
        }
        // Not every system has the socket, so it failing means nothing
        if !on {
            let _ = run("systemctl", &[verb, SOCKET]).await;
        }
    }

    let enabled = output("systemctl", &["is-enabled", UNIT]).await == "enabled";
    let running = output("systemctl", &["is-active", UNIT]).await == "active";
    println!(
        "[ssh] asked for {}, now {} at boot and {}",
        if on { "on" } else { "off" },
        if enabled { "enabled" } else { "disabled" },
        if running { "running" } else { "stopped" },
    );
    if enabled == on && running == on {
        return Ok(());
    }
    Err(complaint.unwrap_or_else(|| {
        if on {
            "SSH didn't start".into()
        } else {
            "SSH is still running".into()
        }
    }))
}

fn check_password(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(format!("Use at least {MIN_PASSWORD} characters"));
    }
    if password.len() > 256 || password.chars().any(char::is_control) {
        return Err("That password can't be used".into());
    }
    Ok(())
}

pub async fn set_password(settings: &Path, password: &str) -> Result<(), String> {
    check_password(password)?;
    let user = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|p| login_user(&p))
        .ok_or("There's no user account to set a password for")?;

    // On stdin, so the password never shows in the process list
    let mut child = Command::new("chpasswd")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("chpasswd: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{user}:{password}\n").as_bytes())
            .await
            .map_err(|e| format!("chpasswd: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| format!("chpasswd: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(why
            .lines()
            .last()
            .unwrap_or("Couldn't set the password")
            .to_string());
    }
    let _ = std::fs::write(changed_marker(settings), "");
    Ok(())
}

#[cfg(test)]
#[path = "ssh.test.rs"]
mod tests;
