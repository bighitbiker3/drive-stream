#!/usr/bin/env python3 -I
"""Upload failure handling against the fake Drive: back-off, one history
entry per failing file, recovery, and a local copy that vanishes.

usage: verify_failures.py <mountpoint> <state_dir> <mock_port> <ui_port>
"""
import json
import os
import sys
import time
import urllib.request

mnt, state, mock_port, ui_port = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
ok = True


def check(name, cond, detail=""):
    global ok
    ok &= bool(cond)
    print(f"  {'PASS' if cond else 'FAIL'}  {name}{('  — ' + detail) if detail else ''}", flush=True)


def fail_uploads(on):
    req = urllib.request.Request(f"http://127.0.0.1:{mock_port}/_mutate", data=json.dumps({"op": "fail_uploads", "on": on}).encode(),
                                 headers={"Content-Type": "application/json"})
    urllib.request.urlopen(req).read()


def status():
    return json.load(urllib.request.urlopen(f"http://127.0.0.1:{ui_port}/api/status"))


def active(name):
    return [a for a in status()["uploads"]["active"] if a["path"].endswith(name)]


def history(name):
    return [h for h in status()["uploads"]["history"] if h["path"].endswith(name)]


fail_uploads(True)
p = os.path.join(mnt, "fail-test.bin")
open(p, "wb").write(os.urandom(2_000_000))
time.sleep(20)
a = active("fail-test.bin")
check("failing upload stays pending and backs off", a and "retrying in" in a[0]["detail"], a[0]["detail"] if a else "not pending")
h = history("fail-test.bin")
check("repeated failures are one history entry with a count", len(h) == 1 and h[0]["attempts"] >= 2,
      f"{len(h)} entries, attempts {[x['attempts'] for x in h]}")
check("attempts are spaced out, not hammered", h and h[0]["attempts"] <= 6, f"{h[0]['attempts'] if h else 0} attempts in 20s")

fail_uploads(False)
t = time.time()
while active("fail-test.bin") and time.time() - t < 120:
    time.sleep(1)
h = history("fail-test.bin")
check("uploads once Drive accepts it again", not active("fail-test.bin") and h and h[0]["verified"] is True,
      f"after {time.time() - t:.0f}s")

fail_uploads(True)
v = os.path.join(mnt, "vanish.bin")
open(v, "wb").write(os.urandom(500_000))
ino = os.stat(v).st_ino
t = time.time()
while not history("vanish.bin") and time.time() - t < 30:
    time.sleep(0.5)
staged = os.path.join(state, "staging", str(ino))
check("failing file has a local copy in staging", os.path.exists(staged))
os.remove(staged)
t = time.time()
while active("vanish.bin") and time.time() - t < 40:
    time.sleep(1)
h = history("vanish.bin")
check("a vanished local copy is dropped, not retried forever",
      not active("vanish.bin") and h and "not retrying" in (h[0]["error"] or ""), h[0]["error"] if h else "no record")
fail_uploads(False)
print("ALL PASS" if ok else "SOME CHECKS FAILED")
sys.exit(0 if ok else 1)
