import type { StreamStatus } from "./client";

/**
 * Reconnection indicator (Task 1.4, REQ-ARCH-003 failure modes: "frontend
 * shows a reconnecting indicator").
 *
 * Announced through an ARIA live region because losing the stream changes
 * what the rest of the UI means — a terminal that has stopped updating looks
 * identical to one with nothing to say (REQ-NFR-005.2). State is carried by
 * text as well as colour, so colour is never the only signal
 * (REQ-NFR-005.11).
 *
 * Styling is inline until the theming system lands in Task 2.4, at which
 * point these colours become semantic tokens.
 */

const LABELS: Record<StreamStatus, string> = {
  idle: "Stream idle",
  connecting: "Connecting…",
  open: "Live",
  reconnecting: "Reconnecting…",
  closed: "Stream closed",
};

const COLORS: Record<StreamStatus, string> = {
  idle: "#9ca3af",
  connecting: "#fbbf24",
  open: "#34d399",
  reconnecting: "#fbbf24",
  closed: "#f87171",
};

export function StreamStatusIndicator({ status }: { status: StreamStatus }) {
  return (
    <span
      // "polite" rather than "assertive": a reconnect is worth announcing
      // but must not interrupt whatever the user is doing.
      aria-live="polite"
      data-status={status}
      style={{ display: "inline-flex", alignItems: "center", gap: "0.4rem" }}
    >
      <span
        aria-hidden="true"
        style={{
          width: "0.5rem",
          height: "0.5rem",
          borderRadius: "50%",
          background: COLORS[status],
        }}
      />
      {LABELS[status]}
    </span>
  );
}
