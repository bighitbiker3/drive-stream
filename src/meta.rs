//! The local picture of the drive: every Drive file's metadata in SQLite
//! (so a restart is warm and only replays the change feed), plus an
//! in-memory tree built once at startup and then patched one directory at a
//! time as changes arrive.

use crate::drive::{DFile, Drive};
use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{RwLock, RwLockReadGuard};

pub const ROOT_INO: u64 = 1;
const FIRST_INO: u64 = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Dir,
    File,
    /// A Google Docs/Sheets/... item: shown as a small JSON link file.
    Stub,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub ino: u64,
    pub id: String,
    pub parent: u64,
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    pub mtime: (i64, u32),
    pub version: String,
}

#[derive(Default)]
pub struct Tree {
    pub nodes: HashMap<u64, Node>,
    /// Children of each directory, sorted by inode (readdir cookies are inodes).
    pub children: HashMap<u64, Vec<u64>>,
    /// Per directory: display name -> inode.
    pub by_name: HashMap<u64, HashMap<String, u64>>,
    pub by_id: HashMap<String, u64>,
}

impl Tree {
    pub fn node(&self, ino: u64) -> Option<&Node> {
        self.nodes.get(&ino)
    }
    pub fn lookup(&self, dir: u64, name: &str) -> Option<u64> {
        self.by_name.get(&dir)?.get(name).copied()
    }
}

struct Inner {
    db: Connection,
    files: HashMap<String, DFile>,
    /// Drive parent id -> child ids, over every known file.
    by_parent: HashMap<String, Vec<String>>,
    inodes: HashMap<String, u64>,
    next_ino: u64,
    new_inode_rows: Vec<(String, u64)>,
    root_id: String,
    page_token: Option<String>,
    /// Per directory: hash of its listing and the mtime we report for it. The
    /// mtime moves whenever the listing changes, which is what makes NFS
    /// clients drop their cached copy of the directory.
    dir_state: HashMap<u64, (u64, (i64, u32))>,
}

pub struct Meta {
    inner: tokio::sync::Mutex<Inner>,
    tree: RwLock<Tree>,
}

pub struct SyncReport {
    pub changes: usize,
    pub files: usize,
}

fn parse_time(s: Option<&str>) -> (i64, u32) {
    s.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| (t.timestamp(), t.timestamp_subsec_nanos()))
        .unwrap_or((0, 0))
}

fn now() -> (i64, u32) {
    let t = chrono::Utc::now();
    (t.timestamp(), t.timestamp_subsec_nanos())
}

fn parent_of(f: &DFile) -> Option<&str> {
    f.parents.as_ref()?.first().map(|s| s.as_str())
}

pub fn stub_ext(mime: &str) -> Option<&'static str> {
    if !mime.starts_with("application/vnd.google-apps.") || mime == crate::drive::FOLDER_MIME {
        return None;
    }
    Some(match &mime["application/vnd.google-apps.".len()..] {
        "document" => ".gdoc",
        "spreadsheet" => ".gsheet",
        "presentation" => ".gslides",
        "drawing" => ".gdraw",
        "form" => ".gform",
        "map" => ".gmap",
        "site" => ".gsite",
        "script" => ".gscript",
        "jam" => ".gjam",
        _ => ".glink",
    })
}

/// Contents of a stub file; its size is computed from this too.
pub fn stub_body(id: &str) -> Vec<u8> {
    format!("{{\"url\": \"https://drive.google.com/open?id={id}\", \"doc_id\": \"{id}\"}}\n").into_bytes()
}

/// Files created through the mount that Drive hasn't assigned an id to yet.
pub fn is_local(id: &str) -> bool {
    id.starts_with("local-")
}

/// Inverse of `sanitize`, for names we send to Drive.
pub fn drive_name(posix: &str) -> String {
    posix.replace(':', "/")
}

/// Drive names may contain "/"; on macOS ":" in a POSIX name is shown as "/"
/// in Finder, so that's the faithful mapping.
fn sanitize(name: &str) -> String {
    let s = name.replace('/', ":");
    match s.as_str() {
        "" | "." | ".." => format!("_{s}"),
        _ => s,
    }
}

fn with_suffix(name: &str, n: usize) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
}

impl Inner {
    fn ino_for(&mut self, id: &str) -> u64 {
        if let Some(&i) = self.inodes.get(id) {
            return i;
        }
        let i = self.next_ino;
        self.next_ino += 1;
        self.inodes.insert(id.to_string(), i);
        if !is_local(id) {
            self.new_inode_rows.push((id.to_string(), i));
        }
        i
    }

