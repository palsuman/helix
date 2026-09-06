import { describe, expect, it } from "vitest";
import type { CommandDescriptor } from "../generated/CommandDescriptor";
import { groupCommands, rankCommands } from "./ranking";

function command(
  id: string,
  title: string,
  category = "View",
  enablement: string | null = null,
): CommandDescriptor {
  return {
    id,
    title,
    category,
    enablement,
    disabled_reason: enablement === null ? null : "Unavailable",
    keybinding: null,
    source: "Tests",
    target: { kind: "renderer" },
  };
}

describe("command ranking", () => {
  it("fuzzy-ranks Format Document for 'form' and groups categories", () => {
    const ranked = rankCommands(
      [
        command("view.toggle", "Toggle Panel"),
        command("editor.format", "Format Document", "Editor"),
        command("file.open", "Open File", "File"),
      ],
      "form",
      [],
      {},
    );

    expect(ranked[0]?.command.title).toBe("Format Document");
    expect(groupCommands(ranked)[0]?.category).toBe("Editor");
  });

  it("puts recent commands first for an empty query and reports disabled reasons", () => {
    const ranked = rankCommands(
      [command("a", "Alpha"), command("b", "Beta", "Editor", "editorTextFocus")],
      "",
      ["b"],
      { editorTextFocus: false },
    );
    expect(ranked.map((entry) => entry.command.id)).toEqual(["b", "a"]);
    expect(ranked[0]).toMatchObject({ enabled: false, disabledReason: "Unavailable" });
  });

  it("answers a large command catalog within the per-keystroke budget", () => {
    const commands = Array.from({ length: 2_000 }, (_, index) =>
      command(`command.${index}`, `Workspace Command ${index}`, "Workspace"),
    );
    commands.push(command("editor.format", "Format Document", "Editor"));
    const started = performance.now();
    const ranked = rankCommands(commands, "form", [], {});
    const elapsed = performance.now() - started;

    expect(ranked[0]?.command.id).toBe("editor.format");
    expect(elapsed).toBeLessThan(50);
  });
});
