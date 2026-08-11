import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import type { BackpressureEvent, StreamClient, StreamStatus } from "./client";

/**
 * React bindings for the streaming client (Task 1.4).
 *
 * Kept separate from the client so the connection state machine stays
 * testable without a renderer, and so a non-React consumer (a plain module
 * subscribing to logs, for instance) does not pull React in.
 */

/**
 * The client's connection status, re-rendering the caller on change.
 *
 * `useSyncExternalStore` rather than an effect plus `setState`: the client is
 * exactly the external store this hook is for, and the status may already
 * have moved on between render and effect.
 */
export function useStreamStatus(client: StreamClient): StreamStatus {
  const subscribe = useCallback(
    (onStoreChange: () => void) => client.onStatus(onStoreChange),
    [client],
  );
  const getSnapshot = useCallback(() => client.status, [client]);
  return useSyncExternalStore(subscribe, getSnapshot);
}

/**
 * The most recent payload on a channel, or `null` before the first message.
 *
 * The channel is stored alongside the payload so switching channels reads as
 * "nothing yet" immediately, without an effect that resets state and forces a
 * second render.
 */
export function useStreamChannel<T>(client: StreamClient, channel: string): T | null {
  const [entry, setEntry] = useState<{ channel: string; payload: T } | null>(null);
  useEffect(
    () => client.subscribe<T>(channel, (payload: T) => setEntry({ channel, payload })),
    [client, channel],
  );
  return entry !== null && entry.channel === channel ? entry.payload : null;
}

/** The most recent truncation event, or `null` if nothing has been lost. */
export function useStreamBackpressure(client: StreamClient): BackpressureEvent | null {
  const [event, setEvent] = useState<BackpressureEvent | null>(null);
  useEffect(() => client.onBackpressure(setEvent), [client]);
  return event;
}
