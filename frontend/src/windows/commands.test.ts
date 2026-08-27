import { describe, expect, it } from "vitest";
import { IpcClient, type InvokeFn } from "../ipc";
import type { IpcRequest } from "../generated/IpcRequest";
import {
  closeWindowRecord,
  findWindowByWorkspace,
  focusWindow,
  getWindowLayout,
  listWindows,
  openWindow,
  routeNotification,
  setWindowGeometry,
  setWindowLayout,
  WindowClient,
  windowSession,
} from "./commands";

function kernel(handler: (request: IpcRequest<unknown>) => unknown): IpcClient {
  const invoke: InvokeFn = async <T>(_command: string, args?: Record<string, unknown>) => {
    const request = (args as { request: IpcRequest<unknown> }).request;
    return {
      correlation_id: request.correlation_id,
      result: handler(request),
      error: null,
    } as T;
  };
  return new IpcClient({ invoke });
}

describe("window commands", () => {
  it("asks the kernel to focus an already-open workspace unless a duplicate is requested", async () => {
    const client = kernel((request) => {
      expect(request.command).toBe("window.findByWorkspace");
      const payload = request.payload as { force_new: boolean; roots: string[] };
      expect(payload.roots).toEqual(["/tmp/repo"]);
      expect(payload.force_new).toBe(false);
      return { window_id: "main" };
    });
    await expect(findWindowByWorkspace(client, ["/tmp/repo"])).resolves.toEqual({
      window_id: "main",
    });
  });

  it("routes global notifications to the focused window id from the kernel", async () => {
    const client = kernel((request) => {
      expect(request.command).toBe("window.routeNotification");
      return { window_id: "focused" };
    });
    await expect(routeNotification(client, "global")).resolves.toEqual({ window_id: "focused" });
  });

  it("lists open windows with the active one marked", async () => {
    const client = kernel((request) => {
      expect(request.command).toBe("window.list");
      return {
        windows: [{ id: "main", focused: true, roots: [], workspace_key: null }],
        focused_id: "main",
        restore_enabled: true,
      };
    });
    const listed = await listWindows(client);
    expect(listed.focused_id).toBe("main");
    expect(listed.windows).toHaveLength(1);
  });

  it("invokes host commands for new, duplicate, close, and tab move-to-new-window", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invoke: InvokeFn = async <T>(command: string, args?: Record<string, unknown>) => {
      calls.push({ command, args });
      return "w-2" as T;
    };
    const host = new WindowClient(
      kernel(() => ({})),
      invoke,
    );
    await expect(host.newWindow()).resolves.toBe("w-2");
    await expect(host.openFolder("/tmp/repo", true)).resolves.toBe("w-2");
    await expect(host.duplicateWorkspace()).resolves.toBe("w-2");
    await expect(host.moveEditorToNewWindow()).resolves.toBe("w-2");
    await host.close();
    expect(calls.map((call) => call.command)).toEqual([
      "window_new",
      "window_open_folder",
      "window_duplicate",
      "window_move_editor_to_new",
      "window_close",
    ]);
    expect(calls[1]?.args).toEqual({ path: "/tmp/repo", force_new: true });
  });

  it("opens a window with roots, geometry, and layout over IPC", async () => {
    const geometry = {
      x: 0,
      y: 0,
      width: 1600,
      height: 900,
      maximized: true,
      fullscreen: false,
      monitor: null,
    };
    const client = kernel((request) => {
      expect(request.command).toBe("window.open");
      const payload = request.payload as {
        id: string;
        roots: string[];
        geometry: typeof geometry | null;
        layout: unknown;
      };
      expect(payload.id).toBe("w-9");
      expect(payload.roots).toEqual(["/tmp/repo"]);
      expect(payload.geometry).toEqual(geometry);
      expect(payload.layout).toEqual({ panelSize: 300 });
      return { window: { id: "w-9", focused: true }, focused_existing: false };
    });
    const opened = await openWindow(client, {
      id: "w-9",
      roots: ["/tmp/repo"],
      geometry,
      layout: { panelSize: 300 },
    });
    expect(opened.window.id).toBe("w-9");
    expect(opened.focused_existing).toBe(false);
  });

  it("closes a window and reports whether the kernel shuts down", async () => {
    const client = kernel((request) => {
      expect(request.command).toBe("window.close");
      expect((request.payload as { id: string }).id).toBe("w-1");
      return { closed: true, workspace_torn_down: true, remaining: 0, shutdown_kernel: true };
    });
    await expect(closeWindowRecord(client, "w-1")).resolves.toMatchObject({
      closed: true,
      shutdown_kernel: true,
    });
  });

  it("focuses a window by id", async () => {
    const client = kernel((request) => {
      expect(request.command).toBe("window.focus");
      expect((request.payload as { id: string }).id).toBe("w-2");
      return { id: "w-2", focused: true, roots: [], geometry: {}, layout: null };
    });
    await expect(focusWindow(client, "w-2")).resolves.toMatchObject({ id: "w-2" });
  });

  it("persists window geometry per window", async () => {
    const geometry = {
      x: 10,
      y: 20,
      width: 800,
      height: 600,
      maximized: false,
      fullscreen: false,
      monitor: "External Display",
    };
    const client = kernel((request) => {
      expect(request.command).toBe("window.setGeometry");
      const payload = request.payload as { id: string; geometry: typeof geometry };
      expect(payload.id).toBe("w-3");
      expect(payload.geometry.monitor).toBe("External Display");
      return { id: "w-3", focused: false, roots: [], geometry, layout: null };
    });
    await expect(setWindowGeometry(client, "w-3", geometry)).resolves.toMatchObject({
      id: "w-3",
    });
  });

  it("round-trips per-window layout through the kernel", async () => {
    const seen: string[] = [];
    const client = kernel((request) => {
      seen.push(request.command);
      if (request.command === "window.layout.get") {
        return { layout: { panelSize: 240 } };
      }
      return { layout: (request.payload as { layout: unknown }).layout };
    });
    await setWindowLayout(client, "w-4", { panelSize: 240 });
    await expect(getWindowLayout(client, "w-4")).resolves.toEqual({
      layout: { panelSize: 240 },
    });
    expect(seen).toEqual(["window.layout.set", "window.layout.get"]);
  });

  it("loads the saved window session for restore-on-relaunch", async () => {
    const client = kernel((request) => {
      expect(request.command).toBe("window.session");
      return {
        session: {
          windows: [
            { id: "main", focused: true, roots: ["/tmp/repo"], geometry: {}, layout: {} },
          ],
          focused_id: "main",
        },
        restore_enabled: true,
      };
    });
    const loaded = await windowSession(client);
    expect(loaded.restore_enabled).toBe(true);
    expect(loaded.session.windows[0]?.id).toBe("main");
  });
});
