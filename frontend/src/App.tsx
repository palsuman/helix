import { lazy, useEffect, useMemo, useState } from "react";
import { useStore } from "zustand";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { CommandPalette, CommandRegistry, registerWorkbenchCommandHandlers } from "./commands";
import type { IpcClient } from "./ipc";
import { ipc } from "./ipc";
import type { StreamClient } from "./stream";
import { stream } from "./stream";
import { RecoveryOverlay, type SupervisorClient, supervisor } from "./supervisor";
import { ThemeService, applyMonacoTheme, useTheme } from "./theme";
import { Icon, injectIconStyles } from "./icons";
import { ClosePrimaryPanelButton } from "./workbench/ClosePrimaryPanelButton";
import { FileExplorer } from "./explorer/FileExplorer";
import { NotificationCenter, NotificationCenterButton, NotificationToasts } from "./notifications";
import { WorkbenchShell, useLayoutStore } from "./workbench";
import { KeybindingEditor, KeybindingService, displayShortcut } from "./keybindings";
import { LocalizationProvider, useMessage } from "./localization";
import { QuickOpen } from "./quickopen";
import { navigateEditor } from "./editor/navigation";
import { WorkspaceSearchPanel } from "./search";
import { listWorkspaces } from "./workbench/commands";
import type { WorkspaceRoot } from "./generated/WorkspaceRoot";
import { detectPlatform } from "./keybindings/schemes";

const LazyEditorTabs = lazy(() =>
  import("./editor/EditorTabs").then((module) => ({ default: module.EditorTabs })),
);

export interface AppProps {
  client?: IpcClient;
  /** Retained for callers that inject the shared stream client; feature views consume it later. */
  streamClient?: StreamClient;
  supervisorClient?: SupervisorClient;
  /** Native Tauri label; injectable for browser tests and embedders. */
  windowId?: string;
  /** Optional file path to open in the primary editor group. */
  editorPath?: string;
}

