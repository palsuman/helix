import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { CommandDescriptor } from "../generated/CommandDescriptor";
import type { QuickOpenMatch } from "../generated/QuickOpenMatch";
import type { WorkspaceRoot } from "../generated/WorkspaceRoot";
import { Icon, DEFAULT_FILE_ICON_THEME, BUILTIN_FILE_ICON_THEMES, resolveFileIcon } from "../icons";
import type { IpcClient } from "../ipc";
import { useMessage } from "../localization";
import { rankCommands } from "../commands/ranking";
import type { CommandContext } from "../commands/context";
import type { CommandRegistry } from "../commands/registry";
import { listWorkspaces } from "../workbench";
import { queryQuickOpen } from "./commands";
import { parseLineNumber, parseQuickOpenQuery } from "./mode";
import "./quickOpen.css";

const RECENT_FILES_KEY = "helix.quickOpen.recentFiles";
const RESULT_LIMIT = 100;
const EMPTY_CONTEXT: CommandContext = {};

export interface QuickOpenSymbol {
  name: string;
  detail?: string;
  path: string;
  line: number;
}

export interface QuickOpenSymbolProvider {
  document(query: string, signal: AbortSignal): Promise<QuickOpenSymbol[]>;
  workspace(query: string, signal: AbortSignal): Promise<QuickOpenSymbol[]>;
}

export interface QuickOpenProps {
  client: IpcClient;
  registry: CommandRegistry;
  context?: CommandContext;
  symbolProvider?: QuickOpenSymbolProvider;
  onOpen(path: string, split: boolean, line?: number): void;
  onLine(line: number): void;
}

interface FileResult extends QuickOpenMatch {
  rootName: string;
}

type Result =
  | { kind: "file"; file: FileResult }
  | { kind: "symbol"; symbol: QuickOpenSymbol }
  | { kind: "command"; command: CommandDescriptor };

function recentFiles(): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(RECENT_FILES_KEY) ?? "[]") as unknown;
    return Array.isArray(value)
      ? value.filter((item): item is string => typeof item === "string")
      : [];
  } catch {
    return [];
  }
}

function recordRecent(path: string): void {
  const next = [path, ...recentFiles().filter((candidate) => candidate !== path)].slice(0, 50);
  localStorage.setItem(RECENT_FILES_KEY, JSON.stringify(next));
}

