# Performance and memory

Measured on the target machine, release builds only. The primary number is
**aggregate PSS of the whole process tree** (main process, every WebKit helper,
every child), because summed RSS counts shared libraries and shared memory once
per process. RSS is reported too, but it overstates the footprint.

## Test environment

| Item | Value |
| --- | --- |
| OS / kernel | CachyOS, Linux 7.3.3-1-cachyos-bore |
| Desktop | KDE Plasma 6, Wayland (KWin) |
| CPU | Intel Core i5-9600K (6 cores) |
| GPU / driver | NVIDIA RTX 3060 12 GB, driver 620.70.06 |
| GTK / WebKitGTK | GTK 3.24.52, WebKitGTK 2.54.2 |
| Build | `cargo build --release --features custom-protocol` |
| Display | native Wayland (`GDK_BACKEND` unset) |

Idle means: overlay visible, settings closed, Discord not connected (no
credentials were configured on this machine, so no voice session was active).
No screenshots, animations or input were occurring during sampling.

## How the numbers are produced

`scripts/benchmark.sh`:

```sh
./scripts/benchmark.sh --repeat 3 --duration 15
./scripts/benchmark.sh --settings --repeat 3 --duration 15   # + settings open/closed
./scripts/benchmark.sh --long                                # 5 min stability
```

For each run the script starts the release binary, waits until `--diagnose`
reports the overlay visible, waits 3 s, then takes five samples over the
duration. It reports the **median** PSS, RSS, private dirty/clean and shared
memory from `/proc/<pid>/smaps_rollup` summed over the process tree. CPU is the
tree's `utime + stime` delta over 5 s divided by one core. It also prints the
startup time (launch → overlay visible) and global `nvidia-smi` telemetry.

Settings sampling uses the new `--settings` / `--close-settings` control
commands. Closing Settings is followed by a 4 s wait while the WebKit helpers
are terminated.

## Headline results

| Scenario | Before (WebKit overlay) | After (native overlay) | Change |
| --- | --- | --- | --- |
| **Idle overlay, settings closed, PSS** | **376.1 MiB** (376.0–380.4) | **38.1 MiB** (38.1–38.4) | **−338.0 MiB (−89.9%)** |
| Idle overlay, RSS | 734.0 MiB (733.5–737.9) | 65.0 MiB (64.6–65.2) | −669.0 MiB (−91.1%) |
| Settings open, PSS | 390.9 MiB (387.7–395.3) | 313.7 MiB (313.1–315.2) | −77.2 MiB (−19.7%) |
| Settings open, RSS | 751.0 MiB (750.4–759.2) | 511.6 MiB (511.0–513.3) | −239.4 MiB (−31.9%) |
| Settings just closed, PSS | not measured (window was hidden, not destroyed) | 152.4–155.2 MiB | under 200 MiB |
| Idle CPU (% of one core) | 0.00–0.40% | 0.00% | at or below target |
| Startup to overlay visible | 536 ms median (317–788 ms) | 294 ms median (293–308 ms) | −45% |

The settings-open row is above 200 MiB because the settings panel is still a
WebKit web view. Section "The 200 MiB floor" below explains the measured floor
and what a native settings UI would require.

## Process tree before and after

Before (one hidden settings web view plus the overlay web view, idle):

| Process | PSS | RSS |
| --- | --- | --- |
| main (Tauri/GTK/WebKit UI, tray, Discord worker) | ~104 MiB | ~202 MiB |
| `WebKitWebProcess` (overlay) | ~119 MiB | ~239 MiB |
| `WebKitWebProcess` (hidden settings) | ~114 MiB | ~233 MiB |
| `WebKitNetworkProcess` | ~30 MiB | ~66 MiB |
| **total** | **376.1 MiB** | **734.0 MiB** |

After (single native GTK process, idle):

| Process | PSS | RSS |
| --- | --- | --- |
| `discord-linux-overlay` (native GTK overlay + tray + Discord worker) | 38.1 MiB | 64.6 MiB |
| **total** | **38.1 MiB** | **64.6 MiB** |

There are **no helper processes** in the idle tree. When Settings is opened,
the WebKit helpers appear (about 31 MiB network + 150–167 MiB web process);
when it is closed they are terminated again.

