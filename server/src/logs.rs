//! Fight logs: the meter's upload (`share::upload_detailed`), the log pages,
//! the leaderboards and the class stats.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use base64::Engine;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;

use a2tools_dps_meter_lib::entity::fight_record::obscure_nickname;
use a2tools_dps_meter_lib::rederive::DerivedFight;

use crate::auth::{session_user, MeterUser, User, WebUser};
use crate::db::now;
use crate::derive;
use crate::util::{random_id, sha256_hex, ApiError, ApiResult};
use crate::AppState;

pub const VISIBILITIES: &[&str] = &["public", "unlisted", "private"];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    slice: String,
    #[serde(default)]
    names: HashMap<String, String>,
    #[serde(default)]
    uploader_actor_id: Option<i64>,
    #[serde(default)]
    fight_start_ms: i64,
    #[serde(default)]
    app_version: String,
    #[serde(default)]
    ui_language: String,
}

/// The names a log shows: the uploader's own as sent, everyone else masked,
/// whatever the client did (the meter masks them already).
fn display_names(names: &HashMap<String, String>, uploader: Option<i64>) -> HashMap<i32, String> {
    names
        .iter()
        .filter_map(|(id, name)| {
            let id: i32 = id.parse().ok()?;
            let name: String = name.trim().chars().take(32).collect();
            if name.is_empty() {
                return None;
            }
            let shown = if Some(id as i64) == uploader || name.contains('*') { name } else { obscure_nickname(&name) };
            Some((id, shown))
        })
        .collect()
}

/// Whether a log counts for leaderboards and class stats.
fn is_ranked(fight: &DerivedFight) -> bool {
    let c = &fight.checks;
    let whole = c.max_hp <= 0 || c.damage * 100 >= c.max_hp * 95;
    let framed = c.records == 0 || c.lifted * 2 < c.records;
    c.killed && !fight.record.is_train && whole && framed && c.unblinded_names == 0
}

struct Derived {
    stored: serde_json::Value,
    players: Vec<(i32, String, i32, i64, f64, bool)>,
    ranked: bool,
    checks: serde_json::Value,
}

/// The record as the site keeps it: the derived fight with the names applied.
fn prepare(fight: DerivedFight, leaked: usize, names: &HashMap<i32, String>, uploader: Option<i64>) -> Derived {
    let ranked = is_ranked(&fight) && leaked == 0;
    let mut record = fight.record;
    for actor in &mut record.actors {
        actor.nickname = names.get(&actor.actor_id).cloned().unwrap_or_else(|| format!("Player {}", actor.actor_id));
        actor.dbid = 0;
    }
    let mut damage: HashMap<i32, i64> = HashMap::new();
    for s in &record.details.skills {
        *damage.entry(s.actor_id).or_default() += s.dmg as i64;
    }
    let seconds = (record.duration_ms as f64 / 1000.0).max(0.001);
    let players = record
        .actors
        .iter()
        .map(|a| {
            let d = damage.get(&a.actor_id).copied().unwrap_or(0);
            (a.actor_id, a.nickname.clone(), a.job_id, d, d as f64 / seconds, Some(a.actor_id as i64) == uploader)
        })
        .collect();
    let mut checks = serde_json::to_value(&fight.checks).unwrap_or_default();
    checks["leakedNames"] = serde_json::json!(leaked);
    let stored = serde_json::json!({
        "parserVersion": fight.parser_version,
        "totalDamage": fight.total_damage,
        "record": record,
        "checks": checks,
    });
    Derived { stored, players, ranked, checks }
}

