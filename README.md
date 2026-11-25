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

To use the engine from a terminal:

```sh
export PATH="$HOME/.patina/bin:$PATH"
docker context use patina        # or: DOCKER_HOST=unix://$HOME/.patina/run/docker.sock
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

## What works

- **Containers:** grouped by Compose project, with live CPU sparklines, real logs, and real exit
  reasons (out of memory, exit codes, health checks). Start, stop, pause, restart and delete.
  Run image (pulls when needed, with port mapping), open in browser, and open a terminal
  (`docker exec` in Terminal.app).
- **Images, volumes, networks:** sizes, which container uses each one, pulls with per-layer
  progress, prune, create and delete. Deleting something still in use is blocked.
- **Engine card:** boot progress and memory use, plus Start, Stop and Restart.
- **Settings:** CPU and memory limits (applied with a restart), Rosetta, open at login, start
  on launch.
- **Command palette:** `⌘K`.

**Not built yet:** Linux machines and Kubernetes. Those views say so. The menu bar popover from
the design isn't built either.

`--demo` runs the simulated engine from the design instead, with nothing touching Docker:

```sh
cargo run -- --demo
cargo run -- --scenario trouble            # containers needing attention (simulated)
cargo run -- --show run --phase conflict   # run dialog, port in use (simulated)
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
| `src/widgets.rs`, `src/tween.rs`, `src/icons.rs`, `src/theme.rs` | Drawing, animation, icons, colors and fonts |

Set `PATINA_DEBUG=1` to print Docker API errors. The guest's boot log is in
`~/.patina/logs/console.log`.

## Fonts

Instrument Sans and DM Mono (Latin subsets, via Fontsource) are licensed under the SIL Open Font
License 1.1.
