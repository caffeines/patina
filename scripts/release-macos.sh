#!/bin/sh
# Build a Patina DMG in target/dist/, signed and notarized when credentials are set.
#
#   DEVELOPER_ID    "Developer ID Application: Name (TEAMID)". Required to distribute.
#   NOTARY_PROFILE  keychain profile made with `xcrun notarytool store-credentials`.
#                   When set, the DMG is notarized by Apple and the ticket stapled.
#   BUNDLE_ID       bundle identifier, e.g. com.example.patina.
#
# Without DEVELOPER_ID the DMG is ad-hoc signed: it works on this Mac, but
# macOS blocks it on others.
set -eu
cd "$(dirname "$0")/.."

if [ -n "${NOTARY_PROFILE:-}" ] && [ -z "${DEVELOPER_ID:-}" ]; then
  echo "Notarization needs DEVELOPER_ID (a Developer ID Application certificate)." >&2
  exit 1
fi

./scripts/bundle-macos.sh

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
DIST=target/dist
STAGE="$DIST/stage"
DMG="$DIST/Patina-$VERSION-$ARCH.dmg"

rm -rf "$DIST"
mkdir -p "$STAGE"
cp -R target/Patina.app "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "Patina $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG"
rm -rf "$STAGE"

if [ -n "${DEVELOPER_ID:-}" ]; then
  codesign --force --timestamp --sign "$DEVELOPER_ID" "$DMG"
fi

if [ -n "${NOTARY_PROFILE:-}" ]; then
  echo "Submitting to Apple for notarization (usually a few minutes)..."
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature --verbose "$DMG"
fi

echo
echo "Built $DMG"
shasum -a 256 "$DMG"
