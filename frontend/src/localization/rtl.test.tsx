import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { IpcRequest } from "../generated/IpcRequest";
import { IpcClient, type InvokeFn } from "../ipc";
import { Icon } from "../icons";
import { WorkbenchShell } from "../workbench";
import { LocalizationContext } from "./context";
import { LocalizationService } from "./service";

describe("RTL plumbing", () => {
  it("applies Arabic direction and automatically mirrors directional icons", () => {
    const service = new LocalizationService();
    service.activate("ar");
    const invoke: InvokeFn = async <T,>(_endpoint: string, args?: Record<string, unknown>) => {
      const request = (args as { request: IpcRequest<unknown> }).request;
      return {
        correlation_id: request.correlation_id,
        result: { layout: null },
        error: null,
      } as T;
    };
    render(
      <LocalizationContext.Provider value={service}>
        <WorkbenchShell
          client={new IpcClient({ invoke })}
          rightActivityRail={<Icon id="panel-side" label="directional" />}
        />
      </LocalizationContext.Provider>,
    );

    expect(document.documentElement).toHaveAttribute("dir", "rtl");
    expect(screen.getByRole("img", { name: "directional" })).toHaveClass("icon-rtl-flip");
    expect(screen.getByRole("navigation", { name: "Right activity rail" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Right panel" })).toBeInTheDocument();
  });
});
