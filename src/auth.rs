//! OAuth for an installed app: PKCE + loopback redirect, with a paste-the-URL
//! fallback for when the browser is on another device. The refresh token is
//! stored in a 0600 file in the state directory.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
static TOKEN_URL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    std::env::var("DRIVESTREAM_TOKEN_URL").unwrap_or_else(|_| "https://oauth2.googleapis.com/token".into())
});
pub const SCOPE_RW: &str = "https://www.googleapis.com/auth/drive";
pub const SCOPE_RO: &str = "https://www.googleapis.com/auth/drive.readonly";

#[derive(Deserialize)]
struct ClientFile {
    installed: Option<Client>,
    web: Option<Client>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Client {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct StoredToken {
    pub client: Client,
    pub refresh_token: String,
    pub scope: String,
    pub email: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    scope: Option<String>,
}

pub fn token_path(state_dir: &Path) -> PathBuf {
    state_dir.join("token.json")
}

/// Accepts Google's downloaded client JSON, or a dotenv file with
/// DRIVESTREAM_CLIENT_ID and DRIVESTREAM_CLIENT_SECRET.
fn load_client(path: &Path) -> Result<Client> {
    let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if let Ok(f) = serde_json::from_slice::<ClientFile>(&raw) {
        return f.installed.or(f.web).ok_or_else(|| anyhow!("client JSON has neither an \"installed\" nor a \"web\" client"));
    }
    let (mut id, mut secret) = (None, None);
    for line in String::from_utf8_lossy(&raw).lines() {
        let line = line.trim().trim_start_matches("export ").trim();
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
        match k.trim() {
            "DRIVESTREAM_CLIENT_ID" => id = Some(v),
            "DRIVESTREAM_CLIENT_SECRET" => secret = Some(v),
            _ => {}
        }
    }
    match (id, secret) {
        (Some(client_id), Some(client_secret)) if !client_id.is_empty() && !client_secret.is_empty() => {
            Ok(Client { client_id, client_secret })
        }
        _ => bail!("{} is neither a Google client JSON nor a dotenv with DRIVESTREAM_CLIENT_ID/DRIVESTREAM_CLIENT_SECRET", path.display()),
    }
}

fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

/// Pull `code` out of a redirect URL (or a bare query string), checking `state`.
fn code_from_redirect(s: &str, want_state: &str) -> Result<String> {
    let s = s.trim();
    let query = s.split_once('?').map(|(_, q)| q).unwrap_or(s);
    let query = query.split_whitespace().next().unwrap_or("");
    let mut code = None;
    let mut state = None;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => bail!("Google returned an error: {v}"),
            _ => {}
        }
    }
    if state.as_deref() != Some(want_state) {
        bail!("state mismatch in redirect (did you paste the URL from this sign-in?)");
    }
    code.ok_or_else(|| anyhow!("no code in redirect"))
}

async fn wait_loopback(listener: tokio::net::TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
        match code_from_redirect(path, state) {
            Ok(code) => {
                let body = "<h2>Drive Stream is signed in.</h2><p>You can close this tab.</p>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                return Ok(code);
            }
            Err(_) => {
                let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
            }
        }
    }
}

async fn wait_paste(state: &str) -> Result<String> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        match code_from_redirect(&line, state) {
            Ok(code) => return Ok(code),
            Err(e) => eprintln!("  couldn't use that: {e}. Paste the full address from the browser's address bar."),
        }
    }
    bail!("stdin closed before a redirect URL was pasted")
}

/// A sign-in in progress: the consent URL to open, and what's needed to
/// trade the code Google sends back for tokens.
pub struct PendingLogin {
    pub url: String,
    pub state: String,
    verifier: String,
    redirect_uri: String,
    client: Client,
    scope: String,
}

impl PendingLogin {
    pub fn client(&self) -> Client {
        self.client.clone()
    }
}

