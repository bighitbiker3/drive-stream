#!/bin/bash
# Regenerate docs/dashboard.png and docs/menubar.png from the fake Drive
# (a neutral demo account and mount, two uploads in flight). Needs Google
# Chrome for the dashboard capture. Run ./build.sh first.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd); cd "$ROOT"
T=target/screenshots; PORT=18090; UI=17319; MNT="$HOME/DriveStream-demo"
APPBIN="target/Drive Stream.app/Contents/MacOS/Drive Stream"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
rm -rf $T; mkdir -p $T/state docs
MOCK_EMAIL=you@example.com MOCK_UPLOAD_BPS=7e6 python3 -I tests/mockdrive.py $PORT 0.05 50e6 > $T/mock.log 2>&1 & MOCK=$!
sleep 1
DRIVESTREAM_API_BASE=http://127.0.0.1:$PORT DRIVESTREAM_TOKEN_URL=http://127.0.0.1:$PORT/token \
  target/release/drivestream --state-dir $T/state serve "$MNT" --cache-gb 20 --ui-port $UI > $T/d.log 2>&1 & D=$!
trap 'kill -TERM $D 2>/dev/null; for _ in $(seq 1 40); do kill -0 $D 2>/dev/null || break; sleep 0.25; done; kill -KILL $D 2>/dev/null; kill $MOCK 2>/dev/null; /sbin/umount -f "$MNT" 2>/dev/null; rmdir "$MNT" 2>/dev/null' EXIT
sleep 1.5
url=$(curl -s -X POST -H 'X-Drive-Stream: 1' -H 'Content-Type: application/json' \
  -d '{"client_id":"demo.apps.googleusercontent.com","client_secret":"demo"}' http://127.0.0.1:$UI/api/signin/start | python3 -c "import json,sys;print(json.load(sys.stdin)['url'])")
read redir state < <(python3 -c "import sys,urllib.parse as u; q=dict(u.parse_qsl(u.urlparse(sys.argv[1]).query)); print(q['redirect_uri'], q['state'])" "$url")
curl -s "$redir/?state=$state&code=good-code" >/dev/null
for _ in $(seq 1 30); do mount | grep -q "$MNT" && break; sleep 0.5; done
mkdir -p "$MNT/Videos/Weekend trip"
head -c 42000000 /dev/urandom > $T/a.bin; cp $T/a.bin "$MNT/Videos/Weekend trip/Highlights.mov"
head -c 12000000 /dev/urandom > $T/b.bin; cp $T/b.bin "$MNT/Videos/Weekend trip/Drone pass.mov"
sleep 14
head -c 400000000 /dev/urandom > $T/c.bin; head -c 260000000 /dev/urandom > $T/d.bin
cp $T/c.bin "$MNT/Videos/Weekend trip/VID_20261108_101530.mp4" & P1=$!
cp $T/d.bin "$MNT/Videos/Weekend trip/VID_20261108_104112.mp4" & P2=$!
wait $P1 $P2
cat "$MNT/Videos/clip_0089.mov" > /dev/null   # some streaming, so the stats aren't all zero
sleep 22
"$CHROME" --headless=new --disable-gpu --hide-scrollbars --window-size=960,1180 --force-device-scale-factor=2 \
  --screenshot="$T/dashboard-raw.png" --timeout=8000 "http://127.0.0.1:$UI/" >/dev/null 2>&1
magick "$T/dashboard-raw.png" -fuzz 1% -trim +repage -bordercolor "#f5f5f7" -border 56 "$ROOT/docs/dashboard.png"
DRIVESTREAM_API=http://127.0.0.1:$UI "$APPBIN" --snapshot "$ROOT/docs/menubar.png"
ls -la docs/*.png
