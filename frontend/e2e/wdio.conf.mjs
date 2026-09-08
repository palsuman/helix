import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { captureFailure } from "./support.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const artifacts = join(root, "target", "e2e-artifacts");
mkdirSync(artifacts, { recursive: true });
process.env.HELIX_E2E_HOME ??= mkdtempSync(join(artifacts, "run-"));
const home = process.env.HELIX_E2E_HOME;
const logs = join(home, "logs");
mkdirSync(logs, { recursive: true });
process.env.HELIX_E2E_SHUTDOWN_REPORT = join(home, "shutdown.json");

if (!process.env.HELIX_E2E_PORT) {
  const server = createServer();
  await new Promise((resolveListen, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolveListen);
  });
  process.env.HELIX_E2E_PORT = String(server.address().port);
  await new Promise((resolveClose, reject) =>
    server.close((error) => (error ? reject(error) : resolveClose())),
  );
}

process.env.TAURI_WEBDRIVER_PORT = process.env.HELIX_E2E_PORT;
const binary = join(
  root,
  "target/e2e/debug",
  `helix-supervisor${process.platform === "win32" ? ".exe" : ""}`,
);
export const config = {
  runner: "local",
  specs: ["./workbench.e2e.mjs"],
  maxInstances: 1,
  logLevel: "info",
  outputDir: logs,
  framework: "mocha",
  reporters: ["spec"],
  waitforTimeout: 15_000,
  connectionRetryTimeout: 30_000,
  connectionRetryCount: 0,
  mochaOpts: { timeout: 60_000 },
  capabilities: [{ browserName: "tauri", "wdio:tauriServiceOptions": { appBinaryPath: binary } }],
  services: [
    [
      "@wdio/tauri-service",
      {
        appBinaryPath: binary,
        driverProvider: "embedded",
        embeddedPort: Number(process.env.HELIX_E2E_PORT),
        autoInstallTauriDriver: false,
        autoDownloadEdgeDriver: false,
        captureBackendLogs: true,
        captureFrontendLogs: true,
        logDir: logs,
        startTimeout: 60_000,
        env: {
          HOME: home,
          USERPROFILE: home,
          APPDATA: home,
          LOCALAPPDATA: home,
          XDG_CONFIG_HOME: home,
          XDG_DATA_HOME: home,
          XDG_STATE_HOME: home,
          XDG_CACHE_HOME: home,
          HELIX_KERNEL_BIN: join(
            root,
            "target/e2e/debug",
            `helix-kernel${process.platform === "win32" ? ".exe" : ""}`,
          ),
          HELIX_E2E_SHUTDOWN_REPORT: process.env.HELIX_E2E_SHUTDOWN_REPORT,
        },
      },
    ],
  ],
  async afterTest(test, _context, { passed }) {
    if (!passed) await captureFailure(test.title, logs);
  },
  onComplete(exitCode) {
    if (exitCode === 0) {
      for (const directory of ["Library", "Helix", "helix", ".config", ".local", ".cache"]) {
        rmSync(join(home, directory), { recursive: true, force: true });
      }
    }
  },
};
