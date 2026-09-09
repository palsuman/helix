import { useEffect, useRef, useState, type MutableRefObject } from "react";
import { useMessage } from "../localization";
import type { IpcClient } from "../ipc";
import { readEditorFile, writeEditorFile, type EditorFile } from "./fileService";
import {
  hasConflictMarkers,
  normalizeText,
  type AutoSaveMode,
  type BufferLifecycleOptions,
} from "./lifecycle";
import { DEFAULT_EDITOR_SETTINGS, type EditorSettings } from "./settings";
import { FindReplaceBar, type FindSelection } from "./FindReplaceBar";
import { setSearchState } from "./findReplace";
import "./editor.css";

const LARGE_FILE_BYTES = 5n * 1024n * 1024n;

const EMPTY_EDITOR_SETTINGS: Partial<EditorSettings> = {};

export interface EditorRecovery {
  revision: number;
  text: string;
}

export interface EditorSurfaceProps {
  client: IpcClient;
  path: string;
  monacoLoader: MonacoLoader;
  settings?: Partial<EditorSettings>;
  recovery?: EditorRecovery;
  heartbeat?: () => Promise<boolean>;
  heartbeatIntervalMs?: number;
  onDirtyChange?: (dirty: boolean) => void;
  onKernelUnavailable?: () => void;
  onSaved?: (file: EditorFile) => void;
  onSaveReady?: (save: (() => Promise<void>) | null) => void;
  autoSave?: AutoSaveMode;
  autoSaveDelayMs?: number;
  saveOptions?: BufferLifecycleOptions;
  onDropPath?: (path: string, isDirectory: boolean) => void;
  revealLine?: number;
  revealSequence?: number;
}

interface MonacoModel {
  getValue(): string;
  onDidChangeContent(listener: () => void): { dispose(): void };
  setValue(value: string): void;
  dispose(): void;
}

interface MonacoEditor {
  addCommand(keybinding: number, handler: () => void): void;
  getSelection?(): FindSelection | null;
  setSelection?(selection: FindSelection): void;
  onDidChangeCursorPosition?(listener: () => void): { dispose(): void };
  onDidScrollChange?(listener: () => void): { dispose(): void };
  restoreViewState?(state: unknown): void;
  saveViewState?(): unknown;
  setPosition?(position: { lineNumber: number; column: number }): void;
  revealLineInCenter?(lineNumber: number): void;
  focus?(): void;
  dispose(): void;
}

import { useExplorerStore } from "../explorer/model";

interface MonacoApi {
  editor: {
    createModel(value: string, language: string, uri: unknown): MonacoModel;
    create(container: HTMLElement, options: Record<string, unknown>): MonacoEditor;
    onDidChangeMarkers?(listener: () => void): { dispose(): void };
    getModelMarkers?(filter: Record<string, never>): Array<{ resource: { fsPath: string } }>;
  };
  Uri: { parse(value: string): unknown };
  KeyMod: { CtrlCmd: number };
  KeyCode: { KeyS: number; KeyF: number; KeyH: number };
}

export type MonacoLoader = () => Promise<MonacoApi>;

interface EditorState {
  kind: "loading" | "ready" | "binary" | "error";
  file: EditorFile | null;
  message?: string;
  largeFile: boolean;
}

function languageForPath(path: string): string {
  const extension = path.split(".").pop()?.toLowerCase();
  const languages: Record<string, string> = {
    c: "c",
    css: "css",
    go: "go",
    html: "html",
    java: "java",
    js: "javascript",
    json: "json",
    md: "markdown",
    py: "python",
    rs: "rust",
    sh: "shell",
    ts: "typescript",
    tsx: "typescript",
    yaml: "yaml",
    yml: "yaml",
  };
  return (extension && languages[extension]) || "plaintext";
}

function isLargeFile(file: EditorFile): boolean {
  return file.size >= LARGE_FILE_BYTES;
}

