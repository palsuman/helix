/** Base styles for the Icon component (Task 2.6). */
const ICON_STYLES = `
.icon {
  display: inline-flex;
  flex-shrink: 0;
  width: var(--icon-size, 16px);
  height: var(--icon-size, 16px);
  color: var(--helix-icon-foreground, currentColor);
  vertical-align: middle;
}
.icon-sm { --icon-size: 12px; }
.icon-md { --icon-size: 16px; }
.icon-lg { --icon-size: 20px; }
.icon-spin {
  animation: icon-spin 1s linear infinite;
}
@media (prefers-reduced-motion: reduce) {
  .icon-spin { animation: none; }
}
@keyframes icon-spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}
/* RTL mirroring for directional icons */
[dir="rtl"] .icon-rtl-flip { transform: scaleX(-1); }
`;

/** Inject base styles once. */
export function injectIconStyles(): void {
  if (typeof document === "undefined") return;
  if (document.getElementById("helix-icon-styles")) return;
  const style = document.createElement("style");
  style.id = "helix-icon-styles";
  style.textContent = ICON_STYLES;
  document.head.appendChild(style);
}