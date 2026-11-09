//! Drive Stream (`drivestream`): mount Google Drive as a folder that streams
//! file content on demand instead of downloading whole files.
//!
//!   drivestream serve ~/DriveStream     # then sign in at http://127.0.0.1:7317
//!   drivestream auth --client client_secret.json   # or sign in from a terminal

mod auth;
mod cache;
mod drive;
mod fs;
mod meta;
mod ui;
mod writes;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use nfsserve::tcp::{NFSTcp, NFSTcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(version, about = "Mount Google Drive as a folder that streams on demand")]
struct Cli {
    /// Where the sign-in, metadata database and cache live
    #[arg(long, global = true, env = "DRIVESTREAM_STATE")]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Sign in with Google (opens a consent URL; works from another device too)
    Auth {
        /// Google OAuth client ("Desktop app"): the JSON downloaded from Google
        /// Cloud, or a dotenv file with DRIVESTREAM_CLIENT_ID and DRIVESTREAM_CLIENT_SECRET
        #[arg(long)]
        client: PathBuf,
        /// Ask only for read access
        #[arg(long)]
        readonly: bool,
        /// Pre-select this Google account on the consent screen
        #[arg(long)]
        email: Option<String>,
    },
    /// Sync metadata, serve NFS on localhost and mount it
    Serve {
        /// Folder the drive appears at (created if missing)
        mountpoint: PathBuf,
        /// Local NFS port (0 picks a free one)
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Disk cache budget in GB
        #[arg(long, default_value_t = 20.0)]
        cache_gb: f64,
        /// Block size in MiB: one Drive Range request per block
        #[arg(long, default_value_t = 4)]
        block_mb: u64,
        /// How far ahead of a sequential reader to fetch, in MiB
        #[arg(long, default_value_t = 32)]
        readahead_mb: u64,
        /// Concurrent Drive downloads (each for foreground and read-ahead)
        #[arg(long, default_value_t = 8)]
        parallel: usize,
        /// Seconds between change-feed polls
        #[arg(long, default_value_t = 10)]
        poll_secs: u64,
        /// Serve NFS but don't run mount_nfs (for testing)
        #[arg(long)]
        no_mount: bool,
        /// Mount read-only even if the sign-in allows writes
        #[arg(long)]
        readonly: bool,
        /// Seconds a changed file must sit still before it is uploaded
        #[arg(long, default_value_t = 3)]
        settle_secs: u64,
        /// Port for the dashboard (http://127.0.0.1:PORT)
        #[arg(long, default_value_t = 7317)]
        ui_port: u16,
        /// Extra host names the dashboard accepts, for reaching it through a
        /// tunnel or proxy. ".example.com" allows any subdomain. Repeatable.
        #[arg(long = "allow-host", env = "DRIVESTREAM_ALLOW_HOSTS", value_delimiter = ',')]
        allow_hosts: Vec<String>,
    },
    /// Print the running daemon's latest stats
    Status,
}

fn default_state_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Application Support/DriveStream")
}

