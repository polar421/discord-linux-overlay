# Linux, KDE Plasma Wayland and NVIDIA

This document records what actually works on Wayland, what does not, and why the
original Overlayed installation fails on NVIDIA. It is grounded in the WebKitGTK,
Tauri and NVIDIA bug reports listed at the end, and in reading Overlayed's own
source.

Nothing in this file is guesswork presented as fact: items that were not verified
on a real KDE Plasma + NVIDIA machine are marked **unverified**.

## 1. The reported failure, explained

The existing installation is Overlayed 0.6.2 (Flatpak, GNOME Platform 50
runtime), on CachyOS, KDE Plasma 6 Wayland, NVIDIA RTX 3060, driver 620.70.06.

### Case A — `flatpak run dev.overlayed.Overlayed`

```
Failed to create GBM buffer: Invalid argument
```

The Flatpak command has no `--socket=wayland`, so the app runs through
**XWayland/X11**. WebKitGTK 2.42+ uses its DMA-BUF renderer by default, which on
X11 allocates a GBM buffer that must be both renderable **and** CPU-mappable
(`GBM_BO_USE_RENDERING | GBM_BO_USE_LINEAR`). NVIDIA's GBM backend rejects that
exact combination with `EINVAL`, even though
`gbm_device_is_format_supported()` reports success. WebKit has no fallback to its
shared-memory path, so the web view never paints.

