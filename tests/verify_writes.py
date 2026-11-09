#!/usr/bin/env python3 -I
"""Exercise writes through a Drive Stream mount, entirely inside a new scratch
folder: create, upload, rename, move, save-by-rename, partial overwrite,
delete, and Finder's local-only files. Works against the mock or real Drive.

usage: verify_writes.py <mountpoint> <state_dir> [--keep]
"""
import hashlib
import json
import os
import random
import shutil
import sys
import time

mnt, state = sys.argv[1], sys.argv[2]
keep = "--keep" in sys.argv
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(f"  {'PASS' if cond else 'FAIL'}  {name}{('  — ' + detail) if detail else ''}", flush=True)


def stats():
    return json.load(open(os.path.join(state, "stats.json")))


def wait_uploaded(timeout=180):
    """Wait until nothing is pending upload (stats are refreshed every 2s)."""
    t = time.time()
    time.sleep(2.5)
    while time.time() - t < timeout:
        w = stats().get("writes", {})
        if w.get("pending_uploads", 1) == 0:
            return time.time() - t
        time.sleep(1)
    return None


def md5(path):
    h = hashlib.md5()
    with open(path, "rb") as f:
        while b := f.read(1 << 20):
            h.update(b)
    return h.hexdigest()


scratch = os.path.join(mnt, f"drivestream-test-{time.strftime('%Y%m%d-%H%M%S')}")
print(f"scratch folder: {os.path.basename(scratch)}")
uploads0 = stats()["writes"]["uploads"]

print("folders")
t = time.time()
os.mkdir(scratch)
check("mkdir creates a Drive folder", os.path.isdir(scratch), f"{(time.time() - t) * 1000:.0f} ms")
os.mkdir(os.path.join(scratch, "sub"))

print("small file")
small = os.path.join(scratch, "hello.txt")
with open(small, "w") as f:
    f.write("hello from Drive Stream\n")
check("readable immediately (from local staging)", open(small).read() == "hello from Drive Stream\n")
dt = wait_uploaded()
check("uploaded after settling", dt is not None, f"{dt:.1f}s" if dt else "timed out")

print("20 MB file (multi-chunk resumable upload)")
blob = random.Random(1).randbytes(20_000_000)
src = os.path.join(os.environ.get("TMPDIR", "/tmp"), "drivestream-blob.bin")
open(src, "wb").write(blob)
big = os.path.join(scratch, "blob.bin")
t = time.time()
shutil.copyfile(src, big)
print(f"  copy into mount returned in {time.time() - t:.1f}s (write-back)")
dt = wait_uploaded()
check("uploaded", dt is not None, f"{dt:.1f}s after copy" if dt else "timed out")
check("size", os.path.getsize(big) == len(blob))
check("content round-trips", md5(big) == hashlib.md5(blob).hexdigest())

print("rename and move (metadata-only on Drive)")
ino = os.stat(big).st_ino
t = time.time()
os.rename(big, os.path.join(scratch, "renamed.bin"))
check("rename", os.path.exists(os.path.join(scratch, "renamed.bin")) and not os.path.exists(big), f"{(time.time() - t) * 1000:.0f} ms")
t = time.time()
moved = os.path.join(scratch, "sub", "renamed.bin")
os.rename(os.path.join(scratch, "renamed.bin"), moved)
check("move into subfolder", os.path.exists(moved), f"{(time.time() - t) * 1000:.0f} ms")
check("same file after rename+move (inode kept)", os.stat(moved).st_ino == ino)
check("no re-upload for rename/move", stats()["writes"]["uploads"] - uploads0 == 2)

print("editor-style save (write temp, rename over original)")
ino_before = os.stat(small).st_ino
tmp = os.path.join(scratch, ".hello.txt.sb-tmp")
with open(tmp, "w") as f:
    f.write("hello, edited\n")
os.rename(tmp, small)
check("original path has new content", open(small).read() == "hello, edited\n")
check("same file, not a new one (inode kept)", os.stat(small).st_ino == ino_before)
dt = wait_uploaded()
check("uploaded as a new revision", dt is not None)

print("partial overwrite of an uploaded file")
with open(moved, "r+b") as f:
    f.seek(10_000_000)
    f.write(b"PATCHED!")
expect = blob[:10_000_000] + b"PATCHED!" + blob[10_000_008:]
dt = wait_uploaded()
check("uploaded", dt is not None)
check("content correct", md5(moved) == hashlib.md5(expect).hexdigest())

print("Finder's local-only files")
before = stats()["writes"]["uploads"]
open(os.path.join(scratch, ".DS_Store"), "wb").write(b"\0" * 6148)
open(os.path.join(scratch, "._hello.txt"), "wb").write(b"\0" * 4096)
time.sleep(6)
check(".DS_Store / ._* never uploaded", stats()["writes"]["uploads"] == before)

print("short-lived file never uploads")
before = stats()["writes"]["uploads"]
p = os.path.join(scratch, "build.tmp")
open(p, "w").write("x" * 1000)
os.remove(p)
time.sleep(6)
check("created and deleted within the settle window: no upload", stats()["writes"]["uploads"] == before)

print("delete (goes to Drive trash)")
os.remove(small)
check("gone from the mount", not os.path.exists(small))

s = stats()
print(f"\nquota units used so far: {s['api']['quota_units']}, uploads: {s['writes']['uploads']}, failures: {s['writes']['upload_failures']}")
if not keep:
    # macOS's NFS client deletes a file's "._" companion itself, so entries
    # can vanish mid-walk; ignore those like `rm -rf` does.
    shutil.rmtree(scratch, onerror=lambda fn, p, exc: None if issubclass(exc[0], FileNotFoundError) else (_ for _ in ()).throw(exc[1]))
    check("cleanup: scratch folder removed (to Drive trash)", not os.path.exists(scratch))
print("ALL PASS" if ok else "SOME CHECKS FAILED")
sys.exit(0 if ok else 1)
