import type { CSSProperties } from "react";

import type { AppearanceConfig, Participant } from "../lib/types";
import { BotIcon, HeadphonesOffIcon, MicOffIcon } from "./Icons";

const FALLBACK_AVATAR = "/img/default-avatar.svg";

function truncate(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  return `${value.slice(0, Math.max(1, maxLength - 1))}…`;
}

export function ParticipantRow({
  participant,
  appearance,
  compact = false,
}: {
  participant: Participant;
  appearance: AppearanceConfig;
  /** Avatar-only mode used by the row/grid layouts. */
  compact?: boolean;
}) {
  const micOff =
    participant.selfMuted || participant.muted || participant.suppressed;
  const deafened = participant.selfDeafened || participant.deafened;
  const showName = appearance.showNames && !compact;

  const style: CSSProperties = {
    borderRadius: `${appearance.cornerRadius}px`,
  };

  return (
    <div
      className={[
        "participant",
        participant.speaking ? "is-speaking" : "",
        micOff ? "is-muted" : "",
        appearance.background === "none" ? "no-background" : "",
        compact ? "is-compact" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      style={style}
      title={`${participant.displayName}${micOff ? " — muted" : ""}${
        deafened ? " — deafened" : ""
      }`}
    >
      <span className="participant-avatar">
        <img
          src={participant.avatarUrl}
          alt=""
          draggable={false}
          onError={(event) => {
            const image = event.currentTarget;
            if (image.src.endsWith(FALLBACK_AVATAR)) return;
            image.src = FALLBACK_AVATAR;
          }}
        />
        {!showName && (micOff || deafened) ? (
          <span className="participant-badge">
            {deafened ? <HeadphonesOffIcon /> : <MicOffIcon />}
          </span>
        ) : null}
      </span>

      {showName ? (
        <span className="participant-name">
          <span className="participant-label">
            {truncate(participant.displayName, appearance.maxNameLength)}
          </span>
          <span className="participant-icons">
            {micOff ? (
              <MicOffIcon className={participant.muted ? "danger" : undefined} />
            ) : null}
            {deafened ? (
              <HeadphonesOffIcon
                className={participant.deafened ? "danger" : undefined}
              />
            ) : null}
            {participant.bot ? <BotIcon /> : null}
          </span>
        </span>
      ) : null}
    </div>
  );
}
