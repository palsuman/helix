import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { cases, protocol, compareReports } from "./report.mjs";
import { generateWorkspace } from "./generate-workspace.mjs";

function report() {
  return {
    schemaVersion: 1,
    protocol,
    environment: {
      platform: "test",
      arch: "test",
      cpu: "test",
      cpuCount: 1,
      osRelease: "test",
      rustc: "test",
      runner: "test"
    },
    benchmarks: Object.fromEntries(
      cases.map((name) => [name, { meanNs: 100, lowerNs: 99, upperNs: 101, peakRssBytes: 1000 }])
    )
  };
}

test("gate permits exactly 10% and rejects larger timing and RSS regressions", () => {
  const baseline = report();
  const current = report();
  current.benchmarks.file_read = { meanNs: 110, lowerNs: 109, upperNs: 111, peakRssBytes: 1100 };
  assert.ok(compareReports(baseline, current).every((result) => result.passed));
  current.benchmarks.file_read.meanNs = 110.01;
  current.benchmarks.file_read.peakRssBytes = 1101;
  assert.equal(compareReports(baseline, current).filter((result) => !result.passed).length, 2);
});

test("gate rejects missing cases, invalid measurements, and incomparable runners", () => {
  const baseline = report();
  const missing = report();
  delete missing.benchmarks.file_read;
  assert.throws(() => compareReports(baseline, missing), /incomplete/);
  for (const value of [0, -1, NaN, Infinity]) {
    const invalid = report();
    invalid.benchmarks.file_read.meanNs = value;
    assert.throws(() => compareReports(baseline, invalid), /Invalid/);
  }
  const different = report();
  different.environment.cpu = "another runner";
  assert.throws(() => compareReports(baseline, different), /Runner mismatch/);
  different.protocol = "future";
  assert.throws(() => compareReports(baseline, different), /protocol/);
});

test("generator creates exact deterministic monorepo contents and refuses overwrites", () => {
  const temporary = mkdtempSync(join(tmpdir(), "helix-reference-"));
  try {
    const first = join(temporary, "first");
    const second = join(temporary, "second");
    const options = { files: 37, packages: 3 };
    assert.deepEqual(generateWorkspace(first, options), generateWorkspace(second, options));
    const files = readdirSync(first, { recursive: true, withFileTypes: true }).filter((entry) =>
      entry.isFile()
    );
    assert.equal(files.length, 37);
    for (const entry of files) {
      const relative = join(entry.parentPath.slice(first.length), entry.name);
      assert.deepEqual(readFileSync(join(first, relative)), readFileSync(join(second, relative)));
    }
    const manifest = JSON.parse(readFileSync(join(first, "packages/package-2/package.json")));
    assert.equal(manifest.dependencies["@reference/package-1"], "1.0.0");
    assert.throws(() => generateWorkspace(first, options), /overwrite/);
    assert.throws(
      () => generateWorkspace(join(temporary, "invalid"), { files: 2, packages: 3 }),
      /integer/
    );
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});

test("comparison CLI returns failure and writes diagnostic deltas for CI", () => {
  const temporary = mkdtempSync(join(tmpdir(), "helix-performance-gate-"));
  try {
    const baselinePath = join(temporary, "baseline.json");
    const currentPath = join(temporary, "current.json");
    const outputPath = join(temporary, "comparison.json");
    const current = report();
    current.benchmarks.file_read = { meanNs: 120, lowerNs: 119, upperNs: 121, peakRssBytes: 1000 };
    writeFileSync(baselinePath, JSON.stringify(report()));
    writeFileSync(currentPath, JSON.stringify(current));
    const compare = fileURLToPath(new URL("./compare.mjs", import.meta.url));
    const result = spawnSync(process.execPath, [compare, baselinePath, currentPath, outputPath], {
      encoding: "utf8"
    });
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stdout, /FAIL file_read meanNs/);
    const output = JSON.parse(readFileSync(outputPath, "utf8"));
    assert.equal(output.passed, false);
    assert.equal(output.results.filter((entry) => !entry.passed).length, 1);
    current.environment.cpu = "wrong machine";
    writeFileSync(currentPath, JSON.stringify(current));
    const incompatible = spawnSync(
      process.execPath,
      [compare, baselinePath, currentPath, outputPath],
      { encoding: "utf8" }
    );
    assert.equal(incompatible.status, 1);
    assert.match(JSON.parse(readFileSync(outputPath, "utf8")).error, /Runner mismatch/);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});
