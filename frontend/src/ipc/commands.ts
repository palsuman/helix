import type { PingRequest } from "../generated/PingRequest";
import type { PingResponse } from "../generated/PingResponse";
import type { SleepRequest } from "../generated/SleepRequest";
import type { SleepResponse } from "../generated/SleepResponse";
import type { InvokeOptions, IpcClient } from "./client";

/**
 * Typed wrappers for the kernel's built-in commands (Task 1.3).
 *
 * Each wrapper pins the request and response types from `src/generated/`, so
 * a change to the Rust definition surfaces here as a type error rather than
 * as a runtime surprise. Domain commands (`file.*`, `config.*`, …) follow the
 * same shape as they land.
 */

export const IPC_COMMANDS = {
  ping: "ipc.ping",
  sleep: "ipc.sleep",
} as const;

export function ping(
  client: IpcClient,
  message: string,
  options?: InvokeOptions,
): Promise<PingResponse> {
  return client.invoke<PingRequest, PingResponse>(IPC_COMMANDS.ping, { message }, options);
}

/**
 * A deliberately long-running kernel command, used to demonstrate and test
 * cancellation and timeout handling.
 */
export function sleep(
  client: IpcClient,
  durationMs: number,
  options?: InvokeOptions,
): Promise<SleepResponse> {
  return client.invoke<SleepRequest, SleepResponse>(
    IPC_COMMANDS.sleep,
    { duration_ms: durationMs },
    options,
  );
}
