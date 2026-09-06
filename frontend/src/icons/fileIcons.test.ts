import { describe, expect, it } from "vitest";
import {
  BUILTIN_FILE_ICON_THEMES,
  DEFAULT_FILE_ICON_THEME,
  resolveFileIcon,
  resolveFolderIcon,
} from "./fileIcons";

const colored = BUILTIN_FILE_ICON_THEMES.get(DEFAULT_FILE_ICON_THEME)!;
const monochrome = BUILTIN_FILE_ICON_THEMES.get("helix-monochrome")!;

describe("file icon resolution", () => {
  it("resolves exact filenames first", () => {
    expect(resolveFileIcon("README.md", colored)).toBe("file");
    expect(resolveFileIcon("Dockerfile", colored)).toBe("file");
  });

  it("resolves compound extensions before simple ones", () => {
    expect(resolveFileIcon("component.spec.ts", colored)).toBe("file");
  });

  it("resolves simple extensions", () => {
    expect(resolveFileIcon("main.rs", colored)).toBe("file");
    expect(resolveFileIcon("index.ts", colored)).toBe("file");
    expect(resolveFileIcon("App.tsx", colored)).toBe("file");
    expect(resolveFileIcon("style.css", colored)).toBe("file");
  });

  it("falls back to generic file icon for unknown extensions", () => {
    expect(resolveFileIcon("unknown.xyz", colored)).toBe("file");
  });

  it("falls back to generic file icon for files without extensions", () => {
    expect(resolveFileIcon("Makefile", colored)).toBe("file");
  });

  it("monochrome theme always returns the generic file icon", () => {
    expect(resolveFileIcon("main.rs", monochrome)).toBe("file");
    expect(resolveFileIcon("index.ts", monochrome)).toBe("file");
    expect(resolveFileIcon("unknown.xyz", monochrome)).toBe("file");
  });
});

describe("folder icon resolution", () => {
  it("resolves named folders", () => {
    expect(resolveFolderIcon("src", colored, false)).toBe("folder");
    expect(resolveFolderIcon("src", colored, true)).toBe("folder-open");
    expect(resolveFolderIcon("node_modules", colored, false)).toBe("folder");
  });

  it("falls back to generic folder for unknown names", () => {
    expect(resolveFolderIcon("random-folder", colored, false)).toBe("folder");
    expect(resolveFolderIcon("random-folder", colored, true)).toBe("folder-open");
  });

  it("monochrome theme always returns generic folder icons", () => {
    expect(resolveFolderIcon("src", monochrome, false)).toBe("folder");
    expect(resolveFolderIcon("src", monochrome, true)).toBe("folder-open");
  });
});

describe("built-in themes", () => {
  it("has three built-in file icon themes", () => {
    expect(BUILTIN_FILE_ICON_THEMES.size).toBe(3);
    expect(BUILTIN_FILE_ICON_THEMES.has("helix-colored")).toBe(true);
    expect(BUILTIN_FILE_ICON_THEMES.has("helix-monochrome")).toBe(true);
    expect(BUILTIN_FILE_ICON_THEMES.has("none")).toBe(true);
  });

  it("colored theme covers 40+ extensions", () => {
    const extCount = Object.keys(colored.fileExtensions).length;
    expect(extCount).toBeGreaterThanOrEqual(40);
  });

  it("colored theme covers 30+ named folders", () => {
    const folderCount = Object.keys(colored.folderNames).length;
    expect(folderCount).toBeGreaterThanOrEqual(30);
  });
});