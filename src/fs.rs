//! The NFSv3 filesystem macOS mounts: lookups and listings come from the
//! in-memory tree, reads from the block cache (or a file's staging copy while
//! it has unsent changes), and writes go through the write-back layer.

use crate::cache::BlockStore;
use crate::drive::Drive;
use crate::meta::{self, Kind, Meta, Node, stub_body};
use crate::writes::{Writes, is_local_only};
use async_trait::async_trait;
use nfsserve::nfs::*;
use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::Arc;

pub struct GFs {
    pub meta: Arc<Meta>,
    pub blocks: Arc<BlockStore>,
    pub drive: Arc<Drive>,
    pub writes: Arc<Writes>,
    pub writable: bool,
    pub uid: u32,
    pub gid: u32,
    pub generation: u64,
}

fn t(m: (i64, u32)) -> nfstime3 {
    nfstime3 { seconds: m.0.max(0) as u32, nseconds: m.1 }
}

fn name_str(n: &filename3) -> Result<&str, nfsstat3> {
    std::str::from_utf8(n.as_ref()).map_err(|_| nfsstat3::NFS3ERR_INVAL)
}

fn io_err(e: impl std::fmt::Display) -> nfsstat3 {
    tracing::error!("{e}");
    nfsstat3::NFS3ERR_IO
}

impl GFs {
    fn attr(&self, n: &Node) -> fattr3 {
        let (ftype, mode, nlink) = match n.kind {
            Kind::Dir => (ftype3::NF3DIR, 0o755, 2),
            Kind::Stub => (ftype3::NF3REG, 0o444, 1),
            Kind::File => (ftype3::NF3REG, if self.writable { 0o644 } else { 0o444 }, 1),
        };
        let (mut size, mut mtime) = (n.size, n.mtime);
        // "Used" is what's actually on this Mac (Finder shows it as "on
        // disk"): cached blocks, or nothing for a file never opened.
        let mut used = match n.kind {
            Kind::File => self.blocks.cached_bytes(&n.id, &n.version).min(n.size),
            _ => 0,
        };
        // A file with unsent changes is described by its staging copy.
        if let Some(md) = self.writes.path_for(n.ino).and_then(|p| std::fs::metadata(p).ok()) {
            size = md.len();
            used = size;
            if let Ok(d) = md.modified().map(|m| m.duration_since(std::time::UNIX_EPOCH).unwrap_or_default()) {
                mtime = (d.as_secs() as i64, d.subsec_nanos());
            }
        }
        fattr3 {
            ftype,
            mode,
            nlink,
            uid: self.uid,
            gid: self.gid,
            size,
            used,
            rdev: specdata3::default(),
            fsid: 0x6764_7276,
            fileid: n.ino,
            atime: t(mtime),
            mtime: t(mtime),
            ctime: t(mtime),
        }
    }

    fn node(&self, ino: u64) -> Result<Node, nfsstat3> {
        self.meta.tree().node(ino).cloned().ok_or(nfsstat3::NFS3ERR_STALE)
    }

    fn dir(&self, ino: u64) -> Result<Node, nfsstat3> {
        let n = self.node(ino)?;
        if n.kind != Kind::Dir {
            return Err(nfsstat3::NFS3ERR_NOTDIR);
        }
        Ok(n)
    }

    fn has_children(&self, ino: u64) -> bool {
        self.meta.tree().children.get(&ino).is_some_and(|c| !c.is_empty())
    }

    fn require_writable(&self) -> Result<(), nfsstat3> {
        if self.writable { Ok(()) } else { Err(nfsstat3::NFS3ERR_ROFS) }
    }

    /// A local, writable copy of a file's current content.
    async fn staging(&self, n: &Node, keep_content: bool) -> Result<PathBuf, nfsstat3> {
        if let Some(p) = self.writes.path_for(n.ino) {
            return Ok(p);
        }
        if n.kind != Kind::File {
            return Err(nfsstat3::NFS3ERR_ACCES);
        }
        let path = self.writes.begin(n.ino, false, !meta::is_local(&n.id)).map_err(io_err)?;
        if keep_content && n.size > 0 {
            // Editing part of an existing file: bring the whole thing down first.
            let f = std::fs::OpenOptions::new().write(true).open(&path).map_err(io_err)?;
            let mut off = 0;
            while off < n.size {
                let (data, _) = self.blocks.read(n, off, 4 << 20).await.map_err(io_err)?;
                f.write_all_at(&data, off).map_err(io_err)?;
                off += data.len() as u64;
            }
        }
        Ok(path)
    }

