//! The smaller meter endpoints: party presence (`party_presence.rs`) and
//! "Send logs to dev" (`share/dev_logs.rs`), and the admin's view of the latter.

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{token_user, AdminUser};
use crate::db::now;
use crate::util::{random_code, ApiError, ApiResult};
use crate::AppState;

/// A meter that says it is used is a user for this long.
const PRESENCE_DAYS: i64 = 30;

#[derive(Deserialize)]
pub struct Presence {
    me: String,
    #[serde(default)]
    check: Vec<String>,
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// POST /api/presence: record that `me` uses the meter, and say which of
/// `check` do. Only hashes of a name and server go either way.
pub async fn presence(State(state): State<AppState>, headers: HeaderMap, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, Json(body): Json<Presence>) -> ApiResult<Json<serde_json::Value>> {
    let ip = crate::util::client_ip(&headers, peer, state.config.trust_proxy);
    if !state.limiter.allow(&format!("presence:{ip}"), 30, std::time::Duration::from_secs(60)) {
        return Err(ApiError::too_many());
    }
    if !is_hash(&body.me) {
        return Err(ApiError::bad_request("me is a hash"));
    }
    let check: Vec<String> = body.check.into_iter().filter(|h| is_hash(h)).take(64).map(|h| h.to_lowercase()).collect();
    let me = body.me.to_lowercase();
    let t = now();
    let users = state
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO presence (hash, last_seen) VALUES (?1, ?2) ON CONFLICT(hash) DO UPDATE SET last_seen = ?2",
                params![me, t],
            )?;
            let mut stmt = c.prepare("SELECT 1 FROM presence WHERE hash = ?1 AND last_seen >= ?2")?;
            let mut users = Vec::new();
            for h in check {
                if stmt.query_row(params![h, t - PRESENCE_DAYS * 86_400], |_| Ok(())).optional()?.is_some() {
                    users.push(h);
                }
            }
            Ok(users)
        })
        .await?;
    Ok(Json(json!({ "users": users })))
}

pub async fn prune_loop(state: AppState) {
    let mut every = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        every.tick().await;
        let t = now();
        let days = state.config.dev_log_days;
        let old: Vec<String> = state
            .db
            .call(move |c| {
                c.execute("DELETE FROM presence WHERE last_seen < ?1", params![t - PRESENCE_DAYS * 86_400])?;
                c.execute("DELETE FROM sessions WHERE expires_at < ?1", params![t])?;
                let mut stmt = c.prepare("SELECT code FROM dev_reports WHERE created_at < ?1")?;
                let old = stmt.query_map(params![t - days * 86_400], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
                c.execute("DELETE FROM dev_reports WHERE created_at < ?1", params![t - days * 86_400])?;
                Ok(old)
            })
            .await
            .unwrap_or_default();
        for code in old {
            state.store.remove_devlog(&code);
        }
    }
}

// ── dev logs ───────────────────────────────────────────────────────────────

const MAX_DEV_FILES: usize = 5;
const MAX_DEV_FILE: usize = 40 * 1024 * 1024;

fn valid_capture_name(name: &str) -> bool {
    name.len() <= 64
        && name.starts_with("packets_")
        && name.ends_with(".txt")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevReport {
    #[serde(default)]
    app_version: String,
    files: Vec<DevFile>,
}

#[derive(Deserialize, serde::Serialize)]
struct DevFile {
    name: String,
    #[serde(default)]
    size: u64,
}

pub async fn dev_register(State(state): State<AppState>, headers: HeaderMap, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, Json(body): Json<DevReport>) -> ApiResult<Json<serde_json::Value>> {
    let ip = crate::util::client_ip(&headers, peer, state.config.trust_proxy);
    if !state.limiter.allow(&format!("devlogs:{ip}"), 6, std::time::Duration::from_secs(3600)) {
        return Err(ApiError::too_many());
    }
    if body.files.is_empty() || body.files.len() > MAX_DEV_FILES || !body.files.iter().all(|f| valid_capture_name(&f.name)) {
        return Err(ApiError::bad_request("Up to five packet logs, named as the meter names them."));
    }
    let user = token_user(&state, &headers).await?.map(|(u, _)| u.id);
    let code = random_code(8);
    let files = serde_json::to_string(&body.files).unwrap_or_default();
    let version: String = body.app_version.chars().take(32).collect();
    let c2 = code.clone();
    state
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO dev_reports (code, user_id, app_version, created_at, files_json) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![c2, user, version, now(), files],
            )
        })
        .await?;
    if let Some(dir) = state.store.devlog_dir(&code) {
        std::fs::create_dir_all(dir).map_err(ApiError::internal)?;
    }
    Ok(Json(json!({ "code": code, "uploadPath": format!("/api/dev-logs/{code}/") })))
}

