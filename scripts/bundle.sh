#!/usr/bin/env bash
# Assemble a macOS .app bundle from the release binary.
#
# Usage: scripts/bundle.sh
# Output: target/release/MarkForge.app  (+ MarkForge.app.zip)
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="MarkForge"
BIN="markforge"
BUNDLE_ID="dev.rescene.markforge"
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"

APP="target/release/${APP_NAME}.app"
CONTENTS="${APP}/Contents"

echo "→ building release binary"
cargo build --release

echo "→ generating icon"
bash scripts/make_icns.sh

echo "→ assembling ${APP} (v${VERSION})"
rm -rf "${APP}"
mkdir -p "${CONTENTS}/MacOS" "${CONTENTS}/Resources"
cp "target/release/${BIN}" "${CONTENTS}/MacOS/${BIN}"
cp assets/icon.icns "${CONTENTS}/Resources/icon.icns"

cat > "${CONTENTS}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key><string>${APP_NAME}</string>
  <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key><string>${BIN}</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>MIT License</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Markdown Document</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>net.daringfireball.markdown</string>
        <string>public.plain-text</string>
      </array>
      <key>CFBundleTypeExtensions</key>
      <array><string>md</string><string>markdown</string><string>mdown</string><string>txt</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Ad-hoc codesign so Gatekeeper lets it run locally (not notarized).
codesign --force --deep --sign - "${APP}" 2>/dev/null || echo "  (codesign skipped)"

echo "→ zipping"
( cd target/release && rm -f "${APP_NAME}.app.zip" && ditto -c -k --keepParent "${APP_NAME}.app" "${APP_NAME}.app.zip" )

echo "✓ ${APP}"
echo "✓ target/release/${APP_NAME}.app.zip"