## Per-scenario detail

Idle, before (three runs):

```
run 1  PSS 376.1 MiB  RSS 734.0 MiB  priv_dirty 209.1 MiB  CPU 0.00%
run 2  PSS 376.0 MiB  RSS 733.5 MiB  priv_dirty 210.0 MiB  CPU 0.00%
run 3  PSS 380.4 MiB  RSS 737.9 MiB  priv_dirty 215.0 MiB  CPU 0.20%
```

Idle, after (three runs, final binary):

```
run 1  PSS 38.1 MiB  RSS 64.6 MiB  priv_dirty 18.4 MiB  CPU 0.00%  startup 294 ms
run 2  PSS 38.4 MiB  RSS 65.2 MiB  priv_dirty 18.4 MiB  CPU 0.00%  startup 294 ms
run 3  PSS 38.4 MiB  RSS 65.0 MiB  priv_dirty 18.4 MiB  CPU 0.00%  startup 294 ms
```

Settings cycle, after (three runs, same binary, includes `malloc_trim`):

```
run 1  idle 38.1 / 64.6   settings 313.1 / 511.0   closed 152.4 / 192.3
run 2  idle 38.4 / 65.2   settings 313.7 / 511.6   closed 153.4 / 193.4
run 3  idle 38.4 / 65.0   settings 315.2 / 513.3   closed 155.2 / 195.0
```

Five consecutive open/close cycles measured at ~314–319 MiB open and
~156–157 MiB closed, with no growth after the first cycle. The WebKit UI library
mappings stay resident in the main process after the first use of Settings; that
is why the post-Settings floor is ~152–155 MiB rather than the pre-Settings
38 MiB.

## CPU

- Before: 0.00–0.40% of one core at idle; the hidden settings webview ran no
  application timers, but WebKit still had housekeeping threads.
- After: 0.00% in all final idle samples (occasional 0.20% sample includes
  process startup/event-loop noise). Drawing only happens on state changes
  (Discord events, config changes, pointer movement); there is no animation
  loop and no polling in the overlay.
- The 30-minute stability run records cumulative CPU ticks once per minute; the
  derived average is reported in the stability section below.

## Long-running stability

The reproducible 30-minute sampler (`./scripts/stability.sh 30`) ran the final
release binary on the target desktop, sampling once per minute (whole tree,
PSS/RSS/child count/CPU ticks). At minute 5 it performed ten `--toggle`
show/hide cycles; at minutes 10 and 20 it opened Settings for 3 s, then closed
it.

| Phase | Samples | PSS | RSS | Children |
| --- | --- | --- | --- | --- |
| Initial idle | minutes 0–4 | 38,768–38,778 kB (37.86–37.87 MiB) | 65,796 kB | 0 |
| After 10 show/hide toggles | minutes 5–9 | 38,761–38,954 kB (37.85–38.04 MiB) | 65,972 kB | 0 |
| Settings cycle 1, open | minute 10 | 322,985 kB (315.4 MiB) | 525,080 kB | 3 (main + web + network) |
| Settings cycle 1, just closed | minute 10 | 156,760 kB (153.1 MiB) | 197,372 kB | 1 |
| Post-Settings 1 idle | minutes 11–19 | 153,372–153,400 kB (149.8 MiB) | 194,012 kB | 0 |
| Settings cycle 2, open | minute 20 | 324,168 kB (316.6 MiB) | 526,356 kB | 3 |
| Settings cycle 2, just closed | minute 20 | 156,600 kB (152.9 MiB) | 197,212 kB | 1 |
| Post-Settings 2 idle | minutes 21–30 | 154,149–154,184 kB (150.5–150.6 MiB) | 194,764 kB | 0 |

There was no upward trend: the initial idle varied by 10 kB, the post-toggle
state by 193 kB, and after the second Settings cycle the floor settled at
150.5 MiB and stayed within 35 kB for ten minutes. The small increase between
the first and second post-Settings floors (149.8 → 150.5 MiB) is WebKit UI
library/cache pages touched by the second window, not a leak; it remained
completely flat afterwards. No WebKit children survived either close
(`children=0` from minute 11 onwards).