    /// Delete one entry: placeholders just vanish, Drive items go to the trash.
    async fn discard(&self, n: &Node) -> Result<(), nfsstat3> {
        if n.kind == Kind::Dir && self.has_children(n.ino) {
            return Err(nfsstat3::NFS3ERR_NOTEMPTY);
        }
        if !meta::is_local(&n.id) {
            self.drive.trash(&n.id).await.map_err(io_err)?;
        }
        self.writes.forget(n.ino);
        self.meta.remove_local(&n.id).await.map_err(io_err)
    }

    async fn new_file(&self, dirid: fileid3, name: &str) -> Result<(fileid3, fattr3), nfsstat3> {
        let dir = self.dir(dirid)?;
        let (_, ino) = self.meta.add_placeholder(&dir.id, name).await.map_err(io_err)?;
        self.writes.begin(ino, is_local_only(name), false).map_err(io_err)?;
        let n = self.node(ino)?;
        Ok((ino, self.attr(&n)))
    }
}

#[async_trait]
impl NFSFileSystem for GFs {
    fn capabilities(&self) -> VFSCapabilities {
        if self.writable { VFSCapabilities::ReadWrite } else { VFSCapabilities::ReadOnly }
    }

    fn root_dir(&self) -> fileid3 {
        meta::ROOT_INO
    }

    async fn lookup(&self, dirid: fileid3, filename: &filename3) -> Result<fileid3, nfsstat3> {
        let tree = self.meta.tree();
        let dir = tree.node(dirid).ok_or(nfsstat3::NFS3ERR_STALE)?;
        if dir.kind != Kind::Dir {
            return Err(nfsstat3::NFS3ERR_NOTDIR);
        }
        match name_str(filename)? {
            "." => Ok(dirid),
            ".." => Ok(dir.parent),
            name => tree.lookup(dirid, name).ok_or(nfsstat3::NFS3ERR_NOENT),
        }
    }

    async fn getattr(&self, id: fileid3) -> Result<fattr3, nfsstat3> {
        Ok(self.attr(&self.node(id)?))
    }

    async fn setattr(&self, id: fileid3, setattr: sattr3) -> Result<fattr3, nfsstat3> {
        let n = self.node(id)?;
        if let set_size3::size(size) = setattr.size {
            self.require_writable()?;
            if n.kind == Kind::Dir {
                return Err(nfsstat3::NFS3ERR_ISDIR);
            }
            let path = self.staging(&n, size > 0).await?;
            let f = std::fs::OpenOptions::new().write(true).open(&path).map_err(io_err)?;
            let before = f.metadata().map_err(io_err)?.len();
            f.set_len(size).map_err(io_err)?;
            self.writes.touch(id, size > before);
        }
        // Modes, owners and times aren't something Drive stores; accept them.
        Ok(self.attr(&n))
    }

    async fn read(&self, id: fileid3, offset: u64, count: u32) -> Result<(Vec<u8>, bool), nfsstat3> {
        let node = self.node(id)?;
        if let Some(path) = self.writes.path_for(id) {
            let f = std::fs::File::open(&path).map_err(io_err)?;
            let size = f.metadata().map_err(io_err)?.len();
            let end = (offset + count as u64).min(size);
            let mut buf = vec![0u8; end.saturating_sub(offset) as usize];
            f.read_exact_at(&mut buf, offset).map_err(io_err)?;
            return Ok((buf, end >= size));
        }
        match node.kind {
            Kind::Dir => Err(nfsstat3::NFS3ERR_ISDIR),
            Kind::Stub => {
                let body = stub_body(&node.id);
                let s = (offset as usize).min(body.len());
                let e = (s + count as usize).min(body.len());
                Ok((body[s..e].to_vec(), e == body.len()))
            }
            Kind::File => self.blocks.read(&node, offset, count).await.map_err(|e| {
                tracing::error!("read {} @{offset}+{count}: {e:#}", node.name);
                nfsstat3::NFS3ERR_IO
            }),
        }
    }

