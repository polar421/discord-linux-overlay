import { useCallback, useEffect, useState } from "react";

import { call, on } from "./bridge";
import type {
  Config,
  ConnectionInfo,
  OverlayState,
  PlatformInfo,
  Status,
} from "./types";

function useSubscription<T>(
  event: string,
  initial: () => Promise<T>,
): T | null {
  const [value, setValue] = useState<T | null>(null);

  useEffect(() => {
    let alive = true;
    initial().then(
      (next) => {
        if (alive) setValue(next);
      },
      (error: unknown) => console.error(error),
    );

    const unsubscribe = on<T>(event, (payload) => {
      if (alive) setValue(payload);
    });

    return () => {
      alive = false;
      void unsubscribe.then((stop) => stop());
    };
    // `initial` is a stable module-level function at every call site.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [event]);

  return value;
}

export function useConfig() {
  const [config, setConfig] = useState<Config | null>(null);

  useEffect(() => {
    let alive = true;
    call<Config>("get_config").then(
      (next) => {
        if (alive) setConfig(next);
      },
      (error: unknown) => console.error(error),
    );
    const unsubscribe = on<Config>("config:changed", (next) => {
      if (alive) setConfig(next);
    });

    return () => {
      alive = false;
      void unsubscribe.then((stop) => stop());
    };
  }, []);

  const update = useCallback(
    async (patch: Record<string, unknown>): Promise<Config> => {
      const next = await call<Config>("update_config", { patch });
      setConfig(next);
      return next;
    },
    [],
  );

  return { config, update };
}

export function useOverlayState(): OverlayState | null {
  return useSubscription<OverlayState>("overlay:state", () =>
    call<OverlayState>("get_overlay_state"),
  );
}

export function useConnection(): ConnectionInfo | null {
  return useSubscription<ConnectionInfo>("connection:status", () =>
    call<ConnectionInfo>("get_connection"),
  );
}

export function useStatus(): Status | null {
  return useSubscription<Status>("connection:status", () =>
    call<Status>("get_status"),
  );
}

export function usePlatform(): PlatformInfo | null {
  return useSubscription<PlatformInfo>("platform", () =>
    call<PlatformInfo>("get_platform"),
  );
}
