export type Layout = "stack" | "row" | "grid";
export type Background = "solid" | "none";
export type PlacementMode = "manual" | "anchored";
export type Anchor =
  | "top-left"
  | "top-center"
  | "top-right"
  | "middle-left"
  | "center"
  | "middle-right"
  | "bottom-left"
  | "bottom-center"
  | "bottom-right";

export type Support = "supported" | "best-effort" | "unsupported";

export interface GeneralConfig {
  launchAtLogin: boolean;
  startHidden: boolean;
  showOverlayOnJoin: boolean;
  hideOverlayOnLeave: boolean;
  showJoinHistory: boolean;
  showConnectionStatus: boolean;
  disableDmabufRenderer: boolean;
}

export interface AppearanceConfig {
  opacity: number;
  scale: number;
  avatarSize: number;
  showNames: boolean;
  showOwnUser: boolean;
  showOnlySpeaking: boolean;
  layout: Layout;
  cornerRadius: number;
  gap: number;
  background: Background;
  maxNameLength: number;
}

export interface BehaviorConfig {
  alwaysOnTop: boolean;
  clickThrough: boolean;
  lockPosition: boolean;
}

export interface PlacementConfig {
  mode: PlacementMode;
  monitor: string | null;
  anchor: Anchor;
  offsetX: number;
  offsetY: number;
  width: number;
  height: number;
  x: number | null;
  y: number | null;
}

export interface HotkeysConfig {
  toggleOverlay: string | null;
  toggleClickThrough: string | null;
  openSettings: string | null;
  resetPosition: string | null;
}

export interface Config {
  general: GeneralConfig;
  appearance: AppearanceConfig;
  behavior: BehaviorConfig;
  placement: PlacementConfig;
  hotkeys: HotkeysConfig;
}

export const defaultConfig: Config = {
  general: {
    launchAtLogin: false,
    startHidden: false,
    showOverlayOnJoin: true,
    hideOverlayOnLeave: false,
    showJoinHistory: false,
    showConnectionStatus: true,
    disableDmabufRenderer: false,
  },
  appearance: {
    opacity: 100,
    scale: 100,
    avatarSize: 40,
    showNames: true,
    showOwnUser: true,
    showOnlySpeaking: false,
    layout: "stack",
    cornerRadius: 12,
    gap: 6,
    background: "solid",
    maxNameLength: 24,
  },
  behavior: {
    alwaysOnTop: true,
    clickThrough: true,
    lockPosition: false,
  },
  placement: {
    mode: "anchored",
    monitor: null,
    anchor: "bottom-right",
    offsetX: 24,
    offsetY: 24,
    width: 320,
    height: 440,
    x: null,
    y: null,
  },
  hotkeys: {
    toggleOverlay: "CommandOrControl+Shift+G",
    toggleClickThrough: "CommandOrControl+Shift+H",
    openSettings: null,
    resetPosition: null,
  },
};

export interface Participant {
  id: string;
  displayName: string;
  username: string;
  avatarUrl: string;
  speaking: boolean;
  muted: boolean;
  deafened: boolean;
  selfMuted: boolean;
  selfDeafened: boolean;
  suppressed: boolean;
  bot: boolean;
  isSelf: boolean;
}

export interface ChannelInfo {
  id: string;
  name: string;
  guildId: string | null;
  kind: number | null;
}

export interface HistoryEntry {
  id: string;
  displayName: string;
  avatarUrl: string;
  action: "join" | "leave";
  timestamp: number;
}

export interface OverlayState {
  me: Participant | null;
  channel: ChannelInfo | null;
  participants: Participant[];
  history: HistoryEntry[];
  revision: number;
}

export type Status =
  | { state: "starting" }
  | { state: "waitingForDiscord"; message: string }
  | { state: "connecting" }
  | { state: "authorizing" }
  | { state: "authenticating" }
  | { state: "ready"; userName: string }
  | { state: "needsSetup"; message: string }
  | { state: "stopped" };

export interface CredentialsView {
  clientId: string;
  maskedClientId: string;
  hasSecret: boolean;
  redirectUri: string | null;
}

export interface ConnectionInfo {
  status: Status;
  statusText: string;
  credentials: CredentialsView;
  configured: boolean;
  channel: ChannelInfo | null;
  participants: number;
}

export interface PlatformInfo {
  os: string;
  sessionType: "wayland" | "x11";
  desktop: string;
  compositor: string;
  nvidia: boolean;
  nvidiaDriver: string | null;
  alwaysOnTop: Support;
  clickThrough: Support;
  globalShortcuts: Support;
  windowPositioning: Support;
  notes: string[];
}

export interface MonitorInfo {
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
  primary: boolean;
}

export interface MonitorsResponse {
  monitors: MonitorInfo[];
  positioningSupported: boolean;
}

export const statusLabel = (status: Status): string => {
  switch (status.state) {
    case "starting":
      return "Starting…";
    case "waitingForDiscord":
      return "Waiting for the Discord desktop client";
    case "connecting":
      return "Connecting to Discord";
    case "authorizing":
      return "Waiting for authorization in Discord";
    case "authenticating":
      return "Signing in";
    case "ready":
      return `Connected as ${status.userName}`;
    case "needsSetup":
      return "Setup required";
    case "stopped":
      return "Stopped";
  }
};
