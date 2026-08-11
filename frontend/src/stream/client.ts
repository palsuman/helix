import type { ChannelSubscription } from "../generated/ChannelSubscription";
import type { StreamControl } from "../generated/StreamControl";
import type { StreamEndpoint } from "../generated/StreamEndpoint";
import type { StreamEnvelope } from "../generated/StreamEnvelope";
import type { StreamFrame } from "../generated/StreamFrame";
import { ipc } from "../ipc";
import { streamEndpoint } from "./commands";

/**
 * WebSocket streaming client (Task 1.4, REQ-ARCH-003.5-.10).
 *
 * Tauri IPC handles commands; this handles everything high-frequency:
 * terminal output, agent progress, log tailing, diagnostics push, search
 * results.
 *
 * Three behaviours are the reason this is a class rather than a bare socket:
 *
 * - **Reconnect with backoff.** A kernel restart (Task 1.11) changes the
 *   port, so every attempt re-resolves the endpoint over IPC rather than
 *   reusing a cached URL.
 * - **Resume without a gap.** The last sequence seen per channel is
 *   retained across disconnects and replayed from on reconnect, so a dropped
 *   socket costs nothing as long as the kernel still has the messages
 *   buffered.
 * - **Honest truncation.** Both the kernel's `backpressure_warning` and a
 *   locally observed sequence jump surface as the same event, so the UI can
 *   say "output truncated" instead of quietly showing an incomplete stream.
 */

/** Connection lifecycle, as the UI needs to describe it. */
export type StreamStatus = "idle" | "connecting" | "open" | "reconnecting" | "closed";

/** First reconnect delay (REQ-ARCH-003.7). */
export const MIN_BACKOFF_MS = 100;

/** Backoff ceiling (REQ-ARCH-003.7). */
export const MAX_BACKOFF_MS = 10_000;

/** Fallbacks used only if the endpoint is unreachable before first connect. */
const FALLBACK_HEARTBEAT_INTERVAL_MS = 5_000;
const FALLBACK_MISSED_HEARTBEAT_LIMIT = 3;

/** Messages lost, whether the kernel said so or we inferred it. */
export interface BackpressureEvent {
  channel: string;
  dropped: number;
  /** Configured ring depth, when the kernel reported one. */
  bufferDepth: number | null;
  /**
   * `kernel` — the kernel evicted messages before we read them.
   * `gap` — a sequence jump we observed ourselves, which means messages
   * went missing without an accompanying warning and is worth logging.
   */
  source: "kernel" | "gap";
}

export type ChannelListener<T> = (payload: T, envelope: StreamEnvelope) => void;
export type StatusListener = (status: StreamStatus, reason?: string) => void;
export type BackpressureListener = (event: BackpressureEvent) => void;

/**
 * The subset of `WebSocket` this client uses, so tests can drive the full
 * reconnect and heartbeat state machine without a real socket.
 */
export interface StreamSocket {
  send(data: string): void;
  close(): void;
  onopen: ((event?: unknown) => void) | null;
  onclose: ((event?: unknown) => void) | null;
  onerror: ((event?: unknown) => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
}

export type SocketFactory = (url: string) => StreamSocket;

export interface StreamClientOptions {
  /** Defaults to the `stream.endpoint` IPC command. */
  resolveEndpoint?: () => Promise<StreamEndpoint>;
  socketFactory?: SocketFactory;
  minBackoffMs?: number;
  maxBackoffMs?: number;
}

function defaultSocketFactory(url: string): StreamSocket {
  return new WebSocket(url) as unknown as StreamSocket;
}

/**
 * Exponential backoff, capped (REQ-ARCH-003.7): 100, 200, 400 … 10000ms.
 * `attempt` is 1-based.
 */
export function backoffDelayMs(
  attempt: number,
  minMs: number = MIN_BACKOFF_MS,
  maxMs: number = MAX_BACKOFF_MS,
): number {
  if (attempt <= 1) return minMs;
  const growth = minMs * 2 ** (attempt - 1);
  return Math.min(maxMs, growth);
}

export class StreamClient {
  private readonly resolveEndpointFn: () => Promise<StreamEndpoint>;
  private readonly socketFactory: SocketFactory;
  private readonly minBackoffMs: number;
  private readonly maxBackoffMs: number;

  private readonly channelListeners = new Map<string, Set<ChannelListener<never>>>();
  private readonly statusListeners = new Set<StatusListener>();
  private readonly backpressureListeners = new Set<BackpressureListener>();
  /** Last sequence delivered per channel; survives disconnects on purpose. */
  private readonly cursors = new Map<string, number>();

  private socket: StreamSocket | null = null;
  private currentStatus: StreamStatus = "idle";
  private currentEndpoint: StreamEndpoint | null = null;
  private wanted = false;
  private attempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private watchdogTimer: ReturnType<typeof setTimeout> | undefined;
  /**
   * Incremented whenever a socket is abandoned. Callbacks from a socket
   * whose generation has passed are ignored, which is what stops a late
   * `onclose` from a killed socket from cancelling the reconnect its own
   * death triggered.
   */
  private generation = 0;
  private malformedFrames = 0;

