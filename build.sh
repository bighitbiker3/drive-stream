#!/bin/bash
# Build "Drive Stream.app" into target/: the menu bar app with the
# `drivestream` background service inside it.
#
#   ./build.sh                       ad-hoc signed, for this Mac
#   SIGN_IDENTITY="Developer ID Application: …" ./build.sh    for distribution
#   BUNDLE_ID=com.example.drivestream ./build.sh              your own identifier
set -euo pipefail
cd "$(dirname "$0")"
BUNDLE_ID="${BUNDLE_ID:-io.github.drivestream.app}"
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(uname -m)"
APP="target/Drive Stream.app"

echo "==> Building the background service (cargo)"
cargo build --release

echo "==> Building the menu bar app (swiftc, $ARCH)"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
xcrun swiftc -O -parse-as-library -target "$ARCH-apple-macos14.0" -sdk "$(xcrun --show-sdk-path)" \
  menubar/DriveStream.swift -o "$APP/Contents/MacOS/Drive Stream"
cp target/release/drivestream "$APP/Contents/MacOS/drivestream"
[ -f assets/AppIcon.icns ] || assets/build-icons.sh
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>Drive Stream</string>
  <key>CFBundleDisplayName</key><string>Drive Stream</string>
  <key>CFBundleExecutable</key><string>Drive Stream</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHumanReadableCopyright</key><string>MIT License</string>
  <key>NSAppTransportSecurity</key><dict><key>NSAllowsLocalNetworking</key><true/></dict>
</dict>
</plist>
PLIST

echo "==> Signing ($([ "$SIGN_IDENTITY" = "-" ] && echo "ad-hoc" || echo "$SIGN_IDENTITY"))"
codesign --force --options runtime --sign "$SIGN_IDENTITY" "$APP/Contents/MacOS/drivestream"
codesign --force --options runtime --sign "$SIGN_IDENTITY" "$APP"
echo "Built $APP ($VERSION)"
