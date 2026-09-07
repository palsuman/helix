import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { StreamClient } from "../stream/client";
import { useStreamBackpressure, useStreamChannel, useStreamStatus } from "../stream/useStream";
import { createMockStream } from "./mockStream";

function StreamView({
  client,
  channel = "test:updates",
}: {
  client: StreamClient;
  channel?: string;
}) {
  const payload = useStreamChannel<{ text: string }>(client, channel);
  const status = useStreamStatus(client);
  const backpressure = useStreamBackpressure(client);
  return (
    <section aria-label="Stream fixture">
      <p role="status" aria-label="Connection">
        {status}
      </p>
      <output aria-label="Latest message">{payload?.text ?? "Waiting"}</output>
      <output aria-label="Dropped messages">{backpressure?.dropped ?? 0}</output>
    </section>
  );
}

describe("stream-driven components", () => {
  let stream: ReturnType<typeof createMockStream>;
  afterEach(() => {
    cleanup();
    stream?.dispose();
    vi.useRealTimers();
  });

  it("renders messages, gaps, and reconnect state through real stream hooks", async () => {
    vi.useFakeTimers();
    stream = createMockStream();
    render(<StreamView client={stream.client} />);
    expect(screen.getByRole("status", { name: "Connection" })).toHaveTextContent("idle");
    await act(async () => {
      stream.client.connect();
    });
    act(() => stream.latest().open());
    expect(screen.getByRole("status", { name: "Connection" })).toHaveTextContent("open");
    expect(stream.latest().controls()).toContainEqual({
      type: "subscribe",
      channels: [{ channel: "test:updates", from_sequence: null }],
    });
    act(() => stream.latest().emitData("test:updates", 1, { text: "First" }));
    expect(screen.getByLabelText("Latest message")).toHaveTextContent("First");
    act(() => stream.latest().emitData("test:updates", 3, { text: "Third" }));
    expect(screen.getByLabelText("Latest message")).toHaveTextContent("Third");
    expect(screen.getByLabelText("Dropped messages")).toHaveTextContent("1");
    act(() => stream.latest().die());
    expect(screen.getByRole("status", { name: "Connection" })).toHaveTextContent("reconnecting");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
    act(() => stream.latest().open());
    expect(stream.sockets).toHaveLength(2);
    expect(stream.latest().controls()).toContainEqual({
      type: "subscribe",
      channels: [{ channel: "test:updates", from_sequence: 3 }],
    });
    act(() => stream.latest().emitData("test:updates", 4, { text: "Recovered" }));
    expect(screen.getByLabelText("Latest message")).toHaveTextContent("Recovered");
  });

  it("switches subscriptions and releases them on unmount", async () => {
    stream = createMockStream();
    const view = render(<StreamView client={stream.client} />);
    await act(async () => {
      stream.client.connect();
    });
    act(() => stream.latest().open());
    act(() => stream.latest().emitData("test:updates", 1, { text: "Old channel" }));
    view.rerender(<StreamView client={stream.client} channel="test:other" />);
    expect(screen.getByLabelText("Latest message")).toHaveTextContent("Waiting");
    expect(stream.latest().controls()).toContainEqual({
      type: "unsubscribe",
      channels: ["test:updates"],
    });
    act(() => stream.latest().emitData("test:other", 1, { text: "New channel" }));
    expect(screen.getByLabelText("Latest message")).toHaveTextContent("New channel");
    view.unmount();
    expect(stream.latest().controls()).toContainEqual({
      type: "unsubscribe",
      channels: ["test:other"],
    });
    stream.dispose();
    expect(stream.latest().closed).toBe(true);
  });
});
