//! Accounts: username and password on the site, a cookie for the browser, and
//! bearer tokens for meters (issued by the device grant, `device.rs`).

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::now;
use crate::util::{random_token, sha256_hex, ApiError, ApiResult};
use crate::AppState;

pub const SESSION_COOKIE: &str = "a2_session";
const SESSION_DAYS: i64 = 30;

/// What a meter's token may do. The meter asks for none; it gets these.
pub const METER_SCOPES: &[&str] = &["profile", "sync", "characters", "logs"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub is_admin: bool,
    pub supporter: bool,
    pub default_visibility: String,
}

fn user_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        display_name: row.get(2)?,
        is_admin: row.get::<_, i64>(3)? != 0,
        supporter: row.get::<_, i64>(4)? != 0,
        default_visibility: row.get(5)?,
    })
}

const USER_COLUMNS: &str = "id, username, display_name, is_admin, supporter, default_visibility";

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::encode_b64(random_token(16).as_bytes()).map_err(|e| e.to_string())?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

fn valid_username(name: &str) -> bool {
    (3..=32).contains(&name.chars().count())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("authorization")?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer "))?;
    Some(token.trim().to_string()).filter(|t| !t.is_empty())
}

/// The user behind the request's session cookie, if any.
pub async fn session_user(state: &AppState, headers: &HeaderMap) -> ApiResult<Option<User>> {
    let Some(token) = cookie_value(headers, SESSION_COOKIE) else { return Ok(None) };
    let hash = sha256_hex(token.as_bytes());
    let user = state
        .db
        .call(move |c| {
            c.query_row(
                &format!(
                    "SELECT {} FROM users JOIN sessions ON sessions.user_id = users.id
                     WHERE sessions.token_hash = ?1 AND sessions.expires_at > ?2",
                    USER_COLUMNS.split(", ").map(|col| format!("users.{col}")).collect::<Vec<_>>().join(", ")
                ),
                params![hash, now()],
                user_from_row,
            )
            .optional()
        })
        .await?;
    Ok(user)
}

/// The user and scopes behind a bearer token, if any. Marks the token used.
pub async fn token_user(state: &AppState, headers: &HeaderMap) -> ApiResult<Option<(User, Vec<String>)>> {
    let Some(token) = bearer(headers) else { return Ok(None) };
    let hash = sha256_hex(token.as_bytes());
    let found = state
        .db
        .call(move |c| {
            let found = c
                .query_row(
                    "SELECT users.id, users.username, users.display_name, users.is_admin, users.supporter,
                            users.default_visibility, api_tokens.scopes
                     FROM users JOIN api_tokens ON api_tokens.user_id = users.id
                     WHERE api_tokens.token_hash = ?1",
                    params![hash],
                    |row| Ok((user_from_row(row)?, row.get::<_, String>(6)?)),
                )
                .optional()?;
            if found.is_some() {
                c.execute("UPDATE api_tokens SET last_used = ?1 WHERE token_hash = ?2", params![now(), hash])?;
            }
            Ok(found)
        })
        .await?;
    Ok(found.map(|(u, scopes)| (u, scopes.split(' ').filter(|s| !s.is_empty()).map(String::from).collect())))
}

/// A route that needs a signed-in browser.
pub struct WebUser(pub User);

impl FromRequestParts<AppState> for WebUser {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        session_user(state, &parts.headers).await?.map(WebUser).ok_or_else(ApiError::unauthorized)
    }
}

/// A route a meter calls with its token.
pub struct MeterUser {
    pub user: User,
    pub scopes: Vec<String>,
}

impl MeterUser {
    pub fn require(&self, scope: &str) -> ApiResult<()> {
        if self.scopes.iter().any(|s| s == scope) {
            Ok(())
        } else {
            Err(ApiError::new(StatusCode::FORBIDDEN, "insufficient_scope", "This sign-in may not do that."))
        }
    }
}

impl FromRequestParts<AppState> for MeterUser {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        token_user(state, &parts.headers)
            .await?
            .map(|(user, scopes)| MeterUser { user, scopes })
            .ok_or_else(ApiError::unauthorized)
    }
}

