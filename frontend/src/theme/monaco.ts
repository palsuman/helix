/**
 * Monaco ↔ Helix theme bridge (Task 2.5, REQ-THEME-001.4/.6).
 *
 * Monaco is imported lazily so the editor never enters the initial bundle;
 * the dynamic import only runs once an editor surface exists. Until then the
 * bridge records the pending theme and applies it at first import.
 */

import type { ResolvedTheme } from "./service";
import { resolveValue } from "./resolve";

export interface MonacoThemeLike {
  editor: {
    defineTheme(name: string, data: unknown): void;
    setTheme(name: string): void;
  };
}

let monacoPromise: Promise<MonacoThemeLike> | null = null;
let pending: ResolvedTheme | null = null;

/** Injected loader; tests substitute a stub. */
export type MonacoLoader = () => Promise<MonacoThemeLike>;

/**
 * The real loader. Monaco lands with Task 4.1; until then the specifier is
 * built at runtime so Vite does not try to resolve a package that is not in
 * node_modules yet, and the failure path simply keeps the CSS theme.
 */
async function defaultLoader(): Promise<MonacoThemeLike> {
  const specifier = "monaco-editor";
  return import(/* @vite-ignore */ specifier) as unknown as Promise<MonacoThemeLike>;
}

/**
 * Convert a resolved Helix theme into Monaco's `IStandaloneThemeData` shape,
 * mapping our TextMate-ish token names onto Monaco's scopes.
 */
export function toMonacoThemeData(theme: ResolvedTheme): Record<string, unknown> {
  const get = (role: string): string => theme.tokens.get(role) ?? "#000000";
  const rules: Array<Record<string, unknown>> = [];
  const tokenColors = theme.document.editor?.tokenColors ?? {};
  for (const [token, value] of Object.entries(tokenColors)) {
    const color =
      resolveValue(value, theme.document) ?? get("editor.foreground");
    // Monaco rule tokens accept dotted scope prefixes ("comment", "string",
    // "keyword.control", …) directly.
    rules.push({ token, foreground: color.replace("#", "") });
  }
  return {
    base: theme.document.type.startsWith("hc")
      ? "hc-black"
      : isDark(theme)
        ? "vs-dark"
        : "vs",
    inherit: true,
    rules,
    colors: {
      "editor.background": get("editor.background"),
      "editor.foreground": get("editor.foreground"),
      "editor.lineHighlightBackground": get("editor.lineHighlight"),
      "editor.selectionBackground": get("editor.selection"),
      "editor.wordHighlightBackground": get("editor.wordHighlight"),
      "editor.findMatchBackground": get("editor.findMatch"),
      "editor.findMatchHighlightBackground": get("editor.findMatch"),
      "editorBracketHighlight.foreground1": get("bracket.level.1"),
      "editorBracketHighlight.foreground2": get("bracket.level.2"),
      "editorBracketHighlight.foreground3": get("bracket.level.3"),
      "editorBracketHighlight.foreground4": get("bracket.level.4"),
      "editorBracketHighlight.foreground5": get("bracket.level.5"),
      "editorBracketHighlight.foreground6": get("bracket.level.6"),
      "diffEditor.insertedTextBackground": get("diff.added.background"),
      "diffEditor.removedTextBackground": get("diff.removed.background"),
      "editorError.foreground": get("diagnostic.error"),
      "editorWarning.foreground": get("diagnostic.warning"),
      "editorInfo.foreground": get("diagnostic.info"),
      "editorGutter.background": get("editor.background"),
    },
  };
}

function isDark(theme: ResolvedTheme): boolean {
  return theme.document.type === "dark" || theme.document.type === "hc-dark";
}

export function monacoThemeName(helixName: string): string {
  return `helix-${helixName.toLowerCase().replaceAll(/\s+/g, "-")}`;
}

/**
 * Apply a resolved theme to Monaco. Without a live Monaco instance (tests,
 * or no editor mounted yet) the theme is remembered and applied on the next
 * {@link ensureMonocoTheme} call.
 */
export function applyMonacoTheme(
  theme: ResolvedTheme,
  loader: MonacoLoader = defaultLoader,
): void {
  pending = theme;
  void ensureMonacoTheme(loader);
}

async function ensureMonacoTheme(loader: MonacoLoader): Promise<void> {
  if (!pending || monacoPromise) {
    if (!monacoPromise && pending) monacoPromise = loader();
    else if (!monacoPromise) return;
  } else {
    monacoPromise = loader();
  }
  try {
    const monaco = await monacoPromise;
    if (!pending) return;
    const name = monacoThemeName(pending.name);
    monaco.editor.defineTheme(name, toMonacoThemeData(pending));
    monaco.editor.setTheme(name);
  } catch {
    // Monaco unavailable (headless test, lazy chunk rejected): the CSS
    // variables still carry the theme; retry on the next apply.
    monacoPromise = null;
  }
}
