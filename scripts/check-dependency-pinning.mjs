#!/usr/bin/env node
// Task 1.1: CI check rejecting every non-exact external dependency specifier
// in a repository manifest, per the design document's pinning policy.
//
// Scans every package.json (dependencies/devDependencies/optionalDependencies)
// and every Cargo.toml ([dependencies]/[dev-dependencies]/[build-dependencies]
// and their workspace.dependencies equivalents) in the repository.

import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

const REPO_ROOT = new URL("..", import.meta.url).pathname.replace(/^\/([a-zA-Z]):/, "$1:");

/** @returns {string[]} */
function listTrackedFiles(fileName) {
  const out = execFileSync("git", ["ls-files", "-z", "--", fileName, `:(glob)**/${fileName}`], {
    cwd: REPO_ROOT,
    encoding: "utf8",
  });
  return out.split("\0").filter(Boolean);
}

let violations = [];

// ---- package.json ---------------------------------------------------
const packageJsonFiles = listTrackedFiles("package.json").filter(
  (f) => !f.includes("node_modules/"),
);

for (const file of packageJsonFiles) {
  const raw = readFileSync(`${REPO_ROOT}/${file}`, "utf8");
  const json = JSON.parse(raw);
  const depFields = ["dependencies", "devDependencies", "optionalDependencies"];
  for (const field of depFields) {
    const deps = json[field];
    if (!deps) continue;
    for (const [name, spec] of Object.entries(deps)) {
      if (typeof spec !== "string") continue; // workspace:* protocol objects, etc.
      if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(spec)) {
        violations.push(`${file}: ${field}.${name} = "${spec}" (not an exact version)`);
      }
    }
  }
}

// ---- Cargo.toml -------------------------------------------------------
const cargoTomlFiles = listTrackedFiles("Cargo.toml");

function extractDepSpecs(tomlText) {
  const specs = [];
  const lines = tomlText.split("\n");
  let inDepSection = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (/^\[.*\]$/.test(trimmed)) {
      inDepSection = /dependencies/i.test(trimmed);
      continue;
    }
    if (!inDepSection || !trimmed || trimmed.startsWith("#")) continue;
    // name = "1.2.3"  OR  name = { version = "1.2.3", ... }
    const simple = trimmed.match(/^([A-Za-z0-9_-]+)\s*=\s*"([^"]+)"/);
    const table = trimmed.match(/^([A-Za-z0-9_-]+)\s*=\s*\{.*version\s*=\s*"([^"]+)"/);
    const match = simple || table;
    if (match) {
      specs.push({ name: match[1], version: match[2] });
    } else if (/^[A-Za-z0-9_-]+\s*=\s*\{/.test(trimmed) && !trimmed.includes("path")) {
      // Table dependency without an inline version on this line is fine only
      // if it uses `path` (workspace-local crate); anything else is flagged
      // as unparseable so it doesn't silently skip validation.
      if (!trimmed.includes("workspace = true")) {
        specs.push({ name: trimmed.split("=")[0].trim(), version: null, raw: trimmed });
      }
    }
  }
  return specs;
}

for (const file of cargoTomlFiles) {
  const raw = readFileSync(`${REPO_ROOT}/${file}`, "utf8");
  const specs = extractDepSpecs(raw);
  for (const { name, version, raw: rawLine } of specs) {
    if (version === null) {
      // path-based or workspace deps without inline version are fine;
      // anything else unparsed is worth a human look but not a hard fail
      // here since `path = "..."` deps are the workspace-local crates.
      if (rawLine && !rawLine.includes("path")) {
        violations.push(`${file}: ${name} has no parseable version and is not path-based (${rawLine})`);
      }
      continue;
    }
    if (!/^=\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
      violations.push(`${file}: ${name} = "${version}" (Cargo dependencies require =x.y.z)`);
    }
  }
}

if (violations.length > 0) {
  console.error("Dependency pinning policy violations found:\n");
  for (const v of violations) console.error(`  - ${v}`);
  console.error(
    "\nSee design.md 'Dependency pinning policy': npm dependencies require x.y.z and Cargo dependencies require =x.y.z.",
  );
  process.exit(1);
}

console.log(`Dependency pinning check passed (${packageJsonFiles.length} package.json, ${cargoTomlFiles.length} Cargo.toml).`);
