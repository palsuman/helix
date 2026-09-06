import type { TrustStatusResponse } from "../generated/TrustStatusResponse";
import { useMessage } from "../localization";

export function TrustPrompt({
  status,
  onTrust,
  onRestrict,
}: {
  status: TrustStatusResponse;
  onTrust: (path: string) => void;
  onRestrict: (path: string) => void;
}) {
  const t = useMessage();
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
        <h2 id="trust-prompt-title">{t("trustPromptTitle")}</h2>
        <p>{t("trustPromptDescription", { path })}</p>
        <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap" }}>
          <button type="button" onClick={() => onTrust(path)}>
            {t("trustFolder")}
          </button>
          <button type="button" onClick={() => onRestrict(path)}>
            {t("trustStayRestricted")}
          </button>
        </div>
      </div>
    </section>
  );
}