export function EditorSurface({
  client,
  path,
  monacoLoader,
  settings = EMPTY_EDITOR_SETTINGS,
  recovery,
  heartbeat,
  heartbeatIntervalMs = 5_000,
  onDirtyChange,
  onKernelUnavailable,
  onSaved,
  onSaveReady,
  autoSave = "off",
  autoSaveDelayMs = 1_000,
  saveOptions,
  onDropPath,
  revealLine,
  revealSequence,
}: EditorSurfaceProps) {
  const t = useMessage();
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<MonacoEditor | null>(null);
  const modelRef = useRef<MonacoModel | null>(null);
  const viewStateListenersRef = useRef<{ dispose(): void }[]>([]);
  const onSaveReadyRef = useRef(onSaveReady);
  const fileRef = useRef<EditorFile | null>(null);
  const dirtyRef = useRef(false);
  const [dirty, setDirty] = useState(false);
  const [findMode, setFindMode] = useState<"find" | "replace" | null>(null);
  const [findBindings, setFindBindings] = useState<{
    model: MonacoModel;
    editor: MonacoEditor;
  } | null>(null);
  const [state, setState] = useState<EditorState>({
    kind: "loading",
    file: null,
    largeFile: false,
  });

  useEffect(() => {
    onSaveReadyRef.current = onSaveReady;
  }, [onSaveReady]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      setState({ kind: "loading", file: null, largeFile: false });
      try {
        const file = await readEditorFile(client, path);
        if (cancelled) return;
        fileRef.current = file;
        const largeFile = isLargeFile(file);
        if (file.binary || file.text === null) {
          setState({ kind: "binary", file, largeFile });
          return;
        }

        const monaco = await monacoLoader();
        if (cancelled || !containerRef.current) return;
        const model = monaco.editor.createModel(
          file.text,
          languageForPath(path),
          monaco.Uri.parse(`file://${encodeURI(path)}`),
        );
        const mergedSettings = { ...DEFAULT_EDITOR_SETTINGS, ...settings };
        const editor = monaco.editor.create(containerRef.current, {
          model,
          automaticLayout: true,
          minimap: { enabled: largeFile ? false : mergedSettings.minimap },
          bracketPairColorization: { enabled: mergedSettings.bracketColorization },
          guides: { indentation: mergedSettings.indentGuides },
          lineNumbers: mergedSettings.lineNumbers,
          wordWrap: largeFile ? "off" : mergedSettings.wordWrap,
          renderWhitespace: largeFile ? "none" : mergedSettings.renderWhitespace,
          folding: largeFile ? false : mergedSettings.folding,
          largeFileOptimizations: largeFile,
          readOnly: file.readonly,
        });
        editorRef.current = editor;
        modelRef.current = model;
        setFindBindings({ model, editor });
        restoreViewState(editor, path);
        const persistViewState = () => persistEditorViewState(editor, path);
        viewStateListenersRef.current = [
          editor.onDidChangeCursorPosition?.(persistViewState),
          editor.onDidScrollChange?.(persistViewState),
        ].filter((listener): listener is { dispose(): void } => listener !== undefined);
        const updateDiagnostics = () => {
          const diagnostics: Record<string, number> = {};
          for (const marker of monaco.editor.getModelMarkers?.({}) ?? []) {
            const markerPath = marker.resource.fsPath.replace(/\\/g, "/");
            diagnostics[markerPath] = (diagnostics[markerPath] ?? 0) + 1;
          }
          useExplorerStore.setState({ diagnostics });
        };
        const markerListener = monaco.editor.onDidChangeMarkers?.(updateDiagnostics);
        if (markerListener) {
          viewStateListenersRef.current.push(markerListener);
          updateDiagnostics();
        }
        model.onDidChangeContent(() => {
          dirtyRef.current = true;
          setDirty(true);
          onDirtyChange?.(true);
        });
        editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
          void saveModel(
            client,
            fileRef,
            modelRef,
            dirtyRef,
            setDirty,
            onDirtyChange,
            onSaved,
            saveOptions,
          );
        });
        editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyF, () => {
          setSearchState({ replaceMode: false });
          setFindMode("find");
        });
        editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyH, () => {
          setSearchState({ replaceMode: true });
          setFindMode("replace");
        });
        onSaveReadyRef.current?.(() =>
          saveModel(
            client,
            fileRef,
            modelRef,
            dirtyRef,
            setDirty,
            onDirtyChange,
            onSaved,
            saveOptions,
          ),
        );
        setState({ kind: "ready", file, largeFile });
      } catch (error) {
        if (!cancelled) {
          setState({
            kind: "error",
            file: null,
            largeFile: false,
            message: error instanceof Error ? error.message : String(error),
          });
        }
      }
    };
    void load();
    return () => {
      cancelled = true;
      viewStateListenersRef.current.forEach((listener) => listener.dispose());
      viewStateListenersRef.current = [];
      editorRef.current?.dispose();
      modelRef.current?.dispose();
      editorRef.current = null;
      modelRef.current = null;
      setFindBindings(null);
      fileRef.current = null;
      dirtyRef.current = false;
      onSaveReadyRef.current?.(null);
    };
  }, [client, monacoLoader, onDirtyChange, onSaved, path, saveOptions, settings]);

  useEffect(() => {
    if (!recovery || !modelRef.current || !fileRef.current) return;
    modelRef.current.setValue(recovery.text);
    dirtyRef.current = true;
    setDirty(true);
    onDirtyChange?.(true);
  }, [onDirtyChange, recovery]);

  useEffect(() => {
    if (revealLine === undefined || state.kind !== "ready") return;
    const lineNumber = Math.max(1, Math.floor(revealLine));
    editorRef.current?.setPosition?.({ lineNumber, column: 1 });
    editorRef.current?.revealLineInCenter?.(lineNumber);
    editorRef.current?.focus?.();
  }, [revealLine, revealSequence, state.kind]);

  useEffect(() => {
    if (!heartbeat) return;
    const timer = window.setInterval(() => {
      void heartbeat().then((alive) => {
        if (!alive) onKernelUnavailable?.();
      });
    }, heartbeatIntervalMs);
    return () => window.clearInterval(timer);
  }, [heartbeat, heartbeatIntervalMs, onKernelUnavailable]);

  useEffect(() => {
    if (!dirty || autoSave === "off" || state.kind !== "ready") return;
    const save = () => {
      const text = modelRef.current?.getValue() ?? "";
      if (hasConflictMarkers(text)) return;
      void saveModel(
        client,
        fileRef,
        modelRef,
        dirtyRef,
        setDirty,
        onDirtyChange,
        onSaved,
        saveOptions,
      );
    };
    if (autoSave === "afterDelay") {
      const timer = window.setTimeout(save, autoSaveDelayMs);
      return () => window.clearTimeout(timer);
    }
    if (autoSave === "onWindowChange") {
      window.addEventListener("blur", save);
      return () => window.removeEventListener("blur", save);
    }
    return undefined;
  }, [autoSave, autoSaveDelayMs, client, dirty, onDirtyChange, onSaved, saveOptions, state.kind]);

  const saveOnFocusChange = () => {
    if (autoSave !== "onFocusChange" || !dirty) return;
    const text = modelRef.current?.getValue() ?? "";
    if (!hasConflictMarkers(text)) {
      void saveModel(
        client,
        fileRef,
        modelRef,
        dirtyRef,
        setDirty,
        onDirtyChange,
        onSaved,
        saveOptions,
      );
    }
  };

  const dropPath = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    const uri = event.dataTransfer.getData("text/uri-list").split("\n")[0]?.trim();
    if (uri?.startsWith("file://")) onDropPath?.(decodeURIComponent(uri.slice(7)), false);
  };

  return (
    <div
      className="editor-surface"
      aria-label={t("editorSurface", { path })}
      onDragOver={(event) => event.preventDefault()}
      onDrop={dropPath}
      onBlur={saveOnFocusChange}
    >
      {state.kind === "loading" && <p role="status">{t("editorLoading", { path })}</p>}
      {state.kind === "error" && (
        <p role="alert">{t("editorUnableToOpen", { path, message: state.message ?? "" })}</p>
      )}
      {state.kind === "binary" && <p role="status">{t("editorBinaryReadonly")}</p>}
      {state.kind === "ready" && state.largeFile && (
        <p className="editor-notice" role="status">
          {t("editorLargeFileMode")}
        </p>
      )}
      {state.kind === "ready" && state.file && (
        <div className="editor-surface__status" role="status">
          <span>{t("editorEncodingStatus", { encoding: state.file.encoding })}</span>
          <span>{t("editorEolStatus", { eol: state.file.eol.style })}</span>
          {state.file.readonly && <span>{t("editorReadonly")}</span>}
          {dirty && <span>{t("editorModified")}</span>}
        </div>
      )}
      {findMode && findBindings && (
        <FindReplaceBar
          editor={findBindings.editor}
          model={findBindings.model}
          onClose={() => setFindMode(null)}
          replaceMode={findMode === "replace"}
        />
      )}
      <div ref={containerRef} className="editor-surface__monaco" data-testid="monaco-editor" />
    </div>
  );
}

