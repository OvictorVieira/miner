mod auth;
mod bitcoin_address;
mod ci_policy;
mod compose_policy;
mod config;
mod cpuminer_policy;
mod dockerfile_policy;
mod fake_stratum;
mod fork_identity_policy;
mod miner;
mod payout_identity;
mod readme_policy;
mod sha256d_self_test;
mod stats;

use std::{sync::Arc, time::Duration};

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{header, HeaderMap, HeaderValue, Request, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use miner::SharedStatus;
use tokio::sync::{watch, RwLock};

const DASHBOARD_HTML: &str = include_str!("../assets/dashboard.html");
const DASHBOARD_CSS: &str = include_str!("../assets/dashboard.css");
const DASHBOARD_JS: &str = include_str!("../assets/dashboard.js");
const LOGIN_HTML: &str = include_str!("../assets/login.html");
const LOGIN_CSS: &str = include_str!("../assets/login.css");
const LOGIN_JS: &str = include_str!("../assets/login.js");

const MAX_LOGIN_BODY_BYTES: usize = 1024;
// Every rejected login pays the same delay. This slows online guessing with
// no attacker-controlled keys or other growing in-memory state.
const LOGIN_FAILURE_DELAY: Duration = Duration::from_millis(200);

// Every directive the pages need to work is `'self'` — the HTML ships its
// styles and scripts as local assets, so no inline/external allowance exists.
// `default-src 'self'` is what makes viewing the dashboard cause no
// third-party request (US-040); explicit script/style sources document that
// inline code is intentionally rejected.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
                   frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

const SECURITY_HEADERS: &[(&str, &str)] = &[
    ("Content-Security-Policy", CSP),
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
    ("Cache-Control", "no-store, max-age=0"),
];

fn add_security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    for (name, value) in SECURITY_HEADERS {
        if let (Ok(header_name), Ok(header_value)) = (
            name.parse::<header::HeaderName>(),
            value.parse::<HeaderValue>(),
        ) {
            headers.insert(header_name, header_value);
        }
    }
    response
}

struct App {
    cfg: config::Config,
    status: SharedStatus,
    cache: stats::StatsCache,
    auth: auth::Auth,
}

type SharedApp = Arc<App>;

#[tokio::main]
async fn main() {
    if std::env::args().skip(1).any(|a| a == "--self-test") {
        match sha256d_self_test::run() {
            Ok(()) => {
                println!("sha256d self-test: OK");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("sha256d self-test: FAIL\n{e}");
                std::process::exit(1);
            }
        }
    }

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
        .route("/assets/dashboard.css", get(dashboard_css))
        .route("/assets/dashboard.js", get(dashboard_js))
        .route("/assets/login.css", get(login_css))
        .route("/assets/login.js", get(login_js))
        .with_state(app.clone());

    let addr = std::net::SocketAddr::new(app.cfg.bind_address, app.cfg.port);
    let policy = if app.cfg.bind_address.is_loopback() {
        "loopback-only"
    } else {
        "wildcard (container; host publishes 127.0.0.1 only)"
    };
    tracing::info!("dashboard listening on http://{addr} (bind policy: {policy})");
    let listener = tokio::net::TcpListener::bind(addr)
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

async fn index(State(app): State<SharedApp>, headers: HeaderMap) -> Response {
    let html = if app.auth.is_authorized(cookie_header(&headers)) {
        DASHBOARD_HTML
    } else {
        LOGIN_HTML
    };
    add_security_headers(Html(html).into_response())
}

// Local-only assets: styles and scripts are embedded at compile time and
// served from this origin, so the strict CSP needs no inline allowance.
fn asset_response(content_type: &'static str, body: &'static str) -> Response {
    add_security_headers(([(header::CONTENT_TYPE, content_type)], body).into_response())
}

async fn dashboard_css() -> Response {
    asset_response("text/css; charset=utf-8", DASHBOARD_CSS)
}

async fn dashboard_js() -> Response {
    asset_response("text/javascript; charset=utf-8", DASHBOARD_JS)
}

async fn login_css() -> Response {
    asset_response("text/css; charset=utf-8", LOGIN_CSS)
}

async fn login_js() -> Response {
    asset_response("text/javascript; charset=utf-8", LOGIN_JS)
}

async fn health() -> Response {
    // Liveness is intentionally independent of miner state. Keep this body
    // fixed and minimal: process details belong only on /api/stats.
    add_security_headers(Json(serde_json::json!({"status": "ok"})).into_response())
}

#[derive(serde::Deserialize)]
struct LoginBody {
    password: String,
}

async fn login(State(app): State<SharedApp>, request: Request<Body>) -> Response {
    let bytes = match to_bytes(request.into_body(), MAX_LOGIN_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            tokio::time::sleep(LOGIN_FAILURE_DELAY).await;
            return add_security_headers(
                (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Json(serde_json::json!({"error": "login request too large"})),
                )
                    .into_response(),
            );
        }
    };
    let body: LoginBody = match serde_json::from_slice(&bytes) {
        Ok(body) => body,
        Err(_) => {
            tokio::time::sleep(LOGIN_FAILURE_DELAY).await;
            return add_security_headers(
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "invalid login request"})),
                )
                    .into_response(),
            );
        }
    };

    match app.auth.login(&body.password) {
        Some(_) => add_security_headers(
            (
                StatusCode::OK,
                [(header::SET_COOKIE, app.auth.cookie())],
                Json(serde_json::json!({"ok": true})),
            )
                .into_response(),
        ),
        None => {
            tokio::time::sleep(LOGIN_FAILURE_DELAY).await;
            add_security_headers(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "wrong password"})),
                )
                    .into_response(),
            )
        }
    }
}

