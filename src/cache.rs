//! Block cache: files are read in fixed-size blocks, each fetched from Drive
//! with one Range request. Blocks live in a small memory LRU in front of a
//! disk LRU with a byte budget. Concurrent reads of one block share a single
//! download, and sequential readers get the next blocks fetched ahead.

use crate::drive::Drive;
use crate::meta::Node;
use anyhow::{Result, anyhow, bail};
use bytes::Bytes;
use serde::Serialize;
use md5::Md5;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::sync::{Notify, Semaphore, watch};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct BlockKey {
    file: String,
    version: String,
    idx: u64,
}

/// Cache directory name for one version of one file.
fn file_dir(file: &str, version: &str) -> String {
    let h = Sha256::digest(format!("{file}\0{version}").as_bytes());
    h.iter().take(10).map(|b| format!("{b:02x}")).collect()
}

impl BlockKey {
    fn rel_path(&self) -> PathBuf {
        PathBuf::from(file_dir(&self.file, &self.version)).join(self.idx.to_string())
    }
}

/// Byte-budgeted LRU keyed by K; values are whatever the caller tracks.
struct Lru<K: Clone + Eq + std::hash::Hash, V> {
    map: HashMap<K, (V, u64, u64)>, // value, size, tick
    order: BTreeMap<u64, K>,
    tick: u64,
    bytes: u64,
    budget: u64,
}

impl<K: Clone + Eq + std::hash::Hash, V: Clone> Lru<K, V> {
    fn new(budget: u64) -> Self {
        Lru { map: HashMap::new(), order: BTreeMap::new(), tick: 0, bytes: 0, budget }
    }
    fn get(&mut self, k: &K) -> Option<V> {
        let (v, _, t) = self.map.get_mut(k)?;
        self.order.remove(t);
        self.tick += 1;
        *t = self.tick;
        self.order.insert(self.tick, k.clone());
        Some(v.clone())
    }
    /// Insert and return whatever had to be evicted to stay in budget.
    fn put(&mut self, k: K, v: V, size: u64) -> Vec<(K, V, u64)> {
        if let Some((_, s, t)) = self.map.remove(&k) {
            self.order.remove(&t);
            self.bytes -= s;
        }
        self.tick += 1;
        self.map.insert(k.clone(), (v, size, self.tick));
        self.order.insert(self.tick, k);
        self.bytes += size;
        let mut evicted = Vec::new();
        while self.bytes > self.budget {
            let Some((_, oldest)) = self.order.pop_first() else { break };
            if let Some((v, s, _)) = self.map.remove(&oldest) {
                self.bytes -= s;
                evicted.push((oldest, v, s));
            }
        }
        evicted
    }
    fn remove(&mut self, k: &K) -> Option<(V, u64)> {
        let (v, s, t) = self.map.remove(k)?;
        self.order.remove(&t);
        self.bytes -= s;
        Some((v, s))
    }
}

/// The disk LRU plus a running total of cached bytes per file version, which
/// is what we report as the file's space "on disk".
struct DiskIndex {
    lru: Lru<PathBuf, String>,
    per_file: HashMap<String, u64>,
}

impl DiskIndex {
    fn new(budget: u64) -> Self {
        DiskIndex { lru: Lru::new(budget), per_file: HashMap::new() }
    }
    fn sub(&mut self, dir: &str, size: u64) {
        if let Some(v) = self.per_file.get_mut(dir) {
            *v = v.saturating_sub(size);
            if *v == 0 {
                self.per_file.remove(dir);
            }
        }
    }
    /// Insert and return the paths evicted to stay within budget.
    fn put(&mut self, rel: PathBuf, size: u64) -> Vec<PathBuf> {
        let dir = rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
        self.remove(&rel);
        *self.per_file.entry(dir.clone()).or_default() += size;
        let evicted = self.lru.put(rel, dir, size);
        evicted
            .into_iter()
            .map(|(k, d, s)| {
                self.sub(&d, s);
                k
            })
            .collect()
    }
    fn remove(&mut self, rel: &PathBuf) {
        if let Some((d, s)) = self.lru.remove(rel) {
            self.sub(&d, s);
        }
    }
}

#[derive(Default)]
struct ReadState {
    next: u64,
    streak: u32,
    prefetched_to: u64,
}

#[derive(Default, Serialize)]
pub struct CacheStats {
    pub mem_hits: AtomicU64,
    pub disk_hits: AtomicU64,
    pub fetches: AtomicU64,
    pub prefetches: AtomicU64,
    pub foreground_waits: AtomicU64,
    pub foreground_wait_ms: AtomicU64,
    pub bytes_served: AtomicU64,
}

/// A block on its way down. Bytes land in `data` as they arrive and
/// `progress` says how far it has got, so a read waits only for its own bytes.
struct Inflight {
    data: Mutex<Vec<u8>>,
    progress: watch::Sender<Progress>,
    started: AtomicBool,
    urgent: Notify,
}

