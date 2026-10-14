//! Field boss timers, shared: what the player's map shows is sent to
//! a2tools.app, which checks it, combines it with other players' and
//! publishes each server's timers for everyone's Event Timers.
//!
//! On by default (`dpsMeter.shareBossTimers`, "false" turns it off). What is
//! sent: the server, the map, and each field boss on it: up (since when) or
//! dead (back when). Nothing about the player; the site counts reporters by a
//! hash of their address that changes daily, and keeps no address.
//!
//! A map is sent when the player opens it, at most once a minute per map,
//! and not again while it says the same as the last one sent. Only the
//! player's own server's maps: an enemy map viewed from the menu carries
//! nothing, and a map read from inside a rift (its header not 0) isn't sent
//! until it is known whose server it shows.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::json;
use tauri::Manager;

use crate::app::AppState;
use crate::capture::boss_map::BossMap;

pub const SETTING_KEY: &str = "dpsMeter.shareBossTimers";
const CHECK: Duration = Duration::from_secs(10);
const MIN_GAP_MS: i64 = 60_000;
/// The same map, unchanged, is sent again after this long (to confirm it).
const RESEND_SAME_MS: i64 = 10 * 60_000;

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

pub fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut seq = 0u64;
        // (server, map) -> when sent, and what it said.
        let mut sent: HashMap<(u16, u32), (i64, String)> = HashMap::new();
        loop {
            tokio::time::sleep(CHECK).await;
            let Some(state) = app.try_state::<AppState>() else { continue };
            let Some((next, maps)) = state.data_storage.boss_maps_since(seq) else { continue };
            seq = next;
            if state.settings.get(SETTING_KEY).as_deref() == Some("false") {
                continue;
            }
            let server = state.data_storage.local_profile().server_id;
            if server == 0 {
                continue;
            }
            let now = crate::clock::now_ms();
            for map in maps.iter().filter(|m| m.header == 0) {
                let said = serde_json::to_string(&report(server, map, 0)["bosses"]).unwrap_or_default();
                if let Some((at, before)) = sent.get(&(server, map.map_id)) {
                    if now - at < MIN_GAP_MS || (*before == said && now - at < RESEND_SAME_MS) {
                        continue;
                    }
                }
                sent.insert((server, map.map_id), (now, said));
                let url = format!("{}/api/boss-timers/report", crate::account::base_url());
                let body = report(server, map, now);
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
    fn a_report_names_the_server_map_and_each_bosss_state_and_time() {
        let map = BossMap {
            map_id: 1010,
            header: 0,
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
