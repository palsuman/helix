import { create } from "zustand";

export const NOTIFICATION_HISTORY_LIMIT = 500;
export const INFO_DISMISS_MS = 5_000;
export const WARNING_DISMISS_MS = 10_000;

export type NotificationKind = "info" | "warning" | "error" | "progress";

export interface NotificationAction {
  label: string;
  run: () => void | Promise<void>;
}

export interface NotificationInput {
  kind: NotificationKind;
  message: string;
  source: string;
  actions?: readonly NotificationAction[];
  progress?: number | null;
  cancel?: () => void | Promise<void>;
}

export interface NotificationEntry extends NotificationInput {
  id: string;
  createdAt: number;
  actions: readonly NotificationAction[];
  toastVisible: boolean;
}

interface NotificationState {
  entries: NotificationEntry[];
  doNotDisturb: boolean;
  push: (input: NotificationInput) => string;
  updateProgress: (id: string, progress: number | null) => void;
  dismissToast: (id: string) => void;
  dismissAllToasts: () => void;
  remove: (id: string) => void;
  clear: () => void;
  setDoNotDisturb: (enabled: boolean) => void;
  reset: () => void;
}

let nextNotificationId = 1;
const dismissTimers = new Map<string, ReturnType<typeof setTimeout>>();

function autoDismissDelay(kind: NotificationKind): number | null {
  if (kind === "info") return INFO_DISMISS_MS;
  if (kind === "warning") return WARNING_DISMISS_MS;
  return null;
}

function clearDismissTimer(id: string) {
  const timer = dismissTimers.get(id);
  if (timer !== undefined) clearTimeout(timer);
  dismissTimers.delete(id);
}

function clearDismissTimers() {
  for (const timer of dismissTimers.values()) clearTimeout(timer);
  dismissTimers.clear();
}

function normalizedProgress(progress: number | null | undefined): number | null | undefined {
  if (progress === null || progress === undefined) return progress;
  return Math.min(100, Math.max(0, progress));
}

export const useNotificationStore = create<NotificationState>()((set, get) => ({
  entries: [],
  doNotDisturb: false,
  push: (input) => {
    const id = `notification-${nextNotificationId++}`;
    const entry: NotificationEntry = {
      ...input,
      id,
      createdAt: Date.now(),
      actions: input.actions?.slice(0, 3) ?? [],
      progress: normalizedProgress(input.progress),
      toastVisible: !get().doNotDisturb,
    };
    const currentEntries = get().entries;
    const nextEntries = [...currentEntries, entry].slice(-NOTIFICATION_HISTORY_LIMIT);
    const retainedIds = new Set(nextEntries.map((candidate) => candidate.id));
    for (const existing of currentEntries) {
      if (!retainedIds.has(existing.id)) clearDismissTimer(existing.id);
    }

    set({ entries: nextEntries });

    const delay = autoDismissDelay(input.kind);
    if (entry.toastVisible && delay !== null) {
      dismissTimers.set(
        id,
        setTimeout(() => {
          dismissTimers.delete(id);
          get().dismissToast(id);
        }, delay),
      );
    }
    return id;
  },
  updateProgress: (id, progress) =>
    set((state) => ({
      entries: state.entries.map((entry) =>
        entry.id === id && entry.kind === "progress"
          ? { ...entry, progress: normalizedProgress(progress) }
          : entry,
      ),
    })),
  dismissToast: (id) => {
    clearDismissTimer(id);
    set((state) => ({
      entries: state.entries.map((entry) =>
        entry.id === id ? { ...entry, toastVisible: false } : entry,
      ),
    }));
  },
  dismissAllToasts: () => {
    clearDismissTimers();
    set((state) => ({
      entries: state.entries.map((entry) => ({ ...entry, toastVisible: false })),
    }));
  },
  remove: (id) => {
    clearDismissTimer(id);
    set((state) => ({ entries: state.entries.filter((entry) => entry.id !== id) }));
  },
  clear: () => {
    clearDismissTimers();
    set({ entries: [] });
  },
  setDoNotDisturb: (enabled) => {
    if (enabled) get().dismissAllToasts();
    set({ doNotDisturb: enabled });
  },
  reset: () => {
    clearDismissTimers();
    nextNotificationId = 1;
    set({ entries: [], doNotDisturb: false });
  },
}));

export function notify(input: NotificationInput): string {
  return useNotificationStore.getState().push(input);
}
