//! Field boss timers, shared: what the player's map shows is sent to
//! a2tools.app, which checks it, combines it with other players' and
//! publishes each server's timers for everyone's Event Timers.
//!
//! On by default (`dpsMeter.shareBossTimers`, "false" turns it off). What is
//! sent: the server, the map, and each field boss on it: up (since when) or
//! dead (back when). Nothing about the player; the site counts reporters by a
//! hash of their address that changes daily, and keeps no address.
//!
//! A map is sent when it says something new for that server: a boss up or
//! down, a new appearance or respawn time, a boss added or gone. The same map
//! goes again at most every RESEND_SAME_MS, which keeps the site's
//! confirmations (two reporters agreeing) coming without a request for every
//! time the map is opened (a2tools.app's Workers CPU, 2026-10-16). A meter started
//! mid-game doesn't know the server until the game says it: the newest map
//! waits until then, and goes if it is still under MAX_AGE_MS old. Only the
//! player's own server's maps: an enemy map viewed from the menu carries
//! nothing, and a map read from inside a rift (its header not 0) isn't sent
//! until it is known whose server it shows.
//!
//! Field boss kills in the open world go too, as they happen, and a field
//! boss's HP when it changes (checked every 3 seconds), and once a minute
//! while it is fought and holds; a jump back up is marked as a reset
//! (`spawn_events`). a2tools.app
//! passes a drop of a percent or more, and a reset, on to Aion 2 Timers.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::json;
use tauri::Manager;

use crate::app::AppState;
use crate::capture::boss_map::BossMap;
use crate::combat::data_storage::{BossEvent, BossEventKind, BossHp};

pub const SETTING_KEY: &str = "dpsMeter.shareBossTimers";
const CHECK: Duration = Duration::from_secs(2);
/// The same map, unchanged, is sent again after this long at the earliest.
const RESEND_SAME_MS: i64 = 5 * 60_000;
/// A map read longer ago than this is not sent (waiting for the server).
const MAX_AGE_MS: i64 = 10 * 60_000;

/// The report the site's /api/boss-timers/report reads (format 1).
pub fn report(server_id: u16, map: &BossMap, now_ms: i64) -> serde_json::Value {
    let secs = |ms: i64| (ms > 0).then_some(ms / 1000);
    let bosses: Vec<_> = map
        .bosses
        .iter()
        .map(|b| {
            if b.up {
                json!({ "id": b.id, "state": "up", "upSince": secs(b.time_ms) })
            } else {
                json!({ "id": b.id, "state": "dead", "nextSpawn": secs(b.time_ms) })
            }
        })
        .collect();
    json!({
        "format": 1,
        "server": server_id,
        "map": map.map_id,
        "observedAt": now_ms / 1000,
        "bosses": bosses,
    })
}

/// Korea and Taiwan number their servers the same (10xx Elyos, 20xx
/// Asmodian), so a report from one of them names the game server's address
/// too, which a2tools.app tells the two regions apart by. Global ids say
/// their region already and go without it.
pub fn with_host(mut body: serde_json::Value, server_id: u16, host: Option<std::net::Ipv4Addr>) -> serde_json::Value {
    let shared_id = (1000..1100).contains(&server_id) || (2000..2100).contains(&server_id);
    if let (true, Some(host), Some(obj)) = (shared_id, host, body.as_object_mut()) {
        obj.insert("host".into(), json!(host.to_string()));
    }
    body
}

/// A kill or HP milestone not sent yet waits this long for the server to be
/// known (the meter started mid-game).
const EVENT_WAIT_MS: i64 = 5 * 60_000;

/// HP is sent this often, for the bosses whose percent changed.
const HP_EVERY: Duration = Duration::from_secs(3);

/// The request for a kill: (path, body), or None for a boss a2tools.app
/// doesn't know.
pub fn event_request(server_id: u16, event: &BossEvent) -> Option<(&'static str, serde_json::Value)> {
    let id = crate::capture::field_bosses::world_map_id(event.npc)?;
    Some(match event.kind {
        BossEventKind::Killed { at_ms } => (
            "/api/boss-timers/kill",
            json!({ "format": 1, "server": server_id, "id": id, "diedAt": at_ms / 1000 }),
        ),
    })
}

