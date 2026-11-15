# Installation

The primary target is **CachyOS / Arch Linux** on KDE Plasma 6 Wayland. Nothing
here touches or replaces an installed Overlayed; this is a separate application
with its own id, config directory and desktop entry.

## 1. Runtime dependencies

| Package (Arch) | Why |
| --- | --- |
| `webkit2gtk-4.1` | web view (Tauri on Linux) |
| `gtk3` | window toolkit |
| `libayatana-appindicator` | tray icon (StatusNotifierItem) |
| `librsvg` | SVG icon rendering |
| `hicolor-icon-theme` | application icons |

```sh
sudo pacman -S --needed webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg hicolor-icon-theme
```

## 2. Install options

### A. Arch package (recommended)

```sh
git clone https://github.com/polar421/discord-linux-overlay.git
cd discord-linux-overlay/packaging
makepkg -si
```

Or build without installing, then install the produced package yourself:

```sh
makepkg -f                       # produces discord-linux-overlay-git-r*.pkg.tar.zst
sudo pacman -U discord-linux-overlay-git-r*.pkg.tar.zst
discord-linux-overlay --diagnose
```

Verified on CachyOS on 2026-11-15: `makepkg -f` produced
`discord-linux-overlay-git-r8.6b84d31-1-x86_64.pkg.tar.zst` (4.0 MB, 11.96 MiB
installed, 29 files) and the packaged binary ran `--diagnose` correctly after
extraction. The package sets `options=('!lto')` because the distribution's LTO
flags break `ring`'s assembly objects at link time ("undefined symbol
ring_core_0_17_14__*"). Installation itself was not performed in that session
(no root); `makepkg -si` is the documented path.

Uninstall with `sudo pacman -Rns discord-linux-overlay-git`; your configuration
in `~/.config/discord-linux-overlay` is left untouched.

This installs:

```
/usr/bin/discord-linux-overlay
/usr/share/applications/io.github.polar421.DiscordLinuxOverlay.desktop
/usr/share/metainfo/io.github.polar421.DiscordLinuxOverlay.metainfo.xml
/usr/share/icons/hicolor/{32,64,128,256}/apps/io.github.polar421.DiscordLinuxOverlay.png
/usr/share/licenses/discord-linux-overlay-git/{LICENSE,NOTICE}
```

### B. Manual build

```sh
npm ci && npm run build
npm run tauri build -- --no-bundle
install -Dm755 src-tauri/target/release/discord-linux-overlay ~/.local/bin/discord-linux-overlay
install -Dm644 packaging/io.github.polar421.DiscordLinuxOverlay.desktop \
  ~/.local/share/applications/io.github.polar421.DiscordLinuxOverlay.desktop
```

Adjust `Exec=` in the desktop file if the binary is not on `PATH`.

### C. AppImage

```sh
npm run tauri build -- --bundles appimage
./src-tauri/target/release/bundle/appimage/*.AppImage
```

AppImages keep their own bundling; the app still needs access to your Discord IPC
socket, which is granted by default outside a sandbox.

### D. Flatpak (experimental)

`packaging/io.github.polar421.DiscordLinuxOverlay.yml` is provided as a starting
point, not as a supported distribution yet. Sandboxing complicates the Discord IPC
socket (`$XDG_RUNTIME_DIR/discord-ipc-0`), the tray and always-on-top window rules.
Native packaging is recommended for gaming use. See the comments in the manifest.

## 3. First-run setup

1. Start the **official Discord desktop client**.
2. Launch *Discord Linux Overlay* from your application menu. The overlay appears
   in the bottom-right corner and the settings window opens.
3. **Settings → Discord**: paste your Discord application client id and secret
   ([how to create one](docs/DISCORD_SETUP.md)) and save.
4. Approve the authorization prompt inside Discord.
5. Join a voice channel.

If you want the overlay above games on Wayland, add the KWin rule described in
[LINUX.md §3](LINUX.md#3-kwin-window-rule-for-always-on-top-recommended-on-wayland).

## 4. Autostart

Enable *Settings → General → Launch at login*. That writes:

```
~/.config/autostart/io.github.polar421.DiscordLinuxOverlay.desktop
```

with `Exec=<path to binary> --hidden`. Disable the toggle to remove it. You can
also start manually with `discord-linux-overlay --hidden`.

## 5. Configuration location

```
${XDG_CONFIG_HOME:-~/.config}/discord-linux-overlay/
├── config.json
├── credentials.json    (0600, your Discord application secret)
└── discord-token.json  (0600, cached access token)
```

*Settings → About → Open config folder*, or `discord-linux-overlay --status` to
inspect a running instance.

## 6. Removal

```sh
# Arch package
sudo pacman -Rns discord-linux-overlay-git

# manual install
rm ~/.local/bin/discord-linux-overlay
rm ~/.local/share/applications/io.github.polar421.DiscordLinuxOverlay.desktop
rm ~/.config/autostart/io.github.polar421.DiscordLinuxOverlay.desktop

# settings and cached credentials (optional)
rm -rf ~/.config/discord-linux-overlay
```

Nothing outside these paths is modified, and your existing Overlayed installation
is untouched.
