import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IpcRequest } from "../generated/IpcRequest";
import { IpcClient, type InvokeFn } from "../ipc";
import {
  DEFAULT_LAYOUT,
  layoutSnapshot,
  type WorkbenchLayout,
  useLayoutStore,
} from "./layoutStore";
import { WorkbenchShell } from "./WorkbenchShell";

function layoutClient(saved: WorkbenchLayout | null = null) {
  const writes: WorkbenchLayout[] = [];
  const cancellations: string[] = [];
  let current = saved;
  let reads = 0;
  const invoke: InvokeFn = async <T,>(_endpoint: string, args?: Record<string, unknown>) => {
    if (_endpoint === "ipc_cancel") {
      const request = (args as { request: { correlation_id: string } }).request;
      cancellations.push(request.correlation_id);
      return { cancelled: true } as T;
    }
    const request = (args as { request: IpcRequest<unknown> }).request;
    let result: unknown;
    if (request.command === "window.layout.get") {
      reads += 1;
      result = { layout: current };
    } else if (request.command === "window.layout.set") {
      const payload = request.payload as { layout: WorkbenchLayout };
      writes.push(payload.layout);
      current = payload.layout;
      result = { layout: current };
    } else {
      throw new Error(`unexpected command ${request.command}`);
    }
    return {
      correlation_id: request.correlation_id,
      result,
      error: null,
    } as T;
  };
  return {
    client: new IpcClient({ invoke }),
    writes,
    cancellations,
    reads: () => reads,
    replaceKernelLayout: (layout: WorkbenchLayout) => {
      current = layout;
    },
  };
}

