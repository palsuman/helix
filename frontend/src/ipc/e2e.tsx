import { invoke } from "@tauri-apps/api/core";
import { createRoot } from "react-dom/client";
import type { PingResponse } from "../generated/PingResponse";
import { IpcClient } from "./client";
import { isIpcError } from "./errors";
import { ping, sleep } from "./commands";

interface IpcE2eReport {
  passed: boolean;
  rendered_response: PingResponse | null;
  cancellation_ms: number | null;
  stale_peer_rejected: boolean;
  post_restart_response: PingResponse | null;
  benchmark: {
    samples: number;
    p50_ms: number;
    p95_ms: number;
    p99_ms: number;
    max_ms: number;
  } | null;
  error: string | null;
}

function percentile(sorted: readonly number[], fraction: number): number {
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)];
}

async function benchmark(client: IpcClient) {
  for (let index = 0; index < 25; index += 1) {
    await ping(client, `warmup-${index}`);
  }

  const samples: number[] = [];
  for (let index = 0; index < 250; index += 1) {
    const startedAt = performance.now();
    await ping(client, `benchmark-${index}`);
    samples.push(performance.now() - startedAt);
  }
  samples.sort((left, right) => left - right);
  return {
    samples: samples.length,
    p50_ms: percentile(samples, 0.5),
    p95_ms: percentile(samples, 0.95),
    p99_ms: percentile(samples, 0.99),
    max_ms: samples[samples.length - 1],
  };
}

function renderStatus(title: string, detail: unknown): void {
  createRoot(document.getElementById("root")!).render(
    <main>
      <h1>{title}</h1>
      <pre>{JSON.stringify(detail, null, 2)}</pre>
    </main>,
  );
}

export async function runIpcE2e(): Promise<void> {
  const client = new IpcClient();
  const report: IpcE2eReport = {
    passed: false,
    rendered_response: null,
    cancellation_ms: null,
    stale_peer_rejected: false,
    post_restart_response: null,
    benchmark: null,
    error: null,
  };

  try {
    report.rendered_response = await ping(client, "WebView typed round trip");
    renderStatus("Kernel response", report.rendered_response);

    const controller = new AbortController();
    const pending = sleep(client, 10_000, { signal: controller.signal });
    await new Promise((resolve) => setTimeout(resolve, 25));
    const cancelledAt = performance.now();
    controller.abort();
    const cancellation = await pending.catch((error: unknown) => error);
    report.cancellation_ms = performance.now() - cancelledAt;
    if (!isIpcError(cancellation) || !cancellation.isCancelled) {
      throw new Error(`long command did not return a typed cancellation: ${String(cancellation)}`);
    }
    if (report.cancellation_ms >= 100) {
      throw new Error(`cancellation took ${report.cancellation_ms.toFixed(3)}ms`);
    }

    report.stale_peer_rejected = await invoke<boolean>("ipc_e2e_restart");
    if (!report.stale_peer_rejected) {
      throw new Error("pre-restart credentials were accepted by the replacement kernel");
    }
    report.post_restart_response = await ping(client, "replacement kernel");

    report.benchmark = await benchmark(client);
    if (report.benchmark.p95_ms >= 5) {
      throw new Error(`WebView IPC p95 was ${report.benchmark.p95_ms.toFixed(3)}ms`);
    }
    report.passed = true;
    renderStatus("IPC E2E passed", report);
  } catch (error) {
    report.error = error instanceof Error ? (error.stack ?? error.message) : String(error);
    renderStatus("IPC E2E failed", report);
  }

  await invoke("ipc_e2e_report", { report });
}
