/**
 * React binding for the theme service (Task 2.5).
 *
 * One hook per application root: initializes from settings + OS preference,
 * subscribes to `config:changed` so a theme picked in another window applies
 * here, and re-renders on every applied theme.
 */

import { useEffect, useState } from "react";
import type { StreamClient } from "../stream";
import { CONFIG_CHANNEL } from "../config/channel";
import type { ConfigChange } from "../generated/ConfigChange";
import { getConfigValue } from "../config";
import { COLOR_THEME_SETTING, ThemeService, type ResolvedTheme } from "./service";

export function useTheme(
  stream: StreamClient | null,
  service: ThemeService,
): ResolvedTheme {
  const [theme, setTheme] = useState<ResolvedTheme>(service.current);

  useEffect(() => {
    let disposed = false;
    void service.initialize().then((applied) => {
      if (!disposed) setTheme(applied);
    });
    const unsubscribe = stream?.subscribe<ConfigChange>(
      CONFIG_CHANNEL,
      (change) => {
        if (!change.changed_keys.includes(COLOR_THEME_SETTING)) return;
        void reapply();
      },
    );
    async function reapply() {
      const value = await getConfigValue<string>(COLOR_THEME_SETTING);
      await service.onConfigChanged(value);
      if (!disposed) setTheme(service.current);
    }
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [service, stream]);

  return theme;
}
