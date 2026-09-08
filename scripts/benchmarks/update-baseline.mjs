import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { validateReport } from "./report.mjs";

const { values } = parseArgs({
  options: {
    report: { type: "string", default: "target/performance/latest.json" },
    baseline: { type: "string", default: "benchmarks/baseline.json" },
    release: { type: "string" },
    reason: { type: "string" }
  }
});
if (!values.release?.trim() || !values.reason?.trim())
  throw new Error("Baseline promotion requires --release and --reason");
const report = JSON.parse(readFileSync(values.report, "utf8"));
validateReport(report);
writeFileSync(
  values.baseline,
  `${JSON.stringify({ ...report, release: values.release, updateReason: values.reason }, null, 2)}\n`
);
console.log(
  `Promoted ${values.report} to ${values.baseline}; review and commit this release baseline explicitly.`
);
