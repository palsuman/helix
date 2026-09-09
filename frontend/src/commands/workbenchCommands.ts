import { notify } from "../notifications";
import { message } from "../localization/message";
import {
  executeLayoutProfileCommand,
  executeWindowHostCommand,
  LAYOUT_PROFILE_COMMANDS,
  listLayoutProfiles,
  WINDOW_HOST_COMMANDS,
} from "../workbench";
import type { CommandRegistry } from "./registry";
import { useLayoutStore } from "../workbench/layoutStore";
import { revealInExplorer } from "../explorer/model";

function argumentsObject(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
}

function stringArgument(value: unknown, key: string): string | undefined {
  const candidate = argumentsObject(value)[key];
  return typeof candidate === "string" && candidate.trim() !== "" ? candidate.trim() : undefined;
}

function ask(message: string, initialValue = ""): string | undefined {
  const value = globalThis.prompt?.(message, initialValue)?.trim();
  return value === undefined || value === "" ? undefined : value;
}

function profileName(argumentsValue: unknown, message: string): string | undefined {
  return stringArgument(argumentsValue, "name") ?? ask(message);
}

export function registerWorkbenchCommandHandlers(registry: CommandRegistry): void {
  registry.registerHandler("workbench.action.revealInExplorer", (args) =>
    revealInExplorer(stringArgument(args, "path")),
  );
  registry.registerHandler("workbench.action.togglePanel", () => {
    const layout = useLayoutStore.getState();
    layout.setPanelVisible(!layout.panelVisible);
  });
  registry.registerHandler("editor.action.formatDocument", () => {
    notify({
      kind: "warning",
      source: message("workbenchEditorSource"),
      message: message("workbenchNoFormatter"),
    });
  });

  for (const command of LAYOUT_PROFILE_COMMANDS) {
    registry.registerHandler(command.id, (argumentsValue) => {
      switch (command.id) {
        case "workbench.layoutProfile.save": {
          const name = profileName(argumentsValue, message("workbenchSaveProfilePrompt"));
          if (name !== undefined) executeLayoutProfileCommand(command.id, { name });
          return;
        }
        case "workbench.layoutProfile.switch": {
          const profiles = listLayoutProfiles();
          const name = profileName(
            argumentsValue,
            message("workbenchSwitchProfilePrompt", {
              profiles: profiles.map((profile) => profile.name).join(", "),
            }),
          );
          if (name !== undefined) executeLayoutProfileCommand(command.id, { name });
          return;
        }
        case "workbench.layoutProfile.rename": {
          const name = profileName(argumentsValue, message("workbenchRenameProfilePrompt"));
          const nextName =
            stringArgument(argumentsValue, "nextName") ??
            ask(message("workbenchNewProfileNamePrompt"));
          if (name !== undefined && nextName !== undefined) {
            executeLayoutProfileCommand(command.id, { name, nextName });
          }
          return;
        }
        case "workbench.layoutProfile.delete": {
          const name = profileName(argumentsValue, message("workbenchDeleteProfilePrompt"));
          if (name !== undefined) executeLayoutProfileCommand(command.id, { name });
          return;
        }
        case "workbench.layoutProfile.list": {
          const profiles = executeLayoutProfileCommand(command.id) as ReturnType<
            typeof listLayoutProfiles
          >;
          notify({
            kind: "info",
            source: message("workbenchSource"),
            message:
              profiles.length === 0
                ? message("workbenchNoProfiles")
                : message("workbenchProfileList", {
                    profiles: profiles.map((profile) => profile.name).join(", "),
                  }),
          });
          return profiles;
        }
        case "workbench.action.toggleZenMode":
          return executeLayoutProfileCommand(command.id);
      }
    });
  }

  for (const command of WINDOW_HOST_COMMANDS) {
    registry.registerHandler(command.id, (argumentsValue) => {
      if (command.id === "workbench.action.openFolderInNewWindow") {
        const path =
          stringArgument(argumentsValue, "path") ?? ask(message("workbenchFolderPathPrompt"));
        if (path === undefined) return;
        return executeWindowHostCommand(command.id, { path });
      }
      return executeWindowHostCommand(command.id);
    });
  }
}
