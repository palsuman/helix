/**
 * Token resolution (Task 2.5, REQ-THEME-001 failure modes).
 *
 * A semantic value is either a direct color or a reference:
 *
 *   "#3574f0"        used as-is
 *   "palette.blue.500"
 *   "semantic.accent"  (a role referencing another role)
 *
 * Resolution walks the chain with a visited set; a cycle or a missing target
 * falls back to the fallback theme's value for the same role, and only then
 * to the raw text, so no CSS variable is ever left undefined.
 */

import {
  BUILTIN_THEMES,
  FALLBACK_THEME,
  SEMANTIC_ROLES,
  type ThemeDocument,
} from "./themes";
const MAX_CHAIN_DEPTH = 16;

export interface ResolveResult {
  /** Role → concrete color, for every known role. */
  tokens: Map<string, string>;
  /** Roles that could not be resolved from the theme itself. */
  fellBack: string[];
}

function isReference(value: string): boolean {
  return value.startsWith("palette.") || value.startsWith("semantic.");
}

/**
 * Resolve one value to a color, following references up to the depth cap.
 * Returns `null` when the chain dead-ends or cycles.
 */
export function resolveValue(
  value: string,
  theme: ThemeDocument,
  seen: Set<string> = new Set(),
): string | null {
  let current = value;
  for (let depth = 0; depth < MAX_CHAIN_DEPTH; depth += 1) {
    if (!isReference(current)) return current;
    if (seen.has(current)) return null; // cycle
    seen.add(current);
    if (current.startsWith("palette.")) {
      const color = theme.palette?.[current.slice("palette.".length)];
      current = color ?? "";
    } else {
      const next = theme.semantic?.[current.slice("semantic.".length)];
      current = next ?? "";
    }
    if (current === "") return null;
  }
  return null;
}

/**
 * Resolve every known semantic role for `theme`, falling back per-role to the
 * built-in fallback theme (REQ-THEME-001: "missing token: fall back … to the
 * default theme's token").
 */
export function resolveTheme(theme: ThemeDocument): ResolveResult {
  const fallbackDoc = BUILTIN_THEMES.get(FALLBACK_THEME);
  const fallback: ThemeDocument = fallbackDoc ?? {
    name: FALLBACK_THEME,
    type: "dark",
    palette: {},
    semantic: {},
  };
  const tokens = new Map<string, string>();
  const fellBack: string[] = [];

  for (const role of SEMANTIC_ROLES) {
    const own = theme.semantic?.[role];
    let color = own === undefined ? null : resolveValue(own, theme);
    if (color === null && theme !== fallback) {
      // Missing or broken chain: nearest parent first (the role's own chain
      // already failed), then the default theme's resolved value.
      const fallbackValue = fallback.semantic?.[role];
      color = fallbackValue === undefined ? null : resolveValue(fallbackValue, fallback);
      if (color !== null) fellBack.push(role);
    }
    if (color === null) {
      // Last resort so CSS never sees an empty custom property.
      color = "#ff00ff";
      fellBack.push(role);
    }
    tokens.set(role, color);
  }

  // User overrides win after everything else resolves (REQ-THEME-001.10).
  for (const [role, value] of Object.entries(theme.overrides ?? {})) {
    const color = resolveValue(value, theme);
    if (color !== null) tokens.set(role, color);
  }

  return { tokens, fellBack };
}

/** Semantic role → workbench CSS custom property name. */
export function cssVariableName(role: string): string {
  return `--helix-${role.replaceAll(".", "-")}`;
}

/** The resolved theme as a `var`-ready record for `CSSStyleDeclaration`. */
export function toCssVariables(tokens: Map<string, string>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [role, color] of tokens) {
    out[cssVariableName(role)] = color;
  }
  return out;
}
