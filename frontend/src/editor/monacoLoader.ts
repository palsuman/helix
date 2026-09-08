import type { MonacoLoader } from "./EditorSurface";

export const defaultMonacoLoader: MonacoLoader = async () =>
  (await import("monaco-editor/esm/vs/editor/editor.api")) as unknown as Awaited<
    ReturnType<MonacoLoader>
  >;