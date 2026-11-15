# Upstream research: Overlayed (overlayeddev/overlayed)

Research performed for Discord Linux Overlay on 2026-11-15 by cloning and reading
the upstream source, not only its README.

- Repository: <https://github.com/overlayeddev/overlayed>
- Commit inspected: `ea02d23` — *fix: Sync pin state on startup when Start pinned
  is enabled (#306)* (shallow clone, `main` at time of research)
- Version in the app manifest: `0.6.2`
- License of the repository: **AGPL-3.0** (`LICENSE`, 34,523 bytes, GNU Affero
  General Public License v3)
- License in the desktop metadata: **GPL-3.0-only**
  (`distribution/dev.overlayed.Overlayed.metainfo.xml`,
  `<project_license>GPL-3.0-only</project_license>`)

## 1. Repository structure

```
apps/
  desktop/            Tauri v2 application (the overlay itself)
    src/              React 19 + TypeScript front end
    src-tauri/        Rust shell, Tauri configuration, icons
  api/                Cloudflare Worker that finishes Discord OAuth
  web/                Astro marketing site + blog + downloads
  bot/                Discord bot (community automation, not the overlay)
packages/
  types/ utils/       shared TypeScript
distribution/         Flatpak/AppStream metadata (dev.overlayed.Overlayed)
tooling/ scripts/     lint/build helpers
```

The overlay is a two-process design in one binary: a Tauri Rust shell plus two
web views (an overlay window and a settings window) that render React.

## 2. Application entry points

| File | Role |
| --- | --- |
| `apps/desktop/src-tauri/src/main.rs` | Tauri builder, plugin registration, window creation, tray, window events |
| `apps/desktop/src-tauri/tauri.conf.json` | Window definitions (transparent frameless `main`, decorated `settings`), bundle config |
| `apps/desktop/src/main.tsx` | React entry, router (`/`, `/channel`, `/error`, `#settings`) |
| `apps/desktop/src/rpc/manager.ts` | **The entire Discord integration** |
| `apps/desktop/src/store.ts` | Zustand store for participants/channel/self |
| `apps/desktop/src/config.ts` | JSON config persistence through the fs plugin |

## 3. Discord integration flow (the important part)

Overlayed does **not** scrape tokens or read Discord memory. It uses Discord's
supported *local RPC* bridge:

1. Connect to `ws://127.0.0.1:6463/?v=1&client_id=905987126099836938` with
   `Origin: https://api.overlayed.dev`.
2. Discord sends `READY`.
3. If a cached access token exists, send `AUTHENTICATE`; otherwise send
   `AUTHORIZE` with scopes `["identify", "rpc"]`.
4. Discord shows an authorization prompt in the Discord client and returns an
   OAuth2 `code`.
5. The code is POSTed to `https://api.overlayed.dev/token`, which performs the
   `authorization_code` exchange with *their* Discord application secret and
   returns an access token. This is why Overlayed needs its own API server: an
   OAuth application secret cannot be shipped in a desktop binary.
6. `AUTHENTICATE` with the token. On success Discord returns the current user;
   the app subscribes to `VOICE_CHANNEL_SELECT` and asks for
   `GET_SELECTED_VOICE_CHANNEL`.
7. Voice state is then fully event driven — no polling of Discord:
   `SPEAKING_START`, `SPEAKING_STOP`, `VOICE_STATE_CREATE`, `VOICE_STATE_UPDATE`,
   `VOICE_STATE_DELETE`, `VOICE_CHANNEL_SELECT`.
8. Subscriptions are per channel: for each channel the app sends `SUBSCRIBE`
   with `args.channel_id` and the five channel event names
   (`SUBSCRIBABLE_EVENTS` in `manager.ts`).
9. A 20 s `GET_SELECTED_VOICE_CHANNEL` resync catches lost events; two missed
   replies (5 s timeout each) force a reconnect, and reconnects are rate limited
   to two per minute with backoff `[2s, 5s, 15s, 30s]`.
10. On `AUTHENTICATE` errors (`4009 INVALID_TOKEN`, `4010 INVALID_USER`,
    `5000 OAUTH2_ERROR`) the token is cleared and the user is sent back through
    authorization.

Payload shapes are in `apps/desktop/src/types.ts`; the command/event/error
vocabulary is in `rpc/command.ts`, `rpc/event.ts` and `rpc/errors.ts`.

### Voice state model

`createUserStateItem()` maps a Discord `voice_states[]` entry to:

- `username` = `nick` (server nickname) — **not** the global name
- `muted`/`deafened` = server mute/deafen (`voice_state.mute` / `.deaf`)
- `selfMuted`/`selfDeafened` = `voice_state.self_mute` / `.self_deaf`
- `suppressed` = `voice_state.suppress`
- `talking` starts `false` and is only changed by speaking events

Sorting (`store.ts`): the local user first, then `nick.localeCompare`. Rows are
re-sorted only when users are added, never on speaking, so the layout is stable.

## 4. Overlay rendering architecture

- `main` window: `transparent: true`, `decorations: false`, `shadow: false`,
  `resizable: true`, minimum 64×106, initial 420×600.
- `settings` window: decorated 600×480, hidden until opened, URL `#settings`.
- Both share one React bundle; `App.tsx` routes by hash.
- Participant row (`components/user.tsx`): avatar 32 px, a 2 px border that turns
  green while talking, username pill with `rgba(40,40,40,opacity)`, mute/deafen
  icons (`components/icons/mic-off.tsx`, `headphones-off.tsx`), truncation by
  `maxUsernameLength`, whole-row `transform: scale(userScale/100)`.
- Alignment (`views/channel.tsx`) supports left/center/right horizontal alignment
  and top/bottom vertical alignment; when "anchored" the content is measured with
  a `ResizeObserver` so rows shrink to their natural width.
- Overlay background itself is fully transparent; only pills have a background.

## 5. Window management (Rust)

| File | What it does |
| --- | --- |
| `src-tauri/src/commands.rs` | `set_pin` toggles `window.set_ignore_cursor_events(value)` — "pin" in the UI *is* click-through. Also `set_skip_taskbar` when "hide taskbar when pinned" is on. |
| `src-tauri/src/placement.rs` | Anchored placement: monitor resolution by name, nine anchors, offsets, DPI-aware clamping, `apply_placement` / `release_placement`, `get_monitors`. **Explicitly refuses to run on Wayland** (`positioning_supported()` → `"Fixed placement is not supported on Wayland"`). |
| `src-tauri/src/tray.rs` | Tray icon + menu (pin, show, reload, devtools, settings, quit). |
| `src-tauri/src/hotkeys.rs` | `tauri-plugin-global-shortcut` bindings for pin / visibility / next monitor / settings. |
| `src-tauri/src/window_custom.rs` | macOS only: converts the window to an `NSPanel`, sets a high window level, non-activating style, joins all spaces. |
| `src-tauri/src/main.rs` | `set_always_on_top(true)` for non-macOS, window-state plugin (±position/size), close-to-hide for settings, save bounds on move/resize. |

Platform reality check that shaped our design:

- Always-on-top is a plain `set_always_on_top(true)` call. On Wayland this is a
  no-op: there is no client-side "above" protocol, and KWin ignores it for
  ordinary toplevels.
- Click-through is `set_ignore_cursor_events(true)` — an empty input region. This
  is the only mechanism used for non-interactive mode.
- Fixed placement is disabled on Wayland by upstream itself, which is why users
  have to drag the overlay.
- macOS has a much richer implementation (`NSPanel`, levels) because AppKit
  exposes it; Linux/Wayland has no equivalent.

## 6. Settings and persistence

- `config.json` in `AppConfig`, written through `@tauri-apps/plugin-fs`.
- Defaults and schema in `src/config.ts` (`DEFAULT_OVERLAYED_CONFIG`):
  pin, horizontal/vertical alignment, telemetry, join-history notifications,
  "only show talking", "show own user", opacity, opacity target, max username
  length, user scale, hide-taskbar, placement mode/monitor/anchor/offsets,
  start pinned, hotkeys.
- Config is merged with defaults on load, and window position/size is kept by
  `tauri-plugin-window-state` (a forked build because of an upstream bug).
- Changes are broadcast between windows with a `config_update` event.

## 7. Build and packaging

- pnpm + Turborepo monorepo; desktop builds via the Tauri CLI.
- CI (`release.yaml`) builds macOS universal, Windows (msi/nsis) and Linux
  (`deb`, `appimage`) on GitHub runners; Flatpak is published from a separate
  Flathub repository using `distribution/dev.overlayed.Overlayed.metainfo.xml`.
- Linux build dependencies: `libgtk-3-dev libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`.
- The app has no updater-free build path: updates and telemetry are wired to
  Overlayed's services (Axiom, Sentry, `update.overlayed.dev`), and login to
  `api.overlayed.dev`.

## 8. Existing limitations found in the source

1. **Requires Overlayed's API server** for the OAuth2 code exchange
   (`API_URL/token`). Without it the login path cannot work — a fork either needs
   its own server or must hold its own Discord application secret locally.
2. **Only the official Discord client**: the RPC bridge at port 6463 (or its IPC
   equivalent) is not implemented by Vesktop/ArmCord, so no third-party client
   works. Their metadata states this explicitly (upstream issue #78).
3. **Fixed placement disabled on Wayland** (see `placement.rs`).
4. **No hover/interactive affordance when pinned**: with click-through on, the
   only way back is the tray or a global hotkey — and global hotkeys do not work
   on Wayland.
5. **Per-channel resync every 20 s** even when idle (cheap, but it is traffic).
6. Telemetry (Sentry/Axiom) is on by default.
7. macOS-specific window behaviour is substantial; anything written there does
   not transfer to Linux.

## 9. What we reused, what we replaced

| Upstream concept | Our implementation | Reuse level |
| --- | --- | --- |
| Discord RPC command/event vocabulary, `AUTHORIZE` + `AUTHENTICATE` flow, per-channel `SUBSCRIBE` list, 20 s resync | `crates/discord-overlay-core/src/{protocol,client}.rs` | Reimplemented in Rust from the documented protocol; sequence and error handling informed by `manager.ts` (AGPL-3.0 → same licence) |
| Participant mapping and "self first, then case-insensitive name" ordering | `crates/discord-overlay-core/src/state.rs` | Same behaviour, new code |
| Anchored placement maths (9 anchors, DPI-clamped) | `crates/discord-overlay-core/src/geometry.rs` | Same behaviour, independent implementation + tests |
| Click-through = ignore cursor events; always-on-top | `src-tauri/src/windows.rs` | Same Tauri primitives |
| Config shape and window-state persistence | `crates/discord-overlay-core/src/config.rs` | New schema (nested, versioned, clamped), same idea |
| Tray menu, hotkey plugin, close-to-hide settings | `src-tauri/src/{tray,main}.rs` | Same primitives, new menu |
| React participant row/CSS | `src/components/ParticipantRow.tsx`, `src/styles.css` | New UI, different visual identity |
| `api.overlayed.dev/token` server | Local `HttpTokenExchange` with the user's own Discord application | **Replaced** (no shared secret, no third-party service) |
| macOS `NSPanel` code, updater, telemetry, marketing site, bot | not carried over | Out of scope |

No upstream file was copied verbatim. Behaviour that follows upstream is listed
above so the AGPL attribution is explicit; see `NOTICE`.

## 10. Legal review of reuse

- The repository `LICENSE` is **AGPL-3.0**. The Flatpak/AppStream metadata says
  `GPL-3.0-only`, which is **stale metadata**, not an additional grant: the file
  is not a licence, and distributing AGPL code under a GPL-3.0-only label is an
  error by the packager, not a relicense. We therefore treat the work as
  AGPL-3.0 and licence this project **AGPL-3.0-or-later**, which is compatible
  with AGPL-3.0 and with every dependency upstream uses.
- Attribution is preserved in `NOTICE` (project name, repository, commit,
  licence). The application is clearly independent: it has its own name, its own
  application id (`io.github.polar421.discordlinuxoverlay`), its own config
  directory and its own Discord application requirement. It does not claim
  affiliation with Team Overlayed.
- No Overlayed trademark, logo, or artwork is used. The icons in
  `packaging/icons/` are generated by `scripts/generate-icons.mjs`.

## 11. Consequences for our architecture

1. Build the Discord client in **Rust** instead of the web view, over the local
   IPC socket (`$XDG_RUNTIME_DIR/discord-ipc-N`) rather than the WebSocket
   bridge: no origin validation, no plugin needed, and it can be unit tested
   against a fake socket (which is exactly what `tests/discord_session.rs` does).
2. Keep the **authorization code exchange local** and require the user's own
   Discord application (`docs/DISCORD_SETUP.md`). No server, no shared secret.
3. Put every piece of logic that can be tested headlessly into
   `discord-overlay-core` so CI verifies behaviour without a desktop.
4. Treat Wayland's inability to raise/position windows as a first-class,
   documented platform limitation (`LINUX.md`) rather than silently calling a
   no-op Tauri API.
5. Add a CLI/control socket (`--toggle-click-through`, …) because Wayland has no
   global hotkeys — upstream lacks this escape hatch.
6. Do **not** reuse the WebKitGTK settings window as the primary debugging tool:
   on NVIDIA, WebKit's DMA-BUF renderer is the weakest link, so keep the overlay
   simple (no backdrop blur, no animations) and document the mitigations.
