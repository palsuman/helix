import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type { IpcRequest } from "./generated/IpcRequest";
import { IpcClient, type InvokeFn } from "./ipc";
import { DEFAULT_LAYOUT, useLayoutStore, type WorkbenchLayout } from "./workbench/layoutStore";
import { notify, useNotificationStore } from "./notifications";
import App from "./App";
import { keybindingHarness } from "./keybindings/testUtils";
import { detectPlatform } from "./keybindings/schemes";

function shellClient(initialLayout: WorkbenchLayout | null = null) {
  const commands: string[] = [];
  const requests: IpcRequest<unknown>[] = [];
  const invoke: InvokeFn = async <T,>(_endpoint: string, args?: Record<string, unknown>) => {
    const request = (args as { request: IpcRequest<unknown> }).request;
    commands.push(request.command);
    requests.push(request);
    const result =
      request.command === "window.layout.get"
        ? { layout: initialLayout }
        : request.command === "command.list"
          ? { commands: [] }
          : request.command === "keybindings.get"
            ? {
                user: [],
                plugins: [],
                warnings: [],
                revision: "missing",
                writable: true,
                path: null,
              }
            : {};
    return { correlation_id: request.correlation_id, result, error: null } as T;
  };
  return { client: new IpcClient({ invoke }), commands, requests };
}

