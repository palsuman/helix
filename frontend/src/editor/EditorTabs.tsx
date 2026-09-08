import { useCallback, useEffect, useRef, useState } from "react";
import { useMessage } from "../localization";
import type { IpcClient } from "../ipc";
import { EditorEntry } from "./EditorEntry";
import {
  EDITOR_TABS_STORAGE_KEY,
  closeAllEditorTabs,
  closeEditorTab,
  closeEditorTabsToRight,
  closeOtherEditorTabs,
  openEditorTab,
  parseEditorTabs,
  promoteEditorTab,
  reorderEditorTabs,
  setEditorTabDirty,
  toggleEditorTabPinned,
  type EditorTabState,
} from "./tabs";
import "./tabs.css";

interface EditorTabsProps {
  client: IpcClient;
  initialPath?: string;
  onSplit?: () => void;
}

const EMPTY_STATE: EditorTabState = { tabs: [], activeId: null };

export function EditorTabs({ client, initialPath, onSplit }: EditorTabsProps) {
  const t = useMessage();
  const [state, setState] = useState<EditorTabState>(() => restoreState(initialPath));
  const saveHandlers = useRef(new Map<string, () => Promise<void>>());
  const [pendingCloseIds, setPendingCloseIds] = useState<string[]>([]);
  const update = useCallback((next: EditorTabState) => setState(next), []);

  useEffect(() => {
    localStorage.setItem(EDITOR_TABS_STORAGE_KEY, JSON.stringify(state));
  }, [state]);

  const activeTab = state.tabs.find((tab) => tab.id === state.activeId) ?? null;
  const closeTab = useCallback(
    (id: string) => {
      const tab = state.tabs.find((candidate) => candidate.id === id);
      if (!tab) return;
      if (tab.dirty) {
        setPendingCloseIds([id]);
        return;
      }
      update(closeEditorTab(state, id));
    },
    [setPendingCloseIds, state, update],
  );
  const closeAll = useCallback(() => {
    const dirtyIds = state.tabs.filter((tab) => tab.dirty).map((tab) => tab.id);
    if (dirtyIds.length > 0) {
      setPendingCloseIds(dirtyIds);
      return;
    }
    update(closeAllEditorTabs());
  }, [setPendingCloseIds, state, update]);
  const pendingCloseId = pendingCloseIds[0] ?? null;
  const finishPendingClose = useCallback(
    (id: string) => {
      const nextIds = pendingCloseIds.slice(1);
      const nextState = closeEditorTab(state, id);
      update(nextState);
      setPendingCloseIds(nextIds);
    },
    [pendingCloseIds, setPendingCloseIds, state, update],
  );
  const dirtyChange = useCallback(
    (id: string, dirty: boolean) => update(setEditorTabDirty(state, id, dirty)),
    [state, update],
  );

  return (
    <section className="editor-tabs" aria-label={t("editorTabs")}>
      <div className="editor-tabs__bar" role="tablist" aria-label={t("editorTabs")}>
        {state.tabs.map((tab) => (
          <button
            aria-selected={tab.id === state.activeId}
            className={`editor-tab${tab.id === state.activeId ? " is-active" : ""}${tab.pinned ? " is-pinned" : ""}${tab.preview ? " is-preview" : ""}`}
            draggable
            key={tab.id}
            onClick={() => update({ ...state, activeId: tab.id })}
            onDoubleClick={() => update(promoteEditorTab(state, tab.id))}
            onDragOver={(event) => event.preventDefault()}
            onDrop={(event) => update(reorderEditorTabs(state, event.dataTransfer.getData("text/plain"), tab.id))}
            onDragStart={(event) => event.dataTransfer.setData("text/plain", tab.id)}
            role="tab"
            title={tab.path}
            type="button"
          >
            <span className="editor-tab__label">{tab.path.split("/").pop() || tab.path}</span>
            {tab.dirty && (
              <span aria-label={t("editorModified")} className="editor-tab__dirty">
              </span>
            )}
            <span
              aria-label={t("editorCloseTab")}
              className="editor-tab__close"
              onClick={(event) => {
                event.stopPropagation();
                closeTab(tab.id);
              }}
              role="button"
              tabIndex={0}
            >
            </span>
          </button>
        ))}
        {state.tabs.length > 0 && (
          <details className="editor-tabs__overflow">
            <summary aria-label={t("editorTabOverflow")} />
            <div role="menu">
              {state.tabs.map((tab) => (
                <button key={tab.id} onClick={() => update({ ...state, activeId: tab.id })} role="menuitem" type="button">
                  {tab.path}
                </button>
              ))}
            </div>
          </details>
        )}
      </div>
      {activeTab && (
        <details className="editor-tabs__actions">
          <summary aria-label={t("editorTabActions")} />
          <div role="menu">
            <button onClick={() => update(toggleEditorTabPinned(state, activeTab.id))} type="button">{t("editorPinTab")}</button>
            <button onClick={() => update(closeOtherEditorTabs(state, activeTab.id))} type="button">{t("editorCloseOthers")}</button>
            <button onClick={() => update(closeEditorTabsToRight(state, activeTab.id))} type="button">{t("editorCloseToRight")}</button>
            <button onClick={closeAll} type="button">{t("editorCloseAll")}</button>
            <button onClick={() => closeTab(activeTab.id)} type="button">{t("editorCloseTab")}</button>
            {onSplit && <button onClick={onSplit} type="button">{t("editorSplitRight")}</button>}
          </div>
        </details>
      )}
      <div className="editor-tabs__views">
        {state.tabs.map((tab) => (
          <div className="editor-tabs__view" hidden={tab.id !== state.activeId} key={tab.id}>
            <EditorEntry
              client={client}
              onDirtyChange={(dirty) => dirtyChange(tab.id, dirty)}
              onSaveReady={(save) => {
                if (save) saveHandlers.current.set(tab.id, save);
                else saveHandlers.current.delete(tab.id);
              }}
              path={tab.path}
            />
          </div>
        ))}
        {!activeTab && <p className="workbench-placeholder">{t("editorNoOpenFiles")}</p>}
      </div>
      {pendingCloseId && (
        <div className="editor-tabs__prompt-backdrop">
          <section aria-labelledby="editor-close-title" aria-modal="true" className="editor-tabs__prompt" role="dialog">
            <h2 id="editor-close-title">{t("editorConfirmCloseTitle")}</h2>
            <p>{t("editorConfirmCloseDirty", { path: state.tabs.find((tab) => tab.id === pendingCloseId)?.path ?? "" })}</p>
            <div>
              <button
                onClick={async () => {
                  await saveHandlers.current.get(pendingCloseId)?.();
                  finishPendingClose(pendingCloseId);
                }}
                type="button"
              >{t("commonSave")}</button>
              <button onClick={() => finishPendingClose(pendingCloseId)} type="button">{t("editorDontSave")}</button>
              <button onClick={() => setPendingCloseIds([])} type="button">{t("commonCancel")}</button>
            </div>
          </section>
        </div>
      )}
    </section>
  );
}

function restoreState(initialPath?: string): EditorTabState {
  if (typeof localStorage !== "undefined") {
    try {
      const restored = parseEditorTabs(JSON.parse(localStorage.getItem(EDITOR_TABS_STORAGE_KEY) ?? "null"));
      if (restored) return initialPath ? openEditorTab(restored, initialPath) : restored;
    } catch {
      localStorage.removeItem(EDITOR_TABS_STORAGE_KEY);
    }
  }
  return initialPath ? openEditorTab(EMPTY_STATE, initialPath) : EMPTY_STATE;
}