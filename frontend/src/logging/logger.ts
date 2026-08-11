import type { LogLevel } from "../generated/LogLevel";
import { type IpcClient, ipc } from "../ipc";
import { appendLog } from "./commands";
import { levelRank } from "./filter";

/**
 * The frontend's logger (Task 1.5, REQ-OBS-001.3).
 *
 * Frontend records are shipped to the kernel rather than written anywhere
 * locally. That is what unifies the two streams: one ring buffer, one file,
 * one viewer, one redaction pass. A renderer that kept its own log would
 * produce a second timeline the user has to correlate by hand, which is the
 * problem this requirement exists to remove.
 *
 * Three details that are not obvious:
 *
 * - **Shipping is fire-and-forget.** A UI action must not wait on a log
 *   round trip. Failures are counted, not thrown.
 * - **Failures never log.** A failed `log.append` that logged its own
 *   failure would recurse for as long as the kernel stayed unreachable.
 * - **Records are queued while the kernel is unavailable.** A bounded queue
 *   is drained on the next successful call, so the records explaining a
 *   kernel restart are not the ones lost to it.
 */

/** Records held while the kernel is unreachable. Bounded on purpose. */
const MAX_QUEUED = 200;

export interface PendingRecord {
  level: LogLevel;
  source: string;
  message: string;
  fields: Record<string, unknown>;
  correlation_id: string | null;
  ts: string;
}

export interface FrontendLoggerOptions {
  client?: IpcClient;
  /** Source prefix for every record; the kernel namespaces it under `frontend.`. */
  source?: string;
  /** Records below this level are dropped without a call. */
  minLevel?: LogLevel;
  /** Injectable clock, so tests do not depend on the wall clock. */
  now?: () => Date;
  maxQueued?: number;
}

/** The kernel's fixed-width RFC 3339 form: `2026-08-07T10:30:00.123Z`. */
export function timestamp(now: Date): string {
  return now.toISOString().replace(/(\.\d{3})\d*Z$/, "$1Z");
}

export class FrontendLogger {
  private readonly client: IpcClient;
  private readonly source: string;
  private readonly now: () => Date;
  private readonly maxQueued: number;
  private minLevel: LogLevel;
  private queued: PendingRecord[] = [];
  private droppedCount = 0;
  private failureCount = 0;
  /**
   * Serializes shipping, so records reach the kernel in the order they were
   * emitted rather than in whatever order concurrent IPC calls happen to
   * settle. Two records a millisecond apart would otherwise be able to swap
   * places in the viewer.
   */
  private chain: Promise<void> = Promise.resolve();

  constructor(options: FrontendLoggerOptions = {}) {
    this.client = options.client ?? ipc;
    this.source = options.source ?? "app";
    this.minLevel = options.minLevel ?? "info";
    this.now = options.now ?? (() => new Date());
    this.maxQueued = options.maxQueued ?? MAX_QUEUED;
  }

  /** A logger sharing this one's transport but filing under a sub-source. */
  child(source: string): FrontendLogger {
    return new FrontendLogger({
      client: this.client,
      source: `${this.source}.${source}`,
      minLevel: this.minLevel,
      now: this.now,
      maxQueued: this.maxQueued,
    });
  }

  get level(): LogLevel {
    return this.minLevel;
  }

  setLevel(level: LogLevel): void {
    this.minLevel = level;
  }

  enabled(level: LogLevel): boolean {
    return levelRank(level) >= levelRank(this.minLevel);
  }

  /** Records dropped because the queue was full while the kernel was away. */
  get dropped(): number {
    return this.droppedCount;
  }

  /** Failed `log.append` calls. Surfaced for diagnostics, never logged. */
  get failures(): number {
    return this.failureCount;
  }

  get pending(): readonly PendingRecord[] {
    return this.queued;
  }

  trace(message: string, fields?: Record<string, unknown>, correlationId?: string): void {
    this.emit("trace", message, fields, correlationId);
  }

  debug(message: string, fields?: Record<string, unknown>, correlationId?: string): void {
    this.emit("debug", message, fields, correlationId);
  }

  info(message: string, fields?: Record<string, unknown>, correlationId?: string): void {
    this.emit("info", message, fields, correlationId);
  }

  warn(message: string, fields?: Record<string, unknown>, correlationId?: string): void {
    this.emit("warn", message, fields, correlationId);
  }

  error(message: string, fields?: Record<string, unknown>, correlationId?: string): void {
    this.emit("error", message, fields, correlationId);
  }

  /**
   * Build and ship a record. Returns immediately; the argument object is
   * only constructed once the level check passes, so a disabled level costs
   * one array index lookup.
   */
  emit(
    level: LogLevel,
    message: string,
    fields?: Record<string, unknown>,
    correlationId?: string,
  ): void {
    if (!this.enabled(level)) return;
    const record: PendingRecord = {
      level,
      source: this.source,
      message,
      fields: fields ?? {},
      correlation_id: correlationId ?? null,
      ts: timestamp(this.now()),
    };
    this.ship(record);
  }

  /**
   * Resolve once every record emitted so far has been shipped or queued, then
   * retry anything the kernel refused earlier.
   */
  async flush(): Promise<void> {
    await this.chain;
    const queued = this.queued;
    this.queued = [];
    for (const record of queued) {
      this.ship(record);
    }
    await this.chain;
  }

  /** Queue the record behind everything already in flight. */
  private ship(record: PendingRecord): void {
    this.chain = this.chain.then(() => this.send(record));
  }

  private async send(record: PendingRecord): Promise<void> {
    try {
      await appendLog(this.client, record);
    } catch {
      // Deliberately silent: reporting a logging failure through the logger
      // is how a transient kernel outage becomes an infinite loop.
      this.failureCount += 1;
      this.enqueue(record);
    }
  }

  private enqueue(record: PendingRecord): void {
    if (this.queued.length >= this.maxQueued) {
      // Oldest dropped, matching the streaming layer's backpressure
      // semantics: the newest records are the ones that explain the current
      // state.
      this.queued.shift();
      this.droppedCount += 1;
    }
    this.queued.push(record);
  }
}

/** Process-wide frontend logger used by application code. */
export const log = new FrontendLogger();
