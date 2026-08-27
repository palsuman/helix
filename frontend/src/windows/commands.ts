import type { WindowCloseResponse } from "../generated/WindowCloseResponse";
import type { WindowFindRequest } from "../generated/WindowFindRequest";
import type { WindowFindResponse } from "../generated/WindowFindResponse";
import type { WindowGeometry } from "../generated/WindowGeometry";
import type { WindowGeometryRequest } from "../generated/WindowGeometryRequest";
import type { WindowIdRequest } from "../generated/WindowIdRequest";
import type { WindowLayoutGetRequest } from "../generated/WindowLayoutGetRequest";
import type { WindowLayoutGetResponse } from "../generated/WindowLayoutGetResponse";
import type { WindowLayoutSetRequest } from "../generated/WindowLayoutSetRequest";
import type { WindowListResponse } from "../generated/WindowListResponse";
import type { WindowOpenRequest } from "../generated/WindowOpenRequest";
import type { WindowOpenResponse } from "../generated/WindowOpenResponse";
import type { WindowRecord } from "../generated/WindowRecord";
import type { WindowRouteRequest } from "../generated/WindowRouteRequest";
import type { WindowRouteResponse } from "../generated/WindowRouteResponse";
import type { WindowSessionResponse } from "../generated/WindowSessionResponse";
import type { NotificationScope } from "../generated/NotificationScope";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { InvokeFn, InvokeOptions, IpcClient } from "../ipc";
import { ipc } from "../ipc";

export const WINDOW_COMMANDS = {
  list: "window.list",
  open: "window.open",
  close: "window.close",
  focus: "window.focus",
  find: "window.findByWorkspace",
  session: "window.session",
  setGeometry: "window.setGeometry",
  getLayout: "window.layout.get",
  setLayout: "window.layout.set",
  route: "window.routeNotification",
} as const;

export const HOST_WINDOW_COMMANDS = {
  new: "window_new",
  openFolder: "window_open_folder",
  duplicate: "window_duplicate",
  moveEditorToNew: "window_move_editor_to_new",
  close: "window_close",
} as const;

export function listWindows(client: IpcClient, options?: InvokeOptions) {
  return client.invoke<Record<string, never>, WindowListResponse>(
    WINDOW_COMMANDS.list,
    {},
    options,
  );
}

export function openWindow(client: IpcClient, request: WindowOpenRequest, options?: InvokeOptions) {
  return client.invoke<WindowOpenRequest, WindowOpenResponse>(
    WINDOW_COMMANDS.open,
    request,
    options,
  );
}

export function closeWindowRecord(client: IpcClient, id: string, options?: InvokeOptions) {
  return client.invoke<WindowIdRequest, WindowCloseResponse>(
    WINDOW_COMMANDS.close,
    { id },
    options,
  );
}

export function focusWindow(client: IpcClient, id: string, options?: InvokeOptions) {
  return client.invoke<WindowIdRequest, WindowRecord>(WINDOW_COMMANDS.focus, { id }, options);
}

export function findWindowByWorkspace(
  client: IpcClient,
  roots: readonly string[],
  forceNew = false,
  options?: InvokeOptions,
) {
  const request: WindowFindRequest = { roots: [...roots], force_new: forceNew };
  return client.invoke<WindowFindRequest, WindowFindResponse>(
    WINDOW_COMMANDS.find,
    request,
    options,
  );
}

export function windowSession(client: IpcClient, options?: InvokeOptions) {
  return client.invoke<Record<string, never>, WindowSessionResponse>(
    WINDOW_COMMANDS.session,
    {},
    options,
  );
}

export function setWindowGeometry(
  client: IpcClient,
  id: string,
  geometry: WindowGeometry,
  options?: InvokeOptions,
) {
  const request: WindowGeometryRequest = { id, geometry };
  return client.invoke<WindowGeometryRequest, WindowRecord>(
    WINDOW_COMMANDS.setGeometry,
    request,
    options,
  );
}

export function getWindowLayout(client: IpcClient, id = "", options?: InvokeOptions) {
  const request: WindowLayoutGetRequest = { id };
  return client.invoke<WindowLayoutGetRequest, WindowLayoutGetResponse>(
    WINDOW_COMMANDS.getLayout,
    request,
    options,
  );
}

export function setWindowLayout(
  client: IpcClient,
  id: string,
  layout: unknown,
  options?: InvokeOptions,
) {
  const request: WindowLayoutSetRequest = { id, layout };
  return client.invoke<WindowLayoutSetRequest, WindowLayoutGetResponse>(
    WINDOW_COMMANDS.setLayout,
    request,
    options,
  );
}

export function routeNotification(
  client: IpcClient,
  scope: NotificationScope,
  originWindowId?: string | null,
  options?: InvokeOptions,
) {
  const request: WindowRouteRequest = {
    scope,
    origin_window_id: originWindowId ?? null,
  };
  return client.invoke<WindowRouteRequest, WindowRouteResponse>(
    WINDOW_COMMANDS.route,
    request,
    options,
  );
}

export class WindowClient {
  private readonly client: IpcClient;
  private readonly invokeFn?: InvokeFn;

  constructor(client: IpcClient = ipc, invokeFn?: InvokeFn) {
    this.client = client;
    this.invokeFn = invokeFn;
  }

  list(options?: InvokeOptions) {
    return listWindows(this.client, options);
  }

  newWindow() {
    return this.hostInvoke<string>(HOST_WINDOW_COMMANDS.new);
  }

  openFolder(path: string, forceNew = false) {
    return this.hostInvoke<string>(HOST_WINDOW_COMMANDS.openFolder, { path, force_new: forceNew });
  }

  duplicateWorkspace() {
    return this.hostInvoke<string>(HOST_WINDOW_COMMANDS.duplicate);
  }

  moveEditorToNewWindow() {
    return this.hostInvoke<string>(HOST_WINDOW_COMMANDS.moveEditorToNew);
  }

  close() {
    return this.hostInvoke<void>(HOST_WINDOW_COMMANDS.close);
  }

  private hostInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    if (!this.invokeFn) {
      return Promise.reject(new Error(`host command '${command}' requires a Tauri invoke`));
    }
    return this.invokeFn<T>(command, args);
  }
}

export const windows = new WindowClient(ipc, tauriInvoke as InvokeFn);
