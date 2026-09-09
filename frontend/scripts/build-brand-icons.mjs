// The full-color brand is separate from the monochrome product-icon sprite.
import { copyFileSync, mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const frontend = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = join(frontend, "public/helix-logo.svg");
const desktop = resolve(frontend, "../crates/helix-kernel/icons");
const temporary = mkdtempSync(join(tmpdir(), "helix-brand-icons-"));
const cli = join(frontend, "node_modules/@tauri-apps/cli/tauri.js");
try {
  const generated = join(temporary, "desktop");
  execFileSync(process.execPath, [cli, "icon", source, "--output", generated], {
    stdio: "inherit",
  });
  mkdirSync(desktop, { recursive: true });
  for (const file of readdirSync(generated, { withFileTypes: true })) {
    if (file.isFile()) copyFileSync(join(generated, file.name), join(desktop, file.name));
  }
  copyFileSync(join(generated, "icon.png"), resolve(desktop, "../app-icon.png"));
  copyFileSync(join(generated, "32x32.png"), join(frontend, "public/favicon.png"));
  const web = join(temporary, "web");
  execFileSync(process.execPath, [cli, "icon", source, "--output", web, "--png", "180"], {
    stdio: "inherit",
  });
  copyFileSync(join(web, "180x180.png"), join(frontend, "public/apple-touch-icon.png"));
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
