import type { AppError } from "../generated/AppError";
import type { ErrorCategory } from "../generated/ErrorCategory";

/**
 * A kernel error surfaced to frontend code (Task 1.3, REQ-ARCH-003).
 *
 * The kernel returns `Result<T, AppError>`; the client maps the error arm
 * onto this class so every call site can branch on `category` rather than
 * string-matching messages. The four categories are handled distinctly:
 *
 * - `transient` — worth retrying (lock contention, a busy provider).
 * - `permanent` — will not succeed until something changes (missing file,
 *   invalid payload, unknown command).
 * - `cancelled` — the caller aborted it; not a failure to report to the user.
 * - `timeout` — exceeded its budget and was cancelled kernel-side; offer a
 *   retry.
 */
export class IpcError extends Error {
  readonly code: string;
  readonly category: ErrorCategory;
  readonly correlationId: string;
  readonly command: string;
  readonly details: unknown;

  constructor(command: string, correlationId: string, error: AppError) {
    super(error.message);
    this.name = "IpcError";
    this.code = error.code;
    this.category = error.category;
    this.correlationId = correlationId;
    this.command = command;
    this.details = error.details;
  }

  get isTransient(): boolean {
    return this.category === "transient";
  }

  get isPermanent(): boolean {
    return this.category === "permanent";
  }

  get isCancelled(): boolean {
    return this.category === "cancelled";
  }

  get isTimeout(): boolean {
    return this.category === "timeout";
  }

  /** True when retrying the same call has a plausible chance of succeeding. */
  get isRetryable(): boolean {
    return this.isTransient || this.isTimeout;
  }
}

/** Narrowing helper for `catch` blocks, which receive `unknown`. */
export function isIpcError(value: unknown): value is IpcError {
  return value instanceof IpcError;
}

export function appError(
  code: string,
  category: ErrorCategory,
  message: string,
  details?: unknown,
): AppError {
  return { code, category, message, details: details ?? null };
}