/// The HP request for the bosses to report now.
pub fn hp_request(server_id: u16, bosses: &[BossHp]) -> Option<serde_json::Value> {
    let bosses: Vec<_> = bosses
        .iter()
        .filter_map(|b| {
            let id = crate::capture::field_bosses::world_map_id(b.npc)?;
            Some(if b.reset { json!({ "id": id, "hp": b.percent, "reset": true }) } else { json!({ "id": id, "hp": b.percent }) })
        })
        .collect();
    (!bosses.is_empty()).then(|| json!({ "format": 1, "server": server_id, "bosses": bosses }))
}

/// Kills and HP milestones, sent within a second of happening.
fn spawn_events(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut waiting: Vec<(i64, BossEvent)> = Vec::new();
        let mut last_hp = std::time::Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(state) = app.try_state::<AppState>() else { continue };
            let now = crate::clock::now_ms();
            // HP every 3 seconds, the bosses whose percent changed; dropped
            // when it can't go (HP is only worth anything now).
            if last_hp.elapsed() >= HP_EVERY {
                last_hp = std::time::Instant::now();
                let changed = state.data_storage.take_boss_hp(now);
                let server = state.data_storage.local_profile().server_id;
                let off = state.settings.get(SETTING_KEY).as_deref() == Some("false");
                if let (false, false, Some(body)) = (off, server == 0, hp_request(server, &changed)) {
                    let body = with_host(body, server, state.data_storage.game_host());
                    let url = format!("{}/api/boss-timers/hp", crate::account::base_url());
                    let request = state.http.post(&url)
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body.to_string())
                        .timeout(Duration::from_secs(10));
                    if let Err(e) = request.send().await {
                        tracing::debug!("Boss timers: could not send HP: {e}");
                    }
                }
            }
            waiting.extend(state.data_storage.take_boss_events().into_iter().map(|e| (now, e)));
            if waiting.is_empty() {
                continue;
            }
            if state.settings.get(SETTING_KEY).as_deref() == Some("false") {
                waiting.clear();
                continue;
            }
            let server = state.data_storage.local_profile().server_id;
            if server == 0 {
                waiting.retain(|(at, _)| now - at < EVENT_WAIT_MS);
                continue;
            }
            for (_, event) in std::mem::take(&mut waiting) {
                let Some((path, body)) = event_request(server, &event) else { continue };
                let body = with_host(body, server, state.data_storage.game_host());
                let url = format!("{}{path}", crate::account::base_url());
                let request = state.http.post(&url)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body.to_string())
                    .timeout(Duration::from_secs(15));
                match request.send().await {
                    Ok(res) => tracing::info!("Boss timers: {:?} of {} sent ({})", event.kind, event.npc, res.status()),
                    Err(e) => tracing::info!("Boss timers: could not send {:?} of {}: {e}", event.kind, event.npc),
                }
            }
        }
    });
}

