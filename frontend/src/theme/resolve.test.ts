import { describe, expect, it } from "vitest";
import {
  BUILTIN_THEMES,
  DEFAULT_DARK_THEME,
  type ThemeDocument,
} from "./themes";
import { cssVariableName, resolveTheme, resolveValue, toCssVariables } from "./resolve";

describe("token resolution", () => {
  it("resolves direct colors unchanged", () => {
    expect(resolveValue("#3574f0", emptyTheme())).toBe("#3574f0");
  });

  it("follows palette references", () => {
    const theme = withPalette({ "blue.500": "#3574f0" });
    expect(resolveValue("palette.blue.500", theme)).toBe("#3574f0");
  });

  it("follows chained role references", () => {
    const theme: ThemeDocument = {
      name: "t",
      type: "dark",
      palette: { "gray.900": "#1e1f22" },
      semantic: {
        background: "palette.gray.900",
        surface: "semantic.background",
      },
    };
    expect(resolveValue("semantic.surface", theme)).toBe("#1e1f22");
  });

  it("rejects reference cycles instead of hanging", () => {
    const theme: ThemeDocument = {
      name: "t",
      type: "dark",
      semantic: { a: "semantic.b", b: "semantic.a" },
    };
    expect(resolveValue("semantic.a", theme)).toBeNull();
  });

  it("returns null for a dead reference", () => {
    expect(resolveValue("palette.missing.thing", emptyTheme())).toBeNull();
  });
});

describe("theme resolution", () => {
  it("resolves every built-in theme without fallbacks", () => {
    for (const [name, doc] of BUILTIN_THEMES) {
      const result = resolveTheme(doc);
      expect(result.fellBack, `${name} should fully resolve`).toEqual([]);
      for (const color of result.tokens.values()) {
        expect(color).toMatch(/^#/);
      }
    }
  });

  it("falls back per-role to the default dark theme", () => {
    const partial: ThemeDocument = {
      name: "partial",
      type: "dark",
      palette: {},
      semantic: { accent: "#ff0000" },
    };
    const result = resolveTheme(partial);
    expect(result.tokens.get("accent")).toBe("#ff0000");
    // Everything else comes from Helix Dark.
    const dark = resolveTheme(BUILTIN_THEMES.get(DEFAULT_DARK_THEME)!);
    expect(result.tokens.get("card.background")).toBe(
      dark.tokens.get("card.background"),
    );
    expect(result.fellBack).toContain("card.background");
  });

  it("applies user overrides after everything else", () => {
    const themed: ThemeDocument = {
      ...emptyTheme(),
      semantic: { accent: "palette.blue.500" },
      palette: { "blue.500": "#3574f0" },
      overrides: { accent: "#00ff00" },
    };
    expect(resolveTheme(themed).tokens.get("accent")).toBe("#00ff00");
  });
});

describe("css variable mapping", () => {
  it("maps roles to kebab-case custom properties", () => {
    expect(cssVariableName("frame.background")).toBe("--helix-frame-background");
    expect(cssVariableName("bracket.level.1")).toBe("--helix-bracket-level-1");
  });

  it("produces a paintable record", () => {
    const { tokens } = resolveTheme(BUILTIN_THEMES.get(DEFAULT_DARK_THEME)!);
    const vars = toCssVariables(tokens);
    expect(vars["--helix-accent"]).toMatch(/^#/);
    expect(Object.keys(vars).length).toBe(tokens.size);
  });
});

function emptyTheme(): ThemeDocument {
  return { name: "empty", type: "dark", palette: {}, semantic: {} };
}

function withPalette(palette: Record<string, string>): ThemeDocument {
  return { name: "p", type: "dark", palette, semantic: {} };
}
