import { describe, expect, it } from "vitest";
import { defaultBindings, importScheme, SCHEMES, schemeBindings } from "./schemes";
import { resolveBindings } from "./resolver";

describe("keybinding schemes", () => {
  it("uses platform-specific palette and format defaults", () => {
    expect(defaultBindings("mac")[0].key).toBe("meta+shift+p");
    expect(defaultBindings("windows")[0].key).toBe("ctrl+shift+p");
    expect(
      defaultBindings("linux").find((rule) => rule.command === "editor.action.formatDocument")?.key,
    ).toBe("ctrl+shift+i");
  });

  it.each(SCHEMES)("imports %s without duplicates or losing unrelated user rules", (scheme) => {
    const defaults = { source: "default" as const, owner: "Helix", rules: defaultBindings("mac") };
    const original = resolveBindings([defaults], "mac").bindings;
    const user = [{ key: "meta+1", command: "custom.run" }];
    const imported = importScheme(user, original, scheme, "mac");
    const current = resolveBindings(
      [defaults, { source: "user", owner: "User", rules: imported }],
      "mac",
    );
    expect(current.warnings).toEqual([]);
    expect(current.bindings.some((rule) => rule.command === "custom.run")).toBe(true);
    expect(importScheme(imported, current.bindings, scheme, "mac")).toEqual(imported);
    expect(schemeBindings(scheme, "mac").length).toBeGreaterThan(0);
  });

  it("only enables Vim motions in normal mode and Emacs motions in editors", () => {
    expect(
      schemeBindings("Vim (basic motions)", "mac").find((rule) => rule.key === "h")?.when,
    ).toBe("editorTextFocus && vimMode == 'normal'");
    expect(
      schemeBindings("Emacs (basic)", "linux").find((rule) => rule.key === "ctrl+f")?.command,
    ).toBe("cursorRight");
  });
});
