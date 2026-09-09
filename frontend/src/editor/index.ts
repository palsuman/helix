export { EditorSurface, type EditorRecovery, type EditorSurfaceProps } from "./EditorSurface";
export { DEFAULT_EDITOR_SETTINGS, type EditorSettings } from "./settings";
export { EditorEntry } from "./EditorEntry";
export { EditorTabs } from "./EditorTabs";
export { navigateEditor, onEditorNavigation, type EditorNavigationRequest } from "./navigation";
export {
  BufferLifecycle,
  applySaveResult,
  bufferFromFile,
  classifyDropPath,
  createUntitledBuffer,
  hasConflictMarkers,
  markExternallyDeleted,
  normalizeText,
  shouldAutoSave,
  updateBufferText,
  type AutoSaveMode,
  type BufferRecord,
  type BufferWal,
  type SaveDialog,
} from "./lifecycle";
export {
  FILE_READ_COMMAND,
  FILE_WRITE_COMMAND,
  readEditorFile,
  writeEditorFile,
  type EditorFile,
  type EditorWriteResult,
} from "./fileService";
