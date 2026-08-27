import { getCurrentWindow } from "@tauri-apps/api/window";
import type { IpcClient } from "./ipc";
import { ipc } from "./ipc";
import type { StreamClient } from "./stream";
import { stream } from "./stream";
import { RecoveryOverlay, type SupervisorClient, supervisor } from "./supervisor";
import { ThemeService, applyMonacoTheme, useTheme } from "./theme";
import { WorkbenchShell } from "./workbench";

export interface AppProps {
  client?: IpcClient;
  /** Retained for callers that inject the shared stream client; feature views consume it later. */
  streamClient?: StreamClient;
  supervisorClient?: SupervisorClient;
  /** Native Tauri label; injectable for browser tests and embedders. */
  windowId?: string;
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

/** Production application root: visual shell only; later tasks register all feature content. */
function App({
  client = ipc,
  streamClient = stream,
  supervisorClient = supervisor,
  windowId = nativeWindowId(),
}: AppProps) {
  useTheme(streamClient, themeService);
  return (
    <WorkbenchShell
      client={client}
      windowId={windowId}
      overlay={<RecoveryOverlay client={supervisorClient} />}
      activities={[]}
      panels={[]}
      leftPanel={null}
      rightPanel={null}
      editor={null}
      showLayoutControls={false}
    />
  );
}

export default App;