/// Start a sign-in. The returned listener catches Google's redirect when the
/// browser is on this Mac; otherwise the user pastes the final address.
pub async fn begin(client: Client, scope: &str, login_hint: Option<&str>) -> Result<(PendingLogin, tokio::net::TcpListener)> {
    let verifier = random_b64(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_b64(16);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let redirect_uri = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let mut url = url::Url::parse(AUTH_URL)?;
    url.query_pairs_mut()
        .append_pair("client_id", &client.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", scope)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state);
    if let Some(h) = login_hint {
        url.query_pairs_mut().append_pair("login_hint", h);
    }
    let p = PendingLogin { url: url.to_string(), state, verifier, redirect_uri, client, scope: scope.to_string() };
    Ok((p, listener))
}

/// The next request on the loopback listener: the code it carries (or why
/// it doesn't), plus the connection, so the caller can answer the browser
/// once it knows whether the sign-in actually worked.
pub async fn next_redirect(listener: &tokio::net::TcpListener, state: &str) -> Result<(Result<String>, tokio::net::TcpStream)> {
    let (mut sock, _) = listener.accept().await?;
    let mut buf = vec![0u8; 8192];
    let n = sock.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("").to_string();
    if !path.contains("state=") && !path.contains("code=") && !path.contains("error=") {
        return Ok((Err(anyhow!("not a sign-in redirect")), sock)); // e.g. /favicon.ico
    }
    Ok((code_from_redirect(&path, state), sock))
}

/// Answer the browser that landed on the loopback redirect.
pub async fn respond_page(mut sock: tokio::net::TcpStream, ok: bool, title: &str, detail: &str) {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let (color, mark) = if ok { ("#30a14e", "✓") } else { ("#d70015", "!") };
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Drive Stream</title>\
         <body style=\"font:16px -apple-system,sans-serif;display:grid;place-items:center;height:90vh;margin:0;background:#f5f5f7;color:#1d1d1f\">\
         <div style=\"max-width:520px;text-align:center\"><div style=\"font-size:44px;color:{color}\">{mark}</div>\
         <h2>{}</h2><p style=\"color:#6e6e73;line-height:1.5\">{}</p></div>",
        esc(title),
        esc(detail)
    );
    let resp = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        if ok { "200 OK" } else { "400 Bad Request" },
        body.len()
    );
    let _ = sock.write_all(resp.as_bytes()).await;
}

/// Pull the code out of an address pasted from the browser.
pub fn code_from_pasted(pasted: &str, state: &str) -> Result<String> {
    code_from_redirect(pasted, state)
}

