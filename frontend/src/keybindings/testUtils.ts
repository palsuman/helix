import type { CommandDescriptor } from "../generated/CommandDescriptor";
import type { IpcRequest } from "../generated/IpcRequest";
import type { KeybindingsSnapshot } from "../generated/KeybindingsSnapshot";
import type { KeybindingContribution } from "../generated/KeybindingContribution";
import type { KeybindingsSetRequest } from "../generated/KeybindingsSetRequest";
import { IpcClient, type InvokeFn } from "../ipc";
import { CommandRegistry } from "../commands/registry";
import { KeybindingService } from "./service";
import type { KeybindingPlatform, KeybindingRule } from "./resolver";

export function keybindingHarness(
  platform: KeybindingPlatform = "mac",
  user: KeybindingRule[] = [],
) {
  const definitions = [
    ["workbench.action.showCommands", "Show All Commands", null],
    ["workbench.action.openGlobalKeybindings", "Keyboard Shortcuts", null],
    ["workbench.action.toggleZenMode", "Toggle Zen Mode", null],
    ["workbench.action.newWindow", "New Window", null],
    ["workbench.action.closeWindow", "Close Window", null],
    ["workbench.action.togglePanel", "Toggle Panel", null],
    ["editor.action.formatDocument", "Format Document", "editorTextFocus"],
    ["cursorLeft", "Move Cursor Left", "editorTextFocus"],
    ["example.run", "Plugin Command", null],
  ];
  const commands: CommandDescriptor[] = definitions.map(([id, title, enablement]) => ({
    id: id!,
    title: title!,
    enablement,
    disabled_reason: "Focus a text editor.",
    category: "Workbench",
    keybinding: null,
    source: "Helix",
    target: { kind: "renderer" },
  }));
  let snapshot: KeybindingsSnapshot = {
    user,
    plugins: [],
    warnings: [],
    revision: "1",
    writable: true,
    path: "/home/test/.helix/keybindings.json",
  };
  const requests: IpcRequest<unknown>[] = [];
  const executed: { command: string; args: unknown }[] = [];
  const invoke: InvokeFn = async <T>(_endpoint: string, args?: Record<string, unknown>) => {
    const request = (args as { request: IpcRequest<unknown> }).request;
    requests.push(request);
    let result: unknown;
    switch (request.command) {
      case "window.layout.get":
        result = { layout: null };
        break;
      case "window.layout.set":
        result = request.payload;
        break;
      case "command.list":
        result = { commands };
        break;
      case "command.execute":
        result = { ...(request.payload as object), target: { kind: "renderer" } };
        break;
      case "keybindings.get":
        result = snapshot;
        break;
      case "keybindings.set": {
        const payload = request.payload as KeybindingsSetRequest;
        if (payload.revision !== snapshot.revision)
          throw new Error("Keybindings changed on disk. Reload before saving.");
        snapshot = {
          ...snapshot,
          user: payload.rules,
          revision: String(Number(snapshot.revision) + 1),
        };
        result = snapshot;
        break;
      }
      case "keybindings.contribute": {
        const contribution = request.payload as KeybindingContribution;
        snapshot = {
          ...snapshot,
          plugins: [
            ...snapshot.plugins.filter((plugin) => plugin.owner !== contribution.owner),
            contribution,
          ],
        };
        result = snapshot;
        break;
      }
      case "keybindings.removeContribution": {
        snapshot = {
          ...snapshot,
          plugins: snapshot.plugins.filter(
            (plugin) => plugin.owner !== (request.payload as { owner: string }).owner,
          ),
        };
        result = snapshot;
        break;
      }
      default:
        throw new Error(`Unexpected command: ${request.command}`);
    }
    return {
      correlation_id: request.correlation_id,
      result: structuredClone(result),
      error: null,
    } as T;
  };
  const client = new IpcClient({ invoke });
  const registry = new CommandRegistry(client, "test-window");
  commands.forEach((command) =>
    registry.registerHandler(command.id, (args) => {
      executed.push({ command: command.id, args });
    }),
  );
  const service = new KeybindingService(client, registry, "test-window", platform);
  return {
    client,
    registry,
    service,
    requests,
    executed,
    document: () => snapshot,
    externalEdit: (rules: KeybindingRule[]) => {
      snapshot = { ...snapshot, user: rules, revision: String(Number(snapshot.revision) + 1) };
    },
  };
}
