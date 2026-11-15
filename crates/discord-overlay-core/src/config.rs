//! Persistent settings.
//!
//! Settings live in a single JSON file (`$XDG_CONFIG_HOME/discord-linux-overlay/config.json`).
//! The model is deliberately forgiving: every field has a default, unknown keys
//! are ignored and out-of-range values are clamped, so an older or newer config
//! file can never prevent the overlay from starting.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::geometry::Anchor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// Vertical list of avatar + username pills.
    Stack,
    /// Single horizontal strip of avatars.
    Row,
    /// Wrapping grid of avatars.
    Grid,
}

impl Default for Layout {
    fn default() -> Self {
        Layout::Stack
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Background {
    /// Translucent dark pill behind each row.
    Solid,
    /// Avatars and text only, no background.
    None,
}

impl Default for Background {
    fn default() -> Self {
        Background::Solid
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementMode {
    /// The user drags the overlay; the compositor position is remembered.
    Manual,
    /// The overlay is anchored to a monitor corner with a fixed offset.
    Anchored,
}

impl Default for PlacementMode {
    fn default() -> Self {
        PlacementMode::Anchored
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GeneralConfig {
    pub launch_at_login: bool,
    pub start_hidden: bool,
    pub show_overlay_on_join: bool,
    pub hide_overlay_on_leave: bool,
    pub show_join_history: bool,
    pub show_connection_status: bool,
    /// Opt-in fallback for NVIDIA/X11 setups where WebKitGTK cannot allocate a
    /// GBM buffer ("Failed to create GBM buffer: Invalid argument").
    pub disable_dmabuf_renderer: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            start_hidden: false,
            show_overlay_on_join: true,
            hide_overlay_on_leave: false,
            show_join_history: false,
            show_connection_status: true,
            disable_dmabuf_renderer: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppearanceConfig {
    /// 10 - 100, applied to the whole overlay.
    pub opacity: u16,
    /// 50 - 200, scales avatars and text.
    pub scale: u16,
    /// Logical avatar size in pixels (24 - 96).
    pub avatar_size: u16,
    pub show_names: bool,
    pub show_own_user: bool,
    pub show_only_speaking: bool,
    pub layout: Layout,
    /// Corner radius in pixels (0 - 32).
    pub corner_radius: u16,
    /// Gap between rows in pixels (0 - 24).
    pub gap: u16,
    pub background: Background,
    /// Maximum rendered username length before truncation.
    pub max_name_length: u16,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            opacity: 100,
            scale: 100,
            avatar_size: 40,
            show_names: true,
            show_own_user: true,
            show_only_speaking: false,
            layout: Layout::Stack,
            corner_radius: 12,
            gap: 6,
            background: Background::Solid,
            max_name_length: 24,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BehaviorConfig {
    pub always_on_top: bool,
    pub click_through: bool,
    pub lock_position: bool,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            always_on_top: true,
            click_through: true,
            lock_position: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlacementConfig {
    pub mode: PlacementMode,
    /// Monitor name as reported by the windowing layer; `None` = primary.
    pub monitor: Option<String>,
    pub anchor: Anchor,
    pub offset_x: f64,
    pub offset_y: f64,
    /// Logical content size of the overlay window.
    pub width: f64,
    pub height: f64,
    /// Last manual position (physical pixels).
    pub x: Option<i32>,
    pub y: Option<i32>,
}

impl Default for PlacementConfig {
    fn default() -> Self {
        Self {
            mode: PlacementMode::Anchored,
            monitor: None,
            anchor: Anchor::BottomRight,
            offset_x: 24.0,
            offset_y: 24.0,
            width: 320.0,
            height: 440.0,
            x: None,
            y: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HotkeysConfig {
    /// Accelerator strings in Tauri syntax, e.g. `CommandOrControl+Shift+G`.
    pub toggle_overlay: Option<String>,
    pub toggle_click_through: Option<String>,
    pub open_settings: Option<String>,
    pub reset_position: Option<String>,
}

impl Default for HotkeysConfig {
    fn default() -> Self {
        Self {
            toggle_overlay: Some("CommandOrControl+Shift+G".to_string()),
            toggle_click_through: Some("CommandOrControl+Shift+H".to_string()),
            open_settings: None,
            reset_position: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub general: GeneralConfig,
    pub appearance: AppearanceConfig,
    pub behavior: BehaviorConfig,
    pub placement: PlacementConfig,
    pub hotkeys: HotkeysConfig,
}

impl Config {
    /// Clamp everything into a range the UI and window layer can handle.
    pub fn normalized(mut self) -> Self {
        self.appearance.opacity = self.appearance.opacity.clamp(10, 100);
        self.appearance.scale = self.appearance.scale.clamp(50, 200);
        self.appearance.avatar_size = self.appearance.avatar_size.clamp(24, 96);
        self.appearance.corner_radius = self.appearance.corner_radius.clamp(0, 32);
        self.appearance.gap = self.appearance.gap.clamp(0, 24);
        self.appearance.max_name_length = self.appearance.max_name_length.clamp(4, 64);

        self.placement.offset_x = sane_float(self.placement.offset_x, 0.0, 10_000.0, 24.0);
        self.placement.offset_y = sane_float(self.placement.offset_y, 0.0, 10_000.0, 24.0);
        self.placement.width = sane_float(self.placement.width, 120.0, 3840.0, 320.0);
        self.placement.height = sane_float(self.placement.height, 80.0, 2160.0, 440.0);

        self
    }

    /// Deep-merge a partial JSON update (what the settings UI sends) into the
    /// current configuration.
    pub fn apply_patch(&mut self, patch: &Value) -> Result<()> {
        let mut value = serde_json::to_value(&*self)?;
        merge_json(&mut value, patch);
        let updated: Config = serde_json::from_value(value)?;
        *self = updated.normalized();
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

fn sane_float(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn merge_json(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (key, patch_value) in patch_map {
                match base_map.get_mut(key) {
                    Some(slot) => merge_json(slot, patch_value),
                    None => {
                        base_map.insert(key.clone(), patch_value.clone());
                    }
                }
            }
        }
        (slot, patch_value) => *slot = patch_value.clone(),
    }
}

/// Reads and writes [`Config`] on disk.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the config, falling back to defaults for missing or corrupt files.
    pub fn load(&self) -> Config {
        match fs::read_to_string(&self.path) {
            Ok(contents) => match serde_json::from_str::<Config>(&contents) {
                Ok(config) => config.normalized(),
                Err(error) => {
                    log::warn!(
                        "config at {} is invalid ({error}); using defaults",
                        self.path.display()
                    );
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    /// Write the config atomically (temp file + rename) with `0600` permissions.
    pub fn save(&self, config: &Config) -> Result<()> {
        write_private_json(&self.path, &serde_json::to_value(config)?)
    }

    /// Apply a partial update and persist it.
    pub fn patch(&self, patch: &Value) -> Result<Config> {
        let mut config = self.load();
        config.apply_patch(patch)?;
        self.save(&config)?;
        Ok(config)
    }
}

/// Write JSON to `path` atomically, readable only by the current user.
pub fn write_private_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            Error::Settings(format!("could not create {}: {error}", parent.display()))
        })?;
    }

    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value)?;

    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| {
                Error::Settings(format!("could not write {}: {error}", temporary.display()))
            })?;
        file.set_permissions(fs::Permissions::from_mode(0o600)).ok();
        file.write_all(&bytes)?;
        file.sync_all().ok();
    }

    fs::rename(&temporary, path).map_err(|error| {
        Error::Settings(format!("could not replace {}: {error}", path.display()))
    })?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).ok();
    Ok(())
}

/// The config directory used by the application.
///
/// Follows `XDG_CONFIG_HOME`, with `$HOME/.config` as the documented fallback.
pub fn config_dir() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("discord-linux-overlay"),
        None => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(".config/discord-linux-overlay"),
            None => PathBuf::from(".config/discord-linux-overlay"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("dlo-config-{}-{label}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_are_valid_and_clamped() {
        let config = Config::default().normalized();
        assert_eq!(config.appearance.opacity, 100);
        assert_eq!(config.appearance.avatar_size, 40);
        assert!(config.behavior.always_on_top);
        assert_eq!(config.placement.anchor, Anchor::BottomRight);
        assert_eq!(config.placement.width, 320.0);
        assert!(config.hotkeys.toggle_overlay.is_some());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let config: Config = serde_json::from_value::<Config>(json!({
            "appearance": { "opacity": 0, "scale": 9000, "avatarSize": 4, "cornerRadius": 100, "gap": 999, "maxNameLength": 500 },
            "placement": { "offsetX": -40.0, "offsetY": 999999.0, "width": 4.0, "height": 100000.0 }
        }))
        .unwrap()
        .normalized();

        assert_eq!(config.appearance.opacity, 10);
        assert_eq!(config.appearance.scale, 200);
        assert_eq!(config.appearance.avatar_size, 24);
        assert_eq!(config.appearance.corner_radius, 32);
        assert_eq!(config.appearance.gap, 24);
        assert_eq!(config.appearance.max_name_length, 64);
        assert_eq!(config.placement.offset_x, 0.0);
        assert_eq!(config.placement.offset_y, 10_000.0);
        assert_eq!(config.placement.width, 120.0);
        assert_eq!(config.placement.height, 2160.0);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let config: Config = serde_json::from_value(json!({
            "appearance": { "opacity": 55 },
            "behavior": {}
        }))
        .unwrap();

        assert_eq!(config.appearance.opacity, 55);
        assert_eq!(config.appearance.avatar_size, 40);
        assert_eq!(config.behavior.click_through, true);
        assert_eq!(config.general.show_overlay_on_join, true);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let config: Config = serde_json::from_value(json!({
            "futureFeature": { "enabled": true },
            "appearance": { "opacity": 80, "hologram": true }
        }))
        .unwrap();
        assert_eq!(config.appearance.opacity, 80);
    }

    #[test]
    fn patches_merge_instead_of_replacing_sections() {
        let mut config = Config::default();
        config
            .apply_patch(&json!({ "appearance": { "opacity": 42 } }))
            .unwrap();

        assert_eq!(config.appearance.opacity, 42);
        // Untouched sibling fields survive the patch.
        assert_eq!(config.appearance.avatar_size, 40);
        assert_eq!(config.general.show_overlay_on_join, true);

        config
            .apply_patch(&json!({ "behavior": { "clickThrough": false } }))
            .unwrap();
        assert!(!config.behavior.click_through);
        assert_eq!(config.appearance.opacity, 42);
    }

    #[test]
    fn config_round_trips_through_disk_with_private_permissions() {
        let dir = temp_dir("round-trip");
        let path = dir.join("config.json");
        let store = ConfigStore::new(&path);

        let mut config = Config::default();
        config.appearance.opacity = 77;
        config.placement.anchor = Anchor::TopLeft;
        store.save(&config).unwrap();

        let loaded = store.load();
        assert_eq!(loaded, config);

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config must not be world readable");

        assert!(!path.with_extension("json.tmp").exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_corrupt_and_non_object_configs_do_not_panic() {
        let dir = temp_dir("corrupt");

        let missing = ConfigStore::new(dir.join("missing.json"));
        assert_eq!(missing.load(), Config::default());

        let corrupt_path = dir.join("corrupt.json");
        fs::write(&corrupt_path, b"{ not json").unwrap();
        assert_eq!(ConfigStore::new(&corrupt_path).load(), Config::default());

        let array_path = dir.join("array.json");
        fs::write(&array_path, b"[1,2,3]").unwrap();
        assert_eq!(ConfigStore::new(&array_path).load(), Config::default());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn patch_persists_to_disk() {
        let dir = temp_dir("patch-persist");
        let path = dir.join("config.json");
        let store = ConfigStore::new(&path);

        let updated = store
            .patch(&json!({ "placement": { "mode": "manual", "x": 100, "y": 200 } }))
            .unwrap();
        assert_eq!(updated.placement.mode, PlacementMode::Manual);
        assert_eq!(updated.placement.x, Some(100));

        let reloaded = store.load();
        assert_eq!(reloaded.placement.mode, PlacementMode::Manual);
        assert_eq!(reloaded.placement.y, Some(200));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn config_dir_honours_xdg_config_home() {
        // The function reads process-global environment state; assert the shape
        // rather than mutating the environment of the test runner.
        let dir = config_dir();
        assert!(dir.ends_with("discord-linux-overlay"));
    }

    #[test]
    fn layout_and_background_serialize_as_kebab_case() {
        assert_eq!(serde_json::to_string(&Layout::Grid).unwrap(), "\"grid\"");
        assert_eq!(
            serde_json::to_string(&PlacementMode::Anchored).unwrap(),
            "\"anchored\""
        );
        assert_eq!(
            serde_json::to_string(&Background::None).unwrap(),
            "\"none\""
        );
    }
}
