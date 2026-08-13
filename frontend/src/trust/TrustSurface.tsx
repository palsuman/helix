import type { IpcClient } from "../ipc";
import type { StreamClient } from "../stream";
import type { WorkspaceListRequest } from "../generated/WorkspaceListRequest";
import type { WorkspaceListResponse } from "../generated/WorkspaceListResponse";
import { useEffect, useMemo, useState } from "react";
import { TrustClient } from "./client";
import { TrustBanner } from "./TrustBanner";
import { TrustManager } from "./TrustManager";
import { TrustPrompt } from "./TrustPrompt";
import { TrustStatusBar } from "./TrustStatusBar";
import { useTrustState } from "./useTrustState";

const WORKSPACE_LIST = "workspace.list";
const WORKSPACE_CHANNEL = "workspace:changed";

export function TrustSurface({
  paths,
  client,
  streamClient,
  trustClient,
}: {
  paths?: readonly string[];
  client: IpcClient;
  streamClient: StreamClient;
  trustClient?: TrustClient;
}) {
  const resolvedTrustClient = useMemo(
    () => trustClient ?? new TrustClient(client),
    [trustClient, client],
  );
  const discovered = useOpenWorkspacePaths(client, streamClient, paths === undefined);
  const effectivePaths = paths ?? discovered.paths;
  const { status, loading, error, trust, restrict } = useTrustState(
    effectivePaths,
    client,
    streamClient,
  );

  if ((loading || discovered.loading) && !status) {
    return <p role="status">Loading workspace trust…</p>;
  }
  if (discovered.error) {
    return <p role="alert">Workspace trust roots unavailable: {discovered.error}</p>;
  }
  if (error) {
    return <p role="alert">Trust status unavailable: {error}</p>;
  }
  if (!status) return null;

  return (
    <>
      <TrustPrompt
        status={status}
        onTrust={(path) => {
          void trust(path);
        }}
        onRestrict={(path) => {
          void restrict(path);
        }}
      />
      <TrustBanner
        status={status}
        onTrust={(path) => {
          void trust(path);
        }}
      />
      <TrustStatusBar status={status} />
      <TrustManager client={resolvedTrustClient} />
    </>
  );
}

function useOpenWorkspacePaths(
  client: IpcClient,
  streamClient: StreamClient,
  enabled: boolean,
): { paths: readonly string[]; loading: boolean; error: string | null } {
  const [paths, setPaths] = useState<readonly string[]>([]);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    let active = true;
    const refresh = () => {
      void client.invoke<WorkspaceListRequest, WorkspaceListResponse>(WORKSPACE_LIST, {}).then(
        (response) => {
          if (!active) return;
          const unique = new Set<string>();
          for (const workspace of response.workspaces) {
            for (const root of workspace.roots) unique.add(root.path);
          }
          setPaths([...unique]);
          setError(null);
          setLoading(false);
        },
        (reason: unknown) => {
          if (active) {
            setError(String(reason));
            setLoading(false);
          }
        },
      );
    };
    refresh();
    const unsubscribe = streamClient.subscribe(WORKSPACE_CHANNEL, refresh);
    return () => {
      active = false;
      unsubscribe();
    };
  }, [client, enabled, streamClient]);

  return { paths, loading, error };
}
