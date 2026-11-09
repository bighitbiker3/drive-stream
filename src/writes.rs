//! Write-back: writes land in a local staging file at once, and a file is
//! uploaded once it has sat unchanged long enough, so short-lived files
//! (editor temp files, build output) never cost an upload.

use crate::cache::BlockStore;
use crate::drive::Drive;
use crate::meta::{self, Meta};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

/// Finder creates every destination file of a copy up front and fills them
/// one by one, so a new file that is still empty is usually just waiting its
/// turn. Upload it only if it stays empty this long.
const EMPTY_NEW_FILE_SETTLE: Duration = Duration::from_secs(600);
/// A file whose last write made it longer is probably still being copied in
/// (the source can stall, e.g. a camera card); give it longer to settle.
const GROWING_FILE_SETTLE: Duration = Duration::from_secs(10);
const PARALLEL_UPLOADS: usize = 4;
/// Prefix for staging copies of Finder's local-only files, so a restart can
/// discard them instead of mistaking them for unsent work.
const LOCAL_ONLY_PREFIX: &str = "lo-";

pub struct Dirty {
    pub path: PathBuf,
    pub last_write: Instant,
    /// Bumped on every change; an upload only retires the generation it read.
    pub generation: u64,
    pub uploading: bool,
    /// Finder's .DS_Store and ._* files: kept on this Mac, never uploaded.
    pub local_only: bool,
    /// The file already exists on Drive (this is an edit, not a new file).
    pub on_drive: bool,
    /// The last write extended the file.
    pub growing: bool,
    /// Uploads whose checksum didn't match what we sent; we stop after a few.
    pub mismatches: u32,
    /// Failed upload attempts in a row; retries back off exponentially.
    pub failures: u32,
    /// Bytes Drive has acknowledged for the upload in progress.
    pub sent: Arc<AtomicU64>,
    /// When the current upload got a slot and started sending.
    pub upload_started: Option<Instant>,
    pub last_error: Option<String>,
}

/// One finished (or failed) upload, for the UI's history.
#[derive(Clone, serde::Serialize)]
pub struct UploadRecord {
    pub path: String,
    pub bytes: u64,
    pub seconds: f64,
    pub finished_at: i64,
    pub new_revision: bool,
    /// Drive's MD5 matched ours (None: not checked).
    pub verified: Option<bool>,
    pub error: Option<String>,
    /// Consecutive failed attempts this record stands for.
    pub attempts: u32,
}

const HISTORY: usize = 200;

#[derive(Default)]
pub struct WriteStats {
    pub uploads: AtomicU64,
    pub upload_failures: AtomicU64,
    pub checksum_mismatches: AtomicU64,
}

pub struct Writes {
    pub history: Mutex<std::collections::VecDeque<UploadRecord>>,
    dir: PathBuf,
    pub files: Mutex<HashMap<u64, Dirty>>,
    pub settle: Duration,
    pub stats: WriteStats,
    slots: Arc<Semaphore>,
}

pub fn is_local_only(name: &str) -> bool {
    name == ".DS_Store" || name.starts_with("._")
}

