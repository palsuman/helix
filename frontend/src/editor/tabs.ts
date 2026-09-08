export interface EditorTab {
  id: string;
  path: string;
  pinned: boolean;
  preview: boolean;
  dirty: boolean;
}

export interface EditorTabState {
  tabs: EditorTab[];
  activeId: string | null;
}

export const EDITOR_TABS_STORAGE_KEY = "helix.editor.tabs";

export function createEditorTab(path: string, options: Partial<EditorTab> = {}): EditorTab {
  return { id: path, path, pinned: false, preview: false, dirty: false, ...options };
}

export function openEditorTab(state: EditorTabState, path: string): EditorTabState {
  const existing = state.tabs.find((tab) => tab.path === path);
  if (existing) return { ...state, activeId: existing.id };
  const preview = state.tabs.find((tab) => tab.preview && !tab.dirty && !tab.pinned);
  const nextTab = createEditorTab(path, { preview: true });
  const tabs = preview
    ? state.tabs.map((tab) => (tab.id === preview.id ? nextTab : tab))
    : [...state.tabs, nextTab];
  return { tabs, activeId: nextTab.id };
}

export function promoteEditorTab(state: EditorTabState, id: string): EditorTabState {
  return {
    ...state,
    tabs: state.tabs.map((tab) => (tab.id === id ? { ...tab, preview: false } : tab)),
    activeId: id,
  };
}

export function setEditorTabDirty(state: EditorTabState, id: string, dirty: boolean): EditorTabState {
  return {
    ...state,
    tabs: state.tabs.map((tab) => (tab.id === id ? { ...tab, dirty, preview: dirty ? false : tab.preview } : tab)),
  };
}

export function toggleEditorTabPinned(state: EditorTabState, id: string): EditorTabState {
  const tab = state.tabs.find((candidate) => candidate.id === id);
  if (!tab) return state;
  return {
    ...state,
    tabs: sortPinnedTabs(
      state.tabs.map((candidate) =>
        candidate.id === id ? { ...candidate, pinned: !candidate.pinned, preview: false } : candidate,
      ),
    ),
  };
}

export function reorderEditorTabs(state: EditorTabState, sourceId: string, targetId: string): EditorTabState {
  if (sourceId === targetId) return state;
  const source = state.tabs.find((tab) => tab.id === sourceId);
  const target = state.tabs.find((tab) => tab.id === targetId);
  if (!source || !target || source.pinned !== target.pinned) return state;
  const tabs = state.tabs.filter((tab) => tab.id !== sourceId);
  tabs.splice(tabs.findIndex((tab) => tab.id === targetId), 0, source);
  return { ...state, tabs };
}

export function closeEditorTab(state: EditorTabState, id: string): EditorTabState {
  const index = state.tabs.findIndex((tab) => tab.id === id);
  if (index < 0) return state;
  const tabs = state.tabs.filter((tab) => tab.id !== id);
  if (state.activeId !== id) return { tabs, activeId: state.activeId };
  const replacement = tabs[index] ?? tabs[index - 1] ?? null;
  return { tabs, activeId: replacement?.id ?? null };
}

export function closeOtherEditorTabs(state: EditorTabState, id: string): EditorTabState {
  if (!state.tabs.some((tab) => tab.id === id)) return state;
  return { tabs: state.tabs.filter((tab) => tab.id === id || tab.pinned), activeId: id };
}

export function closeAllEditorTabs(): EditorTabState {
  return { tabs: [], activeId: null };
}

export function closeEditorTabsToRight(state: EditorTabState, id: string): EditorTabState {
  const index = state.tabs.findIndex((tab) => tab.id === id);
  if (index < 0) return state;
  const tabs = state.tabs.slice(0, index + 1).concat(state.tabs.slice(index + 1).filter((tab) => tab.pinned));
  return { tabs, activeId: tabs.some((tab) => tab.id === state.activeId) ? state.activeId : id };
}

export function sortPinnedTabs(tabs: EditorTab[]): EditorTab[] {
  return [...tabs].sort((left, right) => Number(right.pinned) - Number(left.pinned));
}

export function parseEditorTabs(value: unknown): EditorTabState | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<EditorTabState>;
  if (!Array.isArray(candidate.tabs) || candidate.tabs.some((tab) => !isEditorTab(tab))) return null;
  const tabs = sortPinnedTabs(candidate.tabs);
  if (candidate.activeId !== null && typeof candidate.activeId !== "string") return null;
  return { tabs, activeId: tabs.some((tab) => tab.id === candidate.activeId) ? candidate.activeId! : tabs[0]?.id ?? null };
}

function isEditorTab(value: unknown): value is EditorTab {
  if (!value || typeof value !== "object") return false;
  const tab = value as Partial<EditorTab>;
  return typeof tab.id === "string" && typeof tab.path === "string" && typeof tab.pinned === "boolean" && typeof tab.preview === "boolean" && typeof tab.dirty === "boolean";
}