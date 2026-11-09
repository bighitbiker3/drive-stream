//! The dashboard: a small local web app (and JSON API, used by the menu bar
//! app too) for signing in to Google and watching uploads and the cache.
//! Listens on 127.0.0.1 only; POSTs need an `X-Drive-Stream` header so other web
//! pages can't drive it, and the Host header must be local (or a host you
//! allowed with `--allow-host`, for a tunnel) so DNS rebinding can't reach it.

use crate::auth::{self, Auth, PendingLogin};
use crate::cache::BlockStore;
use crate::drive::Drive;
use crate::meta::Meta;
use crate::writes::Writes;
use anyhow::Result;
use axum::extract::{Json, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Everything that exists once we're signed in and mounted.
pub struct Core {
    pub drive: Arc<Drive>,
    pub meta: Arc<Meta>,
    pub blocks: Arc<BlockStore>,
    pub writes: Arc<Writes>,
    pub writable: bool,
}

pub struct App {
    pub state_dir: PathBuf,
    pub mountpoint: PathBuf,
    pub auth: Arc<Auth>,
    pub started: Instant,
    pub mounted: AtomicBool,
    pub core: OnceLock<Core>,
    /// Fires when a sign-in completes (startup waits on it).
    pub signed_in: tokio::sync::Notify,
    /// Fires when the daemon should shut down and be restarted (account switch).
    pub restart: tokio::sync::Notify,
    pub restarting: AtomicBool,
    phase: Mutex<String>,
    login: tokio::sync::Mutex<Option<Arc<PendingLogin>>>,
    /// The task listening for the current sign-in's redirect.
    login_task: Mutex<Option<tokio::task::AbortHandle>>,
    login_error: Mutex<Option<String>>,
    events: Mutex<VecDeque<serde_json::Value>>,
}

impl App {
    pub fn new(state_dir: PathBuf, mountpoint: PathBuf, auth: Arc<Auth>, started: Instant) -> Self {
        App {
            state_dir,
            mountpoint,
            auth,
            started,
            mounted: AtomicBool::new(false),
            core: OnceLock::new(),
            signed_in: tokio::sync::Notify::new(),
            restart: tokio::sync::Notify::new(),
            restarting: AtomicBool::new(false),
            phase: Mutex::new("Starting".into()),
            login: tokio::sync::Mutex::new(None),
            login_task: Mutex::new(None),
            login_error: Mutex::new(None),
            events: Mutex::new(VecDeque::new()),
        }
    }

    pub fn set_phase(&self, s: impl Into<String>) {
        *self.phase.lock().unwrap() = s.into();
    }

    /// Something worth showing in the dashboard's activity list.
    pub fn event(&self, level: &str, msg: impl Into<String>) {
        let msg = msg.into();
        match level {
            "error" => tracing::error!("{msg}"),
            "warn" => tracing::warn!("{msg}"),
            _ => tracing::info!("{msg}"),
        }
        let mut ev = self.events.lock().unwrap();
        ev.push_front(serde_json::json!({ "at": chrono::Utc::now().timestamp(), "level": level, "msg": msg }));
        ev.truncate(50);
    }

    fn status(&self) -> serde_json::Value {
        use std::sync::atomic::Ordering::Relaxed as R;
        let core = self.core.get();
        let uploads = core.map(|c| {
            let w = &c.writes;
            serde_json::json!({
                "active": w.snapshot(&c.meta),
                "history": w.history(),
                "totals": {
                    "uploads": w.stats.uploads.load(R),
                    "failures": w.stats.upload_failures.load(R),
                    "checksum_mismatches": w.stats.checksum_mismatches.load(R),
                },
            })
        });
        let drive = core.map(|c| {
            let s = &c.drive.stats;
            let (cache_bytes, cache_blocks) = c.blocks.disk_usage();
            serde_json::json!({
                "files": c.meta.tree().nodes.len(),
                "writable": c.writable,
                "cache_bytes": cache_bytes,
                "cache_blocks": cache_blocks,
                "cache_budget": c.blocks.budget(),
                "bytes_downloaded": s.bytes_down.load(R),
                "bytes_uploaded": s.bytes_up.load(R),
                "requests": s.requests.load(R),
                "quota_units": s.quota_units.load(R),
                "quota_units_last_min": s.units_last_minute(),
                "retries": s.retries.load(R),
            })
        });
        let pending_login = self.login.try_lock().ok().and_then(|l| l.as_ref().map(|p| p.url.clone()));
        serde_json::json!({
            "phase": *self.phase.lock().unwrap(),
            "mounted": self.mounted.load(Relaxed),
            "mountpoint": self.mountpoint,
            "mountpoint_display": std::env::var("HOME").ok().and_then(|h| self.mountpoint.strip_prefix(&h).ok().map(|r| format!("~/{}", r.display()))),
            "uptime_s": self.started.elapsed().as_secs(),
            "account": {
                "email": self.auth.email(),
                "signed_in": self.auth.signed_in(),
                "problem": self.auth.problem(),
                "client_configured": self.auth.client().is_some(),
            },
            "signin": {
                "in_progress": pending_login.is_some(),
                "url": pending_login,
                "error": *self.login_error.lock().unwrap(),
            },
            "uploads": uploads,
            "drive": drive,
            "events": self.events.lock().unwrap().iter().cloned().collect::<Vec<_>>(),
        })
    }

    /// Trade a code for tokens; whichever of redirect or paste gets here
    /// first wins, the other finds the login already taken.
    async fn complete_login(self: &Arc<Self>, p: Arc<PendingLogin>, code: String) -> Result<String, String> {
        {
            let mut slot = self.login.lock().await;
            if !slot.as_ref().is_some_and(|cur| Arc::ptr_eq(cur, &p)) {
                return Err("this sign-in was already finished or replaced".into());
            }
            *slot = None;
        }
        let tok = match auth::finish(&p, &code).await {
            Ok(t) => t,
            Err(e) => {
                let msg = format!("{e:#}");
                *self.login_error.lock().unwrap() = Some(msg.clone());
                self.event("error", format!("Google sign-in failed: {msg}"));
                return Err(msg);
            }
        };
        let email = tok.email.clone().unwrap_or_default();
        if let Err(e) = auth::save(&self.state_dir, &tok) {
            return Err(format!("couldn't save the sign-in: {e:#}"));
        }
        *self.login_error.lock().unwrap() = None;
        let previous = self.auth.email();
        if previous.is_some() && previous.as_deref() != Some(email.as_str()) && self.core.get().is_some() {
            // A different account means a different drive: restart cleanly
            // (pending uploads are flushed first) and start over for it.
            self.event("info", format!("Signed in as {email}; restarting to switch accounts"));
            self.restarting.store(true, Relaxed);
            self.restart.notify_one();
        } else {
            self.auth.install(tok).await;
            self.event("info", format!("Signed in to Google as {email}"));
            self.signed_in.notify_one();
        }
        Ok(email)
    }
}

#[derive(Deserialize, Default)]
struct StartReq {
    client_id: Option<String>,
    client_secret: Option<String>,
}

#[derive(Deserialize)]
struct FinishReq {
    redirect: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

async fn index() -> impl IntoResponse {
    Html(include_str!("../ui/index.html"))
}

async fn icon() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], include_bytes!("../assets/icon-256.png").as_slice())
}

