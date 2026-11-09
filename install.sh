#!/bin/bash
# Install (or update) Drive Stream for the current user:
#   - builds "Drive Stream.app" and puts it in ~/Applications
#   - runs the background service at login (a LaunchAgent), which mounts
#     your drive and serves the dashboard at http://127.0.0.1:7317
# Re-run any time to update; your sign-in and cache are kept.
#
# Settings (environment variables):
#   MOUNTPOINT=~/DriveStream    where the drive appears
#   CACHE_GB=20                 how much disk the local cache may use
#   UI_PORT=7317                dashboard port
#   ALLOW_HOSTS=                extra dashboard host names, comma-separated,
#                               for reaching it through a tunnel (".ts.net")
#   NO_BUILD=1                  install target/Drive Stream.app as already built
#   NO_OPEN=1                   don't open the app or dashboard afterwards
set -euo pipefail
cd "$(dirname "$0")"

MOUNTPOINT="${MOUNTPOINT:-$HOME/DriveStream}"
CACHE_GB="${CACHE_GB:-20}"
UI_PORT="${UI_PORT:-7317}"
ALLOW_HOSTS="${ALLOW_HOSTS:-}"
LABEL="io.github.drivestream"
APP_DIR="$HOME/Applications/Drive Stream.app"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG="$HOME/Library/Logs/DriveStream.log"
DOMAIN="gui/$(id -u)"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf '\033[31mError:\033[0m %s\n' "$*" >&2; exit 1; }

# --- requirements
[ "$(uname)" = Darwin ] || die "Drive Stream runs on macOS."
major="$(sw_vers -productVersion | cut -d. -f1)"
[ "$major" -ge 14 ] || die "macOS 14 (Sonoma) or newer is required; this Mac has $(sw_vers -productVersion)."
if [ -z "${NO_BUILD:-}" ]; then
  command -v cargo >/dev/null || die "Rust is needed to build: install it from https://rustup.rs, then re-run."
  xcrun --find swiftc >/dev/null 2>&1 || die "Xcode Command Line Tools are needed: run 'xcode-select --install', then re-run."
  say "Building"
  ./build.sh
fi
[ -d "target/Drive Stream.app" ] || die "target/Drive Stream.app is missing; run ./build.sh"

# --- stop a running copy (it finishes pending uploads and unmounts first)
if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
  say "Stopping the running service (pending uploads finish first)"
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  for _ in $(seq 1 300); do launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1 || break; sleep 0.5; done
fi
pkill -x "Drive Stream" 2>/dev/null || true

# --- app
say "Installing $APP_DIR"
mkdir -p "$HOME/Applications"
rm -rf "$APP_DIR"
cp -R "target/Drive Stream.app" "$APP_DIR"

# --- background service
say "Setting up the background service ($LABEL)"
mkdir -p "$MOUNTPOINT" "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
python3 - "$PLIST" "$LABEL" "$LOG" "$APP_DIR/Contents/MacOS/drivestream" "$MOUNTPOINT" "$CACHE_GB" "$UI_PORT" "$ALLOW_HOSTS" <<'PY'
import plistlib, sys
plist, label, log, exe, mount, cache_gb, port, hosts = sys.argv[1:]
args = [exe, "serve", mount, "--cache-gb", cache_gb, "--ui-port", port]
for h in filter(None, (x.strip() for x in hosts.split(","))):
    args += ["--allow-host", h]
with open(plist, "wb") as f:
    plistlib.dump({
        "Label": label,
        "ProgramArguments": args,
        "RunAtLoad": True,
        # Restart after a crash; a clean stop (launchctl bootout) stays stopped.
        "KeepAlive": {"SuccessfulExit": False},
        "ThrottleInterval": 15,
        # Time to finish uploads and unmount on logout or stop.
        "ExitTimeOut": 150,
        # Serves interactive file reads: don't let macOS throttle it.
        "ProcessType": "Interactive",
        "LowPriorityIO": False,
        "StandardOutPath": log,
        "StandardErrorPath": log,
        "EnvironmentVariables": {"NO_COLOR": "1"},
    }, f)
PY
for _ in $(seq 1 20); do launchctl bootstrap "$DOMAIN" "$PLIST" 2>/dev/null && break; sleep 0.5; done
launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1 || die "the service didn't start; see $LOG"

say "Waiting for the dashboard"
for _ in $(seq 1 40); do curl -sf "http://127.0.0.1:$UI_PORT/api/status" >/dev/null && break; sleep 0.5; done
status="$(curl -sf "http://127.0.0.1:$UI_PORT/api/status" || true)"
[ -n "$status" ] || die "the dashboard didn't come up on port $UI_PORT; see $LOG"

[ -n "${NO_OPEN:-}" ] || open "$APP_DIR" || true
if printf '%s' "$status" | grep -q '"signed_in":false'; then
  [ -n "${NO_OPEN:-}" ] || open "http://127.0.0.1:$UI_PORT/" || true
  next="Sign in on the dashboard: http://127.0.0.1:$UI_PORT"
else
  next="Already signed in. Your drive is at $MOUNTPOINT"
fi
cat <<MSG

Drive Stream is installed.
  App:        $APP_DIR (menu bar)
  Drive:      $MOUNTPOINT
  Dashboard:  http://127.0.0.1:$UI_PORT
  Log:        $LOG
  CLI:        "$APP_DIR/Contents/MacOS/drivestream" --help

$next
MSG
