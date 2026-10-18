//! The KWin window rule that keeps the meter above a fullscreen game on KDE
//! Plasma, as data: what the rule says and how it joins the rules already in
//! `kwinrulesrc`. OS-neutral so it can be tested anywhere; `linux/window_rules`
//! writes it.
//!
//! KWin treats a borderless game that covers the screen as fullscreen, and a
//! fullscreen window sits above "keep above" windows, so the meter went behind
//! the game (#33, and a CachyOS player on Discord, 2026-10-08). Forcing the
//! meter's layer to Overlay puts it above. Keys and values are KWin's own
//! (src/rulesettings.kcfg): `layerrule`/`aboverule` 2 is Force, `wmclassmatch`
//! 1 is an exact match.

/// The rule's group in `kwinrulesrc`. Fixed, so the meter can tell its own
/// rule from the player's and never adds it twice.
pub const GROUP: &str = "a2tools-dps-meter-overlay";

/// The window class the meter's windows carry.
pub const WINDOW_CLASS: &str = "a2tools-dps-meter";

/// The rule, as (key, value) pairs in its group.
pub fn rule_entries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Description", "A2Tools DPS Meter above fullscreen games (added by the meter)"),
        ("wmclass", WINDOW_CLASS),
        ("wmclassmatch", "1"),
        ("wmclasscomplete", "false"),
        ("layer", "overlay"),
        ("layerrule", "2"),
        // Plasma 5 has no layer rule; keep-above is what it can do.
        ("above", "true"),
        ("aboverule", "2"),
    ]
}

/// The second rule's group: clicking the meter overlay does not take focus
/// from the game (#71). Its own group, so installs that already have the
/// first rule get it too.
pub const NO_FOCUS_GROUP: &str = "a2tools-dps-meter-no-focus";

/// The overlay's title. The meter's other windows (Settings, Details,
/// Battle History, plugins) have their own titles and keep focus as usual:
/// a window that takes no focus takes no typing either.
pub const OVERLAY_TITLE: &str = "A2Tools DPS Meter";

/// The no-focus rule: the overlay window only (class and exact title), and
/// `acceptfocus` false, forced (`acceptfocusrule` 2). Clicks still reach it.
pub fn no_focus_rule_entries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Description", "A2Tools DPS Meter overlay keeps focus on the game (added by the meter)"),
        ("wmclass", WINDOW_CLASS),
        ("wmclassmatch", "1"),
        ("wmclasscomplete", "false"),
        ("title", OVERLAY_TITLE),
        ("titlematch", "1"),
        ("acceptfocus", "false"),
        ("acceptfocusrule", "2"),
    ]
}

/// The third rule's group: plugins' icons (widgets) never take focus. Each
/// one an alert opened took it from the game until it closed, and handing it
/// back after still left the game a moment without it (KDE Plasma 6 on
/// Wayland, 2026-10-19). Clicks still reach them.
pub const ICONS_GROUP: &str = "a2tools-dps-meter-icons";

/// What plugins' icons are titled with: `"<plugin> · <name>"`
/// (plugins::widgets). No other window of the meter's has it.
pub const ICON_TITLE_MARK: &str = " · ";

/// The icons' rule: the meter's windows whose title contains
/// `ICON_TITLE_MARK` (`titlematch` 2), `acceptfocus` false, forced.
pub fn icons_rule_entries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Description", "A2Tools DPS Meter plugin icons keep focus on the game (added by the meter)"),
        ("wmclass", WINDOW_CLASS),
        ("wmclassmatch", "1"),
        ("wmclasscomplete", "false"),
        ("title", ICON_TITLE_MARK),
        ("titlematch", "2"),
        ("acceptfocus", "false"),
        ("acceptfocusrule", "2"),
    ]
}

/// The `[General]` entries that list the rules, with the meter's added.
///
/// KWin 6 orders rules by `Order` and still reads the legacy `rules`/`count`
/// when `Order` is empty. So `Order` is only written when it already exists,
/// or as the legacy list plus ours: writing ours alone would hide every rule
/// the player made before. The legacy pair is kept in step for older KWin.
pub fn general_entries(order: &str, legacy_rules: &str) -> Vec<(&'static str, String)> {
    general_entries_for(GROUP, order, legacy_rules)
}