    fn link(&mut self, f: &DFile) {
        if let Some(p) = parent_of(f) {
            self.by_parent.entry(p.to_string()).or_default().push(f.id.clone());
        }
    }

    fn unlink(&mut self, f: &DFile) {
        if let Some(p) = parent_of(f) {
            if let Some(v) = self.by_parent.get_mut(p) {
                v.retain(|c| c != &f.id);
                if v.is_empty() {
                    self.by_parent.remove(p);
                }
            }
        }
    }

    /// Recompute one directory's entries from `by_parent`: names (with
    /// duplicate numbering), attributes, and the directory's own mtime.
    /// Returns child directories that weren't in the tree yet.
    fn refresh_dir(&mut self, t: &mut Tree, pino: u64) -> Vec<u64> {
        let pid = t.nodes[&pino].id.clone();
        let child_ids = self.by_parent.get(&pid).cloned().unwrap_or_default();
        let mut kids: Vec<(u64, String)> = child_ids.into_iter().map(|id| (self.ino_for(&id), id)).collect();
        kids.sort_by_key(|k| k.0);

        let mut names: HashMap<String, u64> = HashMap::with_capacity(kids.len());
        let mut listing = Vec::with_capacity(kids.len());
        let mut new_dirs = Vec::new();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (ino, id) in kids {
            let f = &self.files[&id];
            let kind = if f.is_folder() {
                Kind::Dir
            } else if stub_ext(&f.mime_type).is_some() {
                Kind::Stub
            } else {
                Kind::File
            };
            let mut base = sanitize(&f.name);
            if let Some(ext) = stub_ext(&f.mime_type) {
                base.push_str(ext);
            }
            // Same convention as Google's Drive app: "name", "name (1)", …
            let mut name = base.clone();
            let mut n = 1;
            while names.contains_key(&name) {
                name = with_suffix(&base, n);
                n += 1;
            }
            let size = match kind {
                Kind::File => f.size(),
                Kind::Stub => stub_body(&f.id).len() as u64,
                Kind::Dir => 0,
            };
            (name.as_str(), ino, size, f.modified_time.as_deref()).hash(&mut hasher);
            let mtime = match kind {
                Kind::Dir => self.dir_state.get(&ino).map(|s| s.1).unwrap_or_else(|| parse_time(f.modified_time.as_deref())),
                _ => parse_time(f.modified_time.as_deref()),
            };
            match t.nodes.get(&ino) {
                Some(old) if old.id != id => {
                    t.by_id.remove(&old.id);
                }
                None if kind == Kind::Dir => new_dirs.push(ino),
                _ => {}
            }
            let node = Node { ino, id: id.clone(), parent: pino, name: name.clone(), kind, size, mtime, version: f.content_version() };
            t.nodes.insert(ino, node);
            t.by_id.insert(id, ino);
            names.insert(name, ino);
            listing.push(ino);
        }
        t.children.insert(pino, listing);
        t.by_name.insert(pino, names);

        let h = hasher.finish();
        let mtime = match self.dir_state.get(&pino) {
            Some((old_h, m)) if *old_h == h => *m,
            Some(_) => now(),
            None => t.nodes[&pino].mtime,
        };
        self.dir_state.insert(pino, (h, mtime));
        t.nodes.get_mut(&pino).unwrap().mtime = mtime;
        new_dirs
    }

    fn fill(&mut self, t: &mut Tree, start: Vec<u64>) {
        let mut queue = VecDeque::from(start);
        let mut seen = HashSet::new();
        while let Some(d) = queue.pop_front() {
            if seen.insert(d) {
                queue.extend(self.refresh_dir(t, d));
            }
        }
    }

    /// Drop a node and everything still filed under it.
    fn remove_subtree(&mut self, t: &mut Tree, ino: u64) {
        let mut stack = vec![(ino, None::<u64>)];
        while let Some((i, expected_parent)) = stack.pop() {
            // A child moved elsewhere in this same batch isn't ours to drop.
            if expected_parent.is_some_and(|p| t.nodes.get(&i).is_some_and(|n| n.parent != p)) {
                continue;
            }
            if let Some(kids) = t.children.remove(&i) {
                stack.extend(kids.into_iter().map(|k| (k, Some(i))));
            }
            t.by_name.remove(&i);
            self.dir_state.remove(&i);
            if let Some(n) = t.nodes.remove(&i) {
                if t.by_id.get(&n.id) == Some(&i) {
                    t.by_id.remove(&n.id);
                }
            }
        }
    }