pub async fn upload(State(state): State<AppState>, meter: MeterUser, Json(body): Json<Upload>) -> ApiResult<Json<serde_json::Value>> {
    meter.require("logs")?;
    let user = meter.user;
    if !state.limiter.allow(&format!("upload:{}", user.id), state.config.uploads_per_hour, std::time::Duration::from_secs(3600)) {
        return Err(ApiError::too_many());
    }
    let slice = base64::engine::general_purpose::STANDARD
        .decode(body.slice.trim())
        .map_err(|_| ApiError::bad_request("The slice is not base64."))?;
    if slice.len() > derive::MAX_COMPRESSED {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", "The recording is larger than any real fight."));
    }
    let sha = sha256_hex(&slice);
    let user_id = user.id;
    let sha_q = sha.clone();
    let existing: Option<(String, String)> = state
        .db
        .call(move |c| {
            c.query_row("SELECT id, visibility FROM logs WHERE user_id = ?1 AND slice_sha256 = ?2", params![user_id, sha_q], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
        })
        .await?;
    if let Some((id, visibility)) = existing {
        return Ok(Json(serde_json::json!({ "url": state.log_url(&id), "visibility": visibility, "duplicate": true, "id": id })));
    }

    let uploader = body.uploader_actor_id;
    let names = display_names(&body.names, uploader);
    let plain: Vec<String> = body.names.values().filter(|n| !n.contains('*')).cloned().collect();
    let names_for_derive = names.clone();
    let slice_for_derive = slice.clone();
    let derived = tokio::task::spawn_blocking(move || {
        derive::derive(&slice_for_derive, &plain).map(|(fight, leaked)| prepare(fight, leaked, &names_for_derive, uploader))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(|f| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, match f {
        derive::Failure::NotASlice => "not_a_slice",
        derive::Failure::NoFight => "no_fight",
    }, f.message()))?;

    let id = random_id(10);
    let record_json = serde_json::to_vec(&derived.stored).map_err(ApiError::internal)?;
    {
        let store = state.store.clone();
        let (id, slice) = (id.clone(), slice.clone());
        tokio::task::spawn_blocking(move || store.write_log(&id, &slice, &record_json)).await.map_err(ApiError::internal)?.map_err(ApiError::internal)?;
    }

    let visibility = user.default_visibility.clone();
    let r = &derived.stored["record"];
    let row = LogRow {
        id: id.clone(),
        user_id,
        visibility: visibility.clone(),
        fight_start_ms: if body.fight_start_ms > 0 { body.fight_start_ms } else { r["startTimeMs"].as_i64().unwrap_or(0) },
        mob_code: r["mobCode"].as_i64().unwrap_or(0),
        boss_name: r["bossName"].as_str().unwrap_or("").to_string(),
        dungeon_id: r["dungeonId"].as_i64().unwrap_or(0),
        server_id: r["serverId"].as_i64().unwrap_or(0),
        duration_ms: r["durationMs"].as_i64().unwrap_or(0),
        total_damage: derived.stored["totalDamage"].as_i64().unwrap_or(0),
        max_hp: derived.checks["maxHp"].as_i64().unwrap_or(0),
        killed: derived.checks["killed"].as_bool().unwrap_or(false),
        is_train: r["isTrain"].as_bool().unwrap_or(false),
        parser_version: derived.stored["parserVersion"].as_str().unwrap_or("").to_string(),
        app_version: body.app_version.chars().take(32).collect(),
        ui_language: body.ui_language.chars().take(16).collect(),
        slice_sha256: sha,
        uploader_actor: uploader,
        ranked: derived.ranked,
        checks_json: derived.checks.to_string(),
        names_json: serde_json::to_string(&names).unwrap_or_default(),
    };
    let players = derived.players;
    state.db.call(move |c| insert_log(c, &row, &players)).await?;
    tracing::info!("Log {id} uploaded by {} ({})", user.username, r["bossName"].as_str().unwrap_or("?"));
    Ok(Json(serde_json::json!({ "url": state.log_url(&id), "visibility": visibility, "duplicate": false, "id": id })))
}

struct LogRow {
    id: String,
    user_id: i64,
    visibility: String,
    fight_start_ms: i64,
    mob_code: i64,
    boss_name: String,
    dungeon_id: i64,
    server_id: i64,
    duration_ms: i64,
    total_damage: i64,
    max_hp: i64,
    killed: bool,
    is_train: bool,
    parser_version: String,
    app_version: String,
    ui_language: String,
    slice_sha256: String,
    uploader_actor: Option<i64>,
    ranked: bool,
    checks_json: String,
    names_json: String,
}

fn insert_log(c: &mut rusqlite::Connection, row: &LogRow, players: &[(i32, String, i32, i64, f64, bool)]) -> rusqlite::Result<()> {
    let tx = c.transaction()?;
    tx.execute(
        "INSERT INTO logs (id, user_id, created_at, visibility, fight_start_ms, mob_code, boss_name, dungeon_id, server_id,
                           duration_ms, total_damage, max_hp, killed, is_train, party_size, parser_version, app_version,
                           ui_language, slice_sha256, uploader_actor, ranked, checks_json, names_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
        params![
            row.id, row.user_id, now(), row.visibility, row.fight_start_ms, row.mob_code, row.boss_name, row.dungeon_id,
            row.server_id, row.duration_ms, row.total_damage, row.max_hp, row.killed as i64, row.is_train as i64,
            players.len() as i64, row.parser_version, row.app_version, row.ui_language, row.slice_sha256,
            row.uploader_actor, row.ranked as i64, row.checks_json, row.names_json
        ],
    )?;
    replace_players(&tx, &row.id, players)?;
    tx.commit()
}

fn replace_players(tx: &rusqlite::Transaction<'_>, id: &str, players: &[(i32, String, i32, i64, f64, bool)]) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM log_players WHERE log_id = ?1", params![id])?;
    let mut stmt = tx.prepare("INSERT INTO log_players (log_id, actor_id, name, job_id, damage, dps, is_uploader) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?;
    for (actor, name, job, damage, dps, up) in players {
        stmt.execute(params![id, actor, name, job, damage, dps, *up as i64])?;
    }
    Ok(())
}

// ── reading ────────────────────────────────────────────────────────────────

const SUMMARY_COLUMNS: &str = "logs.id, logs.created_at, logs.visibility, logs.fight_start_ms, logs.mob_code, logs.boss_name,
    logs.dungeon_id, logs.server_id, logs.duration_ms, logs.total_damage, logs.killed, logs.is_train, logs.party_size,
    logs.ranked, logs.user_id, users.display_name, logs.parser_version, logs.max_hp, logs.uploader_actor";

fn summary(r: &rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?,
        "createdAt": r.get::<_, i64>(1)?,
        "visibility": r.get::<_, String>(2)?,
        "fightStartMs": r.get::<_, i64>(3)?,
        "mobCode": r.get::<_, i64>(4)?,
        "bossName": r.get::<_, String>(5)?,
        "dungeonId": r.get::<_, i64>(6)?,
        "serverId": r.get::<_, i64>(7)?,
        "durationMs": r.get::<_, i64>(8)?,
        "totalDamage": r.get::<_, i64>(9)?,
        "killed": r.get::<_, i64>(10)? != 0,
        "isTrain": r.get::<_, i64>(11)? != 0,
        "partySize": r.get::<_, i64>(12)?,
        "ranked": r.get::<_, i64>(13)? != 0,
        "userId": r.get::<_, i64>(14)?,
        "uploader": r.get::<_, String>(15)?,
        "parserVersion": r.get::<_, String>(16)?,
        "maxHp": r.get::<_, i64>(17)?,
        "uploaderActor": r.get::<_, Option<i64>>(18)?,
    }))
}

