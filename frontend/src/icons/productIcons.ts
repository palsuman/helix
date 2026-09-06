/**
 * Product icon theme support (Task 2.6, REQ-ICON-001).
 *
 * Product icons are the UI chrome icons (toolbars, activity bar, gutters, states).
 * They are independent of color and file icon themes.
 */

import type { IconId } from "../generated/icons.gen";

export interface ProductIconTheme {
  id: string;
  name: string;
  /** Icon ID overrides for specific product icon slots. */
  icons: Partial<Record<string, IconId>>;
}

/** The built-in default product icon theme. */
const DEFAULT_THEME: ProductIconTheme = {
  id: "helix-default",
  name: "Helix Default",
  icons: {},
};

/** Built-in product icon themes. */
export const BUILTIN_PRODUCT_ICON_THEMES: ReadonlyMap<string, ProductIconTheme> = new Map([
  [DEFAULT_THEME.id, DEFAULT_THEME],
]);

export const DEFAULT_PRODUCT_ICON_THEME = DEFAULT_THEME.id;