import { describe, expect, it } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import App from "./App";
import { IpcClient, type InvokeFn } from "./ipc";
import { StreamClient, type StreamSocket } from "./stream";
import type { IpcRequest } from "./generated/IpcRequest";
import type { StreamEnvelope } from "./generated/StreamEnvelope";
import type { StreamFrame } from "./generated/StreamFrame";

/**
 * Drives the Task 1.3 demo surface against a fake kernel: a typed response
 * is rendered, and cancelling the simulated 10s command settles it as
 * cancelled rather than leaving it pending.
 */
function fakeKernelClient() {
  const cancelledIds: string[] = [];
  const pendingCancels = new Map<string, () => void>();

  const invoke: InvokeFn = async <T,>(command: string, args?: Record<string, unknown>) => {
    if (command === "ipc_cancel") {
      const { correlation_id } = (args as { request: { correlation_id: string } }).request;
      cancelledIds.push(correlation_id);
      pendingCancels.get(correlation_id)?.();
      return { correlation_id, cancelled: true } as T;
    }

    const request = (args as { request: IpcRequest<unknown> }).request;
    if (request.command === "ipc.ping") {
      return {
        correlation_id: request.correlation_id,
        result: { echo: "Helix", kernel_version: "0.1.0" },
        error: null,
      } as T;
    }

    // The log viewer is part of the demo surface (Task 1.5); the fake kernel
    // answers its commands so the panel reaches a settled state instead of
    // leaving a pending query behind every assertion.
    if (request.command === "log.query") {
      return {
        correlation_id: request.correlation_id,
        result: {
          entries: [],
          matched: 0,
          ring_len: 0,
          ring_capacity: 10_000,
          evicted: 0,
          sources: [],
        },
        error: null,
      } as T;
    }
    if (request.command === "log.append") {
      return {
        correlation_id: request.correlation_id,
        result: { recorded: true, source: "frontend.app" },
        error: null,
      } as T;
    }
    if (request.command === "trust.status") {
      const payload = request.payload as { paths?: string[] };
      const paths = payload.paths ?? [];
      return {
        correlation_id: request.correlation_id,
        result: {
          enabled: true,
          trust_everything: false,
          store_healthy: true,
          workspace_mode: "trusted",
          pending_prompts: [],
          roots: paths.map((path) => ({
            path,
            decision: "trusted",
            inherited_from: null,
          })),
        },
        error: null,
      } as T;
    }
    if (request.command === "trust.list") {
      return {
        correlation_id: request.correlation_id,
        result: { entries: [] },
        error: null,
      } as T;
    }
    if (request.command === "workspace.list") {
      return {
        correlation_id: request.correlation_id,
        result: { workspaces: [] },
        error: null,
      } as T;
    }

    // ipc.sleep: answer only when cancelled, as the kernel does.
    return new Promise<T>((resolve) => {
      pendingCancels.set(request.correlation_id, () =>
        resolve({
          correlation_id: request.correlation_id,
          result: null,
          error: {
            code: "CANCELLED",
            category: "cancelled",
            message: "command 'ipc.sleep' was cancelled by the client",
            details: null,
          },
        } as T),
      );
    });
  };

  return { client: new IpcClient({ invoke }), cancelledIds };
}

/**
 * A stand-in for the kernel's stream socket, so the demo counter and the
 * reconnection indicator can be driven without a network.
 */
