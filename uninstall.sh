#!/bin/bash
# Remove Drive Stream. Pending uploads finish and the drive unmounts first.
# Your files stay in Google Drive.
#   ./uninstall.sh           keeps your sign-in and cache (re-install picks them up)
#   ./uninstall.sh --purge   also deletes the sign-in, metadata and cache
set -euo pipefail
LABEL="io.github.drivestream"
DOMAIN="gui/$(id -u)"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
STATE="$HOME/Library/Application Support/DriveStream"

if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
  echo "Stopping the background service (pending uploads finish first)…"
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  for _ in $(seq 1 300); do launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1 || break; sleep 0.5; done
fi
rm -f "$PLIST"
pkill -x "Drive Stream" 2>/dev/null || true
rm -rf "$HOME/Applications/Drive Stream.app"
if [ "${1:-}" = "--purge" ]; then
  rm -rf "$STATE" "$HOME/Library/Logs/DriveStream.log"
  echo "Removed Drive Stream, its sign-in and its cache."
else
  echo "Removed Drive Stream. Sign-in and cache kept in $STATE (delete with --purge)."
fi
echo "If you turned on \"Open at Login\" for the menu bar app, remove it in System Settings › General › Login Items."
