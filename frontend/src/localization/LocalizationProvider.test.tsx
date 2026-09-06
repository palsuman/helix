import { act, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { IpcRequest } from "../generated/IpcRequest";
import { IpcClient, type InvokeFn } from "../ipc";
import type { StreamClient } from "../stream";
import { useNotificationStore } from "../notifications";
import { LocalizationProvider } from "./LocalizationProvider";
import { useLocalizationSnapshot } from "./hooks";
import { LocalizationService, PSEUDO_LOCALE } from "./service";

function Probe() {
  const { intl } = useLocalizationSnapshot();
  return (
    <p>
      {intl.formatMessage(
        { id: "localization.welcome", defaultMessage: "Welcome, {name}" },
        { name: "Ada" },
      )}
    </p>
  );
}

describe("LocalizationProvider", () => {
  it("loads helix.locale, applies document direction, and announces restart-required changes", async () => {
    useNotificationStore.getState().reset();
    const requests: IpcRequest<unknown>[] = [];
    const invoke: InvokeFn = async <T,>(_endpoint: string, args?: Record<string, unknown>) => {
      const request = (args as { request: IpcRequest<unknown> }).request;
      requests.push(request);
      return {
        correlation_id: request.correlation_id,
        result: {
          setting: {
            key: "helix.locale",
            value: "ar",
            scope: "user",
            language: null,
            requires_restart: true,
          },
        },
        error: null,
      } as T;
    };
    let changed: ((payload: { changed_keys: string[] }) => void) | undefined;
    const stream = {
      subscribe: (_channel: string, listener: typeof changed) => {
        changed = listener;
        return () => {};
      },
    } as unknown as StreamClient;
    const service = new LocalizationService();

    render(
      <LocalizationProvider client={new IpcClient({ invoke })} stream={stream} service={service}>
        <Probe />
      </LocalizationProvider>,
    );
    await act(async () => Promise.resolve());

    expect(requests[0]?.payload).toMatchObject({ key: "helix.locale" });
    expect(document.documentElement).toHaveAttribute("lang", "ar");
    expect(document.documentElement).toHaveAttribute("dir", "rtl");
    expect(screen.getByText("مرحبًا، Ada")).toBeInTheDocument();

    act(() => changed?.({ changed_keys: ["helix.locale"] }));
    expect(useNotificationStore.getState().entries.at(-1)?.message).toContain("إعادة تشغيل");

    act(() => service.activate(PSEUDO_LOCALE));
    expect(screen.getByText(/^［.+Ada］$/)).toBeInTheDocument();
  });
});
