#!/usr/bin/env python3 -I
"""Reproduce Finder's copy pattern: create every destination file empty up
front, fill them one at a time, with one source stall mid-file. Each file
must upload exactly once, with its real content, verified against Drive.

usage: verify_finder_copy.py <mountpoint> <state_dir> <drivestream.log>
"""
import hashlib
import json
import os
import random
import sys
import time

mnt, state, log = sys.argv[1], sys.argv[2], sys.argv[3]
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(f"  {'PASS' if cond else 'FAIL'}  {name}{('  — ' + detail) if detail else ''}", flush=True)


def pending():
    return json.load(open(os.path.join(state, "stats.json")))["writes"]["pending_uploads"]


d = os.path.join(mnt, f"finder-copy-{int(time.time())}")
os.mkdir(d)
names = [f"VID_{i:03d}.mp4" for i in range(5)]
data = {n: random.Random(n).randbytes(30_000_000) for n in names}
log_start = os.path.getsize(log)

for n in names:  # Finder: create all destinations first
    open(os.path.join(d, n), "wb").close()
t0 = time.time()
for i, n in enumerate(names):  # ...then fill them one by one
    with open(os.path.join(d, n), "r+b") as f:
        b = data[n]
        for off in range(0, len(b), 1 << 20):
            if i == 2 and off == 15 << 20:
                time.sleep(6)  # the source (a camera card) stalls mid-file
            f.seek(off)
            f.write(b[off:off + (1 << 20)])
    time.sleep(1.5)  # Finder moves on to the next file
print(f"  copy finished after {time.time() - t0:.0f}s; waiting for uploads…")
t = time.time()
time.sleep(3)
while pending() and time.time() - t < 300:
    time.sleep(1)
time.sleep(2.5)

lines = open(log).read()[log_start:].splitlines()
for n in names:
    ups = [l.split("INFO ")[-1] for l in lines if f"uploaded {n} " in l]
    check(f"{n}: uploaded exactly once, full size, checksum verified",
          len(ups) == 1 and "30.0 MB" in ups[0] and "checksum matches Drive" in ups[0], "; ".join(ups) or "never uploaded")
    got = hashlib.md5(open(os.path.join(d, n), "rb").read()).hexdigest()
    check(f"{n}: content reads back byte-exact", got == hashlib.md5(data[n]).hexdigest())
check("no empty uploads", not any("(0.0 MB" in l for l in lines))
print("ALL PASS" if ok else "SOME CHECKS FAILED")
sys.exit(0 if ok else 1)
