# Memory analysis

This document explains where the memory went, what was measured, and why the
overlay no longer uses WebKit. All numbers are aggregate PSS for the complete
process tree unless stated otherwise. Methodology and raw runs are in
[PERFORMANCE.md](PERFORMANCE.md).

## 1. Where the original 376 MiB went

The original build created two Tauri web views at startup: the overlay and a
hidden settings window. Idle snapshot:

| Component | PSS | Private dirty | What it contains |
| --- | --- | --- | --- |
| Main process | ~104 MiB | ~58 MiB | GTK, Tauri/tao, WebKit UI process, fontconfig/pango, Mesa/NVIDIA libraries, tray, Discord client state |
| Overlay `WebKitWebProcess` | ~119 MiB | ~66 MiB | JavaScriptCore heap, DOM, React bundle, Skia compositor, shared WebKit libraries |
| Hidden settings `WebKitWebProcess` | ~114 MiB | ~65 MiB | same engine baseline plus the settings React bundle |
| `WebKitNetworkProcess` | ~30 MiB | ~21 MiB | HTTP cache, TLS, network stack |
| **Total** | **376 MiB** | **209 MiB** | |

Important observations:

- The two web processes were nearly identical in size even though the settings
  window was hidden. WebKitGTK does not release a suspended page while the
  window exists; the hidden UI cost ~114 MiB PSS by itself.
- The main process held a large share of shared WebKit/GTK library pages. PSS
  splits those pages between processes, so the main process alone is not a
  meaningful measure.
- The overlay's own content is tiny: a 320×440 list with a few avatars. A blank
  WebKit web view costs almost as much as the populated one.

Summed RSS (734 MiB) was higher than PSS because the same libraries and
surfaces are mapped in several processes. PSS is the right number for "how much
physical memory does this application cost".

## 2. What each experiment actually saved

| Change | Idle PSS | Idle RSS | Verdict |
| --- | --- | --- | --- |
| Original (overlay + hidden settings) | 376.1 | 734.0 | baseline |
| Lazy settings: no settings webview until opened | 302 | 497 | shipped |
| `WEBKIT_DISABLE_DMABUF_RENDERER=1` | 265.8 | 398.9 | not enough; changes GPU path |
| `MALLOC_ARENA_MAX=2` | 307.2 | 503.7 | no gain over lazy settings |
| `WEBKIT_DISABLE_COMPOSITING_MODE=1` | 357.9 | 554.1 | **worse**; rejected |
| DMA-BUF off + arena limit | 265.4 | 398.4 | no further gain |
| DMA-BUF off + compositing off | 265.2 | 398.2 | no further gain |
| Native GTK overlay | **38.2** | **65.0** | shipped |

The best obtainable WebKit configuration (265 MiB) still exceeded the 200 MiB
target by 65 MiB, and it did so while disabling DMA-BUF rendering. Even the two
helper processes alone (web + network) cost ~126 MiB. A single-webview
architecture would not help: closing the settings webview only gets to 302 MiB,
and the remaining overlay webview still needs ~96–155 MiB plus the UI process.

A bare transparent GTK window on the same compositor measured **17.9 MiB PSS /
36.2 MiB RSS**, showing that the target was achievable once WebKit was removed
from the idle path.

## 3. The native architecture

The overlay is now a single GTK3 `Window` with one `DrawingArea`, painted with
cairo; text uses Pango, avatars use gdk-pixbuf. Tauri remains for:

- the tray icon and menu (libayatana-appindicator),
- global shortcut registration,
- the control socket and CLI,
- the Discord RPC worker,
- the on-demand settings webview.

Idle process tree: **one process**, 9–11 threads, 38.1 MiB PSS / 64.6 MiB RSS.
There is no JavaScript engine, no DOM, no network process and no GPU context in
the idle state. Private dirty is 18.4 MiB; shared library pages account for the
rest (GTK, Pango, fontconfig, NVIDIA/Mesa, appindicator).

What the native renderer implements (feature parity with the web overlay):

- transparent, frameless, always-on-top window, 320×440 default and resizable;
- stack / row / grid layouts, compact avatar mode, scale, opacity, gap, radius,
  background on/off, max name length, own-user and speaking-only filters;
- avatars with circular clipping and an initial-letter placeholder, bounded
  cache (48 entries, max three concurrent downloads);
- speaking ring, mute/deafen/bot icons, compact badges;
- toolbar with click-through, lock, reset and settings actions, drag-to-move;
- empty/connection-status row driven by the same `Status` enum;
- monitor-aware placement and manual bounds persistence (Wayland position is
  compositor-managed, exactly as before).

The window uses an empty CSD titlebar on Wayland. Without it, KWin adds
server-side decorations to undecorated GTK3 windows (GTK3 does not implement
`xdg-decoration`). This is the same workaround tao uses for Tauri's own windows.

