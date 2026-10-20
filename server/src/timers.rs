//! Field boss timers, as players' meters report them (`boss_report.rs`):
//! what each server's map shows, kills as they happen and HP while a boss is
//! fought, combined per server and published for everyone: the feed file
//! plugins read (`feeds/boss-timers/<server>.json`), a live WebSocket for
//! plugins that follow bosses (`plugins/boss_live.rs`), and the site's page.
//!
//! An entry is confirmed when two different reporters agree on it. Reporters
//! are told apart by a hash of their address with a salt that changes daily;
//! no address is kept.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use a2tools_dps_meter_lib::capture::field_bosses::FIELD_BOSSES;

use crate::db::now;
use crate::util::{random_token, sha256_hex, ApiError, ApiResult};
use crate::AppState;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// "up" or "dead".
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up_since: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_spawn: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub died_at: Option<i64>,
    pub reported_at: i64,
    pub confirmed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hp_at: Option<i64>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub reset: bool,
    /// Who agreed on this state. Never published.
    #[serde(skip_serializing, default)]
    reporters: HashSet<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct ServerTimers {
    bosses: HashMap<String, Entry>,
    /// Abyss cores, by NPC code, as last reported.
    #[serde(default)]
    cores: HashMap<String, Value>,
    /// The last artifacts report, whole.
    #[serde(default)]
    artifacts: Option<Value>,
    #[serde(default)]
    updated_at: i64,
}

pub struct Timers {
    servers: Mutex<HashMap<u32, ServerTimers>>,
    live: Mutex<HashMap<u32, broadcast::Sender<(String, Value)>>>,
    salt: Mutex<(i64, String)>,
    file: PathBuf,
    dirty: std::sync::atomic::AtomicBool,
}

fn valid_server(server: i64) -> Option<u32> {
    (1000..10000).contains(&server).then_some(server as u32)
}

fn known_boss(id: &str) -> bool {
    id.parse::<u32>().is_ok_and(|id| FIELD_BOSSES.iter().any(|(_, w)| *w == id))
}

impl Timers {
    pub fn load(file: PathBuf) -> Self {
        let servers = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self {
            servers: Mutex::new(servers),
            live: Mutex::new(HashMap::new()),
            salt: Mutex::new((0, String::new())),
            file,
            dirty: Default::default(),
        }
    }