pub fn spawn(app: tauri::AppHandle) {
    spawn_events(app.clone());
    tauri::async_runtime::spawn(async move {
        let mut seq = 0u64;
        // (server, map) -> when sent, and what it said.
        let mut sent: HashMap<(u16, u32), (i64, String)> = HashMap::new();
        loop {
            tokio::time::sleep(CHECK).await;
            let Some(state) = app.try_state::<AppState>() else { continue };
            let Some((next, maps)) = state.data_storage.boss_maps_since(seq) else { continue };
            if state.settings.get(SETTING_KEY).as_deref() == Some("false") {
                seq = next;
                continue;
            }
            // The server isn't known yet (the meter started mid-game): keep
            // the maps for when it is.
            let server = state.data_storage.local_profile().server_id;
            if server == 0 {
                continue;
            }
            seq = next;
            let now = crate::clock::now_ms();
            for map in maps.iter().filter(|m| m.header == 0 && now - m.seen_ms <= MAX_AGE_MS) {
                let said = serde_json::to_string(&report(server, map, 0)["bosses"]).unwrap_or_default();
                if let Some((at, before)) = sent.get(&(server, map.map_id)) {
                    if *before == said && now - at < RESEND_SAME_MS {
                        continue;
                    }
                }
                sent.insert((server, map.map_id), (now, said));
                let url = format!("{}/api/boss-timers/report", crate::account::base_url());
                let body = with_host(report(server, map, now), server, state.data_storage.game_host());
                let request = state.http.post(&url)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body.to_string())
                    .timeout(Duration::from_secs(15));
                match request.send().await {
                    Ok(res) => {
                        let status = res.status();
                        let text = res.text().await.unwrap_or_default();
                        tracing::info!("Boss timers: map {} for server {} sent ({status}): {}", map.map_id, server,
                            text.chars().take(300).collect::<String>());
                    }
                    Err(e) => tracing::info!("Boss timers: could not send map {}: {e}", map.map_id),
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::boss_map::BossOnMap;

    #[test]
    fn a_kill_and_the_hp_name_the_boss_by_its_world_map_id() {
        let kill = BossEvent { npc: 2100079, kind: BossEventKind::Killed { at_ms: 1_791_993_638_495 } };
        let (path, body) = event_request(1301, &kill).unwrap();
        assert_eq!(path, "/api/boss-timers/kill");
        assert_eq!(body, json!({ "format": 1, "server": 1301, "id": 101006, "diedAt": 1_791_993_638 }));
        assert!(event_request(1301, &BossEvent { npc: 1, kind: BossEventKind::Killed { at_ms: 0 } }).is_none());
        let b = |npc, percent, reset| BossHp { npc, percent, reset };
        let hp = hp_request(1301, &[b(2100079, 49, false), b(1, 50, false)]).unwrap();
        assert_eq!(hp, json!({ "format": 1, "server": 1301, "bosses": [{ "id": 101006, "hp": 49 }] }));
        let reset = hp_request(1301, &[b(2100079, 100, true)]).unwrap();
        assert_eq!(reset["bosses"][0], json!({ "id": 101006, "hp": 100, "reset": true }));
        assert!(hp_request(1301, &[b(1, 50, false)]).is_none());
    }

    /// Korea and Taiwan share 10xx/20xx: their reports name the game
    /// server's address (Taiwan's 1016 was 210.242.123.144). Global ids,
    /// or no address known yet, go as before.
    #[test]
    fn a_shared_korea_taiwan_id_names_the_game_server() {
        let host = Some(std::net::Ipv4Addr::new(210, 242, 123, 144));
        let body = || json!({ "format": 1, "server": 1016 });
        assert_eq!(with_host(body(), 1016, host)["host"], "210.242.123.144");
        assert_eq!(with_host(body(), 2018, host)["host"], "210.242.123.144");
        assert!(with_host(body(), 1016, None).get("host").is_none());
        assert!(with_host(body(), 1301, host).get("host").is_none());
        assert!(with_host(body(), 1101, host).get("host").is_none());
    }

    #[test]
    fn a_report_names_the_server_map_and_each_bosss_state_and_time() {
        let map = BossMap {
            map_id: 1010,
            header: 0,
            seen_ms: 1_791_993_638_495,
            bosses: vec![
                BossOnMap { id: 101_006, up: true, time_ms: 1_791_993_400_000 },
                BossOnMap { id: 101_002, up: false, time_ms: 1_791_994_085_000 },
                BossOnMap { id: 2003, up: false, time_ms: 0 },
            ],
        };
        let r = report(1301, &map, 1_791_993_638_495);
        assert_eq!(r["format"], 1);
        assert_eq!((r["server"].as_u64(), r["map"].as_u64(), r["observedAt"].as_i64()), (Some(1301), Some(1010), Some(1_791_993_638)));
        assert_eq!(r["bosses"][0], json!({ "id": 101_006, "state": "up", "upSince": 1_791_993_400 }));
        assert_eq!(r["bosses"][1], json!({ "id": 101_002, "state": "dead", "nextSpawn": 1_791_994_085 }));
        assert_eq!(r["bosses"][2]["nextSpawn"], serde_json::Value::Null);
    }
}
