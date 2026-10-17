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
        if !crate::platform::window::reveal_in_place(&widget) {
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
    let widget = tauri::WebviewWindowBuilder::new(&app, label(&id, &name), tauri::WebviewUrl::App("plugin-widget.html".into()))
        .initialization_script(script)
        // Its own title, so KWin can keep it where it was left (#76).
        .title(format!("{plugin_name} · {name}"))
        .inner_size(width, height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .build()
        .map_err(|e| e.to_string())?;
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
    });
    if let Some((x, y)) = place {
        let _ = widget.set_position(tauri::PhysicalPosition::new(x, y));
    }
    Ok(())
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

/// A widget moved: remember where, once it has stopped moving.
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
    fn a_new_widget_goes_right_of_the_rightmost_one() {
        assert_eq!(beside([].into_iter()), None);
        let places = [(100, 50, 80, 90), (300, 400, 80, 90), (200, 60, 80, 90)];
        assert_eq!(beside(places.into_iter()), Some((386, 400)));
    }
}
