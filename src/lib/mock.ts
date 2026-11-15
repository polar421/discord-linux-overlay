/**
 * Browser-only development harness.
 *
 * `npm run dev` serves the real UI with a fake Discord voice channel so layout,
 * speaking indicators and settings can be reviewed (and screenshotted) without
 * Tauri, a Discord account or a Linux desktop. It is never used inside the
 * application.
 */
import {
  defaultConfig,
  type Config,
  type ConnectionInfo,
  type MonitorInfo,
  type MonitorsResponse,
  type OverlayState,
  type Participant,
  type PlatformInfo,
  type Status,
} from "./types";

type Listener = (payload: unknown) => void;

const listeners = new Map<string, Set<Listener>>();

const avatars = [
  "https://cdn.discordapp.com/embed/avatars/0.png",
  "https://cdn.discordapp.com/embed/avatars/1.png",
  "https://cdn.discordapp.com/embed/avatars/2.png",
  "https://cdn.discordapp.com/embed/avatars/3.png",
];

const people: Participant[] = [
  {
    id: "1",
    displayName: "You",
    username: "polar421",
    avatarUrl: avatars[0],
    speaking: false,
    muted: false,
    deafened: false,
    selfMuted: false,
    selfDeafened: false,
    suppressed: false,
    bot: false,
    isSelf: true,
  },
  {
    id: "2",
    displayName: "nyx",
    username: "nyx",
    avatarUrl: avatars[1],
    speaking: false,
    muted: false,
    deafened: false,
    selfMuted: true,
    selfDeafened: false,
    suppressed: false,
    bot: false,
    isSelf: false,
  },
  {
    id: "3",
    displayName: "Kestrel",
    username: "kestrel",
    avatarUrl: avatars[2],
    speaking: false,
    muted: false,
    deafened: true,
    selfMuted: false,
    selfDeafened: true,
    suppressed: false,
    bot: false,
    isSelf: false,
  },
  {
    id: "4",
    displayName: "build-bot",
    username: "build-bot",
    avatarUrl: avatars[3],
    speaking: false,
    muted: false,
    deafened: false,
    selfMuted: false,
    selfDeafened: false,
    suppressed: false,
    bot: true,
    isSelf: false,
  },
];

// The browser preview starts in interactive mode so the toolbar (drag handle,
// pin, lock, reset, settings) can be inspected and clicked. The real application
// defaults to click-through, which hides it.
let config: Config = {
  ...defaultConfig,
  behavior: { ...defaultConfig.behavior, clickThrough: false },
};
let overlay: OverlayState = {
  me: people[0] ?? null,
  channel: { id: "10", name: "General", guildId: "42", kind: 2 },
  participants: people,
  history: [
    {
      id: "2",
      displayName: "nyx",
      avatarUrl: avatars[1] ?? "",
      action: "join",
      timestamp: Date.now() - 90_000,
    },
    {
      id: "4",
      displayName: "build-bot",
      avatarUrl: avatars[3] ?? "",
      action: "join",
      timestamp: Date.now() - 40_000,
    },
  ],
  revision: 1,
};

let status: Status = { state: "ready", userName: "polar421" };
let visible = true;

function emit(event: string, payload: unknown) {
  const handlers = listeners.get(event);
  if (!handlers) return;
  for (const handler of handlers) handler(payload);
}

function bumpOverlay() {
  overlay = { ...overlay, revision: overlay.revision + 1 };
  emit("overlay:state", overlay);
}

function emitConfig() {
  emit("config:changed", config);
}

const monitors: MonitorInfo[] = [
  {
    name: "DP-1",
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
    scaleFactor: 1,
    primary: true,
  },
  {
    name: "HDMI-A-1",
    x: 1920,
    y: 0,
    width: 1600,
    height: 900,
    scaleFactor: 1,
    primary: false,
  },
];

const platform: PlatformInfo = {
  os: "linux x86_64",
  sessionType: "wayland",
  desktop: "KDE",
  compositor: "KWin (Wayland)",
  nvidia: true,
  nvidiaDriver: "620.70.06",
  alwaysOnTop: "best-effort",
  clickThrough: "best-effort",
  globalShortcuts: "unsupported",
  windowPositioning: "unsupported",
  notes: [
    "Mock mode: this is the browser preview, not the real application.",
    "Wayland has no client-side \"always on top\". KWin honours a window rule instead.",
    "Global hotkeys are unavailable on Wayland; bind the CLI to a KDE shortcut.",
  ],
};

const connection = (): ConnectionInfo => ({
  status,
  statusText:
    status.state === "ready"
      ? `Connected as ${status.userName}`
      : status.state === "waitingForDiscord"
        ? "Waiting for the Discord desktop client"
        : status.state,
  credentials: {
    clientId: "123456789012345678",
    maskedClientId: "1234…6789",
    hasSecret: true,
    redirectUri: null,
  },
  configured: true,
  channel: overlay.channel,
  participants: overlay.participants.length,
});

