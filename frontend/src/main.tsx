import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";

declare const __HELIX_IPC_E2E__: boolean;
declare const __HELIX_WDIO_E2E__: boolean;

async function start() {
  if (__HELIX_WDIO_E2E__) await import("@wdio/tauri-plugin");
  if (__HELIX_IPC_E2E__) {
    const { runIpcE2e } = await import("./ipc/e2e");
    await runIpcE2e();
    return;
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void start();
