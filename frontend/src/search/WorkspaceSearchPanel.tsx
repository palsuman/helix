import { useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { useMessage } from "../localization";
import { Icon } from "../icons";
import { ClosePrimaryPanelButton } from "../workbench/ClosePrimaryPanelButton";
import type { IpcClient } from "../ipc";
import type { ReplaceFileResult } from "../generated/ReplaceFileResult";
import type { SearchMatch } from "../generated/SearchMatch";
import type { SearchQuery } from "../generated/SearchQuery";
import { cancelSearch, replace, search, undoReplace } from "./commands";
import "./workspaceSearch.css";

interface WorkspaceSearchPanelProps {
  client: IpcClient;
  root: string;
  onOpenMatch?: (path: string, line: number, column: number) => void;
}

interface PinnedResults {
  query: string;
  matches: SearchMatch[];
}

const resultKey = (match: SearchMatch) => `${match.path}:${match.line}:${match.column}`;

export function WorkspaceSearchPanel({ client, root, onOpenMatch }: WorkspaceSearchPanelProps) {
  const t = useMessage();
  const [query, setQuery] = useState<SearchQuery>(() => createQuery(root));
  const [replacement, setReplacement] = useState("");
  const [matches, setMatches] = useState<SearchMatch[]>([]);
  const [dismissed, setDismissed] = useState<Set<string>>(new Set());
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [collapsedPaths, setCollapsedPaths] = useState<Set<string>>(new Set());
  const [preview, setPreview] = useState(false);
  const [pinnedResults, setPinnedResults] = useState<PinnedResults | null>(null);
  const [previewFiles, setPreviewFiles] = useState<ReplaceFileResult[]>([]);
  const [operationIds, setOperationIds] = useState<string[]>([]);
  const [running, setRunning] = useState(false);
  const searchController = useRef<AbortController | null>(null);
  const operationController = useRef<AbortController | null>(null);

  const grouped = useMemo(() => groupMatches(matches, dismissed), [dismissed, matches]);
  const pinnedGrouped = useMemo(
    () => (pinnedResults ? groupMatches(pinnedResults.matches, new Set()) : []),
    [pinnedResults],
  );

  const update = (next: Partial<SearchQuery>) => setQuery((current) => ({ ...current, ...next }));

  const runSearch = async () => {
    if (!query.query.trim() || running) return;
    searchController.current?.abort();
    const controller = new AbortController();
    searchController.current = controller;
    setRunning(true);
    setPreviewFiles([]);
    try {
      const response = await search(client, query, { signal: controller.signal });
      if (controller.signal.aborted) return;
      setMatches(response.matches);
      setDismissed(new Set());
      setCollapsedPaths(new Set());
    } catch (error) {
      if (!controller.signal.aborted) throw error;
    } finally {
      if (searchController.current === controller) {
        searchController.current = null;
        setRunning(false);
      }
    }
  };

  const runReplace = async (paths: string[] = []) => {
    if (!query.query.trim() || running) return;
    operationController.current?.abort();
    const controller = new AbortController();
    operationController.current = controller;
    const cancelId = `workspace-${root}`;
    setRunning(true);
    try {
      const response = await replace(
        client,
        { search: query, replacement, paths, preview_only: preview, cancel_id: cancelId },
        { signal: controller.signal },
      );
      if (controller.signal.aborted) return;
      setPreviewFiles(response.files);
      if (!response.cancelled && !response.preview) {
        setOperationIds((current) => [response.operation_id, ...current].slice(0, 10));
        // Refresh the result set after an applied replacement.
        const refreshed = await search(client, query, { signal: controller.signal });
        if (!controller.signal.aborted) setMatches(refreshed.matches);
      }
    } finally {
      if (operationController.current === controller) {
        operationController.current = null;
        setRunning(false);
      }
    }
  };

  const cancel = () => {
    searchController.current?.abort();
    operationController.current?.abort();
    void cancelSearch(client, `workspace-${root}`);
    setRunning(false);
  };

  const undo = async () => {
    const operationId = operationIds[0];
    if (!operationId || running) return;
    setRunning(true);
    try {
      await undoReplace(client, operationId);
      setOperationIds((current) => current.slice(1));
      const refreshed = await search(client, query);
      setMatches(refreshed.matches);
    } finally {
      setRunning(false);
    }
  };

  const pinCurrent = () => {
    if (pinnedResults === null) {
      if (matches.length > 0) setPinnedResults({ query: query.query, matches: [...matches] });
    } else setPinnedResults(null);
  };

  return (
    <section className="workspace-search" aria-label={t("workspaceSearchTitle")}>
      <header className="workspace-search__header workbench-view-header">
        <h2>{t("workspaceSearchTitle")}</h2>
        <div className="workbench-view-tools">
          <button onClick={pinCurrent} type="button">
            {t(pinnedResults ? "workspaceSearchUnpin" : "workspaceSearchPin")}
          </button>
          <ClosePrimaryPanelButton />
        </div>
      </header>
      <div className="workspace-search__controls">
        <input
          aria-label={t("workspaceSearchQuery")}
          onChange={(event) => update({ query: event.target.value })}
          onKeyDown={(event) => {
            if (event.key === "Enter") void runSearch();
          }}
          placeholder={t("workspaceSearchQueryPlaceholder")}
          value={query.query}
        />
        <input
          aria-label={t("workspaceSearchReplacement")}
          onChange={(event) => setReplacement(event.target.value)}
          placeholder={t("workspaceSearchReplacementPlaceholder")}
          value={replacement}
        />
        <input
          aria-label={t("workspaceSearchInclude")}
          onChange={(event) => update({ include_glob: event.target.value || null })}
          placeholder={t("workspaceSearchIncludePlaceholder")}
          value={query.include_glob ?? ""}
        />
        <input
          aria-label={t("workspaceSearchExclude")}
          onChange={(event) => update({ exclude_glob: event.target.value || null })}
          placeholder={t("workspaceSearchExcludePlaceholder")}
          value={query.exclude_glob ?? ""}
        />
      </div>
      <div className="workspace-search__options">
        <label>
          <input
            checked={query.regex}
            onChange={(event) => update({ regex: event.target.checked })}
            type="checkbox"
          />
          {t("editorFindRegex")}
        </label>
        <label>
          <input
            checked={query.case_sensitive}
            onChange={(event) => update({ case_sensitive: event.target.checked })}
            type="checkbox"
          />
          {t("editorFindCaseSensitive")}
        </label>
        <label>
          <input
            checked={query.whole_word}
            onChange={(event) => update({ whole_word: event.target.checked })}
            type="checkbox"
          />
          {t("editorFindWholeWord")}
        </label>
        <label>
          <input
            checked={query.respect_gitignore}
            onChange={(event) => update({ respect_gitignore: event.target.checked })}
            type="checkbox"
          />
          {t("workspaceSearchGitignore")}
        </label>
        <label>
          {t("workspaceSearchContext")}
          <input
            min={0}
            max={5}
            onChange={(event) => update({ context_lines: Number(event.target.value) })}
            type="number"
            value={query.context_lines}
          />
        </label>
      </div>
      <div className="workspace-search__actions">
        <button disabled={running} onClick={() => void runSearch()} type="button">
          {t("workspaceSearchRun")}
        </button>
        <button disabled={running} onClick={() => setPreview((value) => !value)} type="button">
          {t(preview ? "workspaceSearchHidePreview" : "workspaceSearchPreview")}
        </button>
        <button
          disabled={running}
          onClick={() => void runReplace([...selectedPaths])}
          type="button"
        >
          {t("workspaceSearchReplaceSelected")}
        </button>
        <button disabled={running} onClick={() => void runReplace()} type="button">
          {t("workspaceSearchReplaceAll")}
        </button>
        <button
          disabled={running || operationIds.length === 0}
          onClick={() => void undo()}
          type="button"
        >
          {t("workspaceSearchUndo")}
        </button>
        <button disabled={!running} onClick={cancel} type="button">
          {t("workspaceSearchCancel")}
        </button>
      </div>
      {running && (
        <p className="workspace-search__progress" role="status">
          {t("workspaceSearchProgress")}
        </p>
      )}
      {previewFiles.length > 0 && (
        <section
          className="workspace-search__previews"
          aria-label={t("workspaceSearchPreviewResults")}
        >
          {previewFiles.map((file) => (
            <details key={file.path} open>
              <summary>
                {t("workspaceSearchPreviewFile", { path: file.path, count: file.match_count })}
              </summary>
              <div className="workspace-search__diff">
                <div>
                  <strong>{t("workspaceSearchPreviewBefore")}</strong>
                  <pre>{file.before}</pre>
                </div>
                <div>
                  <strong>{t("workspaceSearchPreviewAfter")}</strong>
                  <pre>{file.after}</pre>
                </div>
              </div>
            </details>
          ))}
        </section>
      )}
      <div className="workspace-search__results" aria-live="polite">
        {pinnedGrouped.length > 0 && (
          <section
            className="workspace-search__pinned"
            aria-label={t("workspaceSearchPinnedResults", { query: pinnedResults?.query ?? "" })}
          >
            <h3>{t("workspaceSearchPinnedResults", { query: pinnedResults?.query ?? "" })}</h3>
            {renderGroups(
              pinnedGrouped,
              collapsedPaths,
              setCollapsedPaths,
              selectedPaths,
              setSelectedPaths,
              onOpenMatch,
              t,
              false,
            )}
          </section>
        )}
        {renderGroups(
          grouped,
          collapsedPaths,
          setCollapsedPaths,
          selectedPaths,
          setSelectedPaths,
          onOpenMatch,
          t,
          true,
          setDismissed,
        )}
      </div>
    </section>
  );
}

function createQuery(root: string): SearchQuery {
  return {
    root,
    query: "",
    case_sensitive: false,
    whole_word: false,
    max_results: 500,
    regex: false,
    include_glob: null,
    exclude_glob: null,
    context_lines: 0,
    respect_gitignore: true,
  };
}

function groupMatches(matches: SearchMatch[], dismissed: Set<string>) {
  const groups = new Map<string, SearchMatch[]>();
  for (const match of matches) {
    if (dismissed.has(resultKey(match))) continue;
    groups.set(match.path, [...(groups.get(match.path) ?? []), match]);
  }
  return [...groups.entries()];
}

function renderGroups(
  groups: [string, SearchMatch[]][],
  collapsed: Set<string>,
  setCollapsed: Dispatch<SetStateAction<Set<string>>>,
  selectedPaths: Set<string>,
  setSelectedPaths: Dispatch<SetStateAction<Set<string>>>,
  onOpenMatch: WorkspaceSearchPanelProps["onOpenMatch"],
  t: ReturnType<typeof useMessage>,
  dismissible: boolean,
  setDismissed?: Dispatch<SetStateAction<Set<string>>>,
) {
  return groups.map(([path, values]) => {
    const isCollapsed = collapsed.has(path);
    return (
      <article key={path}>
        <header className="workspace-search__file-header">
          <label>
            <input
              checked={selectedPaths.has(path)}
              onChange={() => setSelectedPaths((current) => toggleSet(current, path))}
              type="checkbox"
            />
            {path}
          </label>
          <button
            aria-label={t(isCollapsed ? "workspaceSearchExpand" : "workspaceSearchCollapse")}
            onClick={() => setCollapsed((current) => toggleSet(current, path))}
            type="button"
          >
            <Icon id={isCollapsed ? "chevron-right" : "chevron-down"} />
          </button>
        </header>
        {!isCollapsed &&
          values.map((match) => (
            <div className="workspace-search__match-row" key={resultKey(match)}>
              <button
                className="workspace-search__match"
                onClick={() => onOpenMatch?.(match.path, match.line, match.column)}
                type="button"
              >
                <span>
                  {t("workspaceSearchLocation", { line: match.line, column: match.column })}
                </span>{" "}
                {match.text}
              </button>
              {dismissible && setDismissed && (
                <button
                  aria-label={t("workspaceSearchDismiss", {
                    location: t("workspaceSearchLocation", {
                      line: match.line,
                      column: match.column,
                    }),
                  })}
                  onClick={() => setDismissed((current) => new Set(current).add(resultKey(match)))}
                  type="button"
                >
                  <Icon id="close" />
                </button>
              )}
            </div>
          ))}
      </article>
    );
  });
}

function toggleSet(current: Set<string>, value: string): Set<string> {
  const next = new Set(current);
  if (next.has(value)) next.delete(value);
  else next.add(value);
  return next;
}
