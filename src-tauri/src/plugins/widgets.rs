//! Plugin widgets (#77): small windows a plugin opens beside its own, such as
//! a countdown icon, which the player can drag anywhere and lock in place.
//!
//! A widget shows the plugin's `widget.html`, sandboxed like its main page,
//! inside `plugin-widget.html`. The plugin's main window opens, closes and
//! feeds its widgets (`a2.widgets`, permission `widgets`); a widget can only
//! ask to be dragged and post a message back to its plugin (`a2.widget`).
//! Each widget remembers where it was left; locked, a plugin's widgets let
//! clicks through to the game and cannot be moved until unlocked. They close
//! with the plugin's main window.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;
use tauri::{Emitter, Manager};

use super::{host, require, valid_id, window_plugin};
use crate::app::AppState;

/// Most widgets one plugin may have open.
const MAX_WIDGETS: usize = 16;
/// Most bytes of data a widget message may carry, either way.
const MAX_MESSAGE: usize = 64 * 1024;

/// A widget's window label: `pw_<plugin>_<name>`. Neither a plugin id nor a
/// widget name has a `_`, so the label reads back unambiguously, and it never
/// starts with `plugin-`, so no main-window command accepts it.
fn label(id: &str, name: &str) -> String {
    format!("pw_{id}_{name}")
}

/// The plugin and widget a window label names, if it is a widget's.
pub fn parse_label(label: &str) -> Option<(String, String)> {
    let (id, name) = label.strip_prefix("pw_")?.split_once('_')?;
    (valid_id(id) && valid_name(name)).then(|| (id.to_string(), name.to_string()))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn locked_key(id: &str) -> String {
    format!("dpsMeter.pluginWidgetsLocked.{id}")
}

fn place_key(id: &str, name: &str) -> String {
    format!("dpsMeter.pluginWidget.{id}.{name}")
}

fn is_locked(app: &tauri::AppHandle, id: &str) -> bool {
    app.try_state::<AppState>().and_then(|s| s.settings.get(&locked_key(id))).as_deref() == Some("true")
}

/// The calling window's widget: its plugin and name.
fn widget_of(window: &tauri::WebviewWindow) -> Result<(String, String), String> {
    parse_label(window.label()).ok_or_else(|| "not a widget window".to_string())
}

/// The widgets of plugin `id` now open, by label.
fn open_widgets(app: &tauri::AppHandle, id: &str) -> Vec<tauri::WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| parse_label(label).is_some_and(|(owner, _)| owner == id))
        .map(|(_, window)| window)
        .collect()
}

/// The part of each widget that takes the mouse, as its page reported it
/// (`a2.widget.area`): CSS px and the page's scale. Unlocked, clicks
/// elsewhere in its window go through (Linux, by an input region), so icons
/// set close together don't catch each other's drags.
type Area = (f64, f64, f64, f64, f64);
fn areas() -> &'static Mutex<HashMap<String, Area>> {
    static AREAS: std::sync::OnceLock<Mutex<HashMap<String, Area>>> = std::sync::OnceLock::new();
    AREAS.get_or_init(Mutex::default)
}

/// Locked: clicks go through to the game. Unlocked: only its area (when it
/// reported one) takes them. On Linux by an input region, which needs the
/// window on screen (tao's own call would abort without one; see
/// `platform::window::hide_in_place`), so a widget is locked again once its
/// page has loaded (`plugin_widget_hello`) and each time it is shown again.
fn apply_lock(window: &tauri::WebviewWindow, locked: bool) {
    let area = areas().lock().unwrap_or_else(|p| p.into_inner()).get(window.label()).copied();
    let applied = match (locked, area) {
        (true, _) => crate::platform::window::click_through_except(window, true, None),
        (false, Some(area)) => crate::platform::window::click_through_except(window, true, Some(area)),
        (false, None) => crate::platform::window::click_through_except(window, false, None),
    };
    if !applied && !cfg!(target_os = "linux") {
        let _ = window.set_ignore_cursor_events(locked);
    }
}

/// The window in front when each widget was made, until its page loads.
fn fronts() -> &'static Mutex<HashMap<String, Option<isize>>> {
    static FRONTS: std::sync::OnceLock<Mutex<HashMap<String, Option<isize>>>> = std::sync::OnceLock::new();
    FRONTS.get_or_init(Mutex::default)
}

