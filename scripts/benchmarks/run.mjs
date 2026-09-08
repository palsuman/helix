import { spawnSync } from "node:child_process";
import { cpus, platform, arch, release } from "node:os";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cases, protocol, validateReport } from "./report.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
if (process.argv.length > 3) throw new Error("Usage: run.mjs [report.json]");
const output = resolve(process.argv[2] ?? join(root, "target/performance/latest.json"));
mkdirSync(dirname(output), { recursive: true });
const runDirectory = mkdtempSync(join(dirname(output), "run-"));
const criterionDirectory = join(runDirectory, "criterion");

function command(executable, args, options = {}) {
  const result = spawnSync(executable, args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    ...options
  });
  if (result.error) throw result.error;
  if (result.status !== 0)
    throw new Error(`${executable} ${args.join(" ")} failed: ${result.stderr ?? ""}`);
  return result.stdout?.trim();
}

const environment = {
  platform: platform(),
  arch: arch(),
  cpu: cpus()[0].model,
  cpuCount: cpus().length,
  osRelease: release(),
  rustc: command("rustc", ["--version"]),
  runner: process.env.HELIX_BENCH_RUNNER ?? "reference-v1"
};
const build = command("cargo", [
  "bench",
  "-p",
  "helix-bench",
  "--bench",
  "platform",
  "--locked",
  "--no-run",
  "--message-format=json"
]);
const artifacts = build
  .split(/\r?\n/)
  .map((line) => JSON.parse(line))
  .filter(
    (message) =>
      message.reason === "compiler-artifact" &&
      message.target.name === "platform" &&
      message.executable
  );
if (artifacts.length !== 1) throw new Error("Expected exactly one Criterion executable");
const measurements = {};
for (const name of cases) {
  console.log(`Measuring ${name} in a fresh process`);
  const rssPath = join(runDirectory, `${name}-rss.json`);
  const result = spawnSync(artifacts[0].executable, ["--bench", "--noplot", "--color", "never"], {
    cwd: root,
    encoding: "utf8",
    timeout: 120_000,
    maxBuffer: 8 * 1024 * 1024,
    env: {
      ...process.env,
      HELIX_BENCH_CASE: name,
      HELIX_BENCH_RSS: rssPath,
      CRITERION_HOME: criterionDirectory
    }
  });
  writeFileSync(
    join(runDirectory, `${name}.log`),
    `${result.stdout ?? ""}\n${result.stderr ?? ""}`
  );
  if (result.error) throw result.error;
  if (result.status !== 0)
    throw new Error(`${name} failed (${result.status}); see ${runDirectory}`);
  const { mean } = JSON.parse(
    readFileSync(join(criterionDirectory, name, "new/estimates.json"), "utf8")
  );
  const memory = JSON.parse(readFileSync(rssPath, "utf8"));
  if (memory.benchmark !== name) throw new Error(`Mismatched RSS result for ${name}`);
  measurements[name] = {
    meanNs: mean.point_estimate,
    lowerNs: mean.confidence_interval.lower_bound,
    upperNs: mean.confidence_interval.upper_bound,
    peakRssBytes: memory.peakRssBytes
  };
}
const report = {
  schemaVersion: 1,
  protocol,
  recordedAt: new Date().toISOString(),
  revision: command("git", ["rev-parse", "HEAD"]),
  dirty: Boolean(command("git", ["status", "--porcelain"])),
  environment,
  benchmarks: measurements
};
validateReport(report);
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
console.log(`Report: ${output}\nRaw samples and logs: ${runDirectory}`);
