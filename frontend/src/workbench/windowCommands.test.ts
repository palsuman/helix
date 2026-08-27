import { describe, expect, it } from "vitest";
import type { InvokeFn } from "../ipc";
import { executeWindowHostCommand } from "./windowCommands";

describe("window host commands", () => {
  it("opens a folder in a new window and moves an editor via the host", async () => {
    const calls: string[] = [];
    const invoke: InvokeFn = async <T>(command: string) => {
      calls.push(command);
      return "w-9" as T;
    };
    await expect(
      executeWindowHostCommand(
        "workbench.action.openFolderInNewWindow",
        { path: "/tmp/a" },
        invoke,
      ),
    ).resolves.toBe("w-9");
    await expect(
      executeWindowHostCommand("workbench.action.moveEditorToNewWindow", {}, invoke),
    ).resolves.toBe("w-9");
    expect(calls).toEqual(["window_open_folder", "window_move_editor_to_new"]);
  });
});
