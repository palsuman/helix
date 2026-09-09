import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { IpcRequest } from "../generated/IpcRequest";
import { IpcClient, type InvokeFn } from "../ipc";
import { CommandRegistry } from "../commands";
import { QuickOpen } from "./QuickOpen";

function harness() {
  const pathQueries: string[] = [];
  const cancelled: string[] = [];
  let releaseSlow: (() => void) | undefined;
  const invoke: InvokeFn = async <T,>(endpoint: string, args?: Record<string, unknown>) => {
    if (endpoint === "ipc_cancel") {
      cancelled.push((args as { request: { correlation_id: string } }).request.correlation_id);
      return { cancelled: true } as T;
    }
    const request = (args as { request: IpcRequest<unknown> }).request;
    let result: unknown;
    if (request.command === "command.execute") {
      result = {
        id: (request.payload as { id: string }).id,
        target: { kind: "renderer" },
        arguments: null,
      };
    } else if (request.command === "workspace.list") {
      result = {
        workspaces: [
          {
            roots: [
              { path: "/workspace", name: "workspace", availability: "available", primary: true },
            ],
          },
        ],
      };
    } else if (request.command === "search.quickOpen") {
      const query = (request.payload as { query: string }).query;
      pathQueries.push(query);
      if (query === "slow") await new Promise<void>((resolve) => (releaseSlow = resolve));
      result = {
        indexing: query === "",
        matches:
          query === "missing"
            ? []
            : [
                {
                  path: "/workspace/src/main.ts",
                  relative_path: "src/main.ts",
                  file_name: "main.ts",
                  score: 3_000_000n,
                },
              ],
      };
    } else if (request.command === "command.list") {
      result = { commands: [] };
    } else {
      throw new Error(`Unexpected command ${request.command}`);
    }
    return { correlation_id: request.correlation_id, result, error: null } as T;
  };
  const client = new IpcClient({ invoke });
  return {
    client,
    registry: new CommandRegistry(client, "main"),
    pathQueries,
    cancelled,
    releaseSlow: () => releaseSlow?.(),
  };
}

describe("QuickOpen", () => {
  it("queries every workspace root and opens the selected file in a split", async () => {
    const { client, registry, pathQueries } = harness();
    const onOpen = vi.fn();
    render(<QuickOpen client={client} registry={registry} onOpen={onOpen} onLine={vi.fn()} />);
    await act(async () => registry.execute("workbench.action.quickOpen"));
    const input = await screen.findByRole("combobox", { name: "Search files and symbols" });
    fireEvent.change(input, { target: { value: "main" } });
    expect(await screen.findByRole("option", { name: /main\.ts/ })).toHaveTextContent(
      "src/main.ts",
    );
    fireEvent.keyDown(input, { key: "Enter", ctrlKey: true });
    expect(onOpen).toHaveBeenCalledWith("/workspace/src/main.ts", true);
    expect(pathQueries).toContain("main");
  });

  it("switches modes, reports an unavailable symbol provider, and navigates to a line", async () => {
    const { client, registry } = harness();
    const onLine = vi.fn();
    render(<QuickOpen client={client} registry={registry} onOpen={vi.fn()} onLine={onLine} />);
    await act(async () => registry.execute("workbench.action.quickOpen"));
    const input = await screen.findByRole("combobox");
    fireEvent.change(input, { target: { value: "@render" } });
    expect(
      await screen.findByText("No symbol provider is available for the active language."),
    ).toBeInTheDocument();
    fireEvent.change(input, { target: { value: ":42" } });
    await waitFor(() => expect(screen.getByText("Go to line 42")).toBeInTheDocument());
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onLine).toHaveBeenCalledWith(42);
  });

  it("cancels the previous path query on every keystroke", async () => {
    const { client, registry, pathQueries, cancelled, releaseSlow } = harness();
    render(<QuickOpen client={client} registry={registry} onOpen={vi.fn()} onLine={vi.fn()} />);
    await act(async () => registry.execute("workbench.action.quickOpen"));
    const input = await screen.findByRole("combobox");
    fireEvent.change(input, { target: { value: "slow" } });
    await waitFor(() => expect(pathQueries).toContain("slow"));
    fireEvent.change(input, { target: { value: "main" } });
    await waitFor(() => expect(cancelled.length).toBeGreaterThan(0));
    releaseSlow();
  });
});