  constructor(options: StreamClientOptions = {}) {
    this.resolveEndpointFn = options.resolveEndpoint ?? (() => streamEndpoint(ipc));
    this.socketFactory = options.socketFactory ?? defaultSocketFactory;
    this.minBackoffMs = options.minBackoffMs ?? MIN_BACKOFF_MS;
    this.maxBackoffMs = options.maxBackoffMs ?? MAX_BACKOFF_MS;
  }

  get status(): StreamStatus {
    return this.currentStatus;
  }

  get endpoint(): StreamEndpoint | null {
    return this.currentEndpoint;
  }

  /** Channels with at least one listener, sorted. */
  get channels(): readonly string[] {
    return [...this.channelListeners.keys()].sort();
  }

  /** Last sequence received on a channel, or undefined if none yet. */
  lastSequence(channel: string): number | undefined {
    return this.cursors.get(channel);
  }

  /** Frames discarded because they did not parse. Surfaced for diagnostics. */
  get malformedFrameCount(): number {
    return this.malformedFrames;
  }

  /** Connect, and keep reconnecting until [`close`] is called. */
  connect(): void {
    if (this.wanted) return;
    this.wanted = true;
    this.attempt = 0;
    void this.openSocket();
  }

  /** Stop reconnecting and close the socket. */
  close(): void {
    this.wanted = false;
    this.generation += 1;
    this.clearTimer("reconnect");
    this.clearTimer("watchdog");
    const socket = this.socket;
    this.socket = null;
    socket?.close();
    this.setStatus("closed");
  }

  onStatus(listener: StatusListener): () => void {
    this.statusListeners.add(listener);
    return () => {
      this.statusListeners.delete(listener);
    };
  }

  onBackpressure(listener: BackpressureListener): () => void {
    this.backpressureListeners.add(listener);
    return () => {
      this.backpressureListeners.delete(listener);
    };
  }

  /**
   * Subscribe to a channel. Returns a function that unsubscribes.
   *
   * Subscribing before the socket is open is fine: the subscription is sent
   * as soon as it opens, and again after every reconnect.
   */
  subscribe<T>(channel: string, listener: ChannelListener<T>): () => void {
    let listeners = this.channelListeners.get(channel);
    const isNewChannel = listeners === undefined;
    if (!listeners) {
      listeners = new Set();
      this.channelListeners.set(channel, listeners);
    }
    listeners.add(listener as ChannelListener<never>);

    if (isNewChannel) {
      this.sendControl({ type: "subscribe", channels: [this.subscriptionFor(channel)] });
    }

    return () => {
      const current = this.channelListeners.get(channel);
      if (!current) return;
      current.delete(listener as ChannelListener<never>);
      if (current.size === 0) {
        this.channelListeners.delete(channel);
        this.sendControl({ type: "unsubscribe", channels: [channel] });
      }
    };
  }

  private subscriptionFor(channel: string): ChannelSubscription {
    return {
      channel,
      // Resume where we left off if we have ever seen this channel, so a
      // reconnect (or a re-subscribe) closes its own gap.
      from_sequence: this.cursors.get(channel) ?? null,
    };
  }

  private async openSocket(): Promise<void> {
    const generation = this.generation;
    this.setStatus(this.attempt === 0 ? "connecting" : "reconnecting");

    let endpoint: StreamEndpoint;
    try {
      endpoint = await this.resolveEndpointFn();
    } catch (error: unknown) {
      // The kernel may simply not be up yet (or is restarting), which is a
      // reconnect case rather than a fatal one.
      this.failAttempt(generation, `endpoint unavailable: ${String(error)}`);
      return;
    }
    if (generation !== this.generation || !this.wanted) return;
    if (this.currentEndpoint && this.currentEndpoint.token !== endpoint.token) {
      // A fresh launch starts channel sequences at 1. Resuming from the old
      // launch's cursor would make those frames look like stale replays.
      this.cursors.clear();
    }
    this.currentEndpoint = endpoint;

    let socket: StreamSocket;
    try {
      socket = this.socketFactory(endpoint.url);
    } catch (error: unknown) {
      this.failAttempt(generation, `socket could not be created: ${String(error)}`);
      return;
    }
    this.socket = socket;

    socket.onopen = () => {
      if (generation !== this.generation) return;
      this.attempt = 0;
      this.setStatus("open");
      this.resubscribeAll();
      this.armWatchdog();
    };
    socket.onmessage = (event) => {
      if (generation !== this.generation) return;
      // Any frame proves liveness, not only a heartbeat.
      this.armWatchdog();
      this.handleFrame(event.data);
    };
    socket.onclose = () => this.handleDisconnect(generation, "socket closed");
    socket.onerror = () => this.handleDisconnect(generation, "socket error");
  }

