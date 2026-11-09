//! Thin Drive v3 client: listing, the change feed, ranged downloads, and the
//! few metadata writes the filesystem needs. Every call is counted against
//! the quota-unit costs Google publishes so we can see real usage.

use crate::auth::Auth;
use anyhow::{Result, anyhow, bail};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

// Overridable so the whole stack can be exercised against a local fake Drive.
static BASE: LazyLock<String> =
    LazyLock::new(|| std::env::var("DRIVESTREAM_API_BASE").unwrap_or_else(|_| "https://www.googleapis.com".into()));
pub fn api_base() -> &'static str {
    BASE.as_str()
}
static API: LazyLock<String> = LazyLock::new(|| format!("{}/drive/v3", *BASE));
static UPLOAD: LazyLock<String> = LazyLock::new(|| format!("{}/upload/drive/v3", *BASE));
pub const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const FIELDS: &str = "id,name,mimeType,size,parents,modifiedTime,md5Checksum,headRevisionId,trashed";

// Quota-unit costs from developers.google.com/workspace/drive/api/guides/limits.
const COST_READ: u64 = 5;
const COST_LIST: u64 = 100;
const COST_DOWNLOAD: u64 = 200;
const COST_EDIT: u64 = 50;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<String>,
    pub parents: Option<Vec<String>>,
    pub modified_time: Option<String>,
    pub md5_checksum: Option<String>,
    pub head_revision_id: Option<String>,
    pub trashed: Option<bool>,
}

