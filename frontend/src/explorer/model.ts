import { create } from "zustand";
import type { FileEntry } from "../generated/FileEntry";
import { useLayoutStore } from "../workbench/layoutStore";

export const parentPath = (path: string) =>
  path.replace(/\\/g, "/").replace(/\/[^/]+\/?$/, "") || "/";
export interface TreeRow {
  entry: FileEntry;
  depth: number;
  root: string;
}
export function flattenTree(
  roots: FileEntry[],
  children: Map<string, FileEntry[]>,
  expanded: Set<string>,
): TreeRow[] {
  const rows: TreeRow[] = [];
  const stack = roots.map((entry) => ({ entry, depth: 0, root: entry.path })).reverse();
  while (stack.length) {
    const row = stack.pop()!;
    rows.push(row);
    if (!expanded.has(row.entry.path)) continue;
    const items = children.get(row.entry.path) ?? [];
    for (let i = items.length - 1; i >= 0; i--)
      stack.push({ entry: items[i]!, depth: row.depth + 1, root: row.root });
  }
  return rows;
}
export const useExplorerStore = create<{
  reveal: { path: string; sequence: number } | null;
  activeFiles: Record<string, string | undefined>;
  diagnostics: Record<string, number>;
}>(() => ({ reveal: null, activeFiles: {}, diagnostics: {} }));

export function revealInExplorer(path?: string) {
  const store = useExplorerStore.getState();
  const target = path ?? store.activeFiles[useLayoutStore.getState().activeEditorGroup];
  useLayoutStore.getState().setActiveActivity("explorer");
  if (target)
    useExplorerStore.setState({
      reveal: { path: target, sequence: (store.reveal?.sequence ?? 0) + 1 },
    });
}

/** Diagnostics providers publish counts independently of any VCS provider. */
export function setExplorerDiagnostics(diagnostics: Record<string, number>) {
  useExplorerStore.setState({ diagnostics });
}
