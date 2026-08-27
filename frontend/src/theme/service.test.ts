import { describe, expect, it, vi } from "vitest";
import { ThemeService, type MediaQueryLike } from "./service";
import { BUILTIN_THEMES, DEFAULT_DARK_THEME } from "./themes";

function mediaQuery(matches: boolean): MediaQueryLike & { set(matches: boolean): void } {
  const listeners = new Set<() => void>();
  const query = {
    matches,
    addEventListener(listener: () => void) {
      listeners.add(listener);
    },
    removeEventListener(listener: () => void) {
      listeners.delete(listener);
    },
    set(next: boolean) {
      query.matches = next;
      for (const listener of listeners) listener();
    },
  };
  return query;
}

interface Harness {
  service: ThemeService;
  target: HTMLElement;
  darkQuery: ReturnType<typeof mediaQuery>;
  contrastQuery: ReturnType<typeof mediaQuery>;
  colorTheme: { value: string | null };
  applied: string[];
}

function harness(initialThemeSetting: string | null = null): Harness {
  const target = document.createElement("div");
  const darkQuery = mediaQuery(true);
  const contrastQuery = mediaQuery(false);
  const colorTheme = { value: initialThemeSetting };
  const applied: string[] = [];
  const service = new ThemeService({
    config: {
      getColorTheme: async () => colorTheme.value,
      setColorTheme: async (name: string) => {
        colorTheme.value = name;
      },
    },
    matchMedia: (query: string) =>
      query.includes("contrast") ? contrastQuery : darkQuery,
    target,
    onApply: (name) => applied.push(name),
  });
  return { service, target, darkQuery, contrastQuery, colorTheme, applied };
}

describe("theme service", () => {
  it("applies the configured theme from settings", async () => {
    const h = harness("Helix Light");
    const theme = await h.service.initialize();
    expect(theme.name).toBe("Helix Light");
    expect(h.service.isDark).toBe(false);
    expect(h.target.style.getPropertyValue("--helix-accent")).toMatch(/^#/);
    expect(h.target.style.colorScheme).toBe("light");
  });

  it("falls back to the OS preference when settings hold no choice", async () => {
    const h = harness(null);
    const theme = await h.service.initialize();
    expect(theme.name).toBe(DEFAULT_DARK_THEME);
  });

  it("picks the light theme when the OS prefers light", async () => {
    const h = harness(null);
    h.darkQuery.set(false);
    const preferred = await h.service.osPreferred();
    expect(preferred).toBe("Helix Light");
  });

  it("switches to a high-contrast theme when the OS asks for contrast", async () => {
    const h = harness(null);
    h.contrastQuery.set(true);
    expect(await h.service.osPreferred()).toBe("High Contrast Dark");
    h.darkQuery.set(false);
    expect(await h.service.osPreferred()).toBe("High Contrast Light");
  });

  it("follows OS changes until the user chooses explicitly", async () => {
    const h = harness(null);
    await h.service.initialize();
    h.service.followOsPreference();
    h.darkQuery.set(false);
    // The listener applies asynchronously.
    await vi.waitFor(() => expect(h.service.current.name).toBe("Helix Light"));
  });

  it("paints a switch by swapping variables only — no layout properties", async () => {
    const h = harness();
    await h.service.initialize();
    const before = h.target.style.cssText;
    await h.service.apply("Helix Light");
    const after = h.target.style.cssText;
    // Only custom properties and color-scheme change; nothing structural.
    expect(after).not.toBe(before);
    for (const declaration of ["width", "height", "display", "position"]) {
      expect(h.target.style.getPropertyValue(declaration)).toBe("");
    }
  });

  it("falls back to the default dark theme for an unknown name", async () => {
    const h = harness();
    const theme = await h.service.apply("Nonexistent Theme");
    expect(theme.name).toBe(DEFAULT_DARK_THEME);
  });

  it("previews without committing and commit restores the selection", async () => {
    const h = harness("Helix Dark");
    await h.service.initialize();
    h.service.preview("Helix Light");
    expect(
      h.target.style.getPropertyValue("--helix-frame-background"),
    ).not.toBe(BUILTIN_THEMES.get("Helix Dark")!.semantic?.["frame.background"]);
    h.service.commit();
    const darkAccent = h.service.current;
    expect(darkAccent.name).toBe("Helix Dark");
  });

  it("persists an explicit choice through the config client", async () => {
    const h = harness();
    await h.service.select("Helix Light");
    expect(h.colorTheme.value).toBe("Helix Light");
    expect(h.service.current.name).toBe("Helix Light");
  });

  it("applies a theme changed in another window via the config stream", async () => {
    const h = harness("Helix Dark");
    await h.service.initialize();
    await h.service.onConfigChanged("Helix Light");
    expect(h.service.current.name).toBe("Helix Light");
    // Same value again is a no-op.
    const before = h.applied.length;
    await h.service.onConfigChanged("Helix Light");
    expect(h.applied.length).toBe(before);
  });
});