impl DFile {
    pub fn size(&self) -> u64 {
        self.size.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0)
    }
    pub fn is_folder(&self) -> bool {
        self.mime_type == FOLDER_MIME
    }
    /// Changes only when content changes, so renames keep their cache.
    pub fn content_version(&self) -> String {
        self.md5_checksum
            .clone()
            .or_else(|| self.head_revision_id.clone())
            .or_else(|| self.modified_time.clone())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileList {
    next_page_token: Option<String>,
    files: Vec<DFile>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub file_id: String,
    pub removed: Option<bool>,
    pub file: Option<DFile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeList {
    pub next_page_token: Option<String>,
    pub new_start_page_token: Option<String>,
    pub changes: Vec<Change>,
}

#[derive(Default)]
pub struct ApiStats {
    pub requests: AtomicU64,
    pub downloads: AtomicU64,
    pub bytes_down: AtomicU64,
    pub bytes_up: AtomicU64,
    pub quota_units: AtomicU64,
    pub retries: AtomicU64,
    pub download_ms: AtomicU64,
    pub ttfb_ms: AtomicU64,
    /// (when, quota units) for calls in the last minute.
    recent_units: std::sync::Mutex<std::collections::VecDeque<(std::time::Instant, u64)>>,
}

impl ApiStats {
    fn charge(&self, units: u64) {
        self.quota_units.fetch_add(units, Relaxed);
        let mut r = self.recent_units.lock().unwrap();
        let now = std::time::Instant::now();
        r.push_back((now, units));
        while r.front().is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(60)) {
            r.pop_front();
        }
    }

    /// Quota units spent in the last 60 seconds (Google's limit is per minute).
    pub fn units_last_minute(&self) -> u64 {
        let r = self.recent_units.lock().unwrap();
        let now = std::time::Instant::now();
        r.iter().filter(|(t, _)| now.duration_since(*t) <= Duration::from_secs(60)).map(|(_, u)| u).sum()
    }
}

pub struct Drive {
    http: reqwest::Client,
    auth: Arc<Auth>,
    pub stats: ApiStats,
}

impl Drive {
    pub fn new(auth: Arc<Auth>) -> Self {
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(32)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .build()
            .expect("http client");
        Drive { http, auth, stats: ApiStats::default() }
    }

    /// Send with bearer auth, retrying rate limits and server errors with
    /// truncated exponential backoff, and one token refresh on 401.
    async fn send(&self, cost: u64, build: impl Fn(&reqwest::Client) -> RequestBuilder) -> Result<reqwest::Response> {
        let mut attempt = 0u32;
        loop {
            let tok = self.auth.token().await?;
            self.stats.requests.fetch_add(1, Relaxed);
            self.stats.charge(cost);
            let resp = build(&self.http).bearer_auth(tok).send().await;
            let retry_reason = match resp {
                Ok(r) if r.status().is_success() => return Ok(r),
                Ok(r) if r.status() == StatusCode::UNAUTHORIZED && attempt == 0 => {
                    self.auth.invalidate().await;
                    "401".to_string()
                }
                Ok(r) => {
                    let status = r.status();
                    let body = r.text().await.unwrap_or_default();
                    let rate_limited = status == StatusCode::TOO_MANY_REQUESTS
                        || (status == StatusCode::FORBIDDEN && body.contains("ateLimitExceeded"));
                    if !(rate_limited || status.is_server_error()) || attempt >= 6 {
                        bail!("Drive API {status}: {}", body.chars().take(400).collect::<String>());
                    }
                    format!("{status}")
                }
                Err(e) if attempt < 6 && (e.is_timeout() || e.is_connect() || e.is_request()) => e.to_string(),
                Err(e) => return Err(e.into()),
            };
            attempt += 1;
            self.stats.retries.fetch_add(1, Relaxed);
            let wait = Duration::from_millis((1000u64 << attempt.min(5)).min(32_000) + rand::random::<u64>() % 1000);
            tracing::warn!("drive request retry {attempt} in {wait:?}: {retry_reason}");
            tokio::time::sleep(wait).await;
        }
    }

    pub async fn get_file(&self, id: &str) -> Result<DFile> {
        let url = format!("{}/files/{id}", *API);
        let r = self.send(COST_READ, |c| c.get(&url).query(&[("fields", FIELDS)])).await?;
        Ok(r.json().await?)
    }

    pub async fn list_all(&self, mut progress: impl FnMut(usize)) -> Result<Vec<DFile>> {
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        let fields = format!("nextPageToken,files({FIELDS})");
        loop {
            let r = self
                .send(COST_LIST, |c| {
                    let mut q = vec![
                        ("q", "trashed=false".to_string()),
                        ("pageSize", "1000".to_string()),
                        ("spaces", "drive".to_string()),
                        ("fields", fields.clone()),
                    ];
                    if let Some(p) = &page {
                        q.push(("pageToken", p.clone()));
                    }
                    c.get(format!("{}/files", *API)).query(&q)
                })
                .await?;
            let fl: FileList = r.json().await?;
            out.extend(fl.files);
            progress(out.len());
            match fl.next_page_token {
                Some(p) => page = Some(p),
                None => return Ok(out),
            }
        }
    }

    pub async fn start_page_token(&self) -> Result<String> {
        let r = self.send(COST_READ, |c| c.get(format!("{}/changes/startPageToken", *API))).await?;
        let v: serde_json::Value = r.json().await?;
        v["startPageToken"].as_str().map(String::from).ok_or_else(|| anyhow!("no startPageToken"))
    }

    pub async fn changes(&self, token: &str) -> Result<ChangeList> {
        let fields = format!("nextPageToken,newStartPageToken,changes(fileId,removed,file({FIELDS}))");
        let r = self
            .send(COST_LIST, |c| {
                c.get(format!("{}/changes", *API)).query(&[
                    ("pageToken", token),
                    ("pageSize", "1000"),
                    ("spaces", "drive"),
                    ("includeRemoved", "true"),
                    ("fields", fields.as_str()),
                ])
            })
            .await?;
        Ok(r.json().await?)
    }

    /// Start a Range download of bytes [start, end); the caller streams the
    /// body with `chunk()` so readers can use bytes as they arrive.
    pub async fn open_range(&self, id: &str, start: u64, end: u64) -> Result<reqwest::Response> {
        let url = format!("{}/files/{id}", *API);
        let range = format!("bytes={}-{}", start, end - 1);
        self.send(COST_DOWNLOAD, |c| c.get(&url).query(&[("alt", "media")]).header("Range", range.as_str()))
            .await
    }

    pub fn note_first_byte(&self, d: Duration) {
        self.stats.ttfb_ms.fetch_add(d.as_millis() as u64, Relaxed);
    }

    pub fn note_download(&self, bytes: u64, d: Duration) {
        self.stats.downloads.fetch_add(1, Relaxed);
        self.stats.bytes_down.fetch_add(bytes, Relaxed);
        self.stats.download_ms.fetch_add(d.as_millis() as u64, Relaxed);
    }

    pub async fn create_folder(&self, name: &str, parent: &str) -> Result<DFile> {
        let body = serde_json::json!({ "name": name, "mimeType": FOLDER_MIME, "parents": [parent] });
        let r = self
            .send(COST_EDIT, |c| c.post(format!("{}/files", *API)).query(&[("fields", FIELDS)]).json(&body))
            .await?;
        Ok(r.json().await?)
    }

    /// Rename and/or move: a metadata-only update, no content is copied.
    pub async fn rename_move(&self, id: &str, new_name: &str, add_parent: Option<&str>, remove_parent: Option<&str>) -> Result<DFile> {
        let url = format!("{}/files/{id}", *API);
        let body = serde_json::json!({ "name": new_name });
        let r = self
            .send(COST_EDIT, |c| {
                let mut q = vec![("fields", FIELDS.to_string())];
                if let Some(p) = add_parent {
                    q.push(("addParents", p.to_string()));
                }
                if let Some(p) = remove_parent {
                    q.push(("removeParents", p.to_string()));
                }
                c.patch(&url).query(&q).json(&body)
            })
            .await?;
        Ok(r.json().await?)
    }

    /// Delete goes to Drive's trash, recoverable for 30 days.
    pub async fn trash(&self, id: &str) -> Result<()> {
        let url = format!("{}/files/{id}", *API);
        let body = serde_json::json!({ "trashed": true });
        self.send(COST_EDIT, |c| c.patch(&url).json(&body)).await?;
        Ok(())
    }

    /// Upload a file's content from disk with a resumable session, in 8 MiB
    /// pieces. Creates a file when `existing` is None, otherwise replaces that
    /// file's content (Drive keeps the old revision).
    pub async fn upload_file(&self, existing: Option<&str>, name: &str, parent: &str, path: &Path, progress: &AtomicU64) -> Result<DFile> {
        const CHUNK: u64 = 8 << 20; // must be a multiple of 256 KiB
        let total = tokio::fs::metadata(path).await?.len();
        let (method, url, meta) = match existing {
            Some(id) => (Method::PATCH, format!("{}/files/{id}", *UPLOAD), serde_json::json!({})),
            None => (Method::POST, format!("{}/files", *UPLOAD), serde_json::json!({ "name": name, "parents": [parent] })),
        };
        let r = self
            .send(COST_EDIT, |c| {
                c.request(method.clone(), &url)
                    .query(&[("uploadType", "resumable"), ("fields", FIELDS)])
                    .header("X-Upload-Content-Length", total)
                    .json(&meta)
            })
            .await?;
        let session = r
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| anyhow!("no resumable session URL"))?
            .to_string();

        let mut file = tokio::fs::File::open(path).await?;
        let mut off = 0u64;
        let mut failures = 0u32;
        loop {
            let len = CHUNK.min(total - off);
            let mut buf = vec![0u8; len as usize];
            file.seek(std::io::SeekFrom::Start(off)).await?;
            file.read_exact(&mut buf).await?;
            let range = if total == 0 { "bytes */0".to_string() } else { format!("bytes {}-{}/{}", off, off + len - 1, total) };
            let tok = self.auth.token().await?;
            self.stats.requests.fetch_add(1, Relaxed);
            let sent = self
                .http
                .put(&session)
                .bearer_auth(&tok)
                .header("Content-Range", range)
                .header("Content-Length", len)
                .body(buf)
                .send()
                .await;
            let resp = match sent {
                Ok(r) if r.status().is_success() => {
                    self.stats.bytes_up.fetch_add(len, Relaxed);
                    progress.store(total, Relaxed);
                    return Ok(r.json().await?);
                }
                Ok(r) if r.status().as_u16() == 308 => {
                    self.stats.bytes_up.fetch_add(len, Relaxed);
                    progress.store(off + len, Relaxed);
                    r
                }
                Ok(r) if !(r.status().is_server_error() || r.status() == StatusCode::TOO_MANY_REQUESTS) => {
                    bail!("upload failed: {} {}", r.status(), r.text().await.unwrap_or_default());
                }
                other => {
                    failures += 1;
                    if failures > 6 {
                        bail!("upload kept failing: {:?}", other.map(|r| r.status()));
                    }
                    self.stats.retries.fetch_add(1, Relaxed);
                    tokio::time::sleep(Duration::from_millis(1000u64 << failures.min(5))).await;
                    // Ask the session how much it has, then carry on from there.
                    let st = self
                        .http
                        .put(&session)
                        .bearer_auth(&tok)
                        .header("Content-Range", format!("bytes */{total}"))
                        .header("Content-Length", 0)
                        .send()
                        .await?;
                    if st.status().is_success() {
                        return Ok(st.json().await?);
                    }
                    st
                }
            };
            // 308 Resume Incomplete: "Range: bytes=0-N" is what the server holds.
            off = resp
                .headers()
                .get("range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('-').next())
                .and_then(|n| n.parse::<u64>().ok())
                .map(|n| n + 1)
                .unwrap_or(0);
        }
    }
}
