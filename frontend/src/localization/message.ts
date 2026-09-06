import { messages, type MessageKey } from "./messages";
import { localization } from "./service";

export function message(key: MessageKey, values?: Record<string, string | number | Date>): string {
  const descriptor = messages[key];
  return descriptor
    ? localization.formatMessage(descriptor, values)
    : localization.format("localization.missing");
}