async fn status(State(app): State<Arc<App>>) -> impl IntoResponse {
    Json(app.status())
}

async fn signin_start(State(app): State<Arc<App>>, body: Option<Json<StartReq>>) -> Response {
    let req = body.map(|b| b.0).unwrap_or_default();
    let typed = auth::client_from_parts(req.client_id.as_deref().unwrap_or(""), req.client_secret.as_deref().unwrap_or(""));
    // Starting over reuses the client from a saved sign-in, or from the one in progress.
    let pending_client = app.login.lock().await.as_ref().map(|p| p.client());
    let Some(client) = typed.or_else(|| app.auth.client()).or(pending_client) else {
        return err(StatusCode::BAD_REQUEST, "Enter your Google OAuth client ID and secret first.");
    };
    let hint = app.auth.email();
    let (p, listener) = match auth::begin(client, auth::SCOPE_RW, hint.as_deref()).await {
        Ok(x) => x,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    };
    let p = Arc::new(p);
    *app.login.lock().await = Some(p.clone());
    *app.login_error.lock().unwrap() = None;
    let url = p.url.clone();
    // If the browser is on this Mac, Google's redirect lands here. Only the
    // newest sign-in listens: starting another closes the previous one.
    let a = app.clone();
    let task = tokio::spawn(async move {
        let listen = async {
            loop {
                let Ok((parsed, sock)) = auth::next_redirect(&listener, &p.state).await else { continue };
                match parsed {
                    Ok(code) => {
                        // Answer the browser only once we know how it went.
                        match a.complete_login(p.clone(), code).await {
                            Ok(email) => {
                                auth::respond_page(sock, true, &format!("Signed in as {email}"), "Drive Stream is syncing again. You can close this tab.").await
                            }
                            Err(e) => {
                                auth::respond_page(sock, false, "Couldn't finish signing in", &format!("{e}. Go back to the Drive Stream dashboard and click “Sign in with Google” to try again.")).await
                            }
                        }
                        return;
                    }
                    Err(e) if e.to_string().contains("Google returned an error") => {
                        auth::respond_page(sock, false, "Sign-in was cancelled", &format!("{e}. You can start again from the Drive Stream dashboard.")).await;
                    }
                    Err(e) if e.to_string().contains("state mismatch") => {
                        auth::respond_page(sock, false, "This sign-in link is out of date", "Start a new one from the Drive Stream dashboard with “Sign in with Google”.").await;
                    }
                    Err(_) => auth::respond_page(sock, false, "Not found", "").await,
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(2 * 3600), listen).await;
    });
    if let Some(old) = app.login_task.lock().unwrap().replace(task.abort_handle()) {
        old.abort();
    }
    Json(serde_json::json!({ "url": url })).into_response()
}

async fn signin_finish(State(app): State<Arc<App>>, Json(req): Json<FinishReq>) -> Response {
    let Some(p) = app.login.lock().await.clone() else {
        return err(StatusCode::BAD_REQUEST, "No sign-in in progress. Click “Sign in with Google” first.");
    };
    let code = match auth::code_from_pasted(&req.redirect, &p.state) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("That address didn't work: {e:#}. Paste the full address from the browser's address bar.")),
    };
    match app.complete_login(p, code).await {
        Ok(email) => Json(serde_json::json!({ "email": email })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

async fn signin_cancel(State(app): State<Arc<App>>) -> impl IntoResponse {
    *app.login.lock().await = None;
    if let Some(t) = app.login_task.lock().unwrap().take() {
        t.abort();
    }
    Json(serde_json::json!({ "ok": true }))
}

async fn open_finder(State(app): State<Arc<App>>) -> Response {
    match std::process::Command::new("/usr/bin/open").arg(&app.mountpoint).status() {
        Ok(s) if s.success() => Json(serde_json::json!({ "ok": true })).into_response(),
        _ => err(StatusCode::INTERNAL_SERVER_ERROR, "couldn't open Finder"),
    }
}

async fn cache_clear(State(app): State<Arc<App>>) -> Response {
    let Some(c) = app.core.get() else { return err(StatusCode::CONFLICT, "not running yet") };
    let freed = c.blocks.clear().await;
    app.event("info", format!("Cleared the cache ({:.1} GB)", freed as f64 / 1e9));
    Json(serde_json::json!({ "freed": freed })).into_response()
}

async fn uploads_retry(State(app): State<Arc<App>>) -> Response {
    let Some(c) = app.core.get() else { return err(StatusCode::CONFLICT, "not running yet") };
    c.writes.retry_now();
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// Host names the dashboard answers to: always localhost, plus whatever was
/// passed with `--allow-host`. An entry starting with "." allows any
/// subdomain (".example.com" matches "a.example.com"), otherwise exact.
#[derive(Clone, Default)]
pub struct AllowedHosts(pub Vec<String>);

impl AllowedHosts {
    fn allows(&self, host: &str) -> bool {
        let name = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host).to_ascii_lowercase();
        matches!(name.as_str(), "127.0.0.1" | "localhost")
            || self.0.iter().any(|a| {
                let a = a.trim().to_ascii_lowercase();
                if a.starts_with('.') { name.ends_with(&a) } else { !a.is_empty() && name == a }
            })
    }
}

async fn guard(State(allowed): State<Arc<AllowedHosts>>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    if !allowed.allows(host) {
        return err(StatusCode::FORBIDDEN, "unrecognized host");
    }
    if req.method() == Method::POST && !req.headers().contains_key("x-drive-stream") {
        return err(StatusCode::FORBIDDEN, "missing X-Drive-Stream header");
    }
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

pub async fn serve(app: Arc<App>, port: u16, allowed: AllowedHosts) -> Result<()> {
    let router = Router::new()
        .route("/", get(index))
        .route("/icon.png", get(icon))
        .route("/api/status", get(status))
        .route("/api/signin/start", post(signin_start))
        .route("/api/signin/finish", post(signin_finish))
        .route("/api/signin/cancel", post(signin_cancel))
        .route("/api/open-finder", post(open_finder))
        .route("/api/cache/clear", post(cache_clear))
        .route("/api/uploads/retry", post(uploads_retry))
        .layer(middleware::from_fn_with_state(Arc::new(allowed), guard))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    axum::serve(listener, router.into_make_service()).await?;
    Ok(())
}
