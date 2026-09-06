import { useCallback, useState } from "react";
import type { LogLevel } from "../generated/LogLevel";
import type { LogRecord } from "../generated/LogRecord";
import { type IpcClient, ipc, isIpcError } from "../ipc";
import { useMessage } from "../localization";
import { type StreamClient, stream } from "../stream";
import { LOG_LEVELS, exportLogs } from "./commands";
import { formatRecord } from "./filter";
import { DEFAULT_MAX_ENTRIES, useLogViewer } from "./useLogViewer";

/**
 * The log viewer panel (Task 1.5, REQ-OBS-001.4, .5).
 *
 * Filter by level, source, and time range; full-text search; follow-tail;
 * copy an entry; export the filtered set. Kernel and frontend records appear
 * in one list because they are one list — the kernel's ring buffer holds both
 * (REQ-OBS-001.3).
 *
 * Notes on two choices:
 *
 * - The entry list is a `<ul>` with `role="log"` and `aria-live="off"`. A
 *   live region that announced every record would make a debug-level stream
 *   unusable with a screen reader, and the panel is a thing users read
 *   deliberately rather than a notification surface.
 * - The time range is interpreted as UTC and says so, because the timestamps
 *   in the list are UTC. Quietly applying the machine's offset would make the
 *   filter disagree with the column it filters.
 *
 * Styling is inline until the theming system lands in Task 2.4, matching the
 * rest of the pre-workbench surface.
 */

const LEVEL_COLORS: Record<LogLevel, string> = {
  trace: "#9ca3af",
  debug: "#93c5fd",
  info: "#e5e7eb",
  warn: "#fbbf24",
  error: "#f87171",
};

export interface LogViewerProps {
  client?: IpcClient;
  streamClient?: StreamClient;
  /** Records rendered at once. */
  maxEntries?: number;
  /** Injectable for tests; defaults to the platform clipboard. */
  clipboard?: { writeText: (text: string) => Promise<void> };
  /**
   * Receives the exported set. Defaults to a browser download, which is
   * replaced by the kernel's save dialog when the file system service lands
   * in Task 1.7.
   */
  onExport?: (fileName: string, content: string) => void;
}

