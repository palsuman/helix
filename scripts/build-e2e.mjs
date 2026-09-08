import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const frontend = join(root, "frontend");
const target = join(root, "target", "e2e");
const env = {
  ...process.env,
  CARGO_TARGET_DIR: target,
  VITE_HELIX_WDIO_E2E: "1",
  VITE_HELIX_IPC_E2E: "0"
};

function run(command, args, cwd = root) {
  const result = spawnSync(command, args, { cwd, env, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run(process.execPath, [join(frontend, "scripts/build-icons.mjs")], frontend);
run(
  process.execPath,
  [join(frontend, "node_modules/vite/bin/vite.js"), "build", "--outDir", "dist-e2e"],
  frontend
);
run("cargo", ["build", "--locked", "-p", "helix-kernel"]);
run(
  process.execPath,
  [
    join(frontend, "node_modules/@tauri-apps/cli/tauri.js"),
    "build",
    "--debug",
    "--no-bundle",
    "--features",
    "e2e",
    "--config",
    "tauri.e2e.conf.json"
  ],
  join(root, "crates/helix-supervisor")
);
