# Building

## Prerequisites

### Arch / CachyOS

```sh
sudo pacman -S --needed base-devel rust nodejs npm pkgconf \
  webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg patchelf
```

### Debian / Ubuntu

```sh
sudo apt install build-essential curl nodejs npm pkg-config \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf
# Rust (if not packaged):
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Versions used while developing: Rust 1.100, Node 26, npm 11. Tauri v2 requires
Rust 1.77+ and Node 20+.

## Front end

```sh
npm ci            # or npm install
npm run typecheck # tsc --noEmit
npm run build     # typecheck + vite build → dist/
npm run dev       # Vite only, browser mock harness on :1420
```

The Vite build produces two documents: `dist/index.html` (overlay) and
`dist/settings.html` (settings).

## Desktop application

```sh
npm run tauri dev                 # dev build with hot reload
npm run tauri build -- --no-bundle  # release binary, no installers
npm run tauri build               # binary + .deb + AppImage
```

Plain Cargo also works if the front end is already built:

```sh
npm run build
cargo build --release --manifest-path src-tauri/Cargo.toml --features custom-protocol
```

`--features custom-protocol` embeds `dist/` in the binary (this is what production
builds use). Without it the binary expects the dev server.

## Artefacts

| Artefact | Path |
| --- | --- |
| Web assets | `dist/` |
| Release binary | `src-tauri/target/release/discord-linux-overlay` |
| Deb package | `src-tauri/target/release/bundle/deb/*.deb` |
| AppImage | `src-tauri/target/release/bundle/appimage/*.AppImage` |
| App icons | `src-tauri/icons/`, `packaging/icons/` |

Regenerate icons (pure Node, no dependencies):

```sh
npm run icons
```

## Checks

```sh
./scripts/check.sh
```

runs, in order:

1. `cargo test -p discord-overlay-core --features http` (unit + fake-Discord
   integration tests),
2. `npm run typecheck`,
3. `npm run build`,
4. `cargo build --manifest-path src-tauri/Cargo.toml` if the GTK/WebKit dev
   headers are installed (skipped with a message otherwise).

## Packaging

- **Arch**: `packaging/PKGBUILD` (`makepkg -si`).
- **Flatpak**: `packaging/io.github.polar421.DiscordLinuxOverlay.yml` — see the
  caveats in that file; native packaging is recommended for RPC socket access.
- **AppImage**: produced by `npm run tauri build -- --bundles appimage`.
- **Desktop entry / AppStream**: `packaging/*.desktop`, `packaging/*.metainfo.xml`,
  installed to `/usr/share/applications` and `/usr/share/metainfo`.

## CI

`.github/workflows/ci.yml` runs the core tests, the front-end typecheck/build and
a full Linux `cargo build` with the WebKit/GTK development packages on Ubuntu
24.04. That job is what verifies the Tauri shell compiles; it could not run in the
headless container this code was authored in (no root, no GTK/WebKit headers).
