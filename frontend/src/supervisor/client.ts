import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { InvokeFn } from "../ipc";

export interface CrashCause {
  timestamp_ms: number;
  exit_code: number | null;
  signal: number | null;
  panic_message: string | null;
  missed_heartbeats: number;
  last_log_lines: string[];
}

export type SupervisorStatus =
  | { state: "starting"; safe_mode: boolean }
  | { state: "running"; safe_mode: boolean; epoch: string }
  | { state: "recovering"; attempt: number; safe_mode: boolean; cause: CrashCause }
  | { state: "recovery_required"; safe_mode: boolean; cause: CrashCause }
  | { state: "stopped" };

export type RecoveryAction = "retry" | "start_without_session_restore" | "open_logs";

export class SupervisorClient {
  private readonly invokeFn: InvokeFn;

  constructor(invoke: InvokeFn = tauriInvoke as InvokeFn) {
    this.invokeFn = invoke;
  }

  status(): Promise<SupervisorStatus> {
    return this.invokeFn<SupervisorStatus>("supervisor_status");
  }

  action(action: RecoveryAction): Promise<void> {
    return this.invokeFn<void>("supervisor_recovery_action", { action });
  }
}

export const supervisor = new SupervisorClient();
