//! Everything the site takes from its environment.

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    /// Where to listen. `A2_LISTEN`, default 0.0.0.0:8080.
    pub listen: SocketAddr,
    /// The address players reach the site at, used for the links the meter
    /// opens and copies (`/link?code=…`, `/logs/<id>`). `A2_PUBLIC_URL`.
    pub public_url: String,
    /// The database, slices and dev logs. `A2_DATA_DIR`, default ./data.
    pub data_dir: PathBuf,
    /// Whether anyone may create an account. `A2_OPEN_REGISTRATION`, default
    /// true. The first account is the admin either way.
    pub open_registration: bool,
    /// Take the client address from X-Forwarded-For / X-Real-IP. Set when the
    /// site is behind a reverse proxy (Caddy, nginx). `A2_TRUST_PROXY`.
    pub trust_proxy: bool,
    /// The site's name in its pages. `A2_SITE_NAME`, default "A2 Tools".
    pub site_name: String,
    /// Uploads one account may make an hour. `A2_UPLOADS_PER_HOUR`.
    pub uploads_per_hour: usize,
    /// Days dev-log reports are kept. `A2_DEV_LOG_DAYS`, default 30.
    pub dev_log_days: i64,
}

fn flag(name: &str, default: bool) -> bool {
    match std::env::var(name).ok().as_deref().map(str::trim) {
        Some("1" | "true" | "yes" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        _ => default,
    }
}

impl Config {
    pub fn from_env() -> Self {
        let listen: SocketAddr = std::env::var("A2_LISTEN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| "0.0.0.0:8080".parse().unwrap());
        let public_url = std::env::var("A2_PUBLIC_URL")
            .ok()
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| format!("http://localhost:{}", listen.port()));
        Self {
            listen,
            public_url,
            data_dir: std::env::var("A2_DATA_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("data")),
            open_registration: flag("A2_OPEN_REGISTRATION", true),
            trust_proxy: flag("A2_TRUST_PROXY", false),
            site_name: std::env::var("A2_SITE_NAME").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "A2 Tools".into()),
            uploads_per_hour: std::env::var("A2_UPLOADS_PER_HOUR").ok().and_then(|v| v.parse().ok()).unwrap_or(120),
            dev_log_days: std::env::var("A2_DEV_LOG_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(30),
        }
    }
}
