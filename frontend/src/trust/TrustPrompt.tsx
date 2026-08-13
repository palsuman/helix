import type { TrustStatusResponse } from "../generated/TrustStatusResponse";

export function TrustPrompt({
  status,
  onTrust,
  onRestrict,
}: {
  status: TrustStatusResponse;
  onTrust: (path: string) => void;
  onRestrict: (path: string) => void;
}) {
  const path = status.pending_prompts[0];
  if (!path || !status.enabled || status.trust_everything) return null;

  return (
    <section
      role="dialog"
      aria-modal="true"
      aria-labelledby="trust-prompt-title"
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 900,
        display: "grid",
        placeItems: "center",
        background: "rgba(0, 0, 0, 0.65)",
      }}
    >
      <div style={{ maxWidth: "34rem", padding: "1.5rem", background: "#25253a" }}>
        <h2 id="trust-prompt-title">Trust this folder?</h2>
        <p>
          Helix opened <code>{path}</code>. In Restricted mode, language servers, tasks, and other
          workspace-supplied code will not run.
        </p>
        <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap" }}>
          <button type="button" onClick={() => onTrust(path)}>
            Trust folder
          </button>
          <button type="button" onClick={() => onRestrict(path)}>
            Stay in Restricted mode
          </button>
        </div>
      </div>
    </section>
  );
}
