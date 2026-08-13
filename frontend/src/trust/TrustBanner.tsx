import { useState } from "react";
import type { TrustStatusResponse } from "../generated/TrustStatusResponse";

export function TrustBanner({
  status,
  onTrust,
}: {
  status: TrustStatusResponse;
  onTrust: (path: string) => void;
}) {
  const [dismissed, setDismissed] = useState(false);
  if (
    dismissed ||
    !status.enabled ||
    status.trust_everything ||
    status.workspace_mode !== "restricted" ||
    status.pending_prompts.length > 0
  ) {
    return null;
  }

  const path =
    status.roots.find((root) => root.decision === "restricted")?.path ?? status.roots[0]?.path;
  if (!path) return null;

  return (
    <aside
      role="status"
      aria-live="polite"
      style={{
        width: "min(60rem, 90vw)",
        padding: "0.75rem 1rem",
        background: "#3b2f1a",
        border: "1px solid #b45309",
        borderRadius: "0.25rem",
        display: "flex",
        gap: "0.75rem",
        alignItems: "center",
        justifyContent: "space-between",
        flexWrap: "wrap",
      }}
    >
      <span>
        Restricted mode: tasks and language servers are blocked for this workspace.
        {!status.store_healthy && " The trust store is unreadable; every folder stays restricted."}
      </span>
      <span style={{ display: "flex", gap: "0.5rem" }}>
        <button type="button" onClick={() => onTrust(path)}>
          Trust folder
        </button>
        <button type="button" onClick={() => setDismissed(true)}>
          Dismiss
        </button>
      </span>
    </aside>
  );
}
