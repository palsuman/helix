import {
  act,
  createEvent,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { createMockIpc } from "../test/mockIpc";
import type { FileEntry } from "../generated/FileEntry";
import type { ExplorerPageRequest } from "../generated/ExplorerPageRequest";
import { FileExplorer } from "./FileExplorer";
import { flattenTree, setExplorerDiagnostics, useExplorerStore, revealInExplorer } from "./model";
import { onEditorNavigation } from "../editor/navigation";

function entry(path: string, is_dir = false): FileEntry {
  return {
    path,
    name: path.split("/").pop()!,
    relative_path: path.slice(6),
    is_dir,
    is_symlink: false,
    size: 0n,
    modified_ms: null,
    readonly: false,
  };
}
function harness(files = [entry("/root/a.ts"), entry("/root/b.ts"), entry("/root/src", true)]) {
  const ipc = createMockIpc();
  ipc.respond("workspace.list", {
    workspaces: [{ roots: [{ path: "/root", name: "root", availability: "available" }] }],
  });
  ipc.handle<ExplorerPageRequest, unknown>("explorer.page", (request) => {
    const entries = request.filter
      ? [entry("/root/src", true), entry("/root/src/component.tsx")]
      : request.path === "/root"
        ? files
        : [entry("/root/src/component.tsx")];
    return {
      entries: entries.slice(request.offset, request.offset + 500),
      total: entries.length,
      snapshot: "snapshot",
      unreadable_paths: [],
    };
  });
  ipc.respond("explorer.mutate", {});
  return ipc;
}
beforeEach(() => useExplorerStore.setState({ reveal: null, diagnostics: {}, activeFiles: {} }));

describe("file explorer", () => {
  it("renders a bounded viewport over 100,000 files, including after scrolling", async () => {
    const entries = Array.from({ length: 100_000 }, (_, i) => entry(`/root/file-${i}.ts`));
    expect(
      flattenTree([entry("/root", true)], new Map([["/root", entries]]), new Set(["/root"])),
    ).toHaveLength(100_001);
    const ipc = harness(entries);
    render(<FileExplorer client={ipc.client} />);
    await screen.findByText("file-0.ts");
    expect(screen.getAllByRole("treeitem").length).toBeLessThan(40);
    const tree = screen.getByRole("tree");
    fireEvent.scroll(tree, { target: { scrollTop: 2_799_000 } });
    expect(screen.getAllByRole("treeitem").length).toBeLessThan(40);
    expect(screen.queryByText("file-0.ts")).toBeNull();
    expect(ipc.requests.filter((r) => r.command === "explorer.page")).toHaveLength(200);
  });

  it("creates files and folders, renames inline, and confirms multi-selection deletion", async () => {
    const ipc = harness();
    render(<FileExplorer client={ipc.client} />);
    await screen.findByText("a.ts");
    fireEvent.click(screen.getByRole("button", { name: "New file" }));
    fireEvent.change(screen.getByRole("textbox", { name: "File or folder name" }), {
      target: { value: "new.ts" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() =>
      expect(
        ipc.requests.some(
          (r) =>
            r.command === "explorer.mutate" &&
            (r.payload as { destination: string }).destination === "/root/new.ts",
        ),
      ).toBe(true),
    );
    await waitFor(() =>
      expect(screen.queryByRole("textbox", { name: "File or folder name" })).toBeNull(),
    );
    await screen.findByText("a.ts");
    fireEvent.click(screen.getByText("a.ts"));
    fireEvent.keyDown(screen.getByRole("tree"), { key: "F2" });
    const input = screen.getByRole("textbox", { name: "File or folder name" });
    expect(input.closest('[role="treeitem"]')).not.toBeNull();
    fireEvent.change(input, { target: { value: "renamed.ts" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() =>
      expect(
        ipc.requests.some((r) => (r.payload as { operation?: string }).operation === "rename"),
      ).toBe(true),
    );
    await waitFor(() =>
      expect(screen.queryByRole("textbox", { name: "File or folder name" })).toBeNull(),
    );
    fireEvent.click(screen.getByText("a.ts"));
    fireEvent.click(screen.getByText("b.ts"), { ctrlKey: true });
    fireEvent.keyDown(screen.getByRole("tree"), { key: "Delete" });
    const dialog = screen.getByRole("alertdialog");
    expect(
      ipc.requests.filter((r) => (r.payload as { operation?: string }).operation === "delete"),
    ).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() =>
      expect(
        ipc.requests.find((r) => (r.payload as { operation?: string }).operation === "delete")
          ?.payload,
      ).toMatchObject({ paths: ["/root/a.ts", "/root/b.ts"] }),
    );
  });

  it("filters matching subtrees, decorates without Git, and reveals editor files", async () => {
    const ipc = harness();
    setExplorerDiagnostics({ "/root/a.ts": 3 });
    render(<FileExplorer client={ipc.client} />);
    await screen.findByText("a.ts");
    expect(screen.getByLabelText("3 diagnostics")).toBeInTheDocument();
    let opened = "";
    const unsubscribe = onEditorNavigation((r) => {
      opened = r.path ?? "";
    });
    fireEvent.doubleClick(screen.getByText("a.ts"));
    expect(opened).toBe("/root/a.ts");
    unsubscribe();
    fireEvent.change(screen.getByRole("textbox", { name: "Filter files and folders" }), {
      target: { value: "component" },
    });
    await screen.findByText("component.tsx");
    expect(screen.queryByText("a.ts")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Collapse all" }));
    expect(screen.queryByText("component.tsx")).toBeNull();
    act(() => revealInExplorer("/root/src/component.tsx"));
    await waitFor(() =>
      expect(screen.getByText("component.tsx").closest('[role="treeitem"]')).toHaveAttribute(
        "aria-selected",
        "true",
      ),
    );
  });

  it("moves multiple dragged files and copies with a modifier", async () => {
    const ipc = harness();
    render(<FileExplorer client={ipc.client} />);
    await screen.findByText("a.ts");
    fireEvent.click(screen.getByText("a.ts"));
    fireEvent.click(screen.getByText("b.ts"), { shiftKey: true });
    const dataTransfer = { effectAllowed: "", dropEffect: "", setData: () => {} };
    fireEvent.dragStart(screen.getByText("a.ts"), { dataTransfer });
    const drop = createEvent.drop(screen.getByText("src"), { dataTransfer });
    Object.defineProperty(drop, "altKey", { value: true });
    fireEvent(screen.getByText("src"), drop);
    await waitFor(() =>
      expect(ipc.requests.find((r) => r.command === "explorer.mutate")?.payload).toMatchObject({
        operation: "copy",
        paths: ["/root/a.ts", "/root/b.ts"],
        destination: "/root/src",
      }),
    );
    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
    fireEvent.dragStart(screen.getByText("a.ts"), { dataTransfer });
    fireEvent.drop(screen.getByText("src"), { dataTransfer });
    await waitFor(() =>
      expect(
        ipc.requests.some((r) => (r.payload as { operation?: string }).operation === "move"),
      ).toBe(true),
    );
  });
});