function fakeStream() {
  const sockets: StreamSocket[] = [];
  const client = new StreamClient({
    resolveEndpoint: async () => ({
      url: "ws://127.0.0.1:1/stream?token=t",
      port: 1,
      token: "t",
      heartbeat_interval_ms: 5_000,
      missed_heartbeat_limit: 3,
      default_buffer_depth: 1_000,
    }),
    socketFactory: () => {
      const socket: StreamSocket = {
        onopen: null,
        onclose: null,
        onerror: null,
        onmessage: null,
        send: () => {},
        close: () => {},
      };
      sockets.push(socket);
      return socket;
    },
  });

  // Which socket the test has opened so far. A reconnect creates a new one
  // after its backoff delay, and opening the dead one instead would be
  // ignored by the client (correctly), so the test has to wait for it.
  let opened = 0;
  const latest = () => sockets[opened - 1];

  return {
    client,
    latest,
    /** Wait for the next socket the client creates, then open it. */
    open: async () => {
      await waitFor(() => {
        expect(sockets.length).toBeGreaterThan(opened);
      });
      const socket = sockets[opened];
      opened += 1;
      await act(async () => {
        socket.onopen?.();
      });
    },
    emitTick: async (sequence: number, value: number) => {
      const envelope: StreamEnvelope = {
        channel: "demo:counter",
        correlation_id: null,
        sequence,
        payload: { value, emitted_at_ms: 0 },
      };
      const frame: StreamFrame = { kind: "data", ...envelope };
      await act(async () => {
        latest().onmessage?.({ data: JSON.stringify(frame) });
      });
    },
    die: async () => {
      await act(async () => {
        latest().onclose?.();
      });
    },
  };
}

describe("App", () => {
  it("renders the Helix title", () => {
    const { client } = fakeKernelClient();
    render(<App client={client} streamClient={fakeStream().client} />);
    expect(screen.getByText("Helix")).toBeInTheDocument();
  });

  it("renders the typed response of an IPC command", async () => {
    const { client } = fakeKernelClient();
    render(<App client={client} streamClient={fakeStream().client} />);

    await waitFor(() => {
      expect(screen.getByText(/Kernel replied/)).toHaveTextContent("Helix");
    });
    expect(screen.getByText(/Kernel replied/)).toHaveTextContent("0.1.0");
  });

  it("cancels a long-running command and reports it as cancelled", async () => {
    const { client, cancelledIds } = fakeKernelClient();
    render(<App client={client} streamClient={fakeStream().client} />);

    fireEvent.click(screen.getByRole("button", { name: "Start 10s command" }));
    expect(await screen.findByText(/Running a 10s kernel command/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(await screen.findByText(/Cancelled after/)).toBeInTheDocument();
    expect(cancelledIds).toHaveLength(1);
  });

  it("surfaces an error with its category guidance and offers a retry when retryable", async () => {
    const invoke: InvokeFn = async () => {
      throw new Error("kernel unreachable");
    };
    render(<App client={new IpcClient({ invoke })} streamClient={fakeStream().client} />);

    // Scoped to the round-trip section: with every command failing, the log
    // viewer reports its own unreachable query as a second alert, which is
    // correct behaviour and not what this test is about.
    const alert = await screen.findByText(/command 'ipc\.ping' could not reach the kernel/);
    expect(alert).toHaveTextContent("IPC_TRANSPORT_FAILED");
    expect(alert).toHaveTextContent("Retrying should work");
    expect(screen.getAllByRole("button", { name: "Retry" }).length).toBeGreaterThan(0);
  });

  it("renders the live counter stream and shows reconnecting when the socket dies", async () => {
    // The Task 1.4 demo criterion, end to end through the UI: a live stream,
    // then a killed socket showing "reconnecting", then the stream resuming
    // where it left off.
    const { client } = fakeKernelClient();
    const streaming = fakeStream();
    render(<App client={client} streamClient={streaming.client} />);

    expect(screen.getByText(/Waiting for the counter/)).toBeInTheDocument();

    await streaming.open();
    await streaming.emitTick(1, 100);
    expect(screen.getByText("Counter 100")).toBeInTheDocument();

    await streaming.die();
    expect(screen.getByText("Reconnecting…")).toBeInTheDocument();

    await streaming.open();
    await streaming.emitTick(2, 101);
    expect(screen.getByText("Counter 101")).toBeInTheDocument();
    expect(screen.getByText("Live")).toBeInTheDocument();
  });

  it("reports truncation when the kernel drops messages", async () => {
    const { client } = fakeKernelClient();
    const streaming = fakeStream();
    render(<App client={client} streamClient={streaming.client} />);

    await streaming.open();
    await streaming.emitTick(1, 1);
    // A jump the kernel did not announce is still a loss the user must see.
    await streaming.emitTick(9, 9);

    const status = await screen.findByText(/Output truncated/);
    expect(status).toHaveTextContent("Output truncated: 7 message(s)");
    expect(status).toHaveTextContent("demo:counter");
  });
});
