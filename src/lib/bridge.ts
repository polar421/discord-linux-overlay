import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { mockCall, mockListen } from "./mock";

/**
 * True inside the Tauri web view. When false (plain `npm run dev` in a browser)
 * every call is served by the mock harness so the UI can be reviewed without a
 * Discord client or a Linux session.
 */
export const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI_IPC__" in window);

export async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isTauri) {
    return mockCall<T>(command, args);
  }
  return invoke<T>(command, args);
}

export async function on<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  if (!isTauri) {
    return mockListen<T>(event, handler);
  }
  return listen<T>(event, (incoming) => handler(incoming.payload));
}

export const openSettings = () => call<void>("open_settings");
export const closeSettings = () => call<void>("close_settings");