/// `general_entries` for any of the meter's rules.
pub fn general_entries_for(group: &str, order: &str, legacy_rules: &str) -> Vec<(&'static str, String)> {
    let split = |list: &str| -> Vec<String> {
        list.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
    };
    let add = |mut list: Vec<String>| {
        if !list.iter().any(|g| g == group) {
            list.push(group.to_string());
        }
        list
    };
    let legacy = add(split(legacy_rules));
    let ordered = if split(order).is_empty() { legacy.clone() } else { add(split(order)) };
    vec![
        ("Order", ordered.join(",")),
        ("rules", legacy.join(",")),
        ("count", legacy.len().to_string()),
    ]
}

/// Whether the session is KDE Plasma (`XDG_CURRENT_DESKTOP` is `KDE`).
pub fn is_kde(current_desktop: &str) -> bool {
    current_desktop.split(':').any(|name| name.eq_ignore_ascii_case("kde"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(entries: &'a [(&str, String)], key: &str) -> &'a str {
        entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str()).unwrap()
    }

    #[test]
    fn a_first_rule_starts_both_lists() {
        let e = general_entries("", "");
        assert_eq!(get(&e, "Order"), GROUP);
        assert_eq!(get(&e, "rules"), GROUP);
        assert_eq!(get(&e, "count"), "1");
    }

    #[test]
    fn the_players_own_rules_stay_and_come_first() {
        // Legacy list only: Order must carry the player's rules too, or KWin
        // 6 would read Order and lose them.
        let e = general_entries("", "abc-1,def-2");
        assert_eq!(get(&e, "Order"), format!("abc-1,def-2,{GROUP}"));
        assert_eq!(get(&e, "count"), "3");
        // Order already kept: ours goes on its end.
        let e = general_entries("x,y", "abc-1");
        assert_eq!(get(&e, "Order"), format!("x,y,{GROUP}"));
        assert_eq!(get(&e, "rules"), format!("abc-1,{GROUP}"));
    }

    #[test]
    fn the_rule_is_never_listed_twice() {
        let listed = format!("abc-1,{GROUP}");
        let e = general_entries(&listed, &listed);
        assert_eq!(get(&e, "Order"), listed);
        assert_eq!(get(&e, "count"), "2");
    }

    #[test]
    fn the_rule_forces_the_overlay_layer_for_the_meter_only() {
        let rule = rule_entries();
        let value = |k: &str| rule.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        assert_eq!(value("layer"), Some("overlay"));
        assert_eq!(value("layerrule"), Some("2"));
        assert_eq!(value("wmclass"), Some("a2tools-dps-meter"));
        assert_eq!(value("wmclassmatch"), Some("1"));
    }

    /// #71: the overlay takes no focus, so a click on it leaves the game
    /// focused; only the overlay, by its exact title, as the other windows
    /// need focus to be typed in.
    #[test]
    fn the_no_focus_rule_is_the_overlays_alone_and_joins_the_first() {
        let rule = no_focus_rule_entries();
        let value = |k: &str| rule.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        assert_eq!(value("title"), Some("A2Tools DPS Meter"));
        assert_eq!(value("titlematch"), Some("1"));
        assert_eq!((value("acceptfocus"), value("acceptfocusrule")), (Some("false"), Some("2")));
        let listed = format!("abc-1,{GROUP}");
        let e = general_entries_for(NO_FOCUS_GROUP, &listed, &listed);
        assert_eq!(get(&e, "Order"), format!("{listed},{NO_FOCUS_GROUP}"));
        assert_eq!(get(&e, "count"), "3");
    }

    /// Icons only, by the mark in their title, and never focused.
    #[test]
    fn the_icons_rule_matches_titles_with_the_mark() {
        let rule = icons_rule_entries();
        let value = |k: &str| rule.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        assert_eq!(value("title"), Some(" · "));
        assert_eq!(value("titlematch"), Some("2"));
        assert_eq!((value("acceptfocus"), value("acceptfocusrule")), (Some("false"), Some("2")));
        assert_eq!(value("wmclass"), Some(WINDOW_CLASS));
    }

    #[test]
    fn kde_is_told_from_its_desktop_name() {
        assert!(is_kde("KDE"));
        assert!(is_kde("kde"));
        assert!(!is_kde("GNOME"));
        assert!(!is_kde("ubuntu:GNOME"));
        assert!(!is_kde(""));
    }
}