describe("WorkbenchShell", () => {
  beforeEach(() => {
    useLayoutStore.getState().restore(DEFAULT_LAYOUT);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("toggles the active sidebar and reopens it after closing its header", async () => {
    const { client } = layoutClient();
    render(<WorkbenchShell client={client} showLayoutControls={false} />);
    await act(async () => Promise.resolve());
    const explorer = screen.getByRole("button", { name: "Explorer" });
    fireEvent.click(explorer);
    expect(screen.queryByRole("complementary", { name: "Left panel" })).toBeNull();
    expect(explorer).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(explorer);
    fireEvent.click(await screen.findByRole("button", { name: "Hide left panel" }));
    expect(screen.queryByRole("complementary", { name: "Left panel" })).toBeNull();
    expect(explorer).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "Source Control" }));
    expect(await screen.findByRole("region", { name: "Source Control" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Hide left panel" }));
    expect(screen.queryByRole("complementary", { name: "Left panel" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(screen.getByRole("complementary", { name: "Left panel" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Search" })).toHaveAttribute("aria-pressed", "true");
  });

  it("renders the complete shell and supports at most four editor groups", async () => {
    const { client } = layoutClient();
    render(<WorkbenchShell client={client} editor={<p>editor remains available</p>} />);

    expect(screen.getByRole("navigation", { name: "Left activity rail" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Right activity rail" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Left panel" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Right panel" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Editor area" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Panel" })).toBeInTheDocument();
    expect(screen.getByRole("contentinfo", { name: "Status bar" })).toBeInTheDocument();

    const split = screen.getByRole("button", { name: "Split right" });
    fireEvent.click(split);
    fireEvent.click(split);
    fireEvent.click(split);
    expect(screen.getAllByRole("article")).toHaveLength(4);
    expect(split).toBeDisabled();
  });

  it("uses one frame token around separated card surfaces", async () => {
    const kernel = layoutClient();
    const { container } = render(<WorkbenchShell client={kernel.client} />);

    expect(screen.getByRole("banner", { name: "Title bar" })).toHaveClass(
      "workbench-frame-surface",
    );
    expect(screen.getByRole("navigation", { name: "Left activity rail" })).toHaveClass(
      "workbench-frame-surface",
    );
    expect(screen.getByRole("navigation", { name: "Right activity rail" })).toHaveClass(
      "workbench-frame-surface",
    );
    expect(screen.getByRole("contentinfo", { name: "Status bar" })).toHaveClass(
      "workbench-frame-surface",
    );
    expect(container.querySelector(".workbench-body")).toHaveClass("workbench-frame-surface");
    expect(container.querySelectorAll(".workbench-card")).toHaveLength(4);
    await waitFor(() => expect(kernel.reads()).toBe(1));
  });

  it("keeps both activity rails when either side panel is collapsed", () => {
    const { client } = layoutClient();
    render(<WorkbenchShell client={client} />);

    fireEvent.click(
      within(screen.getByRole("navigation", { name: "Left activity rail" })).getByRole("button", {
        name: "Hide left panel",
      }),
    );

    expect(screen.queryByRole("complementary", { name: "Left panel" })).not.toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Right panel" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Left activity rail" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Right activity rail" })).toBeInTheDocument();
  });

  it("keeps the physical activity rails fixed when side content is swapped", () => {
    const { client } = layoutClient();
    render(<WorkbenchShell client={client} />);

    const leftRail = screen.getByRole("navigation", { name: "Left activity rail" });
    const rightRail = screen.getByRole("navigation", { name: "Right activity rail" });
    expect(screen.getByRole("separator", { name: "Resize left panel" })).toHaveAttribute(
      "aria-valuenow",
      String(DEFAULT_LAYOUT.primarySidebarSize),
    );

    fireEvent.click(screen.getByRole("button", { name: "Swap left and right panels" }));

    expect(screen.getByRole("navigation", { name: "Left activity rail" })).toBe(leftRail);
    expect(screen.getByRole("navigation", { name: "Right activity rail" })).toBe(rightRail);
    expect(screen.getByRole("separator", { name: "Resize left panel" })).toHaveAttribute(
      "aria-valuenow",
      String(DEFAULT_LAYOUT.secondarySidebarSize),
    );
    expect(screen.getByRole("separator", { name: "Resize right panel" })).toHaveAttribute(
      "aria-valuenow",
      String(DEFAULT_LAYOUT.primarySidebarSize),
    );
  });

  it("reflects shared zen mode state without mutating the saved layout", () => {
    const { client } = layoutClient();
    render(<WorkbenchShell client={client} />);
    const before = layoutSnapshot();

    act(() => useLayoutStore.getState().toggleZenMode());

    expect(screen.getByTestId("workbench")).toHaveClass("workbench--zen");
    expect(layoutSnapshot()).toEqual(before);

    act(() => useLayoutStore.getState().toggleZenMode());
    expect(screen.getByTestId("workbench")).not.toHaveClass("workbench--zen");
    expect(layoutSnapshot()).toEqual(before);
  });

  it("restores layout and persists a constrained resize after two seconds", async () => {
    vi.useFakeTimers();
    const restored = { ...DEFAULT_LAYOUT, primarySidebarSize: 330 };
    const { client, writes } = layoutClient(restored);
    render(<WorkbenchShell client={client} />);

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    const handle = screen.getByRole("separator", { name: "Resize left panel" });
    expect(handle).toHaveAttribute("aria-valuenow", "330");
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(handle).toHaveAttribute("aria-valuenow", "340");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(writes).toHaveLength(1);
    expect(writes[0]?.primarySidebarSize).toBe(340);
  });

  it("persists named profiles through the kernel layout projection", async () => {
    vi.useFakeTimers();
    const kernel = layoutClient(DEFAULT_LAYOUT);
    render(<WorkbenchShell client={kernel.client} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    act(() => useLayoutStore.getState().saveProfile("Debug"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });

    expect(kernel.writes).toHaveLength(1);
    expect(kernel.writes[0]?.profiles).toEqual([expect.objectContaining({ name: "Debug" })]);
    expect(kernel.writes[0]?.activeProfile).toBe("Debug");
  });

  it("flushes a pending profile without cancelling the write on unmount", async () => {
    vi.useFakeTimers();
    const kernel = layoutClient(DEFAULT_LAYOUT);
    const { unmount } = render(<WorkbenchShell client={kernel.client} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    act(() => useLayoutStore.getState().saveProfile("Review"));
    unmount();
    await act(async () => Promise.resolve());

    expect(kernel.writes).toHaveLength(1);
    expect(kernel.writes[0]?.activeProfile).toBe("Review");
    expect(kernel.cancellations).toEqual([]);
  });

  it("isolates a crashing panel and reloads it without unmounting the editor", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    let shouldCrash = true;
    function CrashingPanel() {
      if (shouldCrash) throw new Error("forced panel crash");
      return <p>panel recovered</p>;
    }
    const { client } = layoutClient();
    render(
      <WorkbenchShell
        client={client}
        editor={<p>editor remains available</p>}
        panels={[{ id: "problems", label: "Problems", content: <CrashingPanel /> }]}
      />,
    );

    expect(await screen.findByText("Problems failed to load")).toBeInTheDocument();
    expect(screen.getByText("editor remains available")).toBeInTheDocument();
    shouldCrash = false;
    fireEvent.click(screen.getByRole("button", { name: "Reload panel" }));
    expect(await screen.findByText("panel recovered")).toBeInTheDocument();
    expect(screen.getByText("editor remains available")).toBeInTheDocument();
    expect(consoleError).toHaveBeenCalled();
  });

  it("resets corrupt kernel layout and surfaces a notice", async () => {
    const { client } = layoutClient({ primarySidebarSize: -1 } as WorkbenchLayout);
    render(<WorkbenchShell client={client} />);

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("reset to the default layout");
    });
    expect(useLayoutStore.getState().primarySidebarSize).toBe(DEFAULT_LAYOUT.primarySidebarSize);
  });

  it("uses the default layout when the initial kernel projection is unavailable", async () => {
    useLayoutStore.getState().setPrimarySidebarSize(420);
    useLayoutStore.getState().saveProfile("Stale");
    const client = new IpcClient({
      invoke: async () => {
        throw new Error("kernel unavailable");
      },
    });

    render(<WorkbenchShell client={client} />);

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("default layout is in use");
    });
    expect(useLayoutStore.getState().primarySidebarSize).toBe(DEFAULT_LAYOUT.primarySidebarSize);
    expect(useLayoutStore.getState().profiles).toEqual([]);
  });

  it("resets a corrupt persisted profile store with a notification", async () => {
    const corrupt = {
      ...DEFAULT_LAYOUT,
      profiles: [{ name: "Broken", layout: { panelSize: "huge" } }],
    } as unknown as WorkbenchLayout;
    const { client } = layoutClient(corrupt);
    render(<WorkbenchShell client={client} />);

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("reset to the default layout");
    });
    expect(useLayoutStore.getState().profiles).toEqual([]);
  });

  it("shows a dismissible one-time notice for unavailable profile views", () => {
    const { client } = layoutClient();
    render(
      <WorkbenchShell
        client={client}
        activities={[
          {
            id: "explorer",
            label: "Explorer",
            icon: "E",
            view: <p>available activity view</p>,
          },
        ]}
      />,
    );
    act(() => {
      useLayoutStore.getState().setActiveActivity("removed.activity");
      useLayoutStore.getState().saveProfile("Plugin layout");
      useLayoutStore.getState().switchProfile("Plugin layout", {
        activityIds: ["explorer"],
        panelIds: ["problems"],
      });
    });

    expect(screen.getByRole("status")).toHaveTextContent("removed.activity");
    expect(screen.queryByText("available activity view")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText(/removed\.activity/)).not.toBeInTheDocument();
  });

  it("keeps unavailable profile views empty instead of substituting the first registered view", () => {
    const { client } = layoutClient();
    act(() => {
      useLayoutStore.getState().setActiveActivity("removed.activity");
      useLayoutStore.getState().setActivePanel("removed.panel");
      useLayoutStore.getState().saveProfile("Plugin layout");
      useLayoutStore.getState().switchProfile("Plugin layout", {
        activityIds: ["explorer"],
        panelIds: ["problems"],
      });
    });

    render(
      <WorkbenchShell
        client={client}
        activities={[
          { id: "explorer", label: "Explorer", icon: "E", view: <p>available activity</p> },
        ]}
        panels={[{ id: "problems", label: "Problems", content: <p>available panel</p> }]}
      />,
    );

    expect(screen.queryByText("available activity")).not.toBeInTheDocument();
    expect(screen.queryByText("available panel")).not.toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Left panel" })).toBeInTheDocument();
  });

  it("leaves unavailable views empty when an active profile is restored at startup", async () => {
    const removedLayout = {
      ...DEFAULT_LAYOUT,
      activeActivity: "removed.activity",
      activePanel: "removed.panel",
    };
    const saved = {
      ...removedLayout,
      profiles: [{ name: "Old plugin", layout: removedLayout }],
      activeProfile: "Old plugin",
    };
    const { client } = layoutClient(saved);

    render(
      <WorkbenchShell
        client={client}
        activities={[
          { id: "explorer", label: "Explorer", icon: "E", view: <p>available activity</p> },
        ]}
        panels={[{ id: "problems", label: "Problems", content: <p>available panel</p> }]}
      />,
    );

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("removed.activity");
    });
    expect(useLayoutStore.getState().activeProfile).toBe("Old plugin");
    expect(useLayoutStore.getState().activeActivity).toBe("");
    expect(useLayoutStore.getState().activePanel).toBe("");
    expect(screen.queryByText("available activity")).not.toBeInTheDocument();
    expect(screen.queryByText("available panel")).not.toBeInTheDocument();
  });

  it("reconciles a changed kernel projection every thirty seconds", async () => {
    vi.useFakeTimers();
    const kernel = layoutClient(DEFAULT_LAYOUT);
    render(<WorkbenchShell client={kernel.client} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(kernel.reads()).toBe(1);

    kernel.replaceKernelLayout({ ...DEFAULT_LAYOUT, panelPosition: "right", panelSize: 310 });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });

    expect(kernel.reads()).toBe(2);
    expect(useLayoutStore.getState().panelPosition).toBe("right");
    expect(useLayoutStore.getState().panelSize).toBe(310);
    expect(kernel.writes).toHaveLength(0);
  });
});