fn session_cookie(state: &AppState, token: &str, max_age: i64) -> HeaderValue {
    let secure = if state.config.public_url.starts_with("https://") { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .expect("cookie is ascii")
}

async fn start_session(state: &AppState, user_id: i64) -> ApiResult<HeaderValue> {
    let token = random_token(32);
    let hash = sha256_hex(token.as_bytes());
    let t = now();
    state
        .db
        .call(move |c| {
            c.execute("DELETE FROM sessions WHERE expires_at < ?1", params![t])?;
            c.execute(
                "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
                params![hash, user_id, t, t + SESSION_DAYS * 86_400],
            )
        })
        .await?;
    Ok(session_cookie(state, &token, SESSION_DAYS * 86_400))
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
    #[serde(default)]
    display_name: Option<String>,
}

pub async fn register(State(state): State<AppState>, Json(body): Json<Credentials>) -> ApiResult<Response> {
    let username = body.username.trim().to_string();
    if !valid_username(&username) {
        return Err(ApiError::bad_request("A username is 3 to 32 letters, digits, '.', '-' or '_'."));
    }
    if body.password.chars().count() < 8 {
        return Err(ApiError::bad_request("A password is at least 8 characters."));
    }
    let display = body
        .display_name
        .map(|d| d.trim().chars().take(32).collect::<String>())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| username.clone());
    let hash = hash_password(&body.password).map_err(ApiError::internal)?;
    let open = state.config.open_registration;
    let created = state
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let users: i64 = tx.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
            if users > 0 && !open {
                return Ok(Err("closed"));
            }
            let taken: bool = tx
                .query_row("SELECT 1 FROM users WHERE username = ?1", params![username], |_| Ok(true))
                .optional()?
                .unwrap_or(false);
            if taken {
                return Ok(Err("taken"));
            }
            tx.execute(
                "INSERT INTO users (username, password_hash, display_name, is_admin, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![username, hash, display, (users == 0) as i64, now()],
            )?;
            let id = tx.last_insert_rowid();
            tx.commit()?;
            Ok(Ok(id))
        })
        .await?;
    let id = match created {
        Ok(id) => id,
        Err("closed") => return Err(ApiError::forbidden("This site does not take new accounts. Ask its admin for one.")),
        Err(_) => return Err(ApiError::new(StatusCode::CONFLICT, "taken", "That username is taken.")),
    };
    let cookie = start_session(&state, id).await?;
    Ok(([(SET_COOKIE, cookie)], Json(serde_json::json!({ "ok": true }))).into_response())
}

pub async fn login(State(state): State<AppState>, headers: HeaderMap, peer: axum::extract::ConnectInfo<std::net::SocketAddr>, Json(body): Json<Credentials>) -> ApiResult<Response> {
    let ip = crate::util::client_ip(&headers, peer.0, state.config.trust_proxy);
    if !state.limiter.allow(&format!("login:{ip}"), 20, std::time::Duration::from_secs(600)) {
        return Err(ApiError::too_many());
    }
    let username = body.username.trim().to_string();
    let found: Option<(i64, String)> = state
        .db
        .call(move |c| {
            c.query_row("SELECT id, password_hash FROM users WHERE username = ?1", params![username], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
        })
        .await?;
    let password = body.password;
    let ok = match &found {
        Some((_, hash)) => {
            let hash = hash.clone();
            tokio::task::spawn_blocking(move || verify_password(&password, &hash)).await.unwrap_or(false)
        }
        None => false,
    };
    let Some((id, _)) = found.filter(|_| ok) else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "bad_credentials", "Wrong username or password."));
    };
    let cookie = start_session(&state, id).await?;
    Ok(([(SET_COOKIE, cookie)], Json(serde_json::json!({ "ok": true }))).into_response())
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = cookie_value(&headers, SESSION_COOKIE) {
        let hash = sha256_hex(token.as_bytes());
        state.db.call(move |c| c.execute("DELETE FROM sessions WHERE token_hash = ?1", params![hash])).await?;
    }
    Ok(([(SET_COOKIE, session_cookie(&state, "", 0))], Json(serde_json::json!({ "ok": true }))).into_response())
}