fn saved_place(app: &tauri::AppHandle, id: &str, name: &str) -> Option<(i32, i32)> {
    let value = app.try_state::<AppState>()?.settings.get(&place_key(id, name))?;
    let (x, y) = value.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Open widget `name` of the calling plugin, `width` x `height` logical pixels,
/// or show it again if it is open.
#[tauri::command]
pub async fn plugin_widget_open(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    name: String,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let id = window_plugin(&window)?;
    require(&app, &id, "widgets")?;
    if !valid_name(&name) {
        return Err("a widget name is 1 to 32 lowercase letters, digits and '-'".into());
    }
    let size = |v: f64| if v.is_finite() { v.clamp(24.0, 800.0) } else { 64.0 };
    let (width, height) = (size(width), size(height));
    if let Some(widget) = app.get_webview_window(&label(&id, &name)) {
        let _ = widget.set_size(tauri::LogicalSize::new(width, height));
        if super::game_hides(&app, &id) {
            super::hold_for_game(&widget);
        } else if !crate::platform::window::reveal_in_place(&widget) {
            crate::platform::window::show_on_top_without_focus(&widget);
        }
        apply_lock(&widget, is_locked(&app, &id));
        return Ok(());
    }
    if open_widgets(&app, &id).len() >= MAX_WIDGETS {
        return Err(format!("at most {MAX_WIDGETS} widgets"));
    }
    let plugin_name = host(&app)?.with(&id, |l| l.manifest.name.clone()).ok_or("no such plugin")?;
    let script = format!(
        "window.__A2_PLUGIN_ID__ = {}; window.__A2_WIDGET__ = {};",
        serde_json::to_string(&id).unwrap_or_default(),
        serde_json::to_string(&name).unwrap_or_default(),
    );
    let title = format!("{plugin_name} · {name}");
    crate::app::note_window_title(&title, &label(&id, &name));
    let front = crate::platform::window::front_window();
    let widget = tauri::WebviewWindowBuilder::new(&app, label(&id, &name), tauri::WebviewUrl::App("plugin-widget.html".into()))
        .initialization_script(script)
        // Its own title, so KWin can keep it where it was left (#76).
        .title(title)
        .inner_size(width, height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        // Never takes focus, from the start: clicks and drags still reach it.
        // Focusable, tao turns a GTK window's focus back on after its first
        // draw, and KWin gave a new icon the front at its alert (Hirai,
        // CachyOS KDE, 2.0.71); on Windows it is WS_EX_NOACTIVATE from
        // creation, before WebView2 can take the front.
        .focusable(false)
        .build()
        .map_err(|e| e.to_string())?;
    // Never in front of the game: an alert icon appearing must not tab the
    // player out of it.
    crate::platform::window::never_activate(&widget, front);
    fronts().lock().unwrap_or_else(|p| p.into_inner()).insert(widget.label().to_string(), front);
    // Where it was left; a widget never moved goes beside the plugin's others
    // rather than wherever the window manager puts a new window (the middle
    // of the screen).
    let place = saved_place(&app, &id, &name).or_else(|| {
        let others: Vec<_> = open_widgets(&app, &id).into_iter().filter(|w| w.label() != widget.label()).collect();
        beside(others.iter().filter_map(|w| {
            let at = w.outer_position().ok()?;
            let size = w.outer_size().ok()?;
            Some((at.x, at.y, size.width as i32, size.height as i32))
        }))
    }).or_else(|| beside_left(&app, &id));
    if let Some((x, y)) = place {
        let _ = widget.set_position(tauri::PhysicalPosition::new(x, y));
    }
    // AION2 closed and the plugin set to hide then: shown when it opens.
    if super::game_hides(&app, &id) {
        super::hold_for_game(&widget);
    }
    Ok(())
}

/// With none of the plugin's widgets open, a new one goes beside where its
/// widgets were last left, not mid-screen (Hirai, 2026-10-19: an alert's
/// icon in the middle of the game).
fn beside_left(app: &tauri::AppHandle, id: &str) -> Option<(i32, i32)> {
    let state = app.try_state::<AppState>()?;
    let prefix = format!("dpsMeter.pluginWidget.{id}.");
    let places: Vec<(i32, i32)> = state.settings.get_all().into_iter().filter_map(|(key, value)| {
        key.strip_prefix(&prefix)?;
        let (x, y) = value.split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    }).collect();
    let screens: Vec<(i32, i32, u32, u32)> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let a = m.work_area();
            (a.position.x, a.position.y, a.size.width, a.size.height)
        })
        .collect();
    beside_cluster(&places, &screens)
}