// Rotate the speaking indicator so the overlay looks alive in the preview.
// Exactly one eligible participant speaks at a time, which mirrors a real voice
// channel and keeps screenshots stable.
let speakerIndex = 0;
const rotation = setInterval(() => {
  const eligible = overlay.participants.filter(
    (participant) =>
      !participant.selfMuted && !participant.selfDeafened && !participant.suppressed,
  );
  if (eligible.length === 0) return;

  speakerIndex = (speakerIndex + 1) % eligible.length;
  const speakerId = eligible[speakerIndex]?.id;

  overlay = {
    ...overlay,
    participants: overlay.participants.map((participant) => ({
      ...participant,
      speaking: participant.id === speakerId,
    })),
    revision: overlay.revision + 1,
  };
  emit("overlay:state", overlay);
}, 1600);

if (typeof window !== "undefined") {
  window.addEventListener("beforeunload", () => clearInterval(rotation));
}

function updateConfig(patch: Record<string, unknown>): Config {
  config = { ...config, ...(patch as Partial<Config>) };
  // Deep-merge the sections the UI actually patches.
  if (patch.appearance) {
    config.appearance = {
      ...config.appearance,
      ...(patch.appearance as Partial<Config["appearance"]>),
    };
  }
  if (patch.general) {
    config.general = {
      ...config.general,
      ...(patch.general as Partial<Config["general"]>),
    };
  }
  if (patch.behavior) {
    config.behavior = {
      ...config.behavior,
      ...(patch.behavior as Partial<Config["behavior"]>),
    };
  }
  if (patch.placement) {
    config.placement = {
      ...config.placement,
      ...(patch.placement as Partial<Config["placement"]>),
    };
  }
  if (patch.hotkeys) {
    config.hotkeys = {
      ...config.hotkeys,
      ...(patch.hotkeys as Partial<Config["hotkeys"]>),
    };
  }
  emitConfig();
  return config;
}

export async function mockCall<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  await new Promise((resolve) => setTimeout(resolve, 8));

  const handlers: Record<string, () => unknown> = {
    get_config: () => config,
    update_config: () => updateConfig((args?.patch as Record<string, unknown>) ?? {}),
    get_overlay_state: () => overlay,
    get_status: () => status,
    get_connection: () => connection(),
    get_platform: () => platform,
    get_logs: () => [
      "mock: browser preview (no Discord client attached)",
      "mock: 4 participants in #General",
    ],
    get_monitors: (): MonitorsResponse => ({
      monitors,
      positioningSupported: false,
    }),
    toggle_overlay: () => {
      visible = !visible;
      return visible;
    },
    set_overlay_visible: () => {
      visible = Boolean(args?.value);
      return visible;
    },
    toggle_click_through: () =>
      updateConfig({
        behavior: {
          ...config.behavior,
          clickThrough: !config.behavior.clickThrough,
        },
      }).behavior.clickThrough,
    set_click_through: () =>
      updateConfig({
        behavior: { ...config.behavior, clickThrough: Boolean(args?.value) },
      }).behavior.clickThrough,
    set_always_on_top: () => {
      updateConfig({
        behavior: {
          ...config.behavior,
          alwaysOnTop: Boolean(args?.value),
        },
      });
      return undefined;
    },
    set_locked: () => {
      updateConfig({
        behavior: { ...config.behavior, lockPosition: Boolean(args?.value) },
      });
      return undefined;
    },
    reset_position: () =>
      updateConfig({
        placement: {
          ...config.placement,
          mode: "anchored",
          anchor: "bottom-right",
          monitor: null,
          offsetX: 24,
          offsetY: 24,
        },
      }),
    apply_placement_now: () => undefined,
    set_credentials: () => connection().credentials,
    clear_credentials: () => undefined,
    reconnect_discord: () => {
      status = { state: "connecting" };
      emit("connection:status", status);
      setTimeout(() => {
        status = { state: "ready", userName: "polar421" };
        emit("connection:status", status);
      }, 800);
      return undefined;
    },
    reauthorize_discord: () => undefined,
    sign_out_discord: () => undefined,
    open_settings: () => undefined,
    close_settings: () => undefined,
    clear_join_history: () => {
      overlay = { ...overlay, history: [] };
      bumpOverlay();
    },
    open_config_dir: () => undefined,
    quit_app: () => undefined,
  };

  const handler = handlers[command];
  if (!handler) {
    throw new Error(`mock harness has no handler for "${command}"`);
  }
  return handler() as T;
}

export function mockListen<T>(
  event: string,
  handler: (payload: T) => void,
): () => void {
  const set = listeners.get(event) ?? new Set<Listener>();
  const wrapped: Listener = (payload) => handler(payload as T);
  set.add(wrapped);
  listeners.set(event, set);

  return () => {
    set.delete(wrapped);
  };
}
