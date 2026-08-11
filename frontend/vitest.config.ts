import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Vitest config (Task 3.2 sets up the full component test framework with
// mock IPC/WebSocket clients; this is the minimal scaffold needed for
// Task 1.1's CI type-check/lint/test gate).
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
  },
});
