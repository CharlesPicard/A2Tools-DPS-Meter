//! Which window is active, on KDE. Wayland tells no application about another
//! application's windows, but KWin runs scripts and a script can call D-Bus:
//! the one loaded here tells the meter each time the active window changes,
//! for "Hide when AION2 isn't focused". Other desktops are not covered.
//!
//! On Wayland it also keeps the meter's windows where they were left (#76):
//! no application may place its own windows there, but a KWin script may.
//! The script reports where each of the meter's windows was moved or resized
//! to, and puts each back there when it appears again.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const SERVICE: &str = "app.a2tools.DpsMeter";
const PATH: &str = "/Focus";
/// The script's name in KWin, so a script left by an earlier run is replaced.
const PLUGIN: &str = "a2tools-dps-meter-focus";

/// Values are passed as strings: KWin hands JavaScript numbers to D-Bus as
/// doubles, which an integer argument would refuse. KWin 6 has
/// `windowActivated` and `windowAdded`, KWin 5 `clientActivated` and
/// `clientAdded`. `__PID__` is the meter's process id: only its windows are
/// placed.
const SCRIPT: &str = r#"// A2Tools DPS Meter: tells the meter which window is active, so it can hide
// while the game is not in front, and keeps the meter's own windows where
// they were left. Loaded by the meter at each start.
var SERVICE = "app.a2tools.DpsMeter", PATH = "/Focus", IFACE = "app.a2tools.DpsMeter.Focus";
var MY_PID = __PID__;
function report(w) {
    callDBus(SERVICE, PATH, IFACE, "Active",
             w ? String(w.pid) : "0", w ? String(w.resourceClass) : "", w ? String(w.caption) : "");
}
function mine(w) {
    return !!w && Number(w.pid) === MY_PID;
}
// A plugin's widget (an alert's icon), titled "<plugin> · <name>".
function widget(w) {
    return mine(w) && String(w.caption).indexOf(" · ") >= 0;
}
// The last window not ours to be active, and our widgets that just appeared:
// Wayland activates a new window, so each icon an alert opened took the
// focus from the game (a tester, mid-PvP, 2026-10-17). An icon activated
// because it appeared hands the focus straight back; one clicked keeps it.
var lastOther = null;
var fresh = [];
function activate(w) {
    if (workspace.windowActivated) workspace.activeWindow = w;
    else workspace.activeClient = w;
}
function onActivated(w) {
    if (w && !mine(w)) lastOther = w;
    var i = fresh.indexOf(w);
    if (i >= 0) {
        fresh.splice(i, 1);
        if (lastOther) {
            activate(lastOther);
            return;
        }
    }
    report(w);
}
function remember(w) {
    var g = w.frameGeometry;
    callDBus(SERVICE, PATH, IFACE, "Geometry", String(w.caption), String(Math.round(g.x)), String(Math.round(g.y)),
             String(Math.round(g.width)), String(Math.round(g.height)));
}
function onScreen(r) {
    var area = workspace.virtualScreenGeometry;
    if (!area) return true;
    // Enough of its top edge on screen to be seen and dragged back.
    return r.x + 48 <= area.x + area.width && r.x + r.width >= area.x + 48
        && r.y >= area.y - 8 && r.y + 32 <= area.y + area.height;
}
function place(w) {
    callDBus(SERVICE, PATH, IFACE, "Placement", String(w.caption), function (answer) {
        var p = String(answer).split(",").map(Number);
        if (p.length !== 4 || p.some(isNaN)) return;
        var g = w.frameGeometry;
        // A width or height of 0: the window keeps its own size.
        var r = { x: p[0], y: p[1], width: p[2] > 0 ? p[2] : g.width, height: p[3] > 0 ? p[3] : g.height };
        if (onScreen(r)) w.frameGeometry = r;
    });
}
function follow(w) {
    if (!mine(w)) return;
    if (widget(w)) {
        fresh.push(w);
        if (fresh.length > 32) fresh.shift();
    }
    var done = w.interactiveMoveResizeFinished || w.clientFinishUserMovedResized;
    if (done) done.connect(function () { remember(w); });
    place(w);
}
if (workspace.windowActivated) {
    workspace.windowActivated.connect(onActivated);
    report(workspace.activeWindow);
    workspace.windowAdded.connect(follow);
    workspace.windowList().forEach(follow);
} else {
    workspace.clientActivated.connect(onActivated);
    report(workspace.activeClient);
    workspace.clientAdded.connect(follow);
    workspace.clientList().forEach(follow);
}
"#;

/// A window's place and size, as KWin gives them: x, y, width, height.
pub type Rect = (i32, i32, i32, i32);

/// Where the meter keeps the places KWin reports, by window title.
pub struct Places {
    pub save: Box<dyn Fn(&str, Rect) + Send + Sync>,
    pub load: Box<dyn Fn(&str) -> Option<Rect> + Send + Sync>,
}

static PLACES: OnceLock<Places> = OnceLock::new();

/// Keep the meter's windows where they were left, through KWin (#76). Only
/// on Wayland: under X11 the meter places its own windows, as on Windows.
pub fn keep_places(places: Places) {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var("GDK_BACKEND").map_or(true, |b| !b.starts_with("x11"));
    if wayland {
        let _ = PLACES.set(places);
    }
}

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

    /// One of the meter's windows was moved or resized by the player.
    fn geometry(&self, caption: String, x: String, y: String, width: String, height: String) {
        let Some(places) = PLACES.get() else { return };
        let n = |v: &str| v.trim().parse::<i32>().ok();
        if let (Some(x), Some(y), Some(w), Some(h)) = (n(&x), n(&y), n(&width), n(&height))
            && w > 0
            && h > 0
        {
            (places.save)(&caption, (x, y, w, h));
        }
    }

    /// Where one of the meter's windows belongs, `"x,y,width,height"`, or ""
    /// to leave it where KWin put it.
    fn placement(&self, caption: String) -> String {
        PLACES
            .get()
            .and_then(|places| (places.load)(&caption))
            .map(|(x, y, w, h)| format!("{x},{y},{w},{h}"))
            .unwrap_or_default()
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
    let script = SCRIPT.replace("__PID__", &std::process::id().to_string());
    std::fs::write(&path, script).map_err(|e| format!("writing the script: {e}"))?;

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