    async fn write(&self, id: fileid3, offset: u64, data: &[u8]) -> Result<fattr3, nfsstat3> {
        self.require_writable()?;
        let n = self.node(id)?;
        if n.kind == Kind::Dir {
            return Err(nfsstat3::NFS3ERR_ISDIR);
        }
        let path = self.staging(&n, true).await?;
        let f = std::fs::OpenOptions::new().write(true).open(&path).map_err(io_err)?;
        let before = f.metadata().map_err(io_err)?.len();
        f.write_all_at(data, offset).map_err(io_err)?;
        self.writes.touch(id, offset + data.len() as u64 > before);
        Ok(self.attr(&n))
    }

    async fn create(&self, dirid: fileid3, filename: &filename3, attr: sattr3) -> Result<(fileid3, fattr3), nfsstat3> {
        self.require_writable()?;
        let name = name_str(filename)?;
        let existing = self.meta.tree().lookup(dirid, name);
        if let Some(ino) = existing {
            // Unchecked create of an existing file: open it, truncating if asked.
            let n = self.node(ino)?;
            if n.kind == Kind::Dir {
                return Err(nfsstat3::NFS3ERR_EXIST);
            }
            if let set_size3::size(0) = attr.size {
                let path = self.staging(&n, false).await?;
                std::fs::File::create(path).map_err(io_err)?;
                self.writes.touch(ino, false);
            }
            return Ok((ino, self.attr(&n)));
        }
        self.new_file(dirid, name).await
    }

    async fn create_exclusive(&self, dirid: fileid3, filename: &filename3) -> Result<fileid3, nfsstat3> {
        self.require_writable()?;
        let name = name_str(filename)?;
        if self.meta.tree().lookup(dirid, name).is_some() {
            return Err(nfsstat3::NFS3ERR_EXIST);
        }
        Ok(self.new_file(dirid, name).await?.0)
    }

    async fn mkdir(&self, dirid: fileid3, dirname: &filename3) -> Result<(fileid3, fattr3), nfsstat3> {
        self.require_writable()?;
        let name = name_str(dirname)?;
        let dir = self.dir(dirid)?;
        if self.meta.tree().lookup(dirid, name).is_some() {
            return Err(nfsstat3::NFS3ERR_EXIST);
        }
        let f = self.drive.create_folder(&meta::drive_name(name), &dir.id).await.map_err(io_err)?;
        let id = f.id.clone();
        self.meta.upsert_local(f).await.map_err(io_err)?;
        let tree = self.meta.tree();
        let ino = *tree.by_id.get(&id).ok_or(nfsstat3::NFS3ERR_IO)?;
        Ok((ino, self.attr(tree.node(ino).unwrap())))
    }

    async fn remove(&self, dirid: fileid3, filename: &filename3) -> Result<(), nfsstat3> {
        self.require_writable()?;
        let name = name_str(filename)?;
        let ino = self.meta.tree().lookup(dirid, name).ok_or(nfsstat3::NFS3ERR_NOENT)?;
        let n = self.node(ino)?;
        self.discard(&n).await
    }

    async fn rename(&self, from_dirid: fileid3, from_filename: &filename3, to_dirid: fileid3, to_filename: &filename3) -> Result<(), nfsstat3> {
        self.require_writable()?;
        let (from_name, to_name) = (name_str(from_filename)?, name_str(to_filename)?);
        let src_ino = self.meta.tree().lookup(from_dirid, from_name).ok_or(nfsstat3::NFS3ERR_NOENT)?;
        let src = self.node(src_ino)?;
        let from_dir = self.dir(from_dirid)?;
        let to_dir = self.dir(to_dirid)?;

        let existing = self.meta.tree().lookup(to_dirid, to_name);
        if let Some(dst_ino) = existing {
            if dst_ino == src_ino {
                return Ok(());
            }
            let dst = self.node(dst_ino)?;
            let saving_over = src.kind == Kind::File
                && dst.kind == Kind::File
                && meta::is_local(&src.id)
                && !meta::is_local(&dst.id)
                && !is_local_only(to_name);
            if saving_over {
                // An editor's "write temp file, rename over original": make the
                // new content a revision of the original so its history and
                // sharing survive, instead of trashing it for a new file.
                self.writes.adopt(src_ino, dst_ino).map_err(io_err)?;
                return self.meta.remove_local(&src.id).await.map_err(io_err);
            }
            self.discard(&dst).await?;
        }

        if meta::is_local(&src.id) {
            self.writes.set_local_only(src_ino, is_local_only(to_name)).map_err(io_err)?;
            return self.meta.move_placeholder(&src.id, &to_dir.id, to_name).await.map_err(io_err);
        }
        let mut new_name = meta::drive_name(to_name);
        if let Some(ext) = (src.kind == Kind::Stub).then(|| stub_ext_of(&src.name)).flatten() {
            new_name = new_name.strip_suffix(ext).map(String::from).unwrap_or(new_name);
        }
        let moving = from_dirid != to_dirid;
        let f = self
            .drive
            .rename_move(&src.id, &new_name, moving.then_some(to_dir.id.as_str()), moving.then_some(from_dir.id.as_str()))
            .await
            .map_err(io_err)?;
        self.meta.upsert_local(f).await.map_err(io_err)
    }

