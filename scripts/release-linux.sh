#!/bin/sh
# Build Patina for Linux into target/dist:
#
#   Patina-<version>-<arch>.AppImage   runs on most distributions
#   patina_<version>_<debarch>.deb     for Debian and Ubuntu (apt install ./…)
#
# Run it on Linux, on the architecture to build for (x86_64 or aarch64).
# It needs dpkg-deb, objdump, file, and curl to fetch appimagetool and the
# AppImage runtime, which are pinned and checked against their SHA-256.
#
#   PATINA_BIN      an already built patina binary to package. Unset: cargo build --release.
#   DEB_MAINTAINER  the .deb's Maintainer field (default: the project's page).
set -eu
cd "$(dirname "$0")/.."

if [ "$(uname -s)" != Linux ]; then
  echo "Run this on Linux." >&2
  exit 1
fi

missing=""
for c in dpkg-deb objdump file curl sha256sum; do
  command -v "$c" >/dev/null || missing="$missing $c"
done
if [ -n "$missing" ]; then
  echo "Missing:$missing. On Debian and Ubuntu: sudo apt install dpkg binutils file curl coreutils" >&2
  exit 1
fi

TOOL_VERSION=1.9.1    # github.com/AppImage/appimagetool
RUNTIME_VERSION=20251108  # github.com/AppImage/type2-runtime
ARCH=$(uname -m)
case "$ARCH" in
  x86_64)
    DEB_ARCH=amd64
    TOOL_SHA=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
    RUNTIME_SHA=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
    ;;
  aarch64)
    DEB_ARCH=arm64
    TOOL_SHA=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
    RUNTIME_SHA=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
    ;;
  *)
    echo "Patina builds for x86_64 and aarch64, not $ARCH." >&2
    exit 1
    ;;
esac

BIN="${PATINA_BIN:-}"
if [ -z "$BIN" ]; then
  cargo build --release --locked
  BIN=target/release/patina
fi
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
MAINTAINER="${DEB_MAINTAINER:-Patina <https://github.com/caffeines/patina>}"
WORK=target/linux
DIST=target/dist
rm -rf "$WORK"
mkdir -p "$WORK/tools" "$DIST"

# What both packages install, under /usr: the command, its launcher entry,
# and the icon at the sizes desktops look for.
ROOT=$WORK/root
install -D -m 755 "$BIN" "$ROOT/usr/bin/patina"
install -D -m 644 scripts/linux/patina.desktop "$ROOT/usr/share/applications/patina.desktop"
for s in 32 48 64 128 256 512; do
  dir="$ROOT/usr/share/icons/hicolor/${s}x${s}/apps"
  mkdir -p "$dir"
  "$BIN" --icon-size "$s" --write-icon "$dir/patina.png"
done

# The .deb. Patina links only libc and libgcc; the window system and OpenGL
# libraries are loaded when it starts, so they're listed by hand.
DEB=$WORK/deb
mkdir -p "$DEB/DEBIAN"
cp -R "$ROOT/usr" "$DEB/usr"
GLIBC=$(objdump -T "$BIN" | sed -n 's/.*GLIBC_\([0-9][0-9.]*\).*/\1/p' | sort -t. -k1,1n -k2,2n | tail -1)
if [ -z "$GLIBC" ]; then
  echo "Couldn't read which glibc $BIN needs (is objdump installed?)." >&2
  exit 1
fi
cat > "$DEB/DEBIAN/control" <<EOF
Package: patina
Version: $VERSION
Architecture: $DEB_ARCH
Maintainer: $MAINTAINER
Installed-Size: $(du -sk "$DEB/usr" | cut -f1)
Depends: libc6 (>= $GLIBC), libgcc-s1, libgl1, libegl1, libxkbcommon0, libxkbcommon-x11-0, libx11-6, libx11-xcb1, libxcursor1, libxi6, libxrandr2
Recommends: docker.io | docker-ce, libwayland-client0, libwayland-cursor0, libwayland-egl1
Section: admin
Priority: optional
Homepage: https://github.com/caffeines/patina
Description: Containers on this computer's Docker Engine, in a desktop app
 Patina shows and runs your containers, images, volumes, networks and builds,
 with live logs and stats, using the Docker Engine already on this computer.
EOF
dpkg-deb --root-owner-group --build "$DEB" "$DIST/patina_${VERSION}_${DEB_ARCH}.deb"

# The AppImage: the same files, with the entry and icon at the top where
# AppImage tools look, packed by a pinned appimagetool.
APPDIR=$WORK/Patina.AppDir
mkdir -p "$APPDIR"
cp -R "$ROOT/usr" "$APPDIR/usr"
ln -s usr/bin/patina "$APPDIR/AppRun"
cp scripts/linux/patina.desktop "$APPDIR/patina.desktop"
cp "$ROOT/usr/share/icons/hicolor/256x256/apps/patina.png" "$APPDIR/patina.png"
ln -s patina.png "$APPDIR/.DirIcon"

TOOL=$WORK/tools/appimagetool
RUNTIME=$WORK/tools/runtime
curl -fsSL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/$TOOL_VERSION/appimagetool-$ARCH.AppImage"
curl -fsSL -o "$RUNTIME" "https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME_VERSION/runtime-$ARCH"
printf '%s  %s\n%s  %s\n' "$TOOL_SHA" "$TOOL" "$RUNTIME_SHA" "$RUNTIME" | sha256sum -c -
chmod +x "$TOOL"
# Extract-and-run: CI machines and containers often have no FUSE.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=$ARCH "$TOOL" --no-appstream --runtime-file "$RUNTIME" "$APPDIR" "$DIST/Patina-$VERSION-$ARCH.AppImage"

ls -l "$DIST"