    /// Written every so often by `persist_loop`, not on every report.
    pub fn save(&self) {
        if !self.dirty.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let bytes = {
            let servers = self.servers.lock().unwrap_or_else(|p| p.into_inner());
            serde_json::to_vec(&*servers).unwrap_or_default()
        };
        let tmp = self.file.with_extension("tmp");
        if std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &self.file)).is_err() {
            tracing::warn!("could not save the boss timers");
        }
    }

    fn reporter(&self, ip: std::net::IpAddr) -> String {
        let day = now() / 86_400;
        let mut salt = self.salt.lock().unwrap_or_else(|p| p.into_inner());
        if salt.0 != day {
            *salt = (day, random_token(16));
        }
        sha256_hex(format!("{}|{ip}", salt.1).as_bytes())[..16].to_string()
    }

    fn publish(&self, server: u32, id: &str, entry: &Entry) {
        if let Some(tx) = self.live.lock().unwrap_or_else(|p| p.into_inner()).get(&server) {
            let _ = tx.send((id.to_string(), serde_json::to_value(entry).unwrap_or_default()));
        }
    }

    /// Apply `change` to one boss's entry and tell live followers.
    fn update(&self, server: u32, id: &str, change: impl FnOnce(&mut Entry)) {
        let entry = {
            let mut servers = self.servers.lock().unwrap_or_else(|p| p.into_inner());
            let st = servers.entry(server).or_default();
            st.updated_at = now();
            let entry = st.bosses.entry(id.to_string()).or_default();
            change(entry);
            entry.clone()
        };
        self.dirty.store(true, std::sync::atomic::Ordering::SeqCst);
        self.publish(server, id, &entry);
    }

    fn feed(&self, server: u32) -> Option<Value> {
        let servers = self.servers.lock().unwrap_or_else(|p| p.into_inner());
        let st = servers.get(&server)?;
        Some(json!({ "format": 1, "server": server, "updatedAt": st.updated_at, "bosses": st.bosses }))
    }

    fn entries(&self, server: u32, only: &HashSet<String>) -> serde_json::Map<String, Value> {
        let servers = self.servers.lock().unwrap_or_else(|p| p.into_inner());
        servers
            .get(&server)
            .map(|st| {
                st.bosses
                    .iter()
                    .filter(|(id, _)| only.contains(*id))
                    .map(|(id, e)| (id.clone(), serde_json::to_value(e).unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn subscribe(&self, server: u32) -> broadcast::Receiver<(String, Value)> {
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        live.entry(server).or_insert_with(|| broadcast::channel(256).0).subscribe()
    }
}

pub async fn persist_loop(state: AppState) {
    let mut every = tokio::time::interval(std::time::Duration::from_secs(20));
    loop {
        every.tick().await;
        let t = state.timers.clone();
        let _ = tokio::task::spawn_blocking(move || t.save()).await;
    }
}

fn reporter_of(state: &AppState, headers: &HeaderMap, peer: std::net::SocketAddr) -> ApiResult<String> {
    let ip = crate::util::client_ip(headers, peer, state.config.trust_proxy);
    if !state.limiter.allow(&format!("timers:{ip}"), 120, std::time::Duration::from_secs(60)) {
        return Err(ApiError::too_many());
    }
    Ok(state.timers.reporter(ip))
}

/// An observation agrees with the entry when its state and time are the same
/// (a minute's leeway: map times are rounded by the game).
fn same_time(a: Option<i64>, b: Option<i64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= 60,
        (None, None) => true,
        _ => false,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapReport {
    server: i64,
    #[serde(default)]
    bosses: Vec<MapBoss>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MapBoss {
    id: Value,
    state: String,
    up_since: Option<i64>,
    next_spawn: Option<i64>,
}

fn id_string(v: &Value) -> Option<String> {
    let s = match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    (!s.is_empty() && s.len() <= 16 && s.bytes().all(|b| b.is_ascii_digit())).then_some(s)
}

pub async fn report(State(state): State<AppState>, headers: HeaderMap, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, Json(body): Json<MapReport>) -> ApiResult<Json<Value>> {
    let who = reporter_of(&state, &headers, peer)?;
    let server = valid_server(body.server).ok_or_else(|| ApiError::bad_request("not a server id"))?;
    let t = now();
    let mut taken = 0;
    for b in body.bosses.iter().take(64) {
        let Some(id) = id_string(&b.id) else { continue };
        if b.state != "up" && b.state != "dead" {
            continue;
        }
        let (up_since, next_spawn) = if b.state == "up" { (b.up_since, None) } else { (None, b.next_spawn) };
        let who = who.clone();
        state.timers.update(server, &id, |e| {
            let agrees = e.state == b.state && same_time(e.up_since, up_since) && same_time(e.next_spawn, next_spawn);
            if !agrees {
                *e = Entry { state: b.state.clone(), up_since, next_spawn, reported_at: t, ..Default::default() };
                if b.state == "up" {
                    // HP from fights carries over a map that says the same boss is still up.
                    e.hp = None;
                }
            }
            e.reporters.insert(who);
            e.reported_at = t;
            e.confirmed = e.reporters.len() >= 2;
        });
        taken += 1;
    }
    Ok(Json(json!({ "ok": true, "taken": taken })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kill {
    server: i64,
    id: Value,
    died_at: i64,
}

pub async fn kill(State(state): State<AppState>, headers: HeaderMap, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, Json(body): Json<Kill>) -> ApiResult<Json<Value>> {
    let who = reporter_of(&state, &headers, peer)?;
    let server = valid_server(body.server).ok_or_else(|| ApiError::bad_request("not a server id"))?;
    let id = id_string(&body.id).filter(|id| known_boss(id)).ok_or_else(|| ApiError::bad_request("not a field boss"))?;
    let t = now();
    let died = body.died_at.clamp(t - 3600, t + 60);
    state.timers.update(server, &id, |e| {
        let agrees = e.state == "dead" && same_time(e.died_at, Some(died));
        if !agrees {
            *e = Entry { state: "dead".into(), died_at: Some(died), hp: Some(0.0), hp_at: Some(died), ..Default::default() };
        }
        e.reporters.insert(who);
        e.reported_at = t;
        e.confirmed = e.reporters.len() >= 2;
    });
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct HpReport {
    server: i64,
    #[serde(default)]
    bosses: Vec<HpBoss>,
}

#[derive(Deserialize)]
struct HpBoss {
    id: Value,
    hp: f64,
    #[serde(default)]
    reset: bool,
}

pub async fn hp(State(state): State<AppState>, headers: HeaderMap, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, Json(body): Json<HpReport>) -> ApiResult<Json<Value>> {
    let _who = reporter_of(&state, &headers, peer)?;
    let server = valid_server(body.server).ok_or_else(|| ApiError::bad_request("not a server id"))?;
    let t = now();
    for b in body.bosses.iter().take(16) {
        let Some(id) = id_string(&b.id).filter(|id| known_boss(id)) else { continue };
        if !(0.0..=100.0).contains(&b.hp) {
            continue;
        }
        state.timers.update(server, &id, |e| {
            if e.state != "up" {
                *e = Entry { state: "up".into(), up_since: Some(t), ..Default::default() };
            }
            e.hp = Some(b.hp);
            e.hp_at = Some(t);
            e.reset = b.reset;
            e.reported_at = t;
        });
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct CoresReport {
    server: i64,
    #[serde(default)]
    cores: Vec<Value>,
}

pub async fn cores(State(state): State<AppState>, headers: HeaderMap, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, Json(body): Json<CoresReport>) -> ApiResult<Json<Value>> {
    let _who = reporter_of(&state, &headers, peer)?;
    let server = valid_server(body.server).ok_or_else(|| ApiError::bad_request("not a server id"))?;
    let t = now();
    {
        let mut servers = state.timers.servers.lock().unwrap_or_else(|p| p.into_inner());
        let st = servers.entry(server).or_default();
        for core in body.cores.iter().take(32) {
            let Some(npc) = core.get("npc").and_then(id_string) else { continue };
            let entry = st.cores.entry(npc).or_insert_with(|| json!({}));
            if let (Some(obj), Some(new)) = (entry.as_object_mut(), core.as_object()) {
                for (k, v) in new {
                    obj.insert(k.clone(), v.clone());
                }
                obj.insert("reportedAt".into(), json!(t));
            }
        }
        st.updated_at = t;
    }
    state.timers.dirty.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(Json(json!({ "ok": true })))
}

pub async fn artifacts(State(state): State<AppState>, headers: HeaderMap, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, Json(mut body): Json<Value>) -> ApiResult<Json<Value>> {
    let _who = reporter_of(&state, &headers, peer)?;
    let server = body.get("server").and_then(|s| s.as_i64()).and_then(valid_server).ok_or_else(|| ApiError::bad_request("not a server id"))?;
    if body.to_string().len() > 64 * 1024 {
        return Err(ApiError::bad_request("too large"));
    }
    if let Some(obj) = body.as_object_mut() {
        obj.insert("reportedAt".into(), json!(now()));
        obj.remove("host");
    }
    {
        let mut servers = state.timers.servers.lock().unwrap_or_else(|p| p.into_inner());
        let st = servers.entry(server).or_default();
        st.artifacts = Some(body);
        st.updated_at = now();
    }
    state.timers.dirty.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(Json(json!({ "ok": true })))
}

/// GET /feeds/boss-timers/:file, `<server>.json`: what `a2.bossTimers` reads.
pub async fn feed(State(state): State<AppState>, Path(file): Path<String>) -> Response {
    let Some(server) = file.strip_suffix(".json").and_then(|s| s.parse::<i64>().ok()).and_then(valid_server) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match state.timers.feed(server) {
        Some(v) => ([("cache-control", "public, max-age=2")], Json(v)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The site's page: every server's bosses, cores and artifacts, with names.
pub async fn overview(State(state): State<AppState>) -> Json<Value> {
    let servers = state.timers.servers.lock().unwrap_or_else(|p| p.into_inner());
    let list: Vec<Value> = servers
        .iter()
        .map(|(server, st)| json!({ "server": server, "updatedAt": st.updated_at, "bosses": st.bosses, "cores": st.cores, "artifacts": st.artifacts }))
        .collect();
    Json(json!({ "servers": list }))
}

/// The field bosses the site knows: world-map id, NPC code and name.
pub async fn boss_list(State(state): State<AppState>) -> Json<Value> {
    let list: Vec<Value> = FIELD_BOSSES
        .iter()
        .map(|(npc, id)| json!({ "id": id.to_string(), "npc": npc, "name": state.npc_names.get(npc).cloned().unwrap_or_else(|| format!("Boss {id}")) }))
        .collect();
    Json(json!({ "bosses": list }))
}

#[derive(Deserialize)]
pub struct LiveQuery {
    server: i64,
}

/// GET /api/boss-timers/live?server=: the plugins' live updates. The client
/// sends `{"op":"follow","bosses":[...]}` (again whenever its set changes)
/// and "ping"; it gets `{"type":"state","bosses":{...}}` on each follow and
/// `{"type":"boss","boss":id,"entry":{...}}` on each change.
pub async fn live(State(state): State<AppState>, Query(q): Query<LiveQuery>, ws: WebSocketUpgrade) -> Response {
    let server = valid_server(q.server);
    ws.on_upgrade(move |socket| async move {
        match server {
            Some(server) => run_live(state, server, socket).await,
            None => {
                let mut socket = socket;
                let _ = socket
                    .send(Message::Close(Some(axum::extract::ws::CloseFrame { code: 4004, reason: "unknown server".into() })))
                    .await;
            }
        }
    })
}

async fn run_live(state: AppState, server: u32, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let mut updates = state.timers.subscribe(server);
    let mut follow: HashSet<String> = HashSet::new();
    let idle = std::time::Duration::from_secs(120);
    loop {
        tokio::select! {
            msg = tokio::time::timeout(idle, rx.next()) => {
                let Ok(Some(Ok(msg))) = msg else { break };
                match msg {
                    Message::Text(text) => {
                        if text.as_str() == "ping" {
                            if tx.send(Message::Text("pong".into())).await.is_err() { break }
                            continue;
                        }
                        let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                        if v["op"] == "follow" {
                            follow = v["bosses"].as_array().into_iter().flatten().filter_map(id_string).take(50).collect();
                            let bosses = state.timers.entries(server, &follow);
                            let out = json!({ "type": "state", "bosses": bosses });
                            if tx.send(Message::Text(out.to_string().into())).await.is_err() { break }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            update = updates.recv() => match update {
                Ok((id, entry)) => {
                    if follow.contains(&id) {
                        let out = json!({ "type": "boss", "boss": id, "entry": entry });
                        if tx.send(Message::Text(out.to_string().into())).await.is_err() { break }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let out = json!({ "type": "state", "bosses": state.timers.entries(server, &follow) });
                    if tx.send(Message::Text(out.to_string().into())).await.is_err() { break }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_short_digit_strings() {
        assert_eq!(id_string(&json!(101006)), Some("101006".into()));
        assert_eq!(id_string(&json!("101006")), Some("101006".into()));
        assert_eq!(id_string(&json!("10a")), None);
        assert_eq!(id_string(&json!("")), None);
        assert_eq!(id_string(&json!(null)), None);
    }

    #[test]
    fn map_times_agree_within_a_minute() {
        assert!(same_time(Some(1000), Some(1059)));
        assert!(!same_time(Some(1000), Some(1061)));
        assert!(same_time(None, None));
        assert!(!same_time(Some(1), None));
    }

    #[test]
    fn two_reporters_confirm_a_boss() {
        let dir = std::env::temp_dir().join(format!("a2t-timers-{}", std::process::id()));
        let t = Timers::load(dir.join("none.json"));
        let mut rx = t.subscribe(2101);
        let report = |who: &str| {
            t.update(2101, "101006", |e| {
                if e.state != "dead" {
                    *e = Entry { state: "dead".into(), next_spawn: Some(5000), ..Default::default() };
                }
                e.reporters.insert(who.into());
                e.confirmed = e.reporters.len() >= 2;
            })
        };
        report("a");
        assert_eq!(t.feed(2101).unwrap()["bosses"]["101006"]["confirmed"], false);
        report("a");
        assert_eq!(t.feed(2101).unwrap()["bosses"]["101006"]["confirmed"], false);
        report("b");
        assert_eq!(t.feed(2101).unwrap()["bosses"]["101006"]["confirmed"], true);
        // Followers heard each change, and never who reported it.
        let (id, entry) = rx.try_recv().unwrap();
        assert_eq!(id, "101006");
        assert!(entry.get("reporters").is_none());
        assert!(known_boss("101006") && !known_boss("999999"));
    }
}