/// The browser's own account, or null.
pub async fn session(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<serde_json::Value>> {
    let user = session_user(&state, &headers).await?;
    Ok(Json(serde_json::json!({
        "user": user,
        "siteName": state.config.site_name,
        "openRegistration": state.config.open_registration,
    })))
}

/// GET /api/me: the account behind a meter's token, as `account::whoami`
/// reads it (`displayName`, `supporter`, `scopes`, anything else passed on).
pub async fn me(State(state): State<AppState>, meter: MeterUser) -> ApiResult<Json<serde_json::Value>> {
    let id = meter.user.id;
    let logs: i64 = state.db.call(move |c| c.query_row("SELECT COUNT(*) FROM logs WHERE user_id = ?1", params![id], |r| r.get(0))).await?;
    Ok(Json(serde_json::json!({
        "displayName": meter.user.display_name,
        "supporter": meter.user.supporter,
        "scopes": meter.scopes,
        "username": meter.user.username,
        "defaultVisibility": meter.user.default_visibility,
        "logs": logs,
        "site": state.config.public_url,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    display_name: Option<String>,
    default_visibility: Option<String>,
    current_password: Option<String>,
    new_password: Option<String>,
}

pub async fn update_account(State(state): State<AppState>, WebUser(user): WebUser, Json(body): Json<AccountUpdate>) -> ApiResult<Json<serde_json::Value>> {
    if let Some(v) = &body.default_visibility {
        if !crate::logs::VISIBILITIES.contains(&v.as_str()) {
            return Err(ApiError::bad_request("Visibility is public, unlisted or private."));
        }
    }
    let new_hash = match (&body.current_password, &body.new_password) {
        (_, None) => None,
        (Some(current), Some(new)) => {
            if new.chars().count() < 8 {
                return Err(ApiError::bad_request("A password is at least 8 characters."));
            }
            let id = user.id;
            let stored: String = state.db.call(move |c| c.query_row("SELECT password_hash FROM users WHERE id = ?1", params![id], |r| r.get(0))).await?;
            if !verify_password(current, &stored) {
                return Err(ApiError::forbidden("The current password is wrong."));
            }
            Some(hash_password(new).map_err(ApiError::internal)?)
        }
        (None, Some(_)) => return Err(ApiError::bad_request("Give the current password to change it.")),
    };
    let display = body.display_name.map(|d| d.trim().chars().take(32).collect::<String>()).filter(|d| !d.is_empty());
    let visibility = body.default_visibility;
    let id = user.id;
    state
        .db
        .call(move |c| {
            if let Some(d) = display {
                c.execute("UPDATE users SET display_name = ?1 WHERE id = ?2", params![d, id])?;
            }
            if let Some(v) = visibility {
                c.execute("UPDATE users SET default_visibility = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(h) = new_hash {
                c.execute("UPDATE users SET password_hash = ?1 WHERE id = ?2", params![h, id])?;
            }
            Ok(())
        })
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// The meters signed in to the browser's account.
pub async fn list_tokens(State(state): State<AppState>, WebUser(user): WebUser) -> ApiResult<Json<serde_json::Value>> {
    let id = user.id;
    let tokens = state
        .db
        .call(move |c| {
            let mut stmt = c.prepare("SELECT id, label, created_at, last_used FROM api_tokens WHERE user_id = ?1 ORDER BY created_at DESC")?;
            let rows = stmt.query_map(params![id], |r| {
                Ok(serde_json::json!({ "id": r.get::<_, i64>(0)?, "label": r.get::<_, String>(1)?,
                                       "createdAt": r.get::<_, i64>(2)?, "lastUsed": r.get::<_, Option<i64>>(3)? }))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(serde_json::json!({ "tokens": tokens })))
}

pub async fn revoke_token(State(state): State<AppState>, WebUser(user): WebUser, axum::extract::Path(token_id): axum::extract::Path<i64>) -> ApiResult<Json<serde_json::Value>> {
    let id = user.id;
    let n = state.db.call(move |c| c.execute("DELETE FROM api_tokens WHERE id = ?1 AND user_id = ?2", params![token_id, id])).await?;
    if n == 0 {
        return Err(ApiError::not_found());
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Make the first admin from `A2_ADMIN_USER` / `A2_ADMIN_PASSWORD` when the
/// site has no account yet, so an install with registration closed can be
/// signed in to at all.
pub async fn bootstrap_admin(state: &AppState) {
    let (Ok(user), Ok(password)) = (std::env::var("A2_ADMIN_USER"), std::env::var("A2_ADMIN_PASSWORD")) else { return };
    let user = user.trim().to_string();
    if !valid_username(&user) || password.chars().count() < 8 {
        tracing::warn!("A2_ADMIN_USER / A2_ADMIN_PASSWORD ignored: username 3-32 characters, password at least 8");
        return;
    }
    let Ok(hash) = hash_password(&password) else { return };
    let made = state
        .db
        .call(move |c| {
            let users: i64 = c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
            if users > 0 {
                return Ok(false);
            }
            c.execute(
                "INSERT INTO users (username, password_hash, display_name, is_admin, created_at) VALUES (?1, ?2, ?1, 1, ?3)",
                params![user, hash, now()],
            )?;
            Ok(true)
        })
        .await;
    if let Ok(true) = made {
        tracing::info!("Created the admin account from A2_ADMIN_USER");
    }
}

// ── admin ──────────────────────────────────────────────────────────────────

pub struct AdminUser(pub User);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let WebUser(user) = WebUser::from_request_parts(parts, state).await?;
        if !user.is_admin {
            return Err(ApiError::forbidden("Admins only."));
        }
        Ok(AdminUser(user))
    }
}

pub async fn admin_users(State(state): State<AppState>, _admin: AdminUser) -> ApiResult<Json<serde_json::Value>> {
    let users = state
        .db
        .call(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {USER_COLUMNS}, created_at, (SELECT COUNT(*) FROM logs WHERE logs.user_id = users.id) FROM users ORDER BY id"
            ))?;
            let rows = stmt.query_map([], |r| {
                let u = user_from_row(r)?;
                Ok(serde_json::json!({ "user": u, "createdAt": r.get::<_, i64>(6)?, "logs": r.get::<_, i64>(7)? }))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(serde_json::json!({ "users": users })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserUpdate {
    is_admin: Option<bool>,
    supporter: Option<bool>,
    new_password: Option<String>,
    delete: Option<bool>,
}

pub async fn admin_update_user(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    axum::extract::Path(user_id): axum::extract::Path<i64>,
    Json(body): Json<AdminUserUpdate>,
) -> ApiResult<Json<serde_json::Value>> {
    if user_id == admin.id && (body.delete == Some(true) || body.is_admin == Some(false)) {
        return Err(ApiError::bad_request("You cannot remove your own admin account."));
    }
    let new_hash = match &body.new_password {
        Some(p) if p.chars().count() < 8 => return Err(ApiError::bad_request("A password is at least 8 characters.")),
        Some(p) => Some(hash_password(p).map_err(ApiError::internal)?),
        None => None,
    };
    let slice_ids: Vec<String> = state
        .db
        .call(move |c| {
            let mut ids = Vec::new();
            if body.delete == Some(true) {
                let mut stmt = c.prepare("SELECT id FROM logs WHERE user_id = ?1")?;
                ids = stmt.query_map(params![user_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
                c.execute("DELETE FROM users WHERE id = ?1", params![user_id])?;
                return Ok(ids);
            }
            if let Some(a) = body.is_admin {
                c.execute("UPDATE users SET is_admin = ?1 WHERE id = ?2", params![a as i64, user_id])?;
            }
            if let Some(s) = body.supporter {
                c.execute("UPDATE users SET supporter = ?1 WHERE id = ?2", params![s as i64, user_id])?;
            }
            if let Some(h) = new_hash {
                c.execute("UPDATE users SET password_hash = ?1 WHERE id = ?2", params![h, user_id])?;
                c.execute("DELETE FROM sessions WHERE user_id = ?1", params![user_id])?;
            }
            Ok(ids)
        })
        .await?;
    for id in slice_ids {
        state.store.remove_log(&id);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
