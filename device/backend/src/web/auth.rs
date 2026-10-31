use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

const COOKIE: &str = "ystreamer_session";
const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
const ITERATIONS: u32 = 100_000;
pub const MIN_PASSWORD_LEN: usize = 6;
const WRONG_PASSWORD_DELAY: Duration = Duration::from_secs(1);

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
struct AuthFile {
    /// `pbkdf2-sha256$<iterations>$<salt>$<hash>`, empty when there's no password
    password: String,
    protect_viewing: bool,
    /// Signs session cookies. Replaced with the password, which logs out
    /// every other browser
    secret: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    None,
    Viewer,
    Admin,
}

pub struct Auth {
    path: PathBuf,
    file: RwLock<AuthFile>,
    /// One password check at a time, so wrong-password delays can't be
    /// sidestepped by guessing in parallel
    login: tokio::sync::Mutex<()>,
    /// The last password a network drive sent that matched, signed together
    /// with the stored hash so it stops matching once the password changes
    drive_login: RwLock<Option<Vec<u8>>>,
}

impl Auth {
    pub fn load(path: PathBuf) -> Arc<Self> {
        let file = match fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
                eprintln!(
                    "[auth] {} is unreadable ({e}), no password set",
                    path.display()
                );
                AuthFile::default()
            }),
            Err(_) => AuthFile::default(),
        };
        Arc::new(Self {
            path,
            file: RwLock::new(file),
            login: tokio::sync::Mutex::new(()),
            drive_login: RwLock::new(None),
        })
    }

    /// Pick up a password changed from outside, by `ystreamer password`
    pub fn reload(&self) {
        match fs::read_to_string(&self.path) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(file) => *self.file.write().unwrap() = file,
                Err(e) => eprintln!("[auth] {} is unreadable ({e})", self.path.display()),
            },
            Err(_) => *self.file.write().unwrap() = AuthFile::default(),
        }
        println!("[auth] login reloaded");
    }

    pub fn enabled(&self) -> bool {
        !self.file.read().unwrap().password.is_empty()
    }

    pub fn protect_viewing(&self) -> bool {
        let f = self.file.read().unwrap();
        !f.password.is_empty() && f.protect_viewing
    }

    pub fn access(&self, headers: &HeaderMap) -> Access {
        let f = self.file.read().unwrap();
        if f.password.is_empty() {
            return Access::Admin;
        }
        if cookie(headers).is_some_and(|token| token_valid(&f.secret, token)) {
            return Access::Admin;
        }
        if f.protect_viewing {
            Access::None
        } else {
            Access::Viewer
        }
    }

    /// Slow on a wrong password, and serialized, so guessing stays slow
    pub async fn check_password(self: &Arc<Self>, password: String) -> bool {
        let _one_at_a_time = self.login.lock().await;
        let this = Arc::clone(self);
        let ok = tokio::task::spawn_blocking(move || {
            let stored = this.file.read().unwrap().password.clone();
            verify_hash(&stored, &password)
        })
        .await
        .unwrap_or(false);
        if !ok {
            tokio::time::sleep(WRONG_PASSWORD_DELAY).await;
        }
        ok
    }

    /// HTTP Basic, for network drive clients, which can't log in with the
    /// cookie. They send the password with every request and checking the
    /// hash takes a moment, so the last match is remembered
    pub async fn check_basic(self: &Arc<Self>, headers: &HeaderMap) -> bool {
        let Some(password) = basic_password(headers) else {
            return false;
        };
        let stored = self.file.read().unwrap().clone();
        let mac = || drive_mac(&stored, &password);
        let remembered = self.drive_login.read().unwrap().clone();
        if remembered.is_some_and(|known| mac().verify_slice(&known).is_ok()) {
            return true;
        }
        if !self.check_password(password.clone()).await {
            return false;
        }
        let mac = mac().finalize().into_bytes().to_vec();
        *self.drive_login.write().unwrap() = Some(mac);
        true
    }

    pub fn session_cookie(&self) -> String {
        let secret = self.file.read().unwrap().secret.clone();
        let expires = now_secs() + SESSION_TTL.as_secs();
        let token = format!("{expires}.{}", B64.encode(sign(&secret, expires)));
        format!(
            "{COOKIE}={token}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
            SESSION_TTL.as_secs()
        )
    }

    pub fn clear_cookie() -> String {
        format!("{COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax")
    }

    /// `None` removes the password
    pub async fn set_password(self: &Arc<Self>, new: Option<String>) -> Result<(), String> {
        if let Some(pw) = &new
            && pw.chars().count() < MIN_PASSWORD_LEN
        {
            return Err(format!("Use at least {MIN_PASSWORD_LEN} characters"));
        }
        let hash = match new {
            Some(pw) => tokio::task::spawn_blocking(move || hash_password(&pw))
                .await
                .map_err(|e| e.to_string())?,
            None => String::new(),
        };
        let mut f = self.file.read().unwrap().clone();
        if hash.is_empty() {
            f.protect_viewing = false;
        }
        f.password = hash;
        f.secret = B64.encode(random::<32>());
        self.save(f)
    }

    pub fn set_protect_viewing(&self, on: bool) -> Result<(), String> {
        let mut f = self.file.read().unwrap().clone();
        if f.password.is_empty() {
            return Err("Set a password first".into());
        }
        f.protect_viewing = on;
        self.save(f)
    }

    fn save(&self, f: AuthFile) -> Result<(), String> {
        let err = |e: std::io::Error| format!("Couldn't save {}: {e}", self.path.display());
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(err)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(err)?;
        out.write_all(
            serde_json::to_string_pretty(&f)
                .expect("auth serialize")
                .as_bytes(),
        )
        .map_err(err)?;
        out.sync_all().map_err(err)?;
        fs::rename(&tmp, &self.path).map_err(err)?;
        *self.file.write().unwrap() = f;
        Ok(())
    }
}