/// Trade the code for tokens and find out which account signed in.
pub async fn finish(p: &PendingLogin, code: &str) -> Result<StoredToken> {
    let http = reqwest::Client::new();
    let resp = http
        .post(TOKEN_URL.as_str())
        .form(&[
            ("code", code),
            ("client_id", p.client.client_id.as_str()),
            ("client_secret", p.client.client_secret.as_str()),
            ("redirect_uri", p.redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", p.verifier.as_str()),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!("token exchange failed: {} {}", resp.status(), resp.text().await.unwrap_or_default());
    }
    let tr: TokenResponse = resp.json().await?;
    let refresh_token = tr.refresh_token.ok_or_else(|| anyhow!("Google did not return a refresh token"))?;
    let about: serde_json::Value = http
        .get(format!("{}/drive/v3/about?fields=user(emailAddress)", crate::drive::api_base()))
        .bearer_auth(&tr.access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let email = about["user"]["emailAddress"].as_str().map(String::from);
    Ok(StoredToken { client: p.client.clone(), refresh_token, scope: tr.scope.unwrap_or_else(|| p.scope.clone()), email })
}

pub fn save(state_dir: &Path, tok: &StoredToken) -> Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let path = token_path(state_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(tok)?)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// The command-line sign-in: print the URL, accept the redirect or a paste.
pub async fn login(client_json: &Path, scope: &str, state_dir: &Path, login_hint: Option<&str>) -> Result<StoredToken> {
    let (p, listener) = begin(load_client(client_json)?, scope, login_hint).await?;
    println!("\n1. Open this URL in a browser and approve access:\n\n{}\n", p.url);
    println!("2. If the browser is on this Mac, that's it. If it's on another device, the last page");
    println!("   will fail to load (it points at 127.0.0.1). Copy that page's full address and paste it here:\n");
    let code = tokio::select! {
        c = wait_loopback(listener, &p.state) => c?,
        c = wait_paste(&p.state) => c?,
    };
    let tok = finish(&p, &code).await?;
    save(state_dir, &tok)?;
    Ok(tok)
}

/// Hands out access tokens, refreshing a minute before they expire. The
/// sign-in can be missing (first run) or replaced while running.
pub struct Auth {
    http: reqwest::Client,
    tok: std::sync::RwLock<Option<StoredToken>>,
    cur: Mutex<Option<(String, Instant)>>,
    problem: std::sync::Mutex<Option<String>>,
}

impl Auth {
    pub fn load(state_dir: &Path) -> Self {
        let tok = std::fs::read(token_path(state_dir)).ok().and_then(|raw| serde_json::from_slice(&raw).ok());
        Auth { http: reqwest::Client::new(), tok: std::sync::RwLock::new(tok), cur: Mutex::new(None), problem: std::sync::Mutex::new(None) }
    }

    pub fn signed_in(&self) -> bool {
        self.tok.read().unwrap().is_some()
    }

    pub fn email(&self) -> Option<String> {
        self.tok.read().unwrap().as_ref().and_then(|t| t.email.clone())
    }

    pub fn client(&self) -> Option<Client> {
        self.tok.read().unwrap().as_ref().map(|t| t.client.clone())
    }

    pub fn can_write(&self) -> bool {
        self.tok.read().unwrap().as_ref().is_some_and(|t| t.scope.split_whitespace().any(|s| s == SCOPE_RW))
    }

    /// Why API calls are failing for sign-in reasons, if they are.
    pub fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap().clone()
    }

    /// Swap in a fresh sign-in (same account) without restarting.
    pub async fn install(&self, tok: StoredToken) {
        *self.tok.write().unwrap() = Some(tok);
        *self.cur.lock().await = None;
        *self.problem.lock().unwrap() = None;
    }

    pub async fn token(&self) -> Result<String> {
        let mut cur = self.cur.lock().await;
        if let Some((t, exp)) = cur.as_ref() {
            if Instant::now() + Duration::from_secs(60) < *exp {
                return Ok(t.clone());
            }
        }
        let Some(tok) = self.tok.read().unwrap().clone() else { bail!("not signed in to Google") };
        let resp = self
            .http
            .post(TOKEN_URL.as_str())
            .form(&[
                ("client_id", tok.client.client_id.as_str()),
                ("client_secret", tok.client.client_secret.as_str()),
                ("refresh_token", tok.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if body.contains("invalid_grant") || status == reqwest::StatusCode::UNAUTHORIZED {
                *self.problem.lock().unwrap() = Some(
                    "Google ended the sign-in (expired or revoked). Sign in again to keep syncing. If this happens \
                     every 7 days, your Google Cloud app is still in Testing: publish it first."
                        .into(),
                );
            }
            bail!("token refresh failed: {status} {body}");
        }
        let tr: TokenResponse = resp.json().await?;
        *self.problem.lock().unwrap() = None;
        *cur = Some((tr.access_token.clone(), Instant::now() + Duration::from_secs(tr.expires_in)));
        Ok(tr.access_token)
    }

    pub async fn invalidate(&self) {
        *self.cur.lock().await = None;
    }
}

/// A client entered by hand in the UI (first run, before any sign-in).
pub fn client_from_parts(id: &str, secret: &str) -> Option<Client> {
    (!id.trim().is_empty() && !secret.trim().is_empty())
        .then(|| Client { client_id: id.trim().to_string(), client_secret: secret.trim().to_string() })
}