This is an upstream WebKitGTK ↔ NVIDIA interaction, not an Overlayed bug. It is
the same error reported for every Tauri application on NVIDIA/X11
(tauri#9304, tauri#13493, WebKit 261874, tauri linux-graphics docs).

### Case B — `WEBKIT_DISABLE_DMABUF_RENDERER=1`

The window renders through the shared-memory (software) renderer, which is why the
settings window appears. The rendering defects the user saw are expected:

- `backdrop-filter: blur()` and other GPU-composited effects silently stop
  working (WebKit bug 303811 documents exactly this),
- fonts/scroll performance degrade,
- the accelerated compositor may still be torn down between frames.

So this workaround trades correctness of visuals for the ability to start.

### Case C — `--socket=wayland --env=GDK_BACKEND=wayland --env=__NV_DISABLE_EXPLICIT_SYNC=1`

Now the app runs **natively on Wayland**. The GBM/X11 path is not used, so the
settings window renders correctly — matching the user's observation.

The env var is needed because KWin 6 implements
`wp_linux_drm_syncobj_v1` (explicit synchronization). WebKitGTK 2.46–2.54 commits
a DMA-BUF surface with explicit sync enabled but without setting an acquire
point; the compositor terminates the client with:

```
wl_display#1.error(wp_linux_drm_syncobj_surface_v1#55, 4,
  "explicit sync is used, but no acquire point is set")
Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display
```

Disabling NVIDIA's explicit sync (`__NV_DISABLE_EXPLICIT_SYNC=1`) avoids it while
keeping the fast renderer. That is WebKit bug 324551 / 280210 and the documented
Tauri workaround.

### Why the overlay still was not above games

Because on native Wayland **an application cannot raise itself above other
windows**. There is no `xdg_toplevel` request for it and `tao`'s
`set_always_on_top` is a no-op under Wayland. Overlayed's own placement code
disables fixed positioning on Wayland for the same reason. Above-game visibility
on KWin therefore requires a **window rule**, which no Flatpak can install for
you.

## 2. What this project does about it

### Automatic, safe mitigations (before GTK/WebKit initialise)

`src-tauri/src/platform.rs::apply_graphics_workarounds`:

| Condition | Action | Why |
| --- | --- | --- |
| NVIDIA + Wayland, `__NV_DISABLE_EXPLICIT_SYNC` unset | set it to `1` | avoids the KWin explicit-sync protocol kill while keeping DMA-BUF (WebKit 324551) |
| `general.disableDmabufRenderer` enabled | set `WEBKIT_DISABLE_DMABUF_RENDERER=1` | the only reliable path for the X11/XWayland GBM failure |

Both are logged. `DLO_NO_GRAPHICS_WORKAROUNDS=1` disables the automation, and
any value you export yourself is respected (nothing is overwritten).

We do **not** ship `WEBKIT_DISABLE_COMPOSITING_MODE=1` by default: on NVIDIA it
is the "last resort" for resize crashes and it costs a lot of rendering quality.

### Honest capability reporting

The settings window shows a badge for every compositor-dependent feature:

| Feature | X11 / XWayland | Wayland (KWin) |
| --- | --- | --- |
| Transparent window | supported | supported (ARGB surface) |
| Click-through (`empty input region`) | supported | best effort — **unverified** on KWin 6 |
| Always on top | supported (`_NET_WM_STATE_ABOVE`) | **not possible for an app**; use a KWin rule |
| Absolute positioning / anchoring | supported | **not possible**; drag once and KWin remembers |
| Global hotkeys | supported (`XGrabKey`) | **not possible** without the GlobalShortcuts portal (not implemented); use the CLI |
| Tray icon | supported | supported via StatusNotifierItem (needs a tray host) |

## 3. KWin window rule for always-on-top (recommended on Wayland)

KWin can keep the overlay above other windows, including fullscreen games in
borderless mode, without any protocol support from the app:

1. **System Settings → Window Management → Window Rules → New…**
2. *Window class* → **Exact match** → `io.github.polar421.DiscordLinuxOverlay`
   (for the native build) or `discord-linux-overlay`.
   If the class is not detected, use **Detect Window Properties** and click the
   overlay.
3. Add the property **Keep above other windows** → **Force** → **Yes**.
4. Optionally add **Focus stealing prevention** → **Force** → **High**, and
   **Skip taskbar** → **Force** → **Yes**.
5. Save the rule.

Notes:

- The rule takes effect for new windows; hide and re-show the overlay.
- On KWin, rules for Wayland windows match on the app id, which is derived from
  the desktop file name. Launching via the installed `.desktop` file is the most
  reliable path.
- `Keep above` does not lift the overlay over a **true exclusive-fullscreen**
  Vulkan/DRM surface. See §6.

## 4. Global hotkeys and the control socket

Wayland does not let normal clients grab global keys. Rather than silently
pretending the setting works, this app exposes a tiny command channel:

```sh
discord-linux-overlay --toggle                  # show / hide the overlay
discord-linux-overlay --toggle-click-through    # interactive <-> click-through
discord-linux-overlay --pin on|off
discord-linux-overlay --settings
discord-linux-overlay --reset-position
discord-linux-overlay --toggle-lock
discord-linux-overlay --status
discord-linux-overlay --quit
```

It talks to the running instance over
`$XDG_RUNTIME_DIR/discord-linux-overlay/control.sock` (fallback
`/tmp/discord-linux-overlay-$UID.sock`).

Bind those commands in **System Settings → Shortcuts → Custom Shortcuts → New →
Command/URL**. That gives you working global hotkeys on KDE Wayland, and it also
works for users without a tray.

## 5. Choice of toolkit and renderer

The overlay is a Tauri v2 application, like upstream. That means GTK3 + WebKitGTK
on Linux. The alternative (Chromium/Electron) would avoid WebKitGTK's GBM bug but
costs several hundred MB of RAM and constant GPU work — the opposite of what a
game overlay should do.

Mitigations are therefore:

1. native Wayland by default (GTK picks Wayland when `WAYLAND_DISPLAY` is set),
2. the explicit-sync workaround above,
3. an opt-in software-rendering switch for X11/XWayland,
4. a deliberately cheap overlay UI: no `backdrop-filter`, no blur, no canvas, no
   continuous animations. Speaking is a border/background colour change, which
   WebKit can composite without re-rasterising the whole surface.

`GDK_BACKEND=x11` remains a supported escape hatch: absolute anchoring and
`_NET_WM_STATE_ABOVE` work there, at the cost of needing
`WEBKIT_DISABLE_DMABUF_RENDERER=1` on NVIDIA.

## 6. Games: what is and is not possible

| Game mode | Overlay visibility |
| --- | --- |
| Windowed | yes, subject to the KWin rule |
| Borderless fullscreen | yes with **Keep above** rule; the compositor still composites the game, so the overlay draws on top |
| Exclusive fullscreen (Vulkan/DRM, `VK_KHR_fullscreen_exclusive`, gamescope fullscreen) | **no** — the game bypasses the compositor entirely. This is a platform rule, not something an overlay can bypass without injecting into the game process (deliberately out of scope; that is what Discord's own overlay does) |
| Proton/DXVK borderless | yes, same as native borderless |

Not implemented here: Vulkan/OpenGL layer injection, `LD_PRELOAD`, or anything
that hooks a game process. They are the only way to draw over exclusive
fullscreen, they are fragile under Proton and anti-cheat, and they conflict with
"do not interfere with games".

Input: the overlay never calls `set_focus()`, is `skipTaskbar` and (when
click-through is on) holds an empty input region, so it cannot take keyboard or
controller input. **Unverified** on a real KWin session — see `TESTING.md`.

## 7. CachyOS / Arch specifics

- `webkit2gtk-4.1` and `gtk3` are the runtime dependencies; the package also
  needs `libayatana-appindicator` (tray) and `hicolor-icon-theme`.
- CachyOS's `nvidia` driver ≥ 545 needs `nvidia_drm.modeset=1` (default on
  modern installs). Without modeset, GBM allocation fails for every GPU app.
- `__NV_DISABLE_EXPLICIT_SYNC=1` is applied only on NVIDIA + Wayland; on AMD/Intel
  nothing is changed.
- Fractional scaling (e.g. 125 %) is honoured for anchored placement because the
  maths is done in physical pixels with the monitor's scale factor, but on
  Wayland the position itself is compositor-controlled.

## 8. Sources

- WebKit bug 261874 — GTK 3 rendering broken with 2.42 on NVIDIA, GBM `EINVAL`
- WebKit bug 280210 — `Error 71 (Protocol error) dispatching to Wayland display`
  on NVIDIA + Wayland, fixed by `WEBKIT_DISABLE_DMABUF_RENDERER=1`
- WebKit bug 303811 — blank WebView on NVIDIA (GBM buffer fail) and loss of
  `backdrop-filter` when DMA-BUF is disabled
- WebKit bug 324551 — KWin Wayland + NVIDIA: `wp_linux_drm_syncobj_v1`
  "explicit sync is used, but no acquire point is set"; `__NV_DISABLE_EXPLICIT_SYNC=1`
  avoids it while DMA-BUF keeps working
- tauri-apps/tauri#9304, #13493 — same failure in Tauri apps; root cause analysis
  (GBM renderable+linear allocation, no SHM fallback)
- Tauri documentation, *Linux Graphics Issues* —
  <https://tauri.app/develop/debug/linux-graphics>
- Overlayed source: `apps/desktop/src-tauri/src/placement.rs`
  (`positioning_supported`, "Fixed placement is not supported on Wayland") and
  `src-tauri/src/commands.rs` (`set_ignore_cursor_events` for pinning)
