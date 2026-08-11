import { afterEach, describe, expect, it, vi } from "vitest";
import type { StreamControl } from "../generated/StreamControl";
import type { StreamEndpoint } from "../generated/StreamEndpoint";
import type { StreamEnvelope } from "../generated/StreamEnvelope";
import type { StreamFrame } from "../generated/StreamFrame";
import {
  MAX_BACKOFF_MS,
  MIN_BACKOFF_MS,
  StreamClient,
  backoffDelayMs,
  type BackpressureEvent,
  type StreamSocket,
  type StreamStatus,
} from "./client";

const COUNTER = "demo:counter";

function endpoint(overrides: Partial<StreamEndpoint> = {}): StreamEndpoint {
  return {
    url: "ws://127.0.0.1:12345/stream?token=test-token",
    port: 12345,
    token: "test-token",
    heartbeat_interval_ms: 5_000,
    missed_heartbeat_limit: 3,
    default_buffer_depth: 1_000,
    ...overrides,
  };
}

/**
 * A stand-in for the kernel's socket. Records what the client sent and lets
 * the test drive open, message, close, and error, so the whole reconnect and
 * heartbeat state machine is exercised without a network.
 */
class FakeSocket implements StreamSocket {
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

  /** Deliver a frame as the kernel would. */
  emit(frame: StreamFrame): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }

  emitData(channel: string, sequence: number, payload: unknown): void {
    const envelope: StreamEnvelope = {
      channel,
      correlation_id: null,
      sequence,
      payload,
    };
    this.emit({ kind: "data", ...envelope });
  }

  emitControl(control: StreamControl): void {
    this.emit({ kind: "control", ...control } as StreamFrame);
  }

  emitRaw(data: unknown): void {
    this.onmessage?.({ data });
  }

  /** The socket dying without a close handshake, as a killed kernel would. */
  die(): void {
    this.closed = true;
    this.onclose?.();
  }

  /** Control frames the client sent, with the frame discriminator removed. */
  controls(): StreamControl[] {
    return this.sent.map((raw) => {
      const { kind, ...control } = JSON.parse(raw) as { kind: string } & StreamControl;
      expect(kind).toBe("control");
      return control as StreamControl;
    });
  }
}

function harness(options: { endpoint?: StreamEndpoint; failEndpoint?: boolean } = {}) {
  const sockets: FakeSocket[] = [];
  let failEndpoint = options.failEndpoint ?? false;

  const client = new StreamClient({
    resolveEndpoint: async () => {
      if (failEndpoint) throw new Error("kernel unavailable");
      return options.endpoint ?? endpoint();
    },
    socketFactory: (url) => {
      const socket = new FakeSocket(url);
      sockets.push(socket);
      return socket;
    },
  });

  const statuses: StreamStatus[] = [];
  client.onStatus((status) => statuses.push(status));

  return {
    client,
    sockets,
    statuses,
    latest: () => sockets[sockets.length - 1],
    setEndpointFailing: (value: boolean) => {
      failEndpoint = value;
    },
  };
}

/** Connect and settle the endpoint promise, leaving the socket open. */
async function connected(h: ReturnType<typeof harness>) {
  h.client.connect();
  await vi.runOnlyPendingTimersAsync();
  h.latest().open();
  return h.latest();
}

afterEach(() => {
  vi.useRealTimers();
});

describe("backoffDelayMs", () => {
  it("starts at 100ms and doubles", () => {
    expect(backoffDelayMs(1)).toBe(100);
    expect(backoffDelayMs(2)).toBe(200);
    expect(backoffDelayMs(3)).toBe(400);
    expect(backoffDelayMs(4)).toBe(800);
    expect(backoffDelayMs(5)).toBe(1_600);
  });

  it("caps at 10s no matter how many attempts have failed", () => {
    expect(backoffDelayMs(8)).toBe(10_000);
    expect(backoffDelayMs(50)).toBe(MAX_BACKOFF_MS);
  });

  it("treats a zeroth attempt as the first", () => {
    expect(backoffDelayMs(0)).toBe(MIN_BACKOFF_MS);
  });
});

