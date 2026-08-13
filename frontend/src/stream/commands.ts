import type { StreamEndpoint } from "../generated/StreamEndpoint";
import type { StreamEndpointRequest } from "../generated/StreamEndpointRequest";
import type { InvokeOptions, IpcClient } from "../ipc";

/**
 * IPC commands belonging to the streaming layer (Task 1.4).
 *
 * The socket's port is assigned by the OS at launch and its token is
 * generated per launch, so neither can be compiled into the frontend. Both
 * arrive over IPC, which is already authenticated by virtue of being the
 * webview bridge.
 */

export const STREAM_COMMANDS = {
  endpoint: "stream.endpoint",
} as const;

/**
 * Channels the kernel publishes today. Terminal, diagnostics, search, and
 * agent channels join this list as their subsystems land.
 */
export const STREAM_CHANNELS = {
  /** The Task 1.4 demo stream: a 100Hz counter. */
  demoCounter: "demo:counter",
  /**
   * Every log record as it is emitted, kernel and frontend alike (Task 1.5,
   * REQ-OBS-001.3). The kernel publishes nothing here while no one is
   * subscribed, so a closed log viewer costs nothing.
   */
  logEntries: "log:entries",
  /** Workspace trust changes (Task 1.13, REQ-FS-005). */
  trustChanged: "trust:changed",
} as const;

export function streamEndpoint(
  client: IpcClient,
  options?: InvokeOptions,
): Promise<StreamEndpoint> {
  return client.invoke<StreamEndpointRequest, StreamEndpoint>(
    STREAM_COMMANDS.endpoint,
    {},
    options,
  );
}
