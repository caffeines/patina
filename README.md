# Patina

Containers for your Mac: a self-contained alternative to Docker Desktop and OrbStack, written in
Rust. The app is built with [egui/eframe](https://github.com/emilk/egui) from the *Patina desktop
UI* design: the same layout, colors, type, copy and motion.

Patina doesn't use any other container app. It runs its **own engine**: Docker Engine inside a
small Linux VM that Patina boots itself with Apple's Virtualization framework.

## How the engine works

```
 Patina.app ──┐                       ┌── Linux VM (Debian 13, Virtualization.framework) ──┐
              │ ~/.patina/run/        │                                                     │
 docker CLI ──┼─ docker.sock ── vsock 2375 ── socat ── /run/docker.sock ── Docker Engine     │
              │                       │                                                     │
 localhost:N ─┼─ port forwarder ─ vsock 2376 ── socat ── 127.0.0.1:N (published ports)      │
              │                       │                                                     │
 ~/ (home) ───┼──────── virtiofs ─────┼── mounted at the same path, so -v ~/code:/src works │
 Rosetta ─────┼──────── virtiofs ─────┼── registered with binfmt_misc for linux/amd64       │
              └───────────────────────┴─────────────────────────────────────────────────────┘
```

- **Setup** (first launch, or `patina engine setup`) downloads Debian's official arm64 cloud image
  and checks it against Debian's published SHA-512. It grows the image into a 64 GB sparse disk
  (only used space takes room), points the boot loader at the virtio console, and writes a
  first-boot seed. It also installs the `docker` and `docker-compose` commands into
  `~/.patina/bin` and adds a `patina` docker context, without switching your current one.
- **First boot** installs Docker Engine in the guest with cloud-init (about 2–3 minutes).
  Later boots take about 4 seconds; shutting down takes about 2.
- **The docker command uses Patina.** When the engine finishes starting, Patina makes `patina`
  the current docker context. It also adds one marked line to `~/.zprofile` (and
  `~/.bash_profile`, if you have one) that puts `~/.patina/bin` first on PATH in new terminals.
  Both are switches in Settings › General; turning them off undoes them.
- **The helper** (`patina engine run`) owns the VM. It keeps running after the app quits, like a
  system service. To get the virtualization entitlement, the app keeps an ad-hoc-signed copy of
  itself in `~/.patina/bin/patina-engine`.

Everything lives in `~/.patina`: `vm/` (disk, EFI variables, seed), `bin/` (docker,
docker-compose, the helper), `run/` (sockets), `logs/` (`engine.log`, `console.log`), and
`config.json`.

## Run

```sh
cargo run --release
```

The first launch walks through setup. After that, the app opens to the main window and starts the
engine. You can turn that off in Settings › General.

In a new terminal, `docker` is Patina's and talks to Patina's engine:

```sh
docker run --rm hello-world
```

The engine also has a command line:

```sh
patina engine setup     # download and prepare everything
patina engine start     # boot the VM in the background
patina engine status    # starting | booting | installing | running | stopping
patina engine stop      # shut the guest down cleanly
patina engine reset     # delete the VM disk (keeps downloads); run setup again after
```

To build a double-clickable app into `target/Patina.app`, and to run the tests:

```sh
./scripts/bundle-macos.sh
cargo test
```

## Install with Homebrew

```sh
brew install caffeines/patina/patina           # the latest release
brew install --HEAD caffeines/patina/patina    # the main branch
```

The formula lives in [caffeines/homebrew-patina](https://github.com/caffeines/homebrew-patina).
It builds Patina from source on the Mac it's installed on. That needs no Apple Developer ID or
notarization: Gatekeeper checks apps downloaded from the internet, and this one is compiled
locally. The first install takes a few minutes while Rust compiles.

It installs the `patina` command and `Patina.app` (in `$(brew --prefix)/opt/patina`); `brew info
patina` shows how to add the app to Applications. Homebrew needs current Command Line Tools to
build from source; it says so if they're out of date.

To release a version: set `version` in `Cargo.toml`, commit, then tag and push it here:

```sh
git tag v0.2.0
git push origin main v0.2.0
```

Then set the same tag in the tap's `Formula/patina.rb` and push that. `brew upgrade patina`
picks it up.

## Release a DMG

`./scripts/release-macos.sh` builds `target/dist/Patina-<version>-arm64.dmg`. Without the
variables below, the DMG gets an ad-hoc signature, which works on this Mac but other Macs block.

To publish one that opens cleanly on any Mac:

1. **Get a Developer ID Application certificate.** It comes with the paid Apple Developer
   Program; the account holder creates it in Xcode › Settings › Accounts › Manage
   Certificates › +. (An "Apple Development" certificate can't sign for distribution.)
2. **Save notarization credentials once.** Use an app-specific password from
   account.apple.com:
   ```sh
   xcrun notarytool store-credentials patina-notary \
     --apple-id you@example.com --team-id TEAMID --password app-specific-password
   ```
3. **Build, sign and notarize:**
   ```sh
   DEVELOPER_ID="Developer ID Application: Your Name (TEAMID)" \
   NOTARY_PROFILE=patina-notary \
   BUNDLE_ID=com.yourname.patina \
   ./scripts/release-macos.sh
   ```
   The script signs the app (hardened runtime plus the virtualization entitlement in
   `scripts/Patina.entitlements`), builds the DMG, signs it, sends it to Apple, and staples the
   ticket.
4. **Publish it.** For example, attach it to a GitHub release with
   `gh release create v0.1.0 target/dist/Patina-0.1.0-arm64.dmg`.

Bump `version` in `Cargo.toml` for each release. The DMG is for Apple silicon and macOS 13 or
later.

## What works

- **Containers:** grouped by Compose project, with live CPU sparklines, real logs, and real exit
  reasons (out of memory, exit codes, health checks). Start, stop, pause, restart and delete.
  Run image (pulls when needed, with port mapping), open in browser, and open a terminal
  (`docker exec` in Terminal.app).
- **Images, volumes, networks:** sizes, which container uses each one, pulls with per-layer
  progress, prune, create and delete. Deleting something still in use is blocked.
- **Engine card:** boot progress and memory use, plus Start, Stop and Restart.
- **Settings:** CPU and memory limits (applied with a restart), Rosetta, open at login, start
  on launch, making Patina the docker default, and adding its tools to PATH.
- **Command palette:** `⌘K`.
- **Menu bar:** Patina's mark sits in the menu bar, dimmed while the engine is off. Click it for
  the popover from the design: the engine switch and status, each container with start/stop and
  CPU bars, Open Patina, and Stop all. Right-click for Open Patina and Quit Patina. Closing the
  window keeps Patina running in the menu bar without a Dock icon; opening Patina again brings
  the window back. Settings › General turns it off, and then closing the window quits.

**Not built yet:** Linux machines and Kubernetes. Those views say so.

`--demo` runs the simulated engine from the design instead, with nothing touching Docker:

```sh
cargo run -- --demo
cargo run -- --scenario trouble            # containers needing attention (simulated)
cargo run -- --show run --phase conflict   # run dialog, port in use (simulated)
cargo run -- --popover --scenario failed  # the menu bar popover in a window (simulated)
cargo run -- --help                        # every option
```

## Code map

| File | What it holds |
| --- | --- |
| `src/engine/vm.rs` | The VM: Virtualization.framework through objc2 (EFI boot, disks, NAT, virtio console, vsock, virtiofs, Rosetta) |
| `src/engine/daemon.rs` | `patina engine run`: Docker socket proxy, port forwarding, status and stop |
| `src/engine/setup.rs` | Downloads, checksum check, disk and boot loader, the docker CLI |
| `src/engine/guest.rs` | The guest's first-boot provisioning (cloud-init) |
| `src/engine/control.rs` | The app's side: start, stop, status |
| `src/live.rs` | The app's Docker API client (bollard): polling, stats and log streams, actions |
| `src/sim.rs` | App state and every action; the simulated engine for `--demo` |
| `src/ui/`, `src/onboarding.rs` | The views and the first-run flow |
| `src/menubar.rs` | The status item (AppKit through objc2), its right-click menu, the Dock icon |
| `src/widgets.rs`, `src/tween.rs`, `src/icons.rs`, `src/theme.rs` | Drawing, animation, icons, colors and fonts |

Set `PATINA_DEBUG=1` to print Docker API errors. The guest's boot log is in
`~/.patina/logs/console.log`.

## Fonts

Instrument Sans and DM Mono (Latin subsets, via Fontsource) are licensed under the SIL Open Font
License 1.1.
