import type { TrustEverythingRequest } from "../generated/TrustEverythingRequest";
import type { TrustEverythingResponse } from "../generated/TrustEverythingResponse";
import type { TrustListResponse } from "../generated/TrustListResponse";
import type { TrustProbeRequest } from "../generated/TrustProbeRequest";
import type { TrustProbeResponse } from "../generated/TrustProbeResponse";
import type { TrustRevokeRequest } from "../generated/TrustRevokeRequest";
import type { TrustRevokeResponse } from "../generated/TrustRevokeResponse";
import type { TrustSetRequest } from "../generated/TrustSetRequest";
import type { TrustSetResponse } from "../generated/TrustSetResponse";
import type { TrustStatusRequest } from "../generated/TrustStatusRequest";
import type { TrustStatusResponse } from "../generated/TrustStatusResponse";
import type { InvokeOptions, IpcClient } from "../ipc";

/**
 * Typed wrappers for the kernel's `trust.*` commands (Task 1.13, REQ-FS-005).
 */

export const TRUST_COMMANDS = {
  status: "trust.status",
  set: "trust.set",
  revoke: "trust.revoke",
  list: "trust.list",
  setTrustEverything: "trust.setTrustEverything",
  probe: "trust.probe",
} as const;

/** Emitted when trust decisions or the enabled flag change. */
export const TRUST_CHANNEL = "trust:changed";

export function trustStatus(
  client: IpcClient,
  paths: readonly string[],
  options?: InvokeOptions,
): Promise<TrustStatusResponse> {
  const payload: TrustStatusRequest = { paths: [...paths] };
  return client.invoke<TrustStatusRequest, TrustStatusResponse>(
    TRUST_COMMANDS.status,
    payload,
    options,
  );
}

export function trustSet(
  client: IpcClient,
  request: TrustSetRequest,
  options?: InvokeOptions,
): Promise<TrustSetResponse> {
  return client.invoke<TrustSetRequest, TrustSetResponse>(TRUST_COMMANDS.set, request, options);
}

export function trustRevoke(
  client: IpcClient,
  request: TrustRevokeRequest,
  options?: InvokeOptions,
): Promise<TrustRevokeResponse> {
  return client.invoke<TrustRevokeRequest, TrustRevokeResponse>(
    TRUST_COMMANDS.revoke,
    request,
    options,
  );
}

export function trustList(client: IpcClient, options?: InvokeOptions): Promise<TrustListResponse> {
  return client.invoke<Record<string, never>, TrustListResponse>(TRUST_COMMANDS.list, {}, options);
}

export function trustSetEverything(
  client: IpcClient,
  request: TrustEverythingRequest,
  options?: InvokeOptions,
): Promise<TrustEverythingResponse> {
  return client.invoke<TrustEverythingRequest, TrustEverythingResponse>(
    TRUST_COMMANDS.setTrustEverything,
    request,
    options,
  );
}

export function trustProbe(
  client: IpcClient,
  request: TrustProbeRequest,
  options?: InvokeOptions,
): Promise<TrustProbeResponse> {
  return client.invoke<TrustProbeRequest, TrustProbeResponse>(
    TRUST_COMMANDS.probe,
    request,
    options,
  );
}
