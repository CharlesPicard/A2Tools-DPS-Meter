//! The one SQLite database: accounts, sign-ins, logs and what the leaderboards
//! read. Slices and dev-log files live beside it on disk (`Store`).
//!
//! One connection behind a mutex, used from blocking tasks. A self-hosted site
//! serves a guild or a community, not the world; SQLite in WAL mode is far more
//! than that needs, and it keeps the whole site one file to back up.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id                 INTEGER PRIMARY KEY,
    username           TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash      TEXT NOT NULL,
    display_name       TEXT NOT NULL,
    is_admin           INTEGER NOT NULL DEFAULT 0,
    supporter          INTEGER NOT NULL DEFAULT 0,
    default_visibility TEXT NOT NULL DEFAULT 'public',
    created_at         INTEGER NOT NULL
);

-- Web sign-ins (a cookie). Only the token's hash is kept.
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

-- Meters signed in through the device grant. Revocable from the account page.
CREATE TABLE IF NOT EXISTS api_tokens (
    id         INTEGER PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label      TEXT NOT NULL,
    scopes     TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used  INTEGER
);

-- RFC 8628 device codes. status: pending, approved, denied, issued.
CREATE TABLE IF NOT EXISTS device_codes (
    device_hash TEXT PRIMARY KEY,
    user_code   TEXT NOT NULL UNIQUE,
    client      TEXT NOT NULL,
    label       TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    interval_s  INTEGER NOT NULL,
    last_poll   INTEGER NOT NULL DEFAULT 0,
    status      TEXT NOT NULL DEFAULT 'pending',
    user_id     INTEGER REFERENCES users(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS logs (
    id               TEXT PRIMARY KEY,
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at       INTEGER NOT NULL,
    visibility       TEXT NOT NULL,
    fight_start_ms   INTEGER NOT NULL,
    mob_code         INTEGER NOT NULL,
    boss_name        TEXT NOT NULL,
    dungeon_id       INTEGER NOT NULL,
    server_id        INTEGER NOT NULL,
    duration_ms      INTEGER NOT NULL,
    total_damage     INTEGER NOT NULL,
    max_hp           INTEGER NOT NULL,
    killed           INTEGER NOT NULL,
    is_train         INTEGER NOT NULL,
    party_size       INTEGER NOT NULL,
    parser_version   TEXT NOT NULL,
    app_version      TEXT NOT NULL,
    ui_language      TEXT NOT NULL,
    slice_sha256     TEXT NOT NULL,
    uploader_actor   INTEGER,
    -- Whether the log counts for the leaderboards and class stats: a kill, of
    -- a boss, from a slice the checks found whole.
    ranked           INTEGER NOT NULL,
    checks_json      TEXT NOT NULL,
    names_json       TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS logs_user_slice ON logs(user_id, slice_sha256);
CREATE INDEX IF NOT EXISTS logs_created ON logs(created_at DESC);
CREATE INDEX IF NOT EXISTS logs_mob ON logs(mob_code, ranked);

-- One row per player in a log, for the leaderboards and class stats.
CREATE TABLE IF NOT EXISTS log_players (
    log_id      TEXT NOT NULL REFERENCES logs(id) ON DELETE CASCADE,
    actor_id    INTEGER NOT NULL,
    name        TEXT NOT NULL,
    job_id      INTEGER NOT NULL,
    damage      INTEGER NOT NULL,
    dps         REAL NOT NULL,
    is_uploader INTEGER NOT NULL,
    PRIMARY KEY (log_id, actor_id)
);
CREATE INDEX IF NOT EXISTS log_players_job ON log_players(job_id, dps DESC);

-- Meters that said "I use the meter" (a hash of name and server), for the
-- party marks. Kept only as long as they keep saying it.
CREATE TABLE IF NOT EXISTS presence (
    hash      TEXT PRIMARY KEY,
    last_seen INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS dev_reports (
    code        TEXT PRIMARY KEY,
    user_id     INTEGER REFERENCES users(id) ON DELETE SET NULL,
    app_version TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    files_json  TEXT NOT NULL,
    received    TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    /// Run `f` on the connection off the async runtime.
    pub async fn call<T, F>(&self, f: F) -> rusqlite::Result<T>
    where
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = inner.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut conn)
        })
        .await
        .expect("database task panicked")
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

