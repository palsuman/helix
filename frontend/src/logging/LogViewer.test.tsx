import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { IpcRequest } from "../generated/IpcRequest";
import type { LogQuery } from "../generated/LogQuery";
import type { LogRecord } from "../generated/LogRecord";
import type { StreamEnvelope } from "../generated/StreamEnvelope";
import type { StreamFrame } from "../generated/StreamFrame";
import { IpcClient, type InvokeFn } from "../ipc";
import { StreamClient, type StreamSocket } from "../stream";
import { LOG_CHANNEL } from "./commands";
import { matchesQuery } from "./filter";
import { LogViewer } from "./LogViewer";

/**
 * Drives the viewer against a fake kernel holding a small ring buffer. The
 * fake applies the same filter rules the kernel does (via `matchesQuery`), so
 * these tests exercise the panel's behaviour rather than restating the filter
 * semantics already covered in `filter.test.ts`.
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

const HISTORY: LogRecord[] = [
  record({ ts: "2026-01-01T10:00:00.000Z", level: "debug", message: "cache warmed" }),
  record({
    ts: "2026-01-01T11:00:00.000Z",
    level: "error",
    source: "kernel.ipc",
    message: "dispatch failed",
    correlation_id: "cmd-42",
    fields: { code: "TIMEOUT" },
  }),
  record({ ts: "2026-01-01T12:00:00.000Z", level: "info", message: "file saved" }),
];

function fakeKernel(options: { entries?: LogRecord[]; failQuery?: boolean } = {}) {
  const entries = options.entries ?? HISTORY;
  const queries: LogQuery[] = [];
  let failQuery = options.failQuery ?? false;

  const invoke: InvokeFn = async <T,>(_command: string, args?: Record<string, unknown>) => {
    const request = (args as { request: IpcRequest<unknown> }).request;

    if (request.command === "log.query") {
      const query = (request.payload as { query: LogQuery }).query;
      queries.push(query);
      if (failQuery) {
        return {
          correlation_id: request.correlation_id,
          result: null,
          error: {
            code: "UNKNOWN_COMMAND",
            category: "permanent",
            message: "no handler registered for command 'log.query'",
            details: null,
          },
        } as T;
      }
      const matching = entries.filter((entry) => matchesQuery(entry, query));
      return {
        correlation_id: request.correlation_id,
        result: {
          entries: matching,
          matched: matching.length,
          ring_len: entries.length,
          ring_capacity: 10_000,
          evicted: 2,
          sources: [...new Set(entries.map((entry) => entry.source))].sort(),
        },
        error: null,
      } as T;
    }

    if (request.command === "log.export") {
      const query = (request.payload as { query: LogQuery }).query;
      const matching = entries.filter((entry) => matchesQuery(entry, query));
      return {
        correlation_id: request.correlation_id,
        result: {
          format: "jsonl",
          content: matching.map((entry) => JSON.stringify(entry)).join("\n") + "\n",
          entry_count: matching.length,
          suggested_file_name: "helix-log-export.jsonl",
        },
        error: null,
      } as T;
    }

    throw new Error(`unexpected command ${request.command}`);
  };

  return {
    client: new IpcClient({ invoke }),
    queries,
    breakQuery: (broken: boolean) => {
      failQuery = broken;
    },
  };
}

/** A stand-in for the kernel's stream socket, driven by the test. */
function fakeStream() {
  const sockets: StreamSocket[] = [];
  const sent: string[] = [];
  const client = new StreamClient({
    resolveEndpoint: async () => ({
      url: "ws://127.0.0.1:1/stream?token=t",
      port: 1,
      token: "t",
      heartbeat_interval_ms: 5_000,
      missed_heartbeat_limit: 3,
      default_buffer_depth: 1_000,
    }),
    socketFactory: () => {
      const socket: StreamSocket = {
        onopen: null,
        onclose: null,
        onerror: null,
        onmessage: null,
        send: (data: string) => sent.push(data),
        close: () => {},
      };
      sockets.push(socket);
      return socket;
    },
  });

  let sequence = 0;
  return {
    client,
    sent,
    open: async () => {
      client.connect();
      await waitFor(() => expect(sockets.length).toBeGreaterThan(0));
      await act(async () => {
        sockets[0].onopen?.();
      });
    },
    emit: async (entry: LogRecord) => {
      sequence += 1;
      const envelope: StreamEnvelope = {
        channel: LOG_CHANNEL,
        correlation_id: entry.correlation_id,
        sequence,
        payload: entry,
      };
      const frame: StreamFrame = { kind: "data", ...envelope };
      await act(async () => {
        sockets[0].onmessage?.({ data: JSON.stringify(frame) });
      });
    },
  };
}

