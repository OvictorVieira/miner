mod auth;
mod config;
mod dockerfile_policy;
mod miner;
mod stats;

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use miner::SharedStatus;
use tokio::sync::{watch, RwLock};

const DASHBOARD_HTML: &str = include_str!("../assets/dashboard.html");
const LOGIN_HTML: &str = include_str!("../assets/login.html");

struct App {
    cfg: config::Config,
    status: SharedStatus,
    cache: stats::StatsCache,
    auth: auth::Auth,
}

type SharedApp = Arc<App>;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();

    let cfg = match config::Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("configuration error: {e}");
            std::process::exit(1);
        }
    };

    let status: SharedStatus = Arc::new(RwLock::new(miner::MinerStatus::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let supervisor = miner::spawn_supervisor(cfg.clone(), status.clone(), shutdown_rx);

    if cfg.dashboard_password.is_none() {
        tracing::info!("DASHBOARD_PASSWORD not set — dashboard is public (read-only)");
    }

    let app = Arc::new(App {
        auth: auth::Auth::new(cfg.dashboard_password.clone()),
        cache: stats::StatsCache::new(),
        status,
        cfg,
    });

    let router = Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/api/login", post(login))
        .route("/api/stats", get(api_stats))
        .with_state(app.clone());

    let addr = format!("0.0.0.0:{}", app.cfg.port);
    tracing::info!("dashboard listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind port");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal(shutdown_tx))
        .await
        .expect("server error");

    // Wait for the supervisor to kill the miner before exiting.
    let _ = supervisor.await;
}

async fn shutdown_signal(tx: watch::Sender<bool>) {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to install SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
    tracing::info!("shutdown signal received");
    let _ = tx.send(true);
}

fn cookie_header(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::COOKIE).and_then(|v| v.to_str().ok())
}

async fn index(State(app): State<SharedApp>, headers: HeaderMap) -> Html<&'static str> {
    if app.auth.is_authorized(cookie_header(&headers)) {
        Html(DASHBOARD_HTML)
    } else {
        Html(LOGIN_HTML)
    }
}

async fn health(State(app): State<SharedApp>) -> Json<serde_json::Value> {
    let s = app.status.read().await;
    Json(serde_json::json!({
        "status": "ok",
        "miner": {
            "running": s.running,
            "pid": s.pid,
            "restarts": s.restarts,
            "uptime_seconds": s.started_at
                .filter(|_| s.running)
                .map(|t| t.elapsed().as_secs()),
            "last_error": s.last_error,
        }
    }))
}

#[derive(serde::Deserialize)]
struct LoginBody {
    password: String,
}

async fn login(State(app): State<SharedApp>, Json(body): Json<LoginBody>) -> Response {
    match app.auth.login(&body.password) {
        Some(_) => (
            StatusCode::OK,
            [(header::SET_COOKIE, app.auth.cookie())],
            Json(serde_json::json!({"ok": true})),
        )
            .into_response(),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "wrong password"})),
        )
            .into_response(),
    }
}

async fn api_stats(State(app): State<SharedApp>, headers: HeaderMap) -> Response {
    if !app.auth.is_authorized(cookie_header(&headers)) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let (pool, network) = app.cache.get(&app.cfg.wallet).await;
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let s = app.status.read().await;
    Json(serde_json::json!({
        "miner": {
            "running": s.running,
            "restarts": s.restarts,
            "uptime_seconds": s.started_at
                .filter(|_| s.running)
                .map(|t| t.elapsed().as_secs()),
            "threads": app.cfg.threads(cores),
            "cores": cores,
            "power": app.cfg.power,
            "worker": app.cfg.stratum_user(),
            "pool_url": app.cfg.pool_url,
        },
        "pool": pool,
        "network": network,
    }))
    .into_response()
}
