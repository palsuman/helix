import type { SearchQuery } from "../generated/SearchQuery";
import type { SearchResponse } from "../generated/SearchResponse";
import type { SearchStatsResponse } from "../generated/SearchStatsResponse";
import type { ReplaceRequest } from "../generated/ReplaceRequest";
import type { ReplaceResponse } from "../generated/ReplaceResponse";
import type { UndoRequest } from "../generated/UndoRequest";
import type { InvokeOptions, IpcClient } from "../ipc";

export const SEARCH_COMMANDS = {
  query: "search.query",
  stats: "search.stats",
  replace: "search.replace",
  undo: "search.undo",
  cancel: "search.cancel",
} as const;

export const SEARCH_CHANNELS = {
  results: "search:results",
} as const;

export function search(client: IpcClient, query: SearchQuery, options?: InvokeOptions) {
  return client.invoke<SearchQuery, SearchResponse>(SEARCH_COMMANDS.query, query, options);
}

export function searchStats(client: IpcClient, root: string, options?: InvokeOptions) {
  return client.invoke<SearchQuery, SearchStatsResponse>(
    SEARCH_COMMANDS.stats,
    {
      root,
      query: "",
      case_sensitive: false,
      whole_word: false,
      max_results: 0,
      regex: false,
      include_glob: null,
      exclude_glob: null,
      context_lines: 0,
      respect_gitignore: true,
    },
    options,
  );
}

export function replace(client: IpcClient, request: ReplaceRequest, options?: InvokeOptions) {
  return client.invoke<ReplaceRequest, ReplaceResponse>(SEARCH_COMMANDS.replace, request, options);
}

export function undoReplace(client: IpcClient, operationId: string, options?: InvokeOptions) {
  const request: UndoRequest = { operation_id: operationId };
  return client.invoke<UndoRequest, ReplaceResponse>(SEARCH_COMMANDS.undo, request, options);
}

export function cancelSearch(client: IpcClient, cancelId: string, options?: InvokeOptions) {
  return client.invoke(SEARCH_COMMANDS.cancel, {
    root: "",
    query: `cancel:${cancelId}`,
    case_sensitive: false,
    whole_word: false,
    max_results: 0,
  }, options);
}