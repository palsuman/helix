import { describe, expect, it } from "vitest";
import type { CommandDescriptor } from "../generated/CommandDescriptor";
import { localizeCommand } from "./commands";
import { LocalizationService, PSEUDO_LOCALE } from "./service";

describe("localized command metadata", () => {
  it("localizes built-in metadata IDs and retains plugin fallbacks", () => {
    const localization = new LocalizationService();
    localization.activate(PSEUDO_LOCALE);
    const builtIn: CommandDescriptor = {
      id: "editor.action.formatDocument",
      title: "Format Document",
      title_message_id: "command.editor.action.formatDocument.title",
      category: "Editor",
      category_message_id: "command.category.editor",
      enablement: "editorTextFocus",
      disabled_reason: "Open a text editor to format a document.",
      disabled_reason_message_id: "command.editor.action.formatDocument.disabled",
      keybinding: null,
      source: "Helix",
      target: { kind: "renderer" },
    };
    const localized = localizeCommand(builtIn, localization);
    expect(localized.title).toMatch(/^［/);
    expect(localized.category).toMatch(/^［/);
    expect(localized.disabled_reason).toMatch(/^［/);

    const plugin: CommandDescriptor = {
      id: "plugin.run",
      title: "Run Plugin",
      title_message_id: "plugin.run.title",
      category: "Plugin",
      category_message_id: "plugin.category",
      enablement: null,
      disabled_reason: null,
      keybinding: null,
      source: "example.plugin",
      target: { kind: "renderer" },
    };
    expect(localizeCommand(plugin, localization).title).toBe("Run Plugin");
    localization.registerCatalog(
      PSEUDO_LOCALE,
      { "plugin.run.title": "［Řüñ Þļüğïñ］", "plugin.category": "［Þļüğïñ］" },
      "example.plugin",
    );
    expect(localizeCommand(plugin, localization).title).toBe("［Řüñ Þļüğïñ］");
  });
});
