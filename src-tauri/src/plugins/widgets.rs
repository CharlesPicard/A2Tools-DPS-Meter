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

/// Locked: clicks go through to the game. On Linux by an input region, which
/// needs the window on screen (tao's own call would abort without one; see
/// `platform::window::hide_in_place`), so a widget not yet shown is locked
/// again once its page has loaded (`plugin_widget_hello`).
fn apply_lock(window: &tauri::WebviewWindow, locked: bool) {
    if !crate::platform::window::click_through_except(window, locked, None) && !cfg!(target_os = "linux") {
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
    if let Some((x, y)) = saved_place(&app, &id, &name) {
        let _ = widget.set_position(tauri::PhysicalPosition::new(x, y));
    }
    Ok(())
}

/// Close widget `name` of the calling plugin.
#[tauri::command]
pub fn plugin_widget_close(app: tauri::AppHandle, window: tauri::WebviewWindow, name: String) -> Result<(), String> {
    let id = window_plugin(&window)?;
    require(&app, &id, "widgets")?;
    if let Some(widget) = app.get_webview_window(&label(&id, &name)) {
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

/// Close every widget of plugin `id`: its main window has gone.
pub fn close_all(app: &tauri::AppHandle, id: &str) {
    for widget in open_widgets(app, id) {
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
}
