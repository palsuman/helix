/**
 * Theme token model (Task 2.5, REQ-THEME-001, REQ-THEME-002).
 *
 * Three layers, resolved strictly in one direction:
 *
 *   palette   raw colors ("gray.900" → "#1a1a2e")
 *   semantic  named roles referencing palette entries ("background" → "gray.900")
 *   component workbench CSS variables referencing semantic roles
 *
 * A theme file supplies palette + semantic + editor tokens; the component
 * layer is fixed by the application so a theme author cannot break layout,
 * only recolor it. Resolution walks reference chains with cycle detection
 * and falls back per-token to the default dark theme (REQ-THEME-001 failure
 * modes), never leaving a token unresolved.
 */

/** Raw color values. Keys are dotted names like `gray.900`. */
export type ThemePalette = Record<string, string>;

/**
 * Semantic role references. Values are `palette.<name>` references or direct
 * colors; anything unresolvable falls back to the default theme's role.
 */
export type ThemeSemantic = Record<string, string>;

/**
 * Editor syntax colors: TextMate scopes and LSP semantic token types, plus
 * the code-presentation colors REQ-THEME-002 enumerates.
 */
export interface ThemeEditor {
  /** TextMate scope name → color or palette reference. */
  tokenColors?: Record<string, string>;
  /** LSP semantic token type (+ optional `.modifier`) → color or reference. */
  semanticTokenColors?: Record<string, string>;
}

/** The on-disk / over-IPC theme document. */
export interface ThemeDocument {
  name: string;
  /** `"dark"` | `"light"` | `"hc-dark"` | `"hc-light"`. */
  type: string;
  palette?: ThemePalette;
  semantic?: ThemeSemantic;
  editor?: ThemeEditor;
  /**
   * User overrides from settings (`workbench.themeOverrides`), applied after
   * everything else without authoring a full theme (REQ-THEME-001.10).
   */
  overrides?: ThemeSemantic;
}

/** The four built-in themes (REQ-THEME-001.3). */
export const BUILTIN_THEME_IDS = [
  "Helix Dark",
  "Helix Light",
  "High Contrast Dark",
  "High Contrast Light",
] as const;

export type BuiltinThemeId = (typeof BUILTIN_THEME_IDS)[number];

export const DEFAULT_DARK_THEME: BuiltinThemeId = "Helix Dark";

/** The theme used as the fallback source for every unresolvable token. */
export const FALLBACK_THEME: BuiltinThemeId = DEFAULT_DARK_THEME;

