import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppError } from "../generated/AppError";
import type { IpcRequest } from "../generated/IpcRequest";
import type { IpcResponse } from "../generated/IpcResponse";
import { IpcClient, type InvokeFn } from "./client";
import { IpcError, isIpcError } from "./errors";
import { IPC_COMMANDS, ping, sleep } from "./commands";

type DispatchArgs = { request: IpcRequest<unknown> };

/**
 * A stand-in for the kernel: records the requests it receives and answers
 * them with whatever the test dictates. The envelope shapes are the
 * generated types, so this mock cannot drift from the Rust contract without
 * a type error.
 */
function fakeKernel(handler: (request: IpcRequest<unknown>) => unknown) {
  const requests: IpcRequest<unknown>[] = [];
  const cancelled: string[] = [];

  const invoke: InvokeFn = async <T>(command: string, args?: Record<string, unknown>) => {
    if (command === "ipc_cancel") {
      const request = (args as { request: { correlation_id: string } }).request;
      cancelled.push(request.correlation_id);
      return { correlation_id: request.correlation_id, cancelled: true } as T;
    }
    const request = (args as DispatchArgs).request;
    requests.push(request);
    return (await handler(request)) as T;
  };

  return { invoke, requests, cancelled };
}

function ok<T>(correlationId: string, result: T): IpcResponse<T> {
  return { correlation_id: correlationId, result, error: null };
}