The CPU counters went from 12 ticks at minute 0 to 120 at minute 30, i.e. 108
jiffies over 1800 s = **0.06% of one core on average**, including both Settings
cycles and the toggle burst. The initial idle phase averaged 0.04%.

Raw CSV format (one line per sample):

```
minute,event,pss_kb,rss_kb,children,cpu_ticks
0,idle,38777,65796,0,12
...
10,settings-open,322985,525080,3,0
10,settings-closed,156760,197372,1,55
...
30,idle,154149,194764,0,120
DONE
```

The session ended with `--quit`, no `discord-linux-overlay` process remained,
and the sampler and application logs were clean.

## GPU

`nvidia-smi pmon` does not list the application at all while idle: the native
overlay is drawn with cairo into a GTK surface and composited by KWin, so it
does not own a GPU context or GPU buffers. There is no WebKit web process to
drive Skia/DMA-BUF. Global GPU utilisation during sampling was dominated by
`kwin_wayland`/`Xwayland` and other desktop clients, not by this app.

No continuous redraw is scheduled: the window is only queued for drawing when
participants, config or status change, or when the pointer interacts with it.
Speaking indicators therefore update on the Discord event, not on a frame timer.

## The 200 MiB floor

Measured floors on this machine (idle, settings closed):

| Architecture | Aggregate PSS | RSS | Note |
| --- | --- | --- | --- |
| Original: overlay + hidden settings web views | 376 MiB | 734 MiB | 2 web processes + network |
| Lazy settings only: single WebKit overlay | 302 MiB | 497 MiB | settings no longer preloaded |
| WebKit overlay + `WEBKIT_DISABLE_DMABUF_RENDERER=1` | **265 MiB** | 398 MiB | best WebKit configuration found |
| + `MALLOC_ARENA_MAX=2` | 265 MiB | 398 MiB | no further gain |
| + `WEBKIT_DISABLE_COMPOSITING_MODE=1` | 265 MiB | 398 MiB | no further gain |
| Bare transparent GTK window (prototype) | **17.9 MiB** | 36.2 MiB | no app logic |
| Native overlay (this build) | **38.1 MiB** | 65 MiB | full overlay, tray, Discord client |

The WebKit floor is structural: even a blank 320×440 web view needs a UI
process (~110 MiB PSS), a web process (~95–155 MiB) and a network process
(~30 MiB). No documented WebKitGTK setting removes those processes. Reaching
<200 MiB therefore required removing WebKit from the overlay, not tuning it.

Settings opened: the native app (≈113 MiB after WebKit initialisation) + web
process (≈167 MiB) + network process (≈31 MiB) = **≈313 MiB PSS**. The web
process alone is already close to the whole 200 MiB budget. Achieving <200 MiB
with Settings open would require a native settings UI (GTK widgets) or a
separate short-lived settings process. The latter would restore the 38 MiB idle
floor after closing, but opening Settings would still exceed 200 MiB while
WebKit is running.

## Reproducing and guarding the result

```sh
# full report
./scripts/benchmark.sh --repeat 3 --duration 15

# 30-minute stability run (10 toggles at minute 5, Settings at 10 and 20)
./scripts/stability.sh 30

# fast regression gate (exit 1 if idle PSS > 200 MiB, CPU > 0.8%, startup > 1500 ms)
./scripts/perf-check.sh
```

The regression gate uses the same script and thresholds can be overridden:

```sh
PERF_MAX_PSS_MIB=200 PERF_MAX_CPU=0.8 PERF_MAX_STARTUP_MS=1500 ./scripts/perf-check.sh
```

## Caveats

- Startup time is measured by polling `--diagnose`; it includes spawning the
  short-lived CLI process, so it is an upper bound for both architectures.
- PSS sums are approximate: shared pages are attributed proportionally and can
  round differently between samples.
- Discord was not connected during these measurements. Participant events add
  a few hundred KB of avatar textures (bounded cache) and no polling.
- The long-running stability numbers are produced by the 30-minute sampler and
  are recorded in [MEMORY_ANALYSIS.md](MEMORY_ANALYSIS.md#stability).
