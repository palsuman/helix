import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { InvokeFn } from "../ipc";
import { ipc } from "../ipc";
import { HOST_WINDOW_COMMANDS, WindowClient } from "../windows";

export const WINDOW_HOST_COMMANDS = [
  { id: "workbench.action.newWindow", title: "New Window" },
  { id: "workbench.action.openFolderInNewWindow", title: "Open Folder in New Window" },
  {
    id: "workbench.action.duplicateWorkspaceInNewWindow",
    title: "Duplicate Workspace in New Window",
  },
  { id: "workbench.action.closeWindow", title: "Close Window" },
  { id: "workbench.action.moveEditorToNewWindow", title: "Move Editor to New Window" },
] as const;

export type WindowHostCommandId = (typeof WINDOW_HOST_COMMANDS)[number]["id"];

export interface WindowHostCommandArguments {
  path?: string;
}

/** Host window commands consumed by Task 2.8's registry and palette. */
export async function executeWindowHostCommand(
  id: WindowHostCommandId,
  args: WindowHostCommandArguments = {},
  invokeFn: InvokeFn = tauriInvoke as InvokeFn,
): Promise<string | void> {
  const client = new WindowClient(ipc, invokeFn);
  switch (id) {
    case "workbench.action.newWindow":
      return client.newWindow();
    case "workbench.action.openFolderInNewWindow":
      if (args.path === undefined) {
        throw new Error("Open Folder in New Window requires 'path'.");
      }
      return client.openFolder(args.path, true);
    case "workbench.action.duplicateWorkspaceInNewWindow":
      return client.duplicateWorkspace();
    case "workbench.action.closeWindow":
      await client.close();
      return undefined;
    case "workbench.action.moveEditorToNewWindow":
      return invokeFn<string>(HOST_WINDOW_COMMANDS.moveEditorToNew);
  }
}
