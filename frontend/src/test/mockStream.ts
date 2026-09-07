import type { StreamControl } from "../generated/StreamControl";
import type { StreamEndpoint } from "../generated/StreamEndpoint";
import type { StreamFrame } from "../generated/StreamFrame";
import { StreamClient, type StreamSocket } from "../stream/client";

export class MockSocket implements StreamSocket {
  onopen: ((event?: unknown) => void) | null = null;
  onclose: ((event?: unknown) => void) | null = null;
  onerror: ((event?: unknown) => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  readonly sent: string[] = [];
  readonly url: string;
  closed = false;

  constructor(url: string) {
    this.url = url;
  }

  send(data: string): void {
    if (this.closed) throw new Error("socket is closed");
    this.sent.push(data);
  }

  close(): void {
    this.closed = true;
  }
  open(): void {
    this.onopen?.();
  }
  emit(frame: StreamFrame): void {
    this.emitRaw(JSON.stringify(frame));
  }
  emitData(channel: string, sequence: number, payload: unknown): void {
    this.emit({ kind: "data", channel, sequence, payload, correlation_id: null });
  }
  emitControl(control: StreamControl): void {
    this.emit({ kind: "control", ...control });
  }
  emitRaw(data: unknown): void {
    this.onmessage?.({ data });
  }
  die(): void {
    this.closed = true;
    this.onclose?.();
  }
  controls(): StreamControl[] {
    return this.sent.map((raw) => {
      const { kind, ...control } = JSON.parse(raw) as StreamFrame;
      if (kind !== "control") throw new Error("Expected a stream control frame");
      return control as StreamControl;
    });
  }
}

export function createMockStream(
  options: { resolveEndpoint?: () => Promise<StreamEndpoint> } = {},
) {
  const sockets: MockSocket[] = [];
  const client = new StreamClient({
    resolveEndpoint:
      options.resolveEndpoint ??
      (async () => ({
        url: "ws://127.0.0.1:12345/stream?token=test-token",
        port: 12345,
        token: "test-token",
        heartbeat_interval_ms: 5_000,
        missed_heartbeat_limit: 3,
        default_buffer_depth: 1_000,
      })),
    socketFactory: (url) => {
      const socket = new MockSocket(url);
      sockets.push(socket);
      return socket;
    },
  });
  return {
    client,
    sockets,
    latest() {
      const socket = sockets.at(-1);
      if (!socket) throw new Error("Connect the stream and settle endpoint resolution first");
      return socket;
    },
    dispose() {
      client.close();
    },
  };
}
