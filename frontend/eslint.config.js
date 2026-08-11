import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import babelParser from "@babel/eslint-parser";

// Flat ESLint config (Task 1.1).
//
// NOTE on typescript-eslint: it is deliberately NOT included here.
// typescript-eslint@8.66.0 (the latest at scaffold time) hard-refuses to
// load against TypeScript 7.x — see
// https://github.com/typescript-eslint/typescript-eslint/issues/10940 — and
// the project is pinned to typescript@7.0.2 per the design document's
// technology stack table. TS/TSX files are parsed here with
// @babel/eslint-parser (syntax-only, no type-aware rules) so ESLint can at
// least parse and lint JS-level concerns; full type safety is enforced
// separately by `tsc -b` (`npm run typecheck`) using the native TS7
// compiler. Re-add typescript-eslint to this config, and its type-aware
// rules, once upstream ships TS7 support.
//
// The localization no-literal-string rule (Task 2.9) and icon-label rules
// (Task 2.5) are added when those subsystems land.
export default [
  { ignores: ["dist", "src-tauri", "src/generated"] },
  js.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: { ...globals.browser, ...globals.node },
      parser: babelParser,
      parserOptions: {
        requireConfigFile: false,
        babelOptions: {
          presets: ["@babel/preset-react", "@babel/preset-typescript"],
        },
      },
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      // Babel's parser doesn't understand TS-only constructs the way
      // typescript-eslint's rules expect; disable the JS rules that
      // otherwise false-positive on TS type-only imports/exports, type
      // parameters, interface members, and `as const`. Undefined
      // identifiers and unused bindings in TS/TSX are caught by `tsc -b`
      // (`npm run typecheck`) with full type information instead.
      "no-unused-vars": "off",
      "no-undef": "off",
    },
  },
  {
    files: ["**/*.{js,jsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.node,
    },
  },
];
