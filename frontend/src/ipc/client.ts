import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { CancelRequest } from "../generated/CancelRequest";
import type { CancelResponse } from "../generated/CancelResponse";
import type { IpcRequest } from "../generated/IpcRequest";
import type { IpcResponse } from "../generated/IpcResponse";
import { IpcError, appError } from "./errors";

/**
 * Typed IPC client wrapper (Task 1.3, REQ-ARCH-003.1-.4).
 *
 * Every call carries a correlation ID, runs under a timeout, and is
 * cancellable through a standard `AbortSignal`. Kernel errors arrive as
 * `IpcError` with their category intact so callers can branch on
 * transient / permanent / cancelled / timeout.
 *
 * The request and response shapes are the generated types in
 * `src/generated/`, produced from the Rust definitions in `helix-ipc`, so
 * this wrapper adds behaviour without re-declaring the contract.
 */

/** The two Tauri entry points the kernel exposes for the command layer. */
const DISPATCH_ENDPOINT = "ipc_dispatch";
const CANCEL_ENDPOINT = "ipc_cancel";

/** Matches `helix_ipc::envelope::DEFAULT_TIMEOUT_MS`. */
export const DEFAULT_TIMEOUT_MS = 30_000;

/**
 * Grace period added to the kernel timeout before the client gives up on
 * its own. The kernel is authoritative for timeouts; this only covers the
 * case where the kernel never answers at all (a crashed or wedged process),
 * which would otherwise leave the promise pending forever.
 */
const CLIENT_TIMEOUT_GRACE_MS = 2_000;

export interface InvokeOptions {
  /** Overrides the client default. The kernel enforces this. */
  timeoutMs?: number;
  /** Aborting the signal cancels the command kernel-side. */
  signal?: AbortSignal;
}

/** The subset of Tauri's `invoke` this client needs; swappable in tests. */
export type InvokeFn = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export interface IpcClientOptions {
  invoke?: InvokeFn;
  defaultTimeoutMs?: number;
  correlationIdFactory?: () => string;
}

let fallbackCounter = 0;

function defaultCorrelationId(): string {
  const uuid = globalThis.crypto?.randomUUID?.();
  if (uuid) return uuid;
  fallbackCounter += 1;
  return `corr-${Date.now().toString(36)}-${fallbackCounter}`;
}

export class IpcClient {
  private readonly invokeFn: InvokeFn;
  private readonly defaultTimeoutMs: number;
  private readonly nextCorrelationId: () => string;
  private readonly inflightIds = new Set<string>();

  constructor(options: IpcClientOptions = {}) {
    this.invokeFn = options.invoke ?? (tauriInvoke as InvokeFn);
    this.defaultTimeoutMs = options.defaultTimeoutMs ?? DEFAULT_TIMEOUT_MS;
    this.nextCorrelationId = options.correlationIdFactory ?? defaultCorrelationId;
  }

  /** Correlation IDs currently awaiting a response. */
  get inflight(): readonly string[] {
    return [...this.inflightIds];
  }

  /**
   * Invoke a kernel command and resolve with its typed result, or reject
   * with an {@link IpcError}.
   */
  async invoke<TPayload, TResult>(
    command: string,
    payload: TPayload,
    options: InvokeOptions = {},
  ): Promise<TResult> {
    const correlationId = this.nextCorrelationId();
    const timeoutMs = options.timeoutMs ?? this.defaultTimeoutMs;
    const signal = options.signal;

    if (signal?.aborted) {
      // Nothing was sent, so there is nothing to cancel kernel-side.
      throw new IpcError(
        command,
        correlationId,
        appError("CANCELLED", "cancelled", `command '${command}' was cancelled before it was sent`),
      );
    }

    const request: IpcRequest<TPayload> = {
      command,
      correlation_id: correlationId,
      payload,
      timeout_ms: timeoutMs,
    };

    const onAbort = () => {
      // Fire-and-forget: the kernel's cancelled response is what settles
      // the pending promise below.
      void this.cancel(correlationId);
    };
    signal?.addEventListener("abort", onAbort, { once: true });

    this.inflightIds.add(correlationId);
    try {
      const response = await this.withClientTimeout(
        this.invokeFn<IpcResponse<TResult>>(DISPATCH_ENDPOINT, { request }),
        command,
        correlationId,
        timeoutMs,
      );
      return this.unwrap(command, correlationId, response);
    } finally {
      this.inflightIds.delete(correlationId);
      signal?.removeEventListener("abort", onAbort);
    }
  }

  /**
   * Ask the kernel to abort an in-flight command. Resolves false when the
   * correlation ID was not in flight, which is the benign race of a command
   * that already finished.
   */
  async cancel(correlationId: string): Promise<boolean> {
    const request: CancelRequest = { correlation_id: correlationId };
    try {
      const response = await this.invokeFn<CancelResponse>(CANCEL_ENDPOINT, { request });
      return response.cancelled;
    } catch {
      // A failed cancel must never mask the original command's outcome.
      return false;
    }
  }

  private async withClientTimeout<T>(
    pending: Promise<T>,
    command: string,
    correlationId: string,
    timeoutMs: number,
  ): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const guard = new Promise<never>((_resolve, reject) => {
      timer = setTimeout(
        () =>
          reject(
            new IpcError(
              command,
              correlationId,
              appError(
                "TIMEOUT",
                "timeout",
                `command '${command}' produced no response within ${timeoutMs}ms; the kernel may be unavailable`,
              ),
            ),
          ),
        timeoutMs + CLIENT_TIMEOUT_GRACE_MS,
      );
    });

    try {
      return await Promise.race([pending, guard]);
    } catch (error) {
      // Transport rejections (webview bridge down, kernel restarting) are
      // transient: the same call is worth retrying once the kernel returns.
      if (error instanceof IpcError) throw error;
      throw new IpcError(
        command,
        correlationId,
        appError(
          "IPC_TRANSPORT_FAILED",
          "transient",
          `command '${command}' could not reach the kernel: ${String(error)}`,
          error,
        ),
      );
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }
  }

  private unwrap<TResult>(
    command: string,
    correlationId: string,
    response: IpcResponse<TResult>,
  ): TResult {
    if (response.correlation_id !== correlationId) {
      // An orphan response is discarded rather than misattributed
      // (design document Property 4).
      throw new IpcError(
        command,
        correlationId,
        appError(
          "CORRELATION_MISMATCH",
          "permanent",
          `response for '${command}' carried correlation id '${response.correlation_id}', expected '${correlationId}'`,
        ),
      );
    }

    if (response.error !== null) {
      throw new IpcError(command, correlationId, response.error);
    }

    if (response.result === null) {
      throw new IpcError(
        command,
        correlationId,
        appError(
          "EMPTY_RESPONSE",
          "permanent",
          `command '${command}' returned neither a result nor an error`,
        ),
      );
    }

    return response.result;
  }
}

/** Process-wide client used by application code. */
export const ipc = new IpcClient();
