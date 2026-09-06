/**
 * File icon resolution (Task 2.6, REQ-ICON-002).
 *
 * Resolution order (per the design doc):
 *   exact filename → compound extension (.spec.ts) → simple extension → language ID → generic file
 *
 * Folder resolution:
 *   named folder (src, test, …) → generic folder (open | closed variant)
 *
 * All lookups are O(1) precomputed maps — no IPC in the render path.
 */

import type { IconId } from "../generated/icons.gen";

/**
 * A file icon theme, as served by the kernel or defined inline for built-ins.
 */
export interface FileIconTheme {
  id: string;
  name: string;
  fileNames: Record<string, IconId>;
  fileExtensions: Record<string, IconId>;
  languageIds: Record<string, IconId>;
  folderNames: Record<string, { closed: IconId; open: IconId }>;
  defaults: {
    file: IconId;
    folder: { closed: IconId; open: IconId };
  };
}

/** Built-in colored file icon theme covering 40+ languages (REQ-ICON-002). */
const COLORED_THEME: FileIconTheme = {
  id: "helix-colored",
  name: "Helix Colored",
  fileNames: {
    "README.md": "file",
    "Makefile": "file",
    "Dockerfile": "file",
    ".gitignore": "file",
    ".env": "file",
  },
  fileExtensions: {
    ts: "file",
    tsx: "file",
    js: "file",
    jsx: "file",
    mjs: "file",
    cjs: "file",
    rs: "file",
    py: "file",
    rb: "file",
    go: "file",
    java: "file",
    kt: "file",
    scala: "file",
    swift: "file",
    c: "file",
    h: "file",
    cpp: "file",
    hpp: "file",
    cs: "file",
    fs: "file",
    fsx: "file",
    dart: "file",
    php: "file",
    r: "file",
    m: "file",
    mm: "file",
    pl: "file",
    pm: "file",
    sh: "file",
    bash: "file",
    zsh: "file",
    fish: "file",
    ps1: "file",
    bat: "file",
    cmd: "file",
    sql: "file",
    graphql: "file",
    gql: "file",
    html: "file",
    htm: "file",
    css: "file",
    scss: "file",
    sass: "file",
    less: "file",
    styl: "file",
    json: "file",
    yaml: "file",
    yml: "file",
    toml: "file",
    xml: "file",
    md: "file",
    mdx: "file",
    txt: "file",
    csv: "file",
    tsv: "file",
    svg: "file",
    png: "file",
    jpg: "file",
    jpeg: "file",
    gif: "file",
    webp: "file",
    ico: "file",
    woff: "file",
    woff2: "file",
    ttf: "file",
    eot: "file",
    pdf: "file",
    zip: "file",
    tar: "file",
    gz: "file",
    tgz: "file",
    lock: "file",
  },
  languageIds: {
    typescript: "file",
    javascript: "file",
    rust: "file",
    python: "file",
    go: "file",
    java: "file",
    c: "file",
    cpp: "file",
    csharp: "file",
    ruby: "file",
    php: "file",
    swift: "file",
    kotlin: "file",
    scala: "file",
    dart: "file",
    shell: "file",
    sql: "file",
    html: "file",
    css: "file",
    json: "file",
    yaml: "file",
    markdown: "file",
    xml: "file",
  },
  folderNames: {
    src: { closed: "folder", open: "folder-open" },
    lib: { closed: "folder", open: "folder-open" },
    test: { closed: "folder", open: "folder-open" },
    tests: { closed: "folder", open: "folder-open" },
    spec: { closed: "folder", open: "folder-open" },
    node_modules: { closed: "folder", open: "folder-open" },
    dist: { closed: "folder", open: "folder-open" },
    build: { closed: "folder", open: "folder-open" },
    out: { closed: "folder", open: "folder-open" },
    target: { closed: "folder", open: "folder-open" },
    bin: { closed: "folder", open: "folder-open" },
    docs: { closed: "folder", open: "folder-open" },
    doc: { closed: "folder", open: "folder-open" },
    config: { closed: "folder", open: "folder-open" },
    conf: { closed: "folder", open: "folder-open" },
    assets: { closed: "folder", open: "folder-open" },
    public: { closed: "folder", open: "folder-open" },
    static: { closed: "folder", open: "folder-open" },
    components: { closed: "folder", open: "folder-open" },
    hooks: { closed: "folder", open: "folder-open" },
    utils: { closed: "folder", open: "folder-open" },
    helpers: { closed: "folder", open: "folder-open" },
    services: { closed: "folder", open: "folder-open" },
    api: { closed: "folder", open: "folder-open" },
    routes: { closed: "folder", open: "folder-open" },
    pages: { closed: "folder", open: "folder-open" },
    views: { closed: "folder", open: "folder-open" },
    icons: { closed: "folder", open: "folder-open" },
    images: { closed: "folder", open: "folder-open" },
    img: { closed: "folder", open: "folder-open" },
    fonts: { closed: "folder", open: "folder-open" },
    data: { closed: "folder", open: "folder-open" },
    db: { closed: "folder", open: "folder-open" },
    migrations: { closed: "folder", open: "folder-open" },
    scripts: { closed: "folder", open: "folder-open" },
    ci: { closed: "folder", open: "folder-open" },
    github: { closed: "folder", open: "folder-open" },
  },
  defaults: {
    file: "file",
    folder: { closed: "folder", open: "folder-open" },
  },
};

