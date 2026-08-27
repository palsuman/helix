import type { LayoutGetRequest } from "../generated/LayoutGetRequest";
import type { LayoutGetResponse } from "../generated/LayoutGetResponse";
import type { LayoutSetRequest } from "../generated/LayoutSetRequest";
import type { LayoutSetResponse } from "../generated/LayoutSetResponse";
import type { WorkspaceListRequest } from "../generated/WorkspaceListRequest";
import type { WorkspaceListResponse } from "../generated/WorkspaceListResponse";
import type { InvokeOptions, IpcClient } from "../ipc";

export const LAYOUT_COMMANDS = {
  get: "state.layout.get",
  set: "state.layout.set",
  listWorkspaces: "workspace.list",
} as const;

export function listWorkspaces(client: IpcClient, options?: InvokeOptions) {
  return client.invoke<WorkspaceListRequest, WorkspaceListResponse>(
    LAYOUT_COMMANDS.listWorkspaces,
    {},
    options,
  );
}

export function getLayout(client: IpcClient, workspaceKey: string, options?: InvokeOptions) {
  const request: LayoutGetRequest = { workspace_key: workspaceKey };
  return client.invoke<LayoutGetRequest, LayoutGetResponse>(LAYOUT_COMMANDS.get, request, options);
}

export function setLayout(
  client: IpcClient,
  workspaceKey: string,
  layout: unknown,
  options?: InvokeOptions,
) {
  const request: LayoutSetRequest = { workspace_key: workspaceKey, layout };
  return client.invoke<LayoutSetRequest, LayoutSetResponse>(LAYOUT_COMMANDS.set, request, options);
}
