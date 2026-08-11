import type { LogLevel } from "../generated/LogLevel";
import type { LogQuery } from "../generated/LogQuery";
import type { LogRecord } from "../generated/LogRecord";
import { LOG_LEVELS } from "./commands";

/**
 * Client-side evaluation of a {@link LogQuery} (Task 1.5, REQ-OBS-001.4).
 *
 * The kernel is authoritative: `log.query` applies the same rules over the
 * full ring buffer. This exists for the live tail, where a record arrives on
 * the stream channel and has to be admitted or discarded without a round
 * trip. Re-querying the kernel on every record would make follow-tail cost
 * one IPC call per log line, which for a service logging at debug level is
 * exactly the traffic the streaming channel exists to avoid.
 *
 * The rules deliberately mirror `helix_log::filter::LogQuery::matches`,
 * including the string comparison for timestamps: the record format is
 * fixed-width RFC 3339, so lexicographic order is chronological order.
 */

/** Rank of a level, matching `LogLevel::rank` in Rust. */
export function levelRank(level: LogLevel): number {
  const rank = LOG_LEVELS.indexOf(level);
  // An unknown level from a newer kernel is treated as the most severe
  // rather than dropped: an entry the viewer cannot classify is still an
  // entry the user should see.
  return rank === -1 ? LOG_LEVELS.length : rank;
}

/** A filter entry matches a source exactly, or as its dot-separated ancestor. */
export function sourceMatches(filter: string, source: string): boolean {
  return source === filter || (source.startsWith(filter) && source[filter.length] === ".");
}

/** Whether a record satisfies every populated criterion of a query. */
export function matchesQuery(record: LogRecord, query: LogQuery): boolean {
  if (query.min_level !== null && levelRank(record.level) < levelRank(query.min_level)) {
    return false;
  }
  if (query.levels !== null && !query.levels.includes(record.level)) {
    return false;
  }
  if (query.sources !== null && !query.sources.some((s) => sourceMatches(s, record.source))) {
    return false;
  }
  if (query.from_ts !== null && record.ts < query.from_ts) return false;
  if (query.to_ts !== null && record.ts > query.to_ts) return false;
  if (query.correlation_id !== null && record.correlation_id !== query.correlation_id) {
    return false;
  }
  if (query.search !== null && query.search !== "" && !fullTextMatch(record, query.search)) {
    return false;
  }
  return true;
}

/** Case-insensitive search over message, source, correlation ID, and fields. */
export function fullTextMatch(record: LogRecord, needle: string): boolean {
  const lowered = needle.toLowerCase();
  if (record.message.toLowerCase().includes(lowered)) return true;
  if (record.source.toLowerCase().includes(lowered)) return true;
  if (record.correlation_id !== null && record.correlation_id.toLowerCase().includes(lowered)) {
    return true;
  }
  const fieldKeys = Object.keys(record.fields);
  if (fieldKeys.length === 0) return false;
  return JSON.stringify(record.fields).toLowerCase().includes(lowered);
}

/**
 * Normalize a user-entered instant to the kernel's fixed-width RFC 3339
 * form, or null when it cannot be understood.
 *
 * Accepts what a `datetime-local` input produces (`2026-01-01T10:00` and
 * `2026-01-01T10:00:30`), a bare date, and an already-complete timestamp.
 * The value is interpreted as UTC, which the viewer's labels state
 * explicitly: silently applying the machine's offset would make a time-range
 * filter disagree with the timestamps shown next to it.
 */
export function normalizeTimestampInput(value: string): string | null {
  const trimmed = value.trim();
  if (trimmed === "") return null;

  const match =
    /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?(?:\.(\d{1,3}))?)?Z?$/.exec(
      trimmed,
    );
  if (!match) return null;

  const [, year, month, day, hour = "00", minute = "00", second = "00", milli = "0"] = match;
  return `${year}-${month}-${day}T${hour}:${minute}:${second}.${milli.padEnd(3, "0")}Z`;
}

/** The record as the single JSON line the kernel wrote, for copy and export. */
export function toJsonLine(record: LogRecord): string {
  return JSON.stringify(record);
}

/** A compact human-readable rendering, for a copy that goes into a chat. */
export function formatRecord(record: LogRecord): string {
  const correlation = record.correlation_id === null ? "" : ` [${record.correlation_id}]`;
  const fieldKeys = Object.keys(record.fields);
  const fields = fieldKeys.length === 0 ? "" : ` ${JSON.stringify(record.fields)}`;
  return `${record.ts} ${record.level.toUpperCase()} ${record.source}${correlation} ${record.message}${fields}`;
}