function nativeWindowId() {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

/** One theme service per application; the browser reuses it across mounts. */
const themeService = new ThemeService({
  onApply: (_name, resolved) => applyMonacoTheme(resolved),
});

function setNotificationsOpen(open: boolean) {
  const layout = useLayoutStore.getState();
  if (layout.primarySidebarPosition === "right") layout.setPrimarySidebarVisible(open);
  else layout.setSecondarySidebarVisible(open);
}

function WorkspaceSearchActivity({
  client,
  fallbackPath,
}: {
  client: IpcClient;
  fallbackPath?: string;
}) {
  const t = useMessage();
  const [root, setRoot] = useState(() => fallbackPath?.split("/").slice(0, -1).join("/") ?? "");

  useEffect(() => {
    let active = true;
    void listWorkspaces(client).then((response) => {
      if (!active) return;
      const next = response.workspaces
        .flatMap((workspace) => workspace.roots)
        .find((candidate: WorkspaceRoot) => candidate.availability === "available")?.path;
      if (next) setRoot(next);
    });
    return () => {
      active = false;
    };
  }, [client]);

  if (!root)
    return (
      <section className="workbench-sidebar-view" aria-label={t("workspaceSearchTitle")}>
        <header className="workbench-view-header">
          <h2>{t("workspaceSearchTitle")}</h2>
          <ClosePrimaryPanelButton />
        </header>
        <div className="workbench-view-empty">
          <Icon id="search" size="lg" />
          <p>{t("workspaceSearchNoWorkspace")}</p>
        </div>
      </section>
    );
  return (
    <WorkspaceSearchPanel
      key={root}
      client={client}
      root={root}
      onOpenMatch={(path, line) =>
        navigateEditor({ groupId: useLayoutStore.getState().activeEditorGroup, path, line })
      }
    />
  );
}

function LocalizedWorkbench({
  client = ipc,
  streamClient = stream,
  supervisorClient = supervisor,
  windowId = nativeWindowId(),
  editorPath,
}: AppProps) {
  const t = useMessage();
  const isMac = detectPlatform() === "mac";
  useTheme(streamClient, themeService);
  injectIconStyles();
  const notificationsOpen = useLayoutStore((layout) =>
    layout.primarySidebarPosition === "right"
      ? layout.primarySidebarVisible
      : layout.secondarySidebarVisible,
  );
  const zenMode = useLayoutStore((layout) => layout.zenMode);
  const commandRegistry = useMemo(() => {
    const registry = new CommandRegistry(client, windowId);
    registerWorkbenchCommandHandlers(registry);
    return registry;
  }, [client, windowId]);
  const keybindings = useMemo(
    () => new KeybindingService(client, commandRegistry, windowId),
    [client, commandRegistry, windowId],
  );
  const bindingState = useStore(keybindings.store);
  const commandContext = useStore(keybindings.context.store);
  useEffect(() => keybindings.start(), [keybindings]);
  return (
    <WorkbenchShell
      client={client}
      windowId={windowId}
      titleBar={
        <div className={`workbench-appbar${isMac ? " workbench-appbar--mac" : ""}`} data-tauri-drag-region>
          {!isMac && (
            <img
              className="workbench-brand-logo"
              src="/helix-logo.svg"
              alt={t("helixLogo")}
              width={30}
              height={30}
              draggable={false}
            />
          )}
          {!isMac && <span className="workbench-appbar-title">{t("helixLogo")}</span>}
          {!isMac && (
            <div className="workbench-window-controls">
              <button
                type="button"
                className="workbench-window-control"
                aria-label="Minimize"
                onClick={() => getCurrentWindow().minimize()}
              >
                <svg width="10" height="1" viewBox="0 0 10 1" fill="currentColor">
                  <rect width="10" height="1" />
                </svg>
              </button>
              <button
                type="button"
                className="workbench-window-control"
                aria-label="Maximize"
                onClick={() => getCurrentWindow().toggleMaximize()}
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1">
                  <rect x="0.5" y="0.5" width="9" height="9" />
                </svg>
              </button>
              <button
                type="button"
                className="workbench-window-control workbench-window-control--close"
                aria-label="Close"
                onClick={() => getCurrentWindow().close()}
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2">
                  <path d="M1 1L9 9M9 1L1 9" />
                </svg>
              </button>
            </div>
          )}
        </div>
      }
      overlay={
        <>
          <RecoveryOverlay client={supervisorClient} />
          <NotificationToasts centerVisible={notificationsOpen && !zenMode} />
          <CommandPalette
            registry={commandRegistry}
            context={commandContext}
            shortcutFor={(command) => {
              const key = keybindings.shortcutFor(command);
              return key ? displayShortcut(key, keybindings.platform) : null;
            }}
          />
          <QuickOpen
            client={client}
            registry={commandRegistry}
            context={commandContext}
            onOpen={(path, split, line) => {
              if (split) useLayoutStore.getState().splitEditor("horizontal");
              const groupId = useLayoutStore.getState().activeEditorGroup;
              navigateEditor({ groupId, path, ...(line === undefined ? {} : { line }) });
            }}
            onLine={(line) =>
              navigateEditor({ groupId: useLayoutStore.getState().activeEditorGroup, line })
            }
          />
        </>
      }
      panels={[]}
      explorerView={<FileExplorer client={client} streamClient={streamClient} />}
      searchView={<WorkspaceSearchActivity client={client} fallbackPath={editorPath} />}
      rightPanel={<NotificationCenter onClose={() => setNotificationsOpen(false)} />}
      rightActivityRail={
        <NotificationCenterButton
          open={notificationsOpen}
          onToggle={() => setNotificationsOpen(!notificationsOpen)}
        />
      }
      editor={(groupId, index) =>
        bindingState.editorOpen && index === 0 ? (
          <KeybindingEditor service={keybindings} />
        ) : (
          <LazyEditorTabs
            client={client}
            groupId={groupId}
            initialPath={index === 0 ? editorPath : undefined}
            onSplit={() => useLayoutStore.getState().splitEditor("horizontal")}
          />
        )
      }
      statusLeft={[
        {
          id: "keybinding-chord",
          content:
            bindingState.pending.length > 0 ? (
              <span role="status">
                {t("keybindingsChordPending", {
                  shortcut: displayShortcut(bindingState.pending.join(" "), keybindings.platform),
                })}
              </span>
            ) : null,
        },
      ]}
      showLayoutControls={false}
    />
  );
}

function App(props: AppProps) {
  const client = props.client ?? ipc;
  const streamClient = props.streamClient ?? stream;
  return (
    <LocalizationProvider client={client} stream={streamClient}>
      <LocalizedWorkbench {...props} client={client} streamClient={streamClient} />
    </LocalizationProvider>
  );
}

export default App;
