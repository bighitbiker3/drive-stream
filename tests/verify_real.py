#!/usr/bin/env python3 -I
"""Read-side checks of a Drive Stream mount of your real Google Drive.
Read-only: changes nothing in Drive.

  verify_real.py MOUNT STATE_DIR VIDEO [--compare-with GOOGLE_DRIVE_APP_ROOT FOLDER...]

VIDEO is the path of a large (1 GB+) video inside the mount, relative to it.
With --compare-with, the listed folders (relative to My Drive; "" is the top)
are checked against what Google's own Drive app shows for them, e.g.
  --compare-with ~/Library/CloudStorage/GoogleDrive-you@gmail.com/"My Drive" "" Photos
"""
import json
import os
import random
import subprocess
import sys
import time

args = sys.argv[1:]
if len(args) < 3:
    sys.exit(__doc__)
mnt, state, REL = args[:3]
gapp, compare = None, []
if "--compare-with" in args:
    i = args.index("--compare-with")
    gapp, compare = args[i + 1], args[i + 2:]
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(f"  {'PASS' if cond else 'FAIL'}  {name}{('  — ' + detail) if detail else ''}", flush=True)


def stats():
    return json.load(open(os.path.join(state, "stats.json")))


def listing(root, rel):
    out = {}
    for e in os.scandir(os.path.join(root, rel)):
        if e.name.startswith("."):
            continue
        out[e.name] = None if e.is_dir() else e.stat().st_size
    return out


if gapp:
    print("listings match Google's own Drive app")
for rel in compare:
    try:
        ours, theirs = listing(mnt, rel), listing(gapp, rel)
    except FileNotFoundError as e:
        check(f"/{rel}", False, str(e))
        continue
    # The Drive app shows Google Docs as .gdoc etc. too, so compare everything.
    missing = sorted(set(theirs) - set(ours))
    extra = sorted(set(ours) - set(theirs))
    size_diff = sorted(n for n in set(ours) & set(theirs) if ours[n] != theirs[n] and theirs[n] is not None and not n.endswith((".gdoc", ".gsheet", ".gslides", ".gform", ".gdraw", ".gmap", ".gsite", ".glink")))
    check(f"/{rel}: {len(ours)} entries", not missing and not extra and not size_diff,
          f"missing={missing[:5]} extra={extra[:5]} size_diff={size_diff[:5]}" if (missing or extra or size_diff) else "names and sizes identical")

path = os.path.join(mnt, REL)
size = os.path.getsize(path)
print(f"\n{REL} ({size / 1e9:.2f} GB)")
s0 = stats()

print("4 MiB from the middle (cold)")
with open(path, "rb", buffering=0) as f:
    f.seek(size // 2)
    t = time.time()
    n = 0
    while n < 4 << 20:
        b = f.read(1 << 20)
        if not b:
            break
        n += len(b)
    print(f"  {n >> 20} MiB in {time.time() - t:.2f}s")

print("first 64 KiB after a cold seek")
rng = random.Random(3)
for off in sorted(rng.randrange(0, size - (8 << 20)) for _ in range(4)):
    with open(path, "rb", buffering=0) as f:
        f.seek(off)
        t = time.time()
        f.read(64 << 10)
        print(f"  @{off / 1e9:.2f} GB: {(time.time() - t) * 1000:.0f} ms")

print("96 MiB sequential from 25% (cold)")
with open(path, "rb", buffering=0) as f:
    f.seek(int(size * 0.25))
    t = time.time()
    n = 0
    marks = []
    while n < 96 << 20:
        b = f.read(1 << 20)
        if not b:
            break
        n += len(b)
        if n % (32 << 20) == 0:
            marks.append(f"{n >> 20}MiB@{time.time() - t:.1f}s")
    dt = time.time() - t
    mbps = n / dt / 1e6
    print(f"  {n >> 20} MiB in {dt:.1f}s = {mbps:.1f} MB/s ({mbps * 8:.0f} Mbit/s)   {' '.join(marks)}")
    check("fast enough for 4K video (>= 10 MB/s)", mbps >= 10, "slower links will show less")

print("re-read the same 96 MiB (warm)")
with open(path, "rb", buffering=0) as f:
    f.seek(int(size * 0.25))
    t = time.time()
    n = sum(len(f.read(1 << 20)) for _ in range(96))
    dt = time.time() - t
    print(f"  {n / dt / 1e6:.0f} MB/s from the local cache")

print("\nplayback through the mount (ffmpeg)")
t = time.time()
probe = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration,bit_rate:stream=codec_name,width,height",
                        "-of", "json", path], capture_output=True, text=True, timeout=300)
info = json.loads(probe.stdout or "{}")
vid = next((s for s in info.get("streams", []) if s.get("width")), {})
dur = float(info.get("format", {}).get("duration", 0))
br = int(info.get("format", {}).get("bit_rate", 0))
check("ffprobe reads the container", probe.returncode == 0 and dur > 0,
      f"{vid.get('codec_name')} {vid.get('width')}x{vid.get('height')}, {dur:.0f}s, {br / 1e6:.0f} Mbit/s, probed in {time.time() - t:.1f}s")
if dur:
    for at in [dur * 0.7, dur * 0.15]:
        t = time.time()
        r = subprocess.run(["ffmpeg", "-v", "error", "-ss", f"{at:.1f}", "-i", path, "-t", "5", "-f", "null", "-"],
                           capture_output=True, text=True, timeout=600)
        dt = time.time() - t
        check(f"decode 5s of video starting at {at:.0f}s (a seek)", r.returncode == 0,
              f"took {dt:.1f}s ({'faster' if dt < 5 else 'slower'} than real time)" + (f" {r.stderr[:200]}" if r.returncode else ""))

s1 = stats()
print(f"\nDrive API used by this test: {s1['api']['requests'] - s0['api']['requests']} requests, "
      f"{s1['api']['quota_units'] - s0['api']['quota_units']} quota units "
      f"(limit: 325,000/min per user), avg time to first byte {s1['api']['avg_first_byte_ms']} ms")
print("ALL PASS" if ok else "SOME CHECKS FAILED")
sys.exit(0 if ok else 1)
