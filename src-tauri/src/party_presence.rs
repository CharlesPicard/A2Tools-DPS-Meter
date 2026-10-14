//! Which party members also run A2Tools: a small mark beside their names.
//!
//! On unless turned off (`dpsMeter.partyPresence`, asked once with a popup,
//! ticked by default). While the player is in a party, the meter registers
//! its own character with a2tools.app and asks about its party members: the
//! ones not asked about yet, as the party fills (a dungeon's matchmaking, an
//! invite), and every 10 minutes the ones that weren't using it, in case they
//! started since. Solo, it sends nothing.
//!
//! Only hashes go: sha256 of `a2tools-presence|<name, lower-cased>|<server>`.
//! That hides nobody from someone who knows a name and a server, so the site
//! answers only a meter that registers itself (each sees only those who can
//! see it), one party's worth of names at a time, paced. Registrations last
//! 15 minutes on the site; refreshing every 10 keeps a player in a party seen.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde_json::json;
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::app::AppState;

pub const SETTING_KEY: &str = "dpsMeter.partyPresence";
const CHECK: Duration = Duration::from_secs(3);
/// Own registration refreshed, and the "not using it" answers asked again, this often.
const REFRESH_MS: i64 = 10 * 60_000;
/// Requests at most this often (the site paces at 5 s).
const MIN_GAP_MS: i64 = 6_000;
/// Names one request asks about at most (the site's limit).
const MAX_CHECK: usize = 24;

/// A character's hash, as the site keeps it.
pub fn hash(name: &str, server_id: u16) -> String {
    let mut h = Sha256::new();
    h.update(format!("a2tools-presence|{}|{server_id}", name.trim().to_lowercase()).as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The names to ask about now: those never asked about, and, when the
/// refresh is due, those that weren't using A2Tools last time.
pub fn to_ask<'a>(party: &'a [(String, u16)], asked: &HashSet<String>, users: &HashSet<String>, refresh: bool) -> Vec<&'a (String, u16)> {
    party
        .iter()
        .filter(|(name, _)| !asked.contains(name) || (refresh && !users.contains(name)))
        .take(MAX_CHECK)
        .collect()
}

pub fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Who was asked about, and who uses A2Tools, by name (this session).
        let mut asked: HashSet<String> = HashSet::new();
        let mut users: HashSet<String> = HashSet::new();
        let mut last_sent: i64 = 0;
        let mut last_refresh: i64 = 0;
        loop {
            tokio::time::sleep(CHECK).await;
            let Some(state) = app.try_state::<AppState>() else { continue };
            if state.settings.get(SETTING_KEY).as_deref() == Some("false") {
                if !users.is_empty() {
                    users.clear();
                    state.data_storage.set_a2tools_users(HashSet::new());
                }
                continue;
            }
            let profile = state.data_storage.local_profile();
            let (Some(me), server) = (profile.name.clone(), profile.server_id) else { continue };
            if server == 0 {
                continue;
            }
            let me_key = me.trim().to_lowercase();
            let party: Vec<(String, u16)> = state
                .data_storage
                .get_party_members()
                .into_iter()
                .filter(|(name, m)| m.server_id != 0 && name.trim().to_lowercase() != me_key)
                .map(|(name, m)| (name.trim().to_string(), m.server_id))
                .collect();
            // The marks follow the party: someone who left loses theirs.
            let in_party: HashSet<String> = party.iter().map(|(n, _)| n.clone()).collect();
            state.data_storage.set_a2tools_users(users.intersection(&in_party).cloned().collect());
            if party.is_empty() {
                continue;
            }
            let now = crate::clock::now_ms();
            let refresh = now - last_refresh >= REFRESH_MS;
            let ask = to_ask(&party, &asked, &users, refresh);
            if (ask.is_empty() && !refresh) || now - last_sent < MIN_GAP_MS {
                continue;
            }
            last_sent = now;
            let by_hash: HashMap<String, String> = ask.iter().map(|(n, s)| (hash(n, *s), n.clone())).collect();
            let body = json!({ "format": 1, "me": hash(&me, server), "check": by_hash.keys().collect::<Vec<_>>() });
            let url = format!("{}/api/presence", crate::account::base_url());
            let request = state.http.post(&url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string())
                .timeout(Duration::from_secs(15));
            let Ok(res) = request.send().await else { continue };
            if !res.status().is_success() {
                continue; // paced or down: asked again on a later pass
            }
            let Ok(answer) = serde_json::from_str::<serde_json::Value>(&res.text().await.unwrap_or_default()) else { continue };
            if refresh {
                last_refresh = now;
            }
            let yes: HashSet<&str> = answer["users"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
            for (h, name) in &by_hash {
                asked.insert(name.clone());
                if yes.contains(h.as_str()) {
                    users.insert(name.clone());
                } else {
                    users.remove(name);
                }
            }
            state.data_storage.set_a2tools_users(users.intersection(&in_party).cloned().collect());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_ignores_case_and_spaces_but_not_the_server() {
        assert_eq!(hash("Naicha", 1301), hash(" naicha ", 1301));
        assert_ne!(hash("Naicha", 1301), hash("Naicha", 2301));
        assert_eq!(hash("Naicha", 1301).len(), 64);
    }

    #[test]
    fn new_members_are_asked_about_once_and_non_users_again_at_the_refresh() {
        let party = vec![("A".to_string(), 1301), ("B".to_string(), 1301), ("C".to_string(), 1302)];
        let asked: HashSet<String> = ["A", "B"].iter().map(|s| s.to_string()).collect();
        let users: HashSet<String> = ["A"].iter().map(|s| s.to_string()).collect();
        let names = |v: Vec<&(String, u16)>| v.into_iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        assert_eq!(names(to_ask(&party, &asked, &users, false)), vec!["C"]);
        assert_eq!(names(to_ask(&party, &asked, &users, true)), vec!["B", "C"]);
    }
}