  /** A failure before the socket existed: no socket to close, just retry. */
  private failAttempt(generation: number, reason: string): void {
    if (generation !== this.generation) return;
    this.generation += 1;
    if (!this.wanted) return;
    this.attempt += 1;
    this.setStatus("reconnecting", reason);
    this.scheduleReconnect();
  }

  private handleDisconnect(generation: number, reason: string): void {
    if (generation !== this.generation) return;
    this.generation += 1;
    this.clearTimer("watchdog");
    const socket = this.socket;
    this.socket = null;
    socket?.close();

    if (!this.wanted) {
      this.setStatus("closed", reason);
      return;
    }
    this.attempt += 1;
    this.setStatus("reconnecting", reason);
    this.scheduleReconnect();
  }

  private scheduleReconnect(): void {
    this.clearTimer("reconnect");
    const delay = backoffDelayMs(this.attempt, this.minBackoffMs, this.maxBackoffMs);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      if (!this.wanted) return;
      void this.openSocket();
    }, delay);
  }

  /**
   * Declare the connection dead after the configured number of missed
   * heartbeats (REQ-ARCH-003.9: 3 × 5s). A half-open TCP connection produces
   * no `onclose`, so silence is the only signal available.
   */
  private armWatchdog(): void {
    this.clearTimer("watchdog");
    const interval = this.currentEndpoint?.heartbeat_interval_ms ?? FALLBACK_HEARTBEAT_INTERVAL_MS;
    const limit = this.currentEndpoint?.missed_heartbeat_limit ?? FALLBACK_MISSED_HEARTBEAT_LIMIT;
    this.watchdogTimer = setTimeout(() => {
      this.watchdogTimer = undefined;
      this.handleDisconnect(this.generation, `no heartbeat for ${limit} intervals`);
    }, interval * limit);
  }

  private clearTimer(which: "reconnect" | "watchdog"): void {
    if (which === "reconnect" && this.reconnectTimer !== undefined) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = undefined;
    }
    if (which === "watchdog" && this.watchdogTimer !== undefined) {
      clearTimeout(this.watchdogTimer);
      this.watchdogTimer = undefined;
    }
  }

  private resubscribeAll(): void {
    const channels = this.channels.map((channel) => this.subscriptionFor(channel));
    if (channels.length === 0) return;
    this.sendControl({ type: "subscribe", channels });
  }

  private sendControl(control: StreamControl): void {
    if (!this.socket || this.currentStatus !== "open") return;
    const frame = { kind: "control", ...control } as StreamFrame;
    try {
      this.socket.send(JSON.stringify(frame));
    } catch {
      // The socket died between the status check and the send. The close
      // handler will drive the reconnect, which re-sends every
      // subscription, so dropping this one is safe.
    }
  }

  private handleFrame(data: unknown): void {
    if (typeof data !== "string") return; // binary frames arrive with Task 6.1
    let frame: StreamFrame;
    try {
      frame = JSON.parse(data) as StreamFrame;
    } catch {
      this.malformedFrames += 1;
      return;
    }

    if (frame.kind === "data") {
      this.deliver(frame);
      return;
    }
    if (frame.kind === "control" && frame.type === "backpressure_warning") {
      this.emitBackpressure({
        channel: frame.channel,
        dropped: frame.dropped,
        bufferDepth: frame.buffer_depth,
        source: "kernel",
      });
      // The kernel has already told us what it dropped; move the cursor past
      // the hole so the jump is not reported a second time as a gap.
      const cursor = this.cursors.get(frame.channel);
      if (cursor !== undefined) {
        this.cursors.set(frame.channel, cursor + frame.dropped);
      }
    }
    // subscribed / unsubscribed / heartbeat / closing need no action beyond
    // the watchdog reset every frame already performed.
  }

  private deliver(envelope: StreamEnvelope): void {
    const cursor = this.cursors.get(envelope.channel);
    if (cursor !== undefined) {
      if (envelope.sequence <= cursor) {
        // A replay of something already delivered, which a resume can
        // produce at a boundary. Dropping it keeps delivery exactly-once
        // from the consumer's point of view.
        return;
      }
      const expected = cursor + 1;
      if (envelope.sequence > expected) {
        this.emitBackpressure({
          channel: envelope.channel,
          dropped: envelope.sequence - expected,
          bufferDepth: null,
          source: "gap",
        });
      }
    }
    this.cursors.set(envelope.channel, envelope.sequence);

    const listeners = this.channelListeners.get(envelope.channel);
    if (!listeners) return;
    for (const listener of [...listeners]) {
      (listener as ChannelListener<unknown>)(envelope.payload, envelope);
    }
  }

  private emitBackpressure(event: BackpressureEvent): void {
    for (const listener of [...this.backpressureListeners]) listener(event);
  }

  private setStatus(status: StreamStatus, reason?: string): void {
    if (this.currentStatus === status) return;
    this.currentStatus = status;
    for (const listener of [...this.statusListeners]) listener(status, reason);
  }
}

/** Process-wide client used by application code. */
export const stream = new StreamClient();
