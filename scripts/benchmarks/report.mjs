export const cases = [
  "container_startup",
  "ipc_round_trip",
  "file_read",
  "file_write",
  "config_merge"
];
export const protocol = "helix-platform-v1-criterion-0.8.2-30samples-1s-warmup-3s-measure";

export function validateReport(report) {
  if (report.schemaVersion !== 1 || report.protocol !== protocol)
    throw new Error("Unsupported benchmark report protocol");
  for (const key of ["platform", "arch", "cpu", "cpuCount", "osRelease", "rustc", "runner"]) {
    if (!report.environment?.[key]) throw new Error(`Missing environment ${key}`);
  }
  if (
    JSON.stringify(Object.keys(report.benchmarks ?? {}).sort()) !==
    JSON.stringify([...cases].sort())
  ) {
    throw new Error("Benchmark set is incomplete or unexpected");
  }
  for (const name of cases) {
    const measurement = report.benchmarks[name];
    for (const key of ["meanNs", "lowerNs", "upperNs", "peakRssBytes"]) {
      if (!Number.isFinite(measurement[key]) || measurement[key] <= 0)
        throw new Error(`Invalid ${name}.${key}`);
    }
    if (measurement.lowerNs > measurement.meanNs || measurement.upperNs < measurement.meanNs) {
      throw new Error(`Invalid confidence interval for ${name}`);
    }
  }
}

export function compareReports(baseline, current) {
  validateReport(baseline);
  validateReport(current);
  for (const key of Object.keys(baseline.environment)) {
    if (baseline.environment[key] !== current.environment[key])
      throw new Error(
        `Runner mismatch for ${key}: ${baseline.environment[key]} != ${current.environment[key]}`
      );
  }
  return cases.flatMap((name) =>
    ["meanNs", "peakRssBytes"].map((metric) => {
      const before = baseline.benchmarks[name][metric];
      const after = current.benchmarks[name][metric];
      return {
        name,
        metric,
        baseline: before,
        current: after,
        deltaPercent: (after / before - 1) * 100,
        passed: after <= before * 1.1
      };
    })
  );
}
