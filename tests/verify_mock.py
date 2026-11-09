#!/usr/bin/env python3 -I
"""Check a Drive Stream mount of mockdrive.py: tree shape, byte-exact reads,
sequential throughput, seek latency, and cache behaviour."""
import importlib.util
import json
import os
import random
import sys
import time

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("mockdrive", os.path.join(here, "mockdrive.py"))
mock = importlib.util.module_from_spec(spec)
sys.argv_saved, sys.argv = sys.argv, [sys.argv[0]]
spec.loader.exec_module(mock)
sys.argv = sys.argv_saved

mnt = sys.argv[1]
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(f"  {'PASS' if cond else 'FAIL'}  {name}{('  — ' + detail) if detail else ''}")


def read_range(path, off, n):
    with open(path, "rb", buffering=0) as fh:
        fh.seek(off)
        out = bytearray()
        while len(out) < n:
            b = fh.read(n - len(out))
            if not b:
                break
            out += b
        return bytes(out)


print("tree")
root = sorted(os.listdir(mnt))
check("root listing", root == ["Budget 2026.gsheet", "Videos", "hello.txt", "notes (1).txt", "notes.txt"], str(root))
drone = sorted(os.listdir(f"{mnt}/Videos"))
check("'/' in a Drive name becomes ':' (Finder shows '/')", "Before:After" in drone, str(drone))
check("big file size", os.path.getsize(f"{mnt}/Videos/clip_0066.mov") == 2_076_247_140)
stub = open(f"{mnt}/Budget 2026.gsheet").read()
check("Google Sheet shows as a link stub", json.loads(stub)["doc_id"] == "doc1", stub.strip())
check("small file content", open(f"{mnt}/hello.txt", "rb").read() == mock.content("small", 0, 17))

print("byte-exact reads")
big = f"{mnt}/Videos/clip_0066.mov"
size = 2_076_247_140
rng = random.Random(7)
spots = [0, 4 << 20, (4 << 20) - 100, size // 2, size - 5000] + [rng.randrange(0, size - 300_000) for _ in range(5)]
for off in spots:
    n = min(300_000, size - off)
    got = read_range(big, off, n)
    check(f"range @{off}", got == mock.content("v_big", off, off + n), f"{len(got)} bytes")

print("seek latency (cold blocks)")
for off in [rng.randrange(0, size - 1_000_000) for _ in range(3)]:
    t = time.time()
    read_range(big, off, 64 * 1024)
    print(f"  seek to {off / 1e9:.2f} GB: first 64 KiB in {(time.time() - t) * 1000:.0f} ms")

print("sequential read (cold)")
med = f"{mnt}/Videos/clip_0089.mov"
t = time.time()
n = 0
with open(med, "rb", buffering=0) as fh:
    fh.seek(100_000_000)
    while n < 96 << 20:
        b = fh.read(1 << 20)
        if not b:
            break
        n += len(b)
dt = time.time() - t
print(f"  {n / 2**20:.0f} MiB in {dt:.1f}s = {n / dt / 1e6:.1f} MB/s ({n * 8 / dt / 1e6:.0f} Mbit/s)")
check("sequential beats a single connection (read-ahead in parallel)", n / dt > mock.BANDWIDTH * 1.5,
      f"single connection = {mock.BANDWIDTH / 1e6:.1f} MB/s")

print("re-read (warm)")
t = time.time()
got = read_range(med, 100_000_000, 96 << 20)
dt = time.time() - t
check("warm re-read is local-disk fast", dt < 2.0, f"{len(got) / dt / 1e6:.0f} MB/s")
check("warm re-read is byte-exact", got == mock.content("v_med", 100_000_000, 100_000_000 + (96 << 20)))
print("ALL PASS" if ok else "SOME CHECKS FAILED")
sys.exit(0 if ok else 1)
