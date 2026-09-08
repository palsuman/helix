import { useEffect, useMemo, useRef, useState } from "react";
import { useMessage } from "../localization";
import {
  findMatches,
  replaceAllMatches,
  replaceMatch,
  setSearchState,
  type SearchMatch,
  type SearchState,
} from "./findReplace";

export interface FindModel {
  getValue(): string;
  setValue(value: string): void;
}

export interface FindSelection {
  startLineNumber: number;
  startColumn: number;
  endLineNumber: number;
  endColumn: number;
}

export interface FindEditor {
  getSelection?(): FindSelection | null;
  setSelection?(selection: FindSelection): void;
  focus?(): void;
}

interface FindReplaceBarProps {
  model: FindModel;
  editor: FindEditor;
  replaceMode: boolean;
  onClose: () => void;
}

export function FindReplaceBar({ model, editor, replaceMode, onClose }: FindReplaceBarProps) {
  const t = useMessage();
  const [search, setSearch] = useState<SearchState>(() => setSearchState({ replaceMode }));
  const [currentIndex, setCurrentIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const scope = useMemo(() => {
    const selection = editor.getSelection?.();
    if (!search.findInSelection || !selection) return undefined;
    const lines = model.getValue().split("\n");
    return {
      start: positionToOffset(lines, selection.startLineNumber, selection.startColumn),
      end: positionToOffset(lines, selection.endLineNumber, selection.endColumn),
    } satisfies SearchMatch;
  }, [editor, model, search.findInSelection]);
  const matches = useMemo(() => findMatches(model.getValue(), search, scope), [model, scope, search]);
  const activeIndex = matches.length === 0 ? 0 : Math.min(currentIndex, matches.length - 1);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const updateSearch = (next: Partial<SearchState>) => {
    const updated = setSearchState(next);
    setSearch(updated);
    setCurrentIndex(0);
  };

  const move = (direction: 1 | -1) => {
    if (matches.length === 0) return;
    setCurrentIndex((index) => (index + direction + matches.length) % matches.length);
  };

  const selectCurrent = () => {
    const match = matches[activeIndex];
    if (!match || !editor.setSelection) return;
    const lines = model.getValue().split("\n");
    editor.setSelection({
      startLineNumber: 1 + lineAtOffset(lines, match.start),
      startColumn: 1 + columnAtOffset(lines, match.start),
      endLineNumber: 1 + lineAtOffset(lines, match.end),
      endColumn: 1 + columnAtOffset(lines, match.end),
    });
    editor.focus?.();
  };

  const replaceCurrent = () => {
    const match = matches[activeIndex];
    if (!match) return;
    model.setValue(replaceMatch(model.getValue(), match, search.replace));
    selectCurrent();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      move(event.shiftKey ? -1 : 1);
    } else if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  };

  return (
    <div className="editor-find" role="search" aria-label={t("editorFindReplace")}>
      <div className="editor-find__row">
        <input
          aria-label={t("editorFindInput")}
          onChange={(event) => updateSearch({ query: event.target.value })}
          onKeyDown={onKeyDown}
          placeholder={t("editorFindPlaceholder")}
          ref={inputRef}
          value={search.query}
        />
        {replaceMode && (
          <input
            aria-label={t("editorReplaceInput")}
            onChange={(event) => updateSearch({ replace: event.target.value })}
            onKeyDown={onKeyDown}
            placeholder={t("editorReplacePlaceholder")}
            value={search.replace}
          />
        )}
        <span aria-live="polite" className="editor-find__count">
          {t("editorFindCount", { current: matches.length === 0 ? 0 : activeIndex + 1, total: matches.length })}
        </span>
        <button aria-label={t("editorFindPrevious")} className="editor-find__previous" onClick={() => move(-1)} type="button" />
        <button aria-label={t("editorFindNext")} className="editor-find__next" onClick={() => move(1)} type="button" />
        {replaceMode && <button onClick={replaceCurrent} type="button">{t("editorReplace")}</button>}
        {replaceMode && (
          <button onClick={() => model.setValue(replaceAllMatches(model.getValue(), matches, search.replace))} type="button">
            {t("editorReplaceAll")}
          </button>
        )}
        <button aria-label={t("editorFindClose")} className="editor-find__close" onClick={onClose} type="button" />
      </div>
      <div className="editor-find__options">
        <label><input checked={search.regex} onChange={(event) => updateSearch({ regex: event.target.checked })} type="checkbox" />{t("editorFindRegex")}</label>
        <label><input checked={search.caseSensitive} onChange={(event) => updateSearch({ caseSensitive: event.target.checked })} type="checkbox" />{t("editorFindCaseSensitive")}</label>
        <label><input checked={search.wholeWord} onChange={(event) => updateSearch({ wholeWord: event.target.checked })} type="checkbox" />{t("editorFindWholeWord")}</label>
        <label><input checked={search.findInSelection} onChange={(event) => updateSearch({ findInSelection: event.target.checked })} type="checkbox" />{t("editorFindSelection")}</label>
      </div>
    </div>
  );
}

function positionToOffset(lines: string[], line: number, column: number): number {
  return lines.slice(0, Math.max(0, line - 1)).reduce((total, value) => total + value.length + 1, 0) + Math.max(0, column - 1);
}

function lineAtOffset(lines: string[], offset: number): number {
  let total = 0;
  for (let index = 0; index < lines.length; index += 1) {
    const end = total + lines[index]!.length;
    if (offset <= end) return index;
    total = end + 1;
  }
  return Math.max(0, lines.length - 1);
}

function columnAtOffset(lines: string[], offset: number): number {
  const line = lineAtOffset(lines, offset);
  const start = lines.slice(0, line).reduce((total, value) => total + value.length + 1, 0);
  return offset - start;
}