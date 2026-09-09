import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { IpcClient } from "../ipc";
import type { StreamClient } from "../stream";
import type { FileEntry } from "../generated/FileEntry";
import type { ExplorerPageResponse } from "../generated/ExplorerPageResponse";
import type { ExplorerMutationRequest } from "../generated/ExplorerMutationRequest";
import {
  Icon,
  BUILTIN_FILE_ICON_THEMES,
  DEFAULT_FILE_ICON_THEME,
  resolveFileIcon,
  resolveFolderIcon,
} from "../icons";
import { useMessage } from "../localization";
import { listWorkspaces } from "../workbench/commands";
import { ClosePrimaryPanelButton } from "../workbench/ClosePrimaryPanelButton";
import { useLayoutStore } from "../workbench/layoutStore";
import { navigateEditor } from "../editor/navigation";
import { flattenTree, parentPath, useExplorerStore, type TreeRow } from "./model";
import "./explorer.css";

const ROW_HEIGHT = 28;
const theme = BUILTIN_FILE_ICON_THEMES.get(DEFAULT_FILE_ICON_THEME)!;
type Edit = { operation: string; parent: string; root: string; paths: string[]; value: string };

export function FileExplorer({
  client,
  streamClient,
}: {
  client: IpcClient;
  streamClient?: StreamClient;
}) {
  const t = useMessage();
  const [roots, setRoots] = useState<FileEntry[]>([]);
  const [children, setChildren] = useState(new Map<string, FileEntry[]>());
  const [expanded, setExpanded] = useState(new Set<string>());
  const [selected, setSelected] = useState(new Set<string>());
  const [focused, setFocused] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ row: TreeRow; x: number; y: number } | null>(null);
  const [edit, setEdit] = useState<Edit | null>(null);
  const [deleting, setDeleting] = useState<TreeRow[] | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(500);
  const [revision, setRevision] = useState(0);
  const viewport = useRef<HTMLDivElement>(null);
  const anchor = useRef<string | null>(null);
  const dragging = useRef<TreeRow[]>([]);
  const generation = useRef(0);
  const expandedRef = useRef(expanded);
  const lastFilter = useRef(filter);
  const confirmDialog = useRef<HTMLDialogElement>(null);
  const reveal = useExplorerStore((s) => s.reveal);
  const diagnostics = useExplorerStore((s) => s.diagnostics);
  const rows = useMemo(() => flattenTree(roots, children, expanded), [roots, children, expanded]);
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 5);
  const visible = rows.slice(start, start + Math.ceil(height / ROW_HEIGHT) + 10);
  const selectedRows = useMemo(
    () => rows.filter((r) => selected.has(r.entry.path)),
    [rows, selected],
  );
  const refresh = () => setRevision((n) => n + 1);

  useEffect(() => {
    expandedRef.current = expanded;
  }, [expanded]);
  useEffect(() => {
    if (!deleting || !confirmDialog.current) return;
    const dialog = confirmDialog.current;
    const tree = viewport.current;
    if (dialog.showModal) dialog.showModal();
    else dialog.setAttribute("open", "");
    return () => {
      if (dialog.close) dialog.close();
      tree?.focus();
    };
  }, [deleting]);

  useEffect(() => {
    if (!menu) return;
    const element = document.querySelector<HTMLElement>(".explorer-menu");
    element?.querySelector<HTMLButtonElement>("button")?.focus();
    const dismiss = (event: PointerEvent) => {
      if (!element?.contains(event.target as Node)) setMenu(null);
    };
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, [menu]);

  const load = useCallback(
    async (root: string, path: string, query = "") => {
      let offset = 0;
      let snapshot: string | null = null;
      const entries: FileEntry[] = [];
      let more: boolean;
      do {
        const page: ExplorerPageResponse = await client.invoke("explorer.page", {
          root,
          path,
          filter: query,
          offset,
          snapshot,
        });
        entries.push(...page.entries);
        snapshot = page.snapshot;
        offset += page.entries.length;
        if (page.unreadable_paths.length)
          setError(t("explorerUnreadable", { count: page.unreadable_paths.length }));
        more = offset < page.total && page.entries.length > 0;
      } while (more);
      return entries;
    },
    [client, t],
  );

  useEffect(() => {
    const id = ++generation.current;
    const preserve = lastFilter.current === filter;
    const previousExpanded = preserve ? expandedRef.current : new Set<string>();
    lastFilter.current = filter;
    const timer = setTimeout(
      () => {
        setBusy(true);
        setError(null);
        void listWorkspaces(client)
          .then(async (response) => {
            const roots: FileEntry[] = response.workspaces
              .flatMap((w) => w.roots)
              .filter(
                (r, index, all) =>
                  r.availability === "available" &&
                  all.findIndex((a) => a.path === r.path) === index,
              )
              .map((r) => ({
                path: r.path,
                name: r.name,
                relative_path: "",
                is_dir: true,
                is_symlink: false,
                size: 0n,
                modified_ms: null,
                readonly: false,
              }));
            const next = new Map<string, FileEntry[]>();
            const open = new Set(roots.map((r) => r.path));
            for (const root of roots) {
              const entries = await load(root.path, root.path, filter);
              if (filter) {
                for (const entry of entries) {
                  const parent = parentPath(entry.path);
                  if (!next.has(parent)) next.set(parent, []);
                  next.get(parent)!.push(entry);
                  if (entry.is_dir) open.add(entry.path);
                }
              } else next.set(root.path, entries);
              if (!filter) {
                for (const path of [...previousExpanded].sort()) {
                  if (path === root.path || !path.startsWith(`${root.path}/`)) continue;
                  if (
                    !next
                      .get(parentPath(path))
                      ?.some((e) => e.path === path && e.is_dir && !e.is_symlink)
                  )
                    continue;
                  next.set(path, await load(root.path, path));
                  open.add(path);
                }
              }
            }
            if (generation.current !== id) return;
            setRoots(roots);
            setChildren(next);
            setExpanded(open);
            if (!preserve) {
              setScrollTop(0);
              if (viewport.current) viewport.current.scrollTop = 0;
            }
          })
          .catch((e: unknown) => {
            if (generation.current === id) setError(String(e));
          })
          .finally(() => {
            if (generation.current === id) setBusy(false);
          });
      },
      filter ? 200 : 0,
    );
    return () => {
      generation.current = id + 1;
      clearTimeout(timer);
    };
  }, [client, filter, revision, load]);

  useEffect(() => {
    if (!streamClient) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const changed = () => {
      clearTimeout(timer);
      timer = setTimeout(() => setRevision((n) => n + 1), 200);
    };
    const unsubscribe = streamClient.subscribe("fs:changed", changed);
    const workspaceUnsubscribe = streamClient.subscribe("workspace:changed", changed);
    return () => {
      clearTimeout(timer);
      unsubscribe();
      workspaceUnsubscribe();
    };
  }, [streamClient]);

  useEffect(() => {
    const node = viewport.current;
    if (!node || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => setHeight(entry?.contentRect.height ?? 500));
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const onFocus = () => setRevision((n) => n + 1);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  useEffect(() => {
    if (!reveal || !roots.length || busy) return;
    const root = roots.find((r) => reveal.path.startsWith(`${r.path}/`));
    if (!root) return;
    if (filter) {
      const timer = setTimeout(() => setFilter(""), 0);
      return () => clearTimeout(timer);
    }
    let active = true;
    void (async () => {
      const parents: string[] = [];
      let path = parentPath(reveal.path);
      while (path !== root.path && path.startsWith(`${root.path}/`)) {
        parents.unshift(path);
        path = parentPath(path);
      }
      parents.unshift(root.path);
      const loaded = new Map<string, FileEntry[]>();
      for (const parent of parents) loaded.set(parent, await load(root.path, parent));
      if (!active) return;
      setChildren((current) => new Map([...current, ...loaded]));
      setExpanded((current) => new Set([...current, ...parents]));
      setSelected(new Set([reveal.path]));
      anchor.current = reveal.path;
      useExplorerStore.setState({ reveal: null });
    })().catch((e: unknown) => setError(String(e)));
    return () => {
      active = false;
    };
  }, [reveal, roots, busy, filter, load]);

  useEffect(() => {
    if (!anchor.current || !viewport.current) return;
    const index = rows.findIndex((r) => r.entry.path === anchor.current);
    if (index < 0) return;
    const top = index * ROW_HEIGHT;
    if (
      top < viewport.current.scrollTop ||
      top + ROW_HEIGHT > viewport.current.scrollTop + height
    ) {
      viewport.current.scrollTop = top;
      setScrollTop(top);
    }
  }, [rows, height]);

  const toggle = async (row: TreeRow) => {
    const path = row.entry.path;
    if (!row.entry.is_dir || row.entry.is_symlink) return;
    if (expanded.has(path)) {
      setExpanded((s) => {
        const next = new Set(s);
        next.delete(path);
        return next;
      });
      return;
    }
    try {
      if (!children.has(path)) {
        setBusy(true);
        const entries = await load(row.root, path);
        setChildren((s) => new Map(s).set(path, entries));
      }
      setExpanded((s) => new Set(s).add(path));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const open = (row: TreeRow) => {
    if (row.entry.is_dir) void toggle(row);
    else
      navigateEditor({
        groupId: useLayoutStore.getState().activeEditorGroup,
        path: row.entry.path,
      });
  };
  const select = (row: TreeRow, shift: boolean, additive: boolean) => {
    setFocused(row.entry.path);
    setSelected((current) => {
      const next = additive ? new Set(current) : new Set<string>();
      if (shift && anchor.current) {
        const a = rows.findIndex((r) => r.entry.path === anchor.current);
        const b = rows.indexOf(row);
        for (let i = Math.max(0, Math.min(a, b)); i <= Math.max(a, b); i++)
          next.add(rows[i]!.entry.path);
      } else if (additive && next.has(row.entry.path)) next.delete(row.entry.path);
      else next.add(row.entry.path);
      return next;
    });
    if (!shift) anchor.current = row.entry.path;
  };
  const mutate = async (
    operation: string,
    targets: TreeRow[],
    destination: string | null = null,
    root?: string,
  ) => {
    setBusy(true);
    setError(null);
    setMenu(null);
    try {
      const groups = new Map<string, string[]>();
      if (root) groups.set(root, []);
      for (const row of targets)
        groups.set(row.root, [...(groups.get(row.root) ?? []), row.entry.path]);
      for (const [root, paths] of groups) {
        const request: ExplorerMutationRequest = { root, operation, paths, destination };
        await client.invoke("explorer.mutate", request);
      }
      setEdit(null);
      setDeleting(null);
      setSelected(new Set());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      if (operation !== "reveal") refresh();
    }
  };
  const begin = (operation: string, row = selectedRows[0] ?? rows[0]) => {
    if (!row) return;
    const parent =
      operation === "rename" || !row.entry.is_dir ? parentPath(row.entry.path) : row.entry.path;
    setEdit({
      operation,
      parent,
      root: row.root,
      paths: operation === "rename" ? [row.entry.path] : [],
      value: operation === "rename" ? row.entry.name : "",
    });
    setMenu(null);
  };
  const submit = () => {
    if (!edit || !edit.value.trim()) return;
    if (edit.operation !== "move" && /[/\\]|^\.{1,2}$/.test(edit.value)) {
      setError(t("explorerInvalidName"));
      return;
    }
    const destination =
      edit.operation === "move" ? edit.value : `${edit.parent.replace(/\/$/, "")}/${edit.value}`;
    void mutate(
      edit.operation,
      rows.filter((r) => edit.paths.includes(r.entry.path)),
      destination,
      edit.root,
    );
  };
  const deletable = selectedRows.filter((r) => r.entry.path !== r.root);

  return (
    <section className="file-explorer workbench-sidebar-view" aria-label={t("workbenchExplorer")}>
      <header className="workbench-view-header">
        <h2>{t("workbenchExplorer")}</h2>
        <div className="workbench-view-tools">
          <button
            type="button"
            title={t("explorerNewFile")}
            aria-label={t("explorerNewFile")}
            disabled={busy || !roots.length}
            onClick={() => begin("newFile")}
          >
            <Icon id="file-plus" />
          </button>
          <button
            type="button"
            title={t("explorerNewFolder")}
            aria-label={t("explorerNewFolder")}
            disabled={busy || !roots.length}
            onClick={() => begin("newFolder")}
          >
            <Icon id="folder-plus" />
          </button>
          <button
            type="button"
            title={t("explorerRefresh")}
            aria-label={t("explorerRefresh")}
            disabled={busy}
            onClick={refresh}
          >
            <Icon id="refresh" />
          </button>
          <button
            type="button"
            title={t("explorerCollapse")}
            aria-label={t("explorerCollapse")}
            onClick={() => setExpanded(new Set())}
          >
            <Icon id="collapse-all" />
          </button>
          <ClosePrimaryPanelButton />
        </div>
      </header>
      {roots.length > 0 && (
        <input
          className="explorer-filter"
          aria-label={t("explorerFilter")}
          placeholder={t("explorerFilter")}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      )}
      {busy && (
        <p className="explorer-message" role="status">
          {t("workbenchLoading")}
        </p>
      )}
      {error && (
        <p className="explorer-message" role="alert">
          {error}
        </p>
      )}
      {!roots.length && !busy && (
        <div className="workbench-view-empty">
          <Icon id="explorer" size="lg" />
          <p>{t("workbenchNoFolder")}</p>
        </div>
      )}
      {edit && edit.operation !== "rename" && (
        <form
          className="explorer-inline-edit"
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
        >
          <input
            autoFocus
            aria-label={t(edit.operation === "move" ? "explorerDestination" : "explorerName")}
            value={edit.value}
            onChange={(e) => setEdit({ ...edit, value: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Escape") setEdit(null);
            }}
          />
          <button type="submit" disabled={busy}>
            {t("explorerApply")}
          </button>
          <button type="button" onClick={() => setEdit(null)}>
            {t("explorerCancel")}
          </button>
        </form>
      )}
      <div
        className="explorer-tree"
        hidden={!roots.length}
        ref={viewport}
        role="tree"
        aria-label={t("workbenchExplorer")}
        aria-multiselectable="true"
        tabIndex={0}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
        onKeyDown={(event) => {
          if (
            event.target !== event.currentTarget &&
            (event.target as HTMLElement).tagName === "INPUT"
          )
            return;
          const index = rows.findIndex((r) => r.entry.path === (focused ?? anchor.current));
          const row = rows[Math.max(0, index)];
          if (!row) return;
          if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
            event.preventDefault();
            const next =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? rows.length - 1
                  : Math.max(
                      0,
                      Math.min(rows.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)),
                    );
            select(rows[next]!, event.shiftKey, false);
            const top = next * ROW_HEIGHT;
            if (viewport.current && (top < scrollTop || top + ROW_HEIGHT > scrollTop + height)) {
              viewport.current.scrollTop = top;
              setScrollTop(top);
            }
          } else if (event.key === "Enter") {
            event.preventDefault();
            open(row);
          } else if (event.key === "ArrowRight" && !expanded.has(row.entry.path)) {
            event.preventDefault();
            void toggle(row);
          } else if (event.key === "ArrowLeft") {
            event.preventDefault();
            if (expanded.has(row.entry.path)) void toggle(row);
            else {
              const parent = rows.find((r) => r.entry.path === parentPath(row.entry.path));
              if (parent) select(parent, false, false);
            }
          } else if (event.key === "F2" && row.entry.path !== row.root) {
            event.preventDefault();
            begin("rename", row);
          } else if (event.key === "Delete" && deletable.length) {
            event.preventDefault();
            setDeleting(deletable);
          } else if ((event.ctrlKey || event.metaKey) && event.key === "a") {
            event.preventDefault();
            setSelected(new Set(rows.map((r) => r.entry.path)));
          } else if (event.key === "Escape") setMenu(null);
          else if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
            event.preventDefault();
            const rect = viewport.current?.getBoundingClientRect();
            setMenu({ row, x: rect?.left ?? 0, y: rect?.top ?? 0 });
          }
        }}
      >
        <div style={{ height: rows.length * ROW_HEIGHT, position: "relative" }}>
          {visible.map((row, i) => (
            <div
              key={row.entry.path}
              role="treeitem"
              aria-level={row.depth + 1}
              aria-selected={selected.has(row.entry.path)}
              aria-expanded={row.entry.is_dir ? expanded.has(row.entry.path) : undefined}
              className={`explorer-row${selected.has(row.entry.path) ? " is-selected" : ""}`}
              title={row.entry.path}
              style={{
                position: "absolute",
                top: (start + i) * ROW_HEIGHT,
                height: ROW_HEIGHT,
                paddingInlineStart: 8 + row.depth * 14,
              }}
              draggable={!busy && row.entry.path !== row.root}
              onClick={(e) => {
                select(row, e.shiftKey, e.ctrlKey || e.metaKey);
                viewport.current?.focus();
              }}
              onDoubleClick={() => open(row)}
              onContextMenu={(e) => {
                e.preventDefault();
                if (!selected.has(row.entry.path)) select(row, false, false);
                setMenu({
                  row,
                  x: Math.min(e.clientX, window.innerWidth - 210),
                  y: Math.min(e.clientY, window.innerHeight - 360),
                });
              }}
              onDragStart={(e) => {
                dragging.current = selected.has(row.entry.path) ? deletable : [row];
                e.dataTransfer.effectAllowed = "copyMove";
                e.dataTransfer.setData("text/plain", row.entry.path);
              }}
              onDragOver={(e) => {
                if (row.entry.is_dir && !row.entry.is_symlink) {
                  e.preventDefault();
                  e.dataTransfer.dropEffect = e.altKey || e.ctrlKey ? "copy" : "move";
                }
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (!busy && row.entry.is_dir && dragging.current.length)
                  void mutate(
                    e.altKey || e.ctrlKey ? "copy" : "move",
                    dragging.current,
                    row.entry.path,
                  );
                dragging.current = [];
              }}
              onDragEnd={() => {
                dragging.current = [];
              }}
            >
              <span
                className="explorer-chevron"
                onClick={(e) => {
                  e.stopPropagation();
                  void toggle(row);
                }}
              >
                {row.entry.is_dir && (
                  <Icon id={expanded.has(row.entry.path) ? "chevron-down" : "chevron-right"} />
                )}
              </span>
              <Icon
                id={
                  row.entry.is_dir
                    ? resolveFolderIcon(row.entry.name, theme, expanded.has(row.entry.path))
                    : resolveFileIcon(row.entry.name, theme)
                }
              />
              {edit?.operation === "rename" && edit.paths[0] === row.entry.path ? (
                <input
                  autoFocus
                  aria-label={t("explorerName")}
                  value={edit.value}
                  onClick={(e) => e.stopPropagation()}
                  onChange={(e) => setEdit({ ...edit, value: e.target.value })}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "Enter") submit();
                    if (e.key === "Escape") setEdit(null);
                  }}
                />
              ) : (
                <span className="explorer-name">{row.entry.name}</span>
              )}
              {(diagnostics[row.entry.path] ?? 0) > 0 && (
                <span
                  className="explorer-badge"
                  aria-label={t("explorerDiagnostics", { count: diagnostics[row.entry.path]! })}
                >
                  {diagnostics[row.entry.path]}
                </span>
              )}
            </div>
          ))}
        </div>
      </div>
      {menu && (
        <div
          className="explorer-menu"
          role="menu"
          style={{ left: menu.x, top: menu.y }}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setMenu(null);
              viewport.current?.focus();
            }
            if (e.key === "ArrowDown" || e.key === "ArrowUp") {
              e.preventDefault();
              const buttons = [
                ...e.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"),
              ];
              const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
              buttons[
                (index + (e.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length
              ]?.focus();
            }
          }}
        >
          <button
            role="menuitem"
            type="button"
            onClick={() => {
              open(menu.row);
              setMenu(null);
            }}
          >
            {t("explorerOpen")}
          </button>
          <button role="menuitem" type="button" onClick={() => begin("newFile", menu.row)}>
            {t("explorerNewFile")}
          </button>
          <button role="menuitem" type="button" onClick={() => begin("newFolder", menu.row)}>
            {t("explorerNewFolder")}
          </button>
          <button
            role="menuitem"
            type="button"
            disabled={menu.row.entry.path === menu.row.root}
            onClick={() => begin("rename", menu.row)}
          >
            {t("explorerRename")}
          </button>
          <button
            role="menuitem"
            type="button"
            disabled={!deletable.length}
            onClick={() => {
              setEdit({
                operation: "move",
                parent: "",
                root: menu.row.root,
                paths: deletable.map((r) => r.entry.path),
                value: parentPath(menu.row.entry.path),
              });
              setMenu(null);
            }}
          >
            {t("explorerMove")}
          </button>
          <button
            role="menuitem"
            type="button"
            disabled={!deletable.length}
            onClick={() => void mutate("duplicate", deletable)}
          >
            {t("explorerDuplicate")}
          </button>
          <button
            role="menuitem"
            type="button"
            disabled={!deletable.length}
            onClick={() => {
              setDeleting(deletable);
              setMenu(null);
            }}
          >
            {t("explorerDelete")}
          </button>
          <button
            role="menuitem"
            type="button"
            onClick={() => {
              void navigator.clipboard
                .writeText(selectedRows.map((r) => r.entry.path).join("\n"))
                .catch((e: unknown) => setError(String(e)));
              setMenu(null);
            }}
          >
            {t("explorerCopyPath")}
          </button>
          <button
            role="menuitem"
            type="button"
            onClick={() => {
              void navigator.clipboard
                .writeText(
                  selectedRows.map((r) => r.entry.path.slice(r.root.length + 1)).join("\n"),
                )
                .catch((e: unknown) => setError(String(e)));
              setMenu(null);
            }}
          >
            {t("explorerCopyRelative")}
          </button>
          <button role="menuitem" type="button" onClick={() => void mutate("reveal", [menu.row])}>
            {t("explorerRevealOS")}
          </button>
          <button role="menuitem" type="button" onClick={() => setMenu(null)}>
            {t("explorerCancel")}
          </button>
        </div>
      )}
      {deleting && (
        <dialog
          ref={confirmDialog}
          className="explorer-confirm"
          role="alertdialog"
          aria-modal="true"
          aria-label={t("explorerDelete")}
          onCancel={(event) => {
            event.preventDefault();
            setDeleting(null);
          }}
        >
          <p>{t("explorerDeleteConfirm", { count: deleting.length })}</p>
          <ul>
            {deleting.map((r) => (
              <li key={r.entry.path}>{r.entry.path}</li>
            ))}
          </ul>
          <button type="button" autoFocus onClick={() => setDeleting(null)}>
            {t("explorerCancel")}
          </button>
          <button type="button" disabled={busy} onClick={() => void mutate("delete", deleting)}>
            {t("explorerDelete")}
          </button>
        </dialog>
      )}
    </section>
  );
}
