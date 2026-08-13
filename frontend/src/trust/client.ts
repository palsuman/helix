import type { TrustCapability } from "../generated/TrustCapability";
import type { TrustDecision } from "../generated/TrustDecision";
import type { TrustStatusResponse } from "../generated/TrustStatusResponse";
import { ipc, type InvokeOptions, type IpcClient } from "../ipc";
import {
  TRUST_COMMANDS,
  trustList,
  trustProbe,
  trustRevoke,
  trustSet,
  trustSetEverything,
  trustStatus,
} from "./commands";

/**
 * Frontend client for workspace trust (Task 1.13, REQ-FS-005).
 */
export class TrustClient {
  private readonly client: IpcClient;

  constructor(client: IpcClient = ipc) {
    this.client = client;
  }

  status(paths: readonly string[], options?: InvokeOptions): Promise<TrustStatusResponse> {
    return trustStatus(this.client, paths, options);
  }

  trust(path: string, inheritToChildren = false, options?: InvokeOptions) {
    return trustSet(
      this.client,
      { path, decision: "trusted" satisfies TrustDecision, inherit_to_children: inheritToChildren },
      options,
    );
  }

  restrict(path: string, options?: InvokeOptions) {
    return trustSet(
      this.client,
      { path, decision: "restricted" satisfies TrustDecision, inherit_to_children: false },
      options,
    );
  }

  revoke(path: string, options?: InvokeOptions) {
    return trustRevoke(this.client, { path }, options);
  }

  list(options?: InvokeOptions) {
    return trustList(this.client, options);
  }

  setTrustEverything(enabled: boolean, acknowledgedWarning: boolean, options?: InvokeOptions) {
    return trustSetEverything(
      this.client,
      { enabled, acknowledged_warning: acknowledgedWarning },
      options,
    );
  }

  probe(path: string, capability: TrustCapability, options?: InvokeOptions) {
    return trustProbe(this.client, { path, capability }, options);
  }
}

export const trust = new TrustClient();
export { TRUST_COMMANDS };
