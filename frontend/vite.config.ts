import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite config for the Helix frontend, tuned for Tauri dev (Task 1.1).
// See design.md "Technology Stack" for the pinned dependency policy.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Ignore Rust build artifacts so file changes there don't trigger
      // frontend HMR.
      ignored: ["**/src-tauri/**", "**/target/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    // safari14.1 rather than the Tauri template's safari13: esbuild's compat
    // table marks destructuring unsupported below Safari 14.1 and refuses to
    // transpile it, which rules out ordinary React hook syntax (`const [a,
    // setA] = useState()`). Safari 14.1 is available on macOS 10.15.7, so
    // this stays within Tauri 2's minimum supported macOS (10.15).
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari14.1",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
