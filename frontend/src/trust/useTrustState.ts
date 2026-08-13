import { useCallback, useEffect, useMemo, useState } from "react";
import type { TrustStatusResponse } from "../generated/TrustStatusResponse";
import type { IpcClient } from "../ipc";
import type { StreamClient } from "../stream";
import { TRUST_CHANNEL } from "./commands";
import { TrustClient } from "./client";

export interface TrustState {
  status: TrustStatusResponse | null;
  loading: boolean;
  error: string | null;
  refresh: () => void;
  trust: (path: string, inheritToChildren?: boolean) => Promise<void>;
  restrict: (path: string) => Promise<void>;
  revoke: (path: string) => Promise<void>;
}

export function useTrustState(
  paths: readonly string[],
  client: IpcClient,
  streamClient: StreamClient,
): TrustState {
  const [status, setStatus] = useState<TrustStatusResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const trustClient = useMemo(() => new TrustClient(client), [client]);

  const refresh = useCallback(() => {
    void trustClient.status(paths).then(
      (next) => {
        setStatus(next);
        setError(null);
        setLoading(false);
      },
      (reason: unknown) => {
        setError(String(reason));
        setLoading(false);
      },
    );
  }, [paths, trustClient]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useEffect(() => streamClient.subscribe(TRUST_CHANNEL, () => refresh()), [streamClient, refresh]);

  const trust = useCallback(
    async (path: string, inheritToChildren = false) => {
      await trustClient.trust(path, inheritToChildren);
      refresh();
    },
    [trustClient, refresh],
  );

  const restrict = useCallback(
    async (path: string) => {
      await trustClient.restrict(path);
      refresh();
    },
    [trustClient, refresh],
  );

  const revoke = useCallback(
    async (path: string) => {
      await trustClient.revoke(path);
      refresh();
    },
    [trustClient, refresh],
  );

  return { status, loading, error, refresh, trust, restrict, revoke };
}
