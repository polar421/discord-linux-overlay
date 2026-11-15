import { useMemo, useState, type CSSProperties } from "react";

import {
  GripIcon,
  LockIcon,
  MicIcon,
  PointerIcon,
  PointerOffIcon,
  RefreshIcon,
  SettingsIcon,
  UnlockIcon,
} from "../components/Icons";
import { ParticipantRow } from "../components/ParticipantRow";
import { call, isTauri, openSettings } from "../lib/bridge";
import { useConfig, useOverlayState, useStatus } from "../lib/hooks";
import type { Participant } from "../lib/types";

async function startDragging(locked: boolean) {
  if (!isTauri || locked) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().startDragging();
}

export function Overlay() {
  const { config } = useConfig();
  const state = useOverlayState();
  const status = useStatus();
  const [busy, setBusy] = useState(false);

  const appearance = config?.appearance;

  const participants = useMemo<Participant[]>(() => {
    if (!state || !appearance) return [];
    return state.participants
      .filter((participant) =>
        appearance.showOwnUser ? true : !participant.isSelf,
      )
      .filter((participant) =>
        appearance.showOnlySpeaking ? participant.speaking : true,
      );
  }, [state, appearance]);

  if (!config || !appearance) {
    return null;
  }

  const scale = appearance.scale / 100;
  const clickThrough = config.behavior.clickThrough;
  const locked = config.behavior.lockPosition;
  const interactive = !clickThrough;

  const style = {
    "--gap": `${Math.round(appearance.gap * scale)}px`,
    "--radius": `${Math.round(appearance.cornerRadius * scale)}px`,
    "--avatar-size": `${Math.round(appearance.avatarSize * scale)}px`,
    "--font-size": `${(13 * scale).toFixed(1)}px`,
    opacity: appearance.opacity / 100,
  } as CSSProperties;

  const channelName = state?.channel?.name;
  const showEmptyState =
    config.general.showConnectionStatus && participants.length === 0;

  const statusText = (() => {
    if (!status) return "Connecting…";
    switch (status.state) {
      case "ready":
        return channelName
          ? `#${channelName} — nobody else is here yet`
          : "Not in a voice channel";
      case "waitingForDiscord":
        return "Waiting for Discord";
      case "authorizing":
        return "Approve the prompt in Discord";
      case "authenticating":
      case "connecting":
        return "Connecting to Discord";
      case "needsSetup":
        return "Discord needs to be set up";
      case "starting":
        return "Starting…";
      case "stopped":
        return "Overlay stopped";
    }
  })();

  const run = async (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    try {
      await action();
    } catch (error) {
      console.error(error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="overlay" style={style}>
      {interactive ? (
        <div
          className="overlay-toolbar"
          onMouseDown={(event) => {
            if (event.button !== 0) return;
            void run(() => startDragging(locked));
          }}
        >
          <span className={`overlay-grip ${locked ? "is-locked" : ""}`}>
            {locked ? <LockIcon /> : <GripIcon />}
          </span>
          <span className="overlay-channel">
            {channelName ? `#${channelName}` : "Discord voice"}
          </span>
          <span className="overlay-actions">
            <button
              type="button"
              className="icon-button"
              title={clickThrough ? "Enable interaction" : "Enable click-through"}
              onClick={(event) => {
                event.stopPropagation();
                void run(() => call("toggle_click_through"));
              }}
            >
              {clickThrough ? <PointerOffIcon /> : <PointerIcon />}
            </button>
            <button
              type="button"
              className="icon-button"
              title={locked ? "Unlock position" : "Lock position"}
              onClick={(event) => {
                event.stopPropagation();
                void run(() => call("set_locked", { value: !locked }));
              }}
            >
              {locked ? <UnlockIcon /> : <LockIcon />}
            </button>
            <button
              type="button"
              className="icon-button"
              title="Reset position"
              onClick={(event) => {
                event.stopPropagation();
                void run(() => call("reset_position"));
              }}
            >
              <RefreshIcon />
            </button>
            <button
              type="button"
              className="icon-button"
              title="Settings"
              onClick={(event) => {
                event.stopPropagation();
                void run(() => openSettings());
              }}
            >
              <SettingsIcon />
            </button>
          </span>
        </div>
      ) : null}

      <div
        className={`overlay-participants layout-${appearance.layout} ${
          config.placement.anchor.startsWith("bottom") ||
          config.placement.anchor.startsWith("middle")
            ? "align-bottom"
            : "align-top"
        }`}
        data-tauri-drag-region={interactive && !locked ? "" : undefined}
      >
        {participants.map((participant) => (
          <ParticipantRow
            key={participant.id}
            participant={participant}
            appearance={appearance}
            compact={appearance.layout !== "stack"}
          />
        ))}

        {showEmptyState ? (
          <div className="overlay-empty">
            <span className="overlay-empty-icon">
              <MicIcon />
            </span>
            <span>{statusText}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
