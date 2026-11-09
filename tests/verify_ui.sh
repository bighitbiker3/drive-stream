#!/bin/bash
# Dashboard end to end against the fake Drive: start signed out, sign in via
# the loopback redirect, re-sign-in via paste, watch an upload's progress,
# and check the request guards. Leaves a screenshot in target/ui-test/.
set -u -o pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
T="$ROOT/target/ui-test"; PORT=18082; UI=17317; U="http://127.0.0.1:$UI"
rm -rf "$T" && mkdir -p "$T/state"
MOCK_UPLOAD_BPS=8e6 python3 -I "$ROOT/tests/mockdrive.py" $PORT 0.05 50e6 > "$T/mock.log" 2>&1 & MOCK=$!
sleep 1
DRIVESTREAM_API_BASE=http://127.0.0.1:$PORT DRIVESTREAM_TOKEN_URL=http://127.0.0.1:$PORT/token \
  "$ROOT/target/release/drivestream" --state-dir "$T/state" serve "$T/mnt" --cache-gb 1 --poll-secs 3 --ui-port $UI > "$T/drivestream.log" 2>&1 & GFS=$!
trap 'kill -TERM $GFS 2>/dev/null; for _ in $(seq 1 40); do kill -0 $GFS 2>/dev/null || break; sleep 0.25; done; kill -KILL $GFS 2>/dev/null; kill $MOCK 2>/dev/null; /sbin/umount -f "$T/mnt" 2>/dev/null' EXIT
ok=1; pass() { echo "  PASS  $*"; }; fail() { echo "  FAIL  $*"; ok=0; }
st() { curl -s "$U/api/status"; }
j() { python3 -I -c "import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1]))" "$1"; }
for _ in $(seq 1 20); do st >/dev/null && break; sleep 0.3; done

echo "== starts signed out, waiting"
[ "$(st | j 'd["phase"]')" = "Waiting for Google sign-in" ] && pass "phase: waiting for sign-in" || fail "phase: $(st | j 'd["phase"]')"
[ "$(st | j 'd["mounted"]')" = "False" ] && pass "not mounted yet" || fail "mounted before sign-in"
curl -s "$U/" | grep -q "Drive Stream" && pass "dashboard page served" || fail "no dashboard page"
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu --hide-scrollbars --window-size=1000,900 \
  --screenshot="$T/dashboard-signin.png" --virtual-time-budget=2500 "$U/" >/dev/null 2>&1

echo "== request guards"
c=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$U/api/uploads/retry"); [ "$c" = 403 ] && pass "POST without X-Drive-Stream refused" || fail "POST w/o header: $c"
c=$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: evil.example' "$U/api/status"); [ "$c" = 403 ] && pass "foreign Host refused (DNS rebinding)" || fail "foreign host: $c"
c=$(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'X-Drive-Stream: 1' -H 'Content-Type: application/json' -d '{}' "$U/api/signin/start"); [ "$c" = 400 ] && pass "sign-in without a client asks for one" || fail "start w/o client: $c"

echo "== sign in (browser on this Mac: loopback redirect)"
url=$(curl -s -X POST -H 'X-Drive-Stream: 1' -H 'Content-Type: application/json' -d '{"client_id":"cid.apps.googleusercontent.com","client_secret":"GOCSPX-x"}' "$U/api/signin/start" | j 'd["url"]')
read redir state < <(python3 -I -c "import sys,urllib.parse as u; q=dict(u.parse_qsl(u.urlparse(sys.argv[1]).query)); print(q['redirect_uri'], q['state'])" "$url")
[ "$(st | j 'd["signin"]["in_progress"]')" = "True" ] && pass "sign-in in progress shown" || fail "no in-progress state"
curl -s "$redir/?state=$state&code=good-code&scope=x" | grep -q "signed in" && pass "redirect page says signed in" || fail "redirect page"
for i in $(seq 1 40); do [ "$(st | j 'd["mounted"]')" = "True" ] && break; sleep 0.5; done
[ "$(st | j 'd["mounted"]')" = "True" ] && pass "mounted after sign-in ($(st | j 'd["account"]["email"]'))" || fail "not mounted after sign-in: $(st | j 'd["phase"]')"
[ -f "$T/state/token.json" ] && [ "$(stat -f %Lp "$T/state/token.json")" = 600 ] && pass "sign-in saved with 0600 permissions" || fail "token file"

echo "== sign in again (browser elsewhere: paste the address)"
url=$(curl -s -X POST -H 'X-Drive-Stream: 1' -H 'Content-Type: application/json' -d '{}' "$U/api/signin/start" | j 'd["url"]')
read redir state < <(python3 -I -c "import sys,urllib.parse as u; q=dict(u.parse_qsl(u.urlparse(sys.argv[1]).query)); print(q['redirect_uri'], q['state'])" "$url")
bad=$(curl -s -X POST -H 'X-Drive-Stream: 1' -H 'Content-Type: application/json' -d "{\"redirect\":\"$redir/?state=WRONG&code=good-code\"}" "$U/api/signin/finish" | j 'd.get("error","")')
[ -n "$bad" ] && pass "pasted address with the wrong state rejected" || fail "wrong state accepted"
r=$(curl -s -X POST -H 'X-Drive-Stream: 1' -H 'Content-Type: application/json' -d "{\"redirect\":\"$redir/?state=$state&code=good-code\"}" "$U/api/signin/finish" | j 'd.get("email") or d')
[ "$r" = "mock@example.com" ] && pass "pasted address signs in; same account swaps in live" || fail "paste finish: $r"
[ "$(st | j 'd["mounted"]')" = "True" ] && pass "still mounted (no restart)" || fail "lost mount"

echo "== upload progress"
head -c 80000000 /dev/urandom > "$T/src.bin"; cp "$T/src.bin" "$T/mnt/clip-from-camera.mp4"
seen_progress=""; for i in $(seq 1 60); do
  s=$(st | j '[ (a["state"], a["sent"], a["bytes"]) for a in d["uploads"]["active"] ]')
  echo "$s" | grep -q "uploading" && echo "$s" | grep -q -v ", 0, " && seen_progress="$s"
  [ -n "$seen_progress" ] && [ $i -gt 3 ] && break; sleep 0.5; done
[ -n "$seen_progress" ] && pass "progress visible while uploading: $seen_progress" || fail "never saw upload progress"
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu --hide-scrollbars --window-size=1000,1500 \
  --screenshot="$T/dashboard-uploading.png" --virtual-time-budget=2500 "$U/" >/dev/null 2>&1
for i in $(seq 1 60); do [ "$(st | j 'len(d["uploads"]["active"])')" = 0 ] && break; sleep 1; done
h=$(st | j '[(x["path"], x["verified"]) for x in d["uploads"]["history"]]')
echo "$h" | grep -q "('clip-from-camera.mp4', True)" && pass "history shows it uploaded and verified: $h" || fail "history: $h"
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu --hide-scrollbars --window-size=1000,1500 \
  --screenshot="$T/dashboard-done.png" --virtual-time-budget=2500 "$U/" >/dev/null 2>&1
[ $ok = 1 ] && echo "ALL PASS" || echo "SOME CHECKS FAILED"
