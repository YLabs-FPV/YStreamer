use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::net::network::run;
use crate::settings::SettingsStore;

/// The private half signs releases and never comes near the repository
const PUBLIC_KEY: &str = include_str!("../../../packaging/update-key.pub");
/// Ours rather than the host of the day, since devices in the field keep
/// asking here whatever the releases move to
const MANIFEST_URL: &str = "https://ystreamer.yarosfpv.com/update/manifest.json";
/// Points a test at a release served from somewhere else
const MANIFEST_URL_ENV: &str = "YSTREAMER_UPDATE_URL";
const MAX_PACKAGE_BYTES: u64 = 64 << 20;
const PACKAGE: &str = "ystreamer";
/// Long enough after starting for the network to be up
const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// The newest release the update server has told us about
static LATEST: Mutex<Option<Release>> = Mutex::new(None);
const DIR: &str = "/var/lib/ystreamer/update";
/// Runs the install outside this service, which the package restarts
/// partway through
const UNIT: &str = "ystreamer-update";

const STAGED: &str = "staged.deb";
const CURRENT: &str = "current.deb";
const PREVIOUS: &str = "previous.deb";
const LOG: &str = "log";
const RESULT: &str = "result";
const ATTEMPT: &str = "attempt";
/// In `result` when the new version never answered: the old one is back,
/// or there was none to go back to
const REVERTED: &str = "reverted";
const UNANSWERED: &str = "unanswered";
const HEALTH_TRIES: u32 = 30;

/// Where the web interface listens, for asking the new version if it's up
pub static PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

#[derive(Serialize)]
pub struct Status {
    current: &'static str,
    /// Uploaded and verified, waiting to be installed
    staged: Option<String>,
    /// What "roll back" would install
    previous: Option<String>,
    /// A newer release, once a check has found one
    available: Option<Release>,
    installing: bool,
    failure: Option<Failure>,
}

#[derive(Serialize)]
struct Failure {
    version: String,
    /// It installed but never answered, and the version before is back on
    reverted: bool,
    log: String,
}

/// What a release publishes for devices to find it
#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    /// By Debian architecture
    packages: std::collections::HashMap<String, Download>,
}

#[derive(Deserialize)]
struct Download {
    url: String,
    signature: String,
}

#[derive(Serialize, Clone)]
pub struct Release {
    version: String,
    notes: String,
    /// Newer than what's running
    newer: bool,
}

pub async fn check() -> Result<Release, String> {
    let manifest = manifest().await?;
    let release = Release {
        newer: is_newer(&manifest.version, env!("CARGO_PKG_VERSION")),
        version: manifest.version,
        notes: manifest.notes,
    };
    *LATEST.lock().unwrap() = Some(release.clone());
    Ok(release)
}

/// Look for a newer version in the background, for as long as the setting
/// allows it
pub async fn watch(store: std::sync::Arc<SettingsStore>) {
    let mut wait = FIRST_CHECK;
    loop {
        tokio::time::sleep(wait).await;
        if !store.get().advanced.check_for_updates {
            *LATEST.lock().unwrap() = None;
            wait = RETRY_INTERVAL;
            continue;
        }
        wait = match check().await {
            Ok(release) => {
                if release.newer {
                    println!("[update] version {} is available", release.version);
                }
                CHECK_INTERVAL
            }
            Err(e) => {
                if crate::settings::verbose() {
                    eprintln!("[update] check failed: {e}");
                }
                RETRY_INTERVAL
            }
        };
    }
}

/// Fetch the latest release and keep it for installing
pub async fn download() -> Result<(), String> {
    let manifest = manifest().await?;
    let arch = architecture().await?;
    let package = manifest.packages.get(&arch).ok_or(format!(
        "Version {} has no package for {arch} devices",
        manifest.version
    ))?;
    fs::create_dir_all(DIR).map_err(|e| format!("Couldn't create {DIR}: {e}"))?;
    let path = file("download.deb");
    let fetched = fetch(&package.url, Some(&path)).await;
    let deb = fs::read(&path);
    let _ = fs::remove_file(&path);
    fetched?;
    let deb = deb.map_err(|e| format!("Couldn't read the download: {e}"))?;
    stage(&deb, &package.signature).await
}

