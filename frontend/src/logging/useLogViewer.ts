import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { LogLevel } from "../generated/LogLevel";
import type { LogQuery } from "../generated/LogQuery";
import type { LogRecord } from "../generated/LogRecord";
import { isIpcError, type IpcClient } from "../ipc";
import type { StreamClient } from "../stream";
import { LOG_CHANNEL, emptyQuery, queryLogs } from "./commands";
import { matchesQuery, normalizeTimestampInput } from "./filter";

/**
 * State for the log viewer panel (Task 1.5, REQ-OBS-001.4).
 *
 * Kept out of the component so the filter, follow-tail, and live-append
 * behaviour can be reasoned about (and tested) without a renderer, and so a
 * second surface — the health dashboard's "show me this service's logs" jump
 * in Task 13.3 — can reuse it rather than reimplementing the query.
 *
 * The viewer holds two sources of records and they are deliberately not
 * merged in the kernel: `log.query` supplies history over the whole ring, and
 * the stream channel supplies new records as they happen. A query is
 * re-issued when the filter changes; the live tail is filtered client-side so
 * follow-tail does not cost one IPC round trip per log line.
 */

/** Filter state as the UI holds it: strings, with "" meaning unset. */
export interface LogFilterState {
  minLevel: LogLevel | "";
  source: string;
  fromTs: string;
  toTs: string;
  search: string;
  correlationId: string;
}

export const EMPTY_FILTER: LogFilterState = {
  minLevel: "",
  source: "",
  fromTs: "",
  toTs: "",
  search: "",
  correlationId: "",
};

/** Records rendered at once. The ring holds 10k; a DOM list should not. */
export const DEFAULT_MAX_ENTRIES = 1_000;

export type LogViewerStatus = "loading" | "ready" | "error";

export interface UseLogViewerOptions {
  client: IpcClient;
  streamClient?: StreamClient;
  maxEntries?: number;
  initialFilter?: Partial<LogFilterState>;
  initialFollow?: boolean;
}

export interface LogViewerState {
  filter: LogFilterState;
  setFilter: (patch: Partial<LogFilterState>) => void;
  resetFilter: () => void;
  /** The filter translated into the kernel's query shape. */
  query: LogQuery;
  entries: readonly LogRecord[];
  sources: readonly string[];
  /** Matches before the display cap, so "showing N of M" is honest. */
  matched: number;
  ringCapacity: number;
  evicted: number;
  follow: boolean;
  setFollow: (follow: boolean) => void;
  status: LogViewerStatus;
  error: string | null;
  refresh: () => void;
}

/** Translate UI filter state into a kernel query. */
export function buildQuery(filter: LogFilterState, limit: number): LogQuery {
  return {
    ...emptyQuery(),
    min_level: filter.minLevel === "" ? null : filter.minLevel,
    sources: filter.source === "" ? null : [filter.source],
    from_ts: normalizeTimestampInput(filter.fromTs),
    to_ts: normalizeTimestampInput(filter.toTs),
    search: filter.search === "" ? null : filter.search,
    correlation_id: filter.correlationId === "" ? null : filter.correlationId,
    limit,
  };
}

export function useLogViewer(options: UseLogViewerOptions): LogViewerState {
  const { client, streamClient, maxEntries = DEFAULT_MAX_ENTRIES } = options;

  const [filter, setFilterState] = useState<LogFilterState>({
    ...EMPTY_FILTER,
    ...options.initialFilter,
  });
  const [follow, setFollow] = useState(options.initialFollow ?? true);
  const [entries, setEntries] = useState<readonly LogRecord[]>([]);
  const [sources, setSources] = useState<readonly string[]>([]);
  const [matched, setMatched] = useState(0);
  const [ringCapacity, setRingCapacity] = useState(0);
  const [evicted, setEvicted] = useState(0);
  const [status, setStatus] = useState<LogViewerStatus>("loading");
  const [error, setError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);

  const query = useMemo(() => buildQuery(filter, maxEntries), [filter, maxEntries]);

  // Mirrored into refs so the stream subscription can test an arriving record
  // against the current filter without resubscribing on every keystroke. The
  // mirroring happens in an effect rather than during render, so a render that
  // React discards cannot leave the refs describing a filter the user never
  // saw.
  const queryRef = useRef(query);
  const followRef = useRef(follow);
  useEffect(() => {
    queryRef.current = query;
    followRef.current = follow;
  }, [query, follow]);

  // "Loading" is entered from the handlers that change the filter rather than
  // from the fetching effect, so the effect only ever settles state in a
  // promise callback and never triggers a synchronous render cascade.
  const setFilter = useCallback((patch: Partial<LogFilterState>) => {
    setStatus("loading");
    setFilterState((current) => ({ ...current, ...patch }));
  }, []);

  const resetFilter = useCallback(() => {
    setStatus("loading");
    setFilterState(EMPTY_FILTER);
  }, []);

  const refresh = useCallback(() => {
    setStatus("loading");
    setReloadToken((token) => token + 1);
  }, []);

  // History. Re-run on every filter change: the kernel owns the ring, so
  // narrowing a filter can reveal records the viewer never received live.
  useEffect(() => {
    let active = true;
    queryLogs(client, query).then(
      (response) => {
        if (!active) return;
        setEntries(response.entries);
        setSources(response.sources);
        setMatched(response.matched);
        setRingCapacity(response.ring_capacity);
        setEvicted(response.evicted);
        setError(null);
        setStatus("ready");
      },
      (cause: unknown) => {
        if (!active) return;
        setError(isIpcError(cause) ? `${cause.code}: ${cause.message}` : String(cause));
        setStatus("error");
      },
    );
    return () => {
      active = false;
    };
  }, [client, query, reloadToken]);

  // Live tail. The subscription is held for the panel's lifetime rather than
  // toggled with follow-tail, because the kernel publishes nothing while
  // nobody is subscribed and unsubscribing would lose the records emitted
  // while the user was scrolled back.
  useEffect(() => {
    if (!streamClient) return;
    return streamClient.subscribe<LogRecord>(LOG_CHANNEL, (record) => {
      if (!followRef.current) return;
      if (!matchesQuery(record, queryRef.current)) return;
      setEntries((current) => {
        const next = [...current, record];
        return next.length > maxEntries ? next.slice(next.length - maxEntries) : next;
      });
      setMatched((count) => count + 1);
    });
  }, [streamClient, maxEntries]);

  return {
    filter,
    setFilter,
    resetFilter,
    query,
    entries,
    sources,
    matched,
    ringCapacity,
    evicted,
    follow,
    setFollow,
    status,
    error,
    refresh,
  };
}
