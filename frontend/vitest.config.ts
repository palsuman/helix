import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
export default defineConfig({
  plugins: [react()],
  optimizeDeps: { exclude: ["monaco-editor"] },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/generated/**",
        "src/test/**",
        "src/**/*.test.{ts,tsx}",
        "src/**/*.d.ts",
        "src/**/testUtils.ts",
        "src/main.tsx",
        "src/ipc/e2e.tsx",
      ],
      reporter: ["text-summary", "html", "lcov", "json-summary"],
      thresholds: { statements: 70, branches: 70, functions: 70, lines: 70 },
    },
  },
});
