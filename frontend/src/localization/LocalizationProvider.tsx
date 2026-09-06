import { useEffect, useSyncExternalStore, type ReactNode } from "react";
import { RawIntlProvider } from "react-intl";
import type { IpcClient } from "../ipc";
import type { StreamClient } from "../stream";
import { CONFIG_CHANNEL } from "../config/channel";
import type { ConfigChange } from "../generated/ConfigChange";
import { getConfigValueFrom } from "../config";
import { notify } from "../notifications";
import { LocalizationContext } from "./context";
import { messages } from "./messages";
import { LOCALE_SETTING, LocalizationService, localization } from "./service";

export function LocalizationProvider({
  children,
  client,
  stream,
  service = localization,
}: {
  children: ReactNode;
  client: IpcClient;
  stream: StreamClient | null;
  service?: LocalizationService;
}) {
  const snapshot = useSyncExternalStore(
    (listener) => service.subscribe(listener),
    () => service.current,
    () => service.current,
  );

  useEffect(() => {
    let active = true;
    void getConfigValueFrom<string>(client, LOCALE_SETTING).then(
      (locale) => {
        if (!active) return;
        const applied = service.activate(locale);
        if (applied.notice)
          notify({
            kind: "warning",
            source: service.formatMessage(messages.localizationSource),
            message: applied.notice,
          });
      },
      () => {
        if (active) service.activate("auto");
      },
    );
    const unsubscribe = stream?.subscribe<ConfigChange>(CONFIG_CHANNEL, (change) => {
      if (!change.changed_keys.includes(LOCALE_SETTING)) return;
      notify({
        kind: "info",
        source: service.formatMessage(messages.localizationSource),
        message: service.format("localization.restartRequired"),
      });
    });
    return () => {
      active = false;
      unsubscribe?.();
    };
  }, [client, service, stream]);

  return (
    <LocalizationContext.Provider value={service}>
      <RawIntlProvider value={snapshot.intl}>{children}</RawIntlProvider>
    </LocalizationContext.Provider>
  );
}
