import { describe, expect, it } from "vitest";
import type { LogLevel } from "../generated/LogLevel";
import type { LogRecord } from "../generated/LogRecord";
import { emptyQuery } from "./commands";
import {
  formatRecord,
  fullTextMatch,
  levelRank,
  matchesQuery,
  normalizeTimestampInput,
  sourceMatches,
  toJsonLine,
} from "./filter";

/**
 * The client-side filter must agree with `helix_log::filter::LogQuery`, since
 * the live tail is admitted by this code and the history by that code. A
 * disagreement would show as records appearing while following and vanishing
 * on the next query.
 */

function record(overrides: Partial<LogRecord> = {}): LogRecord {
  return {
    ts: "2026-01-01T12:00:00.000Z",
    level: "info",
    source: "kernel.fs",
    correlation_id: null,
    message: "file saved",
    fields: {},
    ...overrides,
  };
}

describe("levelRank", () => {
  it("orders levels from trace to error", () => {
    const levels: LogLevel[] = ["trace", "debug", "info", "warn", "error"];
    const ranks = levels.map(levelRank);
    expect(ranks).toEqual([...ranks].sort((a, b) => a - b));
  });

  it("treats an unrecognized level as the most severe rather than hiding it", () => {
    expect(levelRank("fatal" as LogLevel)).toBeGreaterThan(levelRank("error"));
  });
});

describe("sourceMatches", () => {
  it("matches a source exactly and as a dot-separated ancestor", () => {
    expect(sourceMatches("kernel", "kernel")).toBe(true);
    expect(sourceMatches("kernel", "kernel.fs.watcher")).toBe(true);
  });

  it("respects segment boundaries", () => {
    expect(sourceMatches("kernel.fs", "kernel.fsevents")).toBe(false);
    expect(sourceMatches("kernel", "kernelish")).toBe(false);
  });
});

describe("matchesQuery", () => {
  it("admits everything when no criterion is set", () => {
    expect(matchesQuery(record({ level: "trace" }), emptyQuery())).toBe(true);
  });

  it("filters by minimum level", () => {
    const query = { ...emptyQuery(), min_level: "warn" as LogLevel };
    expect(matchesQuery(record({ level: "error" }), query)).toBe(true);
    expect(matchesQuery(record({ level: "info" }), query)).toBe(false);
  });

  it("filters by an explicit level set", () => {
    const query = { ...emptyQuery(), levels: ["warn", "error"] as LogLevel[] };
    expect(matchesQuery(record({ level: "warn" }), query)).toBe(true);
    expect(matchesQuery(record({ level: "debug" }), query)).toBe(false);
  });

  it("filters by source, including descendants", () => {
    const query = { ...emptyQuery(), sources: ["kernel"] };
    expect(matchesQuery(record({ source: "kernel.ipc" }), query)).toBe(true);
    expect(matchesQuery(record({ source: "frontend.app" }), query)).toBe(false);
  });

  it("filters by an inclusive time range", () => {
    const query = {
      ...emptyQuery(),
      from_ts: "2026-01-01T12:00:00.000Z",
      to_ts: "2026-01-01T12:00:00.000Z",
    };
    expect(matchesQuery(record(), query)).toBe(true);
    expect(matchesQuery(record({ ts: "2026-01-01T11:59:59.999Z" }), query)).toBe(false);
    expect(matchesQuery(record({ ts: "2026-01-01T12:00:00.001Z" }), query)).toBe(false);
  });

  it("filters by exact correlation id", () => {
    const query = { ...emptyQuery(), correlation_id: "cmd-1" };
    expect(matchesQuery(record({ correlation_id: "cmd-1" }), query)).toBe(true);
    expect(matchesQuery(record({ correlation_id: "cmd-2" }), query)).toBe(false);
    expect(matchesQuery(record(), query)).toBe(false);
  });

  it("combines criteria with AND", () => {
    const entry = record({ level: "error", source: "kernel.fs", message: "disk full" });
    const query = { ...emptyQuery(), min_level: "warn" as LogLevel, search: "disk" };
    expect(matchesQuery(entry, query)).toBe(true);
    expect(matchesQuery(entry, { ...query, search: "network" })).toBe(false);
  });

  it("ignores an empty search string", () => {
    expect(matchesQuery(record(), { ...emptyQuery(), search: "" })).toBe(true);
  });
});

describe("fullTextMatch", () => {
  it("searches message, source, correlation id, and fields case-insensitively", () => {
    const entry = record({
      message: "Server started",
      source: "lsp_host",
      correlation_id: "cmd-ABC",
      fields: { language: "TypeScript", startup_ms: 1200 },
    });
    expect(fullTextMatch(entry, "server")).toBe(true);
    expect(fullTextMatch(entry, "LSP_HOST")).toBe(true);
    expect(fullTextMatch(entry, "cmd-abc")).toBe(true);
    expect(fullTextMatch(entry, "typescript")).toBe(true);
    expect(fullTextMatch(entry, "1200")).toBe(true);
    expect(fullTextMatch(entry, "python")).toBe(false);
  });
});

describe("normalizeTimestampInput", () => {
  it("pads a partial instant to the kernel's fixed-width form", () => {
    expect(normalizeTimestampInput("2026-01-01T10:00")).toBe("2026-01-01T10:00:00.000Z");
    expect(normalizeTimestampInput("2026-01-01T10:00:30")).toBe("2026-01-01T10:00:30.000Z");
    expect(normalizeTimestampInput("2026-01-01")).toBe("2026-01-01T00:00:00.000Z");
    expect(normalizeTimestampInput("2026-01-01T10:00:30.5")).toBe("2026-01-01T10:00:30.500Z");
  });

  it("accepts a complete timestamp unchanged", () => {
    expect(normalizeTimestampInput("2026-01-01T10:00:30.123Z")).toBe("2026-01-01T10:00:30.123Z");
  });

  it("returns null for empty or unparseable input rather than a wrong bound", () => {
    expect(normalizeTimestampInput("")).toBeNull();
    expect(normalizeTimestampInput("   ")).toBeNull();
    expect(normalizeTimestampInput("yesterday")).toBeNull();
    expect(normalizeTimestampInput("01/01/2026")).toBeNull();
  });

  it("produces bounds that compare chronologically as strings", () => {
    const from = normalizeTimestampInput("2026-01-01T09:00")!;
    const to = normalizeTimestampInput("2026-01-01T17:00")!;
    expect(from < to).toBe(true);
    expect(from.length).toBe(to.length);
  });
});

describe("record rendering", () => {
  it("copies as one JSON line, matching the kernel's file format", () => {
    const line = toJsonLine(record({ fields: { path: "/tmp/x" } }));
    expect(line).not.toContain("\n");
    expect(JSON.parse(line).fields.path).toBe("/tmp/x");
  });

  it("formats a readable single line including correlation id and fields", () => {
    const text = formatRecord(
      record({ level: "warn", correlation_id: "cmd-9", fields: { free_mb: 12 } }),
    );
    expect(text).toContain("WARN");
    expect(text).toContain("kernel.fs");
    expect(text).toContain("cmd-9");
    expect(text).toContain("free_mb");
  });
});