    async fn readdir(&self, dirid: fileid3, start_after: fileid3, max_entries: usize) -> Result<ReadDirResult, nfsstat3> {
        let tree = self.meta.tree();
        let dir = tree.node(dirid).ok_or(nfsstat3::NFS3ERR_STALE)?;
        if dir.kind != Kind::Dir {
            return Err(nfsstat3::NFS3ERR_NOTDIR);
        }
        let kids = tree.children.get(&dirid).map(|v| v.as_slice()).unwrap_or(&[]);
        let start = kids.partition_point(|&ino| ino <= start_after);
        let mut entries = Vec::new();
        for &ino in &kids[start..] {
            if entries.len() >= max_entries {
                break;
            }
            let n = &tree.nodes[&ino];
            entries.push(DirEntry { fileid: ino, name: n.name.as_bytes().into(), attr: self.attr(n) });
        }
        let end = start + entries.len() >= kids.len();
        Ok(ReadDirResult { entries, end })
    }

    async fn symlink(&self, _dirid: fileid3, _linkname: &filename3, _symlink: &nfspath3, _attr: &sattr3) -> Result<(fileid3, fattr3), nfsstat3> {
        Err(nfsstat3::NFS3ERR_NOTSUPP)
    }

    async fn readlink(&self, _id: fileid3) -> Result<nfspath3, nfsstat3> {
        Err(nfsstat3::NFS3ERR_NOTSUPP)
    }

    async fn fsinfo(&self, root_fileid: fileid3) -> Result<fsinfo3, nfsstat3> {
        let obj_attributes = match self.getattr(root_fileid).await {
            Ok(a) => post_op_attr::attributes(a),
            Err(_) => post_op_attr::Void,
        };
        Ok(fsinfo3 {
            obj_attributes,
            rtmax: 1 << 20,
            rtpref: 1 << 20,
            rtmult: 4096,
            wtmax: 1 << 20,
            wtpref: 1 << 20,
            wtmult: 4096,
            dtpref: 64 << 10,
            maxfilesize: 5 << 40,
            time_delta: nfstime3 { seconds: 0, nseconds: 1_000_000 },
            properties: FSF_HOMOGENEOUS | FSF_CANSETTIME,
        })
    }

    // Handles carry a generation stored in the database (not the process
    // start time), so they stay valid across daemon restarts.
    fn id_to_fh(&self, id: fileid3) -> nfs_fh3 {
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&self.generation.to_le_bytes());
        data.extend_from_slice(&id.to_le_bytes());
        nfs_fh3 { data }
    }

    fn fh_to_id(&self, fh: &nfs_fh3) -> Result<fileid3, nfsstat3> {
        if fh.data.len() != 16 {
            return Err(nfsstat3::NFS3ERR_BADHANDLE);
        }
        let generation = u64::from_le_bytes(fh.data[0..8].try_into().unwrap());
        if generation != self.generation {
            return Err(nfsstat3::NFS3ERR_STALE);
        }
        Ok(u64::from_le_bytes(fh.data[8..16].try_into().unwrap()))
    }
}

fn stub_ext_of(display_name: &str) -> Option<&'static str> {
    [".gdoc", ".gsheet", ".gslides", ".gdraw", ".gform", ".gmap", ".gsite", ".gscript", ".gjam", ".glink"]
        .into_iter()
        .find(|e| display_name.ends_with(e))
}