async fn api_stats(State(app): State<SharedApp>, headers: HeaderMap) -> Response {
    if !app.auth.is_authorized(cookie_header(&headers)) {
        return add_security_headers(StatusCode::UNAUTHORIZED.into_response());
    }

    let (pool, network) = app.cache.get(&app.cfg.payout_address).await;
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let s = app.status.read().await;
    add_security_headers(
        Json(serde_json::json!({
            "miner": {
                "running": s.running,
                "pid": s.pid,
                "restarts": s.restarts,
                "uptime_seconds": s.started_at
                    .filter(|_| s.running)
                    .map(|t| t.elapsed().as_secs()),
                "last_error": s.last_error,
                "threads": app.cfg.threads(cores),
                "cores": cores,
                "power": app.cfg.power,
                "worker": app.cfg.pool_username,
                "pool_url": app.cfg.pool_url,
            },
            "pool": pool,
            "network": network,
        }))
        .into_response(),
    )
}

#[cfg(test)]
mod security_headers_tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        Router,
    };
    use tower::util::ServiceExt;

    fn test_cfg() -> config::Config {
        config::Config::from_vars(|k| {
            (k == "WALLET").then(|| "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_string())
        })
        .expect("test config must load")
    }

    /// `password` is always `Some` for API routes that would otherwise take
    /// the authorized path: an authorized `/api/stats` fetches mempool.space,
    /// and these tests must stay offline.
    fn test_app(password: Option<&str>) -> Router {
        let status: SharedStatus = Arc::new(RwLock::new(miner::MinerStatus::default()));
        let app = Arc::new(App {
            auth: auth::Auth::new(password.map(str::to_string)),
            cache: stats::StatsCache::new(),
            status,
            cfg: test_cfg(),
        });
        Router::new()
            .route("/", get(index))
            .route("/health", get(health))
            .route("/api/login", post(login))
            .route("/api/stats", get(api_stats))
            .route("/assets/dashboard.css", get(dashboard_css))
            .route("/assets/dashboard.js", get(dashboard_js))
            .route("/assets/login.css", get(login_css))
            .route("/assets/login.js", get(login_js))
            .with_state(app)
    }

    fn assert_security_headers(response: &Response) {
        let headers = response.headers();
        assert_eq!(headers.get("content-security-policy").unwrap(), CSP);
        assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(headers.get("referrer-policy").unwrap(), "no-referrer");
        assert_eq!(headers.get("cache-control").unwrap(), "no-store, max-age=0");
    }

    async fn get_response(app: Router, uri: &str) -> Response {
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn security_headers_on_dashboard() {
        let response = get_response(test_app(None), "/").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_security_headers(&response);
    }

    #[tokio::test]
    async fn security_headers_on_login_page() {
        let response = get_response(test_app(Some("hunter2")), "/").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_security_headers(&response);
    }

    #[tokio::test]
    async fn security_headers_on_health() {
        let response = get_response(test_app(None), "/health").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_security_headers(&response);
    }

    #[tokio::test]
    async fn health_is_a_fixed_bounded_liveness_response() {
        let app = test_app(None);
        let response = get_response(app, "/health").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_security_headers(&response);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("health must declare its JSON content type");
        assert_eq!(content_type, "application/json");

        const MAX_HEALTH_BYTES: usize = 32;
        let body = axum::body::to_bytes(response.into_body(), MAX_HEALTH_BYTES)
            .await
            .expect("health response must remain bounded");
        assert_eq!(body.as_ref(), br#"{"status":"ok"}"#);

        let text = std::str::from_utf8(&body).unwrap();
        for forbidden in [
            "miner", "running", "pid", "wallet", "worker", "pool", "error", "restart", "uptime",
        ] {
            assert!(
                !text.contains(forbidden),
                "/health leaked forbidden process detail {forbidden:?}: {text}"
            );
        }
    }

    #[tokio::test]
    async fn security_headers_on_api_login_rejection() {
        let app = test_app(Some("hunter2"));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"password":"wrong"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_security_headers(&response);
    }

    async fn login_response(app: Router, password: &str) -> Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"password": password}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn login_accepts_correct_password_and_sets_twelve_hour_cookie() {
        let response = login_response(test_app(Some("hunter2")), "hunter2").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_security_headers(&response);
        let cookie = response.headers().get(header::SET_COOKIE).unwrap();
        assert!(cookie.to_str().unwrap().contains("Max-Age=43200"));
    }

    #[tokio::test]
    async fn login_rejects_wrong_and_empty_passwords() {
        for attempt in ["wrong", ""] {
            let response = login_response(test_app(Some("hunter2")), attempt).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_security_headers(&response);
        }
    }

    #[tokio::test]
    async fn login_rejects_oversized_body() {
        let oversized = format!(r#"{{"password":"{}"}}"#, "x".repeat(MAX_LOGIN_BODY_BYTES));
        let response = test_app(Some("hunter2"))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(oversized))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_security_headers(&response);
    }

    #[tokio::test]
    async fn repeated_failed_logins_are_uniformly_delayed() {
        let started = std::time::Instant::now();
        for _ in 0..3 {
            let response = login_response(test_app(Some("hunter2")), "wrong").await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        assert!(
            started.elapsed() >= LOGIN_FAILURE_DELAY * 3,
            "every failed attempt must pay the configured delay"
        );
    }

    #[tokio::test]
    async fn security_headers_on_api_stats_unauthorized() {
        let response = get_response(test_app(Some("hunter2")), "/api/stats").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_security_headers(&response);
    }

    /// Every asset the pages load — all served from this origin.
    const ASSETS: [(&str, &str); 6] = [
        ("dashboard.html", DASHBOARD_HTML),
        ("dashboard.css", DASHBOARD_CSS),
        ("dashboard.js", DASHBOARD_JS),
        ("login.html", LOGIN_HTML),
        ("login.css", LOGIN_CSS),
        ("login.js", LOGIN_JS),
    ];

    /// Every `<...>` tag in document order. Raw-text element content is
    /// skipped so code like `i < units.length` is never parsed as a tag.
    fn tags_of(html: &str) -> Vec<String> {
        let mut tags = Vec::new();
        let mut rest = html;
        while let Some(start) = rest.find('<') {
            let Some(end) = rest[start..].find('>') else {
                break;
            };
            let tag = rest[start..start + end + 1].to_string();
            rest = &rest[start + end + 1..];
            let tag_name = tag_name_of(&tag);
            if matches!(tag_name.as_str(), "script" | "style") {
                let lower = rest.to_ascii_lowercase();
                let close = format!("</{tag_name}>");
                if let Some(at) = lower.find(&close) {
                    rest = &rest[at + close.len()..];
                }
            }
            tags.push(tag);
        }
        tags
    }

    fn tag_name_of(tag: &str) -> String {
        tag[1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase()
    }

    /// Lowercased attribute names on a tag (`style`, `onclick`, ...).
    /// Quote-aware, so an `=` inside a URL value does not invent a name.
    fn attr_names(tag: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = tag.trim_start_matches('<').trim_end_matches('>');
        let Some(name_end) = rest.find(char::is_whitespace) else {
            return names;
        };
        rest = &rest[name_end..]; // skip the tag name
        loop {
            rest = rest.trim_start();
            if rest.is_empty() || rest.starts_with('/') {
                break;
            }
            let len = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                .unwrap_or(rest.len());
            if len == 0 {
                break;
            }
            let name = rest[..len].to_ascii_lowercase();
            rest = rest[len..].trim_start();
            if let Some(after_eq) = rest.strip_prefix('=') {
                let value = after_eq.trim_start();
                rest = match value.as_bytes().first() {
                    Some(&q @ (b'"' | b'\'')) => {
                        let close = value[1..]
                            .find(q as char)
                            .map(|at| at + 2)
                            .unwrap_or(value.len());
                        &value[close..]
                    }
                    _ => {
                        let end = value.find(char::is_whitespace).unwrap_or(value.len());
                        &value[end..]
                    }
                };
            }
            names.push(name);
        }
        names
    }

    /// `src=`/`href=` values declared on a tag (quotes stripped).
    fn subresource_values(tag: &str) -> Vec<String> {
        let mut values = Vec::new();
        for attr in ["src=", "href="] {
            for piece in tag.split(attr).skip(1) {
                let piece = piece.trim_start_matches(['"', '\'']);
                values.push(
                    piece
                        .chars()
                        .take_while(|c| !matches!(c, '"' | '\'' | '>' | ' ' | '\t' | '\n'))
                        .collect(),
                );
            }
        }
        values
    }

    /// US-040: viewing the dashboard must cause no third-party request.
    /// `default-src 'self'` already blocks such loads at runtime; this test
    /// pins the sources so the CSP is never the only thing standing between a
    /// future edit and an outbound request. Anchor navigations (`<a href>`)
    /// are user-initiated and allowed — subresource attributes are not.
    #[test]
    fn assets_reference_no_third_party_subresources() {
        for (name, html) in [
            ("dashboard.html", DASHBOARD_HTML),
            ("login.html", LOGIN_HTML),
        ] {
            for tag in tags_of(html) {
                let tag_name = tag_name_of(&tag);
                // `<a>`/`<area>` navigations are user-initiated, not page loads.
                if matches!(tag_name.as_str(), "a" | "area" | "base") {
                    continue;
                }
                for value in subresource_values(&tag) {
                    assert!(
                        value.is_empty()
                            || value.starts_with('#')
                            || value.starts_with('/')
                            || value.starts_with("data:"),
                        "{name} loads a third-party subresource: <{tag_name}> {value}"
                    );
                }
            }
        }

        // Scripted requests in HTML, CSS, and JS must stay on the origin too.
        for (name, content) in ASSETS {
            for needle in [
                "fetch(\"http",
                "fetch('http",
                "@import",
                "url(http",
                "url(\"http",
            ] {
                assert!(
                    !content.contains(needle),
                    "{name} contains a third-party request primitive: {needle}"
                );
            }
            let lower = content.to_ascii_lowercase();
            assert!(
                !lower.contains("fonts.googleapis.com") && !lower.contains("fonts.gstatic.com"),
                "{name} still references Google Fonts"
            );
        }
    }

    /// The pages must keep working under `script-src 'self'` / `style-src
    /// 'self'` with no inline allowance: browsers block inline `<script>`,
    /// `<style>` blocks, `style="..."` attributes, and `on*=` handlers under
    /// this CSP, so any of these is a silently broken dashboard.
    #[test]
    fn pages_are_compatible_with_the_strict_csp() {
        for (name, html) in [
            ("dashboard.html", DASHBOARD_HTML),
            ("login.html", LOGIN_HTML),
        ] {
            assert!(
                !html.to_ascii_lowercase().contains("<style"),
                "{name} has an inline <style> block; move it to a local asset"
            );
            for tag in tags_of(html) {
                if tag_name_of(&tag) == "script" {
                    assert!(
                        tag.contains("src="),
                        "{name} has an inline <script>; move it to a local asset: {tag}"
                    );
                }
                for attr in attr_names(&tag) {
                    assert_ne!(
                        attr, "style",
                        "{name} uses a style attribute the strict CSP blocks: {tag}"
                    );
                    assert!(
                        !attr.starts_with("on"),
                        "{name} uses an inline event handler the strict CSP blocks: {tag}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn local_assets_are_served_from_this_origin() {
        for (uri, content_type, sample) in [
            ("/assets/dashboard.css", "text/css", ":root"),
            ("/assets/dashboard.js", "text/javascript", "fmtHash"),
            ("/assets/login.css", "text/css", ":root"),
            ("/assets/login.js", "text/javascript", "fetch("),
        ] {
            let response = get_response(test_app(None), uri).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_security_headers(&response);
            let ct = response
                .headers()
                .get("content-type")
                .unwrap_or_else(|| panic!("{uri} must set content-type"))
                .to_str()
                .unwrap();
            assert!(ct.starts_with(content_type), "{uri}: content-type {ct}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert!(body.contains(sample), "{uri} served unexpected body");
        }
    }

    /// Every local asset the pages reference must resolve on this origin —
    /// a dead reference fails silently in the browser, and the third-party
    /// subresource test only proves the reference is relative, not that it
    /// answers.
    #[tokio::test]
    async fn every_asset_referenced_by_the_pages_is_served() {
        for (name, html) in [
            ("dashboard.html", DASHBOARD_HTML),
            ("login.html", LOGIN_HTML),
        ] {
            for tag in tags_of(html) {
                for value in subresource_values(&tag) {
                    if !value.starts_with("/assets/") {
                        continue;
                    }
                    let response = get_response(test_app(None), &value).await;
                    assert_eq!(
                        response.status(),
                        StatusCode::OK,
                        "{name} references {value}, which is not served"
                    );
                }
            }
        }
    }
}