const HELIX_DARK: ThemeDocument = {
  name: "Helix Dark",
  type: "dark",
  palette: {
    "gray.950": "#16171a",
    "gray.900": "#1e1f22",
    "gray.850": "#26282c",
    "gray.800": "#2b2d30",
    "gray.700": "#393b40",
    "gray.600": "#4b4d52",
    "gray.400": "#9da0a8",
    "gray.200": "#dfe1e5",
    "blue.600": "#2b5fc7",
    "blue.500": "#3574f0",
    "blue.100": "#cfe0fb",
    "red.500": "#f05f57",
    "red.100": "#fcd9d8",
    "green.500": "#5fad65",
    "yellow.500": "#e3b341",
    "yellow.100": "#ffe08a",
    "purple.500": "#b180f0",
    "cyan.500": "#4dd0e1",
  },
  semantic: {
    "frame.background": "palette.gray.800",
    "card.background": "palette.gray.900",
    "card.border": "palette.gray.700",
    foreground: "palette.gray.200",
    "foreground.muted": "palette.gray.400",
    accent: "palette.blue.500",
    "accent.foreground": "palette.gray.950",
    "hover.background": "palette.gray.850",
    "focus.border": "palette.blue.600",
    "selection.background": "palette.blue.600",
    "error.foreground": "palette.red.500",
    "warning.foreground": "palette.yellow.500",
    "success.foreground": "palette.green.500",
    "notice.background": "palette.gray.850",
    "notice.border": "palette.yellow.500",
    "notice.foreground": "palette.yellow.100",
    // Editor surface (REQ-THEME-001.6/.7)
    "editor.background": "palette.gray.900",
    "editor.foreground": "palette.gray.200",
    "editor.lineHighlight": "palette.gray.850",
    "editor.selection": "palette.blue.600",
    "editor.wordHighlight": "palette.gray.700",
    "editor.findMatch": "palette.yellow.500",
    "editor.findMatchCurrent": "palette.red.500",
    // Diff (REQ-THEME-002.4)
    "diff.added.background": "#20303b",
    "diff.removed.background": "#372b31",
    "diff.modified.background": "#2b3040",
    // Git decorations (REQ-THEME-002.5)
    "git.modified": "palette.blue.500",
    "git.added": "palette.green.500",
    "git.untracked": "palette.green.500",
    "git.deleted": "palette.red.500",
    "git.conflict": "palette.yellow.500",
    "git.ignored": "palette.gray.600",
    // Diagnostics (REQ-THEME-002.6)
    "diagnostic.error": "palette.red.500",
    "diagnostic.warning": "palette.yellow.500",
    "diagnostic.info": "palette.blue.500",
    "diagnostic.hint": "palette.gray.400",
    // Brackets, min 6 levels (REQ-THEME-002.3)
    "bracket.level.1": "palette.cyan.500",
    "bracket.level.2": "palette.yellow.500",
    "bracket.level.3": "palette.purple.500",
    "bracket.level.4": "palette.green.500",
    "bracket.level.5": "palette.blue.500",
    "bracket.level.6": "palette.red.500",
    // Icons (REQ-THEME-002.9)
    "icon.foreground": "palette.gray.200",
    "icon.disabled": "palette.gray.600",
  },
  editor: {
    tokenColors: {
      comment: "palette.gray.400",
      keyword: "palette.purple.500",
      string: "palette.green.500",
      number: "palette.cyan.500",
      type: "palette.yellow.500",
      function: "palette.blue.100",
      variable: "palette.gray.200",
      constant: "palette.cyan.500",
      operator: "palette.gray.200",
      tag: "palette.blue.100",
      attribute: "palette.yellow.500",
    },
    semanticTokenColors: {
      namespace: "palette.yellow.500",
      class: "palette.yellow.500",
      enum: "palette.yellow.500",
      interface: "palette.yellow.500",
      struct: "palette.yellow.500",
      typeParameter: "palette.yellow.500",
      function: "palette.blue.100",
      method: "palette.blue.100",
      macro: "palette.purple.500",
      variable: "palette.gray.200",
      parameter: "palette.gray.200",
      property: "palette.gray.200",
      keyword: "palette.purple.500",
      comment: "palette.gray.400",
      string: "palette.green.500",
      number: "palette.cyan.500",
      operator: "palette.gray.200",
      "variable.readonly": "palette.cyan.500",
      "function.deprecated": "palette.gray.600",
    },
  },
};

const HELIX_LIGHT: ThemeDocument = {
  name: "Helix Light",
  type: "light",
  palette: {
    "gray.950": "#1a1c1e",
    "gray.900": "#2b2d30",
    "gray.850": "#494b4f",
    "gray.800": "#ffffff",
    "gray.700": "#dfe1e5",
    "gray.600": "#c4c6cc",
    "gray.400": "#6f737a",
    "gray.200": "#2b2d30",
    "blue.600": "#2b5fc7",
    "blue.500": "#3574f0",
    "blue.100": "#1a4fa0",
    "red.500": "#d33d29",
    "red.100": "#a63a2b",
    "green.500": "#3d8f45",
    "yellow.500": "#9a7b1c",
    "yellow.100": "#6b5510",
    "purple.500": "#8240c8",
    "cyan.500": "#0e7f96",
  },
  semantic: {
    "frame.background": "palette.gray.700",
    "card.background": "palette.gray.800",
    "card.border": "palette.gray.600",
    foreground: "palette.gray.200",
    "foreground.muted": "palette.gray.400",
    accent: "palette.blue.500",
    "accent.foreground": "palette.gray.800",
    "hover.background": "palette.gray.700",
    "focus.border": "palette.blue.600",
    "selection.background": "palette.blue.500",
    "error.foreground": "palette.red.500",
    "warning.foreground": "palette.yellow.500",
    "success.foreground": "palette.green.500",
    "notice.background": "palette.gray.800",
    "notice.border": "palette.yellow.500",
    "notice.foreground": "palette.yellow.100",
    "editor.background": "palette.gray.800",
    "editor.foreground": "palette.gray.950",
    "editor.lineHighlight": "palette.gray.700",
    "editor.selection": "palette.blue.500",
    "editor.wordHighlight": "palette.gray.600",
    "editor.findMatch": "palette.yellow.500",
    "editor.findMatchCurrent": "palette.red.500",
    "diff.added.background": "#dff0e0",
    "diff.removed.background": "#fbe0dc",
    "diff.modified.background": "#dde4f7",
    "git.modified": "palette.blue.500",
    "git.added": "palette.green.500",
    "git.untracked": "palette.green.500",
    "git.deleted": "palette.red.500",
    "git.conflict": "palette.yellow.500",
    "git.ignored": "palette.gray.600",
    "diagnostic.error": "palette.red.500",
    "diagnostic.warning": "palette.yellow.500",
    "diagnostic.info": "palette.blue.500",
    "diagnostic.hint": "palette.gray.400",
    "bracket.level.1": "palette.cyan.500",
    "bracket.level.2": "palette.yellow.500",
    "bracket.level.3": "palette.purple.500",
    "bracket.level.4": "palette.green.500",
    "bracket.level.5": "palette.blue.500",
    "bracket.level.6": "palette.red.500",
    "icon.foreground": "palette.gray.200",
    "icon.disabled": "palette.gray.600",
  },
  editor: {
    tokenColors: {
      comment: "palette.gray.400",
      keyword: "palette.purple.500",
      string: "palette.green.500",
      number: "palette.cyan.500",
      type: "palette.yellow.500",
      function: "palette.blue.100",
      variable: "palette.gray.950",
      constant: "palette.cyan.500",
      operator: "palette.gray.950",
      tag: "palette.blue.100",
      attribute: "palette.yellow.500",
    },
    semanticTokenColors: {
      namespace: "palette.yellow.500",
      class: "palette.yellow.500",
      enum: "palette.yellow.500",
      interface: "palette.yellow.500",
      struct: "palette.yellow.500",
      typeParameter: "palette.yellow.500",
      function: "palette.blue.100",
      method: "palette.blue.100",
      macro: "palette.purple.500",
      variable: "palette.gray.950",
      parameter: "palette.gray.950",
      property: "palette.gray.950",
      keyword: "palette.purple.500",
      comment: "palette.gray.400",
      string: "palette.green.500",
      number: "palette.cyan.500",
      operator: "palette.gray.950",
      "variable.readonly": "palette.cyan.500",
      "function.deprecated": "palette.gray.600",
    },
  },
};

