import type { CommandDescriptor } from "../generated/CommandDescriptor";
import type { LocalizationService } from "./service";

export function localizeCommand(
  command: CommandDescriptor,
  localization: LocalizationService,
): CommandDescriptor {
  return {
    ...command,
    title: command.title_message_id
      ? localization.formatMessage({ id: command.title_message_id, defaultMessage: command.title })
      : command.title,
    category: command.category_message_id
      ? localization.formatMessage({
          id: command.category_message_id,
          defaultMessage: command.category,
        })
      : command.category,
    disabled_reason: command.disabled_reason_message_id
      ? localization.formatMessage({
          id: command.disabled_reason_message_id,
          defaultMessage: command.disabled_reason ?? undefined,
        })
      : command.disabled_reason,
  };
}
