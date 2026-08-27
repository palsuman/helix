import { describe, expect, it } from "vitest";
import { applyMonacoTheme, monacoThemeName, toMonacoThemeData } from "./monaco";
import { resolveTheme } from "./resolve";
import { BUILTIN_THEMES, DEFAULT_DARK_THEME } from "./themes";
import type { ResolvedTheme } from "./service";
import type { MonacoThemeLike } from "./monaco";

function resolved(name = DEFAULT_DARK_THEME): ResolvedTheme {
  const doc = BUILTIN_THEMES.get(name)!;
  const { tokens } = resolveTheme(doc);
  return { name: doc.name, document: doc, tokens };
}

describe("monaco bridge", () => {
  it("names themes safely for Monaco", () => {
    expect(monacoThemeName("Helix Dark")).toBe("helix-helix-dark");
    expect(monacoThemeName("High Contrast Light")).toBe(
      "helix-high-contrast-light",
    );
  });

  it("maps a dark theme onto the vs-dark base", () => {
    const data = toMonacoThemeData(resolved()) as {
      base: string;
      colors: Record<string, string>;
      rules: Array<{ token: string; foreground: string }>;
    };
    expect(data.base).toBe("vs-dark");
    expect(data.colors["editor.background"]).toMatch(/^#/);
    expect(data.rules.some((rule) => rule.token === "keyword")).toBe(true);
  });

  it("maps a light theme onto the vs base and hc onto hc-black", () => {
    expect((toMonacoThemeData(resolved("Helix Light")) as { base: string }).base).toBe("vs");
    expect(
      (toMonacoThemeData(resolved("High Contrast Dark")) as { base: string }).base,
    ).toBe("hc-black");
  });

  it("covers six bracket levels (REQ-THEME-002.3)", () => {
    const data = toMonacoThemeData(resolved()) as {
      colors: Record<string, string>;
    };
    for (let level = 1; level <= 6; level += 1) {
      expect(data.colors[`editorBracketHighlight.foreground${level}`]).toMatch(/^#/);
    }
  });

  it("defines and applies the theme once Monaco loads", async () => {
    const defined: unknown[] = [];
    const set: string[] = [];
    const monaco: MonacoThemeLike = {
      editor: {
        defineTheme: (_name: string, data: unknown) => defined.push(data),
        setTheme: (name: string) => set.push(name),
      },
    };
    applyMonacoTheme(resolved(), async () => monaco);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(defined).toHaveLength(1);
    expect(set).toEqual(["helix-helix-dark"]);
  });
});