fn mount(port: u16, mountpoint: &Path) -> Result<()> {
    std::fs::create_dir_all(mountpoint)?;
    // A daemon that died without unmounting leaves a dead mount behind.
    let _ = std::process::Command::new("/sbin/umount").arg("-f").arg(mountpoint).output();
    let opts = format!(
        "nolocks,vers=3,tcp,hard,intr,rsize=1048576,wsize=1048576,readahead=16,\
         acregmin=2,acregmax=10,acdirmin=2,acdirmax=10,port={port},mountport={port}"
    );
    let out = std::process::Command::new("/sbin/mount_nfs")
        .args(["-o", &opts, "localhost:/"])
        .arg(mountpoint)
        .output()
        .context("running mount_nfs")?;
    if !out.status.success() {
        bail!("mount_nfs failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn unmount(mountpoint: &Path) {
    for args in [vec!["/sbin/umount"], vec!["/sbin/umount", "-f"], vec!["/usr/sbin/diskutil", "unmount", "force"]] {
        let ok = std::process::Command::new(args[0])
            .args(&args[1..])
            .arg(mountpoint)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            tracing::info!("unmounted {}", mountpoint.display());
            return;
        }
    }
    tracing::warn!("could not unmount {}", mountpoint.display());
}

fn stats_json(drive: &drive::Drive, blocks: &cache::BlockStore, meta: &meta::Meta, started: Instant) -> serde_json::Value {
    let s = &drive.stats;
    let c = &blocks.stats;
    let (cache_bytes, cache_blocks) = blocks.disk_usage();
    let downloads = s.downloads.load(Relaxed);
    let bytes_down = s.bytes_down.load(Relaxed);
    let dl_ms = s.download_ms.load(Relaxed).max(1);
    serde_json::json!({
        "uptime_s": started.elapsed().as_secs(),
        "files_visible": meta.tree().nodes.len(),
        "api": {
            "requests": s.requests.load(Relaxed),
            "range_downloads": downloads,
            "bytes_downloaded": bytes_down,
            "bytes_uploaded": s.bytes_up.load(Relaxed),
            "quota_units": s.quota_units.load(Relaxed),
            "retries": s.retries.load(Relaxed),
            "avg_download_ms": if downloads > 0 { dl_ms / downloads } else { 0 },
            "avg_first_byte_ms": if downloads > 0 { s.ttfb_ms.load(Relaxed) / downloads } else { 0 },
            "per_request_MBps": if downloads > 0 { bytes_down as f64 / 1e3 / dl_ms as f64 } else { 0.0 },
        },
        "cache": {
            "mem_hits": c.mem_hits.load(Relaxed),
            "disk_hits": c.disk_hits.load(Relaxed),
            "fetches": c.fetches.load(Relaxed),
            "prefetches": c.prefetches.load(Relaxed),
            "foreground_waits": c.foreground_waits.load(Relaxed),
            "foreground_wait_ms": c.foreground_wait_ms.load(Relaxed),
            "bytes_served": c.bytes_served.load(Relaxed),
            "disk_bytes": cache_bytes,
            "disk_blocks": cache_blocks,
        },
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "drivestream=info,nfsserve=warn".into()),
        )
        .with_target(false)
        .init();
    let cli = Cli::parse();
    let state_dir = cli.state_dir.unwrap_or_else(default_state_dir);

    match cli.cmd {
        Cmd::Auth { client, readonly, email } => {
            let scope = if readonly { auth::SCOPE_RO } else { auth::SCOPE_RW };
            let tok = auth::login(&client, scope, &state_dir, email.as_deref()).await?;
            println!("\nSigned in as {}. Saved to {}", tok.email.as_deref().unwrap_or("?"), auth::token_path(&state_dir).display());
        }
        Cmd::Status => {
            let raw = std::fs::read_to_string(state_dir.join("stats.json")).context("no stats yet; is `drivestream serve` running?")?;
            println!("{raw}");
        }
        Cmd::Serve { mountpoint, port, cache_gb, block_mb, readahead_mb, parallel, poll_secs, no_mount, readonly, settle_secs, ui_port, allow_hosts } => {
            let started = Instant::now();
            let auth = Arc::new(auth::Auth::load(&state_dir));
            let app = Arc::new(ui::App::new(state_dir.clone(), mountpoint.clone(), auth.clone(), started));
            {
                let app = app.clone();
                tokio::spawn(async move {
                    if let Err(e) = ui::serve(app, ui_port, ui::AllowedHosts(allow_hosts)).await {
                        tracing::error!("dashboard stopped: {e:#}");
                    }
                });
            }
            tracing::info!("dashboard on http://127.0.0.1:{ui_port}");
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

            // First run (or signed out): wait for a sign-in from the dashboard.
            while !auth.signed_in() {
                app.set_phase("Waiting for Google sign-in");
                tokio::select! {
                    _ = app.signed_in.notified() => {},
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                    _ = term.recv() => return Ok(()),
                }
            }
            let email = auth.email().unwrap_or_default();
            tracing::info!("account: {email}");
            let drive = Arc::new(drive::Drive::new(auth.clone()));
            let meta = Arc::new(meta::Meta::open(&state_dir)?);
            meta.ensure_account(&email).await?;
            let t = Instant::now();
            app.set_phase("Loading your file list");
            let r = loop {
                let progress = |n: usize| app.set_phase(format!("Downloading your file list from Google… {n} files so far"));
                match meta.bootstrap(&drive, progress).await {
                    Ok(r) => break r,
                    Err(e) => {
                        // Keep the dashboard up (e.g. so an expired sign-in can be renewed) and retry.
                        app.set_phase(format!("Couldn't load the file list from Google; retrying: {e:#}"));
                        app.event("error", format!("Starting up failed: {e:#}"));
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(20)) => {},
                            _ = app.signed_in.notified() => {},
                            _ = term.recv() => return Ok(()),
                        }
                    }
                }
            };
            tracing::info!("metadata ready in {:.1}s: {} files, {} changes applied", t.elapsed().as_secs_f64(), r.files, r.changes);

            let blocks = Arc::new(cache::BlockStore::open(
                drive.clone(),
                &state_dir.join("cache"),
                (cache_gb * 1e9) as u64,
                block_mb << 20,
                readahead_mb << 20,
                parallel,
            )?);
            let writable = auth.can_write() && !readonly;
            let writes = Arc::new(writes::Writes::open(&state_dir.join("staging"), Duration::from_secs(settle_secs))?);
            if writable {
                tracing::info!("mode: read-write (uploads after a file sits unchanged for {settle_secs}s)");
            } else {
                tracing::info!("mode: read-only");
            }
            let gfs = fs::GFs {
                meta: meta.clone(),
                blocks: blocks.clone(),
                drive: drive.clone(),
                writes: writes.clone(),
                writable,
                uid: unsafe { libc::getuid() },
                gid: unsafe { libc::getgid() },
                generation: meta.handle_generation().await?,
            };
            let listener = NFSTcpListener::bind(&format!("127.0.0.1:{port}"), gfs).await?;
            let port = listener.get_listen_port();
            tracing::info!("NFS server on 127.0.0.1:{port}");
            tokio::spawn(async move {
                if let Err(e) = listener.handle_forever().await {
                    tracing::error!("NFS server stopped: {e}");
                }
            });
            let _ = app.core.set(ui::Core { drive: drive.clone(), meta: meta.clone(), blocks: blocks.clone(), writes: writes.clone(), writable });

            if !no_mount {
                let mp = mountpoint.clone();
                app.set_phase("Mounting");
                tokio::task::spawn_blocking(move || mount(port, &mp)).await??;
                app.mounted.store(true, Relaxed);
                tracing::info!("mounted at {}", mountpoint.display());
            }
            app.set_phase(if no_mount { "Serving (not mounted)" } else { "Mounted" });

            // Change feed: other devices' edits show up within a poll interval.
            {
                let (meta, drive, app) = (meta.clone(), drive.clone(), app.clone());
                tokio::spawn(async move {
                    let mut failing = false;
                    loop {
                        tokio::time::sleep(Duration::from_secs(poll_secs)).await;
                        match meta.poll(&drive).await {
                            Ok(r) => {
                                if r.changes > 0 {
                                    tracing::info!("change feed: applied {} changes", r.changes);
                                }
                                if failing {
                                    app.event("info", "Back in touch with Google Drive");
                                    failing = false;
                                }
                            }
                            Err(e) => {
                                if !failing {
                                    app.event("warn", format!("Can't reach Google Drive for changes: {e:#}"));
                                    failing = true;
                                }
                            }
                        }
                    }
                });
            }
            {
                let (writes, meta, drive, blocks) = (writes.clone(), meta.clone(), drive.clone(), blocks.clone());
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        let _ = writes.upload_ready(&meta, &drive, &blocks, false);
                    }
                });
            }
            {
                let (drive, blocks, meta, dir, writes) = (drive.clone(), blocks.clone(), meta.clone(), state_dir.clone(), writes.clone());
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        let mut s = stats_json(&drive, &blocks, &meta, started);
                        s["writes"] = serde_json::json!({
                            "pending_uploads": writes.pending(),
                            "uploads": writes.stats.uploads.load(Relaxed),
                            "upload_failures": writes.stats.upload_failures.load(Relaxed),
                            "checksum_mismatches": writes.stats.checksum_mismatches.load(Relaxed),
                        });
                        let _ = std::fs::write(dir.join("stats.json"), serde_json::to_vec_pretty(&s).unwrap());
                    }
                });
            }

            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = term.recv() => {},
                _ = app.restart.notified() => {},
            }
            tracing::info!("shutting down");
            app.set_phase("Shutting down");
            if writes.pending() > 0 {
                tracing::info!("uploading {} file(s) with unsent changes before exit…", writes.pending());
                writes.flush(&meta, &drive, &blocks, Duration::from_secs(120)).await;
            }
            if !no_mount {
                tokio::task::spawn_blocking(move || unmount(&mountpoint)).await?;
            }
            if app.restarting.load(Relaxed) {
                // launchd's KeepAlive brings us back up as the new account.
                std::process::exit(3);
            }
        }
    }
    Ok(())
}
