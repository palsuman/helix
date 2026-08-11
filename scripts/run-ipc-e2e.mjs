import { spawn, spawnSync } from "node:child_process";
import { readFile, rm } from "node:fs/promises";
import { arch, platform } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const frontend = join(root, "frontend");
const supervisor = join(root, "crates", "helix-supervisor");
const target = join(root, "target", "release");
const reportPath = join(root, "target", "ipc-e2e-report.json");
const tauri = join(frontend, "node_modules", ".bin", platform() === "win32" ? "tauri.cmd" : "tauri");

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run(process.execPath, [join(root, "scripts", "prepare-kernel-sidecar.mjs"), "release"]);
run(tauri, ["build", "--features", "ipc-e2e"], {
  cwd: supervisor,
  env: { ...process.env, VITE_HELIX_IPC_E2E: "1" },
});

await rm(reportPath, { force: true });
const executable =
  platform() === "darwin"
    ? join(target, "bundle", "macos", "Helix.app", "Contents", "MacOS", "helix-supervisor")
    : join(target, `helix-supervisor${platform() === "win32" ? ".exe" : ""}`);
const environment = { ...process.env, HELIX_IPC_E2E_REPORT: reportPath };
delete environment.HELIX_KERNEL_BIN;

const app = spawn(executable, [], { env: environment, stdio: "inherit" });
const exitCode = await new Promise((resolveExit, reject) => {
  const timeout = setTimeout(() => {
    app.kill("SIGKILL");
    reject(new Error("IPC E2E application did not finish within 30 seconds"));
  }, 30_000);
  app.once("error", reject);
  app.once("exit", (code) => {
    clearTimeout(timeout);
    resolveExit(code);
  });
});
if (exitCode !== 0) throw new Error(`IPC E2E application exited with code ${exitCode}`);

const report = JSON.parse(await readFile(reportPath, "utf8"));
console.log(JSON.stringify({ platform: platform(), arch: arch(), ...report }, null, 2));
if (!report.passed) process.exit(1);