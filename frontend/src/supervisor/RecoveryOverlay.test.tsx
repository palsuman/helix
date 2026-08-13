import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { InvokeFn } from "../ipc";
import { SupervisorClient } from "./client";
import { RecoveryOverlay } from "./RecoveryOverlay";

describe("RecoveryOverlay", () => {
  it("offers every storm-recovery action and forwards the selection", async () => {
    const actions: string[] = [];
    const invoke: InvokeFn = async <T,>(command: string, args?: Record<string, unknown>) => {
      if (command === "supervisor_status") {
        return {
          state: "recovery_required",
          safe_mode: true,
          cause: {
            timestamp_ms: 1,
            exit_code: 101,
            signal: null,
            panic_message: "kernel panicked",
            missed_heartbeats: 0,
            last_log_lines: [],
          },
        } as T;
      }
      actions.push(args?.action as string);
      return undefined as T;
    };
    render(<RecoveryOverlay client={new SupervisorClient(invoke)} />);

    expect(await screen.findByRole("alertdialog")).toHaveTextContent("Safe mode is enabled");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    fireEvent.click(screen.getByRole("button", { name: "Start without session restore" }));
    fireEvent.click(screen.getByRole("button", { name: "Open logs" }));
    await waitFor(() => expect(actions).toEqual([
      "retry",
      "start_without_session_restore",
      "open_logs",
    ]));
  });

  it("stays absent while the kernel is running", async () => {
    const invoke: InvokeFn = async <T,>() =>
      ({ state: "running", safe_mode: false, epoch: "one" }) as T;
    render(<RecoveryOverlay client={new SupervisorClient(invoke)} />);
    await waitFor(() => expect(invoke).toBeDefined());
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });
});
