# Component Test Framework

Run from the repository root:

```sh
npm --prefix frontend test
npm --prefix frontend run test:watch
npm --prefix frontend run test:coverage
npm --prefix frontend test -- src/App.test.tsx
```

Tests use Vitest, jsdom, React Testing Library, and jest-dom matchers from
`setup.ts`. Import `render`, `screen`, `act`, and interaction helpers directly
from Testing Library. Prefer accessible roles and names when asserting rendered
behavior. Testing Library automatically cleans up rendered trees after each test.
Create transport fixtures per test and reset any Zustand stores the test uses.

## Mock IPC

`createMockIpc()` injects a fake transport into the production `IpcClient`.
`respond<Response>(command, value)` sets a constant response;
`handle<Request, Response>(command, handler)` supports payload-dependent or async
responses. Prefer generated request and response types from `src/generated`.
`fail(command, appError(...))` returns a kernel error envelope, preserving its
category through the real client's error mapping. Registering a handler or
response clears the configured failure.

The fixture records `requests` and `commands` and assigns deterministic unique
correlation IDs. Missing commands reject instead of silently succeeding. This
helper mocks command dispatch, not kernel execution or cancellation; use the
lower-level `InvokeFn` injection in the IPC client tests for cancellation scenarios.

`App.test.tsx` demonstrates rendering the production workbench with a typed mocked
layout response, then asserting the restored panel visibility and size. Await
asynchronous rendering using `act`, `findByRole`, or `waitFor` before assertions.

## Mock Streaming

`createMockStream()` injects `MockSocket` into the production `StreamClient`.
Call `client.connect()` inside async `act` to settle endpoint resolution, then
`latest().open()` inside `act`. No network connection is created. An optional
`resolveEndpoint` callback supports endpoint errors or IPC-driven discovery.

Use `emitData(channel, sequence, payload)`, `emitControl(control)`, `emitRaw(data)`,
and `die()` to drive server behavior. Inspect `controls()` to assert subscription
and resume cursors. `mockStream.test.tsx` demonstrates rendered updates, gap
notifications, reconnection, channel switching, and unmount cleanup.

Use fake timers for reconnect/heartbeat tests; advance them inside `act`.
Unmount the component, call `dispose()` to stop client timers, and restore real
timers in teardown. The shared socket is also used by `stream/client.test.ts`.

## Accessibility

Run the focused accessibility harness with `npm run test:a11y`. It registers
`vitest-axe` in the shared setup and provides reusable assertions for axe
violations, tab order, keyboard activation, modal focus traps, focus
restoration, and WCAG contrast ratios. Keep the manual screen-reader pass in
`screen-reader-checklist.md` alongside component changes.

## Coverage and CI

The coverage command enforces a global 70% minimum for statements, branches,
functions, and lines. All application TS/TSX files are included, even when no
test imports them. Exclusions are generated contracts, declarations, test files
and helpers, the DOM mount entry point, and the separate Tauri IPC E2E surface.
Do not exclude application modules simply to pass the gate.

Reports are written to `frontend/coverage`: terminal summary, HTML, LCOV, and
JSON summary. Generated reports are ignored by Git, ESLint, and Prettier.
The frontend CI workflow runs lint, type-checking, and coverage on every push
and pull request and retains coverage artifacts for 14 days. It uses Node
24.15.0, satisfying the installed jsdom package's Node requirement; local Node
22 users need at least 22.22.2.
