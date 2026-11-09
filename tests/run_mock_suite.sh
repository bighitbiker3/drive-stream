#!/bin/bash
# Whole suite against the fake Drive: start mockdrive + Drive Stream, run the
# read, change-feed and write checks, then tear everything down.
set -u -o pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
T="$ROOT/target/mock-test"
PORT=18080
BIN="$ROOT/target/release/drivestream"

rm -rf "$T" && mkdir -p "$T/state"
cat > "$T/state/token.json" <<'EOF'
{"client": {"client_id": "fake", "client_secret": "fake"}, "refresh_token": "fake", "scope": "https://www.googleapis.com/auth/drive", "email": "mock@example.com"}
EOF

python3 -I "$ROOT/tests/mockdrive.py" $PORT 0.15 4e6 > "$T/mock.log" 2>&1 &
MOCK=$!
sleep 1
DRIVESTREAM_API_BASE=http://127.0.0.1:$PORT DRIVESTREAM_TOKEN_URL=http://127.0.0.1:$PORT/token \
  "$BIN" --state-dir "$T/state" serve "$T/mnt" --cache-gb 1 --poll-secs 3 --ui-port 17320 > "$T/drivestream.log" 2>&1 &
GFS=$!
cleanup() {
  kill -TERM $GFS 2>/dev/null
  for _ in $(seq 1 40); do kill -0 $GFS 2>/dev/null || break; sleep 0.25; done
  kill -KILL $GFS 2>/dev/null; kill $MOCK 2>/dev/null; /sbin/umount -f "$T/mnt" 2>/dev/null
}
trap cleanup EXIT
for _ in $(seq 1 30); do mount | grep -q "$T/mnt" && break; sleep 0.5; done

status=0
echo "== reads";  python3 -I "$ROOT/tests/verify_mock.py" "$T/mnt" | grep -v "PASS  range" || status=1

echo "== change feed (edits from another device)"
m() { curl -s -X POST -H 'Content-Type: application/json' -d "$1" http://127.0.0.1:$PORT/_mutate > /dev/null; }
m '{"op":"rename","id":"v_med","name":"sunset.mov"}'
m '{"op":"add","id":"new1","name":"from-phone.txt","parent":"ROOT","size":5}'
m '{"op":"trash","id":"dup2"}'
m '{"op":"add","id":"newdir","name":"New Folder From Web","parent":"d_drone","mime":"application/vnd.google-apps.folder"}'
m '{"op":"add","id":"newdirkid","name":"inside.txt","parent":"newdir","size":3}'
seen=""
for i in $(seq 1 30); do
  sleep 1
  ls "$T/mnt/Videos" | grep -q "sunset.mov" && ls "$T/mnt" | grep -q from-phone.txt \
    && ! ls "$T/mnt" | grep -q "notes (1).txt" && [ -f "$T/mnt/Videos/New Folder From Web/inside.txt" ] \
    && { seen=$i; break; }
done
if [ -n "$seen" ]; then echo "  PASS  rename, add, trash, new folder with a file: all visible after ${seen}s"; else echo "  FAIL  change feed"; ls -R "$T/mnt" | head -30; status=1; fi
[ ! -e "$T/mnt/Videos/clip_0089.mov" ] && echo "  PASS  old name gone" || { echo "  FAIL  old name still listed"; status=1; }

echo "== writes"; python3 -I "$ROOT/tests/verify_writes.py" "$T/mnt" "$T/state" | grep -E "FAIL|PASS  (mkdir|rename|move|same)|ALL PASS|SOME" || status=1
echo "== Finder-style copy"; python3 -I "$ROOT/tests/verify_finder_copy.py" "$T/mnt" "$T/state" "$T/drivestream.log" || status=1
echo "== upload failures"; python3 -I "$ROOT/tests/verify_failures.py" "$T/mnt" "$T/state" $PORT 17320 || status=1
echo "== Drive Stream log warnings/errors"; grep -E "WARN|ERROR" "$T/drivestream.log" | sed 's/\x1b\[[0-9;]*m//g' | head -10
exit $status