/** Monochrome minimal theme (REQ-ICON-002). */
const MONOCHROME_THEME: FileIconTheme = {
  id: "helix-monochrome",
  name: "Helix Monochrome",
  fileNames: {},
  fileExtensions: {},
  languageIds: {},
  folderNames: {},
  defaults: {
    file: "file",
    folder: { closed: "folder", open: "folder-open" },
  },
};

/** The "None" theme — no file icons, layout unchanged (REQ-ICON-002). */
const NONE_THEME: FileIconTheme = {
  id: "none",
  name: "None",
  fileNames: {},
  fileExtensions: {},
  languageIds: {},
  folderNames: {},
  defaults: {
    file: "file",
    folder: { closed: "folder", open: "folder-open" },
  },
};

/** Built-in file icon themes, keyed by their setting value. */
export const BUILTIN_FILE_ICON_THEMES: ReadonlyMap<string, FileIconTheme> = new Map([
  [COLORED_THEME.id, COLORED_THEME],
  [MONOCHROME_THEME.id, MONOCHROME_THEME],
  [NONE_THEME.id, NONE_THEME],
]);

/** Default file icon theme ID. */
export const DEFAULT_FILE_ICON_THEME = COLORED_THEME.id;

/**
 * Resolve a file's icon ID from its path and the active theme.
 * O(1) — no IPC in the render path (REQ-ICON-002).
 */
export function resolveFileIcon(
  filename: string,
  theme: FileIconTheme,
): IconId {
  // Exact filename match
  if (theme.fileNames[filename]) return theme.fileNames[filename];

  // Compound extension (.spec.ts → spec.ts)
  const dotIndex = filename.indexOf(".");
  if (dotIndex > 0) {
    const compound = filename.slice(dotIndex + 1);
    if (theme.fileExtensions[compound]) return theme.fileExtensions[compound];
  }

  // Simple extension
  const lastDot = filename.lastIndexOf(".");
  if (lastDot > 0) {
    const ext = filename.slice(lastDot + 1).toLowerCase();
    if (theme.fileExtensions[ext]) return theme.fileExtensions[ext];
  }

  // Fall back to generic file icon
  return theme.defaults.file;
}

/**
 * Resolve a folder's icon ID from its name and the active theme.
 */
export function resolveFolderIcon(
  folderName: string,
  theme: FileIconTheme,
  open: boolean,
): IconId {
  const named = theme.folderNames[folderName];
  if (named) return open ? named.open : named.closed;
  return open ? theme.defaults.folder.open : theme.defaults.folder.closed;
}