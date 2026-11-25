#!/bin/sh
# Build Patina.app (release) with its icon, in target/.
set -eu
cd "$(dirname "$0")/.."

cargo build --release

APP=target/Patina.app
ICONSET=target/Patina.iconset
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

rm -rf "$APP" "$ICONSET"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$ICONSET"
cp target/release/patina "$APP/Contents/MacOS/Patina"

target/release/patina --write-icon target/icon-1024.png
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
  <key>CFBundleIdentifier</key><string>local.patina.desktop</string>
  <key>CFBundleExecutable</key><string>Patina</string>
  <key>CFBundleIconFile</key><string>Patina</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo "Built $APP"
