#!/bin/bash
# Render the app icon from SVG: AppIcon.icns for the Mac app, PNGs for the
# README and dashboard. Needs rsvg-convert (brew install librsvg) and iconutil.
# The 16 and 32 px slots use icon-small.svg, which drops detail that would
# blur at that size.
set -euo pipefail
cd "$(dirname "$0")"
SET=".build/AppIcon.iconset"; rm -rf .build; mkdir -p "$SET"
r() { rsvg-convert -w "$2" -h "$2" "$1" -o "$SET/$3"; }
r icon-small.svg 16   icon_16x16.png
r icon-small.svg 32   icon_16x16@2x.png
r icon-small.svg 32   icon_32x32.png
r icon.svg       64   icon_32x32@2x.png
r icon.svg       128  icon_128x128.png
r icon.svg       256  icon_128x128@2x.png
r icon.svg       256  icon_256x256.png
r icon.svg       512  icon_256x256@2x.png
r icon.svg       512  icon_512x512.png
r icon.svg       1024 icon_512x512@2x.png
iconutil -c icns "$SET" -o AppIcon.icns
rsvg-convert -w 256 -h 256 icon.svg -o icon-256.png
rsvg-convert -w 1024 -h 1024 icon.svg -o icon-1024.png
rm -rf .build
echo "wrote AppIcon.icns, icon-256.png, icon-1024.png"
