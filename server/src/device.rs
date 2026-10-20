//! The Device Authorization Grant (RFC 8628) the meter signs in with
//! (`src-tauri/src/account/mod.rs`): the meter asks for a code, the player
//! approves it on /link while signed in to the site, the meter polls for its
//! token.

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;

use crate::auth::{WebUser, METER_SCOPES};
use crate::db::now;
use crate::util::{random_code, random_token, sha256_hex, ApiError, ApiResult};
use crate::AppState;

const EXPIRES_IN: i64 = 15 * 60;
const INTERVAL: i64 = 5;

#[derive(Deserialize)]
pub struct CodeRequest {
    #[serde(default)]
    client: String,
    #[serde(default)]
    device_label: String,
}

pub async fn code(
    State(state): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<CodeRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let ip = crate::util::client_ip(&headers, peer, state.config.trust_proxy);
    if !state.limiter.allow(&format!("device:{ip}"), 20, std::time::Duration::from_secs(3600)) {
        return Err(ApiError::too_many());
    }
    let client: String = body.client.trim().chars().take(32).collect();
    let label: String = body.device_label.trim().chars().take(80).collect();
    let device_code = random_token(32);
    let device_hash = sha256_hex(device_code.as_bytes());
    let raw = random_code(8);
    let user_code = format!("{}-{}", &raw[..4], &raw[4..]);
    let t = now();
    let uc = user_code.clone();
    state
        .db
        .call(move |c| {
            c.execute("DELETE FROM device_codes WHERE expires_at < ?1", params![t - 3600])?;
            c.execute(
                "INSERT INTO device_codes (device_hash, user_code, client, label, created_at, expires_at, interval_s)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![device_hash, uc, if client.is_empty() { "dps-meter".into() } else { client }, label, t, t + EXPIRES_IN, INTERVAL],
            )
        })
        .await?;
    let base = &state.config.public_url;
    Ok(Json(serde_json::json!({
        "device_code": device_code,
        "user_code": user_code,
        "expires_in": EXPIRES_IN,
        "interval": INTERVAL,
        "verification_uri": format!("{base}/link"),
        "verification_uri_complete": format!("{base}/link?code={user_code}"),
    })))
}

#[derive(Deserialize)]
pub struct TokenRequest {
    device_code: String,
}

fn oauth_error(status: StatusCode, error: &str) -> Response {
    (status, Json(serde_json::json!({ "error": error }))).into_response()
}

pub async fn token(State(state): State<AppState>, Json(body): Json<TokenRequest>) -> ApiResult<Response> {
    let device_hash = sha256_hex(body.device_code.trim().as_bytes());
    let t = now();
    let outcome = state
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let row: Option<(String, i64, i64, i64, Option<i64>, String)> = tx
                .query_row(
                    "SELECT status, expires_at, interval_s, last_poll, user_id, label FROM device_codes WHERE device_hash = ?1",
                    params![device_hash],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .optional()?;
            let Some((status, expires_at, interval, last_poll, user_id, label)) = row else {
                return Ok(Err("expired_token"));
            };
            if status == "issued" || expires_at < t {
                return Ok(Err("expired_token"));
            }
            if status == "denied" {
                return Ok(Err("access_denied"));
            }
            // A poll sooner than the interval allows (a second's leeway for
            // timers) is told to slow down, as RFC 8628 says.
            let too_soon = last_poll > 0 && t - last_poll < interval - 1;
            tx.execute("UPDATE device_codes SET last_poll = ?1 WHERE device_hash = ?2", params![t, device_hash])?;
            if status == "pending" {
                tx.commit()?;
                return Ok(Err(if too_soon { "slow_down" } else { "authorization_pending" }));
            }
            let user_id = user_id.expect("approved code has a user");
            let token = random_token(32);
            tx.execute(
                "INSERT INTO api_tokens (token_hash, user_id, label, scopes, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![sha256_hex(token.as_bytes()), user_id, if label.is_empty() { "DPS meter".to_string() } else { label }, METER_SCOPES.join(" "), t],
            )?;
            tx.execute("UPDATE device_codes SET status = 'issued' WHERE device_hash = ?1", params![device_hash])?;
            tx.commit()?;
            Ok(Ok(token))
        })
        .await?;
    Ok(match outcome {
        Ok(token) => Json(serde_json::json!({
            "access_token": token,
            "token_type": "bearer",
            "scope": METER_SCOPES.join(" "),
        }))
        .into_response(),
        Err("authorization_pending") => oauth_error(StatusCode::PRECONDITION_REQUIRED, "authorization_pending"),
        Err(other) => oauth_error(StatusCode::BAD_REQUEST, other),
    })
}

fn normalise_code(code: &str) -> String {
    let raw: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    if raw.len() == 8 { format!("{}-{}", &raw[..4], &raw[4..]) } else { raw }
}

#[derive(Deserialize)]
pub struct LookupQuery {
    code: String,
}

/// What the /link page shows before the player approves: which device asks.
pub async fn lookup(State(state): State<AppState>, _user: WebUser, Query(q): Query<LookupQuery>) -> ApiResult<Json<serde_json::Value>> {
    let code = normalise_code(&q.code);
    let t = now();
    let found = state
        .db
        .call(move |c| {
            c.query_row(
                "SELECT client, label, status, expires_at FROM device_codes WHERE user_code = ?1",
                params![code],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)),
            )
            .optional()
        })
        .await?;
    match found {
        Some((client, label, status, expires_at)) if expires_at >= t && status == "pending" => {
            Ok(Json(serde_json::json!({ "client": client, "label": label, "expiresIn": expires_at - t })))
        }
        Some((_, _, status, _)) if status != "pending" => Err(ApiError::new(StatusCode::GONE, "used", "This code was already used.")),
        _ => Err(ApiError::new(StatusCode::NOT_FOUND, "unknown_code", "No such code, or it has expired. Start the sign-in again in the meter.")),
    }
}

#[derive(Deserialize)]
pub struct Decision {
    code: String,
    approve: bool,
}

pub async fn decide(State(state): State<AppState>, WebUser(user): WebUser, Json(body): Json<Decision>) -> ApiResult<Json<serde_json::Value>> {
    let code = normalise_code(&body.code);
    let t = now();
    let status = if body.approve { "approved" } else { "denied" };
    let n = state
        .db
        .call(move |c| {
            c.execute(
                "UPDATE device_codes SET status = ?1, user_id = ?2
                 WHERE user_code = ?3 AND status = 'pending' AND expires_at >= ?4",
                params![status, user.id, code, t],
            )
        })
        .await?;
    if n == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "unknown_code", "No such code, or it has expired. Start the sign-in again in the meter."));
    }
    Ok(Json(serde_json::json!({ "ok": true, "status": status })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_read_however_they_are_typed() {
        assert_eq!(normalise_code("abcd-efgh"), "ABCD-EFGH");
        assert_eq!(normalise_code(" ABCDEFGH "), "ABCD-EFGH");
        assert_eq!(normalise_code("ab cd ef gh"), "ABCD-EFGH");
        assert_eq!(normalise_code("short"), "SHORT");
    }
}
