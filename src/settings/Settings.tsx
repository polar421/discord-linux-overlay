import { useEffect, useMemo, useState, type CSSProperties } from "react";

import {
  ExternalIcon,
  HeadphonesIcon,
  MicIcon,
  PointerOffIcon,
  RefreshIcon,
  SettingsIcon,
} from "../components/Icons";
import {
  Badge,
  Button,
  Card,
  NumberField,
  SelectField,
  Slider,
  TextField,
  Toggle,
} from "../components/controls";
import { ParticipantRow } from "../components/ParticipantRow";
import { call, isTauri } from "../lib/bridge";
import {
  useConfig,
  useConnection,
  useOverlayState,
  usePlatform,
} from "../lib/hooks";
import {
  defaultConfig,
  statusLabel,
  type Config,
  type HotkeysConfig,
  type MonitorsResponse,
  type Participant,
  type PlatformInfo,
  type Support,
} from "../lib/types";

type Tab = "general" | "appearance" | "position" | "behavior" | "discord" | "about";

const TABS: { id: Tab; label: string; icon?: "settings" | "mic" | "pointer" }[] = [
  { id: "general", label: "General" },
  { id: "appearance", label: "Appearance" },
  { id: "position", label: "Position" },
  { id: "behavior", label: "Behavior" },
  { id: "discord", label: "Discord", icon: "mic" },
  { id: "about", label: "About" },
];

const previewParticipants: Participant[] = [
  {
    id: "preview-1",
    displayName: "nyx",
    username: "nyx",
    avatarUrl: "/img/default-avatar.svg",
    speaking: true,
    muted: false,
    deafened: false,
    selfMuted: false,
    selfDeafened: false,
    suppressed: false,
    bot: false,
    isSelf: false,
  },
  {
    id: "preview-2",
    displayName: "Kestrel",
    username: "kestrel",
    avatarUrl: "/img/default-avatar.svg",
    speaking: false,
    muted: false,
    deafened: false,
    selfMuted: true,
    selfDeafened: false,
    suppressed: false,
    bot: false,
    isSelf: false,
  },
];

function supportBadge(level: Support, label: string) {
  const tone = level === "supported" ? "ok" : level === "best-effort" ? "warn" : "bad";
  return <Badge tone={tone}>{label}</Badge>;
}