describe("App", () => {
  beforeEach(() => {
    useLayoutStore.getState().reset();
    useNotificationStore.getState().reset();
  });

  it("opens the shortcut editor and toggles Zen Mode through the shared chord dispatcher", async () => {
    const kernel = keybindingHarness();
    render(<App client={kernel.client} windowId="test-window" />);
    await act(async () => Promise.resolve());
    const modifier = detectPlatform() === "mac" ? { metaKey: true } : { ctrlKey: true };
    fireEvent.keyDown(window, { key: "k", ...modifier });
    fireEvent.keyDown(window, { key: "s", ...modifier });
    expect(await screen.findByRole("region", { name: "Keyboard Shortcuts" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Close Keyboard Shortcuts" }));
    fireEvent.keyDown(window, { key: "k", ...modifier });
    fireEvent.keyDown(window, { key: "z" });
    await waitFor(() => expect(screen.getByTestId("workbench")).toHaveClass("workbench--zen"));
    const executions = kernel.requests.filter((request) => request.command === "command.execute");
    expect(executions.map((request) => (request.payload as { id: string }).id)).toEqual([
      "workbench.action.openGlobalKeybindings",
      "workbench.action.toggleZenMode",
    ]);
  });

  it("docks notifications in the right panel and toggles it from the right activity rail", async () => {
    const { client } = shellClient();
    render(<App client={client} />);
    await act(async () => Promise.resolve());

    const rail = screen.getByRole("navigation", { name: "Right activity rail" });
    const toggle = within(rail).getByRole("button", {
      name: "Notification center, 0 notifications",
    });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(
      within(screen.getByRole("complementary", { name: "Right panel" })).getByRole("region", {
        name: "Notification center",
      }),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole("contentinfo", { name: "Status bar" })).queryByRole("button", {
        name: /Notification center/,
      }),
    ).not.toBeInTheDocument();

    fireEvent.click(toggle);
    expect(screen.queryByRole("complementary", { name: "Right panel" })).not.toBeInTheDocument();
    expect(rail).toBeInTheDocument();
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("complementary", { name: "Left panel" })).toBeInTheDocument();

    fireEvent.click(toggle);
    expect(screen.getByRole("region", { name: "Notification center" })).toBeInTheDocument();
    fireEvent.pointerDown(screen.getByRole("region", { name: "Editor area" }));
    expect(screen.getByRole("region", { name: "Notification center" })).toBeInTheDocument();
  });

  it.each(["left", "right"] as const)(
    "restores the docked panel visibility and width with %s content placement",
    async (position) => {
      const { client } = shellClient({
        ...DEFAULT_LAYOUT,
        primarySidebarPosition: position,
        primarySidebarVisible: position === "left",
        secondarySidebarVisible: position === "right",
        primarySidebarSize: 340,
        secondarySidebarSize: 320,
      });
      render(<App client={client} />);

      const rail = screen.getByRole("navigation", { name: "Right activity rail" });
      const toggle = within(rail).getByRole("button", { name: /Notification center/ });
      await waitFor(() => expect(toggle).toHaveAttribute("aria-expanded", "false"));
      expect(screen.queryByRole("complementary", { name: "Right panel" })).not.toBeInTheDocument();

      fireEvent.click(toggle);
      const panel = screen.getByRole("complementary", { name: "Right panel" });
      expect(panel).toHaveStyle({ width: position === "right" ? "340px" : "320px" });
      const handle = screen.getByRole("separator", { name: "Resize right panel" });
      fireEvent.keyDown(handle, { key: "ArrowLeft" });
      const resizedWidth = handle.getAttribute("aria-valuenow");

      fireEvent.click(screen.getByRole("button", { name: "Hide notification center" }));
      expect(toggle).toHaveFocus();
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(screen.getByRole("complementary", { name: "Left panel" })).toBeInTheDocument();
      fireEvent.click(toggle);
      expect(screen.getByRole("separator", { name: "Resize right panel" })).toHaveAttribute(
        "aria-valuenow",
        resizedWidth,
      );
    },
  );

  it("keeps toasts visible in Zen Mode when the dock is saved as open", async () => {
    const { client } = shellClient();
    render(<App client={client} />);
    await act(async () => Promise.resolve());

    act(() => {
      useLayoutStore.getState().toggleZenMode();
      notify({ kind: "error", source: "Tasks", message: "Build failed" });
    });

    const toasts = within(screen.getByRole("region", { name: "Notifications" }));
    expect(toasts.getByText("Build failed")).toBeInTheDocument();
    expect(useLayoutStore.getState().secondarySidebarVisible).toBe(true);

    act(() => useLayoutStore.getState().toggleZenMode());
    expect(toasts.queryByText("Build failed")).not.toBeInTheDocument();
    expect(screen.getByRole("log", { name: "Notification history" })).toHaveTextContent(
      "Build failed",
    );
  });

  it("renders only the production card-layout shell", () => {
    const { client } = shellClient();
    render(<App client={client} />);

    expect(screen.getByTestId("workbench")).toBeInTheDocument();
    expect(screen.getByRole("banner", { name: "Title bar" })).toHaveClass("workbench-titlebar");
    expect(screen.getByRole("navigation", { name: "Left activity rail" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Right activity rail" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Left panel" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Right panel" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Editor area" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Panel" })).toBeInTheDocument();
    expect(screen.getByRole("contentinfo", { name: "Status bar" })).toBeInTheDocument();

    expect(document.querySelectorAll(".workbench-card")).toHaveLength(4);
    expect(screen.queryByLabelText(/primary|secondary/i)).not.toBeInTheDocument();
    expect(screen.queryByText("Open a file to start editing.")).not.toBeInTheDocument();
    expect(screen.queryByText("Explorer")).not.toBeInTheDocument();
    expect(screen.queryByText("Problems")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Notification center, 0 notifications" }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("button")).toHaveLength(3);
  });

  it("does not mount the retired transport demo", () => {
    const { client, commands } = shellClient();
    render(<App client={client} />);

    expect(screen.queryByText("IPC round trip")).not.toBeInTheDocument();
    expect(screen.queryByText("Cancellation")).not.toBeInTheDocument();
    expect(screen.queryByText("Streaming")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Start 10s command" })).not.toBeInTheDocument();
    expect(commands).not.toContain("ipc.ping");
    expect(commands).not.toContain("ipc.sleep");
  });

  it("scopes layout persistence to the current native window", async () => {
    const { client, requests } = shellClient();
    render(<App client={client} windowId="w-secondary" />);

    await waitFor(() =>
      expect(requests.some((request) => request.command === "window.layout.get")).toBe(true),
    );
    const layoutRequest = requests.find((request) => request.command === "window.layout.get");
    expect(layoutRequest?.window_id).toBe("w-secondary");
    expect(layoutRequest?.payload).toEqual({ id: "w-secondary" });
  });
});
