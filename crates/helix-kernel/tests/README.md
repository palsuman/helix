# Rust Integration Framework

Run the focused suite from the repository root:

```sh
cargo test -p helix-kernel --test framework_integration
```

The helpers in `support/mod.rs` launch Cargo's real `helix-kernel` binary,
including its production service container. Tests communicate through the
authenticated loopback RPC transport and the real WebSocket server. No Tauri
window, Node installation, or external tool installation is needed.

## Helpers

- `TestWorkspace::new`, `write`, and `assert_file` create and populate an isolated
  temporary checkout and assert its contents. Population rejects absolute paths
  and parent traversal; use these helpers only with test-owned directories.
- `TestWorkspace::mock_nx` installs a workspace-local shell/batch stand-in that
  returns a deterministic affected-project result. `assert_mock_nx_called`
  confirms that the kernel actually invoked it rather than taking a fallback.
- `KernelHarness::start` waits for authenticated transport readiness and verifies
  the child PID and epoch. User configuration, state, cache, and temporary paths
  are redirected only in the child environment, never process-global test state.
  PATH is isolated; tests must explicitly install any external tool fixtures.
- `command::<Request, Response>` serializes requests, assigns unique correlation
  IDs, checks response IDs and errors, and deserializes the expected response
  type. The public `ipc` client supports negative protocol and cancellation tests.
- `StreamClient::subscribe` discovers the endpoint over RPC and awaits the
  subscription acknowledgement. `collect_ordered` returns channel envelopes and
  checks contiguous sequences, including across successive collections. It
  handles heartbeat/ping traffic and fails on unexpected control frames.
- `crash_and_restart` force-kills and reaps the child, preserves the isolated
  directories, and launches a new epoch. Rediscover stream endpoints after restart.
- `shutdown` checks the clean-shutdown handshake and successful process exit.
  Children also have kill-on-drop enabled for assertion failures.

Startup, RPC, stream subscription/collection, and shutdown have bounded waits.
Each test owns separate directories and OS-assigned ports, allowing parallel runs.
The recovery example writes layout state over IPC, kills the kernel without a
shutdown flush, reopens the workspace, and verifies its key, layout, and hash.
This proves acknowledged layout persistence, not unsaved-buffer WAL recovery;
the existing state and supervisor suites cover those lower-level scenarios.

## CI

The existing Linux, macOS, and Windows matrix compiles the workspace tests before
running `cargo test --workspace --locked` under a two-minute step timeout. This
budget covers test execution, not dependency compilation. The framework suite is
automatically included in the workspace run.