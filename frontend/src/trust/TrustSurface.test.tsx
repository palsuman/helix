import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { TrustStatusResponse } from "../generated/TrustStatusResponse";
import type { IpcRequest } from "../generated/IpcRequest";
import { IpcClient, type InvokeFn } from "../ipc";
import type { StreamClient } from "../stream";
import { TrustBanner } from "./TrustBanner";
import { TrustManager } from "./TrustManager";
import { TrustPrompt } from "./TrustPrompt";
import { TrustStatusBar } from "./TrustStatusBar";
import { TrustSurface } from "./TrustSurface";
import type { TrustClient } from "./client";

function restrictedStatus(overrides: Partial<TrustStatusResponse> = {}): TrustStatusResponse {
  return {
    enabled: true,
    trust_everything: false,
    store_healthy: true,
    workspace_mode: "restricted",
    roots: [{ path: "/tmp/repo", decision: "restricted", inherited_from: null }],
    pending_prompts: [],
    ...overrides,
  };
}

describe("TrustPrompt", () => {
  it("asks to trust or restrict an unfamiliar folder", () => {
    const trusted: string[] = [];
    render(
      <TrustPrompt
        status={restrictedStatus({ pending_prompts: ["/tmp/repo"] })}
        onTrust={(path) => trusted.push(path)}
        onRestrict={() => undefined}
      />,
    );

    expect(screen.getByRole("dialog")).toHaveTextContent("/tmp/repo");
    fireEvent.click(screen.getByRole("button", { name: "Trust folder" }));
    expect(trusted).toEqual(["/tmp/repo"]);
  });
});

describe("TrustBanner", () => {
  it("offers one-click trust while restricted", () => {
    const trusted: string[] = [];
    render(<TrustBanner status={restrictedStatus()} onTrust={(path) => trusted.push(path)} />);

    fireEvent.click(screen.getByRole("button", { name: "Trust folder" }));
    expect(trusted).toEqual(["/tmp/repo"]);
  });

  it("can be dismissed", () => {
    render(<TrustBanner status={restrictedStatus()} onTrust={() => undefined} />);

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });
});

describe("TrustStatusBar", () => {
  it("shows restricted mode in the status bar", () => {
    render(<TrustStatusBar status={restrictedStatus()} />);
    expect(screen.getByRole("status")).toHaveTextContent("Restricted mode");
  });
});

describe("TrustManager", () => {
  it("requires an explicit warning acknowledgement before trusting everything", async () => {
    const setTrustEverything = vi.fn().mockResolvedValue({ enabled: true });
    const client = {
      list: vi.fn().mockResolvedValue({ entries: [] }),
      status: vi.fn().mockResolvedValue(restrictedStatus({ roots: [], pending_prompts: [] })),
      setTrustEverything,
    } as unknown as TrustClient;
    render(<TrustManager client={client} />);

    fireEvent.click(await screen.findByRole("button", { name: "Trust all folders…" }));
    const confirm = screen.getByRole("button", { name: "Trust every folder" });
    expect(confirm).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("launch processes");

    fireEvent.click(screen.getByRole("checkbox", { name: "I understand the security risk" }));
    fireEvent.click(confirm);
    await waitFor(() => expect(setTrustEverything).toHaveBeenCalledWith(true, true));
  });
});

describe("TrustSurface", () => {
  it("derives prompt paths from open workspaces instead of a demo folder", async () => {
    const invoke: InvokeFn = async <T,>(_endpoint: string, args?: Record<string, unknown>) => {
      const request = (args as { request: IpcRequest<unknown> }).request;
      const result = (() => {
        if (request.command === "workspace.list") {
          return { workspaces: [{ roots: [{ path: "/actual/open/repo" }] }] };
        }
        if (request.command === "trust.list") return { entries: [] };
        if (request.command === "trust.status") {
          const paths = (request.payload as { paths: string[] }).paths;
          return restrictedStatus({
            roots: paths.map((path) => ({
              path,
              decision: "restricted" as const,
              inherited_from: null,
            })),
            pending_prompts: paths,
          });
        }
        throw new Error(`unexpected command ${request.command}`);
      })();
      return {
        correlation_id: request.correlation_id,
        result,
        error: null,
      } as T;
    };
    const streamClient = {
      subscribe: vi.fn(() => () => undefined),
    } as unknown as StreamClient;

    render(<TrustSurface client={new IpcClient({ invoke })} streamClient={streamClient} />);

    expect(await screen.findByRole("dialog")).toHaveTextContent("/actual/open/repo");
    expect(screen.queryByText("/tmp/helix-demo-repo")).not.toBeInTheDocument();
  });
});
