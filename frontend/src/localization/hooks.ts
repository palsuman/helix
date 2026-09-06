import { useCallback, useContext, useSyncExternalStore } from "react";
import type { IntlShape } from "react-intl";
import { LocalizationContext } from "./context";
import { messages, type MessageKey } from "./messages";
import { localization, type LocalizationSnapshot, type LocalizationService } from "./service";

export function useLocalization(): LocalizationService {
  return useContext(LocalizationContext) ?? localization;
}

export function useLocalizationSnapshot(): LocalizationSnapshot {
  const service = useLocalization();
  return useSyncExternalStore(
    (listener) => service.subscribe(listener),
    () => service.current,
    () => service.current,
  );
}

export function useIntlShape(): IntlShape {
  return useLocalizationSnapshot().intl;
}

export function useMessage() {
  const service = useLocalization();
  useLocalizationSnapshot();
  return useCallback(
    (key: MessageKey, values?: Record<string, string | number | Date>) => {
      const descriptor = messages[key];
      return descriptor
        ? service.formatMessage(descriptor, values)
        : service.format("localization.missing");
    },
    [service],
  );
}
