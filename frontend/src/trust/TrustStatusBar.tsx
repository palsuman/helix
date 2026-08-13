import type { TrustStatusResponse } from "../generated/TrustStatusResponse";

export function TrustStatusBar({ status }: { status: TrustStatusResponse | null }) {
  if (!status?.enabled || status.trust_everything) return null;

  const label =
    status.workspace_mode === "restricted"
      ? "Restricted mode"
      : status.pending_prompts.length > 0
        ? "Trust required"
        : "Trusted";

  const color =
    status.workspace_mode === "restricted" || status.pending_prompts.length > 0
      ? "#fbbf24"
      : "#34d399";

  return (
    <span
      role="status"
      aria-label={`Workspace trust: ${label}`}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "0.35rem",
        fontSize: "0.85rem",
        color,
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: "0.55rem",
          height: "0.55rem",
          borderRadius: "999px",
          background: color,
        }}
      />
      {label}
    </span>
  );
}
