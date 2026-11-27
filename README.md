# Patina

Containers for your Mac: a self-contained alternative to Docker Desktop and OrbStack, written in
Rust. Patina doesn't use any other container app. It runs its **own engine**: Docker Engine in a
small Linux VM that Patina boots itself with Apple's Virtualization framework.

Patina also runs on Linux, where it needs no VM: it uses the Docker Engine already on the
computer.

## What it does

- **Dashboard:** what's on the engine at a glance: disk use by kind and what's reclaimable, the
  engine's status and memory, counts of everything, CPU and memory of all containers over the
  last minute, the busiest containers, and the biggest images and volumes.
- **Containers:** grouped by Compose project, with live CPU sparklines, logs, and real exit
  reasons (out of memory, exit codes, health checks). Run an image with its ports, then start,
  stop, pause, restart and delete containers, open their ports in a browser, or open a shell in
  your terminal.
- **Images, volumes and networks:** sizes, which container uses each one, pulls with per-layer
  progress, create, delete and prune. Deleting something still in use is blocked.
- **Clean up:** choose what goes, with each kind's count and size: dangling and unused images,
  build cache, unused networks, stopped containers and unused volumes. Machines and Kubernetes'
  pods are never touched.
- **Linux machines** (macOS): Ubuntu, Debian, Fedora, Arch and Alpine, each a full system with
  its own init, your user with sudo, and your home folder at the same path. Each one gets its
  own address and a `<name>.patina.local` URL on your Mac.
- **Kubernetes** (macOS): a single-node k3s cluster, added to `~/.kube/config` as the `patina`
  context, with a browser for pods, their logs, delete and scale.
- **Menu bar** (macOS): the engine switch and every container, one click away. Closing the
  window keeps Patina running there.
- **Settings:** open at login, the terminal app to use (Terminal, Warp, iTerm, Ghostty, or on
  Linux, the desktop's), and on macOS, the VM's CPUs and memory and Rosetta for `linux/amd64`
  images. The command palette is `⌘K` (Ctrl+K on Linux).

## Install

### macOS

Patina needs a Mac with Apple silicon and macOS 13 or later.

```sh
brew install caffeines/patina/patina           # the latest release
brew install --HEAD caffeines/patina/patina    # the main branch
```

Homebrew builds Patina from source on your Mac, so Gatekeeper has nothing to block. The first
install takes a few minutes while Rust compiles, and needs current Command Line Tools (Homebrew
says if they're out of date). It installs the `patina` command and `Patina.app`, and `brew info
patina` shows how to add the app to Applications. To update: `brew update && brew upgrade patina`.

Each [release](https://github.com/caffeines/patina/releases) also has a DMG. If the release's
notes say it's ad-hoc signed, macOS blocks it on Macs other than the one that built it, so use
Homebrew.

**The first launch sets up the engine.** It downloads Debian's arm64 cloud image, checks it
against Debian's SHA-512, and installs Docker Engine in the VM. That takes about 2–3 minutes,
once; after that, the engine starts in about 4 seconds. Everything lives in `~/.patina`.

Then `docker` in a new terminal uses Patina's engine:

```sh
docker run --rm hello-world
```

For that, Patina makes `patina` the current docker context, and adds one marked line to
`~/.zprofile` (and `~/.bash_profile`, if you have one) that puts its `docker` and
`docker-compose` first on PATH. Both are switches in Settings › General, and turning them off
undoes them.

### Linux

Patina runs on x86_64 and aarch64, with glibc 2.35 or later (Ubuntu 22.04, Debian 12, Fedora 36
and later). It uses the Docker Engine on the computer and leaves running it to you: it doesn't
set it up, start, stop or reset it.

```sh
curl -fsSL https://raw.githubusercontent.com/caffeines/patina/main/install.sh | sh
```

[`install.sh`](install.sh) installs the latest release for your architecture, and checks the
download against the release's `SHA256SUMS`:

- **Debian and Ubuntu:** it installs the .deb with apt, which asks for your password through
  sudo and adds the libraries the window needs. If there's no Docker yet, apt also installs
  Docker Engine (`docker.io`).
- **Other distributions, or no sudo:** it installs the AppImage for you alone, as
  `~/.local/bin/patina`, with a menu entry and an icon. The AppImage needs FUSE (the `fuse3`
  package).

At the end, it checks that Patina can reach Docker, and says what to do if not: usually
`sudo usermod -aG docker $USER`, then sign out and back in. Run it again to update. To choose
the release or the format, set `PATINA_VERSION=1.1.0` or `PATINA_INSTALL=appimage` on `sh`:
`curl … | PATINA_INSTALL=appimage sh`. To remove Patina, run `sudo apt remove patina`, or delete
the three files the script lists.

Or download a file from a [release](https://github.com/caffeines/patina/releases) yourself:

```sh
sudo apt install ./patina_1.1.0_amd64.deb          # Debian, Ubuntu (or _arm64)
chmod +x Patina-1.1.0-x86_64.AppImage && ./Patina-1.1.0-x86_64.AppImage   # any distribution
```

Linux machines, Kubernetes and the menu bar item need Patina's VM or macOS, so they're
macOS-only for now.

## Contributing

To run Patina from source, find your way around the code, or make a release, see
[CONTRIBUTING.md](CONTRIBUTING.md).
