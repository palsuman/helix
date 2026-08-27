import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { IpcRequest } from "./generated/IpcRequest";
import { IpcClient, type InvokeFn } from "./ipc";
import App from "./App";

function shellClient() {
  const commands: string[] = [];
  const requests: IpcRequest<unknown>[] = [];
  const invoke: InvokeFn = async <T,>(_endpoint: string, args?: Record<string, unknown>) => {
    const request = (args as { request: IpcRequest<unknown> }).request;
    commands.push(request.command);
    requests.push(request);
    const result = request.command === "window.layout.get" ? { layout: null } : {};
    return { correlation_id: request.correlation_id, result, error: null } as T;
  };
  return { client: new IpcClient({ invoke }), commands, requests };
}

describe("App", () => {
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
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
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

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]?.window_id).toBe("w-secondary");
    expect(requests[0]?.payload).toEqual({ id: "w-secondary" });
  });
});