export function QuickOpen({
  client,
  registry,
  context = EMPTY_CONTEXT,
  symbolProvider,
  onOpen,
  onLine,
}: QuickOpenProps) {
  const t = useMessage();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [roots, setRoots] = useState<WorkspaceRoot[]>([]);
  const [rootsReady, setRootsReady] = useState(false);
  const [results, setResults] = useState<Result[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [indexing, setIndexing] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const listId = useId();
  const parsed = useMemo(() => parseQuickOpenQuery(query), [query]);
  const theme = BUILTIN_FILE_ICON_THEMES.get(DEFAULT_FILE_ICON_THEME)!;

  useEffect(
    () =>
      registry.registerHandler("workbench.action.quickOpen", () => {
        previousFocus.current = document.activeElement as HTMLElement | null;
        setOpen(true);
        setQuery("");
        setActiveIndex(0);
        setMessage(null);
        setRootsReady(false);
      }),
    [registry],
  );

  useEffect(() => {
    if (!open) return;
    const controller = new AbortController();
    void listWorkspaces(client, { signal: controller.signal })
      .then((response) => {
        const available = response.workspaces.flatMap((workspace) =>
          workspace.roots.filter((root) => root.availability === "available"),
        );
        setRoots(available);
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) setMessage(String(error));
      })
      .finally(() => {
        if (!controller.signal.aborted) setRootsReady(true);
      });
    requestAnimationFrame(() => inputRef.current?.focus());
    return () => controller.abort();
  }, [client, open]);

  useEffect(() => {
    if (!open) return;
    const controller = new AbortController();
    const load = async () => {
      setActiveIndex(0);
      setMessage(null);
      setIndexing(false);
      if (parsed.mode === "line") {
        setResults([]);
        const line = parseLineNumber(parsed.query);
        setMessage(line === null ? t("quickOpenLineHint") : t("quickOpenGoToLine", { line }));
        return;
      }
      if (parsed.mode === "commands") {
        setLoading(true);
        const commands = await registry.list();
        if (controller.signal.aborted) return;
        setResults(
          rankCommands(
            commands,
            parsed.query,
            registry.recent(),
            context,
            t("commandPaletteUnavailable"),
          )
            .filter((entry) => entry.enabled)
            .map((entry) => ({ kind: "command" as const, command: entry.command })),
        );
        return;
      }
      if (parsed.mode === "documentSymbols" || parsed.mode === "workspaceSymbols") {
        if (symbolProvider === undefined) {
          setResults([]);
          setMessage(t("quickOpenNoSymbolProvider"));
          return;
        }
        setLoading(true);
        const symbols = await symbolProvider[
          parsed.mode === "documentSymbols" ? "document" : "workspace"
        ](parsed.query, controller.signal);
        if (!controller.signal.aborted) {
          setResults(symbols.map((symbol) => ({ kind: "symbol" as const, symbol })));
        }
        return;
      }

      if (!rootsReady) return;
      if (roots.length === 0) {
        setResults([]);
        setMessage(t("quickOpenNoWorkspace"));
        return;
      }
      setLoading(true);
      const responses = await Promise.all(
        roots.map(async (root) => ({
          root,
          response: await queryQuickOpen(
            client,
            {
              root: root.path,
              query: parsed.query,
              max_results: RESULT_LIMIT,
              recent_files: recentFiles(),
            },
            { signal: controller.signal },
          ),
        })),
      );
      if (controller.signal.aborted) return;
      setIndexing(responses.some(({ response }) => response.indexing));
      setResults(
        responses
          .flatMap(({ root, response }) =>
            response.matches.map((file) => ({ ...file, rootName: root.name })),
          )
          .sort((left, right) =>
            left.score === right.score
              ? left.relative_path.localeCompare(right.relative_path)
              : left.score > right.score
                ? -1
                : 1,
          )
          .slice(0, RESULT_LIMIT)
          .map((file) => ({ kind: "file" as const, file })),
      );
    };

    void load()
      .catch((error: unknown) => {
        if (!controller.signal.aborted) {
          setResults([]);
          setMessage(String(error));
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [client, context, open, parsed, registry, roots, rootsReady, symbolProvider, t]);

  const close = () => {
    setOpen(false);
    setResults([]);
    setMessage(null);
    requestAnimationFrame(() => previousFocus.current?.focus());
  };

  const accept = async (split: boolean, selected: Result | undefined = results[activeIndex]) => {
    if (parsed.mode === "line") {
      const line = parseLineNumber(parsed.query);
      if (line !== null) {
        onLine(line);
        close();
      }
      return;
    }
    const result = selected;
    if (result?.kind === "file") {
      recordRecent(result.file.path);
      onOpen(result.file.path, split);
      close();
    } else if (result?.kind === "symbol") {
      recordRecent(result.symbol.path);
      onOpen(result.symbol.path, split, result.symbol.line);
      close();
    } else if (result?.kind === "command") {
      await registry.execute(result.command.id);
      close();
    }
  };

  if (!open) return null;
  return (
    <div
      className="quick-open-backdrop"
      onMouseDown={(event) => event.target === event.currentTarget && close()}
    >
      <section
        className="quick-open"
        role="dialog"
        aria-modal="true"
        aria-label={t("quickOpenTitle")}
      >
        <div className="quick-open__input-row">
          <input
            ref={inputRef}
            role="combobox"
            aria-label={t("quickOpenSearch")}
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={results[activeIndex] ? `${listId}-${activeIndex}` : undefined}
            value={query}
            placeholder={t("quickOpenPlaceholder")}
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") close();
              else if (event.key === "ArrowDown") {
                event.preventDefault();
                setActiveIndex((index) => Math.min(results.length - 1, index + 1));
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                setActiveIndex((index) => Math.max(0, index - 1));
              } else if (event.key === "Enter") {
                event.preventDefault();
                void accept(event.ctrlKey);
              }
            }}
          />
          <button type="button" aria-label={t("quickOpenClose")} onClick={close}>
            <Icon id="close" size="sm" label={t("quickOpenClose")} />
          </button>
        </div>
        <div id={listId} className="quick-open__results" role="listbox">
          {indexing && (
            <p className="quick-open__hint" role="status">
              {t("quickOpenIndexing")}
            </p>
          )}
          {loading && results.length === 0 && (
            <p className="quick-open__state">{t("quickOpenLoading")}</p>
          )}
          {!loading && message && (
            <p className="quick-open__state" role="status">
              {message}
            </p>
          )}
          {!loading && !message && results.length === 0 && (
            <p className="quick-open__state">{t("quickOpenEmpty")}</p>
          )}
          {results.map((result, index) => {
            const active = index === activeIndex;
            const key =
              result.kind === "command"
                ? result.command.id
                : result.kind === "symbol"
                  ? `${result.symbol.path}:${result.symbol.line}:${result.symbol.name}`
                  : result.file.path;
            return (
              <div
                id={`${listId}-${index}`}
                key={key}
                className={`quick-open__option${active ? " is-active" : ""}`}
                role="option"
                aria-selected={active}
                onMouseEnter={() => setActiveIndex(index)}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => void accept(false, result)}
              >
                {result.kind === "file" && (
                  <Icon id={resolveFileIcon(result.file.file_name, theme)} size="sm" />
                )}
                <span>
                  <strong>
                    {result.kind === "file"
                      ? result.file.file_name
                      : result.kind === "symbol"
                        ? result.symbol.name
                        : result.command.title}
                  </strong>
                  <small>
                    {result.kind === "file"
                      ? t("quickOpenPathDescription", {
                          root: result.file.rootName,
                          path: result.file.relative_path,
                        })
                      : result.kind === "symbol"
                        ? t("quickOpenSymbolDescription", {
                            detail: result.symbol.detail ?? result.symbol.path,
                            line: result.symbol.line,
                          })
                        : result.command.category}
                  </small>
                </span>
              </div>
            );
          })}
        </div>
        <footer>{t("quickOpenFooter")}</footer>
      </section>
    </div>
  );
}