function relativeTime(timestamp: number): string {
  const seconds = Math.max(1, Math.round((Date.now() - timestamp) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.round(minutes / 60)}h ago`;
}

function useGlobalShortcuts(config: Config | null) {
  const [errors, setErrors] = useState<Record<string, string>>({});
  const bindingKey = JSON.stringify(config?.hotkeys ?? null);

  useEffect(() => {
    if (!isTauri || !config) return;
    let cancelled = false;

    void (async () => {
      try {
        const shortcuts = await import("@tauri-apps/plugin-global-shortcut");
        await shortcuts.unregisterAll().catch(() => undefined);

        const nextErrors: Record<string, string> = {};
        const entries: [keyof HotkeysConfig, string][] = [
          ["toggleOverlay", "toggle_overlay"],
          ["toggleClickThrough", "toggle_click_through"],
          ["openSettings", "open_settings"],
          ["resetPosition", "reset_position"],
        ];

        for (const [field, command] of entries) {
          const accelerator = config.hotkeys[field];
          if (!accelerator) continue;
          try {
            await shortcuts.register(accelerator, (event) => {
              if (event.state === "Pressed") {
                void call(command).catch(console.error);
              }
            });
          } catch (error) {
            nextErrors[field] =
              error instanceof Error ? error.message : String(error);
          }
        }

        if (!cancelled) setErrors(nextErrors);
      } catch (error) {
        if (!cancelled) {
          setErrors({ global: error instanceof Error ? error.message : String(error) });
        }
      }
    })();

    return () => {
      cancelled = true;
    };
    // Re-register only when the accelerators change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bindingKey]);

  return errors;
}

export function Settings() {
  const { config, update } = useConfig();
  const connection = useConnection();
  const platform = usePlatform();
  const overlay = useOverlayState();
  const [tab, setTab] = useState<Tab>("general");
  const hotkeyErrors = useGlobalShortcuts(config);

  if (!config) {
    return (
      <div className="settings">
        <aside className="sidebar">
          <div className="sidebar-brand">
            <span className="brand-mark">
              <MicIcon />
            </span>
            Discord Linux Overlay
          </div>
        </aside>
        <main className="content">
          <p>Loading settings…</p>
        </main>
      </div>
    );
  }

  return (
    <div className="settings">
      <aside className="sidebar">
        <div className="sidebar-brand">
          <span className="brand-mark">
            <MicIcon />
          </span>
          <span>
            Discord Linux Overlay
            <br />
            <span className="brand-sub">v{__APP_VERSION__}</span>
          </span>
        </div>

        {TABS.map((item) => (
          <button
            key={item.id}
            type="button"
            className={`nav-button ${tab === item.id ? "active" : ""}`}
            onClick={() => setTab(item.id)}
          >
            {item.label}
            {item.id === "discord" && connection ? (
              <span
                style={{ marginLeft: "auto" }}
                className={`dot ${
                  connection.status.state === "ready" ? "ok" : "warn"
                }`}
              />
            ) : null}
          </button>
        ))}

        <span className="nav-spacer" />

        <div className="nav-footer">
          {platform
            ? `${platform.compositor} · ${platform.sessionType}`
            : "detecting session…"}
        </div>
      </aside>

      <main className="content">
        {tab === "general" ? (
          <GeneralTab config={config} update={update} platform={platform} />
        ) : null}
        {tab === "appearance" ? (
          <AppearanceTab config={config} update={update} />
        ) : null}
        {tab === "position" ? (
          <PositionTab config={config} update={update} />
        ) : null}
        {tab === "behavior" ? (
          <BehaviorTab
            config={config}
            update={update}
            platform={platform}
            errors={hotkeyErrors}
          />
        ) : null}
        {tab === "discord" ? (
          <DiscordTab connection={connection} history={overlay?.history ?? []} />
        ) : null}
        {tab === "about" ? <AboutTab platform={platform} /> : null}
      </main>
    </div>
  );
}

type Update = (patch: Record<string, unknown>) => Promise<Config>;

function Header({ title, children }: { title: string; children: string }) {
  return (
    <div className="content-header">
      <h1>{title}</h1>
      <p>{children}</p>
    </div>
  );
}

function GeneralTab({
  config,
  update,
  platform,
}: {
  config: Config;
  update: Update;
  platform: PlatformInfo | null;
}) {
  return (
    <>
      <Header title="General">
        Startup, automatic overlay visibility and compatibility options.
      </Header>

      <Card title="Startup">
        <Toggle
          label="Launch at login"
          hint="Adds an XDG autostart entry so the overlay starts with your desktop."
          checked={config.general.launchAtLogin}
          onChange={(value) => void update({ general: { launchAtLogin: value } })}
        />
        <Toggle
          label="Start minimized"
          hint="Start with the overlay hidden; use the tray or a shortcut to show it."
          checked={config.general.startHidden}
          onChange={(value) => void update({ general: { startHidden: value } })}
        />
      </Card>

      <Card title="Automation">
        <Toggle
          label="Show the overlay when joining voice"
          checked={config.general.showOverlayOnJoin}
          onChange={(value) => void update({ general: { showOverlayOnJoin: value } })}
        />
        <Toggle
          label="Hide the overlay when leaving voice"
          checked={config.general.hideOverlayOnLeave}
          onChange={(value) => void update({ general: { hideOverlayOnLeave: value } })}
        />
        <Toggle
          label="Show join and leave history"
          hint="Records who enters and leaves your channel in this session."
          checked={config.general.showJoinHistory}
          onChange={(value) => void update({ general: { showJoinHistory: value } })}
        />
        <Toggle
          label="Show connection status in the overlay"
          hint="Displays a small status row when you are not connected or alone."
          checked={config.general.showConnectionStatus}
          onChange={(value) => void update({ general: { showConnectionStatus: value } })}
        />
      </Card>

      <Card
        title="Rendering compatibility"
        description="WebKitGTK's GPU path is unreliable on some NVIDIA drivers. Prefer the software renderer only if the window is blank or crashes."
      >
        <Toggle
          label="Disable WebKit DMA-BUF renderer"
          hint="Equivalent to WEBKIT_DISABLE_DMABUF_RENDERER=1. Fixes 'Failed to create GBM buffer: Invalid argument' on NVIDIA/X11 at the cost of GPU-accelerated web rendering. Restart required."
          checked={config.general.disableDmabufRenderer}
          onChange={(value) =>
            void update({ general: { disableDmabufRenderer: value } })
          }
        />

        {platform ? (
          <>
            <dl className="kv">
              <dt>Session</dt>
              <dd>
                {platform.sessionType} · {platform.compositor}
              </dd>
              <dt>Desktop</dt>
              <dd>{platform.desktop}</dd>
              <dt>GPU</dt>
              <dd>
                {platform.nvidia
                  ? `NVIDIA${platform.nvidiaDriver ? ` (driver ${platform.nvidiaDriver})` : ""}`
                  : "not NVIDIA"}
              </dd>
            </dl>
            <ul className="note-list">
              {platform.notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          </>
        ) : null}
      </Card>

      <Card title="Reset" description="Restore every setting to its default value.">
        <div className="button-row">
          <Button
            variant="danger"
            onClick={() => {
              if (
                window.confirm(
                  "Reset all settings to their defaults? The overlay position is reset too.",
                )
              ) {
                void update(defaultConfig as unknown as Record<string, unknown>);
                void call("reset_position");
              }
            }}
          >
            Reset all settings
          </Button>
        </div>
      </Card>
    </>
  );
}

function AppearanceTab({ config, update }: { config: Config; update: Update }) {
  const appearance = config.appearance;
  const setAppearance = (patch: Record<string, unknown>) =>
    void update({ appearance: patch });

  const previewStyle = {
    "--avatar-size": `${appearance.avatarSize}px`,
    "--font-size": "13px",
    "--gap": `${appearance.gap}px`,
    "--radius": `${appearance.cornerRadius}px`,
  } as CSSProperties;

  return (
    <>
      <Header title="Appearance">
        Everything here applies to the overlay immediately.
      </Header>

      <Card title="Preview">
        <div className={`preview layout-${appearance.layout}`} style={previewStyle}>
          {previewParticipants.map((participant) => (
            <ParticipantRow
              key={participant.id}
              participant={participant}
              appearance={appearance}
              compact={appearance.layout !== "stack"}
            />
          ))}
        </div>
      </Card>

      <Card title="Layout">
        <SelectField
          label="Layout"
          hint="Stack shows names beside avatars. Row and Grid are compact avatar strips."
          value={appearance.layout}
          options={[
            { value: "stack", label: "Stack (names)" },
            { value: "row", label: "Row (avatars)" },
            { value: "grid", label: "Grid (avatars)" },
          ]}
          onChange={(value) => setAppearance({ layout: value })}
        />
        <Slider
          label="Scale"
          hint="Scales avatars, text and spacing together."
          value={appearance.scale}
          min={50}
          max={200}
          step={5}
          suffix="%"
          onChange={(value) => setAppearance({ scale: value })}
        />
        <Slider
          label="Opacity"
          value={appearance.opacity}
          min={10}
          max={100}
          step={5}
          suffix="%"
          onChange={(value) => setAppearance({ opacity: value })}
        />
        <Slider
          label="Avatar size"
          value={appearance.avatarSize}
          min={24}
          max={96}
          step={2}
          suffix="px"
          onChange={(value) => setAppearance({ avatarSize: value })}
        />
        <Slider
          label="Spacing"
          value={appearance.gap}
          min={0}
          max={24}
          suffix="px"
          onChange={(value) => setAppearance({ gap: value })}
        />
        <Slider
          label="Corner radius"
          value={appearance.cornerRadius}
          min={0}
          max={32}
          suffix="px"
          onChange={(value) => setAppearance({ cornerRadius: value })}
        />
        <SelectField
          label="Row background"
          hint="Solid draws a translucent pill behind each row."
          value={appearance.background}
          options={[
            { value: "solid", label: "Solid" },
            { value: "none", label: "None" },
          ]}
          onChange={(value) => setAppearance({ background: value })}
        />
      </Card>

      <Card title="Content">
        <Toggle
          label="Show usernames"
          checked={appearance.showNames}
          onChange={(value) => setAppearance({ showNames: value })}
        />
        <Toggle
          label="Show my own user"
          checked={appearance.showOwnUser}
          onChange={(value) => setAppearance({ showOwnUser: value })}
        />
        <Toggle
          label="Only show people who are speaking"
          checked={appearance.showOnlySpeaking}
          onChange={(value) => setAppearance({ showOnlySpeaking: value })}
        />
        <Slider
          label="Maximum name length"
          hint="Long names are truncated with an ellipsis."
          value={appearance.maxNameLength}
          min={4}
          max={64}
          onChange={(value) => setAppearance({ maxNameLength: value })}
        />
      </Card>
    </>
  );
}

function PositionTab({ config, update }: { config: Config; update: Update }) {
  const [monitors, setMonitors] = useState<MonitorsResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useMemo(
    () => () => {
      call<MonitorsResponse>("get_monitors").then(setMonitors, (reason: unknown) =>
        setError(String(reason)),
      );
    },
    [],
  );

  useEffect(() => {
    refresh();
  }, [refresh]);

  const placement = config.placement;
  const setPlacement = (patch: Record<string, unknown>) =>
    void update({ placement: patch });

  return (
    <>
      <Header title="Position">
        Anchor the overlay to a monitor corner or drag it wherever you like.
      </Header>

      <Card title="Placement mode">
        <SelectField
          label="Mode"
          hint="Anchored keeps the overlay at a fixed offset from a monitor edge. Manual remembers the last dragged position."
          value={placement.mode}
          options={[
            { value: "anchored", label: "Anchored" },
            { value: "manual", label: "Manual (drag)" },
          ]}
          onChange={(value) => setPlacement({ mode: value })}
        />

        {placement.mode === "manual" ? (
          <p className="field-hint" style={{ marginTop: 8 }}>
            Turn off click-through, then drag the overlay with its handle. The
            position and size are stored every time you let go.
          </p>
        ) : (
          <>
            <SelectField
              label="Monitor"
              value={placement.monitor ?? ""}
              options={[
                { value: "", label: "Primary monitor" },
                ...(monitors?.monitors ?? []).map((monitor) => ({
                  value: monitor.name,
                  label: `${monitor.name || "Monitor"} — ${monitor.width}×${monitor.height}${
                    monitor.primary ? " (primary)" : ""
                  }`,
                })),
              ]}
              onChange={(value) =>
                setPlacement({ monitor: value === "" ? null : value })
              }
            />
            <SelectField
              label="Anchor"
              value={placement.anchor}
              options={[
                { value: "top-left", label: "Top left" },
                { value: "top-center", label: "Top center" },
                { value: "top-right", label: "Top right" },
                { value: "middle-left", label: "Middle left" },
                { value: "center", label: "Center" },
                { value: "middle-right", label: "Middle right" },
                { value: "bottom-left", label: "Bottom left" },
                { value: "bottom-center", label: "Bottom center" },
                { value: "bottom-right", label: "Bottom right" },
              ]}
              onChange={(value) => setPlacement({ anchor: value })}
            />
            <NumberField
              label="Horizontal offset"
              value={placement.offsetX}
              min={0}
              max={2000}
              suffix="px"
              onChange={(value) => setPlacement({ offsetX: value })}
            />
            <NumberField
              label="Vertical offset"
              value={placement.offsetY}
              min={0}
              max={2000}
              suffix="px"
              onChange={(value) => setPlacement({ offsetY: value })}
            />
          </>
        )}

        <NumberField
          label="Overlay width"
          hint="Logical size of the overlay window."
          value={Math.round(placement.width)}
          min={120}
          max={1920}
          suffix="px"
          onChange={(value) => setPlacement({ width: value })}
        />
        <NumberField
          label="Overlay height"
          value={Math.round(placement.height)}
          min={80}
          max={1080}
          suffix="px"
          onChange={(value) => setPlacement({ height: value })}
        />

        <div className="button-row">
          <Button variant="primary" onClick={() => void call("apply_placement_now")}>
            Apply position
          </Button>
          <Button onClick={() => void call("reset_position")}>Reset position</Button>
          <Button onClick={refresh}>
            <RefreshIcon /> Refresh monitors
          </Button>
        </div>

        {monitors && !monitors.positioningSupported ? (
          <div className="hint-box">
            <strong>Wayland does not allow applications to position windows.</strong>{" "}
            The overlay is placed by the compositor; drag it once with the handle
            and KWin will remember where it belongs. Running the app through
            XWayland (<span className="mono">GDK_BACKEND=x11</span>) restores exact
            anchoring.
          </div>
        ) : null}
        {error ? <div className="hint-box">{error}</div> : null}
      </Card>
    </>
  );
}

function BehaviorTab({
  config,
  update,
  platform,
  errors,
}: {
  config: Config;
  update: Update;
  platform: PlatformInfo | null;
  errors: Record<string, string>;
}) {
  const setBehavior = (patch: Record<string, unknown>) =>
    void update({ behavior: patch });
  const setHotkey = (key: keyof HotkeysConfig, value: string) =>
    void update({ hotkeys: { [key]: value.trim() === "" ? null : value.trim() } });

  return (
    <>
      <Header title="Behavior">
        Window behaviour, interaction and keyboard shortcuts.
      </Header>

      <Card title="Window">
        <Toggle
          label="Always on top"
          hint={
            platform?.alwaysOnTop === "best-effort"
              ? "Attempted through the toolkit. KWin on Wayland ignores this for ordinary windows — use a window rule (see LINUX.md)."
              : "Keeps the overlay above other windows."
          }
          checked={config.behavior.alwaysOnTop}
          onChange={(value) => setBehavior({ alwaysOnTop: value })}
        />
        <Toggle
          label="Click-through"
          hint="The overlay ignores the mouse so clicks reach the game. Turn this off to drag or use the overlay buttons."
          checked={config.behavior.clickThrough}
          onChange={(value) => setBehavior({ clickThrough: value })}
        />
        <Toggle
          label="Lock position"
          hint="Prevents accidental dragging and disables resizing."
          checked={config.behavior.lockPosition}
          onChange={(value) => setBehavior({ lockPosition: value })}
        />

        {platform ? (
          <div className="status-row" style={{ flexWrap: "wrap", gap: 6 }}>
            {supportBadge(
              platform.alwaysOnTop,
              `always on top: ${platform.alwaysOnTop}`,
            )}
            {supportBadge(
              platform.clickThrough,
              `click-through: ${platform.clickThrough}`,
            )}
            {supportBadge(
              platform.globalShortcuts,
              `global shortcuts: ${platform.globalShortcuts}`,
            )}
          </div>
        ) : null}

        <div className="button-row">
          <Button onClick={() => void call("toggle_click_through")}>
            <PointerOffIcon /> Toggle click-through now
          </Button>
          <Button onClick={() => void call("toggle_overlay")}>Show / hide overlay</Button>
          <Button onClick={() => void call("open_settings")}>
            <SettingsIcon /> Focus this window
          </Button>
        </div>
      </Card>

      <Card
        title="Keyboard shortcuts"
        description="Accelerators use Tauri syntax, for example CommandOrControl+Shift+G. Leave a field empty to disable it."
      >
        <TextField
          label="Show / hide overlay"
          value={config.hotkeys.toggleOverlay ?? ""}
          placeholder="CommandOrControl+Shift+G"
          onChange={(value) => setHotkey("toggleOverlay", value)}
        />
        {errors.toggleOverlay ? (
          <div className="hint-box">{errors.toggleOverlay}</div>
        ) : null}
        <TextField
          label="Toggle click-through"
          value={config.hotkeys.toggleClickThrough ?? ""}
          placeholder="CommandOrControl+Shift+H"
          onChange={(value) => setHotkey("toggleClickThrough", value)}
        />
        {errors.toggleClickThrough ? (
          <div className="hint-box">{errors.toggleClickThrough}</div>
        ) : null}
        <TextField
          label="Open settings"
          value={config.hotkeys.openSettings ?? ""}
          placeholder="CommandOrControl+Shift+S"
          onChange={(value) => setHotkey("openSettings", value)}
        />
        <TextField
          label="Reset position"
          value={config.hotkeys.resetPosition ?? ""}
          placeholder="not set"
          onChange={(value) => setHotkey("resetPosition", value)}
        />

        {platform?.globalShortcuts === "unsupported" ? (
          <div className="hint-box">
            <strong>Global shortcuts are not available on Wayland.</strong> Bind a
            KDE shortcut to the CLI instead, for example:
            <br />
            <span className="mono">discord-linux-overlay --toggle-click-through</span>
            <br />
            <span className="mono">discord-linux-overlay --toggle</span>
          </div>
        ) : null}
        {errors.global ? <div className="hint-box">{errors.global}</div> : null}
      </Card>
    </>
  );
}

function DiscordTab({
  connection,
  history,
}: {
  connection: ReturnType<typeof useConnection>;
  history: { id: string; displayName: string; avatarUrl: string; action: "join" | "leave"; timestamp: number }[];
}) {
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [redirectUri, setRedirectUri] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);

  useEffect(() => {
    if (connection?.credentials.clientId) {
      setClientId(connection.credentials.clientId);
    }
    if (connection?.credentials.redirectUri) {
      setRedirectUri(connection.credentials.redirectUri);
    }
  }, [
    connection?.credentials.clientId,
    connection?.credentials.redirectUri,
  ]);

  const refreshLogs = () => {
    call<string[]>("get_logs").then(setLogs, (error: unknown) => setNotice(String(error)));
  };

  useEffect(() => {
    refreshLogs();
  }, []);

  const save = async () => {
    setBusy(true);
    setNotice(null);
    try {
      await call("set_credentials", {
        clientId,
        clientSecret,
        redirectUri: redirectUri.trim() === "" ? null : redirectUri.trim(),
      });
      setClientSecret("");
      setNotice("Credentials saved. Discord may now ask you to authorize the application.");
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const status = connection?.status;

  return (
    <>
      <Header title="Discord">
        Connection state, your Discord application credentials and diagnostics.
      </Header>

      <Card title="Connection">
        <div className="status-row">
          <span
            className={`dot ${status?.state === "ready" ? "ok" : "warn"}`}
          />
          <span>{status ? statusLabel(status) : "Loading…"}</span>
          {status?.state === "ready" ? <Badge tone="ok">live</Badge> : null}
        </div>
        {status?.state === "needsSetup" ? (
          <div className="hint-box">{status.message}</div>
        ) : null}
        {status?.state === "waitingForDiscord" ? (
          <p className="field-hint" style={{ marginTop: 8 }}>
            Start the official Discord desktop client. The overlay talks to it
            over the local IPC socket; third-party clients such as Vesktop or
            ArmCord do not provide the RPC voice API.
          </p>
        ) : null}
        {connection?.channel ? (
          <dl className="kv">
            <dt>Channel</dt>
            <dd>#{connection.channel.name}</dd>
            <dt>Participants</dt>
            <dd>{connection.participants}</dd>
          </dl>
        ) : null}

        <div className="button-row">
          <Button onClick={() => void call("reconnect_discord")}>
            <RefreshIcon /> Reconnect
          </Button>
          <Button onClick={() => void call("reauthorize_discord")}>
            Authorize again
          </Button>
          <Button onClick={() => void call("sign_out_discord")}>Sign out</Button>
        </div>
      </Card>

      <Card
        title="Discord application"
        description="This project does not ship a Discord application id, so every installation uses its own free Discord application. Create one at discord.com/developers/applications, enable RPC, and paste its client id and secret here. The secret is stored in a 0600 file in your config directory and never leaves this machine except to discord.com for the OAuth token exchange."
      >
        <TextField
          label="Client id"
          value={clientId}
          placeholder="123456789012345678"
          onChange={setClientId}
        />
        <TextField
          label="Client secret"
          type="password"
          value={clientSecret}
          placeholder={
            connection?.credentials.hasSecret ? "already configured" : "required"
          }
          onChange={setClientSecret}
        />
        <TextField
          label="Redirect URI"
          hint="Only needed if you set one on the Discord application."
          value={redirectUri}
          placeholder="http://localhost"
          onChange={setRedirectUri}
        />

        <div className="button-row">
          <Button variant="primary" disabled={busy} onClick={() => void save()}>
            Save credentials
          </Button>
          <Button
            variant="danger"
            onClick={() => {
              if (window.confirm("Remove the stored Discord credentials?")) {
                void call("clear_credentials");
              }
            }}
          >
            Remove credentials
          </Button>
          <Button
            onClick={() =>
              window.open(
                "https://github.com/polar421/discord-linux-overlay/blob/main/docs/DISCORD_SETUP.md",
                "_blank",
                "noopener",
              )
            }
          >
            <ExternalIcon /> Setup guide
          </Button>
        </div>
        {notice ? <div className="hint-box">{notice}</div> : null}
      </Card>

      <Card title="Join history">
        {history.length === 0 ? (
          <p className="field-hint">No one has joined or left this session yet.</p>
        ) : (
          <div className="history-list">
            {history
              .slice()
              .reverse()
              .slice(0, 20)
              .map((entry, index) => (
                <div className="history-row" key={`${entry.id}-${entry.timestamp}-${index}`}>
                  <img src={entry.avatarUrl} alt="" />
                  <span>{entry.displayName}</span>
                  <Badge tone={entry.action === "join" ? "ok" : "muted"}>
                    {entry.action === "join" ? (
                      <>
                        <MicIcon /> joined
                      </>
                    ) : (
                      <>
                        <HeadphonesIcon /> left
                      </>
                    )}
                  </Badge>
                  <time>{relativeTime(entry.timestamp)}</time>
                </div>
              ))}
          </div>
        )}
        <div className="button-row">
          <Button onClick={() => void call("clear_join_history")}>Clear history</Button>
        </div>
      </Card>

      <Card title="Diagnostics">
        <div className="button-row" style={{ marginTop: 0 }}>
          <Button onClick={refreshLogs}>
            <RefreshIcon /> Refresh log
          </Button>
          <Button onClick={() => void call("open_config_dir")}>Open config folder</Button>
        </div>
        <pre className="log" style={{ marginTop: 10 }}>
          {logs.length === 0 ? "No log lines yet." : logs.join("\n")}
        </pre>
      </Card>
    </>
  );
}

function AboutTab({ platform }: { platform: PlatformInfo | null }) {
  return (
    <>
      <Header title="About">Discord Linux Overlay</Header>

      <Card title="Discord Linux Overlay">
        <dl className="kv">
          <dt>Version</dt>
          <dd>{__APP_VERSION__}</dd>
          <dt>License</dt>
          <dd>AGPL-3.0-or-later</dd>
          <dt>Repository</dt>
          <dd>
            <a
              href="https://github.com/polar421/discord-linux-overlay"
              target="_blank"
              rel="noreferrer"
            >
              polar421/discord-linux-overlay
            </a>
          </dd>
          <dt>Session</dt>
          <dd>
            {platform ? `${platform.compositor} (${platform.sessionType})` : "unknown"}
          </dd>
          <dt>GPU</dt>
          <dd>{platform?.nvidia ? "NVIDIA" : "unknown"}</dd>
        </dl>

        <p className="field-hint" style={{ marginTop: 12 }}>
          An independent Discord voice overlay for Linux. It is inspired by the
          user experience of Overlayed (AGPL-3.0, overlaydev/overlayed) and is not
          affiliated with or endorsed by the Overlayed project. See NOTICE and
          docs/UPSTREAM_RESEARCH.md for the reuse breakdown.
        </p>

        <div className="button-row">
          <Button
            onClick={() =>
              window.open(
                "https://github.com/polar421/discord-linux-overlay/blob/main/LINUX.md",
                "_blank",
                "noopener",
              )
            }
          >
            <ExternalIcon /> Linux &amp; Wayland notes
          </Button>
          <Button variant="danger" onClick={() => void call("quit_app")}>
            Quit Discord Linux Overlay
          </Button>
        </div>
      </Card>

      <Card title="Why does it need my own Discord application?">
        <p className="field-hint">
          Discord's local RPC only exposes voice state to a registered
          application, and the authorization code must be exchanged with that
          application's client secret. Shipping a shared secret inside an open
          source binary would be unsafe, so the overlay asks you to create your
          own free application once. No Discord token is ever scraped.
        </p>
      </Card>
    </>
  );
}
