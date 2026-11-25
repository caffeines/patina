#!/bin/sh
# Build target/Patina.app (release), with its icon, signed with the
# virtualization entitlement and the hardened runtime.
#
#   DEVELOPER_ID  signing identity, e.g. "Developer ID Application: Name (TEAMID)".
#                 Unset: ad-hoc signature (fine on this Mac, not for distribution).
#   BUNDLE_ID     bundle identifier (default: local.patina.desktop).
#   PATINA_BIN    an already built patina binary to package (the Homebrew
#                 formula uses this). Unset: cargo build --release.
set -eu
cd "$(dirname "$0")/.."

BIN="${PATINA_BIN:-}"
if [ -z "$BIN" ]; then
  cargo build --release
  BIN=target/release/patina
fi

APP=target/Patina.app
ICONSET=target/Patina.iconset
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
BUNDLE_ID="${BUNDLE_ID:-local.patina.desktop}"
IDENTITY="${DEVELOPER_ID:--}"

rm -rf "$APP" "$ICONSET"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$ICONSET"
cp "$BIN" "$APP/Contents/MacOS/Patina"

"$BIN" --write-icon target/icon-1024.png
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" target/icon-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z "$d" "$d" target/icon-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Patina.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Patina</string>
  <key>CFBundleDisplayName</key><string>Patina</string>
  <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
  <key>CFBundleExecutable</key><string>Patina</string>
  <key>CFBundleIconFile</key><string>Patina</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

if [ "$IDENTITY" = "-" ]; then
  TIMESTAMP="--timestamp=none"
else
  TIMESTAMP="--timestamp"
fi
codesign --force --options runtime $TIMESTAMP \
  --entitlements scripts/Patina.entitlements \
  --sign "$IDENTITY" "$APP"
codesign --verify --strict "$APP"

echo "Built $APP (signed: $IDENTITY)"
