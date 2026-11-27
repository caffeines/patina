#!/bin/sh
# Install Patina on Linux from its latest GitHub release:
#
#   curl -fsSL https://raw.githubusercontent.com/caffeines/patina/main/install.sh | sh
#
# On Debian and Ubuntu it installs the .deb with apt, which also installs the
# libraries the window needs (it asks for your password through sudo).
# Elsewhere, or without sudo, it installs the AppImage for you alone: the
# command at ~/.local/bin/patina, with a menu entry and an icon. Downloads are
# checked against the release's SHA256SUMS.
#
#   PATINA_VERSION  a release to install, such as 1.1.0 (default: the latest)
#   PATINA_INSTALL  deb or appimage (default: deb where apt is)
set -eu

REPO=caffeines/patina

say() { printf '%s\n' "$*"; }
fail() { printf 'patina: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Linux) ;;
  Darwin) fail "this installs the Linux app. On a Mac: brew install caffeines/patina/patina" ;;
  *) fail "Patina runs on Linux and macOS, not $(uname -s)." ;;
esac
for c in curl sha256sum; do
  command -v "$c" > /dev/null || fail "this needs $c. Install it, then run this again."
done

case "$(uname -m)" in
  x86_64 | amd64) ARCH=x86_64 DEB_ARCH=amd64 ;;
  aarch64 | arm64) ARCH=aarch64 DEB_ARCH=arm64 ;;
  *) fail "Patina builds for x86_64 and aarch64, not $(uname -m)." ;;
esac

# The latest release, from where GitHub redirects /releases/latest.
VERSION=${PATINA_VERSION:-}
if [ -z "$VERSION" ]; then
  url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") ||
    fail "couldn't reach GitHub to find the latest release."
  VERSION=${url##*/tag/v}
  case "$VERSION" in
    */* | "") fail "couldn't find the latest release at https://github.com/$REPO/releases." ;;
  esac
fi
VERSION=${VERSION#v}

METHOD=${PATINA_INSTALL:-}
if [ -z "$METHOD" ]; then
  METHOD=appimage
  if command -v apt-get > /dev/null && command -v dpkg > /dev/null; then METHOD=deb; fi
fi
case "$METHOD" in
  deb) command -v apt-get > /dev/null || fail "PATINA_INSTALL=deb needs apt. Use PATINA_INSTALL=appimage." ;;
  appimage) ;;
  *) fail "PATINA_INSTALL is deb or appimage, not $METHOD." ;;
esac

# apt runs as root. Without root or sudo, fall back to the AppImage.
SUDO=""
if [ "$METHOD" = deb ] && [ "$(id -u)" -ne 0 ]; then
  if command -v sudo > /dev/null && sudo -v < /dev/null; then
    SUDO=sudo
  elif [ -n "${PATINA_INSTALL:-}" ]; then
    fail "installing the .deb needs root or sudo. PATINA_INSTALL=appimage installs Patina for you alone."
  else
    say "Without sudo, this installs the AppImage, for you alone."
    METHOD=appimage
  fi
fi
if [ "$METHOD" = deb ]; then
  FILE=patina_${VERSION}_${DEB_ARCH}.deb
else
  FILE=Patina-${VERSION}-${ARCH}.AppImage
fi

TMP=$(mktemp -d)
NEW=""
trap 'rm -rf "$TMP" ${NEW:+"$NEW"}' EXIT
# apt reads the .deb as its own _apt user, so let it in.
chmod 755 "$TMP"
BASE=https://github.com/$REPO/releases/download/v$VERSION

curl -fsSL -o "$TMP/SHA256SUMS" "$BASE/SHA256SUMS" ||
  fail "there's no v$VERSION release with Linux downloads (they start after 1.0.0): https://github.com/$REPO/releases"
awk -v f="$FILE" '$2 == f || $2 == "*" f' "$TMP/SHA256SUMS" > "$TMP/sum"
[ -s "$TMP/sum" ] || fail "release v$VERSION has no $FILE."
say "Downloading $FILE"
curl -fSL --progress-bar -o "$TMP/$FILE" "$BASE/$FILE" || fail "couldn't download $BASE/$FILE."
(cd "$TMP" && sha256sum -c sum > /dev/null) || fail "$FILE doesn't match its SHA-256 in the release, so it wasn't installed."
chmod 644 "$TMP/$FILE"

if [ "$METHOD" = deb ]; then
  # The .deb recommends a Docker Engine. Where there's already one, maybe not
  # from apt, leave recommendations out so apt doesn't add a second.
  REC=--no-install-recommends
  if ! command -v docker > /dev/null; then
    REC=""
    say "Docker isn't installed, so apt installs Docker Engine (docker.io) with Patina."
  fi
  # Fresh package lists, so apt finds the libraries the window needs.
  say "Installing it with apt"
  $SUDO apt-get update -qq < /dev/null || fail "apt couldn't update its package lists."
  $SUDO apt-get install -y $REC "$TMP/$FILE" < /dev/null || fail "apt couldn't install $FILE."
  WHERE=/usr/bin/patina
  REMOVE="sudo apt remove patina"
else
  BIN_DIR=$HOME/.local/bin
  DATA=${XDG_DATA_HOME:-$HOME/.local/share}
  ICON=$DATA/icons/hicolor/256x256/apps/patina.png
  ENTRY=$DATA/applications/patina.desktop
  WHERE=$BIN_DIR/patina

  # Next to where it goes, so it can run even where /tmp can't, and so the
  # final move is a rename that leaves a running copy working. Running it
  # once to write the icon also checks it runs here.
  mkdir -p "$BIN_DIR" "$(dirname "$ICON")" "$(dirname "$ENTRY")"
  NEW=$WHERE.new
  mv -f "$TMP/$FILE" "$NEW"
  chmod 755 "$NEW"
  if ! "$NEW" --icon-size 256 --write-icon "$TMP/patina.png" > "$TMP/run.log" 2>&1 < /dev/null; then
    cat "$TMP/run.log" >&2
    fail "the AppImage didn't run on this computer, so it wasn't installed. If it mentions FUSE, install your distribution's fuse3 package."
  fi
  mv -f "$NEW" "$WHERE"
  NEW=""
  mv -f "$TMP/patina.png" "$ICON"
  cat > "$ENTRY" << EOF
[Desktop Entry]
Type=Application
Name=Patina
GenericName=Container Manager
Comment=Containers, images, volumes and builds on this computer's Docker Engine
Exec="$WHERE"
Icon=patina
Terminal=false
Categories=Development;
Keywords=docker;containers;compose;images;volumes;
StartupWMClass=patina
EOF
  REMOVE="rm \"$WHERE\" \"$ENTRY\" \"$ICON\""
fi

say ""
say "Installed Patina $VERSION at $WHERE. Open it from your app menu, or run: patina"
case ":$PATH:" in
  *":$(dirname "$WHERE"):"*) ;;
  *) say "$(dirname "$WHERE") isn't on your PATH, so run it as $WHERE, or add that folder to PATH." ;;
esac
say "To remove it: $REMOVE"
# Patina's own check: whether it can reach Docker, and what to do if not.
say ""
"$WHERE" engine start < /dev/null || true
