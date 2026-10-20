//! A self-hosted A2 Tools site for the DPS meter.
//!
//! What a2tools.app does for the meter, on your own server: accounts and the
//! meter's sign-in, fight log uploads re-derived from their Evidence Slice by
//! the meter's own parser, log pages, leaderboards and class stats, field boss
//! timers (with the feed and live updates plugins read), party presence marks,
//! and "Send logs to dev". One binary, one SQLite file, a data directory.
//!
//! Point a meter at it under Settings → A2 Tools Account → Server (or the
//! `A2TOOLS_API` environment variable).

mod auth;
mod config;
mod db;
mod derive;
mod device;
mod logs;
mod misc;
mod store;
mod timers;
mod util;
mod web;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub config: config::Config,
    pub db: db::Db,
    pub store: store::Store,
    pub limiter: util::RateLimiter,
    pub timers: Arc<timers::Timers>,
    pub npc_names: HashMap<i32, String>,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl Inner {
    pub fn log_url(&self, id: &str) -> String {
        format!("{}/logs/{id}", self.config.public_url)
    }
}

fn router(state: AppState) -> Router {
    let api = Router::new()
        // The meter's sign-in (RFC 8628) and account.
        .route("/api/device/code", post(device::code))
        .route("/api/device/token", post(device::token))
        .route("/api/device/lookup", get(device::lookup))
        .route("/api/device/decide", post(device::decide))
        .route("/api/me", get(auth::me))
        // The site's own sign-in.
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/session", get(auth::session))
        .route("/api/account", patch(auth::update_account))
        .route("/api/account/tokens", get(auth::list_tokens))
        .route("/api/account/tokens/{id}", delete(auth::revoke_token))
        // Logs.
        .route("/api/logs", post(logs::upload).layer(DefaultBodyLimit::max(24 * 1024 * 1024)).get(logs::list))
        .route("/api/logs/{id}", get(logs::get).patch(logs::update).delete(logs::delete))
        .route("/api/logs/{id}/slice", get(logs::slice))
        .route("/api/bosses", get(logs::bosses))
        .route("/api/leaderboards", get(logs::leaderboard))
        .route("/api/stats/classes", get(logs::class_stats))
        .route("/api/overview", get(logs::overview))
        // Field boss timers.
        .route("/api/boss-timers/report", post(timers::report))
        .route("/api/boss-timers/kill", post(timers::kill))
        .route("/api/boss-timers/hp", post(timers::hp))
        .route("/api/boss-timers/cores", post(timers::cores))
        .route("/api/boss-timers/artifacts", post(timers::artifacts))
        .route("/api/boss-timers/live", get(timers::live))
        .route("/api/boss-timers", get(timers::overview))
        .route("/api/boss-timers/bosses", get(timers::boss_list))
        .route("/feeds/boss-timers/{file}", get(timers::feed))
        // Presence and dev logs.
        .route("/api/presence", post(misc::presence))
        .route("/api/dev-logs", post(misc::dev_register))
        .route("/api/dev-logs/{code}/{name}", put(misc::dev_put).layer(DefaultBodyLimit::max(41 * 1024 * 1024)))
        // Admin.
        .route("/api/admin/users", get(auth::admin_users))
        .route("/api/admin/users/{id}", patch(auth::admin_update_user))
        .route("/api/admin/dev-logs", get(misc::dev_list))
        .route("/api/admin/dev-logs/{code}", delete(misc::dev_delete))
        .route("/api/admin/dev-logs/{code}/{name}", get(misc::dev_download))
        .route("/api/admin/rederive", post(logs::rederive_all));
    api.merge(web::routes()).with_state(state)
}

/// Ctrl-C, or SIGTERM from `docker stop` and systemd.
async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = config::Config::from_env();
    std::fs::create_dir_all(&config.data_dir).expect("could not create the data directory");
    let db = db::Db::open(&config.data_dir.join("a2tools.db")).expect("could not open the database");
    let store = store::Store::new(&config.data_dir).expect("could not create the data directories");
    let timers = Arc::new(timers::Timers::load(config.data_dir.join("boss-timers.json")));
    let state = AppState(Arc::new(Inner {
        config: config.clone(),
        db,
        store,
        limiter: util::RateLimiter::new(),
        timers: timers.clone(),
        npc_names: derive::npc_names(),
    }));
    auth::bootstrap_admin(&state).await;
    tokio::spawn(timers::persist_loop(state.clone()));
    tokio::spawn(misc::prune_loop(state.clone()));

    let app = router(state).layer(tower_http::trace::TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(config.listen).await.expect("could not listen");
    tracing::info!("A2 Tools server on {} (public address {}), parser {}", config.listen, config.public_url, derive::parser_version());
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown())
        .await
        .expect("server error");
    timers.save();
}