async fn manifest() -> Result<Manifest, String> {
    let url = std::env::var(MANIFEST_URL_ENV).unwrap_or_else(|_| MANIFEST_URL.into());
    let text = fetch(&url, None).await?;
    serde_json::from_str(&text).map_err(|_| "The update server sent something unexpected".into())
}

/// The body, or nothing when it goes to `into`
async fn fetch(url: &str, into: Option<&Path>) -> Result<String, String> {
    use tokio::process::Command;

    // Only a test's own server may be plain HTTP
    let protocols = if std::env::var_os(MANIFEST_URL_ENV).is_some() {
        "=http,https"
    } else {
        "=https"
    };
    let mut curl = Command::new("curl");
    curl.args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--proto", protocols, "--proto-redir", protocols])
        .args(["--connect-timeout", "15", "--max-time", "600"])
        .args(["--max-filesize", &MAX_PACKAGE_BYTES.to_string()]);
    if let Some(path) = into {
        curl.arg("--output").arg(path);
    }
    let out = curl
        .arg(url)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("Couldn't run curl: {e}"))?;
    if !out.status.success() {
        let reason = String::from_utf8_lossy(&out.stderr);
        let reason = reason.trim().trim_start_matches("curl: ");
        return Err(format!("Couldn't reach the update server: {reason}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Dotted numbers compared piece by piece; anything else never counts as
/// newer, so a garbled manifest can't offer itself
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| {
        v.split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    match (parse(candidate), parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

async fn architecture() -> Result<String, String> {
    run("dpkg", &["--print-architecture"])
        .await
        .map(|arch| arch.trim().to_string())
        .map_err(|e| format!("Couldn't tell what kind of device this is: {e}"))
}

pub async fn status() -> Status {
    let installing = run("systemctl", &["is-active", UNIT])
        .await
        .is_ok_and(|state| matches!(state.trim(), "active" | "activating"));
    let result = fs::read_to_string(file(RESULT)).unwrap_or_default();
    let failed = !matches!(result.trim(), "" | "0");
    Status {
        current: env!("CARGO_PKG_VERSION"),
        staged: version_of(&file(STAGED)).await,
        previous: version_of(&file(PREVIOUS)).await,
        available: LATEST.lock().unwrap().clone().filter(|r| r.newer),
        installing,
        failure: (failed && !installing).then(|| Failure {
            version: fs::read_to_string(file(ATTEMPT)).unwrap_or_default(),
            reverted: result.trim() == REVERTED,
            log: fs::read_to_string(file(LOG)).unwrap_or_default(),
        }),
    }
}

/// Keep an uploaded package for installing, once it's proven to be ours
pub async fn stage(deb: &[u8], signature: &str) -> Result<(), String> {
    verify(PUBLIC_KEY, deb, signature)?;
    fs::create_dir_all(DIR).map_err(|e| format!("Couldn't create {DIR}: {e}"))?;
    let upload = file("upload.deb");
    fs::write(&upload, deb).map_err(|e| format!("Couldn't store the package: {e}"))?;
    let checked = check_package(&upload).await;
    if checked.is_err() {
        let _ = fs::remove_file(&upload);
    }
    checked?;
    fs::rename(&upload, file(STAGED)).map_err(|e| format!("Couldn't store the package: {e}"))?;
    let _ = fs::remove_file(file(RESULT));
    Ok(())
}

pub fn discard() {
    let _ = fs::remove_file(file(STAGED));
    let _ = fs::remove_file(file(RESULT));
}

pub async fn install() -> Result<(), String> {
    let version = version_of(&file(STAGED))
        .await
        .ok_or("Upload a package first")?;
    // What ran before stays as the one to roll back to
    start(
        &version,
        STAGED,
        &format!("[ ! -f {CURRENT} ] || mv -f {CURRENT} {PREVIOUS}; mv -f {STAGED} {CURRENT}"),
    )
    .await
}

pub async fn rollback() -> Result<(), String> {
    let version = version_of(&file(PREVIOUS))
        .await
        .ok_or("There's no earlier version to go back to")?;
    // The two swap places, so rolling back twice returns to the newer one
    start(
        &version,
        PREVIOUS,
        &format!(
            "mv -f {PREVIOUS} swap.deb; [ ! -f {CURRENT} ] || mv -f {CURRENT} {PREVIOUS}; mv -f swap.deb {CURRENT}"
        ),
    )
    .await
}

/// Install `deb`, then give the version it brings a minute to answer. One
/// that doesn't would leave no web interface to fix it from, so the package
/// that was running goes back on. `keep` files the packages once it's
/// settled
fn script(dir: &str, deb: &str, version: &str, port: u16, keep: &str) -> String {
    format!(
        r#"apt-get install -y --allow-downgrades {dir}/{deb} >{LOG} 2>&1
code=$?
if [ $code != 0 ]; then
    echo $code >{RESULT}
    exit 0
fi
tries=0
until curl --fail --silent --max-time 2 http://localhost:{port}/health | grep -q '"version":"{version}"'; do
    tries=$((tries + 1))
    if [ $tries -ge {HEALTH_TRIES} ]; then
        echo "Version {version} didn't start" >>{LOG}
        if [ -f {CURRENT} ] && apt-get install -y --allow-downgrades {dir}/{CURRENT} >>{LOG} 2>&1; then
            echo {REVERTED} >{RESULT}
        else
            echo {UNANSWERED} >{RESULT}
        fi
        exit 0
    fi
    sleep 2
done
{keep}
echo 0 >{RESULT}
"#
    )
}

async fn start(version: &str, deb: &str, keep: &str) -> Result<(), String> {
    if status().await.installing {
        return Err("An update is already being installed".into());
    }
    let _ = fs::remove_file(file(RESULT));
    fs::write(file(ATTEMPT), version).map_err(|e| format!("Couldn't start the update: {e}"))?;
    let port = PORT.get().copied().unwrap_or(80);
    let script = script(DIR, deb, version, port, keep);
    run(
        "systemd-run",
        &[
            "--unit",
            UNIT,
            "--collect",
            "--quiet",
            "--setenv=DEBIAN_FRONTEND=noninteractive",
            &format!("--working-directory={DIR}"),
            "sh",
            "-c",
            &script,
        ],
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("Couldn't start the update: {e}"))
}

fn verify(public_key: &str, deb: &[u8], signature: &str) -> Result<(), String> {
    let decode = |text: &str| base64::engine::general_purpose::STANDARD.decode(text.trim());
    let key = decode(public_key)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
        .expect("update key is valid");
    let signature = decode(signature)
        .ok()
        .and_then(|bytes| Signature::from_slice(&bytes).ok())
        .ok_or("That isn't a signature file")?;
    key.verify_strict(deb, &signature)
        .map_err(|_| "The signature doesn't match this package".into())
}

/// A signed package for something else, or for another kind of device,
/// would still install
async fn check_package(deb: &Path) -> Result<(), String> {
    let field = async |name: &str| {
        run("dpkg-deb", &["-f", &deb.to_string_lossy(), name])
            .await
            .map(|value| value.trim().to_string())
            .map_err(|_| "That isn't a package file".to_string())
    };
    if field("Package").await? != PACKAGE {
        return Err("That package isn't YStreamer".into());
    }
    let ours = architecture().await?;
    let theirs = field("Architecture").await?;
    if theirs != ours {
        return Err(format!(
            "That package is for {theirs} devices, this one is {ours}"
        ));
    }
    Ok(())
}

async fn version_of(deb: &Path) -> Option<String> {
    if !deb.exists() {
        return None;
    }
    let version = run("dpkg-deb", &["-f", &deb.to_string_lossy(), "Version"])
        .await
        .ok()?;
    Some(display_version(version.trim()).to_string())
}

/// Without the packaging revision, to compare with what About shows
fn display_version(version: &str) -> &str {
    version
        .rsplit_once('-')
        .map_or(version, |(upstream, _)| upstream)
}

fn file(name: &str) -> PathBuf {
    Path::new(DIR).join(name)
}

#[cfg(test)]
#[path = "update.test.rs"]
mod tests;
