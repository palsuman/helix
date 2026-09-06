import { useEffect, useState } from "react";
import { useMessage } from "../localization";
import { SupervisorClient, type RecoveryAction, type SupervisorStatus } from "./client";

export function RecoveryOverlay({ client }: { client: SupervisorClient }) {
  const t = useMessage();
  const [status, setStatus] = useState<SupervisorStatus | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const refresh = () => {
      void client.status().then(
        (next) => {
          if (active) setStatus(next);
        },
        () => {
          // During initial host bootstrap the command may not exist yet. The
          // stream watchdog remains the immediate recovery indicator.
        },
      );
    };
    refresh();
    const timer = setInterval(refresh, 500);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [client]);

  if (status?.state !== "recovery_required") return null;

  const act = (action: RecoveryAction) => {
    setActionError(null);
    void client.action(action).catch((error: unknown) => setActionError(String(error)));
  };

  return (
    <section
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="kernel-recovery-title"
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 1000,
        display: "grid",
        placeItems: "center",
        background: "rgba(0, 0, 0, 0.72)",
      }}
    >
      <div style={{ maxWidth: "36rem", padding: "2rem", background: "#202033" }}>
        <h2 id="kernel-recovery-title">{t("recoveryTitle")}</h2>
        <p>{t("recoveryDescription")}</p>
        {status.safe_mode && <p>{t("recoverySafeMode")}</p>}
        {status.cause.panic_message && (
          <p>{t("recoveryLastError", { error: status.cause.panic_message })}</p>
        )}
        <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap" }}>
          <button type="button" onClick={() => act("retry")}>
            {t("recoveryRetry")}
          </button>
          <button type="button" onClick={() => act("start_without_session_restore")}>
            {t("recoveryWithoutRestore")}
          </button>
          <button type="button" onClick={() => act("open_logs")}>
            {t("recoveryOpenLogs")}
          </button>
        </div>
        {actionError && <p role="alert">{t("recoveryActionFailed", { error: actionError })}</p>}
      </div>
    </section>
  );
}