#[derive(Clone, Default)]
struct Progress {
    filled: usize,
    done: bool,
    err: Option<Arc<str>>,
}

pub struct BlockStore {
    drive: Arc<Drive>,
    dir: PathBuf,
    pub block_size: u64,
    readahead_blocks: u64,
    disk: Mutex<DiskIndex>,
    mem: Mutex<Lru<BlockKey, Bytes>>,
    inflight: Mutex<HashMap<BlockKey, Arc<Inflight>>>,
    downloads: Arc<Semaphore>,
    fg_downloads: Arc<Semaphore>,
    readers: Mutex<HashMap<u64, ReadState>>,
    pub stats: CacheStats,
}

impl BlockStore {
    pub fn open(drive: Arc<Drive>, dir: &Path, disk_budget: u64, block_size: u64, readahead: u64, parallel: usize) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        // Rebuild the disk index, oldest-modified first so LRU order survives restarts.
        let mut found = Vec::new();
        for d in std::fs::read_dir(dir)?.flatten() {
            if !d.file_type()?.is_dir() {
                continue;
            }
            for f in std::fs::read_dir(d.path())?.flatten() {
                let md = f.metadata()?;
                let name = f.file_name().to_string_lossy().to_string();
                if name.ends_with(".tmp") {
                    let _ = std::fs::remove_file(f.path());
                    continue;
                }
                let rel = PathBuf::from(d.file_name()).join(&name);
                found.push((md.modified()?, rel, md.len()));
            }
        }
        found.sort();
        let mut disk = DiskIndex::new(disk_budget);
        for (_, rel, len) in found {
            for old in disk.put(rel, len) {
                let _ = std::fs::remove_file(dir.join(old));
            }
        }
        tracing::info!("disk cache: {:.1} GB in {} blocks (budget {:.1} GB)", disk.lru.bytes as f64 / 1e9, disk.lru.map.len(), disk_budget as f64 / 1e9);
        Ok(BlockStore {
            drive,
            dir: dir.to_path_buf(),
            block_size,
            readahead_blocks: (readahead / block_size).max(1),
            disk: Mutex::new(disk),
            mem: Mutex::new(Lru::new(256 << 20)),
            inflight: Mutex::new(HashMap::new()),
            downloads: Arc::new(Semaphore::new(parallel)),
            fg_downloads: Arc::new(Semaphore::new(parallel)),
            readers: Mutex::new(HashMap::new()),
            stats: CacheStats::default(),
        })
    }

    pub fn disk_usage(&self) -> (u64, usize) {
        let d = self.disk.lock().unwrap();
        (d.lru.bytes, d.lru.map.len())
    }

    pub fn budget(&self) -> u64 {
        self.disk.lock().unwrap().lru.budget
    }

    /// Drop everything cached (downloads in flight finish normally).
    pub async fn clear(&self) -> u64 {
        let paths: Vec<PathBuf> = {
            let mut d = self.disk.lock().unwrap();
            let paths = d.lru.map.keys().cloned().collect();
            let budget = d.lru.budget;
            *d = DiskIndex::new(budget);
            paths
        };
        *self.mem.lock().unwrap() = Lru::new(256 << 20);
        let mut freed = 0;
        for rel in paths {
            let p = self.dir.join(rel);
            freed += tokio::fs::metadata(&p).await.map(|m| m.len()).unwrap_or(0);
            let _ = tokio::fs::remove_file(p).await;
        }
        freed
    }

    /// Bytes of this file version held in the disk cache.
    pub fn cached_bytes(&self, file: &str, version: &str) -> u64 {
        self.disk.lock().unwrap().per_file.get(&file_dir(file, version)).copied().unwrap_or(0)
    }

    fn key(node: &Node, idx: u64) -> BlockKey {
        BlockKey { file: node.id.clone(), version: node.version.clone(), idx }
    }

    fn is_cached(&self, key: &BlockKey) -> bool {
        self.mem.lock().unwrap().map.contains_key(key) || self.disk.lock().unwrap().lru.map.contains_key(&key.rel_path())
    }

    async fn disk_get(&self, key: &BlockKey) -> Option<Bytes> {
        let rel = key.rel_path();
        if self.disk.lock().unwrap().lru.get(&rel).is_none() {
            return None;
        }
        match tokio::fs::read(self.dir.join(&rel)).await {
            Ok(b) => Some(Bytes::from(b)),
            Err(_) => {
                self.disk.lock().unwrap().remove(&rel);
                None
            }
        }
    }

    async fn disk_put(&self, key: &BlockKey, data: &Bytes) {
        let rel = key.rel_path();
        let path = self.dir.join(&rel);
        let tmp = path.with_extension("tmp");
        let res: std::io::Result<()> = async {
            tokio::fs::create_dir_all(path.parent().unwrap()).await?;
            tokio::fs::write(&tmp, data).await?;
            tokio::fs::rename(&tmp, &path).await
        }
        .await;
        if let Err(e) = res {
            tracing::warn!("cache write failed: {e}");
            return;
        }
        let evicted = self.disk.lock().unwrap().put(rel, data.len() as u64);
        for old in evicted {
            let _ = tokio::fs::remove_file(self.dir.join(old)).await;
        }
    }

    /// Join the download of a block, starting one if nobody has.
    fn ensure(self: &Arc<Self>, node: &Node, idx: u64, foreground: bool) -> Arc<Inflight> {
        let key = Self::key(node, idx);
        let mut map = self.inflight.lock().unwrap();
        if let Some(inf) = map.get(&key) {
            if foreground && !inf.started.load(Relaxed) {
                // Read-ahead still queued, and now a reader needs it: jump the queue.
                inf.urgent.notify_one();
            }
            return inf.clone();
        }
        let start = idx * self.block_size;
        let len = ((start + self.block_size).min(node.size) - start) as usize;
        let inf = Arc::new(Inflight {
            data: Mutex::new(Vec::with_capacity(len)),
            progress: watch::channel(Progress::default()).0,
            started: AtomicBool::new(false),
            urgent: Notify::new(),
        });
        map.insert(key.clone(), inf.clone());
        drop(map);
        let (me, node, inf2) = (self.clone(), node.clone(), inf.clone());
        tokio::spawn(async move { me.download(node, idx, key, inf2, foreground).await });
        inf
    }

    async fn download(self: Arc<Self>, node: Node, idx: u64, key: BlockKey, inf: Arc<Inflight>, foreground: bool) {
        // Foreground reads get their own slots so a seek never queues behind
        // read-ahead for a part of the file nobody wants any more.
        let _permit = if foreground {
            self.fg_downloads.clone().acquire_owned().await
        } else {
            tokio::select! {
                p = self.downloads.clone().acquire_owned() => p,
                _ = inf.urgent.notified() => self.fg_downloads.clone().acquire_owned().await,
            }
        }
        .expect("semaphore");
        inf.started.store(true, Relaxed);
        self.stats.fetches.fetch_add(1, Relaxed);
        if !foreground {
            self.stats.prefetches.fetch_add(1, Relaxed);
        }
        let start = idx * self.block_size;
        let end = (start + self.block_size).min(node.size);
        let want = (end - start) as usize;
        let t = Instant::now();
        let result: Result<()> = async {
            let mut resp = self.drive.open_range(&node.id, start, end).await?;
            self.drive.note_first_byte(t.elapsed());
            // If the server ignored Range and sent the whole file, skip ahead.
            let mut skip = if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT { 0 } else { start as usize };
            while let Some(chunk) = resp.chunk().await? {
                let mut c = &chunk[..];
                if skip > 0 {
                    let n = skip.min(c.len());
                    c = &c[n..];
                    skip -= n;
                }
                let filled = {
                    let mut d = inf.data.lock().unwrap();
                    let room = want - d.len();
                    d.extend_from_slice(&c[..c.len().min(room)]);
                    d.len()
                };
                inf.progress.send_modify(|p| p.filled = filled);
                if filled >= want {
                    break;
                }
            }
            let got = inf.data.lock().unwrap().len();
            if got != want {
                bail!("short read: wanted {want} bytes, got {got} (file changed?)");
            }
            Ok(())
        }
        .await;
        self.drive.note_download(inf.data.lock().unwrap().len() as u64, t.elapsed());
        match result {
            Ok(()) => {
                let b = Bytes::copy_from_slice(&inf.data.lock().unwrap());
                self.mem.lock().unwrap().put(key.clone(), b.clone(), b.len() as u64);
                self.inflight.lock().unwrap().remove(&key);
                inf.progress.send_modify(|p| p.done = true);
                self.disk_put(&key, &b).await;
            }
            Err(e) => {
                tracing::warn!("download {} block {idx} failed: {e:#}", node.name);
                self.inflight.lock().unwrap().remove(&key);
                inf.progress.send_modify(|p| p.err = Some(format!("{e:#}").into()));
            }
        }
    }

    async fn wait_slice(inf: &Inflight, s: usize, e: usize) -> Result<Bytes> {
        let mut rx = inf.progress.subscribe();
        loop {
            let p = rx.borrow_and_update().clone();
            if let Some(err) = p.err {
                bail!("{err}");
            }
            if p.filled >= e || p.done {
                let d = inf.data.lock().unwrap();
                let e = e.min(d.len());
                return Ok(Bytes::copy_from_slice(&d[s.min(e)..e]));
            }
            rx.changed().await.map_err(|_| anyhow!("download abandoned"))?;
        }
    }

    /// Bytes [s, e) of block `idx`, from memory, disk, or the network.
    async fn slice(self: &Arc<Self>, node: &Node, idx: u64, s: usize, e: usize) -> Result<Bytes> {
        let key = Self::key(node, idx);
        let hit = self.mem.lock().unwrap().get(&key);
        if let Some(b) = hit {
            self.stats.mem_hits.fetch_add(1, Relaxed);
            return Ok(b.slice(s.min(b.len())..e.min(b.len())));
        }
        if let Some(b) = self.disk_get(&key).await {
            self.stats.disk_hits.fetch_add(1, Relaxed);
            self.mem.lock().unwrap().put(key, b.clone(), b.len() as u64);
            return Ok(b.slice(s.min(b.len())..e.min(b.len())));
        }
        let t = Instant::now();
        let inf = self.ensure(node, idx, true);
        let r = Self::wait_slice(&inf, s, e).await;
        self.stats.foreground_waits.fetch_add(1, Relaxed);
        self.stats.foreground_wait_ms.fetch_add(t.elapsed().as_millis() as u64, Relaxed);
        r
    }

    /// Check a just-uploaded file against Drive's MD5 of the upload, and put
    /// it in the cache under its new version if it matches and fits (one
    /// block at a time: uploads can be far bigger than memory). Returns
    /// whether it matched; None if the file couldn't be read.
    pub async fn seed(&self, file_id: &str, version: &str, path: &Path, drive_md5: &str) -> Option<bool> {
        let size = tokio::fs::metadata(path).await.ok()?.len();
        let cache_it = size <= self.disk.lock().unwrap().lru.budget / 4;
        let mut f = tokio::fs::File::open(path).await.ok()?;
        let mut md5 = Md5::new();
        let mut cached = Vec::new();
        for idx in 0.. {
            let mut buf = vec![0u8; self.block_size as usize];
            let mut n = 0;
            while n < buf.len() {
                match f.read(&mut buf[n..]).await {
                    Ok(0) => break,
                    Ok(k) => n += k,
                    Err(_) => return None,
                }
            }
            if n == 0 {
                break;
            }
            buf.truncate(n);
            md5.update(&buf);
            if cache_it {
                let key = BlockKey { file: file_id.to_string(), version: version.to_string(), idx };
                self.disk_put(&key, &Bytes::from(buf)).await;
                cached.push(key);
            }
            if (n as u64) < self.block_size {
                break;
            }
        }
        let ours: String = md5.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let matches = ours == drive_md5;
        if !matches {
            for key in cached {
                let rel = key.rel_path();
                self.disk.lock().unwrap().remove(&rel);
                let _ = tokio::fs::remove_file(self.dir.join(rel)).await;
            }
        }
        Some(matches)
    }

    /// Track each file's access pattern; returns blocks worth fetching ahead.
    fn plan_readahead(&self, node: &Node, offset: u64, end: u64) -> std::ops::Range<u64> {
        let bs = self.block_size;
        let nblocks = node.size.div_ceil(bs);
        let mut readers = self.readers.lock().unwrap();
        let st = readers.entry(node.ino).or_default();
        // The NFS client keeps several reads in flight, so "sequential" means
        // near where the last reads ended, not exactly at it.
        let sequential = offset.abs_diff(st.next) <= 2 * bs;
        if sequential {
            st.streak += 1;
            st.next = st.next.max(end);
        } else {
            st.streak = 0;
            st.next = end;
            st.prefetched_to = 0;
        }
        if st.streak < 2 {
            return 0..0;
        }
        let from = (end / bs + 1).max(st.prefetched_to);
        let to = (end / bs + 1 + self.readahead_blocks).min(nblocks);
        if from >= to {
            return 0..0;
        }
        st.prefetched_to = to;
        from..to
    }

    pub async fn read(self: &Arc<Self>, node: &Node, offset: u64, count: u32) -> Result<(Vec<u8>, bool)> {
        if offset >= node.size {
            return Ok((Vec::new(), true));
        }
        let end = (offset + count as u64).min(node.size);
        let bs = self.block_size;
        for idx in self.plan_readahead(node, offset, end) {
            if !self.is_cached(&Self::key(node, idx)) {
                self.ensure(node, idx, false);
            }
        }
        let mut out = Vec::with_capacity((end - offset) as usize);
        for idx in offset / bs..=(end - 1) / bs {
            let bstart = idx * bs;
            let s = (offset.max(bstart) - bstart) as usize;
            let e = (end.min(bstart + bs) - bstart) as usize;
            out.extend_from_slice(&self.slice(node, idx, s, e).await?);
        }
        self.stats.bytes_served.fetch_add(out.len() as u64, Relaxed);
        Ok((out, end >= node.size))
    }
}