## 4. Settings window lifecycle

The settings UI remains WebKit on purpose: it is a complex form with preview,
tabs and validation, and it is only needed while open. It is no longer declared
in `tauri.conf.json`; the first `--settings`/tray action builds it, and closing
it destroys the window.

Measured cycle (PSS / RSS, MiB):

| State | PSS | RSS | Processes |
| --- | --- | --- | --- |
| Idle before first Settings | 38.2 | 65.0 | 1 |
| Settings open | 313–315 | 511–513 | 3 (main + web + network) |
| Settings closed | 152–155 | 192–195 | 1 |

After the window is destroyed, WebKitGTK leaves its web and network processes
in a suspended pool. The app terminates **only its own descendants** whose
executable path contains `WebKit`, after checking that no newer Settings window
was opened, then calls `malloc_trim(0)`. Reopening Settings starts fresh
helpers. Five open/close cycles were stable at ~314 MiB open and ~156 MiB
closed; after the trim change the closed state is ~152–155 MiB.

The residual 153 MiB is the main process after WebKit's UI libraries and font/
cache pages have been touched; those mappings cannot be unloaded without
restarting the process. It is still below the 200 MiB target. A separate
settings helper process would return to 38 MiB after closing, but it would add
IPC/proxy complexity for every settings command and is not required by the
target.

## 5. Rust and event-path analysis

- `OverlayState` is small (participants + at most 50 history entries). Each
  Discord event replaces the snapshot and emits it to listeners; the native
  overlay clones the snapshot once per event to paint. At idle there are no
  events.
- The overlay view keeps no unbounded collections. Avatar cache: 48 entries,
  LRU eviction; failed URLs are not retried in a loop; downloads are capped at
  three concurrent threads and 2 MiB per image.
- Logs are capped at 250 lines. The control socket handles one request per
  connection; no queue accumulates.
- The Discord client thread remains event-driven with a resync watchdog; no new
  polling was added by this work.
- Avatar decoding happens on the GTK main thread after a small byte buffer is
  handed over from the download thread; the pixbuf cache is bounded and the
  buffer is dropped immediately.
- No Rust-side leaks were observed: repeated show/hide, lock and click-through
  toggles, and repeated Settings open/close leave PSS flat after the first
  cycle.

## 6. Frontend memory

The React overlay is no longer instantiated at runtime. Its JS bundle, DOM,
image cache and event listeners do not exist in the idle process. The browser
preview (`npm run dev`) still uses it for visual development, but that is not
part of the desktop application.

The settings React app runs only inside the on-demand WebKit webview. While
open it accounts for the web process's 150–167 MiB; after close the process is
terminated. No background timers from the overlay remain (the simulated
speaker rotation exists only in the browser mock under `src/lib/mock.ts`).

## 7. Stability

The 30-minute sampler (`./scripts/stability.sh 30`: minute-by-minute PSS/RSS,
10 show/hide toggles at minute 5, Settings open/close at minutes 10 and 20)
verifies:

- initial idle was flat at 37.86–37.87 MiB PSS and 65,796 kB RSS for five
  minutes;
- ten show/hide toggles left it at 37.85–38.04 MiB (no growth);
- Settings cycles were 315.4 and 316.6 MiB open, then returned to 153.1 and
  152.9 MiB just after close and 149.8–150.6 MiB once the helpers were gone;
- the post-Settings floor stayed flat within 35 kB for the last ten minutes and
  no WebKit children survived (`children=0`);
- 108 CPU jiffies over 1800 s, i.e. **0.06% of one core** including both
  Settings cycles and the toggle burst.

Full phase-by-phase table: [PERFORMANCE.md](PERFORMANCE.md#long-running-stability). Discord was not
running on this machine during the run, so voice-participant churn (speaking
start/stop, joins/leaves) could not be exercised against a real client; the
drawing code was verified with a temporary synthetic state instead (screenshots
in `docs/screenshots/native-overlay-participants.png`).

## 8. Remaining opportunities

1. **Native settings UI** (GTK forms): would remove WebKit entirely and keep
   the whole application near 40–60 MiB even with settings open. Largest work
   item; not required for the <200 MiB idle target.
2. **Separate settings helper process**: restores 38 MiB after close without a
   UI rewrite, but opening settings still exceeds 200 MiB and command plumbing
   becomes remote.
3. **Shared image decode** for avatars: decoding at 2× display size and scaling
   in cairo costs a little CPU; a size-bucketed cache would help Discord-heavy
   sessions, at the cost of more memory per size.
4. **Smarter WebKit teardown**: `malloc_trim` recovers ~3 MiB; glib/WebKit
   caches keep the rest. Process isolation is the only way to fully reclaim it.
