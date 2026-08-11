import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";

declare const __HELIX_IPC_E2E__: boolean;

if (__HELIX_IPC_E2E__) {
  void import("./ipc/e2e").then(({ runIpcE2e }) => runIpcE2e());
} else {
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