pub async fn dev_put(State(state): State<AppState>, Path((code, name)): Path<(String, String)>, body: Bytes) -> ApiResult<Json<serde_json::Value>> {
    if !valid_capture_name(&name) || body.len() > MAX_DEV_FILE {
        return Err(ApiError::bad_request("Not a file of this report."));
    }
    let c2 = code.clone();
    let n2 = name.clone();
    let ok = state
        .db
        .call(move |c| {
            let row: Option<(String, String, i64)> = c
                .query_row("SELECT files_json, received, created_at FROM dev_reports WHERE code = ?1", params![c2], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()?;
            let Some((files, received, created)) = row else { return Ok(false) };
            let files: Vec<DevFile> = serde_json::from_str(&files).unwrap_or_default();
            let mut received: Vec<String> = serde_json::from_str(&received).unwrap_or_default();
            // Files go within the hour of the report, each once.
            if now() - created > 3600 || !files.iter().any(|f| f.name == n2) || received.contains(&n2) {
                return Ok(false);
            }
            received.push(n2);
            c.execute("UPDATE dev_reports SET received = ?1 WHERE code = ?2", params![serde_json::to_string(&received).unwrap_or_default(), c2])?;
            Ok(true)
        })
        .await?;
    if !ok {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "No such report, or this file was already sent."));
    }
    let dir = state.store.devlog_dir(&code).ok_or_else(ApiError::not_found)?;
    tokio::fs::write(dir.join(format!("{name}.gz")), &body).await.map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn dev_list(State(state): State<AppState>, _admin: AdminUser) -> ApiResult<Json<serde_json::Value>> {
    let reports = state
        .db
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT dev_reports.code, dev_reports.app_version, dev_reports.created_at, dev_reports.received, users.display_name
                 FROM dev_reports LEFT JOIN users ON users.id = dev_reports.user_id ORDER BY dev_reports.created_at DESC",
            )?;
            let rows = stmt.query_map([], |r| {
                let received: Vec<String> = serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default();
                Ok(json!({ "code": r.get::<_, String>(0)?, "appVersion": r.get::<_, String>(1)?, "createdAt": r.get::<_, i64>(2)?,
                           "files": received, "user": r.get::<_, Option<String>>(4)? }))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(json!({ "reports": reports, "keepDays": state.config.dev_log_days })))
}

pub async fn dev_download(State(state): State<AppState>, _admin: AdminUser, Path((code, name)): Path<(String, String)>) -> ApiResult<Response> {
    if !valid_capture_name(&name) {
        return Err(ApiError::not_found());
    }
    let dir = state.store.devlog_dir(&code).ok_or_else(ApiError::not_found)?;
    let bytes = tokio::fs::read(dir.join(format!("{name}.gz"))).await.map_err(|_| ApiError::not_found())?;
    let disposition = format!("attachment; filename=\"{code}-{name}.gz\"");
    Ok(([("content-type", "application/gzip".to_string()), ("content-disposition", disposition)], bytes).into_response())
}

pub async fn dev_delete(State(state): State<AppState>, _admin: AdminUser, Path(code): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let c2 = code.clone();
    state.db.call(move |c| c.execute("DELETE FROM dev_reports WHERE code = ?1", params![c2])).await?;
    state.store.remove_devlog(&code);
    Ok(Json(json!({ "ok": true })))
}
