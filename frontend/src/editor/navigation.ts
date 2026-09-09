export interface EditorNavigationRequest {
  groupId: string;
  path?: string;
  line?: number;
}

type Listener = (request: EditorNavigationRequest) => void;
const listeners = new Set<Listener>();

/** Renderer-local bridge from workbench pickers into independently mounted editor groups. */
export function navigateEditor(request: EditorNavigationRequest): void {
  for (const listener of listeners) listener(request);
}

export function onEditorNavigation(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
