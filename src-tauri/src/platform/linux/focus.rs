//! Which window is active, on KDE. Wayland tells no application about another
//! application's windows, but KWin runs scripts and a script can call D-Bus:
//! the one loaded here tells the meter each time the active window changes,
//! for "Hide when AION2 isn't focused". Other desktops are not covered.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const SERVICE: &str = "app.a2tools.DpsMeter";
const PATH: &str = "/Focus";
/// The script's name in KWin, so a script left by an earlier run is replaced.
const PLUGIN: &str = "a2tools-dps-meter-focus";

/// Values are passed as strings: KWin hands JavaScript numbers to D-Bus as
/// doubles, which an integer argument would refuse. KWin 6 has
/// `windowActivated`, KWin 5 `clientActivated`.
const SCRIPT: &str = r#"// A2Tools DPS Meter: tells the meter which window is active, so it can hide
// while the game is not in front. Loaded by the meter at each start.
function report(w) {
    callDBus("app.a2tools.DpsMeter", "/Focus", "app.a2tools.DpsMeter.Focus", "Active",
             w ? String(w.pid) : "0", w ? String(w.resourceClass) : "", w ? String(w.caption) : "");
}
if (workspace.windowActivated) {
    workspace.windowActivated.connect(report);
    report(workspace.activeWindow);
} else {
    workspace.clientActivated.connect(report);
    report(workspace.activeClient);
}
"#;

/// 0: not known (no report yet, or not KDE). 1: the game or one of the
/// meter's own windows is active. 2: something else is.
static ACTIVE: AtomicU8 = AtomicU8::new(0);

/// The game's window has been reported active at least once. Until then the
/// reports are not trusted: the setting is on by default, and a game window
/// this does not recognise must not leave the meter hidden for good.
static SEEN_GAME: AtomicBool = AtomicBool::new(false);

/// The connection that serves `Focus`; dropped, the service would go away.
static CONNECTION: OnceLock<zbus::blocking::Connection> = OnceLock::new();

/// Told when the game or the meter gains or loses the front.
static ON_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Whether the game (or the meter) is the active window, if KWin has said
/// and the game has been recognised.
pub fn game_or_meter_active() -> Option<bool> {
    if !SEEN_GAME.load(Ordering::Relaxed) {
        return None;
    }
    match ACTIVE.load(Ordering::Relaxed) {
        0 => None,
        1 => Some(true),
        _ => Some(false),
    }
}

struct Focus;

#[zbus::interface(name = "app.a2tools.DpsMeter.Focus")]
impl Focus {
    fn active(&self, pid: String, class: String, caption: String) {
        let who = classify(pid.parse().unwrap_or(0), &class, &caption);
        if who == Who::Game && !SEEN_GAME.swap(true, Ordering::Relaxed) {
            tracing::info!("KWin: the game's window is active (class {class:?})");
        }
        let was = ACTIVE.swap(if who == Who::Other { 2 } else { 1 }, Ordering::Relaxed);
        if was != ACTIVE.load(Ordering::Relaxed) && SEEN_GAME.load(Ordering::Relaxed) {
            if let Some(on_change) = ON_CHANGE.get() {
                on_change();
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Who {
    Game,
    Meter,
    Other,
}

/// The game by title or class, as on Windows, or by its process; the meter by
/// its own process id, so opening Settings does not hide the overlay.
fn classify(pid: u32, class: &str, caption: &str) -> Who {
    if caption.starts_with("AION2") || class.eq_ignore_ascii_case("aion2.exe") {
        return Who::Game;
    }
    if pid == 0 {
        return Who::Other;
    }
    if pid == std::process::id() {
        return Who::Meter;
    }
    let dir = std::path::PathBuf::from(format!("/proc/{pid}"));
    let comm = std::fs::read_to_string(dir.join("comm")).unwrap_or_default();
    let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
    if crate::platform::procfs::is_aion2_process(&comm, &cmdline) { Who::Game } else { Who::Other }
}

/// Serve `Focus` on the session bus and load the script into KWin. Does
/// nothing outside KDE: there the game is taken as always focused.
pub fn watch(on_change: impl Fn() + Send + Sync + 'static) -> Result<(), String> {
    if !super::process::is_kde() || CONNECTION.get().is_some() {
        return Ok(());
    }
    let connection = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(SERVICE))
        .and_then(|b| b.serve_at(PATH, Focus))
        .and_then(|b| b.build())
        .map_err(|e| format!("D-Bus service: {e}"))?;

    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = dir.join("a2tools-dps-meter-focus.js");
    std::fs::write(&path, SCRIPT).map_err(|e| format!("writing the script: {e}"))?;

    let kwin = zbus::blocking::Proxy::new(&connection, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting")
        .map_err(|e| format!("KWin scripting: {e}"))?;
    let _ = kwin.call_method("unloadScript", &(PLUGIN,));
    let id: i32 = kwin
        .call("loadScript", &(path.to_string_lossy().as_ref(), PLUGIN))
        .map_err(|e| format!("loading the script: {e}"))?;
    if id < 0 {
        return Err("KWin did not load the script".into());
    }
    kwin.call_method("start", &()).map_err(|e| format!("starting the script: {e}"))?;
    let _ = ON_CHANGE.set(Box::new(on_change));
    let _ = CONNECTION.set(connection);
    tracing::info!("KWin reports the active window (script {id})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_is_known_by_title_or_class_and_the_meter_by_itself() {
        assert_eq!(classify(0, "", "AION2"), Who::Game);
        assert_eq!(classify(0, "aion2.exe", ""), Who::Game);
        assert_eq!(classify(std::process::id(), "a2tools-dps-meter", "A2Tools DPS Meter"), Who::Meter);
        assert_eq!(classify(0, "firefox", "Mozilla Firefox"), Who::Other);
    }
}