pub async fn admin_only(State(auth): State<Arc<Auth>>, req: Request, next: Next) -> Response {
    require(&auth, Access::Admin, req, next).await
}

pub async fn viewers_only(State(auth): State<Arc<Auth>>, req: Request, next: Next) -> Response {
    require(&auth, Access::Viewer, req, next).await
}

async fn require(auth: &Auth, level: Access, req: Request, next: Next) -> Response {
    if auth.access(req.headers()) >= level {
        return next.run(req).await;
    }
    let body = serde_json::json!({ "error": "Log in first" }).to_string();
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(COOKIE)?.strip_prefix('='))
}

/// The password of `Authorization: Basic`, whatever the user name
fn basic_password(headers: &HeaderMap) -> Option<String> {
    use base64::engine::general_purpose::STANDARD;
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, encoded) = value.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = String::from_utf8(STANDARD.decode(encoded.trim()).ok()?).ok()?;
    let (_user, password) = decoded.split_once(':')?;
    Some(password.to_string())
}

fn drive_mac(stored: &AuthFile, password: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(stored.secret.as_bytes()).expect("any key length");
    mac.update(stored.password.as_bytes());
    mac.update(b"\0");
    mac.update(password.as_bytes());
    mac
}

fn sign(secret: &str, expires: u64) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(expires.to_string().as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn token_valid(secret: &str, token: &str) -> bool {
    let Some((expires, sig)) = token.split_once('.') else {
        return false;
    };
    let (Ok(expires), Ok(sig)) = (expires.parse::<u64>(), B64.decode(sig)) else {
        return false;
    };
    if expires < now_secs() {
        return false;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(expires.to_string().as_bytes());
    mac.verify_slice(&sig).is_ok()
}

fn hash_password(password: &str) -> String {
    let salt = random::<16>();
    let mut hash = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, ITERATIONS, &mut hash);
    format!(
        "pbkdf2-sha256${ITERATIONS}${}${}",
        B64.encode(salt),
        B64.encode(hash)
    )
}

fn verify_hash(stored: &str, password: &str) -> bool {
    let mut parts = stored.split('$');
    let (Some("pbkdf2-sha256"), Some(iters), Some(salt), Some(hash), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    let (Ok(iters), Ok(salt), Ok(expected)) =
        (iters.parse::<u32>(), B64.decode(salt), B64.decode(hash))
    else {
        return false;
    };
    let mut actual = vec![0u8; expected.len()];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iters, &mut actual);
    actual
        .iter()
        .zip(&expected)
        .fold(0u8, |d, (a, b)| d | (a ^ b))
        == 0
        && actual.len() == expected.len()
}

fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand::rng().fill_bytes(&mut out);
    out
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "auth.test.rs"]
mod tests;
