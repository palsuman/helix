import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";

const check = process.argv.includes("--check");
const root = resolve(import.meta.dirname, "..");
const catalogPath = join(root, "src/localization/locales/en.json");
const temp = mkdtempSync(join(tmpdir(), "helix-i18n-"));
const extractedPath = join(temp, "en.json");

try {
  execFileSync(
    process.execPath,
    [
      join(root, "node_modules/@formatjs/cli/bin/formatjs"),
      "extract",
      join(root, "src/localization/messages.ts"),
      "--format",
      "simple",
      "--out-file",
      extractedPath,
    ],
    { stdio: "inherit" },
  );
  const extracted = JSON.parse(readFileSync(extractedPath, "utf8"));
  const output = `${JSON.stringify(Object.fromEntries(Object.entries(extracted).sort()), null, 2)}\n`;
  if (check) {
    const current = readFileSync(catalogPath, "utf8");
    if (current !== output) {
      console.error("English catalog is out of date. Run npm run i18n:extract.");
      process.exitCode = 1;
    }
  } else {
    writeFileSync(catalogPath, output);
    console.log(`Extracted ${Object.keys(extracted).length} messages to ${catalogPath}`);
  }
} finally {
  rmSync(temp, { recursive: true, force: true });
}
