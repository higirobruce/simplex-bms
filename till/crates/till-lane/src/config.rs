//! Where the cloud sync finds its two settings.
//!
//! * **Base URL** (not secret, changes with the Cloudflare tunnel hostname):
//!   the `SIMPLEX_CLOUD_URL` environment variable if set, otherwise the
//!   `base_url` field of a small JSON file, `{"base_url": "https://..."}`. The
//!   file is `SIMPLEX_TILL_CONFIG` if that is set, otherwise
//!   `%APPDATA%\Simplex\Till\cloud.json` on Windows and
//!   `$XDG_CONFIG_HOME/simplex-till/cloud.json` (or `~/.config/...`) elsewhere.
//!   Read at start-up, so changing it needs a restart of the lane, not a rebuild.
//! * **Lane token** (a secret): the OS credential store, service
//!   `simplex-till`, entry `lane-token`, via the `keyring` crate. It is written
//!   with `till-lane --set-token` (the token is read from standard input, so it
//!   never appears on a command line or in shell history). It is never read
//!   from a file or an environment variable.
//!
//! Nothing here ever formats the token into an error or a `Debug` string.

use std::fmt;
use std::path::PathBuf;

use serde::Deserialize;
use thiserror::Error;

const KEYRING_SERVICE: &str = "simplex-till";
const KEYRING_USER: &str = "lane-token";

pub const URL_ENV: &str = "SIMPLEX_CLOUD_URL";
pub const CONFIG_PATH_ENV: &str = "SIMPLEX_TILL_CONFIG";

/// Everything the sync needs to talk to the cloud.
#[derive(Clone)]
pub struct SyncConfig {
    /// Origin and optional path prefix, no trailing slash.
    pub base_url: String,
    /// The lane bearer token. A secret.
    pub token: String,
}

// Not derived: a derive would print the token into any log line or test
// failure that formats a config.
impl fmt::Debug for SyncConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncConfig")
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Why the sync is not configured. None of these carry the token.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(
        "no cloud address: set {URL_ENV}, or put {{\"base_url\": \"https://...\"}} in the config file"
    )]
    NoBaseUrl,
    #[error(
        "the cloud address {0:?} is not usable: it must be https:// (http:// only for localhost)"
    )]
    BadBaseUrl(String),
    #[error("the config file {path} could not be read: {reason}")]
    File { path: String, reason: String },
    #[error("no lane token is stored in the OS credential store; run `till-lane --set-token`")]
    NoToken,
    #[error("the OS credential store failed: {0}")]
    Keyring(String),
}

#[derive(Deserialize)]
struct FileConfig {
    base_url: Option<String>,
}

/// Validates and tidies a base URL. https is required because the lane token
/// travels in a header; plain http is allowed only to this machine.
pub fn normalise_base_url(raw: &str) -> Result<String, ConfigError> {
    let url = raw.trim().trim_end_matches('/');
    let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
        .iter()
        .any(|p| {
            url.strip_prefix(p)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        });
    let remote = url
        .strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'));
    if remote || local {
        Ok(url.to_owned())
    } else {
        Err(ConfigError::BadBaseUrl(raw.trim().to_owned()))
    }
}

/// Where the JSON config file lives.
pub fn config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(CONFIG_PATH_ENV).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if cfg!(windows) {
        let appdata = std::env::var_os("APPDATA")?;
        return Some(
            PathBuf::from(appdata)
                .join("Simplex")
                .join("Till")
                .join("cloud.json"),
        );
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("simplex-till").join("cloud.json"))
}

/// Picks the base URL: the environment beats the file. `file` is the file's
/// text, if it exists.
fn resolve_base_url(env: Option<String>, file: Option<&str>) -> Result<String, ConfigError> {
    if let Some(v) = env.filter(|v| !v.trim().is_empty()) {
        return normalise_base_url(&v);
    }
    let parsed: Option<FileConfig> = match file {
        None => None,
        Some(text) => Some(serde_json::from_str(text).map_err(|e| {
            ConfigError::File {
                path: config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                reason: e.to_string(),
            }
        })?),
    };
    match parsed
        .and_then(|c| c.base_url)
        .filter(|u| !u.trim().is_empty())
    {
        Some(u) => normalise_base_url(&u),
        None => Err(ConfigError::NoBaseUrl),
    }
}

fn entry() -> Result<keyring::Entry, ConfigError> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| ConfigError::Keyring(e.to_string()))
}

/// Saves the lane token to the OS credential store.
pub fn store_token(token: &str) -> Result<(), ConfigError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(ConfigError::NoToken);
    }
    entry()?
        .set_password(token)
        .map_err(|e| ConfigError::Keyring(e.to_string()))
}

fn load_token() -> Result<String, ConfigError> {
    match entry()?.get_password() {
        Ok(t) if !t.trim().is_empty() => Ok(t.trim().to_owned()),
        Ok(_) | Err(keyring::Error::NoEntry) => Err(ConfigError::NoToken),
        Err(e) => Err(ConfigError::Keyring(e.to_string())),
    }
}

/// Reads the base URL (environment, then file) and the token (credential store).
pub fn load() -> Result<SyncConfig, ConfigError> {
    let file = match config_path() {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(ConfigError::File {
                    path: p.display().to_string(),
                    reason: e.to_string(),
                });
            }
        },
        None => None,
    };
    let base_url = resolve_base_url(std::env::var(URL_ENV).ok(), file.as_deref())?;
    Ok(SyncConfig {
        base_url,
        token: load_token()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_is_required_except_for_this_machine() {
        assert_eq!(
            normalise_base_url(" https://pilot.example.com/ ").unwrap(),
            "https://pilot.example.com"
        );
        assert_eq!(
            normalise_base_url("http://localhost:3000").unwrap(),
            "http://localhost:3000"
        );
        assert_eq!(
            normalise_base_url("http://127.0.0.1:8080/").unwrap(),
            "http://127.0.0.1:8080"
        );
        for bad in [
            "http://pilot.example.com",
            "pilot.example.com",
            "",
            "https://",
            "http://localhost.evil.com",
        ] {
            assert!(normalise_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_environment_beats_the_file_and_a_missing_one_is_an_error() {
        let file = r#"{"base_url": "https://file.example.com"}"#;
        assert_eq!(
            resolve_base_url(Some("https://env.example.com".into()), Some(file)).unwrap(),
            "https://env.example.com"
        );
        assert_eq!(
            resolve_base_url(None, Some(file)).unwrap(),
            "https://file.example.com"
        );
        assert_eq!(
            resolve_base_url(Some("  ".into()), Some(file)).unwrap(),
            "https://file.example.com"
        );
        assert!(matches!(
            resolve_base_url(None, None),
            Err(ConfigError::NoBaseUrl)
        ));
        assert!(matches!(
            resolve_base_url(None, Some("{}")),
            Err(ConfigError::NoBaseUrl)
        ));
        assert!(matches!(
            resolve_base_url(None, Some("not json")),
            Err(ConfigError::File { .. })
        ));
    }

    #[test]
    fn the_debug_form_never_shows_the_token() {
        let c = SyncConfig {
            base_url: "https://x.example.com".into(),
            token: "TOP-SECRET-TOKEN".into(),
        };
        let shown = format!("{c:?}");
        assert!(!shown.contains("TOP-SECRET-TOKEN"), "{shown}");
        assert!(shown.contains("x.example.com"));
    }
}