    fn build(&mut self, t: &mut Tree) {
        self.by_parent.clear();
        let all: Vec<DFile> = self.files.values().cloned().collect();
        for f in &all {
            self.link(f);
        }
        *t = Tree::default();
        let root_mtime = self.dir_state.get(&ROOT_INO).map(|s| s.1).unwrap_or_else(now);
        t.nodes.insert(
            ROOT_INO,
            Node { ino: ROOT_INO, id: self.root_id.clone(), parent: ROOT_INO, name: String::new(), kind: Kind::Dir, size: 0, mtime: root_mtime, version: String::new() },
        );
        t.by_id.insert(self.root_id.clone(), ROOT_INO);
        self.fill(t, vec![ROOT_INO]);
    }

    /// Patch the tree for a batch of changes, touching only the directories
    /// they affect.
    fn apply(&mut self, t: &mut Tree, upserts: Vec<DFile>, removals: Vec<String>) {
        let mut dirty: HashSet<u64> = HashSet::new();
        let mark = |t: &Tree, f: &DFile, dirty: &mut HashSet<u64>| {
            if let Some(p) = parent_of(f).and_then(|p| t.by_id.get(p)) {
                dirty.insert(*p);
            }
        };
        for f in upserts {
            if let Some(old) = self.files.remove(&f.id) {
                mark(t, &old, &mut dirty);
                self.unlink(&old);
            }
            mark(t, &f, &mut dirty);
            self.link(&f);
            self.files.insert(f.id.clone(), f);
        }
        for id in &removals {
            if let Some(old) = self.files.remove(id) {
                mark(t, &old, &mut dirty);
                self.unlink(&old);
            }
            if let Some(&ino) = t.by_id.get(id) {
                self.remove_subtree(t, ino);
            }
        }
        let before: Vec<(u64, Vec<u64>)> = dirty.iter().filter_map(|d| t.children.get(d).map(|c| (*d, c.clone()))).collect();
        let mut new_dirs = Vec::new();
        for d in &dirty {
            if t.nodes.contains_key(d) {
                new_dirs.extend(self.refresh_dir(t, *d));
            }
        }
        self.fill(t, new_dirs);
        // Dropped from a listing and not listed anywhere else: now unreachable.
        for (d, old_kids) in before {
            for k in old_kids {
                let still_here = t.children.get(&d).is_some_and(|c| c.binary_search(&k).is_ok());
                if !still_here && t.nodes.get(&k).is_some_and(|n| n.parent == d) {
                    self.remove_subtree(t, k);
                }
            }
        }
    }

