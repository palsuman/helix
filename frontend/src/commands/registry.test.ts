import { describe, expect, it } from "vitest";
import type { IpcRequest } from "../generated/IpcRequest";
import { IpcClient, type InvokeFn } from "../ipc";
import { COMMANDS, CommandRegistry } from "./registry";

describe("frontend command registry", () => {
  it("registers dynamically and routes renderer execution through IPC", async () => {
    const requests: IpcRequest<unknown>[] = [];
    const invoke: InvokeFn = async <T>(_endpoint: string, args?: Record<string, unknown>) => {
      const request = (args as { request: IpcRequest<unknown> }).request;
      requests.push(request);
      const result =
        request.command === COMMANDS.register
          ? { replaced: false }
          : { id: "plugin.hello", target: { kind: "renderer" }, arguments: { name: "Ada" } };
      return { correlation_id: request.correlation_id, result, error: null } as T;
    };
    const registry = new CommandRegistry(new IpcClient({ invoke }), "window-2");
    const calls: unknown[] = [];
    registry.registerHandler("plugin.hello", (argumentsValue) => calls.push(argumentsValue));

    await expect(
      registry.register({
        id: "plugin.hello",
        title: "Hello",
        category: "Plugin",
        enablement: null,
        disabled_reason: null,
        keybinding: null,
        source: "plugin.test",
        target: { kind: "renderer" },
      }),
    ).resolves.toBe(false);
    await registry.execute("plugin.hello", { name: "Ada" });

    expect(requests.map((request) => request.command)).toEqual([
      COMMANDS.register,
      COMMANDS.execute,
    ]);
    expect(requests.every((request) => request.window_id === "window-2")).toBe(true);
    expect(calls).toEqual([{ name: "Ada" }]);
    expect(registry.recent()).toEqual(["plugin.hello"]);
  });

  it("dispatches IPC-target commands and updates MRU only after success", async () => {
    const commands: string[] = [];
    const invoke: InvokeFn = async <T>(_endpoint: string, args?: Record<string, unknown>) => {
      const request = (args as { request: IpcRequest<unknown> }).request;
      commands.push(request.command);
      const result =
        request.command === COMMANDS.execute
          ? {
              id: "kernel.run",
              target: { kind: "ipc", command: "tasks.run" },
              arguments: { id: 7 },
            }
          : { started: true };
      return { correlation_id: request.correlation_id, result, error: null } as T;
    };
    const registry = new CommandRegistry(new IpcClient({ invoke }), "main");

    await registry.execute("kernel.run");
    expect(commands).toEqual([COMMANDS.execute, "tasks.run"]);
    expect(registry.recent()).toEqual(["kernel.run"]);
  });
});
