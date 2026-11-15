# Testing

Honest record of what was executed and what was not.

## Environment used for this work

| Item | Value |
| --- | --- |
| Machine | headless cloud container (no display server, no GPU) |
| OS | Ubuntu 24.04.5 LTS, 2 vCPU |
| Rust | 1.100.0 (rustup, installed in-container) |
| Node / npm | 26.13.0 / 11.20.0 |
| Discord client | not available |
| KDE Plasma / Wayland | not available |
| NVIDIA GPU | not available |
| Display / X11 / Wayland socket | none |

Consequence: everything below is **automated/headless** verification. The
application could not be compiled locally because GTK3/WebKitGTK development
headers need root to install, and it cannot be run here because there is no
desktop. The Tauri shell **is** compiled by CI (`.github/workflows/ci.yml`) on a
GitHub-hosted Ubuntu 24.04 runner with those packages — see section 5.

## Executed and passing

### 1. Core crate tests — `cargo test -p discord-overlay-core --features http`

```
62 unit tests + 5 integration tests, all passing
```

| Suite | What it proves |
| --- | --- |
| `ipc::tests` | frame encoding, fragmented reads, multiple frames per read, oversized-frame rejection, invalid JSON rejection, socket candidate discovery (native/Flatpak/Snap/`/tmp`), explicit `DISCORD_IPC_PATH` override |
| `protocol::tests` | real Discord `voice_states` payload parsing, nickname/global-name fallback, channel `type` parsing, nullable fields, RPC error-code mapping |
| `avatar::tests` | CDN URL construction, default avatar buckets, animated (`a_`) avatars, size clamping, non-numeric ids |
| `state::tests` | self-first ordering, case-insensitive sorting, mute/deafen/suppress mapping, speaking toggling without reordering, resync preserving speaking flags, removal on resync, join/leave history + cap, duplicate dedupe, channel switch clearing, disconnect reset |
| `config::tests` | defaults, clamping, missing/unknown keys, deep patch merge, round-trip on disk with `0600`, corrupt/missing file recovery, XDG path |
| `credentials::tests` | configured detection, secret never in the public view, masked id, `0600` round-trip, idempotent removal |
| `geometry::tests` | all nine anchors at 2× scale, fractional DPI, clamping oversized windows/offsets, invalid input rejection, off-screen detection and re-clamping, serialization |
| `token::tests` | form encoding, error-body reduction, truncation |
| `control::tests` | CLI switch parsing, pin values, request serialization, socket path |
| `client::tests` | backoff capping, status serialization, private token file round-trip |
| `tests/discord_session.rs` | **end-to-end over a real Unix socket against a fake Discord server**: handshake → `READY` → `AUTHORIZE` → local code exchange → `AUTHENTICATE` → subscribe → channel snapshot → `SPEAKING_START`/`STOP` → `VOICE_STATE_CREATE`/`DELETE`, plus cached-token path, reconnect after the socket drops, `WaitingForDiscord` when absent, and `PING`→`PONG` |

### 2. Front end — `npx tsc --noEmit` and `npx vite build`

```
tsc: exit 0 (strict, noUnusedLocals, verbatimModuleSyntax)
vite: 47 modules transformed, dist/index.html + dist/settings.html built
```

### 3. Dev server

`npx vite --host 0.0.0.0 --port 1420` (and `--port 3000` for the Cloud preview
panel) → `GET /` and `GET /settings.html` both returned HTTP 200 with the
preview host header. The browser mock harness renders the overlay and settings
with simulated participants.

### 4. Tauri shell compilation — GitHub Actions `.github/workflows/ci.yml`

```
run for commit 22c0714 (main): Core crate tests ✓  Front end typecheck and build ✓  Tauri shell build (Linux) ✓
```

