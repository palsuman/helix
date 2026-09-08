export interface SearchOptions {
  query: string;
  regex: boolean;
  caseSensitive: boolean;
  wholeWord: boolean;
}

export interface SearchMatch {
  start: number;
  end: number;
}

export interface SearchState extends SearchOptions {
  replace: string;
  replaceMode: boolean;
  findInSelection: boolean;
}

export const DEFAULT_SEARCH_STATE: SearchState = {
  query: "",
  replace: "",
  regex: false,
  caseSensitive: false,
  wholeWord: false,
  replaceMode: false,
  findInSelection: false,
};

let sharedSearchState: SearchState = { ...DEFAULT_SEARCH_STATE };

export function getSearchState(): SearchState {
  return { ...sharedSearchState };
}

export function setSearchState(next: Partial<SearchState>): SearchState {
  sharedSearchState = { ...sharedSearchState, ...next };
  return getSearchState();
}

export function findMatches(text: string, options: SearchOptions, scope?: SearchMatch): SearchMatch[] {
  if (!options.query) return [];
  const offset = scope?.start ?? 0;
  const source = scope ? text.slice(scope.start, scope.end) : text;
  const pattern = buildPattern(options);
  if (!pattern) return [];
  const matches: SearchMatch[] = [];
  for (const match of source.matchAll(pattern)) {
    const value = match[0];
    if (value.length === 0) continue;
    matches.push({ start: offset + (match.index ?? 0), end: offset + (match.index ?? 0) + value.length });
  }
  return matches;
}

export function replaceMatch(text: string, match: SearchMatch, replacement: string): string {
  return `${text.slice(0, match.start)}${replacement}${text.slice(match.end)}`;
}

export function replaceAllMatches(text: string, matches: SearchMatch[], replacement: string): string {
  let result = text;
  for (let index = matches.length - 1; index >= 0; index -= 1) {
    const match = matches[index];
    if (match) result = replaceMatch(result, match, replacement);
  }
  return result;
}

function buildPattern(options: SearchOptions): RegExp | null {
  try {
    const source = options.regex ? options.query : escapeRegExp(options.query);
    const wrapped = options.wholeWord ? `\\b(?:${source})\\b` : source;
    return new RegExp(wrapped, options.caseSensitive ? "gu" : "giu");
  } catch {
    return null;
  }
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}