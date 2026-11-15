# Changelog

## Unreleased — first real desktop verification

- Added `--diagnose`: a full report of version, OS, session, GPU/driver, Discord
  socket discovery, RPC and authentication state, subscription, participants,
  window state, configuration and active graphics workarounds. It never prints
  credentials, tokens or secrets, and works with or without a running instance.
- Added `scripts/local-test.sh`: checks the toolchain, WebKitGTK/GTK/AppIndicator
  libraries, session, NVIDIA driver, graphics environment variables, Discord IPC
  sockets and KDE Wayland caveats, then runs the core tests and builds the front
  end and the application. Failures carry the package that fixes them.
- Verified on CachyOS + KDE Plasma 6 Wayland + RTX 3060: native Wayland launch,
  transparent overlay (alpha channel), settings window, hide/show/pin/lock/reset/
  status/quit over the CLI, settings persistence across restarts, tray
  registration, and an idle cost of ~0.8 % of one core (debug build).
- Reproduced `Failed to create GBM buffer: Invalid argument` under XWayland and
  confirmed both mitigations on the same machine.
- Fixed double punctuation in `--status`.


All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — 2026-11-15

First implementation. The GUI has not been exercised on a real desktop yet; see
[TESTING.md](TESTING.md).

### Added

- **Discord IPC client** (`discord-overlay-core`): socket discovery (native,
  Flatpak, Snap, `DISCORD_IPC_PATH`), opcode framing with fragmented-read
  support, `AUTHORIZE`/`AUTHENTICATE` flow, local OAuth2 code exchange with the
  user's own Discord application, per-channel event subscriptions
  (`SPEAKING_*`, `VOICE_STATE_*`), 20 s resync watchdog, exponential reconnect.
- **Overlay state model**: participants, avatars from the Discord CDN, speaking
  flags, self/server mute and deafen, suppressed state, join/leave history,
  stable ordering, revision-based change detection.
- **Transparent overlay window** (Tauri v2): frameless, always-on-top request,
  click-through, interactive toolbar, drag, resize, nine anchors, monitor
  selection, position/size persistence.
- **Settings window** with General, Appearance (with live preview), Position,
  Behavior, Discord and About tabs; partial-patch persistence in
  `~/.config/discord-linux-overlay/config.json`.
- **System tray** menu and **CLI/control socket** (`--toggle`,
  `--toggle-click-through`, `--pin`, `--settings`, `--reset-position`,
  `--toggle-lock`, `--status`, `--quit`) for Wayland-friendly KDE shortcuts.
- **Join history**, connection status, in-app diagnostics log, autostart entry.
- **NVIDIA/WebKitGTK mitigations**: automatic `__NV_DISABLE_EXPLICIT_SYNC=1` on
  NVIDIA + Wayland, opt-in `WEBKIT_DISABLE_DMABUF_RENDERER=1`, capability
  reporting for the current session.
- **Packaging**: PKGBUILD, desktop entry, AppStream metainfo, Flatpak manifest
  (experimental), generated icons, CI workflow.
- **Documentation**: architecture, Linux/Wayland/NVIDIA analysis, upstream
  research report, Discord setup guide, build/install/testing instructions.

### Verified

- 62 unit tests + 5 end-to-end fake-Discord socket tests pass.
- TypeScript strict typecheck and Vite production build pass.
- Dev server serves the overlay and settings documents.

### Not verified

Real Discord session, KDE Wayland behaviour, NVIDIA rendering, tray, global
shortcuts, games, packaging execution. See [TESTING.md](TESTING.md).
- Built and inspected the Arch package on the target machine
  (`discord-linux-overlay-git-r8.6b84d31-1-x86_64.pkg.tar.zst`, 29 files, packaged
  binary runs `--diagnose`). `packaging/PKGBUILD` now sets `options=('!lto')`
  because the distribution's LTO flags break `ring`'s assembly at link time.
