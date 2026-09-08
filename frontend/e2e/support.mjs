import { browser, $ } from "@wdio/globals";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { randomUUID } from "node:crypto";

export async function invokeKernel(command, payload = {}) {
  const correlationId = randomUUID();
  const response = await browser.tauri.execute(
    ({ core }, request) => core.invoke("ipc_dispatch", { request }),
    { command, payload, correlation_id: correlationId, timeout_ms: 5_000, window_id: null },
  );
  if (response.correlation_id !== correlationId) throw new Error("IPC correlation ID mismatch");
  if (response.error) throw new Error(`${command}: ${response.error.message}`);
  return response.result;
}

export async function waitForWorkbench() {
  await $('[data-testid="workbench"]').waitForDisplayed();
  await browser.waitUntil(
    async () => {
      const status = await browser.tauri.execute(({ core }) => core.invoke("supervisor_status"));
      return status.state === "running";
    },
    { timeoutMsg: "The supervised kernel did not become ready" },
  );
}

export async function mockKernelCommands(responses) {
  await browser.execute((fixtures) => {
    globalThis.__HELIX_E2E_RESPONSES__ = fixtures;
  }, responses);
  await browser.execute(() => {
    globalThis.__HELIX_E2E_ORIGINAL_INVOKE__ = window.__TAURI_INTERNALS__.invoke.bind(
      window.__TAURI_INTERNALS__,
    );
  });
  const mock = await browser.tauri.mock("ipc_dispatch");
  await mock.mockImplementation(async (args) => {
    const request = args.request;
    const fixtures = globalThis.__HELIX_E2E_RESPONSES__;
    if (Object.hasOwn(fixtures, request.command)) {
      return {
        correlation_id: request.correlation_id,
        result: fixtures[request.command],
        error: null,
      };
    }
    return globalThis.__HELIX_E2E_ORIGINAL_INVOKE__("ipc_dispatch", args);
  });
  return async () => {
    await mock.mockRestore();
    await browser.execute(() => {
      delete globalThis.__HELIX_E2E_RESPONSES__;
      delete globalThis.__HELIX_E2E_ORIGINAL_INVOKE__;
    });
  };
}

export async function captureFailure(title, directory) {
  const name = title.replace(/[^a-zA-Z0-9_-]/g, "_");
  const captures = [
    ["screenshot", () => browser.saveScreenshot(join(directory, `${name}.png`))],
    ["page", async () => writeFile(join(directory, `${name}.html`), await browser.getPageSource())],
    [
      "kernel",
      async () =>
        writeFile(
          join(directory, `${name}-kernel.json`),
          JSON.stringify(await invokeKernel("log.query", { query: { limit: 200 } }), null, 2),
        ),
    ],
  ];
  for (const [kind, capture] of captures) {
    try {
      await capture();
    } catch (error) {
      await writeFile(join(directory, `${name}-${kind}-capture-error.txt`), String(error));
    }
  }
}

export async function closeCleanly() {
  await browser.tauri.execute(({ core }) => {
    setTimeout(() => {
      void core.invoke("window_close", { id: "main" });
    }, 100);
  });
  let report;
  await browser.waitUntil(
    async () => {
      try {
        report = JSON.parse(await readFile(process.env.HELIX_E2E_SHUTDOWN_REPORT, "utf8"));
      } catch (error) {
        if (error.code === "ENOENT") return false;
        throw error;
      }
      return report.kernelStopped;
    },
    { timeout: 15_000, timeoutMsg: "Host did not acknowledge clean kernel shutdown" },
  );
  await browser.waitUntil(
    () => {
      try {
        process.kill(report.hostPid, 0);
        return false;
      } catch (error) {
        if (error.code === "ESRCH") return true;
        throw error;
      }
    },
    { timeout: 15_000, timeoutMsg: "Host remained alive after shutdown" },
  );
  browser.overwriteCommand("deleteSession", async () => undefined);
}