/// The width of an icon left somewhere is not kept; an Event Timers icon is
/// about this wide.
const ICON_WIDTH: i32 = 96;

/// Beside the busiest spot among `places`: the place with the most others
/// near it, so one icon left far off (a test icon on another screen) does
/// not draw the next one there (2.0.72: the anchor opened on the second
/// monitor). On that place's screen: to its right, or its left when the
/// right runs off the screen.
fn beside_cluster(places: &[(i32, i32)], screens: &[(i32, i32, u32, u32)]) -> Option<(i32, i32)> {
    const NEAR: i64 = 400;
    let near = |a: &(i32, i32), b: &(i32, i32)| {
        let (dx, dy) = ((a.0 - b.0) as i64, (a.1 - b.1) as i64);
        dx * dx + dy * dy <= NEAR * NEAR
    };
    let &(x, y) = places.iter().max_by_key(|p| (places.iter().filter(|q| near(p, q)).count(), std::cmp::Reverse(p.0)))?;
    let screen = screens.iter().find(|&&(sx, sy, w, h)| {
        x >= sx && x < sx + w as i32 && y >= sy && y < sy + h as i32
    });
    let right = x + ICON_WIDTH + 6;
    match screen {
        Some(&(sx, _, w, _)) if right + ICON_WIDTH > sx + w as i32 => Some(((x - ICON_WIDTH - 6).max(sx), y)),
        _ => Some((right, y)),
    }
}

/// Just right of the rightmost of these places, on its row, with a gap.
pub fn beside(places: impl Iterator<Item = (i32, i32, i32, i32)>) -> Option<(i32, i32)> {
    places.max_by_key(|&(x, _, w, _)| x + w).map(|(x, y, w, _)| (x + w + 6, y))
}

/// The widget a window label names, beside its plugin's other widgets, for
/// the window manager to place it (Linux on Wayland, see app.rs): from where
/// the others were left.
pub fn place_beside_others(app: &tauri::AppHandle, label: &str, place_of: impl Fn(&str) -> Option<(i32, i32, i32, i32)>) -> Option<(i32, i32)> {
    let (id, _) = parse_label(label)?;
    beside(open_widgets(app, &id).iter().filter(|w| w.label() != label).filter_map(|w| place_of(w.label())))
        .or_else(|| beside_left(app, &id))
}

/// The part of the calling widget that takes the mouse while unlocked (CSS
/// px from its top left, and the page's scale), from `a2.widget.area`.
#[tauri::command]
pub fn plugin_widget_area(app: tauri::AppHandle, window: tauri::WebviewWindow, x: f64, y: f64, width: f64, height: f64, scale: f64) -> Result<(), String> {
    let (id, _) = widget_of(&window)?;
    let ok = |v: f64| v.is_finite() && (0.0..=4000.0).contains(&v);
    if !(ok(x) && ok(y) && ok(width) && ok(height) && scale.is_finite() && scale > 0.0 && scale <= 8.0) {
        return Err("not an area".into());
    }
    areas().lock().unwrap_or_else(|p| p.into_inner()).insert(window.label().to_string(), (x, y, width, height, scale));
    apply_lock(&window, is_locked(&app, &id));
    Ok(())
}

/// Where widget `name` of the calling plugin is: `{ x, y, width, height }`
/// in physical screen pixels, and the work area of the screen it is on
/// (`screen`, the same), or null when it is not open.
#[tauri::command]
pub fn plugin_widget_where(app: tauri::AppHandle, window: tauri::WebviewWindow, name: String) -> Result<Value, String> {
    let id = window_plugin(&window)?;
    require(&app, &id, "widgets")?;
    let Some(widget) = app.get_webview_window(&label(&id, &name)) else { return Ok(Value::Null) };
    let at = widget.outer_position().map_err(|e| e.to_string())?;
    let size = widget.outer_size().map_err(|e| e.to_string())?;
    let screen = widget.current_monitor().ok().flatten().map(|m| {
        let area = m.work_area();
        serde_json::json!({ "x": area.position.x, "y": area.position.y,
                            "width": area.size.width, "height": area.size.height })
    });
    Ok(serde_json::json!({ "x": at.x, "y": at.y, "width": size.width, "height": size.height, "screen": screen }))
}

