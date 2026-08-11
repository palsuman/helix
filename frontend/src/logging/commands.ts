import type { LevelConfig } from "../generated/LevelConfig";
import type { LogAppendRequest } from "../generated/LogAppendRequest";
import type { LogAppendResponse } from "../generated/LogAppendResponse";
import type { LogExportRequest } from "../generated/LogExportRequest";
import type { LogExportResponse } from "../generated/LogExportResponse";
import type { LogLevel } from "../generated/LogLevel";
import type { LogLevelsRequest } from "../generated/LogLevelsRequest";
import type { LogLevelsResponse } from "../generated/LogLevelsResponse";
import type { LogQuery } from "../generated/LogQuery";
import type { LogQueryRequest } from "../generated/LogQueryRequest";
import type { LogQueryResponse } from "../generated/LogQueryResponse";
import type { LogSetLevelRequest } from "../generated/LogSetLevelRequest";
import type { InvokeOptions, IpcClient } from "../ipc";
import { STREAM_CHANNELS } from "../stream";

/**
 * Typed wrappers for the kernel's `log.*` commands (Task 1.5, REQ-OBS-001).
 *
 * The request and response shapes are the generated types in
 * `src/generated/`, produced from the Rust definitions in `helix-log`, so a
 * change to the record model surfaces here as a type error.
 */

export const LOG_COMMANDS = {
  query: "log.query",
  export: "log.export",
  append: "log.append",
  levels: "log.levels",
  setLevel: "log.set_level",
} as const;

/** The channel carrying every record live, for follow-tail. */
export const LOG_CHANNEL = STREAM_CHANNELS.logEntries;

/** Every level, most verbose first. Mirrors `LogLevel::ALL`. */
export const LOG_LEVELS: readonly LogLevel[] = ["trace", "debug", "info", "warn", "error"] as const;

/** An unfiltered query: what the viewer opens with. */
export function emptyQuery(): LogQuery {
  return {
    min_level: null,
    levels: null,
    sources: null,
    from_ts: null,
    to_ts: null,
    search: null,
    correlation_id: null,
    limit: null,
  };
}

export function queryLogs(
  client: IpcClient,
  query: LogQuery,
  options?: InvokeOptions,
): Promise<LogQueryResponse> {
  return client.invoke<LogQueryRequest, LogQueryResponse>(LOG_COMMANDS.query, { query }, options);
}

/** The filtered set as JSON lines, ready to save (REQ-OBS-001.5). */
export function exportLogs(
  client: IpcClient,
  query: LogQuery,
  options?: InvokeOptions,
): Promise<LogExportResponse> {
  return client.invoke<LogExportRequest, LogExportResponse>(
    LOG_COMMANDS.export,
    { query },
    options,
  );
}

/**
 * Ship a frontend record into the kernel's unified stream
 * (REQ-OBS-001.3). The kernel files it under `frontend.<source>` and
 * redacts it like any other record.
 */
export function appendLog(
  client: IpcClient,
  record: LogAppendRequest,
  options?: InvokeOptions,
): Promise<LogAppendResponse> {
  return client.invoke<LogAppendRequest, LogAppendResponse>(LOG_COMMANDS.append, record, options);
}

export function logLevels(client: IpcClient, options?: InvokeOptions): Promise<LogLevelsResponse> {
  return client.invoke<LogLevelsRequest, LogLevelsResponse>(LOG_COMMANDS.levels, {}, options);
}

/**
 * Set a module's level, or the default level when `module` is null. A null
 * `level` with a module clears that module's override (REQ-OBS-001.2).
 */
export function setLogLevel(
  client: IpcClient,
  module: string | null,
  level: LogLevel | null,
  options?: InvokeOptions,
): Promise<LevelConfig> {
  return client
    .invoke<LogSetLevelRequest, LogLevelsResponse>(
      LOG_COMMANDS.setLevel,
      { module, level },
      options,
    )
    .then((response) => response.levels);
}
