import { useMemo, useState } from "react";
import { useMessage } from "../localization";
import type { IpcClient } from "../ipc";
import type { SearchMatch } from "../generated/SearchMatch";
import type { SearchQuery } from "../generated/SearchQuery";
import { cancelSearch, replace, search, undoReplace } from "./commands";
import "./workspaceSearch.css";

interface WorkspaceSearchPanelProps {
  client: IpcClient;
  root: string;
  onOpenMatch?: (path: string, line: number, column: number) => void;
}

export function WorkspaceSearchPanel({ client, root, onOpenMatch }: WorkspaceSearchPanelProps) {
  const t = useMessage();
  const [query, setQuery] = useState<SearchQuery>({
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
  });
  const [replacement, setReplacement] = useState("");
  const [matches, setMatches] = useState<SearchMatch[]>([]);
  const [dismissed, setDismissed] = useState<Set<string>>(new Set());
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [preview, setPreview] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [operationId, setOperationId] = useState<string | null>(null);

  const grouped = useMemo(() => {
    const groups = new Map<string, SearchMatch[]>();
    for (const match of matches) {
      const key = `${match.path}:${match.line}:${match.column}`;
      if (dismissed.has(key)) continue;
      const values = groups.get(match.path) ?? [];
      values.push(match);
      groups.set(match.path, values);
    }
    return [...groups.entries()];
  }, [dismissed, matches]);

  const update = (next: Partial<SearchQuery>) => setQuery((current) => ({ ...current, ...next }));
  const runSearch = async () => {
    if (!query.query) return;
    const response = await search(client, query);
    setMatches(response.matches);
    setDismissed(new Set());
  };
  const runReplace = async (paths: string[] = []) => {
    const response = await replace(client, {
      search: query,
      replacement,
      paths,
      preview_only: preview,
      cancel_id: null,
    });
    setOperationId(response.operation_id);
    if (!preview) await runSearch();
  };

  return (
    <section className={`workspace-search${pinned ? " is-pinned" : ""}`} aria-label={t("workspaceSearchTitle")}>
      <header className="workspace-search__header">
        <h2>{t("workspaceSearchTitle")}</h2>
        <button onClick={() => setPinned((value) => !value)} type="button">{t("workspaceSearchPin")}</button>
      </header>
      <div className="workspace-search__controls">
        <input aria-label={t("workspaceSearchQuery")} onChange={(event) => update({ query: event.target.value })} onKeyDown={(event) => { if (event.key === "Enter") void runSearch(); }} placeholder={t("workspaceSearchQueryPlaceholder")} value={query.query} />
        <input aria-label={t("workspaceSearchReplacement")} onChange={(event) => setReplacement(event.target.value)} placeholder={t("workspaceSearchReplacementPlaceholder")} value={replacement} />
        <input aria-label={t("workspaceSearchInclude")} onChange={(event) => update({ include_glob: event.target.value || null })} placeholder={t("workspaceSearchIncludePlaceholder")} value={query.include_glob ?? ""} />
        <input aria-label={t("workspaceSearchExclude")} onChange={(event) => update({ exclude_glob: event.target.value || null })} placeholder={t("workspaceSearchExcludePlaceholder")} value={query.exclude_glob ?? ""} />
      </div>
      <div className="workspace-search__options">
        <label><input checked={query.regex} onChange={(event) => update({ regex: event.target.checked })} type="checkbox" />{t("editorFindRegex")}</label>
        <label><input checked={query.case_sensitive} onChange={(event) => update({ case_sensitive: event.target.checked })} type="checkbox" />{t("editorFindCaseSensitive")}</label>
        <label><input checked={query.whole_word} onChange={(event) => update({ whole_word: event.target.checked })} type="checkbox" />{t("editorFindWholeWord")}</label>
        <label><input checked={query.respect_gitignore} onChange={(event) => update({ respect_gitignore: event.target.checked })} type="checkbox" />{t("workspaceSearchGitignore")}</label>
        <label>{t("workspaceSearchContext")}<input min={0} max={5} onChange={(event) => update({ context_lines: Number(event.target.value) })} type="number" value={query.context_lines} /></label>
      </div>
      <div className="workspace-search__actions">
        <button onClick={() => void runSearch()} type="button">{t("workspaceSearchRun")}</button>
        <button onClick={() => setPreview((value) => !value)} type="button">{t(preview ? "workspaceSearchHidePreview" : "workspaceSearchPreview")}</button>
        <button onClick={() => void runReplace([...selectedPaths])} type="button">{t("workspaceSearchReplaceSelected")}</button>
        <button onClick={() => void runReplace()} type="button">{t("workspaceSearchReplaceAll")}</button>
        <button onClick={() => { if (operationId) void undoReplace(client, operationId); }} type="button">{t("workspaceSearchUndo")}</button>
        <button onClick={() => void cancelSearch(client, `workspace-${root}`)} type="button">{t("workspaceSearchCancel")}</button>
      </div>
      <div className="workspace-search__results" aria-live="polite">
        {grouped.map(([path, values]) => (
          <article key={path}>
            <label><input checked={selectedPaths.has(path)} onChange={() => setSelectedPaths((current) => { const next = new Set(current); if (next.has(path)) next.delete(path); else next.add(path); return next; })} type="checkbox" />{path}</label>
            {values.map((match) => (
              <button className="workspace-search__match" key={`${match.path}:${match.line}:${match.column}`} onClick={() => onOpenMatch?.(match.path, match.line, match.column)} type="button">
                <span>{t("workspaceSearchLocation", { line: match.line, column: match.column })}</span> {match.text}
              </button>
            ))}
          </article>
        ))}
      </div>
    </section>
  );
}