function viewStateKey(path: string): string {
  return `helix.editor.view-state:${path}`;
}

function restoreViewState(editor: MonacoEditor, path: string): void {
  if (!editor.restoreViewState || typeof localStorage === "undefined") return;
  try {
    const stored = localStorage.getItem(viewStateKey(path));
    if (stored) editor.restoreViewState(JSON.parse(stored) as unknown);
  } catch {
    localStorage.removeItem(viewStateKey(path));
  }
}

function persistEditorViewState(editor: MonacoEditor, path: string): void {
  if (!editor.saveViewState || typeof localStorage === "undefined") return;
  const state = editor.saveViewState();
  if (state === null || state === undefined) return;
  try {
    localStorage.setItem(viewStateKey(path), JSON.stringify(state));
  } catch {
    // View state is an optimization; a full storage quota must not break editing.
  }
}

async function saveModel(
  client: IpcClient,
  fileRef: MutableRefObject<EditorFile | null>,
  modelRef: { current: MonacoModel | null },
  dirtyRef: MutableRefObject<boolean>,
  setDirty: (dirty: boolean) => void,
  onDirtyChange: ((dirty: boolean) => void) | undefined,
  onSaved: ((file: EditorFile) => void) | undefined,
  saveOptions?: BufferLifecycleOptions,
): Promise<void> {
  const file = fileRef.current;
  const model = modelRef.current;
  if (!file || !model || file.binary || file.readonly || !dirtyRef.current) return;
  const outcome = await writeEditorFile(client, file, normalizeText(model.getValue(), saveOptions));
  fileRef.current = { ...file, hash: outcome.hash };
  dirtyRef.current = false;
  setDirty(false);
  onDirtyChange?.(false);
  onSaved?.(fileRef.current);
}