/// Move widget `name` of the calling plugin to `x, y` (physical screen
/// pixels, its top left), as a plugin that lays its widgets out does.
/// Locked or not: the lock is the player's hand off them, not the plugin's.
/// A widget not open yet opens there: the place is kept for it, where the
/// window manager reads it too (KDE on Wayland places a new window itself,
/// and lets no app move one after: an icon placed only once open stayed
/// mid-screen there, Hirai 2026-10-19).
#[tauri::command]
pub fn plugin_widget_place(app: tauri::AppHandle, window: tauri::WebviewWindow, name: String, x: i32, y: i32) -> Result<(), String> {
    let id = window_plugin(&window)?;
    require(&app, &id, "widgets")?;
    if !valid_name(&name) {
        return Err("a widget name is 1 to 32 lowercase letters, digits and '-'".into());
    }
    let far = 1_000_000;
    if !(-far..far).contains(&x) || !(-far..far).contains(&y) {
        return Err("not a place on a screen".into());
    }
    let widget_label = label(&id, &name);
    if let Some(state) = app.try_state::<AppState>() {
        state.settings.set(&place_key(&id, &name), &format!("{x},{y}"));
        state.settings.set(&format!("window.{widget_label}.placed"), &format!("{x},{y},0,0"));
    }
    match app.get_webview_window(&widget_label) {
        Some(widget) => widget.set_position(tauri::PhysicalPosition::new(x, y)).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// Close widget `name` of the calling plugin.
#[tauri::command]
pub fn plugin_widget_close(app: tauri::AppHandle, window: tauri::WebviewWindow, name: String) -> Result<(), String> {
    let id = window_plugin(&window)?;
    require(&app, &id, "widgets")?;
    if let Some(widget) = app.get_webview_window(&label(&id, &name)) {
        areas().lock().unwrap_or_else(|p| p.into_inner()).remove(widget.label());
        let _ = widget.close();
    }
    Ok(())
}

/// Hand widget `name` data from its plugin: it hears it as a "widget" event.
#[tauri::command]
pub fn plugin_widget_send(app: tauri::AppHandle, window: tauri::WebviewWindow, name: String, data: Value) -> Result<(), String> {
    let id = window_plugin(&window)?;
    require(&app, &id, "widgets")?;
    if data.to_string().len() > MAX_MESSAGE {
        return Err("message too large".into());
    }
    let event = serde_json::json!({ "plugin": id, "event": "widget", "data": data });
    app.emit_to(label(&id, &name), "plugin-event", event).map_err(|e| e.to_string())
}

/// Lock (clicks go through, no moving) or unlock all the calling plugin's
/// widgets. Kept across restarts.
#[tauri::command]
pub fn plugin_widgets_lock(app: tauri::AppHandle, window: tauri::WebviewWindow, locked: bool) -> Result<(), String> {
    let id = window_plugin(&window)?;
    require(&app, &id, "widgets")?;
    if let Some(state) = app.try_state::<AppState>() {
        state.settings.set(&locked_key(&id), if locked { "true" } else { "false" });
    }
    for widget in open_widgets(&app, &id) {
        apply_lock(&widget, locked);
        let event = serde_json::json!({ "plugin": id, "event": "locked", "data": locked });
        let _ = widget.emit("plugin-event", event);
    }
    Ok(())
}

/// Whether the calling plugin's widgets are locked.
#[tauri::command]
pub fn plugin_widgets_locked(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<bool, String> {
    let id = window_plugin(&window)?;
    require(&app, &id, "widgets")?;
    Ok(is_locked(&app, &id))
}

/// What a widget's host page needs to start. Its window is on screen by now,
/// so this is where a locked widget is made click-through.
#[tauri::command]
pub fn plugin_widget_hello(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<Value, String> {
    let (id, name) = widget_of(&window)?;
    // Its page has loaded: if that took the front from the game, give it back.
    let before = fronts().lock().unwrap_or_else(|p| p.into_inner()).remove(window.label()).flatten();
    crate::platform::window::give_front_back(&window, before);
    let locked = is_locked(&app, &id);
    apply_lock(&window, locked);
    let language = app.try_state::<AppState>().and_then(|s| s.settings.get("dpsMeter.language")).unwrap_or_else(|| "en".into());
    Ok(serde_json::json!({ "plugin": id, "name": name, "locked": locked, "language": language }))
}

/// A widget's message to its plugin's main window, heard there as a
/// "widget-message" event: `{ name, data }`.
#[tauri::command]
pub fn plugin_widget_post(app: tauri::AppHandle, window: tauri::WebviewWindow, data: Value) -> Result<(), String> {
    let (id, name) = widget_of(&window)?;
    if data.to_string().len() > MAX_MESSAGE {
        return Err("message too large".into());
    }
    let event = serde_json::json!({ "plugin": id, "event": "widget-message", "data": { "name": name, "data": data } });
    app.emit_to(format!("plugin-{id}"), "plugin-event", event).map_err(|e| e.to_string())
}

/// Move the widget with the pointer, unless its plugin's widgets are locked.
#[tauri::command]
pub fn plugin_widget_drag(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    let (id, _) = widget_of(&window)?;
    if !is_locked(&app, &id) {
        crate::platform::window::start_drag(&window);
    }
    Ok(())
}

/// Whether a widget is on screen and unlocked (so a double-click on it can
/// open its plugin's window): a way back to the plugins with the meter hidden.
pub fn any_unlocked_widget_shown(app: &tauri::AppHandle) -> bool {
    app.webview_windows().into_iter().any(|(label, w)| {
        parse_label(&label).is_some_and(|(id, _)| !is_locked(app, &id))
            && w.is_visible().unwrap_or(false)
            && !crate::platform::window::is_hidden_in_place(&w)
    })
}

/// Close every widget of plugin `id`: its main window has gone.
pub fn close_all(app: &tauri::AppHandle, id: &str) {
    for widget in open_widgets(app, id) {
        areas().lock().unwrap_or_else(|p| p.into_inner()).remove(widget.label());
        let _ = widget.close();
    }
}

/// A widget moved: remember where, once it has stopped moving, and tell its
/// plugin ("widget-moved", `{ name, x, y }`), which may lay its widgets out
/// around it.
pub fn note_moved(app: &tauri::AppHandle, label: &str, at: tauri::PhysicalPosition<i32>) {
    static LATEST: std::sync::OnceLock<Mutex<HashMap<String, (i32, i32)>>> = std::sync::OnceLock::new();
    let Some((id, name)) = parse_label(label) else { return };
    let key = place_key(&id, &name);
    let latest = LATEST.get_or_init(Mutex::default);
    let first = latest.lock().unwrap_or_else(|p| p.into_inner()).insert(key.clone(), (at.x, at.y)).is_none();
    if !first {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        let Some((x, y)) = latest.lock().unwrap_or_else(|p| p.into_inner()).remove(&key) else { return };
        if let Some(state) = app.try_state::<AppState>() {
            state.settings.set(&key, &format!("{x},{y}"));
        }
        let event = serde_json::json!({ "plugin": id, "event": "widget-moved", "data": { "name": name, "x": x, "y": y } });
        let _ = app.emit_to(format!("plugin-{id}"), "plugin-event", event);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_widget_label_reads_back_and_is_never_a_plugin_window() {
        let l = label("event-timers", "shugo-2");
        assert_eq!(parse_label(&l), Some(("event-timers".into(), "shugo-2".into())));
        assert!(!l.starts_with("plugin-"));
        assert_eq!(parse_label("plugin-event-timers"), None);
        assert_eq!(parse_label("pw_Bad_name"), None);
        assert!(!valid_name("has_underscore") && !valid_name("") && valid_name("boss-101006"));
    }

    #[test]
    fn a_new_widget_goes_beside_the_busiest_spot_on_its_screen() {
        // This PC on 2.0.72: icons around (2000, 250) on the main screen, a
        // test icon at (2680, 122) just over on the second one.
        let screens = [(2560, 0, 1920, 1080), (0, 0, 2560, 1440)];
        let places = [(2090, 250), (1956, 187), (1994, 225), (2232, 250), (2088, 370), (1971, 291),
            (2112, 122), (2001, 315), (2075, 228), (2680, 122), (17, 394), (43, 491)];
        let (x, y) = beside_cluster(&places, &screens).unwrap();
        assert!(x < 2560 && (1900..2400).contains(&x) && (100..400).contains(&y), "{x},{y}");
        // At a screen's right edge it goes to the left instead.
        assert_eq!(beside_cluster(&[(2500, 100)], &screens), Some((2398, 100)));
        assert_eq!(beside_cluster(&[], &screens), None);
    }

    #[test]
    fn a_new_widget_goes_right_of_the_rightmost_one() {
        assert_eq!(beside([].into_iter()), None);
        let places = [(100, 50, 80, 90), (300, 400, 80, 90), (200, 60, 80, 90)];
        assert_eq!(beside(places.into_iter()), Some((386, 400)));
    }
}
