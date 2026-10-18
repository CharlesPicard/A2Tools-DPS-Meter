//! Asking the desktop to keep the meter above a fullscreen game. On KDE
//! Plasma that is a KWin window rule (see `platform::kwin_rules`), written with
//! KDE's own `kwriteconfig6`/`kwriteconfig5` and loaded with KWin's
//! `reconfigure`. Other desktops are left alone.

use std::process::Command;

use crate::platform::kwin_rules::{
    general_entries_for, icons_rule_entries, is_kde, no_focus_rule_entries, rule_entries, GROUP, ICONS_GROUP, NO_FOCUS_GROUP,
};

fn tool(names: &[&'static str]) -> Option<&'static str> {
    names.iter().copied().find(|name| {
        Command::new(name).arg("--help").output().is_ok_and(|o| o.status.success() || !o.stdout.is_empty())
    })
}

fn read(reader: &str, group: &str, key: &str) -> String {
    Command::new(reader)
        .args(["--file", "kwinrulesrc", "--group", group, "--key", key])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn write(writer: &str, group: &str, key: &str, value: &str) -> Result<(), String> {
    let status = Command::new(writer)
        .args(["--file", "kwinrulesrc", "--group", group, "--key", key, "--", value])
        .status()
        .map_err(|e| format!("{writer}: {e}"))?;
    status.success().then_some(()).ok_or_else(|| format!("{writer} {group}/{key} failed"))
}

/// Ask KWin to load its rules again. Several ways, as distributions ship
/// different D-Bus tools.
fn reconfigure() {
    let attempts: [(&str, &[&str]); 3] = [
        ("dbus-send", &["--session", "--type=method_call", "--dest=org.kde.KWin", "/KWin", "org.kde.KWin.reconfigure"]),
        ("qdbus6", &["org.kde.KWin", "/KWin", "reconfigure"]),
        ("qdbus", &["org.kde.KWin", "/KWin", "reconfigure"]),
    ];
    for (program, args) in attempts {
        if Command::new(program).args(args).status().is_ok_and(|s| s.success()) {
            return;
        }
    }
    tracing::info!("KWin rule written; KWin applies it after the next login");
}

/// Make sure the desktop keeps the meter above a fullscreen game. Called at
/// every start with `done` false, so a missing rule is put back (it is the only
/// thing that keeps the meter on top there); nothing is written when it is
/// there. Returns whether the job is done now (added, or already there).
pub fn keep_above_fullscreen(done: bool) -> Result<bool, String> {
    add_rule(GROUP, &rule_entries(), done, "the meter stays above fullscreen games")
}

/// Make sure a click on the meter overlay leaves the game focused (#71), as
/// `keep_above_fullscreen` does for its rule.
pub fn click_without_focus(done: bool) -> Result<bool, String> {
    add_rule(NO_FOCUS_GROUP, &no_focus_rule_entries(), done, "clicking the overlay keeps the game focused")
}

/// Make sure plugins' icons never take the focus from the game.
pub fn icons_without_focus(done: bool) -> Result<bool, String> {
    add_rule(ICONS_GROUP, &icons_rule_entries(), done, "plugin icons keep the game focused")
}

fn add_rule(group: &str, entries: &[(&str, &str)], done: bool, what: &str) -> Result<bool, String> {
    if done || !is_kde(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()) {
        return Ok(done);
    }
    let (Some(writer), Some(reader)) =
        (tool(&["kwriteconfig6", "kwriteconfig5"]), tool(&["kreadconfig6", "kreadconfig5"]))
    else {
        return Err("kwriteconfig is not installed".into());
    };
    if !read(reader, group, "wmclass").is_empty() {
        return Ok(true);
    }
    for (key, value) in entries {
        write(writer, group, key, value)?;
    }
    let order = read(reader, "General", "Order");
    let legacy = read(reader, "General", "rules");
    for (key, value) in general_entries_for(group, &order, &legacy) {
        write(writer, "General", key, &value)?;
    }
    reconfigure();
    tracing::info!("KWin rule added: {what}");
    Ok(true)
}