The `Tauri shell build (Linux)` job installs `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`libayatana-appindicator3-dev`, `librsvg2-dev` and `patchelf`, builds the front end,
then runs `cargo build --manifest-path src-tauri/Cargo.toml --features custom-protocol`.
It succeeds, and produces the `discord-linux-overlay` binary. That proves the whole
GUI shell — Tauri configuration, capabilities, tray, window management, commands,
control socket, Discord client thread — **compiles and links** on Linux.
It is a compile check only: no window is opened and nothing is exercised at runtime.

### 5. Browser preview (UI only)

The overlay and settings documents were rendered in the Cloud browser panel
against the mock feed: four simulated participants with exactly one speaking
ring, one mute badge and one deafen badge, the interactive toolbar, and the
settings tabs (General, Appearance, Position, Behavior, Discord, About) with
their live preview. Screenshots are checked in as
`docs/screenshots/overlay.png` and `docs/screenshots/settings.png`.
This exercises React and CSS only — it is **not** the Tauri application.

## Not executed — unverified

Every item in this list is implemented in code but has **not** been verified, and
must not be described as working:

| # | Item | Why not |
| --- | --- | --- |
| 1 | `cargo build` of `src-tauri` (Tauri shell) | no GTK/WebKit dev headers, no root in the container; CI covers it |
| 2 | Launching the application | no display server |
| 3 | Connecting to a real Discord client | no Discord, no account |
| 4 | Joining a voice channel, participants appearing | requires 2+ Discord accounts |
| 5 | Speaking indicators with real audio | requires a real voice session |
| 6 | Mute/deafen indicators from a real client | same |
| 7 | Avatars loading through the Tauri CSP | needs the real web view |
| 8 | Another person joining/leaving, channel switching | same as 4 |
| 9 | Moving and resizing the overlay | needs a compositor |
| 10 | Click-through on/off | needs a compositor |
| 11 | Always-on-top / KWin rule behaviour | needs KDE Plasma 6 Wayland |
| 12 | Multi-monitor anchoring and DPI | needs two monitors |
| 13 | Settings persistence across restarts | needs the app to run |
| 14 | Tray icon and menu | needs a tray host |
| 15 | Global shortcuts (X11) and CLI shortcuts (Wayland) | needs a desktop session |
| 16 | NVIDIA `Failed to create GBM buffer` mitigation | no NVIDIA GPU |
| 17 | Steam / Proton / Vulkan / OpenGL / fullscreen games | no games, no GPU |
| 18 | CPU/GPU/RAM/startup measurements | no way to run the GUI |
| 19 | PKGBUILD / Flatpak manifest execution | no root, no `base-devel` |
| 20 | AppImage/deb bundling | `tauri build` requires GTK/WebKit |

## How to verify on the target machine (CachyOS, KDE Plasma 6 Wayland, RTX 3060)

Run these in order and record results in this file.

### A. Build and start

```sh
npm ci && npm run build
npm run tauri build -- --no-bundle
./src-tauri/target/release/discord-linux-overlay 2>&1 | tee /tmp/dlo.log
```

Expected: the overlay appears bottom-right, the settings window opens, no
`Failed to create GBM buffer` in the log. If the window is blank, enable
*Disable WebKit DMA-BUF renderer* and restart.

### B. Discord integration

1. Settings → Discord → paste credentials → Save.
2. Approve the prompt in Discord.
3. Join a voice channel.

Expected: your name appears within ~1 s; the status shows "Connected as …".
With a second account, verify: speaking ring on the active speaker, mute icon on
a self-muted user, deafen icon on a deafened user, join/leave rows appear and
disappear, switching channels replaces the list.

### C. Window behaviour

| Test | Expected |
| --- | --- |
| Turn click-through off, drag by the handle | overlay moves, position persists after restart |
| Drag the window edge (manual mode) | resizes, size persists |
| Turn click-through on, click over the overlay | click reaches the window underneath |
| Settings → Position → Apply with each anchor | overlay snaps to that corner (X11) or is placed by KWin (Wayland) |
| Reset position | returns to bottom-right, primary monitor |
| Second monitor: choose it as the placement monitor | overlay moves there (X11); on Wayland drag it and it stays |
| Tray icon | menu opens; toggle/pin/reset/settings/quit work |
| KWin rule "Keep above" | overlay stays over a borderless game |

### D. Games

Test with a Vulkan native game (e.g. `vkcube`), an OpenGL game, a Steam Proton
game in **borderless windowed** mode, then in **exclusive fullscreen**.

Expected: borderless works with the KWin rule, windowed works, exclusive
fullscreen does not (documented limitation). Check that mouse/keyboard/gamepad
input is unaffected while click-through is on, and that frame pacing is unchanged
(no continuous repaint).

### E. Performance

```sh
# idle CPU while connected and alone in a channel
pidstat -p $(pgrep -f target/release/discord-linux-overlay) 1 30
# and GPU
nvidia-smi dmon -s u -c 30
```

Targets: well under 1 % CPU while idle, no continuous GPU work. Record actual
numbers here.

### F. Fill in

| Test | Result |
| --- | --- |
| Build | ☐ |
| Launch (Wayland) | ☐ |
| Launch (XWayland `GDK_BACKEND=x11`) | ☐ |
| Discord auth | ☐ |
| Participants + speaking | ☐ |
| Mute/deafen | ☐ |
| Join/leave/channel switch | ☐ |
| Drag/resize/persist | ☐ |
| Click-through | ☐ |
| Always-on-top (KWin rule) | ☐ |
| Multi-monitor | ☐ |
| Tray | ☐ |
| Shortcuts / CLI | ☐ |
| Borderless game | ☐ |
| Exclusive fullscreen | ☐ (expected: no overlay) |
| Idle CPU / GPU | ☐ |
