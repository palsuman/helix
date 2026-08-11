import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const profile = process.argv[2] === "debug" ? "debug" : "release";
const cargoArgs = ["build", "-p", "helix-kernel"];
if (profile === "release") cargoArgs.push("--release");

const build = spawnSync("cargo", cargoArgs, { cwd: root, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

const rustc = spawnSync("rustc", ["-vV"], {
  cwd: root,
  encoding: "utf8",
});
if (rustc.status !== 0) {
  process.stderr.write(rustc.stderr);
  process.exit(rustc.status ?? 1);
}
const host = rustc.stdout
  .split(/\r?\n/)
  .find((line) => line.startsWith("host: "))
  ?.slice("host: ".length);
if (!host) throw new Error("rustc did not report its host target triple");

const executableSuffix = process.platform === "win32" ? ".exe" : "";
const source = join(root, "target", profile, `helix-kernel${executableSuffix}`);
const binaries = join(root, "crates", "helix-supervisor", "binaries");
const destination = join(
  binaries,
  `helix-kernel-${host}${executableSuffix}`,
);
mkdirSync(binaries, { recursive: true });
copyFileSync(source, destination);
console.log(`Prepared Tauri sidecar: ${destination}`);