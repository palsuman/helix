import { describe, expect, it } from "vitest";
import { appError } from "../ipc/errors";
import { createMockIpc } from "./mockIpc";

describe("createMockIpc", () => {
  it("drives the real client with typed handlers and records requests", async () => {
    const kernel = createMockIpc();
    const { commands } = kernel;
    kernel.handle<{ value: number }, { doubled: number }>("test.double", async (payload) => ({
      doubled: payload.value * 2,
    }));
    await expect(
      kernel.client.invoke("test.double", { value: 3 }, { windowId: "window-2" }),
    ).resolves.toEqual({ doubled: 6 });
    kernel.respond("test.fixed", { value: 4 });
    await expect(kernel.client.invoke("test.fixed", {})).resolves.toEqual({ value: 4 });
    expect(commands).toEqual(["test.double", "test.fixed"]);
    expect(kernel.requests[0]).toMatchObject({
      correlation_id: "mock-1",
      window_id: "window-2",
      payload: { value: 3 },
    });
    expect(kernel.requests[1].correlation_id).toBe("mock-2");
    expect(kernel.client.inflight).toEqual([]);
  });

  it.each(["transient", "permanent", "cancelled", "timeout"] as const)(
    "preserves %s kernel errors and allows recovery",
    async (category) => {
      const kernel = createMockIpc();
      kernel.fail("test.failure", appError("TEST_FAILURE", category, "fixture error"));
      await expect(kernel.client.invoke("test.failure", {})).rejects.toMatchObject({
        code: "TEST_FAILURE",
        category,
        correlationId: "mock-1",
      });
      kernel.respond("test.failure", { recovered: true });
      await expect(kernel.client.invoke("test.failure", {})).resolves.toEqual({ recovered: true });
    },
  );

  it("rejects commands without a configured response", async () => {
    const kernel = createMockIpc();
    await expect(kernel.client.invoke("missing", {})).rejects.toThrow(
      "No mock IPC handler for missing",
    );
  });
});
