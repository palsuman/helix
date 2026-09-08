import { mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { resolve, join, dirname } from "node:path";
import { pathToFileURL } from "node:url";

export function generateWorkspace(destination, { files = 50_000, packages = 100 } = {}) {
  if (
    !Number.isSafeInteger(packages) ||
    packages < 1 ||
    !Number.isSafeInteger(files) ||
    files < packages * 2 + 3
  ) {
    throw new Error("Expected integer file/package counts with at least one source per package");
  }
  const root = resolve(destination);
  mkdirSync(root, { recursive: true });
  if (readdirSync(root).length) throw new Error("Refusing to overwrite a non-empty workspace");
  const write = (relative, value) => {
    const path = join(root, relative);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, value, { flag: "wx" });
  };
  write(
    "package.json",
    JSON.stringify({ name: "helix-reference", private: true, workspaces: ["packages/*"] }, null, 2)
  );
  write("nx.json", '{"defaultBase":"main"}\n');
  for (let index = 0; index < packages; index++) {
    write(
      `packages/package-${index}/package.json`,
      JSON.stringify(
        {
          name: `@reference/package-${index}`,
          version: "1.0.0",
          private: true,
          dependencies: index ? { [`@reference/package-${index - 1}`]: "1.0.0" } : {}
        },
        null,
        2
      )
    );
  }
  const sourceFiles = files - packages - 3;
  for (let index = 0; index < sourceFiles; index++) {
    const packageIndex = index % packages;
    write(
      `packages/package-${packageIndex}/src/module-${index}.ts`,
      `export const module${index} = { package: ${packageIndex}, value: ${index} };\n` +
        "// Deterministic reference source for search, indexing, and file benchmarks.\n".repeat(12)
    );
  }
  const manifest = {
    schemaVersion: 1,
    generator: "helix-reference-v1",
    files,
    packages,
    sourceFiles
  };
  write(".helix-benchmark.json", JSON.stringify(manifest, null, 2));
  return manifest;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (process.argv.length > 3) throw new Error("Usage: generate-workspace.mjs [destination]");
  console.log(
    JSON.stringify(generateWorkspace(process.argv[2] ?? "target/reference-workspace"), null, 2)
  );
}