function err(correlationId: string, error: AppError): IpcResponse<never> {
  return { correlation_id: correlationId, result: null, error };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("IpcClient", () => {
  it("round-trips a typed command and carries a correlation id and timeout", async () => {
    const kernel = fakeKernel((request) =>
      ok(request.correlation_id, { echo: "hello", kernel_version: "0.1.0" }),
    );
    const client = new IpcClient({
      invoke: kernel.invoke,
      correlationIdFactory: () => "corr-1",
    });

    const response = await ping(client, "hello");

    expect(response.echo).toBe("hello");
    expect(response.kernel_version).toBe("0.1.0");
    expect(kernel.requests).toHaveLength(1);
    expect(kernel.requests[0]).toMatchObject({
      command: IPC_COMMANDS.ping,
      correlation_id: "corr-1",
      timeout_ms: 30_000,
      payload: { message: "hello" },
    });
  });

  it("uses a per-call timeout override", async () => {
    const kernel = fakeKernel((request) => ok(request.correlation_id, { slept_ms: 5 }));
    const client = new IpcClient({ invoke: kernel.invoke });

    await sleep(client, 5, { timeoutMs: 250 });

    expect(kernel.requests[0].timeout_ms).toBe(250);
  });

  it("generates a distinct correlation id per call", async () => {
    const kernel = fakeKernel((request) =>
      ok(request.correlation_id, { echo: "x", kernel_version: "0.1.0" }),
    );
    const client = new IpcClient({ invoke: kernel.invoke });

    await Promise.all([ping(client, "a"), ping(client, "b")]);

    const [first, second] = kernel.requests;
    expect(first.correlation_id).not.toBe(second.correlation_id);
  });

  it("clears in-flight tracking once a call settles", async () => {
    const kernel = fakeKernel((request) =>
      ok(request.correlation_id, { echo: "x", kernel_version: "0.1.0" }),
    );
    const client = new IpcClient({ invoke: kernel.invoke });

    const pending = ping(client, "x");
    expect(client.inflight).toHaveLength(1);
    await pending;
    expect(client.inflight).toHaveLength(0);
  });

  describe("error categories", () => {
    const cases: {
      name: string;
      error: AppError;
      expect: (e: IpcError) => void;
    }[] = [
      {
        name: "transient errors are retryable",
        error: { code: "LOCKED", category: "transient", message: "busy", details: null },
        expect: (e) => {
          expect(e.isTransient).toBe(true);
          expect(e.isRetryable).toBe(true);
          expect(e.isPermanent).toBe(false);
        },
      },
      {
        name: "permanent errors are not retryable",
        error: { code: "NOT_FOUND", category: "permanent", message: "gone", details: null },
        expect: (e) => {
          expect(e.isPermanent).toBe(true);
          expect(e.isRetryable).toBe(false);
        },
      },
      {
        name: "cancelled errors are distinguishable from failures",
        error: { code: "CANCELLED", category: "cancelled", message: "aborted", details: null },
        expect: (e) => {
          expect(e.isCancelled).toBe(true);
          expect(e.isRetryable).toBe(false);
        },
      },
      {
        name: "timeouts are retryable",
        error: { code: "TIMEOUT", category: "timeout", message: "too slow", details: null },
        expect: (e) => {
          expect(e.isTimeout).toBe(true);
          expect(e.isRetryable).toBe(true);
        },
      },
    ];

    for (const testCase of cases) {
      it(testCase.name, async () => {
        const kernel = fakeKernel((request) => err(request.correlation_id, testCase.error));
        const client = new IpcClient({ invoke: kernel.invoke });

        const thrown = await ping(client, "x").catch((e: unknown) => e);

        expect(isIpcError(thrown)).toBe(true);
        const error = thrown as IpcError;
        expect(error.code).toBe(testCase.error.code);
        expect(error.message).toBe(testCase.error.message);
        expect(error.command).toBe(IPC_COMMANDS.ping);
        testCase.expect(error);
      });
    }

    it("preserves error details for diagnostics", async () => {
      const kernel = fakeKernel((request) =>
        err(request.correlation_id, {
          code: "NOT_FOUND",
          category: "permanent",
          message: "gone",
          details: { path: "/tmp/x" },
        }),
      );
      const client = new IpcClient({ invoke: kernel.invoke });

      const error = (await ping(client, "x").catch((e: unknown) => e)) as IpcError;
      expect(error.details).toEqual({ path: "/tmp/x" });
    });
  });

  describe("cancellation", () => {
    it("aborting the signal cancels the command kernel-side", async () => {
      const controller = new AbortController();
      // A kernel that answers only once it has been asked to cancel.
      const kernel = fakeKernel(
        (request) =>
          new Promise((resolve) => {
            controller.signal.addEventListener("abort", () => {
              resolve(
                err(request.correlation_id, {
                  code: "CANCELLED",
                  category: "cancelled",
                  message: "command 'ipc.sleep' was cancelled by the client",
                  details: null,
                }),
              );
            });
          }),
      );
      const client = new IpcClient({
        invoke: kernel.invoke,
        correlationIdFactory: () => "long-running",
      });

      const pending = sleep(client, 10_000, { signal: controller.signal });
      controller.abort();

      const error = (await pending.catch((e: unknown) => e)) as IpcError;
      expect(error.isCancelled).toBe(true);
      expect(kernel.cancelled).toEqual(["long-running"]);
      expect(client.inflight).toHaveLength(0);
    });

    it("a signal already aborted never reaches the kernel", async () => {
      const kernel = fakeKernel((request) => ok(request.correlation_id, { slept_ms: 0 }));
      const client = new IpcClient({ invoke: kernel.invoke });

      const error = (await sleep(client, 10_000, {
        signal: AbortSignal.abort(),
      }).catch((e: unknown) => e)) as IpcError;

      expect(error.isCancelled).toBe(true);
      expect(kernel.requests).toHaveLength(0);
      expect(kernel.cancelled).toHaveLength(0);
    });

    it("cancel resolves false when the correlation id is unknown", async () => {
      const invoke: InvokeFn = async <T>() => ({ correlation_id: "gone", cancelled: false }) as T;
      const client = new IpcClient({ invoke });

      await expect(client.cancel("gone")).resolves.toBe(false);
    });

    it("a failing cancel does not throw into the caller", async () => {
      const invoke: InvokeFn = async () => {
        throw new Error("bridge down");
      };
      const client = new IpcClient({ invoke });

      await expect(client.cancel("whatever")).resolves.toBe(false);
    });
  });

  describe("transport and protocol failures", () => {
    it("maps a transport rejection to a transient error", async () => {
      const invoke: InvokeFn = async () => {
        throw new Error("webview bridge unavailable");
      };
      const client = new IpcClient({ invoke });

      const error = (await ping(client, "x").catch((e: unknown) => e)) as IpcError;
      expect(error.code).toBe("IPC_TRANSPORT_FAILED");
      expect(error.isTransient).toBe(true);
      expect(error.message).toContain("webview bridge unavailable");
    });

    it("discards a response whose correlation id does not match", async () => {
      const kernel = fakeKernel(() => ok("someone-elses-id", { echo: "x", kernel_version: "0" }));
      const client = new IpcClient({
        invoke: kernel.invoke,
        correlationIdFactory: () => "mine",
      });

      const error = (await ping(client, "x").catch((e: unknown) => e)) as IpcError;
      expect(error.code).toBe("CORRELATION_MISMATCH");
      expect(error.isPermanent).toBe(true);
    });

    it("rejects an envelope carrying neither result nor error", async () => {
      const kernel = fakeKernel((request) => ({
        correlation_id: request.correlation_id,
        result: null,
        error: null,
      }));
      const client = new IpcClient({ invoke: kernel.invoke });

      const error = (await ping(client, "x").catch((e: unknown) => e)) as IpcError;
      expect(error.code).toBe("EMPTY_RESPONSE");
    });

    it("times out client-side when the kernel never answers at all", async () => {
      vi.useFakeTimers();
      const kernel = fakeKernel(() => new Promise(() => {}));
      const client = new IpcClient({ invoke: kernel.invoke });

      const pending = ping(client, "x", { timeoutMs: 100 }).catch((e: unknown) => e);
      // The kernel owns the timeout; the client only steps in after a grace
      // period, for the case where the kernel is gone entirely.
      await vi.advanceTimersByTimeAsync(2_101);

      const error = (await pending) as IpcError;
      expect(error.isTimeout).toBe(true);
      expect(error.message).toContain("100ms");
    });
  });
});