function downloadFile(fileName: string, content: string): void {
  const url = URL.createObjectURL(new Blob([content], { type: "application/x-ndjson" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = fileName;
  anchor.click();
  URL.revokeObjectURL(url);
}

export function LogViewer({
  client = ipc,
  streamClient = stream,
  maxEntries = DEFAULT_MAX_ENTRIES,
  clipboard,
  onExport = downloadFile,
}: LogViewerProps) {
  const t = useMessage();
  const viewer = useLogViewer({ client, streamClient, maxEntries });
  const [notice, setNotice] = useState<string | null>(null);

  const copyEntry = useCallback(
    async (record: LogRecord) => {
      const target = clipboard ?? globalThis.navigator?.clipboard;
      if (!target) {
        setNotice(t("logsCopyUnavailable"));
        return;
      }
      try {
        await target.writeText(formatRecord(record));
        setNotice(t("logsCopied"));
      } catch {
        setNotice(t("logsCopyFailed"));
      }
    },
    [clipboard, t],
  );

  const exportFiltered = useCallback(async () => {
    try {
      const result = await exportLogs(client, viewer.query);
      onExport(result.suggested_file_name, result.content);
      setNotice(t("logsExported", { count: result.entry_count }));
    } catch (cause: unknown) {
      setNotice(
        t("logsExportFailed", { error: isIpcError(cause) ? cause.message : String(cause) }),
      );
    }
  }, [client, onExport, t, viewer.query]);

  return (
    <section
      aria-labelledby="log-viewer-heading"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "0.5rem",
        fontFamily: "system-ui, sans-serif",
        color: "#e5e7eb",
        textAlign: "start",
      }}
    >
      <h2 id="log-viewer-heading" style={{ fontSize: "1rem", margin: 0 }}>
        {t("logsTitle")}
      </h2>

      <div
        style={{ display: "flex", flexWrap: "wrap", gap: "0.75rem", alignItems: "flex-end" }}
        role="group"
        aria-label={t("logsFilters")}
      >
        <span style={{ display: "flex", flexDirection: "column" }}>
          <label htmlFor="log-level">{t("logsMinimumLevel")}</label>
          <select
            id="log-level"
            value={viewer.filter.minLevel}
            onChange={(event) =>
              viewer.setFilter({ minLevel: event.target.value as LogLevel | "" })
            }
          >
            <option value="">{t("logsAllLevels")}</option>
            {LOG_LEVELS.map((level) => (
              <option key={level} value={level}>
                {level}
              </option>
            ))}
          </select>
        </span>

        <span style={{ display: "flex", flexDirection: "column" }}>
          <label htmlFor="log-source">{t("logsSource")}</label>
          <select
            id="log-source"
            value={viewer.filter.source}
            onChange={(event) => viewer.setFilter({ source: event.target.value })}
          >
            <option value="">{t("logsAllSources")}</option>
            {viewer.sources.map((source) => (
              <option key={source} value={source}>
                {source}
              </option>
            ))}
          </select>
        </span>

        <span style={{ display: "flex", flexDirection: "column" }}>
          <label htmlFor="log-search">{t("logsSearch")}</label>
          <input
            id="log-search"
            type="search"
            value={viewer.filter.search}
            placeholder={t("logsSearchPlaceholder")}
            onChange={(event) => viewer.setFilter({ search: event.target.value })}
          />
        </span>

        <span style={{ display: "flex", flexDirection: "column" }}>
          <label htmlFor="log-from">{t("logsFromUtc")}</label>
          <input
            id="log-from"
            type="text"
            value={viewer.filter.fromTs}
            placeholder={t("logsFromPlaceholder")}
            onChange={(event) => viewer.setFilter({ fromTs: event.target.value })}
          />
        </span>

        <span style={{ display: "flex", flexDirection: "column" }}>
          <label htmlFor="log-to">{t("logsToUtc")}</label>
          <input
            id="log-to"
            type="text"
            value={viewer.filter.toTs}
            placeholder={t("logsToPlaceholder")}
            onChange={(event) => viewer.setFilter({ toTs: event.target.value })}
          />
        </span>

        <span style={{ display: "flex", flexDirection: "column" }}>
          <label htmlFor="log-correlation">{t("logsCorrelation")}</label>
          <input
            id="log-correlation"
            type="text"
            value={viewer.filter.correlationId}
            placeholder={t("logsCorrelationPlaceholder")}
            onChange={(event) => viewer.setFilter({ correlationId: event.target.value })}
          />
        </span>

        <span>
          <input
            id="log-follow"
            type="checkbox"
            checked={viewer.follow}
            onChange={(event) => viewer.setFollow(event.target.checked)}
          />
          <label htmlFor="log-follow">{t("logsFollow")}</label>
        </span>

        <button type="button" onClick={() => void exportFiltered()}>
          {t("logsExport")}
        </button>
        <button type="button" onClick={viewer.resetFilter}>
          {t("logsClearFilters")}
        </button>
      </div>

      <p style={{ margin: 0, fontSize: "0.85rem", color: "#9ca3af" }}>
        {viewer.status === "loading" && t("logsLoading")}
        {viewer.status === "ready" &&
          t("logsShowing", {
            shown: viewer.entries.length,
            matched: viewer.matched,
            evicted: viewer.evicted,
          })}
      </p>

      {viewer.status === "error" && viewer.error !== null && (
        <p role="alert">
          {viewer.error}{" "}
          <button type="button" onClick={viewer.refresh}>
            {t("logsRetry")}
          </button>
        </p>
      )}

      {notice !== null && (
        <p role="status" style={{ margin: 0, fontSize: "0.85rem" }}>
          {notice}
        </p>
      )}

      <ul
        // Not a live region: announcing every record would make a debug
        // stream unusable with a screen reader.
        role="log"
        aria-live="off"
        aria-label={t("logsEntries")}
        style={{
          listStyle: "none",
          margin: 0,
          padding: 0,
          fontFamily: "ui-monospace, monospace",
          fontSize: "0.8rem",
          maxHeight: "20rem",
          overflowY: "auto",
        }}
      >
        {viewer.entries.map((record, index) => (
          <li
            key={`${record.ts}-${record.source}-${index}`}
            data-level={record.level}
            data-correlation-id={record.correlation_id ?? undefined}
            style={{
              display: "flex",
              gap: "0.5rem",
              padding: "0.15rem 0",
              borderBottom: "1px solid #2a2a44",
            }}
          >
            <span style={{ color: "#9ca3af" }}>{record.ts}</span>
            <span style={{ color: LEVEL_COLORS[record.level] ?? "#e5e7eb" }}>
              {record.level.toUpperCase()}
            </span>
            <span dir="auto" style={{ color: "#c4b5fd" }}>
              {record.source}
            </span>
            {record.correlation_id !== null && (
              <span style={{ color: "#6ee7b7" }}>{record.correlation_id}</span>
            )}
            <span dir="auto" style={{ flex: 1 }}>
              {record.message}
            </span>
            {Object.keys(record.fields).length > 0 && (
              <span style={{ color: "#9ca3af" }}>{JSON.stringify(record.fields)}</span>
            )}
            <button
              type="button"
              aria-label={t("logsCopyEntry", { message: record.message })}
              onClick={() => void copyEntry(record)}
            >
              {t("logsCopy")}
            </button>
          </li>
        ))}
      </ul>

      {viewer.status === "ready" && viewer.entries.length === 0 && (
        <p style={{ margin: 0 }}>{t("logsNoEntries")}</p>
      )}
    </section>
  );
}
