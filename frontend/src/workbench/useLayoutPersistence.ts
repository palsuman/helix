import { useEffect, useState } from "react";
import type { IpcClient } from "../ipc";
import type { MessageKey } from "../localization/messages";
import { getWindowLayout, setWindowLayout } from "../windows";
import {
  DEFAULT_LAYOUT,
  layoutSnapshot,
  parseLayout,
  useLayoutStore,
  type WorkbenchLayout,
} from "./layoutStore";

export type LayoutPersistenceStatus =
  | { kind: "loading" }
  | { kind: "ready" }
  | {
      kind: "unavailable" | "reset";
      message: MessageKey;
      values?: Record<string, string | number | Date>;
    };

const SAVE_DEBOUNCE_MS = 2_000;
export const RECONCILE_INTERVAL_MS = 30_000;

const isEmptyLayout = (value: unknown) =>
  value === null ||
  (typeof value === "object" && value !== null && Object.keys(value).length === 0);

const hashLayout = (layout: unknown) => JSON.stringify(layout);

/** Restore per-window layout from the kernel, then debounce UI mutations back. */
export function useLayoutPersistence(
  client: IpcClient,
  windowId = "main",
): LayoutPersistenceStatus {
  const [status, setStatus] = useState<LayoutPersistenceStatus>({ kind: "loading" });

  useEffect(() => {
    const controller = new AbortController();
    let hydrated = false;
    let dirty = false;
    let applyingProjection = false;
    let projectionHash: string | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let reconcileTimer: ReturnType<typeof setInterval> | undefined;
    const options = { signal: controller.signal, windowId };

    const restore = (layout: WorkbenchLayout) => {
      applyingProjection = true;
      useLayoutStore.getState().restore(layout);
      applyingProjection = false;
    };

    const persist = (lifecycleFlush = false) => {
      if (!hydrated || !dirty) return;
      dirty = false;
      const snapshot = layoutSnapshot();
      const persistOptions = lifecycleFlush ? { windowId } : options;
      void setWindowLayout(client, windowId, snapshot, persistOptions).then(
        (response) => {
          if (!controller.signal.aborted) {
            projectionHash = hashLayout(response.layout);
            setStatus({ kind: "ready" });
          }
        },
        (error: unknown) => {
          if (controller.signal.aborted) return;
          dirty = true;
          setStatus({
            kind: "unavailable",
            message: "workbenchLayoutSaveFailed",
            values: { error: String(error) },
          });
        },
      );
    };

    const reconcile = async () => {
      if (!hydrated || dirty) return;
      try {
        const response = await getWindowLayout(client, windowId, options);
        if (controller.signal.aborted || hashLayout(response.layout) === projectionHash) return;
        const reconciled = parseLayout(response.layout);
        if (reconciled === null && !isEmptyLayout(response.layout)) {
          restore(DEFAULT_LAYOUT);
          dirty = true;
          setStatus({
            kind: "reset",
            message: "workbenchLayoutInvalid",
          });
          if (timer !== undefined) clearTimeout(timer);
          timer = setTimeout(persist, SAVE_DEBOUNCE_MS);
          return;
        }
        restore(reconciled ?? DEFAULT_LAYOUT);
        projectionHash = hashLayout(response.layout);
        setStatus({ kind: "ready" });
      } catch (error: unknown) {
        if (controller.signal.aborted) return;
        setStatus({
          kind: "unavailable",
          message: "workbenchLayoutReconcileFailed",
          values: { error: String(error) },
        });
      }
    };

    const unsubscribe = useLayoutStore.subscribe(() => {
      if (!hydrated || applyingProjection) return;
      dirty = true;
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(persist, SAVE_DEBOUNCE_MS);
    });

    const flushBeforeUnload = () => persist(true);
    window.addEventListener("beforeunload", flushBeforeUnload);

    void getWindowLayout(client, windowId, options)
      .then((response) => {
        if (controller.signal.aborted) return;
        projectionHash = hashLayout(response.layout);
        if (isEmptyLayout(response.layout)) {
          restore(DEFAULT_LAYOUT);
          hydrated = true;
          setStatus({ kind: "ready" });
          reconcileTimer = setInterval(() => void reconcile(), RECONCILE_INTERVAL_MS);
          return;
        }
        const restored = parseLayout(response.layout);
        if (restored === null) {
          restore(DEFAULT_LAYOUT);
          hydrated = true;
          dirty = true;
          setStatus({
            kind: "reset",
            message: "workbenchLayoutInvalid",
          });
          timer = setTimeout(persist, SAVE_DEBOUNCE_MS);
          reconcileTimer = setInterval(() => void reconcile(), RECONCILE_INTERVAL_MS);
          return;
        }
        restore(restored);
        hydrated = true;
        setStatus({ kind: "ready" });
        reconcileTimer = setInterval(() => void reconcile(), RECONCILE_INTERVAL_MS);
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        restore(DEFAULT_LAYOUT);
        hydrated = true;
        setStatus({
          kind: "unavailable",
          message: "workbenchLayoutUnavailable",
          values: { error: String(error) },
        });
      });

    return () => {
      unsubscribe();
      if (timer !== undefined) clearTimeout(timer);
      if (reconcileTimer !== undefined) clearInterval(reconcileTimer);
      window.removeEventListener("beforeunload", flushBeforeUnload);
      if (dirty) persist(true);
      controller.abort();
    };
  }, [client, windowId]);

  return status;
}
