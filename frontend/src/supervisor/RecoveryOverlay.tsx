import { useEffect, useState } from "react";
import { SupervisorClient, type RecoveryAction, type SupervisorStatus } from "./client";

export function RecoveryOverlay({ client }: { client: SupervisorClient }) {
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
        <h2 id="kernel-recovery-title">Kernel recovery required</h2>
        <p>
          Helix stopped restarting the kernel after repeated crashes. Your persisted work remains
          on disk.
        </p>
        {status.safe_mode && <p>Safe mode is enabled: plugins and session restore are disabled.</p>}
        {status.cause.panic_message && <p>Last error: {status.cause.panic_message}</p>}
        <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap" }}>
          <button type="button" onClick={() => act("retry")}>Retry</button>
          <button type="button" onClick={() => act("start_without_session_restore")}>
            Start without session restore
          </button>
          <button type="button" onClick={() => act("open_logs")}>Open logs</button>
        </div>
        {actionError && <p role="alert">Recovery action failed: {actionError}</p>}
      </div>
    </section>
  );
}
