#!/bin/bash
# Render the menu bar panel (signed out, then mid-upload) against the fake
# Drive, into target/menu-test/*.png.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd); cd "$ROOT"
T=target/menu-test; PORT=18083; UI=17318; APPBIN="target/Drive Stream.app/Contents/MacOS/Drive Stream"
rm -rf $T; mkdir -p $T/state
MOCK_UPLOAD_BPS=6e6 python3 -I tests/mockdrive.py $PORT 0.05 50e6 > $T/mock.log 2>&1 & MOCK=$!
sleep 1
DRIVESTREAM_API_BASE=http://127.0.0.1:$PORT DRIVESTREAM_TOKEN_URL=http://127.0.0.1:$PORT/token \
  target/release/drivestream --state-dir $T/state serve "$PWD/$T/mnt" --cache-gb 1 --ui-port $UI > $T/g.log 2>&1 & G=$!
trap 'kill -TERM $G 2>/dev/null; for _ in $(seq 1 40); do kill -0 $G 2>/dev/null || break; sleep 0.25; done; kill -KILL $G 2>/dev/null; kill $MOCK 2>/dev/null; /sbin/umount -f "$PWD/$T/mnt" 2>/dev/null' EXIT
sleep 1.5
DRIVESTREAM_API=http://127.0.0.1:$UI "$APPBIN" --snapshot "$PWD/$T/panel-signedout.png"
url=$(curl -s -X POST -H 'X-Drive-Stream: 1' -H 'Content-Type: application/json' \
  -d '{"client_id":"c.apps.googleusercontent.com","client_secret":"s"}' http://127.0.0.1:$UI/api/signin/start | python3 -c "import json,sys;print(json.load(sys.stdin)['url'])")
read redir state < <(python3 -c "import sys,urllib.parse as u; q=dict(u.parse_qsl(u.urlparse(sys.argv[1]).query)); print(q['redirect_uri'], q['state'])" "$url")
curl -s "$redir/?state=$state&code=good-code" >/dev/null
for _ in $(seq 1 30); do mount | grep -q "$T/mnt" && break; sleep 0.5; done
head -c 30000000 /dev/urandom > $T/a.bin; cp $T/a.bin "$PWD/$T/mnt/earlier-clip.mp4"; sleep 9
head -c 160000000 /dev/urandom > $T/b.bin; head -c 95000000 /dev/urandom > $T/c.bin
cp $T/b.bin "$PWD/$T/mnt/VID_20261102_190112_043.mp4" & P1=$!
cp $T/c.bin "$PWD/$T/mnt/VID_20261102_190544_044.mp4" & P2=$!
wait $P1 $P2
sleep 13
curl -s http://127.0.0.1:$UI/api/status | python3 -c "import json,sys; d=json.load(sys.stdin); print('active:', [(a['path'],a['state'],a['sent']) for a in d['uploads']['active']])"
DRIVESTREAM_API=http://127.0.0.1:$UI "$APPBIN" --snapshot "$PWD/$T/panel-uploading.png"
ls -la $T/*.png