    fn flush_inodes(&mut self) -> Result<()> {
        if self.new_inode_rows.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.new_inode_rows);
        let tx = self.db.transaction()?;
        {
            let mut st = tx.prepare("INSERT OR REPLACE INTO inodes(id, ino) VALUES (?1, ?2)")?;
            for (id, ino) in &rows {
                st.execute(params![id, *ino as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn put_kv(&self, k: &str, v: &str) -> Result<()> {
        self.db.execute("INSERT OR REPLACE INTO kv(k, v) VALUES (?1, ?2)", params![k, v])?;
        Ok(())
    }

    fn save_files(&mut self, upserts: &[DFile], removals: &[String]) -> Result<()> {
        let tx = self.db.transaction()?;
        {
            let mut up = tx.prepare("INSERT OR REPLACE INTO files(id, json) VALUES (?1, ?2)")?;
            for f in upserts.iter().filter(|f| !is_local(&f.id)) {
                up.execute(params![f.id, serde_json::to_string(f)?])?;
            }
            let mut rm = tx.prepare("DELETE FROM files WHERE id = ?1")?;
            for id in removals {
                rm.execute(params![id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

impl Meta {
    pub fn open(state_dir: &Path) -> Result<Meta> {
        std::fs::create_dir_all(state_dir)?;
        let db = Connection::open(state_dir.join("meta.sqlite"))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS kv(k TEXT PRIMARY KEY, v TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS files(id TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS inodes(id TEXT PRIMARY KEY, ino INTEGER NOT NULL UNIQUE);",
        )?;
        let kv = |k: &str| -> Result<Option<String>> {
            Ok(db.query_row("SELECT v FROM kv WHERE k = ?1", [k], |r| r.get(0)).optional()?)
        };
        let root_id = kv("root_id")?.unwrap_or_default();
        let page_token = kv("page_token")?;
        let mut files = HashMap::new();
        {
            let mut st = db.prepare("SELECT json FROM files")?;
            for row in st.query_map([], |r| r.get::<_, String>(0))? {
                let f: DFile = serde_json::from_str(&row?)?;
                files.insert(f.id.clone(), f);
            }
        }
        let mut inodes = HashMap::new();
        let mut next_ino = FIRST_INO;
        {
            let mut st = db.prepare("SELECT id, ino FROM inodes")?;
            for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))? {
                let (id, ino) = row?;
                next_ino = next_ino.max(ino + 1);
                inodes.insert(id, ino);
            }
        }
        let inner = Inner {
            db,
            files,
            by_parent: HashMap::new(),
            inodes,
            next_ino,
            new_inode_rows: Vec::new(),
            root_id,
            page_token,
            dir_state: HashMap::new(),
        };
        Ok(Meta { inner: tokio::sync::Mutex::new(inner), tree: RwLock::new(Tree::default()) })
    }

    pub fn tree(&self) -> RwLockReadGuard<'_, Tree> {
        self.tree.read().unwrap()
    }

    /// Path of a node relative to the mount, for display.
    pub fn path_of(&self, ino: u64) -> String {
        let tree = self.tree();
        let mut parts = Vec::new();
        let mut cur = ino;
        while cur != ROOT_INO {
            let Some(n) = tree.node(cur) else { break };
            parts.push(n.name.clone());
            cur = n.parent;
            if parts.len() > 256 {
                break;
            }
        }
        parts.reverse();
        parts.join("/")
    }

    /// The database belongs to one Google account; signing in as another
    /// starts over with a fresh listing.
    pub async fn ensure_account(&self, email: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let have: Option<String> = inner.db.query_row("SELECT v FROM kv WHERE k='account'", [], |r| r.get(0)).optional()?;
        match have.as_deref() {
            Some(a) if a == email => {}
            Some(a) if !inner.files.is_empty() => {
                tracing::warn!("signed in as {email}, but the local database is {a}'s: starting over");
                inner.db.execute_batch("DELETE FROM files; DELETE FROM inodes; DELETE FROM kv WHERE k IN ('page_token','root_id');")?;
                inner.files.clear();
                inner.inodes.clear();
                inner.page_token = None;
                inner.root_id.clear();
                inner.put_kv("account", email)?;
            }
            _ => inner.put_kv("account", email)?,
        }
        Ok(())
    }

    /// Persistent value used in NFS file handles so they survive restarts.
    pub async fn handle_generation(&self) -> Result<u64> {
        let inner = self.inner.lock().await;
        let g: Option<String> = inner.db.query_row("SELECT v FROM kv WHERE k='fh_gen'", [], |r| r.get(0)).optional()?;
        match g.and_then(|g| g.parse().ok()) {
            Some(g) => Ok(g),
            None => {
                let g = chrono::Utc::now().timestamp_millis() as u64;
                inner.put_kv("fh_gen", &g.to_string())?;
                Ok(g)
            }
        }
    }

    fn patch(&self, inner: &mut Inner, upserts: Vec<DFile>, removals: Vec<String>) -> Result<()> {
        let started = std::time::Instant::now();
        {
            let mut t = self.tree.write().unwrap();
            inner.apply(&mut t, upserts, removals);
        }
        tracing::debug!("tree patched in {:?}", started.elapsed());
        inner.flush_inodes()
    }

    /// Warm start from SQLite if we have a page token; otherwise list everything.
    pub async fn bootstrap(&self, drive: &Drive, progress: impl Fn(usize)) -> Result<SyncReport> {
        let mut inner = self.inner.lock().await;
        if inner.page_token.is_none() || inner.root_id.is_empty() {
            // Token first, then list: anything that changes mid-listing is replayed.
            let token = drive.start_page_token().await?;
            let root = drive.get_file("root").await?;
            let all = drive
                .list_all(|n| {
                    progress(n);
                    if n % 5000 < 1000 {
                        tracing::info!("listed {n} files…")
                    }
                })
                .await?;
            tracing::info!("full listing: {} files", all.len());
            inner.db.execute("DELETE FROM files", [])?;
            inner.files = all.into_iter().map(|f| (f.id.clone(), f)).collect();
            let rows: Vec<DFile> = inner.files.values().cloned().collect();
            inner.save_files(&rows, &[])?;
            inner.root_id = root.id.clone();
            inner.put_kv("root_id", &root.id)?;
            inner.put_kv("page_token", &token)?;
            inner.page_token = Some(token);
        } else {
            tracing::info!("warm start: {} files from the local database", inner.files.len());
        }
        let started = std::time::Instant::now();
        {
            let mut t = self.tree.write().unwrap();
            inner.build(&mut t);
            tracing::info!("tree built in {:?} ({} nodes)", started.elapsed(), t.nodes.len());
        }
        inner.flush_inodes()?;
        let files = inner.files.len();
        drop(inner);
        // Offline at startup is fine on a warm start: serve what we have and
        // let the background poll catch up once the network is back.
        match self.poll(drive).await {
            Ok(r) => Ok(SyncReport { changes: r.changes, files }),
            Err(e) => {
                tracing::warn!("change feed unavailable at startup, serving cached metadata: {e:#}");
                Ok(SyncReport { changes: 0, files })
            }
        }
    }

    /// Apply everything in the change feed since the saved page token.
    pub async fn poll(&self, drive: &Drive) -> Result<SyncReport> {
        let mut inner = self.inner.lock().await;
        let mut token = inner.page_token.clone().ok_or_else(|| anyhow!("not bootstrapped"))?;
        let mut upserts = Vec::new();
        let mut removals = Vec::new();
        loop {
            let cl = drive.changes(&token).await?;
            for c in cl.changes {
                match c.file {
                    Some(f) if c.removed != Some(true) && f.trashed != Some(true) => upserts.push(f),
                    _ => removals.push(c.file_id),
                }
            }
            match (cl.next_page_token, cl.new_start_page_token) {
                (Some(next), _) => token = next,
                (None, Some(new)) => {
                    token = new;
                    break;
                }
                (None, None) => break,
            }
        }
        let n = upserts.len() + removals.len();
        if n > 0 {
            inner.save_files(&upserts, &removals)?;
            self.patch(&mut inner, upserts, removals)?;
        }
        if inner.page_token.as_deref() != Some(token.as_str()) {
            inner.put_kv("page_token", &token)?;
            inner.page_token = Some(token);
        }
        Ok(SyncReport { changes: n, files: inner.files.len() })
    }

    /// Reflect our own write immediately instead of waiting for the feed.
    pub async fn upsert_local(&self, f: DFile) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if f.trashed == Some(true) {
            inner.save_files(&[], &[f.id.clone()])?;
            self.patch(&mut inner, vec![], vec![f.id])
        } else {
            inner.save_files(std::slice::from_ref(&f), &[])?;
            self.patch(&mut inner, vec![f], vec![])
        }
    }

    pub async fn remove_local(&self, id: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        inner.save_files(&[], &[id.to_string()])?;
        self.patch(&mut inner, vec![], vec![id.to_string()])
    }

    /// Show a file created through the mount before Drive knows about it.
    pub async fn add_placeholder(&self, parent_id: &str, name: &str) -> Result<(String, u64)> {
        let mut inner = self.inner.lock().await;
        let id = format!("local-{:016x}", rand::random::<u64>());
        let f = DFile {
            id: id.clone(),
            name: drive_name(name),
            mime_type: "application/octet-stream".into(),
            size: Some("0".into()),
            parents: Some(vec![parent_id.to_string()]),
            modified_time: Some(chrono::Utc::now().to_rfc3339()),
            md5_checksum: None,
            head_revision_id: None,
            trashed: None,
        };
        let ino = inner.ino_for(&id);
        self.patch(&mut inner, vec![f], vec![])?;
        Ok((id, ino))
    }

    /// Rename/move a placeholder (nothing to tell Drive yet).
    pub async fn move_placeholder(&self, id: &str, parent_id: &str, name: &str) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let Some(mut f) = inner.files.get(id).cloned() else { return Ok(()) };
        f.name = drive_name(name);
        f.parents = Some(vec![parent_id.to_string()]);
        self.patch(&mut inner, vec![f], vec![])
    }

    /// The upload finished: swap in Drive's record, keeping the same inode so
    /// open handles on the client stay valid.
    pub async fn resolve_placeholder(&self, local_id: &str, real: DFile) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if let Some(ino) = inner.inodes.remove(local_id) {
            inner.inodes.insert(real.id.clone(), ino);
            inner.new_inode_rows.push((real.id.clone(), ino));
        }
        inner.save_files(std::slice::from_ref(&real), &[])?;
        self.patch(&mut inner, vec![real], vec![local_id.to_string()])
    }
}