describe("LogViewer", () => {
  it("renders kernel and frontend entries in one list", async () => {
    const kernel = fakeKernel({
      entries: [
        record({ source: "kernel.fs", message: "kernel side" }),
        record({ source: "frontend.app", message: "frontend side" }),
      ],
    });
    render(<LogViewer client={kernel.client} streamClient={fakeStream().client} />);

    expect(await screen.findByText("kernel side")).toBeInTheDocument();
    expect(screen.getByText("frontend side")).toBeInTheDocument();
    expect(screen.getByRole("log", { name: "Log entries" })).toBeInTheDocument();
  });

  it("filters by minimum level", async () => {
    const kernel = fakeKernel();
    render(<LogViewer client={kernel.client} streamClient={fakeStream().client} />);
    expect(await screen.findByText("cache warmed")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Minimum level"), { target: { value: "error" } });

    await waitFor(() => expect(screen.queryByText("cache warmed")).not.toBeInTheDocument());
    expect(screen.getByText("dispatch failed")).toBeInTheDocument();
    expect(kernel.queries.at(-1)?.min_level).toBe("error");
  });

  it("filters by source, listing only the sources that logged", async () => {
    const kernel = fakeKernel();
    render(<LogViewer client={kernel.client} streamClient={fakeStream().client} />);
    await screen.findByText("cache warmed");

    const sourceFilter = screen.getByLabelText("Source");
    expect(screen.getByRole("option", { name: "kernel.ipc" })).toBeInTheDocument();
    fireEvent.change(sourceFilter, { target: { value: "kernel.ipc" } });

    await waitFor(() => expect(screen.queryByText("file saved")).not.toBeInTheDocument());
    expect(screen.getByText("dispatch failed")).toBeInTheDocument();
  });

  it("filters by a UTC time range typed in a partial form", async () => {
    const kernel = fakeKernel();
    render(<LogViewer client={kernel.client} streamClient={fakeStream().client} />);
    await screen.findByText("cache warmed");

    fireEvent.change(screen.getByLabelText("From (UTC)"), {
      target: { value: "2026-01-01T11:30" },
    });

    await waitFor(() => expect(screen.queryByText("cache warmed")).not.toBeInTheDocument());
    expect(kernel.queries.at(-1)?.from_ts).toBe("2026-01-01T11:30:00.000Z");
    expect(screen.getByText("file saved")).toBeInTheDocument();
  });

  it("searches full text across message and fields", async () => {
    const kernel = fakeKernel();
    render(<LogViewer client={kernel.client} streamClient={fakeStream().client} />);
    await screen.findByText("cache warmed");

    fireEvent.change(screen.getByLabelText("Search"), { target: { value: "TIMEOUT" } });

    await waitFor(() => expect(screen.queryByText("cache warmed")).not.toBeInTheDocument());
    expect(screen.getByText("dispatch failed")).toBeInTheDocument();
  });

  it("filters by correlation id, which is how a command's kernel work is found", async () => {
    const kernel = fakeKernel();
    render(<LogViewer client={kernel.client} streamClient={fakeStream().client} />);
    await screen.findByText("cache warmed");

    fireEvent.change(screen.getByLabelText("Correlation ID"), { target: { value: "cmd-42" } });

    await waitFor(() => expect(screen.queryByText("file saved")).not.toBeInTheDocument());
    expect(screen.getByText("dispatch failed")).toBeInTheDocument();
    expect(screen.getByText("cmd-42")).toBeInTheDocument();
  });

  it("clears every filter at once", async () => {
    const kernel = fakeKernel();
    render(<LogViewer client={kernel.client} streamClient={fakeStream().client} />);
    await screen.findByText("cache warmed");

    fireEvent.change(screen.getByLabelText("Search"), {
      target: { value: "nothing matches this" },
    });
    await waitFor(() => expect(screen.getByText(/No entries match/)).toBeInTheDocument());

    fireEvent.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(await screen.findByText("cache warmed")).toBeInTheDocument();
  });

  it("appends live entries while following the tail", async () => {
    const kernel = fakeKernel();
    const streaming = fakeStream();
    render(<LogViewer client={kernel.client} streamClient={streaming.client} />);
    await screen.findByText("cache warmed");
    await streaming.open();

    await streaming.emit(record({ ts: "2026-01-01T13:00:00.000Z", message: "arrived live" }));
    expect(screen.getByText("arrived live")).toBeInTheDocument();
  });

  it("stops appending when follow tail is switched off", async () => {
    const kernel = fakeKernel();
    const streaming = fakeStream();
    render(<LogViewer client={kernel.client} streamClient={streaming.client} />);
    await screen.findByText("cache warmed");
    await streaming.open();

    fireEvent.click(screen.getByLabelText("Follow tail"));
    await streaming.emit(record({ message: "ignored while paused" }));
    expect(screen.queryByText("ignored while paused")).not.toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("Follow tail"));
    await streaming.emit(record({ message: "back to following" }));
    expect(screen.getByText("back to following")).toBeInTheDocument();
  });

  it("does not append a live entry the current filter excludes", async () => {
    const kernel = fakeKernel();
    const streaming = fakeStream();
    render(<LogViewer client={kernel.client} streamClient={streaming.client} />);
    await screen.findByText("cache warmed");
    await streaming.open();

    fireEvent.change(screen.getByLabelText("Minimum level"), { target: { value: "error" } });
    await waitFor(() => expect(screen.queryByText("cache warmed")).not.toBeInTheDocument());

    await streaming.emit(record({ level: "debug", message: "too quiet to show" }));
    expect(screen.queryByText("too quiet to show")).not.toBeInTheDocument();

    await streaming.emit(record({ level: "error", message: "loud enough" }));
    expect(screen.getByText("loud enough")).toBeInTheDocument();
  });

  it("copies an entry to the clipboard", async () => {
    const kernel = fakeKernel({ entries: [record({ message: "copy me" })] });
    const written: string[] = [];
    render(
      <LogViewer
        client={kernel.client}
        streamClient={fakeStream().client}
        clipboard={{
          writeText: async (text: string) => {
            written.push(text);
          },
        }}
      />,
    );
    await screen.findByText("copy me");

    fireEvent.click(screen.getByRole("button", { name: "Copy entry: copy me" }));

    await waitFor(() => expect(written).toHaveLength(1));
    expect(written[0]).toContain("copy me");
    expect(written[0]).toContain("kernel.fs");
    expect(await screen.findByText("Entry copied.")).toBeInTheDocument();
  });

  it("reports a clipboard failure instead of throwing", async () => {
    const kernel = fakeKernel({ entries: [record({ message: "copy me" })] });
    render(
      <LogViewer
        client={kernel.client}
        streamClient={fakeStream().client}
        clipboard={{
          writeText: async () => {
            throw new Error("permission denied");
          },
        }}
      />,
    );
    await screen.findByText("copy me");

    fireEvent.click(screen.getByRole("button", { name: "Copy entry: copy me" }));
    expect(await screen.findByText("The entry could not be copied.")).toBeInTheDocument();
  });

  it("exports the filtered set as JSON lines", async () => {
    const kernel = fakeKernel();
    const exported: { fileName: string; content: string }[] = [];
    render(
      <LogViewer
        client={kernel.client}
        streamClient={fakeStream().client}
        onExport={(fileName, content) => exported.push({ fileName, content })}
      />,
    );
    await screen.findByText("cache warmed");

    fireEvent.change(screen.getByLabelText("Minimum level"), { target: { value: "error" } });
    await waitFor(() => expect(screen.queryByText("cache warmed")).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "Export filtered set" }));

    await waitFor(() => expect(exported).toHaveLength(1));
    expect(exported[0].fileName).toBe("helix-log-export.jsonl");
    expect(exported[0].content.trim().split("\n")).toHaveLength(1);
    expect(JSON.parse(exported[0].content.trim()).message).toBe("dispatch failed");
    expect(await screen.findByText("Exported 1 entry.")).toBeInTheDocument();
  });

  it("reports how many entries are shown, matched, and no longer retained", async () => {
    const kernel = fakeKernel();
    render(<LogViewer client={kernel.client} streamClient={fakeStream().client} />);
    expect(await screen.findByText(/Showing 3 of 3 matching entries/)).toBeInTheDocument();
    expect(screen.getByText(/2 older entries no longer retained/)).toBeInTheDocument();
  });

  it("surfaces a query failure and retries on request", async () => {
    const kernel = fakeKernel({ failQuery: true });
    render(<LogViewer client={kernel.client} streamClient={fakeStream().client} />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("UNKNOWN_COMMAND");

    kernel.breakQuery(false);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("cache warmed")).toBeInTheDocument();
  });

  it("caps the rendered list, keeping the newest entries", async () => {
    const kernel = fakeKernel({ entries: [record({ message: "history" })] });
    const streaming = fakeStream();
    render(<LogViewer client={kernel.client} streamClient={streaming.client} maxEntries={2} />);
    await screen.findByText("history");
    await streaming.open();

    await streaming.emit(record({ message: "second" }));
    await streaming.emit(record({ message: "third" }));

    expect(screen.queryByText("history")).not.toBeInTheDocument();
    expect(screen.getByText("second")).toBeInTheDocument();
    expect(screen.getByText("third")).toBeInTheDocument();
  });

  it("subscribes to the log channel so the kernel starts publishing", async () => {
    const kernel = fakeKernel();
    const streaming = fakeStream();
    render(<LogViewer client={kernel.client} streamClient={streaming.client} />);
    await screen.findByText("cache warmed");
    await streaming.open();

    await vi.waitFor(() =>
      expect(streaming.sent.some((frame) => frame.includes(LOG_CHANNEL))).toBe(true),
    );
  });
});
