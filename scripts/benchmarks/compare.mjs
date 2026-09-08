import { readFileSync, writeFileSync } from "node:fs";
import { compareReports } from "./report.mjs";

const [
  baselinePath = "benchmarks/baseline.json",
  currentPath = "target/performance/latest.json",
  outputPath = "target/performance/comparison.json",
  ...extra
] = process.argv.slice(2);
if (extra.length)
  throw new Error("Usage: compare.mjs [baseline.json] [current.json] [comparison.json]");
const baseline = JSON.parse(readFileSync(baselinePath, "utf8"));
const current = JSON.parse(readFileSync(currentPath, "utf8"));
try {
  const results = compareReports(baseline, current);
  const passed = results.every((result) => result.passed);
  writeFileSync(
    outputPath,
    `${JSON.stringify({ passed, thresholdPercent: 10, baselineRelease: baseline.release, results }, null, 2)}\n`
  );
  for (const result of results) {
    console.log(
      `${result.passed ? "PASS" : "FAIL"} ${result.name} ${result.metric}: ${result.baseline.toFixed(2)} -> ${result.current.toFixed(2)} (${result.deltaPercent >= 0 ? "+" : ""}${result.deltaPercent.toFixed(2)}%)`
    );
  }
  if (!passed) process.exitCode = 1;
} catch (error) {
  writeFileSync(
    outputPath,
    `${JSON.stringify({ passed: false, error: error.message }, null, 2)}\n`
  );
  throw error;
}
