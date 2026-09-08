import { lazy, useEffect, useMemo } from "react";
import { useStore } from "zustand";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { CommandPalette, CommandRegistry, registerWorkbenchCommandHandlers } from "./commands";
import type { IpcClient } from "./ipc";
import { ipc } from "./ipc";
import type { StreamClient } from "./stream";
import { stream } from "./stream";
import { RecoveryOverlay, type SupervisorClient, supervisor } from "./supervisor";
import { ThemeService, applyMonacoTheme, useTheme } from "./theme";
import { injectIconStyles } from "./icons";
import { NotificationCenter, NotificationCenterButton, NotificationToasts } from "./notifications";
import { WorkbenchShell, useLayoutStore } from "./workbench";
import { KeybindingEditor, KeybindingService, displayShortcut } from "./keybindings";
import { LocalizationProvider, useMessage } from "./localization";

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

function LocalizedWorkbench({
  client = ipc,
  streamClient = stream,
  supervisorClient = supervisor,
  windowId = nativeWindowId(),
  editorPath,
}: AppProps) {
  const t = useMessage();
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
        </>
      }
      activities={[]}
      panels={[]}
      leftPanel={null}
      rightPanel={<NotificationCenter onClose={() => setNotificationsOpen(false)} />}
      rightActivityRail={
        <NotificationCenterButton
          open={notificationsOpen}
          onToggle={() => setNotificationsOpen(!notificationsOpen)}
        />
      }
      editor={
        bindingState.editorOpen ? (
          <KeybindingEditor service={keybindings} />
        ) : (
          <LazyEditorTabs
            client={client}
            initialPath={editorPath}
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
