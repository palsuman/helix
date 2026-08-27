/**
 * Theme service (Task 2.5, REQ-THEME-001).
 *
 * Owns the active theme end to end:
 *
 *   config.get(workbench.colorTheme) ──► resolve ──► apply CSS variables
 *   matchMedia(prefers-color-scheme) ─┘               (+ Monaco theme)
 *   config:changed stream ────────────┘
 *
 * Switching is a CSS custom property swap on one element, so the browser
 * repaints without reflow: no layout shift, well under 100ms (REQ-THEME-001.4).
 * The service is transport-agnostic — tests inject a fake config client and
 * a fake media query list.
 */

import { getConfigValue, setConfigValue } from "../config";import {
  BUILTIN_THEMES,
  DEFAULT_DARK_THEME,
  type ThemeDocument,
} from "./themes";
import { resolveTheme, toCssVariables } from "./resolve";

export const COLOR_THEME_SETTING = "workbench.colorTheme";

/** Themes that respond to `prefers-color-scheme` as the dark half. */
function isDarkTheme(theme: ThemeDocument): boolean {
  return theme.type === "dark" || theme.type === "hc-dark";
}

/** The minimal config surface this service needs; swappable in tests. */
export interface ThemeConfigClient {
  getColorTheme(): Promise<string | null>;
  setColorTheme(name: string): Promise<void>;
}

/** The minimal media-query surface; matches the parts of `MediaQueryList` we use. */
export interface MediaQueryLike {
  matches: boolean;
  addEventListener?(listener: (this: MediaQueryLike, ev?: unknown) => void): void;
  removeEventListener?(listener: (this: MediaQueryLike, ev?: unknown) => void): void;
  addListener?(listener: () => void): void;
  removeListener?(listener: () => void): void;
}

export interface ThemeServiceOptions {
  config?: ThemeConfigClient;
  /** `window.matchMedia`, injectable for tests. */
  matchMedia?: (query: string) => MediaQueryLike;
  /** Element receiving the CSS variables; defaults to `document.documentElement`. */
  target?: HTMLElement;
  /** Called after each apply, for Monaco theme sync. */
  onApply?: (themeName: string, theme: ResolvedTheme) => void;
}

/** A theme plus its resolved token table. */
export interface ResolvedTheme {
  name: string;
  document: ThemeDocument;
  tokens: Map<string, string>;
}

export class ThemeService {
  private readonly config: ThemeConfigClient;
  private readonly matchMedia: (query: string) => MediaQueryLike;
  private readonly target: HTMLElement | null;
  private readonly onApply: ((name: string, theme: ResolvedTheme) => void) | undefined;
  private active: ResolvedTheme;
  private osDarkListener: (() => void) | null = null;
  private osDarkQuery: MediaQueryLike | null = null;

  constructor(options: ThemeServiceOptions = {}) {
    this.config =
      options.config ??
      ({
        getColorTheme: () => getConfigValue<string>(COLOR_THEME_SETTING),
        setColorTheme: (name: string) =>
          setConfigValue("user", COLOR_THEME_SETTING, name).then(() => undefined),
      } satisfies ThemeConfigClient);
    this.matchMedia =
      options.matchMedia ??
      ((query: string) => window.matchMedia(query) as unknown as MediaQueryLike);
    this.target =
      options.target ??
      (typeof document !== "undefined" ? document.documentElement : null);
    this.onApply = options.onApply;
    this.active = this.resolve(DEFAULT_DARK_THEME);
  }

  /** The currently applied theme. */
  get current(): ResolvedTheme {
    return this.active;
  }

  /** True when the active theme is a dark variant. */
  get isDark(): boolean {
    return isDarkTheme(this.active.document);
  }

  /**
   * Read the configured theme from settings, falling back to the OS
   * preference when the setting still holds its default and the user has
   * not chosen (REQ-THEME-001.5). Applies the result.
   */
  async initialize(): Promise<ResolvedTheme> {
    const configured = await this.config.getColorTheme().catch(() => null);
    const name = configured ?? (await this.osPreferred());
    return this.apply(name);
  }

