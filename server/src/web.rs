//! The site's pages: one small app (web/), compiled into the binary, served
//! for every page path, and the meter's own class icons and logo.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::util::escape_html;
use crate::AppState;

const INDEX: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const APP_CSS: &str = include_str!("../web/app.css");
const SERVERS: &str = include_str!("../../src/data/servers.json");
const DUNGEONS: &str = include_str!("../../src/data/i18n/dungeons/en.json");
const LOGO: &[u8] = include_bytes!("../../src/assets/logo.png");

/// Class icons by job id (`JobClass::class_prefix`).
const CLASS_ICONS: &[(u32, &[u8])] = &[
    (11, include_bytes!("../../src/assets/검성.png")),
    (12, include_bytes!("../../src/assets/수호성.png")),
    (13, include_bytes!("../../src/assets/살성.png")),
    (14, include_bytes!("../../src/assets/궁성.png")),
    (15, include_bytes!("../../src/assets/마도성.png")),
    (16, include_bytes!("../../src/assets/정령성.png")),
    (17, include_bytes!("../../src/assets/치유성.png")),
    (18, include_bytes!("../../src/assets/호법성.png")),
    (19, include_bytes!("../../src/assets/권성.png")),
];

/// Every path the app draws itself.
const PAGES: &[&str] = &[
    "/", "/logs", "/logs/{id}", "/leaderboards", "/classes", "/timers", "/link", "/login", "/register", "/account", "/admin",
    "/download",
];

async fn page(State(state): State<AppState>) -> Response {
    let html = INDEX.replace("{{SITE_NAME}}", &escape_html(&state.config.site_name));
    ([(header::CACHE_CONTROL, "no-cache")], Html(html)).into_response()
}

fn asset(content_type: &'static str, body: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "public, max-age=3600")], body).into_response()
}

async fn class_icon(Path(file): Path<String>) -> Response {
    let job = file.strip_suffix(".png").and_then(|j| j.parse::<u32>().ok());
    match job.and_then(|j| CLASS_ICONS.iter().find(|(id, _)| *id == j)) {
        Some((_, bytes)) => asset("image/png", bytes),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub fn routes() -> Router<AppState> {
    let mut router = Router::new()
        .route("/static/app.js", get(|| async { asset("text/javascript; charset=utf-8", APP_JS.as_bytes()) }))
        .route("/static/app.css", get(|| async { asset("text/css; charset=utf-8", APP_CSS.as_bytes()) }))
        .route("/static/servers.json", get(|| async { asset("application/json", SERVERS.as_bytes()) }))
        .route("/static/dungeons.json", get(|| async { asset("application/json", DUNGEONS.as_bytes()) }))
        .route("/static/logo.png", get(|| async { asset("image/png", LOGO) }))
        .route("/favicon.ico", get(|| async { asset("image/png", LOGO) }))
        .route("/static/class/{file}", get(class_icon))
        .route("/healthz", get(|| async { "ok" }));
    for path in PAGES {
        router = router.route(path, get(page));
    }
    router
}