#[derive(Deserialize)]
pub struct ListQuery {
    boss: Option<i64>,
    job: Option<i64>,
    mine: Option<bool>,
    page: Option<i64>,
}

const PAGE: i64 = 50;

pub async fn list(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<ListQuery>) -> ApiResult<Json<serde_json::Value>> {
    let viewer = session_user(&state, &headers).await?;
    let mine = q.mine.unwrap_or(false);
    if mine && viewer.is_none() {
        return Err(ApiError::unauthorized());
    }
    let viewer_id = viewer.as_ref().map(|u| u.id).unwrap_or(-1);
    let page = q.page.unwrap_or(0).max(0);
    let (logs, total) = state
        .db
        .call(move |c| {
            let mut wh = vec![if mine { "logs.user_id = :viewer".to_string() } else { "logs.visibility = 'public'".to_string() }];
            if q.boss.is_some() {
                wh.push("logs.mob_code = :boss".into());
            }
            if q.job.is_some() {
                wh.push("EXISTS (SELECT 1 FROM log_players p WHERE p.log_id = logs.id AND p.job_id = :job)".into());
            }
            let where_sql = wh.join(" AND ");
            let sql = format!(
                "SELECT {SUMMARY_COLUMNS} FROM logs JOIN users ON users.id = logs.user_id WHERE {where_sql}
                 ORDER BY logs.created_at DESC LIMIT :limit OFFSET :offset"
            );
            let mut named: Vec<(&str, &dyn rusqlite::ToSql)> = Vec::new();
            let limit = PAGE;
            let offset = page * PAGE;
            let boss = q.boss.unwrap_or(0);
            let job = q.job.unwrap_or(0);
            if mine {
                named.push((":viewer", &viewer_id));
            }
            if q.boss.is_some() {
                named.push((":boss", &boss));
            }
            if q.job.is_some() {
                named.push((":job", &job));
            }
            let count_sql = format!("SELECT COUNT(*) FROM logs WHERE {where_sql}");
            let total: i64 = c.query_row(&count_sql, &named[..], |r| r.get(0))?;
            named.push((":limit", &limit));
            named.push((":offset", &offset));
            let mut stmt = c.prepare(&sql)?;
            let mut logs = stmt.query_map(&named[..], summary)?.collect::<rusqlite::Result<Vec<_>>>()?;
            // Each log's players, for the list's class icons and top DPS.
            let mut pstmt = c.prepare("SELECT name, job_id, dps, is_uploader FROM log_players WHERE log_id = ?1 ORDER BY damage DESC")?;
            for log in &mut logs {
                let id = log["id"].as_str().unwrap_or_default().to_string();
                let players = pstmt
                    .query_map(params![id], |r| {
                        Ok(serde_json::json!({ "name": r.get::<_, String>(0)?, "jobId": r.get::<_, i64>(1)?,
                                               "dps": r.get::<_, f64>(2)?, "isUploader": r.get::<_, i64>(3)? != 0 }))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                log["players"] = serde_json::json!(players);
            }
            Ok((logs, total))
        })
        .await?;
    Ok(Json(serde_json::json!({ "logs": logs, "total": total, "page": page, "pageSize": PAGE })))
}

async fn find(state: &AppState, id: &str) -> ApiResult<serde_json::Value> {
    let id = id.to_string();
    state
        .db
        .call(move |c| {
            c.query_row(&format!("SELECT {SUMMARY_COLUMNS} FROM logs JOIN users ON users.id = logs.user_id WHERE logs.id = ?1"), params![id], summary)
                .optional()
        })
        .await?
        .ok_or_else(ApiError::not_found)
}

fn may_see(log: &serde_json::Value, viewer: Option<&User>) -> bool {
    log["visibility"] != "private" || viewer.is_some_and(|u| u.is_admin || Some(u.id) == log["userId"].as_i64())
}

pub async fn get(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let viewer = session_user(&state, &headers).await?;
    let mut log = find(&state, &id).await?;
    if !may_see(&log, viewer.as_ref()) {
        return Err(ApiError::not_found());
    }
    let store = state.store.clone();
    let record = tokio::task::spawn_blocking(move || store.read_record(&id)).await.map_err(ApiError::internal)?;
    let record: serde_json::Value = record.and_then(|r| serde_json::from_slice(&r).ok()).ok_or_else(ApiError::not_found)?;
    log["fight"] = record;
    log["mine"] = serde_json::json!(viewer.as_ref().is_some_and(|u| Some(u.id) == log["userId"].as_i64()));
    log["canDelete"] = serde_json::json!(viewer.as_ref().is_some_and(|u| u.is_admin || Some(u.id) == log["userId"].as_i64()));
    Ok(Json(log))
}

#[derive(Deserialize)]
pub struct LogUpdate {
    visibility: String,
}

pub async fn update(State(state): State<AppState>, WebUser(user): WebUser, Path(id): Path<String>, Json(body): Json<LogUpdate>) -> ApiResult<Json<serde_json::Value>> {
    if !VISIBILITIES.contains(&body.visibility.as_str()) {
        return Err(ApiError::bad_request("Visibility is public, unlisted or private."));
    }
    let n = state
        .db
        .call(move |c| c.execute("UPDATE logs SET visibility = ?1 WHERE id = ?2 AND (user_id = ?3 OR ?4)", params![body.visibility, id, user.id, user.is_admin]))
        .await?;
    if n == 0 {
        return Err(ApiError::not_found());
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn delete(State(state): State<AppState>, WebUser(user): WebUser, Path(id): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let lid = id.clone();
    let n = state
        .db
        .call(move |c| c.execute("DELETE FROM logs WHERE id = ?1 AND (user_id = ?2 OR ?3)", params![lid, user.id, user.is_admin]))
        .await?;
    if n == 0 {
        return Err(ApiError::not_found());
    }
    state.store.remove_log(&id);
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// GET /api/logs/:id/slice: the Evidence Slice itself, so anyone may check a
/// log against the published parser. Same visibility as the log.
pub async fn slice(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let viewer = session_user(&state, &headers).await?;
    let log = find(&state, &id).await?;
    if !may_see(&log, viewer.as_ref()) {
        return Err(ApiError::not_found());
    }
    let store = state.store.clone();
    let bytes = tokio::task::spawn_blocking(move || store.read_slice(&id)).await.map_err(ApiError::internal)?.ok_or_else(ApiError::not_found)?;
    let name = format!("attachment; filename=\"{}.a2es\"", log["id"].as_str().unwrap_or("log"));
    Ok(([("content-type", "application/octet-stream".to_string()), ("content-disposition", name)], bytes).into_response())
}

// ── leaderboards and class stats ───────────────────────────────────────────

/// Bosses with ranked public logs, for the pickers.
pub async fn bosses(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let bosses = state
        .db
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT mob_code, boss_name, dungeon_id, COUNT(*), SUM(ranked), MAX(created_at) FROM logs
                 WHERE visibility = 'public' GROUP BY mob_code ORDER BY MAX(created_at) DESC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(serde_json::json!({ "mobCode": r.get::<_, i64>(0)?, "bossName": r.get::<_, String>(1)?,
                    "dungeonId": r.get::<_, i64>(2)?, "logs": r.get::<_, i64>(3)?, "ranked": r.get::<_, i64>(4)?,
                    "latest": r.get::<_, i64>(5)? }))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(serde_json::json!({ "bosses": bosses })))
}

#[derive(Deserialize)]
pub struct BoardQuery {
    boss: i64,
    job: Option<i64>,
    days: Option<i64>,
    /// "dps" (default) or "speed" (fastest kills).
    by: Option<String>,
}

pub async fn leaderboard(State(state): State<AppState>, Query(q): Query<BoardQuery>) -> ApiResult<Json<serde_json::Value>> {
    let since = q.days.filter(|d| *d > 0).map(|d| now() - d * 86_400).unwrap_or(0);
    let by_speed = q.by.as_deref() == Some("speed");
    let rows = state
        .db
        .call(move |c| {
            if by_speed {
                let mut stmt = c.prepare(&format!(
                    "SELECT {SUMMARY_COLUMNS} FROM logs JOIN users ON users.id = logs.user_id
                     WHERE logs.visibility = 'public' AND logs.ranked = 1 AND logs.mob_code = ?1 AND logs.created_at >= ?2
                     ORDER BY logs.duration_ms ASC LIMIT 100"
                ))?;
                let mut logs = stmt.query_map(params![q.boss, since], summary)?.collect::<rusqlite::Result<Vec<_>>>()?;
                let mut pstmt = c.prepare("SELECT name, job_id, dps FROM log_players WHERE log_id = ?1 ORDER BY damage DESC")?;
                for log in &mut logs {
                    let id = log["id"].as_str().unwrap_or_default().to_string();
                    let players = pstmt
                        .query_map(params![id], |r| Ok(serde_json::json!({ "name": r.get::<_, String>(0)?, "jobId": r.get::<_, i64>(1)?, "dps": r.get::<_, f64>(2)? })))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    log["players"] = serde_json::json!(players);
                }
                return Ok(logs);
            }
            let job_clause = if q.job.is_some() { "AND p.job_id = ?3" } else { "AND ?3 IS NULL" };
            let mut stmt = c.prepare(&format!(
                "SELECT p.name, p.job_id, p.dps, p.damage, p.is_uploader, logs.id, logs.duration_ms, logs.created_at,
                        logs.fight_start_ms, logs.party_size, users.display_name
                 FROM log_players p JOIN logs ON logs.id = p.log_id JOIN users ON users.id = logs.user_id
                 WHERE logs.visibility = 'public' AND logs.ranked = 1 AND logs.mob_code = ?1 AND logs.created_at >= ?2 {job_clause}
                   AND p.job_id > 0
                 ORDER BY p.dps DESC LIMIT 100"
            ))?;
            let rows = stmt.query_map(params![q.boss, since, q.job], |r| {
                Ok(serde_json::json!({
                    "name": r.get::<_, String>(0)?, "jobId": r.get::<_, i64>(1)?, "dps": r.get::<_, f64>(2)?,
                    "damage": r.get::<_, i64>(3)?, "isUploader": r.get::<_, i64>(4)? != 0, "logId": r.get::<_, String>(5)?,
                    "durationMs": r.get::<_, i64>(6)?, "createdAt": r.get::<_, i64>(7)?, "fightStartMs": r.get::<_, i64>(8)?,
                    "partySize": r.get::<_, i64>(9)?, "uploader": r.get::<_, String>(10)?,
                }))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(serde_json::json!({ "rows": rows, "by": if by_speed { "speed" } else { "dps" } })))
}

#[derive(Deserialize)]
pub struct StatsQuery {
    boss: Option<i64>,
    days: Option<i64>,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = p * (sorted.len() - 1) as f64;
    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64)
}

/// Per class: how many players, and their DPS spread, over ranked public logs.
pub async fn class_stats(State(state): State<AppState>, Query(q): Query<StatsQuery>) -> ApiResult<Json<serde_json::Value>> {
    let since = q.days.filter(|d| *d > 0).map(|d| now() - d * 86_400).unwrap_or(0);
    let by_job: HashMap<i64, Vec<f64>> = state
        .db
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT p.job_id, p.dps FROM log_players p JOIN logs ON logs.id = p.log_id
                 WHERE logs.visibility = 'public' AND logs.ranked = 1 AND logs.created_at >= ?1
                   AND (?2 IS NULL OR logs.mob_code = ?2) AND p.job_id > 0",
            )?;
            let mut out: HashMap<i64, Vec<f64>> = HashMap::new();
            for row in stmt.query_map(params![since, q.boss], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))? {
                let (job, dps) = row?;
                out.entry(job).or_default().push(dps);
            }
            Ok(out)
        })
        .await?;
    let mut classes: Vec<serde_json::Value> = by_job
        .into_iter()
        .map(|(job, mut v)| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            serde_json::json!({
                "jobId": job, "count": v.len(), "mean": mean,
                "p25": percentile(&v, 0.25), "median": percentile(&v, 0.5), "p75": percentile(&v, 0.75),
                "p95": percentile(&v, 0.95), "max": v.last().copied().unwrap_or(0.0),
            })
        })
        .collect();
    classes.sort_by(|a, b| b["median"].as_f64().partial_cmp(&a["median"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
    Ok(Json(serde_json::json!({ "classes": classes })))
}

/// Site totals for the home page.
pub async fn overview(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let (logs, players, users, recent) = state
        .db
        .call(|c| {
            let logs: i64 = c.query_row("SELECT COUNT(*) FROM logs WHERE visibility = 'public'", [], |r| r.get(0))?;
            let players: i64 = c.query_row("SELECT COUNT(*) FROM log_players p JOIN logs ON logs.id = p.log_id WHERE logs.visibility = 'public'", [], |r| r.get(0))?;
            let users: i64 = c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
            let mut stmt = c.prepare(&format!(
                "SELECT {SUMMARY_COLUMNS} FROM logs JOIN users ON users.id = logs.user_id WHERE logs.visibility = 'public'
                 ORDER BY logs.created_at DESC LIMIT 8"
            ))?;
            let recent = stmt.query_map([], summary)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((logs, players, users, recent))
        })
        .await?;
    Ok(Json(serde_json::json!({ "logs": logs, "players": players, "users": users, "recent": recent, "parserVersion": derive::parser_version() })))
}

/// Re-derive every log whose parser is older than this build's, from its
/// kept slice, the way a2tools.app does when the parser changes. Admin only.
pub async fn rederive_all(State(state): State<AppState>, _admin: crate::auth::AdminUser) -> ApiResult<Json<serde_json::Value>> {
    let current = derive::parser_version();
    let cur = current.clone();
    let todo: Vec<(String, Option<i64>, String)> = state
        .db
        .call(move |c| {
            let mut stmt = c.prepare("SELECT id, uploader_actor, names_json FROM logs WHERE parser_version != ?1")?;
            let rows = stmt.query_map(params![cur], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await?;
    let (mut updated, mut failed) = (0usize, 0usize);
    for (id, uploader, names_json) in todo {
        let store = state.store.clone();
        let lid = id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let slice = store.read_slice(&lid)?;
            let names: HashMap<i32, String> = serde_json::from_str(&names_json).unwrap_or_default();
            let (fight, leaked) = derive::derive(&slice, &[]).ok()?;
            let d = prepare(fight, leaked, &names, uploader);
            store.write_record(&lid, &serde_json::to_vec(&d.stored).ok()?).ok()?;
            Some(d)
        })
        .await
        .map_err(ApiError::internal)?;
        let Some(d) = result else {
            failed += 1;
            continue;
        };
        let pv = d.stored["parserVersion"].as_str().unwrap_or("").to_string();
        let total = d.stored["totalDamage"].as_i64().unwrap_or(0);
        state
            .db
            .call(move |c| {
                let tx = c.transaction()?;
                tx.execute(
                    "UPDATE logs SET parser_version = ?1, total_damage = ?2, ranked = ?3, checks_json = ?4, party_size = ?5 WHERE id = ?6",
                    params![pv, total, d.ranked as i64, d.checks.to_string(), d.players.len() as i64, id],
                )?;
                replace_players(&tx, &id, &d.players)?;
                tx.commit()
            })
            .await?;
        updated += 1;
    }
    Ok(Json(serde_json::json!({ "parserVersion": current, "updated": updated, "failed": failed })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everyone_but_the_uploader_is_masked() {
        let names: HashMap<String, String> = [
            ("10".to_string(), "Charles".to_string()),
            ("11".to_string(), "Naicha".to_string()),
            ("12".to_string(), "Ts**i".to_string()),
            ("x".to_string(), "Bad".to_string()),
            ("13".to_string(), "  ".to_string()),
        ]
        .into();
        let shown = display_names(&names, Some(10));
        assert_eq!(shown[&10], "Charles");
        assert_eq!(shown[&11], "Na***a");
        // Already masked by the meter: not masked again.
        assert_eq!(shown[&12], "Ts**i");
        assert_eq!(shown.len(), 3);
    }

    #[test]
    fn percentiles_interpolate() {
        let v = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&v, 0.0), 10.0);
        assert_eq!(percentile(&v, 1.0), 40.0);
        assert_eq!(percentile(&v, 0.5), 25.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }
}