impl Writes {
    pub fn open(dir: &Path, settle: Duration) -> Result<Self> {
        if dir.exists() {
            for e in std::fs::read_dir(dir)?.flatten() {
                if e.file_name().to_string_lossy().starts_with(LOCAL_ONLY_PREFIX) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
            // Anything else left by a crash was never uploaded: keep it aside, don't serve it.
            if std::fs::read_dir(dir)?.next().is_some() {
                let aside = dir.with_file_name(format!("staging-orphaned-{}", chrono::Utc::now().format("%Y%m%dT%H%M%S")));
                tracing::warn!("unuploaded staging files from a previous run moved to {}", aside.display());
                std::fs::rename(dir, &aside)?;
            }
        }
        std::fs::create_dir_all(dir)?;
        Ok(Writes {
            history: Mutex::new(std::collections::VecDeque::new()),
            dir: dir.to_path_buf(),
            files: Mutex::new(HashMap::new()),
            settle,
            stats: WriteStats::default(),
            slots: Arc::new(Semaphore::new(PARALLEL_UPLOADS)),
        })
    }

    fn staging_path(&self, ino: u64, local_only: bool) -> PathBuf {
        self.dir.join(if local_only { format!("{LOCAL_ONLY_PREFIX}{ino}") } else { ino.to_string() })
    }

    pub fn path_for(&self, ino: u64) -> Option<PathBuf> {
        self.files.lock().unwrap().get(&ino).map(|d| d.path.clone())
    }

    /// Start tracking `ino` with an (empty) staging file.
    pub fn begin(&self, ino: u64, local_only: bool, on_drive: bool) -> std::io::Result<PathBuf> {
        let path = self.staging_path(ino, local_only);
        std::fs::File::create(&path)?;
        self.files.lock().unwrap().insert(
            ino,
            Dirty {
                path: path.clone(),
                last_write: Instant::now(),
                generation: 1,
                uploading: false,
                local_only,
                on_drive,
                growing: false,
                mismatches: 0,
                failures: 0,
                sent: Arc::default(),
                upload_started: None,
                last_error: None,
            },
        );
        Ok(path)
    }

    pub fn touch(&self, ino: u64, growing: bool) {
        if let Some(d) = self.files.lock().unwrap().get_mut(&ino) {
            d.last_write = Instant::now();
            d.generation += 1;
            d.growing = growing;
        }
    }

    /// A rename can turn a local-only name into a real one or back.
    pub fn set_local_only(&self, ino: u64, local_only: bool) -> std::io::Result<()> {
        let mut files = self.files.lock().unwrap();
        let Some(d) = files.get_mut(&ino) else { return Ok(()) };
        if d.local_only != local_only {
            let path = self.staging_path(ino, local_only);
            std::fs::rename(&d.path, &path)?;
            d.path = path;
            d.local_only = local_only;
            d.last_write = Instant::now();
        }
        Ok(())
    }

    pub fn forget(&self, ino: u64) {
        if let Some(d) = self.files.lock().unwrap().remove(&ino) {
            let _ = std::fs::remove_file(d.path);
        }
    }

    /// Hand `from`'s staged content to `to` (an editor's save-by-rename).
    pub fn adopt(&self, from: u64, to: u64) -> std::io::Result<()> {
        let mut files = self.files.lock().unwrap();
        let Some(src) = files.remove(&from) else { return Ok(()) };
        let path = self.staging_path(to, false);
        std::fs::rename(&src.path, &path)?;
        let generation = files.get(&to).map(|d| d.generation + 1).unwrap_or(1);
        files.insert(
            to,
            Dirty {
                path,
                last_write: Instant::now(),
                generation,
                uploading: false,
                local_only: false,
                on_drive: true,
                growing: false,
                mismatches: 0,
                failures: 0,
                sent: Arc::default(),
                upload_started: None,
                last_error: None,
            },
        );
        Ok(())
    }

    pub fn pending(&self) -> usize {
        self.files.lock().unwrap().values().filter(|d| !d.local_only).count()
    }

    fn settle_for(&self, d: &Dirty) -> Duration {
        let empty = std::fs::metadata(&d.path).map(|m| m.len() == 0).unwrap_or(false);
        if d.failures > 0 {
            // Back off: 2s, 4s, 8s … up to 5 minutes between attempts.
            Duration::from_secs((1u64 << d.failures.min(8)).min(300)).max(self.settle)
        } else if empty && !d.on_drive {
            EMPTY_NEW_FILE_SETTLE
        } else if d.growing {
            GROWING_FILE_SETTLE.max(self.settle)
        } else {
            self.settle
        }
    }

    /// Start uploads for everything that has settled (or everything, when
    /// `all`), at most PARALLEL_UPLOADS at a time. Returns without waiting.
    pub fn upload_ready(self: &Arc<Self>, meta: &Arc<Meta>, drive: &Arc<Drive>, blocks: &Arc<BlockStore>, all: bool) -> Vec<JoinHandle<()>> {
        let ready: Vec<(u64, u64, PathBuf)> = {
            let mut files = self.files.lock().unwrap();
            let ready: Vec<u64> = files
                .iter()
                .filter(|(_, d)| {
                    !d.local_only && !d.uploading && d.mismatches < 3 && (all || d.last_write.elapsed() >= self.settle_for(d))
                })
                .map(|(ino, _)| *ino)
                .collect();
            ready
                .into_iter()
                .map(|ino| {
                    let d = files.get_mut(&ino).unwrap();
                    d.uploading = true;
                    (ino, d.generation, d.path.clone())
                })
                .collect()
        };
        ready
            .into_iter()
            .map(|(ino, generation, path)| {
                let (me, meta, drive, blocks) = (self.clone(), meta.clone(), drive.clone(), blocks.clone());
                tokio::spawn(async move {
                    let _slot = me.slots.clone().acquire_owned().await.expect("semaphore");
                    if let Some(d) = me.files.lock().unwrap().get_mut(&ino) {
                        d.upload_started = Some(Instant::now());
                        d.sent.store(0, Relaxed);
                    }
                    me.upload_one(ino, generation, &path, &meta, &drive, &blocks).await
                })
            })
            .collect()
    }

    /// Upload everything with unsent changes, waiting up to `limit`.
    /// Up to three attempts per file, within `limit`; whatever still fails
    /// is left in staging (and set aside on the next start).
    pub async fn flush(self: &Arc<Self>, meta: &Arc<Meta>, drive: &Arc<Drive>, blocks: &Arc<BlockStore>, limit: Duration) {
        let start = Instant::now();
        for round in 0..3 {
            if self.pending() == 0 || start.elapsed() >= limit {
                break;
            }
            if round > 0 {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            for h in self.upload_ready(meta, drive, blocks, true) {
                let _ = tokio::time::timeout(limit.saturating_sub(start.elapsed()), h).await;
            }
        }
    }

    async fn upload_one(&self, ino: u64, generation: u64, path: &Path, meta: &Meta, drive: &Drive, blocks: &BlockStore) {
        let (node, parent_id) = {
            let tree = meta.tree();
            let node = tree.node(ino).cloned();
            let parent_id = node.as_ref().and_then(|n| tree.node(n.parent)).map(|p| p.id.clone()).unwrap_or_default();
            (node, parent_id)
        };
        let Some(node) = node else {
            self.forget(ino); // deleted before it was uploaded
            return;
        };
        let existing = (!meta::is_local(&node.id)).then_some(node.id.as_str());
        let t = Instant::now();
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let display_path = meta.path_of(ino);
        let sent = self.files.lock().unwrap().get(&ino).map(|d| d.sent.clone()).unwrap_or_default();
        let result = drive.upload_file(existing, &meta::drive_name(&node.name), &parent_id, path, &sent).await;
        let mut record = UploadRecord {
            path: display_path,
            bytes: size,
            seconds: t.elapsed().as_secs_f64(),
            finished_at: chrono::Utc::now().timestamp(),
            new_revision: existing.is_some(),
            verified: None,
            error: None,
            attempts: 1,
        };
        match result {
            Ok(f) => {
                self.stats.uploads.fetch_add(1, Relaxed);
                let unchanged = || self.files.lock().unwrap().get(&ino).is_some_and(|d| d.generation == generation);
                // Cache what we uploaded (a read would download the same bytes),
                // but only if it still matches Drive's checksum of the upload.
                let verified = match (&f.md5_checksum, unchanged()) {
                    (Some(md5), true) => blocks.seed(&f.id, &f.content_version(), path, md5).await,
                    _ => None,
                };
                if verified == Some(false) {
                    self.stats.checksum_mismatches.fetch_add(1, Relaxed);
                }
                record.verified = verified;
                record.seconds = t.elapsed().as_secs_f64();
                self.remember(record.clone());
                tracing::info!(
                    "uploaded {} ({:.1} MB in {:.1}s){}{}",
                    node.name,
                    size as f64 / 1e6,
                    t.elapsed().as_secs_f64(),
                    if existing.is_some() { " as a new revision" } else { "" },
                    match verified {
                        Some(true) => ", checksum matches Drive",
                        Some(false) => ", CHECKSUM MISMATCH with Drive (file changed during upload?)",
                        None => "",
                    }
                );
                let res = if existing.is_some() { meta.upsert_local(f).await } else { meta.resolve_placeholder(&node.id, f).await };
                if let Err(e) = res {
                    tracing::warn!("recording upload of {}: {e:#}", node.name);
                }
                let mut files = self.files.lock().unwrap();
                if let Some(d) = files.get_mut(&ino) {
                    d.upload_started = None;
                    d.last_error = None;
                }
                if files.get(&ino).is_some_and(|d| d.generation == generation) && verified != Some(false) {
                    let d = files.remove(&ino).unwrap();
                    let _ = std::fs::remove_file(d.path);
                } else if let Some(d) = files.get_mut(&ino) {
                    // Changed mid-upload (or didn't verify): goes again after it settles.
                    d.uploading = false;
                    d.on_drive = true;
                    d.failures = 0;
                    d.last_write = Instant::now();
                    if verified == Some(false) {
                        d.mismatches += 1;
                        if d.mismatches >= 3 {
                            tracing::error!("{}: Drive's checksum keeps disagreeing with what was sent; keeping the local copy at {} and not retrying", node.name, d.path.display());
                        }
                    }
                }
            }
            Err(e) => {
                self.stats.upload_failures.fetch_add(1, Relaxed);
                if !path.exists() {
                    // Nothing left to send; retrying can never succeed.
                    tracing::error!("{}: its local copy disappeared before it finished uploading", node.name);
                    record.error = Some("its local copy disappeared before it finished uploading; not retrying".into());
                    self.remember(record);
                    self.files.lock().unwrap().remove(&ino);
                    return;
                }
                tracing::warn!("upload of {} failed, will retry: {e:#}", node.name);
                record.error = Some(format!("{e:#}").chars().take(300).collect());
                self.remember(record);
                if let Some(d) = self.files.lock().unwrap().get_mut(&ino) {
                    d.uploading = false;
                    d.upload_started = None;
                    d.failures += 1;
                    d.last_error = Some(format!("{e:#}").chars().take(300).collect());
                    d.last_write = Instant::now();
                }
            }
        }
    }

    fn remember(&self, mut r: UploadRecord) {
        let mut h = self.history.lock().unwrap();
        // Repeated failures of one file are one entry with a count.
        if r.error.is_some() && h.front().is_some_and(|f| f.error.is_some() && f.path == r.path) {
            r.attempts += h.pop_front().unwrap().attempts;
        }
        h.push_front(r);
        h.truncate(HISTORY);
    }

    /// What's waiting or uploading right now, for the UI.
    pub fn snapshot(&self, meta: &Meta) -> Vec<serde_json::Value> {
        let files = self.files.lock().unwrap();
        let mut out: Vec<serde_json::Value> = files
            .iter()
            .filter(|(_, d)| !d.local_only)
            .map(|(ino, d)| {
                let size = std::fs::metadata(&d.path).map(|m| m.len()).unwrap_or(0);
                let sent = d.sent.load(Relaxed);
                let (state, detail) = if d.mismatches >= 3 {
                    ("stopped", "Drive's checksum kept disagreeing; not retrying".to_string())
                } else if let Some(t) = d.upload_started {
                    let secs = t.elapsed().as_secs_f64().max(0.1);
                    ("uploading", format!("{:.1} MB/s", sent as f64 / secs / 1e6))
                } else if d.uploading {
                    ("queued", "waiting for a free upload slot".to_string())
                } else {
                    let left = self.settle_for(d).saturating_sub(d.last_write.elapsed()).as_secs();
                    let detail = if d.failures > 0 {
                        format!("retrying in {left}s (failed {}×)", d.failures)
                    } else if size == 0 && !d.on_drive {
                        "waiting for its content".to_string()
                    } else {
                        format!("starts in {left}s")
                    };
                    ("waiting", detail)
                };
                serde_json::json!({
                    "path": meta.path_of(*ino),
                    "bytes": size,
                    "sent": sent.min(size),
                    "state": state,
                    "detail": detail,
                    "error": d.last_error,
                    "elapsed_s": d.upload_started.map(|t| t.elapsed().as_secs()),
                })
            })
            .collect();
        out.sort_by_key(|v| (v["state"] != "uploading", v["path"].as_str().unwrap_or("").to_string()));
        out
    }

    pub fn history(&self) -> Vec<UploadRecord> {
        self.history.lock().unwrap().iter().cloned().collect()
    }

    /// Clear back-offs and give-ups so everything pending goes now.
    pub fn retry_now(&self) {
        for d in self.files.lock().unwrap().values_mut() {
            d.mismatches = 0;
            d.last_write = Instant::now() - Duration::from_secs(3600);
        }
    }
}
