# Optimization changelog

Chronological record of the memory/CPU work. Every number comes from
`scripts/benchmark.sh` on the target machine (CachyOS, KDE Plasma 6 Wayland,
RTX 3060); see [PERFORMANCE.md](PERFORMANCE.md) for methodology and full runs.

## 1. Accurate baseline first

The previous "785 MB" figure was summed RSS with two web views and no PSS
breakdown. Added `scripts/benchmark.sh` and measured the real tree before
changing code:

- **376.1 MiB PSS / 734.0 MiB RSS idle**
- main 104 MiB, overlay web process 119 MiB, hidden settings web process 114 MiB,
  network process 30 MiB.

This established that the hidden settings webview was a full ~114 MiB, not a
minor overhead, and that RSS overstated the footprint by ~2×.

## 2. Lazy settings window

`tauri.conf.json` no longer declares the settings window. `windows.rs` builds it
with `WebviewWindowBuilder` on first use, and `close_settings` closes (destroys)
it instead of hiding it. The configuration lives on disk, so reopening restores
state from `get_config`.

- Idle: 376.1 → **302 MiB PSS**, RSS 734 → 497 MiB.
- Settings still opened correctly, and configuration survived.

## 3. WebKit tuning experiments (rejected as insufficient)

Measured with the single lazy overlay: DMA-BUF renderer off, compositing off,
`MALLOC_ARENA_MAX=2`, and combinations.

- Best result: `WEBKIT_DISABLE_DMABUF_RENDERER=1` → **265 MiB PSS / 398 MiB
  RSS**; the web process dropped from ~155 to ~96 MiB but the main process grew.
- `MALLOC_ARENA_MAX` and compositing-off added nothing; compositing-off alone
  was worse (358 MiB).
- Conclusion: the remaining cost is WebKit's process model (UI + web + network
  processes), not a tunable cache. 265 MiB is the practical WebKit floor and
  still 65 MiB above the target.

## 4. Native GTK overlay

Replaced the overlay webview with a native GTK3/cairo renderer
(`src-tauri/src/native_overlay.rs`) while keeping Tauri for tray, shortcuts,
control socket, Discord worker and the on-demand settings window.

- Idle: 302 → **38.1 MiB PSS / 65.0 MiB RSS**, single process.
- No WebKit web or network process in the idle tree; no JavaScript heap; no GPU
  context; nine threads.
- Verified transparency (alpha 0 outside the pills), frameless window, all three
  layouts, avatars/placeholder, speaking ring, mute/deafen/bot icons, empty
  state, CLI controls and config persistence.
- Fixed KWin adding server-side decorations to undecorated GTK3 Wayland windows
  by installing an empty CSD titlebar, the same workaround tao uses.
- Kept the click-through behaviour by installing an empty GTK input region when
  `clickThrough` is enabled.

## 5. Destroy and release the settings webview

Closing Settings used to hide the window (and keep both WebKit helpers alive
forever). Now the window is destroyed, and after the last webview is gone the
app sends `SIGTERM` then `SIGKILL` to its own WebKit descendants, guarded by a
generation counter so a quick reopen is never killed. It then calls
`malloc_trim(0)`.

- Settings open: **313–315 MiB PSS / 511–513 MiB RSS**.
- Settings closed: **152–155 MiB PSS / 192–195 MiB RSS**, flat across five
  open/close cycles (no leak).
- Before the change the hidden window kept the tree at ~313 MiB indefinitely.

## 6. Event, CPU and cache discipline

- Overlay repaints only on Discord/config/status events or pointer input; no
  animation loop, no polling, no per-frame timers.
- Avatar cache bounded to 48 entries with LRU eviction, max three concurrent
  downloads, 2 MiB per image, failed URLs not retried in a loop.
- Overlay state history already capped at 50 entries; logs at 250 lines.
- Result: idle CPU **0.00%** in final samples (previously 0.0–0.4%), startup
  **294 ms** median versus 536 ms before.

## 7. Test and documentation infrastructure

- `scripts/benchmark.sh`: whole-tree PSS/RSS/private/shared/CPU/startup/GPU,
  median of five samples, three runs, idle and settings-cycle scenarios,
  optional 5-minute long mode.
- `scripts/perf-check.sh`: fast regression gate (PSS ≤ 200 MiB, CPU ≤ 0.8%,
  startup ≤ 1500 ms by default).
- `--diagnose` now reports the native renderer, decoration state, RGBA visual,
  avatar cache size and Wayland positioning behaviour.
- Added `--close-settings` so the settings lifecycle can be scripted and
  benchmarked.

## 8. What was deliberately not done

- **Did not keep WebKit for the overlay with DMA-BUF disabled.** 265 MiB is
  still over target and it changes the rendering path on NVIDIA.
- **Did not build a native settings UI.** It would bring settings-open under
  200 MiB too, but it is a large form/preview rewrite and the idle target is
  already met; documented as the next big opportunity.
- **Did not spawn settings in a separate process.** It would restore 38 MiB
  after close but requires proxying every settings command over IPC, and
  settings-open would still exceed 200 MiB.
- **Did not disable hardware acceleration globally or change driver/KDE
  settings.** The native overlay does not use a GPU context; the settings
  webview keeps the existing opt-in DMA-BUF workaround.
