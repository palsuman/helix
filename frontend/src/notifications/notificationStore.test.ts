import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  INFO_DISMISS_MS,
  NOTIFICATION_HISTORY_LIMIT,
  WARNING_DISMISS_MS,
  notify,
  useNotificationStore,
} from "./notificationStore";

describe("notification store", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    useNotificationStore.getState().reset();
  });

  afterEach(() => {
    useNotificationStore.getState().reset();
    vi.useRealTimers();
  });

  it("applies the lifecycle for each notification kind", () => {
    const infoId = notify({ kind: "info", message: "Indexed", source: "Search" });
    const warningId = notify({ kind: "warning", message: "Slow", source: "File watcher" });
    const errorId = notify({ kind: "error", message: "Failed", source: "Kernel" });
    const progressId = notify({ kind: "progress", message: "Building", source: "Tasks" });

    vi.advanceTimersByTime(INFO_DISMISS_MS);
    expect(
      useNotificationStore.getState().entries.find((entry) => entry.id === infoId)?.toastVisible,
    ).toBe(false);
    expect(
      useNotificationStore.getState().entries.find((entry) => entry.id === warningId)?.toastVisible,
    ).toBe(true);

    vi.advanceTimersByTime(WARNING_DISMISS_MS - INFO_DISMISS_MS);
    expect(
      useNotificationStore.getState().entries.find((entry) => entry.id === warningId)?.toastVisible,
    ).toBe(false);
    expect(
      useNotificationStore.getState().entries.find((entry) => entry.id === errorId)?.toastVisible,
    ).toBe(true);
    expect(
      useNotificationStore.getState().entries.find((entry) => entry.id === progressId)
        ?.toastVisible,
    ).toBe(true);
  });

  it("limits actions, clamps progress, and dispatches cancellation", async () => {
    const calls: string[] = [];
    const id = notify({
      kind: "progress",
      message: "Downloading",
      source: "Extensions",
      progress: 120,
      actions: [
        { label: "One", run: () => void calls.push("one") },
        { label: "Two", run: () => void calls.push("two") },
        { label: "Three", run: () => void calls.push("three") },
        { label: "Four", run: () => void calls.push("four") },
      ],
      cancel: () => void calls.push("cancel"),
    });
    const entry = useNotificationStore.getState().entries[0]!;

    expect(entry.actions).toHaveLength(3);
    expect(entry.progress).toBe(100);
    await entry.actions[0]!.run();
    await entry.cancel?.();
    expect(calls).toEqual(["one", "cancel"]);

    useNotificationStore.getState().updateProgress(id, -10);
    expect(useNotificationStore.getState().entries[0]?.progress).toBe(0);
    useNotificationStore.getState().updateProgress(id, null);
    expect(useNotificationStore.getState().entries[0]?.progress).toBeNull();
  });

  it("suppresses toasts during do-not-disturb while retaining capped history", () => {
    useNotificationStore.getState().setDoNotDisturb(true);
    for (let index = 0; index < NOTIFICATION_HISTORY_LIMIT + 2; index += 1) {
      notify({ kind: "info", message: `Notice ${index}`, source: "Tests" });
    }

    const state = useNotificationStore.getState();
    expect(state.entries).toHaveLength(NOTIFICATION_HISTORY_LIMIT);
    expect(state.entries.every((entry) => !entry.toastVisible)).toBe(true);
    expect(state.entries[0]?.message).toBe("Notice 2");
  });

  it("clears auto-dismiss timers when old history entries are evicted", () => {
    for (let index = 0; index < NOTIFICATION_HISTORY_LIMIT + 2; index += 1) {
      notify({ kind: "info", message: `Notice ${index}`, source: "Tests" });
    }

    expect(useNotificationStore.getState().entries).toHaveLength(NOTIFICATION_HISTORY_LIMIT);
    expect(vi.getTimerCount()).toBe(NOTIFICATION_HISTORY_LIMIT);
  });
});
