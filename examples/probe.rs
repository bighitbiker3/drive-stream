// Mount-feasibility probe: a read-only NFS export with one small file and one
// synthetic 1 GiB file whose bytes are a function of their offset.
use async_trait::async_trait;
use nfsserve::nfs::*;
use nfsserve::tcp::{NFSTcp, NFSTcpListener};
use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};

const HELLO: &[u8] = b"hello from a user-space NFS server\n";
const BIG: u64 = 1 << 30;

struct Probe;

fn attr(id: fileid3) -> fattr3 {
    let (ftype, mode, size) = match id {
        1 => (ftype3::NF3DIR, 0o755, 0),
        2 => (ftype3::NF3REG, 0o644, HELLO.len() as u64),
        _ => (ftype3::NF3REG, 0o644, BIG),
    };
    fattr3 { ftype, mode, nlink: 1, uid: 501, gid: 20, size, used: size, fileid: id, ..Default::default() }
}

#[async_trait]
impl NFSFileSystem for Probe {
    fn capabilities(&self) -> VFSCapabilities { VFSCapabilities::ReadOnly }
    fn root_dir(&self) -> fileid3 { 1 }
    async fn lookup(&self, dirid: fileid3, name: &filename3) -> Result<fileid3, nfsstat3> {
        match (dirid, name.as_ref()) {
            (1, b"hello.txt") => Ok(2),
            (1, b"big.bin") => Ok(3),
            (1, b".") | (1, b"..") => Ok(1),
            _ => Err(nfsstat3::NFS3ERR_NOENT),
        }
    }
    async fn getattr(&self, id: fileid3) -> Result<fattr3, nfsstat3> { Ok(attr(id)) }
    async fn setattr(&self, _: fileid3, _: sattr3) -> Result<fattr3, nfsstat3> { Err(nfsstat3::NFS3ERR_ROFS) }
    async fn read(&self, id: fileid3, offset: u64, count: u32) -> Result<(Vec<u8>, bool), nfsstat3> {
        let size = attr(id).size;
        let end = (offset + count as u64).min(size);
        if offset >= size { return Ok((vec![], true)); }
        let data = if id == 2 {
            HELLO[offset as usize..end as usize].to_vec()
        } else {
            (offset..end).map(|o| (o / 4096) as u8).collect()
        };
        eprintln!("READ id={id} off={offset} count={count}");
        Ok((data, end >= size))
    }
    async fn write(&self, _: fileid3, _: u64, _: &[u8]) -> Result<fattr3, nfsstat3> { Err(nfsstat3::NFS3ERR_ROFS) }
    async fn create(&self, _: fileid3, _: &filename3, _: sattr3) -> Result<(fileid3, fattr3), nfsstat3> { Err(nfsstat3::NFS3ERR_ROFS) }
    async fn create_exclusive(&self, _: fileid3, _: &filename3) -> Result<fileid3, nfsstat3> { Err(nfsstat3::NFS3ERR_ROFS) }
    async fn mkdir(&self, _: fileid3, _: &filename3) -> Result<(fileid3, fattr3), nfsstat3> { Err(nfsstat3::NFS3ERR_ROFS) }
    async fn remove(&self, _: fileid3, _: &filename3) -> Result<(), nfsstat3> { Err(nfsstat3::NFS3ERR_ROFS) }
    async fn rename(&self, _: fileid3, _: &filename3, _: fileid3, _: &filename3) -> Result<(), nfsstat3> { Err(nfsstat3::NFS3ERR_ROFS) }
    async fn readdir(&self, dirid: fileid3, start_after: fileid3, _max: usize) -> Result<ReadDirResult, nfsstat3> {
        if dirid != 1 { return Err(nfsstat3::NFS3ERR_NOTDIR); }
        let entries = [(2u64, "hello.txt"), (3, "big.bin")]
            .into_iter()
            .filter(|(id, _)| *id > start_after)
            .map(|(id, n)| DirEntry { fileid: id, name: n.as_bytes().into(), attr: attr(id) })
            .collect();
        Ok(ReadDirResult { entries, end: true })
    }
    async fn symlink(&self, _: fileid3, _: &filename3, _: &nfspath3, _: &sattr3) -> Result<(fileid3, fattr3), nfsstat3> { Err(nfsstat3::NFS3ERR_ROFS) }
    async fn readlink(&self, _: fileid3) -> Result<nfspath3, nfsstat3> { Err(nfsstat3::NFS3ERR_NOTSUPP) }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = NFSTcpListener::bind("127.0.0.1:11111", Probe).await?;
    eprintln!("listening on 127.0.0.1:11111");
    listener.handle_forever().await?;
    Ok(())
}
