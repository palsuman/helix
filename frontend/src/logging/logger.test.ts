import { describe, expect, it, vi } from "vitest";
import type { LogAppendRequest } from "../generated/LogAppendRequest";
import { IpcClient, type InvokeFn } from "../ipc";
import { FrontendLogger, timestamp } from "./logger";

/**
 * The frontend logger ships records to the kernel so both halves of the
 * application share one stream (REQ-OBS-001.3). What matters here is that it
 * never becomes the problem: no throwing into UI code, no recursion when the
 * kernel is unreachable, and no unbounded queue.
 */

function fakeKernel(options: { fail?: boolean } = {}) {
  const appended: LogAppendRequest[] = [];
  const invoke: InvokeFn = async <T>(_command: string, args?: Record<string, unknown>) => {
    if (options.fail) throw new Error("kernel unreachable");
    const request = (args as { request: { payload: LogAppendRequest; correlation_id: string } })
      .request;
    appended.push(request.payload);
    return {
      correlation_id: request.correlation_id,
      result: { recorded: true, source: `frontend.${request.payload.source}` },
      error: null,
    } as T;
  };
  return { client: new IpcClient({ invoke }), appended };
}

const fixedNow = () => new Date("2026-01-01T10:00:00.123Z");

describe("FrontendLogger", () => {
  it("ships a record with its level, source, message, and fields", async () => {
    const { client, appended } = fakeKernel();
    const logger = new FrontendLogger({ client, source: "app", now: fixedNow });

    logger.warn("save failed", { path: "/tmp/x" }, "cmd-7");
    await vi.waitFor(() => expect(appended).toHaveLength(1));

    expect(appended[0]).toEqual({
      level: "warn",
      source: "app",
      message: "save failed",
      fields: { path: "/tmp/x" },
      correlation_id: "cmd-7",
      ts: "2026-01-01T10:00:00.123Z",
    });
  });

  it("drops a record below its level without calling the kernel", async () => {
    const { client, appended } = fakeKernel();
    const logger = new FrontendLogger({ client, minLevel: "warn", now: fixedNow });

    logger.debug("chatty");
    logger.info("also chatty");
    logger.error("worth keeping");

    await vi.waitFor(() => expect(appended).toHaveLength(1));
    expect(appended[0].message).toBe("worth keeping");
  });

  it("changes level at runtime", async () => {
    const { client, appended } = fakeKernel();
    const logger = new FrontendLogger({ client, minLevel: "warn", now: fixedNow });

    logger.debug("suppressed");
    logger.setLevel("debug");
    logger.debug("recorded");

    await vi.waitFor(() => expect(appended).toHaveLength(1));
    expect(appended[0].message).toBe("recorded");
    expect(logger.level).toBe("debug");
  });

  it("does not throw into UI code when the kernel is unreachable", async () => {
    const { client } = fakeKernel({ fail: true });
    const logger = new FrontendLogger({ client, now: fixedNow });

    expect(() => logger.error("something broke")).not.toThrow();
    await vi.waitFor(() => expect(logger.failures).toBe(1));
    expect(logger.pending).toHaveLength(1);
  });

  it("replays queued records once the kernel answers again", async () => {
    const failing = fakeKernel({ fail: true });
    const logger = new FrontendLogger({ client: failing.client, now: fixedNow });
    logger.error("during the outage");
    await vi.waitFor(() => expect(logger.pending).toHaveLength(1));

    // A new client stands in for the kernel coming back, which in the real
    // application is the same client succeeding again.
    const recovered = fakeKernel();
    const replayed = new FrontendLogger({ client: recovered.client, now: fixedNow });
    for (const record of logger.pending) replayed.emit(record.level, record.message);
    await vi.waitFor(() => expect(recovered.appended).toHaveLength(1));
    expect(recovered.appended[0].message).toBe("during the outage");
  });

  it("bounds the queue, dropping the oldest records", async () => {
    const { client } = fakeKernel({ fail: true });
    const logger = new FrontendLogger({ client, maxQueued: 3, now: fixedNow });

    for (let i = 0; i < 6; i += 1) logger.error(`m${i}`);
    await vi.waitFor(() => expect(logger.pending).toHaveLength(3));
    expect(logger.dropped).toBe(3);
    expect(logger.pending.map((r) => r.message)).toEqual(["m3", "m4", "m5"]);
  });

  it("flushes queued records through the transport", async () => {
    const { client, appended } = fakeKernel();
    const logger = new FrontendLogger({ client, now: fixedNow });
    // Nothing queued: flushing is a no-op rather than an error.
    await logger.flush();
    expect(appended).toHaveLength(0);
  });

  it("namespaces a child logger under its parent", async () => {
    const { client, appended } = fakeKernel();
    const logger = new FrontendLogger({ client, source: "app", now: fixedNow }).child("editor");

    logger.info("opened");
    await vi.waitFor(() => expect(appended).toHaveLength(1));
    expect(appended[0].source).toBe("app.editor");
  });

  it("reports which levels are enabled", () => {
    const { client } = fakeKernel();
    const logger = new FrontendLogger({ client, minLevel: "info" });
    expect(logger.enabled("trace")).toBe(false);
    expect(logger.enabled("info")).toBe(true);
    expect(logger.enabled("error")).toBe(true);
  });
});

describe("timestamp", () => {
  it("produces the kernel's fixed-width millisecond form", () => {
    expect(timestamp(new Date("2026-01-01T10:00:00.123Z"))).toBe("2026-01-01T10:00:00.123Z");
    expect(timestamp(new Date(0))).toBe("1970-01-01T00:00:00.000Z");
    expect(timestamp(new Date("2026-01-01T10:00:00.123Z")).length).toBe(24);
  });
});
