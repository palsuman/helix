import { useCallback, useEffect, useState } from "react";
import type { TrustedFolderEntry } from "../generated/TrustedFolderEntry";
import type { TrustClient } from "./client";

export function TrustManager({ client }: { client: TrustClient }) {
  const [entries, setEntries] = useState<TrustedFolderEntry[]>([]);
  const [trustEverything, setTrustEverything] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [warningOpen, setWarningOpen] = useState(false);
  const [warningAcknowledged, setWarningAcknowledged] = useState(false);

  const refresh = useCallback(() => {
    void Promise.all([client.list(), client.status([])]).then(
      ([list, status]) => {
        setEntries(list.entries);
        setTrustEverything(status.trust_everything);
        setError(null);
      },
      (reason: unknown) => setError(String(reason)),
    );
  }, [client]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return (
    <section aria-labelledby="trust-manager-title">
      <h2 id="trust-manager-title" style={{ fontSize: "1rem" }}>
        Trusted folders
      </h2>
      {error && <p role="alert">{error}</p>}
      {entries.length === 0 ? (
        <p>No trusted folders yet.</p>
      ) : (
        <ul style={{ paddingLeft: "1.25rem" }}>
          {entries.map((entry) => (
            <li key={entry.path} style={{ marginBottom: "0.35rem" }}>
              <code>{entry.path}</code>
              {entry.inherit_to_children ? " (includes subfolders)" : ""}
              {" — "}
              <button
                type="button"
                onClick={() => {
                  void client.revoke(entry.path).then(refresh, (reason: unknown) => {
                    setError(String(reason));
                  });
                }}
              >
                Remove trust
              </button>
            </li>
          ))}
        </ul>
      )}
      {trustEverything ? (
        <p>
          All folders are trusted.{" "}
          <button
            type="button"
            onClick={() => {
              void client.setTrustEverything(false, false).then(refresh, (reason: unknown) => {
                setError(String(reason));
              });
            }}
          >
            Require trust decisions
          </button>
        </p>
      ) : warningOpen ? (
        <div role="alert" style={{ border: "1px solid #b45309", padding: "0.75rem" }}>
          <p>
            Trusting every folder allows code from any repository you open to launch processes on
            this machine. Only continue if you understand this risk.
          </p>
          <label>
            <input
              type="checkbox"
              checked={warningAcknowledged}
              onChange={(event) => setWarningAcknowledged(event.currentTarget.checked)}
            />{" "}
            I understand the security risk
          </label>{" "}
          <button
            type="button"
            disabled={!warningAcknowledged}
            onClick={() => {
              void client.setTrustEverything(true, true).then(
                () => {
                  setWarningOpen(false);
                  setWarningAcknowledged(false);
                  refresh();
                },
                (reason: unknown) => setError(String(reason)),
              );
            }}
          >
            Trust every folder
          </button>{" "}
          <button type="button" onClick={() => setWarningOpen(false)}>
            Cancel
          </button>
        </div>
      ) : (
        <button type="button" onClick={() => setWarningOpen(true)}>
          Trust all folders…
        </button>
      )}
    </section>
  );
}