describe("StreamClient", () => {
  it("resolves the endpoint over IPC and connects to the advertised url", async () => {
    vi.useFakeTimers();
    const h = harness({
      endpoint: endpoint({ port: 54321, url: "ws://127.0.0.1:54321/s?token=t" }),
    });

    await connected(h);

    expect(h.latest().url).toBe("ws://127.0.0.1:54321/s?token=t");
    expect(h.client.status).toBe("open");
    expect(h.client.endpoint?.port).toBe(54321);
  });

  it("delivers channel payloads in order to subscribers", async () => {
    vi.useFakeTimers();
    const h = harness();
    const seen: number[] = [];
    h.client.subscribe<{ value: number }>(COUNTER, (payload) => seen.push(payload.value));

    const socket = await connected(h);
    for (let i = 1; i <= 5; i += 1) socket.emitData(COUNTER, i, { value: i * 10 });

    expect(seen).toEqual([10, 20, 30, 40, 50]);
    expect(h.client.lastSequence(COUNTER)).toBe(5);
  });

  it("subscribes on open for channels registered before connecting", async () => {
    vi.useFakeTimers();
    const h = harness();
    h.client.subscribe(COUNTER, () => {});
    h.client.subscribe("health:status", () => {});

    const socket = await connected(h);

    expect(socket.controls()).toEqual([
      {
        type: "subscribe",
        channels: [
          { channel: COUNTER, from_sequence: null },
          { channel: "health:status", from_sequence: null },
        ],
      },
    ]);
  });

  it("subscribes immediately for a channel added while open", async () => {
    vi.useFakeTimers();
    const h = harness();
    const socket = await connected(h);

    h.client.subscribe(COUNTER, () => {});

    expect(socket.controls()).toEqual([
      { type: "subscribe", channels: [{ channel: COUNTER, from_sequence: null }] },
    ]);
  });

  it("unsubscribes only once the last listener for a channel is gone", async () => {
    vi.useFakeTimers();
    const h = harness();
    const socket = await connected(h);

    const first = h.client.subscribe(COUNTER, () => {});
    const second = h.client.subscribe(COUNTER, () => {});
    first();
    expect(socket.controls()).toHaveLength(1);

    second();
    expect(socket.controls()[1]).toEqual({ type: "unsubscribe", channels: [COUNTER] });
    expect(h.client.channels).toEqual([]);
  });

  it("stops delivering to a listener after it unsubscribes", async () => {
    vi.useFakeTimers();
    const h = harness();
    const seen: number[] = [];
    const unsubscribe = h.client.subscribe<number>(COUNTER, (payload) => seen.push(payload));
    const socket = await connected(h);

    socket.emitData(COUNTER, 1, 1);
    unsubscribe();
    socket.emitData(COUNTER, 2, 2);

    expect(seen).toEqual([1]);
  });

  describe("reconnection", () => {
    it("reports reconnecting and retries with exponential backoff", async () => {
      vi.useFakeTimers();
      const h = harness();
      await connected(h);

      h.latest().die();
      expect(h.client.status).toBe("reconnecting");
      expect(h.statuses).toEqual(["connecting", "open", "reconnecting"]);

      // Nothing before the first backoff window elapses.
      await vi.advanceTimersByTimeAsync(99);
      expect(h.sockets).toHaveLength(1);

      await vi.advanceTimersByTimeAsync(1);
      await vi.runOnlyPendingTimersAsync();
      expect(h.sockets).toHaveLength(2);

      // A second failure waits twice as long.
      h.latest().die();
      await vi.advanceTimersByTimeAsync(199);
      expect(h.sockets).toHaveLength(2);
      await vi.advanceTimersByTimeAsync(1);
      await vi.runOnlyPendingTimersAsync();
      expect(h.sockets).toHaveLength(3);
    });

    it("resumes each channel from its last sequence so the stream has no gap", async () => {
      vi.useFakeTimers();
      const h = harness();
      const seen: number[] = [];
      h.client.subscribe<number>(COUNTER, (payload) => seen.push(payload));

      const first = await connected(h);
      first.emitData(COUNTER, 1, 1);
      first.emitData(COUNTER, 2, 2);
      first.die();

      await vi.advanceTimersByTimeAsync(MIN_BACKOFF_MS);
      await vi.runOnlyPendingTimersAsync();
      const second = h.latest();
      second.open();

      expect(second.controls()).toEqual([
        { type: "subscribe", channels: [{ channel: COUNTER, from_sequence: 2 }] },
      ]);

      // The kernel replays from where we left off.
      second.emitData(COUNTER, 3, 3);
      second.emitData(COUNTER, 4, 4);
      expect(seen).toEqual([1, 2, 3, 4]);
    });

    it("re-resolves the endpoint on every attempt, so a restarted kernel's new port is picked up", async () => {
      vi.useFakeTimers();
      let port = 1111;
      const client = new StreamClient({
        resolveEndpoint: async () => {
          port += 1;
          return endpoint({ port, url: `ws://127.0.0.1:${port}/stream?token=t` });
        },
        socketFactory: (url) => new FakeSocket(url),
      });

      client.connect();
      await vi.runOnlyPendingTimersAsync();
      expect(client.endpoint?.port).toBe(1112);
      client.close();
    });

    it("keeps retrying while the kernel is unreachable, then connects when it returns", async () => {
      vi.useFakeTimers();
      const h = harness({ failEndpoint: true });

      h.client.connect();
      await vi.runOnlyPendingTimersAsync();
      expect(h.client.status).toBe("reconnecting");
      expect(h.sockets).toHaveLength(0);

      h.setEndpointFailing(false);
      await vi.advanceTimersByTimeAsync(MIN_BACKOFF_MS);
      await vi.runOnlyPendingTimersAsync();
      h.latest().open();

      expect(h.client.status).toBe("open");
    });

    it("does not reconnect after an explicit close", async () => {
      vi.useFakeTimers();
      const h = harness();
      const socket = await connected(h);

      h.client.close();
      expect(socket.closed).toBe(true);
      expect(h.client.status).toBe("closed");

      await vi.advanceTimersByTimeAsync(MAX_BACKOFF_MS * 2);
      expect(h.sockets).toHaveLength(1);
    });

    it("treats a socket error followed by a close as one disconnection", async () => {
      vi.useFakeTimers();
      const h = harness();
      const socket = await connected(h);

      socket.onerror?.();
      socket.onclose?.();

      await vi.advanceTimersByTimeAsync(MIN_BACKOFF_MS);
      await vi.runOnlyPendingTimersAsync();
      expect(h.sockets).toHaveLength(2);
      // One transition, not two: the status stays "reconnecting" across
      // retries so the indicator does not flicker between attempts.
      expect(h.statuses).toEqual(["connecting", "open", "reconnecting"]);
    });
  });

  describe("heartbeat", () => {
    it("declares the connection dead after the configured missed beats", async () => {
      vi.useFakeTimers();
      const h = harness();
      const socket = await connected(h);

      // 3 × 5s of silence. Just short of it, the connection is still live.
      await vi.advanceTimersByTimeAsync(14_999);
      expect(h.client.status).toBe("open");

      await vi.advanceTimersByTimeAsync(1);
      expect(h.client.status).toBe("reconnecting");
      expect(socket.closed).toBe(true);
    });

    it("a heartbeat keeps the connection alive", async () => {
      vi.useFakeTimers();
      const h = harness();
      const socket = await connected(h);

      for (let beat = 1; beat <= 10; beat += 1) {
        await vi.advanceTimersByTimeAsync(5_000);
        socket.emitControl({ type: "heartbeat", sequence: beat });
      }

      expect(h.client.status).toBe("open");
    });

    it("any frame counts as liveness, not only a heartbeat", async () => {
      vi.useFakeTimers();
      const h = harness();
      h.client.subscribe(COUNTER, () => {});
      const socket = await connected(h);

      for (let i = 1; i <= 5; i += 1) {
        await vi.advanceTimersByTimeAsync(10_000);
        socket.emitData(COUNTER, i, i);
      }

      expect(h.client.status).toBe("open");
    });

    it("honours a heartbeat interval the kernel configured differently", async () => {
      vi.useFakeTimers();
      const h = harness({
        endpoint: endpoint({ heartbeat_interval_ms: 100, missed_heartbeat_limit: 2 }),
      });
      await connected(h);

      await vi.advanceTimersByTimeAsync(199);
      expect(h.client.status).toBe("open");
      await vi.advanceTimersByTimeAsync(1);
      expect(h.client.status).toBe("reconnecting");
    });
  });

  describe("backpressure", () => {
    it("surfaces the kernel's warning with the channel and the count", async () => {
      vi.useFakeTimers();
      const h = harness();
      const events: BackpressureEvent[] = [];
      h.client.onBackpressure((event) => events.push(event));
      h.client.subscribe(COUNTER, () => {});
      const socket = await connected(h);

      socket.emitControl({
        type: "backpressure_warning",
        channel: COUNTER,
        dropped: 42,
        buffer_depth: 1_000,
      });

      expect(events).toEqual([
        { channel: COUNTER, dropped: 42, bufferDepth: 1_000, source: "kernel" },
      ]);
    });

    it("infers a loss from a sequence jump the kernel did not announce", async () => {
      vi.useFakeTimers();
      const h = harness();
      const events: BackpressureEvent[] = [];
      h.client.onBackpressure((event) => events.push(event));
      h.client.subscribe(COUNTER, () => {});
      const socket = await connected(h);

      socket.emitData(COUNTER, 1, 1);
      socket.emitData(COUNTER, 5, 5);

      expect(events).toEqual([{ channel: COUNTER, dropped: 3, bufferDepth: null, source: "gap" }]);
      expect(h.client.lastSequence(COUNTER)).toBe(5);
    });

    it("does not report a kernel-announced drop twice as a gap", async () => {
      vi.useFakeTimers();
      const h = harness();
      const events: BackpressureEvent[] = [];
      h.client.onBackpressure((event) => events.push(event));
      h.client.subscribe(COUNTER, () => {});
      const socket = await connected(h);

      socket.emitData(COUNTER, 1, 1);
      socket.emitControl({
        type: "backpressure_warning",
        channel: COUNTER,
        dropped: 3,
        buffer_depth: 4,
      });
      socket.emitData(COUNTER, 5, 5);

      expect(events).toHaveLength(1);
      expect(events[0].source).toBe("kernel");
    });

    it("ignores a replayed message it has already delivered", async () => {
      vi.useFakeTimers();
      const h = harness();
      const seen: number[] = [];
      h.client.subscribe<number>(COUNTER, (payload) => seen.push(payload));
      const socket = await connected(h);

      socket.emitData(COUNTER, 1, 1);
      socket.emitData(COUNTER, 2, 2);
      socket.emitData(COUNTER, 2, 2); // boundary replay after a resume
      socket.emitData(COUNTER, 3, 3);

      expect(seen).toEqual([1, 2, 3]);
    });
  });

  describe("malformed input", () => {
    it("discards an unparseable frame and keeps the connection", async () => {
      vi.useFakeTimers();
      const h = harness();
      const seen: number[] = [];
      h.client.subscribe<number>(COUNTER, (payload) => seen.push(payload));
      const socket = await connected(h);

      socket.emitRaw("{not json");
      socket.emitData(COUNTER, 1, 1);

      expect(h.client.malformedFrameCount).toBe(1);
      expect(h.client.status).toBe("open");
      expect(seen).toEqual([1]);
    });

    it("ignores binary frames until a consumer for them exists", async () => {
      vi.useFakeTimers();
      const h = harness();
      const socket = await connected(h);

      socket.emitRaw(new ArrayBuffer(4));

      expect(h.client.malformedFrameCount).toBe(0);
      expect(h.client.status).toBe("open");
    });
  });
});
