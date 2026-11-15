# Milestones

Status of the development roadmap. "Done" means implemented **and** verified in
the headless container; anything needing a desktop is marked accordingly.

| # | Milestone | State |
| --- | --- | --- |
| 1 | **Upstream research** — source inspection, architecture report, rendering investigation, licensing | ✅ done — [UPSTREAM_RESEARCH.md](UPSTREAM_RESEARCH.md), [LINUX.md](../LINUX.md) |
| 2 | **Repository foundation** — structure, workspace, build system, app shell | ✅ code done, ⚠️ Tauri shell compiled only by CI (no GTK headers locally) |
| 3 | **Core Discord integration** — connection, channel, participants, speaking, avatars, status | ✅ done — 67 automated tests pass, including end-to-end fake-Discord sessions; ⚠️ not verified against a real Discord client |
| 4 | **Overlay implementation** — transparent window, move, resize, persistence, always-on-top, click-through | ✅ code done, ⚠️ unverified on a desktop (needs compositor) |
| 5 | **Linux integration** — native Wayland, KWin, NVIDIA, XWayland fallback, multi-monitor | ⚠️ mitigations and documentation done; behaviour unverified on the target machine |
| 6 | **Settings and UX** — settings UI, persistence, tray, shortcuts | ✅ UI typechecked + browser-previewed; ⚠️ runtime unverified |
| 7 | **Gaming compatibility** — Steam, Proton, Vulkan, OpenGL, borderless, input | 📄 designed and documented; ❌ untested (no games/GPU here) |
| 8 | **Optimization and packaging** — profiling, CPU/GPU, Arch, Flatpak, AppImage | ⚠️ design done (event-driven, no idle repaint); measurements and packaging execution require the target machine |
| 9 | **Final verification** — functional testing, docs, release prep, repo completion | ⚠️ docs and release scaffolding done; functional testing is the user's step (checklist in [TESTING.md](../TESTING.md)) |

## Next steps on the target machine

1. Build (`npm ci && npm run build && npm run tauri build -- --no-bundle`).
2. Add Discord credentials, join voice, confirm participants/speaking.
3. Add the KWin *Keep above* rule and test over a borderless game.
4. Work through [TESTING.md](../TESTING.md) §A–F and fill in the results table.
5. Report anything that fails; the known weak spots are click-through on KWin
   Wayland, tray behaviour in a Wayland session, and WebKitGTK rendering with
   driver 620.70.06.
