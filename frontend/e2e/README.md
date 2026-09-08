# Native E2E Framework

From the repository root:

```sh
npm --prefix frontend ci
npm --prefix frontend run e2e:build
npm --prefix frontend run test:e2e
```

Use Node 24.15.0 and the repository's pinned Rust toolchain, plus normal Tauri
platform prerequisites. Linux needs a desktop session or `xvfb-run -a` around
the test command. The build is separate from the test runtime budget.

## Isolation

The explicit `e2e` Cargo feature enables both WDIO Rust plugins. Normal builds
do not activate either dependency. The E2E-only Tauri config grants plugin
permissions and global Tauri APIs; it is not merged into production builds.
Vite includes the frontend WDIO plugin only when `VITE_HELIX_WDIO_E2E=1`.
Never distribute feature-enabled E2E executables.

`e2e:build` writes frontend assets to `frontend/dist-e2e` and Rust binaries to
`target/e2e/debug`, keeping them separate from ordinary release outputs. The
kernel is launched from that same isolated build directory. No dev server is
needed. The service launches the app with temporary user settings/state/cache
directories under `target/e2e-artifacts/run-*`. No test reads personal settings.

The embedded WebDriver provider is selected explicitly on every platform; no
external `tauri-driver`, EdgeDriver, or CrabNebula service is launched. A free
loopback port is allocated and propagated to both the launcher and worker's
`TAURI_WEBDRIVER_PORT`, which the direct-execution API also needs. One worker
owns one native app. Port allocation has the usual close-before-bind race;
startup errors fail the run rather than attaching to another application.

## Journeys and Helpers

Add `*.e2e.mjs` journeys and list them in `wdio.conf.mjs`. Mocha drives the real
native WebView, separate from Vitest's component tests.

- `waitForWorkbench` waits for rendered shell and running supervisor.
- `invokeKernel` verifies correlation IDs and error envelopes over real IPC.
- `mockKernelCommands` replaces selected domain commands inside `ipc_dispatch`.
  Its restore callback belongs in `finally`. Unmatched commands use Tauri's
  internal invoke transport to bypass the plugin's intercepted global wrapper.
  This relies on the pinned Tauri/WDIO integration and is covered by a real
  passthrough assertion. Use one mock map at a time; nested mocks are unsupported.
- `captureFailure` attempts a screenshot, DOM source, and kernel log query
  independently. A failed capture writes an error file without hiding the test
  failure. The service also captures frontend/backend logs; isolated on-disk
  kernel logs remain available if IPC is unavailable.
- `closeCleanly` invokes the native close command, verifies the feature-gated
  receipt confirms kernel shutdown, and waits for the host PID to disappear.
  Only then does it bypass redundant WebDriver session deletion, since the
  embedded server has exited with the app. Call this last, with the main window
  as the only remaining window. Failure paths retain the service's usual cleanup.

The smoke journey renders landmarks, collapses and restores the right panel,
checks real IPC, mocks one response, verifies passthrough and restoration, and
closes cleanly. Set `HELIX_E2E_SCREENSHOT=1` to retain a successful workbench
screenshot. Baseline pixel comparison is intentionally not enabled yet; it is
optional in task 3.3 and requires platform-specific baselines.

## CI

The Native E2E workflow runs after pushes/merges to `main` or `master`, or by
manual dispatch, on Windows, macOS, and Linux. Each suite has a ten-minute
runtime timeout after compilation. Diagnostics are retained for 14 days.
Successful runs prune application state while retaining logs and the shutdown
receipt; failed runs retain the isolated directory for diagnosis.

The initial journey has been run locally on macOS. Windows and Linux must be
confirmed by the CI matrix; configuring those jobs is not evidence they passed.