/** High-contrast variants: same structure, boosted separation. */
const HC_DARK: ThemeDocument = {
  ...structuredClone(HELIX_DARK),
  name: "High Contrast Dark",
  type: "hc-dark",
  palette: {
    ...HELIX_DARK.palette,
    "gray.950": "#000000",
    "gray.900": "#0d0d0f",
    "gray.850": "#1a1a1e",
    "gray.800": "#131316",
    "gray.700": "#5a5d66",
    "gray.600": "#787c86",
    "gray.400": "#c8ccd4",
    "gray.200": "#ffffff",
    "blue.500": "#6ea8ff",
    "blue.100": "#a8c8ff",
    "red.500": "#ff7b74",
    "green.500": "#7ee08a",
    "yellow.500": "#ffd75e",
    "yellow.100": "#ffe9a8",
    "purple.500": "#cf9fff",
    "cyan.500": "#6fe3f5",
  },
  semantic: {
    ...HELIX_DARK.semantic,
    "card.border": "palette.gray.400",
    "focus.border": "palette.yellow.500",
    "icon.disabled": "palette.gray.600",
  },
};

const HC_LIGHT: ThemeDocument = {
  ...structuredClone(HELIX_LIGHT),
  name: "High Contrast Light",
  type: "hc-light",
  palette: {
    ...HELIX_LIGHT.palette,
    "gray.950": "#000000",
    "gray.900": "#1a1c1e",
    "gray.850": "#3d3f43",
    "gray.800": "#ffffff",
    "gray.700": "#e9eaee",
    "gray.600": "#b8bac2",
    "gray.400": "#4d5058",
    "gray.200": "#000000",
    "blue.500": "#0f4fc0",
    "blue.100": "#0d3d94",
    "red.500": "#b3261e",
    "green.500": "#1e6b28",
    "yellow.500": "#7a5c00",
    "yellow.100": "#4d3a00",
    "purple.500": "#6a1fa8",
    "cyan.500": "#005f73",
  },
  semantic: {
    ...HELIX_LIGHT.semantic,
    "card.border": "palette.gray.400",
    "focus.border": "palette.yellow.500",
    "icon.disabled": "palette.gray.600",
  },
};

/** The built-in theme registry, keyed by the names settings refer to. */
export const BUILTIN_THEMES: ReadonlyMap<string, ThemeDocument> = new Map([
  [HELIX_DARK.name, HELIX_DARK],
  [HELIX_LIGHT.name, HELIX_LIGHT],
  [HC_DARK.name, HC_DARK],
  [HC_LIGHT.name, HC_LIGHT],
]);

/** Every semantic role the built-in themes define; the resolution target set. */
export const SEMANTIC_ROLES: readonly string[] = Object.keys(HELIX_DARK.semantic ?? {});