  /**
   * The OS-preferred built-in theme: dark or light, high contrast when the
   * OS requests more contrast (REQ-THEME-001 failure modes).
   */
  async osPreferred(): Promise<string> {
    const hc = this.query("(prefers-contrast: more)");
    const dark = this.query("(prefers-color-scheme: dark)");
    const highContrast = hc?.matches ?? false;
    const shade = dark?.matches === false ? "Light" : "Dark";
    // Built-ins are named "Helix Light"/"Helix Dark" and
    // "High Contrast Light"/"High Contrast Dark".
    const name = highContrast ? `High Contrast ${shade}` : `Helix ${shade}`;
    return BUILTIN_THEMES.has(name) ? name : DEFAULT_DARK_THEME;
  }

  /**
   * Follow OS appearance changes until the user picks a theme explicitly.
   * Returns a disposer.
   */
  followOsPreference(): () => void {
    this.stopFollowingOsPreference();
    const query = this.query("(prefers-color-scheme: dark)");
    if (!query) return () => {};
    this.osDarkQuery = query;
    this.osDarkListener = () => {
      void this.osPreferred().then((name) => this.apply(name));
    };
    query.addEventListener?.(this.osDarkListener);
    query.addListener?.(this.osDarkListener); // legacy Safari
    return () => this.stopFollowingOsPreference();
  }

  private stopFollowingOsPreference(): void {
    if (!this.osDarkQuery || !this.osDarkListener) return;
    this.osDarkQuery.removeEventListener?.(this.osDarkListener);
    this.osDarkQuery.removeListener?.(this.osDarkListener);
    this.osDarkQuery = null;
    this.osDarkListener = null;
  }

  /**
   * Switch themes by name (REQ-THEME-001.4). Unknown names fall back to the
   * default dark theme rather than leaving the UI unthemed.
   */
  async apply(name: string): Promise<ResolvedTheme> {
    const resolved = this.resolve(name);
    this.paint(resolved);
    this.active = resolved;
    this.onApply?.(resolved.name, resolved);
    return resolved;
  }

  /**
   * Preview a theme without committing the choice (REQ-THEME-001.11).
   * {@link commit} restores the real selection.
   */
  preview(name: string): void {
    this.paint(this.resolve(name));
  }

  /** End a preview, repainting the committed theme. */
  commit(): void {
    this.paint(this.active);
  }

  /**
   * Persist a theme choice to settings. The `config:changed` stream echoes
   * it back to every window, where {@link onConfigChanged} applies it.
   */
  async select(name: string): Promise<void> {
    await this.apply(name);
    await this.config.setColorTheme(name);
  }

  /**
   * React to a `workbench.colorTheme` change made anywhere (another window,
   * a settings file edit). A no-op when this window already shows it.
   */
  async onConfigChanged(value: unknown): Promise<void> {
    if (typeof value !== "string" || value === this.active.name) return;
    await this.apply(value);
  }

  private resolve(name: string): ResolvedTheme {
    const document = BUILTIN_THEMES.get(name) ?? BUILTIN_THEMES.get(DEFAULT_DARK_THEME);
    const doc = document ?? {
      name: DEFAULT_DARK_THEME,
      type: "dark",
      palette: {},
      semantic: {},
    };
    const { tokens } = resolveTheme(doc);
    return { name: doc.name, document: doc, tokens };
  }

  private paint(theme: ResolvedTheme): void {
    if (!this.target) return;
    const style = this.target.style;
    for (const [property, value] of Object.entries(toCssVariables(theme.tokens))) {
      style.setProperty(property, value);
    }
    // Color scheme drives native control rendering and default scrollbar colors.
    style.colorScheme = isDarkTheme(theme.document) ? "dark" : "light";
  }

  private query(text: string): MediaQueryLike | null {
    try {
      return this.matchMedia(text);
    } catch {
      return null;
    }
  }
}
