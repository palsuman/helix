import type { Encoding } from "../generated/Encoding";
import type { LineEnding } from "../generated/LineEnding";
import type { IpcClient } from "../ipc";
import { writeEditorFile, type EditorFile, type EditorWriteResult } from "./fileService";

export type AutoSaveMode = "off" | "afterDelay" | "onFocusChange" | "onWindowChange";

export interface BufferRecord {
  id: string;
  path: string | null;
  text: string;
  language: string;
  dirty: boolean;
  dirtyWithNoFile: boolean;
  readonly: boolean;
  encoding: Encoding;
  eol: LineEnding;
  hash: string | null;
  conflictMarkers: boolean;
  autoSave: AutoSaveMode;
}

export interface BufferWal {
  load(): Promise<BufferRecord[]>;
  put(buffer: BufferRecord): Promise<void>;
  remove(id: string): Promise<void>;
}

export interface SaveDialog {
  saveAs(defaultPath?: string): Promise<string | null>;
}

export interface BufferLifecycleOptions {
  trimTrailingWhitespace?: boolean;
  insertFinalNewline?: boolean;
  defaultEol?: LineEnding;
}

export interface SaveOutcome {
  buffer: BufferRecord;
  result: EditorWriteResult;
}

export function createUntitledBuffer(
  language = "plaintext",
  text = "",
  options: Pick<BufferLifecycleOptions, "defaultEol"> = {},
): BufferRecord {
  const id = `untitled-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  return {
    id,
    path: null,
    text,
    language,
    dirty: text.length > 0,
    dirtyWithNoFile: false,
    readonly: false,
    encoding: "utf8",
    eol: options.defaultEol ?? "lf",
    hash: null,
    conflictMarkers: false,
    autoSave: "off",
  };
}

export function bufferFromFile(file: EditorFile, language: string): BufferRecord {
  return {
    id: file.path,
    path: file.path,
    text: file.text ?? "",
    language,
    dirty: false,
    dirtyWithNoFile: false,
    readonly: file.readonly,
    encoding: file.encoding,
    eol: file.eol.style,
    hash: file.hash,
    conflictMarkers: false,
    autoSave: "off",
  };
}

export function normalizeText(text: string, options: BufferLifecycleOptions = {}): string {
  let normalized = text.replace(/\r\n?/g, "\n");
  if (options.trimTrailingWhitespace) normalized = normalized.replace(/[ \t]+(?=\n|$)/g, "");
  if (options.insertFinalNewline && normalized.length > 0 && !normalized.endsWith("\n")) {
    normalized += "\n";
  }
  return normalized;
}

export function hasConflictMarkers(text: string): boolean {
  return /^(<<<<<<<|=======|>>>>>>>)(?: .*)?$/m.test(text);
}

export function shouldAutoSave(buffer: BufferRecord, trigger: Exclude<AutoSaveMode, "off">): boolean {
  return buffer.dirty && !buffer.readonly && !buffer.dirtyWithNoFile && !buffer.conflictMarkers && buffer.autoSave === trigger;
}

export function classifyDropPath(_path: string, isDirectory: boolean): "editor" | "workspace-root" {
  return isDirectory ? "workspace-root" : "editor";
}

export function markExternallyDeleted(buffer: BufferRecord): BufferRecord {
  return { ...buffer, dirty: true, dirtyWithNoFile: true };
}

export function updateBufferText(buffer: BufferRecord, text: string): BufferRecord {
  return { ...buffer, text, dirty: true, conflictMarkers: hasConflictMarkers(text) };
}

export function applySaveResult(buffer: BufferRecord, result: EditorWriteResult): BufferRecord {
  return {
    ...buffer,
    path: result.path,
    id: result.path,
    dirty: false,
    dirtyWithNoFile: false,
    hash: result.hash,
    encoding: result.encoding,
    eol: result.eol,
  };
}

export class BufferLifecycle {
  private readonly buffers = new Map<string, BufferRecord>();
  private readonly client: IpcClient;
  private readonly wal: BufferWal;
  private readonly dialog: SaveDialog;
  private readonly options: BufferLifecycleOptions;

  constructor(
    client: IpcClient,
    wal: BufferWal,
    dialog: SaveDialog,
    options: BufferLifecycleOptions = {},
  ) {
    this.client = client;
    this.wal = wal;
    this.dialog = dialog;
    this.options = options;
  }

  async restore(): Promise<BufferRecord[]> {
    const restored = await this.wal.load();
    restored.forEach((buffer) => this.buffers.set(buffer.id, buffer));
    return restored;
  }

  add(buffer: BufferRecord): BufferRecord {
    this.buffers.set(buffer.id, buffer);
    void this.wal.put(buffer);
    return buffer;
  }

  get(id: string): BufferRecord | undefined {
    return this.buffers.get(id);
  }

  update(buffer: BufferRecord): BufferRecord {
    this.buffers.set(buffer.id, buffer);
    void this.wal.put(buffer);
    return buffer;
  }

  async save(id: string): Promise<SaveOutcome | null> {
    const buffer = this.buffers.get(id);
    if (!buffer || !buffer.dirty || buffer.readonly) return null;
    const path = buffer.path ?? (await this.dialog.saveAs());
    if (!path) return null;
    if (this.hasPathCollision(path, buffer.id)) throw new Error(`EDITOR_PATH_COLLISION:${path}`);
    const file: Pick<EditorFile, "path" | "encoding" | "eol" | "hash"> = {
      path,
      encoding: buffer.encoding,
      eol: { style: buffer.eol, lf_count: 0, crlf_count: 0 },
      hash: buffer.hash ?? "",
    };
    const result = await writeEditorFile(this.client, file, normalizeText(buffer.text, this.options));
    const saved = applySaveResult(buffer, result);
    this.buffers.delete(id);
    this.buffers.set(saved.id, saved);
    await this.wal.remove(id);
    await this.wal.put(saved);
    return { buffer: saved, result };
  }

  async saveAll(): Promise<{ saved: SaveOutcome[]; errors: Array<{ id: string; error: unknown }> }> {
    const saved: SaveOutcome[] = [];
    const errors: Array<{ id: string; error: unknown }> = [];
    for (const id of this.buffers.keys()) {
      try {
        const outcome = await this.save(id);
        if (outcome) saved.push(outcome);
      } catch (error) {
        errors.push({ id, error });
      }
    }
    return { saved, errors };
  }

  async saveAs(id: string): Promise<SaveOutcome | null> {
    const buffer = this.buffers.get(id);
    if (!buffer) return null;
    return this.save(id);
  }

  private hasPathCollision(path: string, excludingId: string): boolean {
    return [...this.buffers.values()].some((buffer) => buffer.id !== excludingId && buffer.path === path);
  }
}