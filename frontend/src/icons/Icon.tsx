import { useMemo } from "react";
import type { IconId, IconSize } from "../generated/icons.gen";
import { resolveIconId, RTL_MIRRORED_ICON_IDS } from "../generated/icons.gen";

/**
 * Icon component (Task 2.6, REQ-ICON-001).
 *
 * Renders an SVG <use> referencing the build-time sprite.
 * - `id`: compile-time union of known IDs (unknown IDs fall back to placeholder)
 * - `size`: sm (12px) | md (16px) | lg (20px), scales with UI zoom
 * - `label`: present → aria-label + tooltip; absent → aria-hidden="true"
 * - `spin`: adds rotation animation (suppressed under prefers-reduced-motion)
 * - Color comes exclusively from theme tokens via CSS classes
 */
export interface IconProps {
  id: IconId;
  size?: IconSize;
  label?: string;
  spin?: boolean;
  className?: string;
}

const SIZE_CLASS = {
  sm: "icon-sm",
  md: "icon-md",
  lg: "icon-lg",
} as const satisfies Record<IconSize, string>;

export function Icon({ id, size = "md", label, spin = false, className = "" }: IconProps) {
  const resolvedId = useMemo(() => resolveIconId(id), [id]);

  const hasLabel = label !== undefined && label !== "";
  const ariaLabel = hasLabel ? label : undefined;
  const ariaHidden = !hasLabel;
  const rtlMirror = (RTL_MIRRORED_ICON_IDS as readonly IconId[]).includes(resolvedId);

  return (
    <svg
      data-testid="icon"
      className={`icon ${SIZE_CLASS[size]} ${spin ? "icon-spin" : ""} ${rtlMirror ? "icon-rtl-flip" : ""} ${className}`}
      aria-label={ariaLabel}
      aria-hidden={ariaHidden}
      role={hasLabel ? "img" : undefined}
      focusable="false"
    >
      <use href={`/sprite.svg#${resolvedId}`} />
      {hasLabel && <title>{label}</title>}
    </svg>
  );
}
