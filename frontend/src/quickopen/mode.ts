export type QuickOpenMode = "files" | "documentSymbols" | "workspaceSymbols" | "line" | "commands";

export interface ParsedQuickOpenQuery {
  mode: QuickOpenMode;
  query: string;
}

export function parseQuickOpenQuery(value: string): ParsedQuickOpenQuery {
  const prefix = value[0];
  if (prefix === "@") return { mode: "documentSymbols", query: value.slice(1).trimStart() };
  if (prefix === "#") return { mode: "workspaceSymbols", query: value.slice(1).trimStart() };
  if (prefix === ":") return { mode: "line", query: value.slice(1).trim() };
  if (prefix === ">") return { mode: "commands", query: value.slice(1).trimStart() };
  return { mode: "files", query: value };
}

export function parseLineNumber(query: string): number | null {
  if (!/^\d+$/.test(query)) return null;
  const line = Number(query);
  return Number.isSafeInteger(line) && line > 0 ? line : null;
}
