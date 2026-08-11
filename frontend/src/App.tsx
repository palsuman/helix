import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { IpcClient, IpcError, ipc, isIpcError, ping, sleep } from "./ipc";
import { FrontendLogger, LogViewer } from "./logging";
import {
  STREAM_CHANNELS,
  StreamClient,
  StreamStatusIndicator,
  stream,
  useStreamBackpressure,
  useStreamChannel,
  useStreamStatus,
} from "./stream";
import type { PingResponse } from "./generated/PingResponse";

// Task 1.3, 1.4, and 1.5 demo surface: the frontend invokes a typed command
// and renders the typed response, cancelling a simulated 10s command aborts it
// within 100ms, the 100Hz counter stream renders live with a visible
// reconnection indicator, and the log viewer shows kernel and frontend records
// in one stream. The workbench shell replaces this in Task 2.1; the clients and
// panels it exercises (src/ipc/, src/stream/, src/logging/) are permanent.

/** Payload shape of the kernel's demo counter channel. */
interface CounterTick {
  value: number;
  emitted_at_ms: number;
}

type PingState =
  | { kind: "loading" }
  | { kind: "ready"; response: PingResponse }
  | { kind: "error"; error: IpcError };

type LongCommandState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "cancelled"; elapsedMs: number }
  | { kind: "completed"; sleptMs: number }
  | { kind: "error"; error: IpcError };

/** How the frontend reacts to each error category (REQ-ARCH-003.3). */
function guidanceFor(error: IpcError): string {
  switch (error.category) {
    case "transient":
      return "Temporary problem. Retrying should work.";
    case "permanent":
      return "This will not succeed until something changes.";
    case "cancelled":
      return "Cancelled.";
    case "timeout":
      return "The kernel took too long and the command was cancelled. Retry when ready.";
  }
}

function asIpcError(value: unknown): IpcError {
  if (isIpcError(value)) return value;
  throw value;
}

function App({
  client = ipc,
  streamClient = stream,
}: {
  client?: IpcClient;
  streamClient?: StreamClient;
}) {
  const [pingState, setPingState] = useState<PingState>({ kind: "loading" });
  const [longState, setLongState] = useState<LongCommandState>({ kind: "idle" });
  const abortRef = useRef<AbortController | null>(null);

  // Frontend records travel to the kernel, so they land in the same ring
  // buffer, the same file, and the same viewer as kernel records
  // (REQ-OBS-001.3).
  const logger = useMemo(() => new FrontendLogger({ client, source: "app" }), [client]);

  const streamStatus = useStreamStatus(streamClient);
  const tick = useStreamChannel<CounterTick>(streamClient, STREAM_CHANNELS.demoCounter);
  const truncation = useStreamBackpressure(streamClient);

  // The client keeps itself connected, re-resolving the endpoint on every
  // attempt, so a kernel restart on a new port needs nothing from here.
  useEffect(() => {
    streamClient.connect();
    return () => streamClient.close();
  }, [streamClient]);

  // Resolved through promise callbacks rather than an awaited call in the
  // effect body, so state only ever changes in response to the kernel
  // answering — the effect itself performs no synchronous render cascade.
  const runPing = useCallback(
    (isCurrent: () => boolean = () => true) =>
      ping(client, "Helix").then(
        (response) => {
          if (isCurrent()) setPingState({ kind: "ready", response });
        },
        (error: unknown) => {
          if (isCurrent()) setPingState({ kind: "error", error: asIpcError(error) });
        },
      ),
    [client],
  );

  const retryPing = useCallback(() => {
    setPingState({ kind: "loading" });
    void runPing();
  }, [runPing]);

  useEffect(() => {
    let active = true;
    void runPing(() => active);
    return () => {
      active = false;
    };
  }, [runPing]);

  const startLongCommand = useCallback(async () => {
    const controller = new AbortController();
    abortRef.current = controller;
    setLongState({ kind: "running" });
    const startedAt = performance.now();
    logger.info("starting the long command", { duration_ms: 10_000 });

    try {
      const response = await sleep(client, 10_000, { signal: controller.signal });
      setLongState({ kind: "completed", sleptMs: response.slept_ms });
    } catch (error: unknown) {
      const ipcError = asIpcError(error);
      if (ipcError.isCancelled) {
        setLongState({
          kind: "cancelled",
          elapsedMs: Math.round(performance.now() - startedAt),
        });
      } else {
        setLongState({ kind: "error", error: ipcError });
      }
    } finally {
      abortRef.current = null;
    }
  }, [client, logger]);

  const cancelLongCommand = useCallback(() => {
    abortRef.current?.abort();
  }, []);

  return (
    <main
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: "1rem",
        height: "100vh",
        fontFamily: "system-ui, sans-serif",
        color: "#e5e7eb",
        background: "#1a1a2e",
      }}
    >
      <h1>Helix</h1>

      <section aria-labelledby="ipc-round-trip">
        <h2 id="ipc-round-trip" style={{ fontSize: "1rem" }}>
          IPC round trip
        </h2>
        {pingState.kind === "loading" && <p>Calling the kernel…</p>}
        {pingState.kind === "ready" && (
          <p>
            Kernel replied “{pingState.response.echo}” from version{" "}
            {pingState.response.kernel_version}
          </p>
        )}
        {pingState.kind === "error" && (
          <p role="alert">
            {pingState.error.code}: {pingState.error.message} — {guidanceFor(pingState.error)}
          </p>
        )}
        {pingState.kind === "error" && pingState.error.isRetryable && (
          <button type="button" onClick={retryPing}>
            Retry
          </button>
        )}
      </section>

      <section aria-labelledby="ipc-cancellation">
        <h2 id="ipc-cancellation" style={{ fontSize: "1rem" }}>
          Cancellation
        </h2>
        <button type="button" onClick={() => void startLongCommand()}>
          Start 10s command
        </button>
        <button type="button" onClick={cancelLongCommand} disabled={longState.kind !== "running"}>
          Cancel
        </button>
        {longState.kind === "running" && <p>Running a 10s kernel command…</p>}
        {longState.kind === "cancelled" && <p>Cancelled after {longState.elapsedMs}ms</p>}
        {longState.kind === "completed" && <p>Completed after {longState.sleptMs}ms</p>}
        {longState.kind === "error" && (
          <p role="alert">
            {longState.error.code}: {longState.error.message} — {guidanceFor(longState.error)}
          </p>
        )}
      </section>

      <section aria-labelledby="stream-counter">
        <h2 id="stream-counter" style={{ fontSize: "1rem" }}>
          Streaming
        </h2>
        <p>
          <StreamStatusIndicator status={streamStatus} />
        </p>
        <p>{tick === null ? "Waiting for the counter…" : `Counter ${tick.value}`}</p>
        {truncation !== null && (
          <p role="status">
            Output truncated: {truncation.dropped} message(s) dropped on {truncation.channel}
          </p>
        )}
      </section>

      <section style={{ width: "min(60rem, 90vw)" }}>
        <LogViewer client={client} streamClient={streamClient} />
      </section>
    </main>
  );
}

export default App;
