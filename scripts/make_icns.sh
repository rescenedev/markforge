#!/usr/bin/env bash
# Generate assets/icon.icns from the PNG iconset.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 scripts/make_icon.py
iconutil -c icns assets/MarkForge.iconset -o assets/icon.icns
rm -rf assets/MarkForge.iconset
echo "→ assets/icon.icns"
