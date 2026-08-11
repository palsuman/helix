# Requirements Document

## Introduction

Helix is a desktop-first, cross-platform IDE built on Tauri 2, React, TypeScript, Monaco Editor, and an authoritative Rust kernel. It targets polyglot enterprise developers and treats AI as an autonomous peer with configurable trust levels.

**Scope:** IDE core platform + bundled first-party plugins + plugin marketplace/distribution ecosystem + remote-capable architecture (future).

**Technology Stack:** Tauri >= 2.0, React >= 18, TypeScript >= 7.0 (native Go compiler), Monaco Editor (latest stable), Rust 2021+ edition.

**Platforms:** Windows 10+, macOS 12+, Linux (Ubuntu 22.04+, Fedora 38+).

### How to read this document

- Requirements are numbered sequentially (`Requirement 1` … `Requirement 86`) for stable navigation.
- Each requirement carries a **stable ID** (`REQ-ARCH-001`, `REQ-ED-004`, …). IDs never change or get reassigned, even when requirements are inserted or reordered. All cross-references in this spec, in `design.md`, and in `tasks.md` use the stable ID.
- Acceptance criteria are numbered within each requirement so a task can cite a specific criterion (e.g. `REQ-ARCH-003.7`).
- Each requirement declares a **Tier** (see MVP Strategy below). The requirement's own `Tier` field is authoritative; the tier index is derived data.

### Document set

| Document | Purpose |
|----------|---------|
| `requirements.md` | What the system must do, and how we verify it |
| `design.md` | How it is built: architecture, interfaces, data models, correctness properties |
| `tasks.md` | Ordered implementation plan, each task traced back to requirements |

---

## Glossary

| Term | Definition |
|------|-----------|
| Kernel | The Rust backend process that owns all application state and business logic |
| Helix Host | The thin Tauri Core process that owns windows, terminates WebView IPC, forwards typed commands to the kernel, and supervises the kernel process |
| Frontend | The React application rendered in Tauri's webview |
| Supervisor | The process-supervision role implemented by the Helix Host; it launches, monitors, and restarts the kernel |
| IPC | Inter-process communication via Tauri's invoke/command system |
| Service | A kernel component with defined lifecycle, registered in the DI container |
| WAL | Write-ahead log; append-only record of unsaved work, replayed on recovery |
| Snapshot | Periodic full capture of session state (open editors, layout, terminals) |
| Provider | An LLM API endpoint (OpenAI, Anthropic, Ollama, etc.) |
| Agent | The autonomous AI coding assistant that plans and executes tasks |
| Worktree | A Git worktree used to isolate agent changes from the user's working tree |
| Plugin (WASM) | A lightweight extension running in a WebAssembly sandbox |
| Plugin (Process) | A heavyweight extension running in a separate OS process |
| MCP | Model Context Protocol — standard for AI tool/resource discovery |
| Trust Level | User-configured autonomy setting for the AI agent |
| Workspace Trust | User decision on whether a folder's code may execute automatically |
| Checkpoint | A saved snapshot of agent state at a significant action |
| Product icon | A UI chrome icon (toolbar, gutter, status) as opposed to a file-type icon |
| Reference hardware | The benchmark configuration all performance targets are measured against (Appendix B) |

---

## MVP Strategy and Release Tiers

### Tier 1 — MVP (First Public Release)
Core editing experience with basic AI. A usable daily driver for a single developer.

**Includes:** REQ-ARCH-001 … 006, REQ-WB-001 … 003, REQ-ED-001 … 003, REQ-ED-005 … 008, REQ-FS-001 … 005, REQ-LANG-001 … 004, REQ-TERM-001, REQ-TASK-001, REQ-GIT-001, REQ-GIT-004, REQ-SEARCH-001, REQ-CONFIG-001, REQ-CONFIG-002, REQ-AI-001, REQ-AI-002, REQ-AI-010, REQ-AI-020, REQ-AI-030, REQ-AI-071, REQ-PLUG-003, REQ-SEC-002, REQ-NFR-001, REQ-NFR-002, REQ-NFR-003, REQ-NFR-005, REQ-THEME-001, REQ-THEME-002, REQ-ICON-001, REQ-ICON-002, REQ-OBS-001, REQ-OBS-004

### Tier 2 — Developer Productivity
Full development workflow: debugging, testing, advanced git, distribution.

**Includes:** REQ-WB-004, REQ-WB-005, REQ-ED-004, REQ-TEST-001, REQ-DEBUG-001, REQ-GIT-002, REQ-GIT-003, REQ-CLI-001, REQ-AI-003, REQ-AI-050, REQ-AI-060, REQ-AI-072, REQ-DIST-001, REQ-DIST-002, REQ-OBS-002, REQ-OBS-003, REQ-PREVIEW-001

### Tier 3 — AI Agent Platform
Autonomous agent with isolation, trust, and recovery. The differentiating feature.

**Includes:** REQ-GIT-005, REQ-AI-040 … 044, REQ-AI-070, REQ-AI-073 … 076, REQ-SEC-003

### Tier 4 — Ecosystem
Plugin marketplace, enterprise features, supply chain hardening.

**Includes:** REQ-PLUG-001, REQ-PLUG-002, REQ-PLUG-004, REQ-SEC-001, REQ-SEC-004, REQ-NFR-004

### Future — Post-v1.0
**Includes:** REQ-REMOTE-001

**Cut Criteria:** If a Tier 3/4 feature threatens Tier 1/2 delivery, it is deferred.

### Deliberate tier inversions

Two requirements intentionally cross tier boundaries. Both are recorded here so they are not mistaken for planning errors.

1. **REQ-PLUG-003 (bundled plugins) is Tier 1, but the plugin runtime (REQ-PLUG-001) is Tier 4.** Bundled language support compiles into the core binary for MVP and migrates onto the public plugin API in Tier 4. The migration is an explicit task, not an aspiration.
2. **REQ-GIT-001 (Tier 1) references the merge editor (REQ-ED-004, Tier 2).** For Tier 1, conflicted files open in the normal text editor with conflict markers plus next/previous-conflict navigation and accept-ours/accept-theirs commands. The three-way merge UI arrives in Tier 2.

---

## Requirements Index

| # | ID | Title | Category | Tier |
|---|----|-------|----------|------|
| 1 | REQ-ARCH-001 | Authoritative Rust Kernel | Architecture | 1 |
| 2 | REQ-ARCH-002 | Service Container | Architecture | 1 |
| 3 | REQ-ARCH-003 | Communication Layer (IPC + WebSocket) | Architecture | 1 |
| 4 | REQ-ARCH-004 | Frontend Architecture | Architecture | 1 |
| 5 | REQ-ARCH-005 | Process Supervision and Recovery | Architecture | 1 |
| 6 | REQ-ARCH-006 | Window Management | Architecture | 1 |
| 7 | REQ-WB-001 | Workbench Layout | Workbench | 1 |
| 8 | REQ-WB-002 | Command Palette and Quick Open | Workbench | 1 |
| 9 | REQ-WB-003 | Notifications | Workbench | 1 |
| 10 | REQ-WB-004 | Welcome and Onboarding | Workbench | 2 |
| 11 | REQ-WB-005 | Localization and Internationalization | Workbench | 2 |
| 12 | REQ-ED-001 | Core Editor | Editor | 1 |
| 13 | REQ-ED-006 | File Lifecycle and Buffer Management | Editor | 1 |
| 14 | REQ-ED-007 | Snippets | Editor | 1 |
| 15 | REQ-ED-008 | Code Structure Navigation | Editor | 1 |
| 16 | REQ-ED-002 | Workspace Find and Replace | Editor | 1 |
| 17 | REQ-ED-003 | Diff Editor | Editor | 1 |
| 18 | REQ-ED-004 | Merge Editor | Editor | 2 |
| 19 | REQ-ED-005 | Formatting | Editor | 1 |
| 20 | REQ-FS-001 | Multi-Root Workspaces | File System | 1 |
| 21 | REQ-FS-002 | Monorepo Awareness | File System | 1 |
| 22 | REQ-FS-003 | File Explorer | File System | 1 |
| 23 | REQ-FS-004 | File Watching | File System | 1 |
| 24 | REQ-FS-005 | Workspace Trust | File System | 1 |
| 25 | REQ-LANG-001 | LSP Host Manager | Language | 1 |
| 26 | REQ-LANG-002 | LSP 3.17+ Feature Support | Language | 1 |
| 27 | REQ-LANG-003 | Tree-sitter Integration | Language | 1 |
| 28 | REQ-LANG-004 | Diagnostics | Language | 1 |
| 29 | REQ-TERM-001 | Integrated Terminal | Terminal | 1 |
| 30 | REQ-TASK-001 | Task System | Tasks | 1 |
| 31 | REQ-TEST-001 | Test Explorer | Testing | 2 |
| 32 | REQ-DEBUG-001 | DAP Client and Debug UI | Debugging | 2 |
| 33 | REQ-GIT-001 | Core Git Operations | Version Control | 1 |
| 34 | REQ-GIT-002 | Remote Operations | Version Control | 2 |
| 35 | REQ-GIT-003 | Advanced Git Workflows | Version Control | 2 |
| 36 | REQ-GIT-004 | Source Control UI | Version Control | 1 |
| 37 | REQ-GIT-005 | VCS Abstraction Layer | Version Control | 3 |
| 38 | REQ-SEARCH-001 | Workspace Search and Indexing | Search | 1 |
| 39 | REQ-CONFIG-001 | Settings System | Configuration | 1 |
| 40 | REQ-CONFIG-002 | Keybinding System | Configuration | 1 |
| 41 | REQ-CLI-001 | Command-Line Interface | Platform | 2 |
| 42 | REQ-AI-001 | LLM Provider Architecture | AI | 1 |
| 43 | REQ-AI-002 | Model Routing and Budget | AI | 1 |
| 44 | REQ-AI-003 | Local Model Management | AI | 2 |
| 45 | REQ-AI-010 | Inline AI Completion | AI | 1 |
| 46 | REQ-AI-020 | Inline AI Edit | AI | 1 |
| 47 | REQ-AI-030 | AI Chat | AI | 1 |
| 48 | REQ-AI-040 | Autonomous Agent | AI Agent | 3 |
| 49 | REQ-AI-041 | Agent Trust and Approval Model | AI Agent | 3 |
| 50 | REQ-AI-042 | Agent Workspace Isolation | AI Agent | 3 |
| 51 | REQ-AI-043 | Agent State and Recovery | AI Agent | 3 |
| 52 | REQ-AI-044 | Agent Review and Merge | AI Agent | 3 |
| 53 | REQ-AI-050 | AI-Enhanced Workflows | AI | 2 |
| 54 | REQ-AI-060 | MCP Support | AI | 2 |
| 55 | REQ-AI-070 | Project Genesis / Greenfield Agent | AI Agent | 3 |
| 56 | REQ-AI-071 | Native Agent Tool Protocol | AI | 1 |
| 57 | REQ-AI-072 | Context Engine | AI | 2 |
| 58 | REQ-AI-073 | Verification Agent | AI Agent | 3 |
| 59 | REQ-AI-074 | Development Environment Manager | AI Agent | 3 |
| 60 | REQ-AI-075 | Skills and Project Recipes | AI Agent | 3 |
| 61 | REQ-AI-076 | Specialist Agents and Delegation | AI Agent | 3 |
| 62 | REQ-PLUG-001 | Plugin Architecture | Plugins | 4 |
| 63 | REQ-PLUG-002 | Plugin Marketplace | Plugins | 4 |
| 64 | REQ-PLUG-003 | Bundled First-Party Plugins | Plugins | 1 |
| 65 | REQ-PLUG-004 | Plugin Development Kit | Plugins | 4 |
| 66 | REQ-SEC-001 | Plugin Sandbox | Security | 4 |
| 67 | REQ-SEC-002 | Secret Management | Security | 1 |
| 68 | REQ-SEC-003 | Agent Security | Security | 3 |
| 69 | REQ-SEC-004 | Supply Chain Security | Security | 4 |
| 70 | REQ-DIST-001 | Cross-Platform Distribution | Distribution | 2 |
| 71 | REQ-DIST-002 | Update System | Distribution | 2 |
| 72 | REQ-NFR-001 | Performance | Non-Functional | 1 |
| 73 | REQ-NFR-002 | Reliability | Non-Functional | 1 |
| 74 | REQ-NFR-003 | Offline Capability | Non-Functional | 1 |
| 75 | REQ-NFR-004 | API Stability | Non-Functional | 4 |
| 76 | REQ-NFR-005 | Accessibility | Non-Functional | 1 |
| 77 | REQ-THEME-001 | Theme Architecture | Theming | 1 |
| 78 | REQ-THEME-002 | Syntax Theme Colors | Theming | 1 |
| 79 | REQ-ICON-001 | Product Icon System | Theming | 1 |
| 80 | REQ-ICON-002 | File Icon Themes | Theming | 1 |
| 81 | REQ-OBS-001 | Structured Logging | Observability | 1 |
| 82 | REQ-OBS-002 | Crash Reporting | Observability | 2 |
| 83 | REQ-OBS-003 | Performance Telemetry | Observability | 2 |
| 84 | REQ-OBS-004 | Health Monitoring | Observability | 1 |
| 85 | REQ-PREVIEW-001 | Embedded Web Preview | Preview | 2 |
| 86 | REQ-REMOTE-001 | Remote Development | Remote | Future |

---

## Requirements

### Requirement 1: Authoritative Rust Kernel

**ID:** REQ-ARCH-001 | **Category:** Architecture | **Tier:** 1 (MVP)

The Rust kernel SHALL be the single source of truth for all application state.

**User Story:** As a platform developer, I want all state centralized in the Rust kernel so the frontend is a pure view layer with no business logic.

#### Acceptance Criteria
1. All application state is owned and managed by Rust services.
2. The frontend cannot mutate state except through a kernel command.
3. State is persisted and recoverable on graceful shutdown.
4. Kernel services are independently testable with mock dependencies.
5. No business logic or persistent state resides in the frontend.
6. The Helix Host owns only Tauri window/capability lifecycle, command forwarding, kernel supervision, and recovery UI state; it contains no IDE domain business logic.

#### Failure Modes
- Kernel panic: supervised restart (see REQ-ARCH-005), last persisted state recovered, unsaved buffers restored from WAL.
- State corruption: detected via CRC checksum on state files; falls back to last valid snapshot with user notification.
- Graceful shutdown timeout (> 5s): force-kill with best-effort state flush.

---

### Requirement 2: Service Container

**ID:** REQ-ARCH-002 | **Category:** Architecture | **Tier:** 1 (MVP)

The kernel SHALL expose services through a dependency-injection service container with lifecycle management.

**User Story:** As a platform developer, I want services wired through a container so dependencies are explicit, mockable, and independently restartable.

#### Acceptance Criteria
1. Services register with the container at startup.
2. Container supports singleton, transient, and scoped lifetimes.
3. Services declare dependencies, resolved at construction.
4. Shutdown proceeds in reverse registration order.
5. Circular dependencies are detected at registration time (compile-time where possible, runtime fallback).
6. Every service exposes a health check (liveness probe).
7. A panicked service can be restarted in isolation without restarting the kernel.

#### Failure Modes
- Dependency resolution failure: kernel logs error, affected services marked degraded, unaffected services continue.
- Service panic: isolated restart of the panicked service; dependent services notified via health channel.

---

### Requirement 3: Communication Layer (IPC + WebSocket)

**ID:** REQ-ARCH-003 | **Category:** Architecture | **Tier:** 1 (MVP)

**User Story:** As a developer, I want the UI to stay responsive while the backend streams high-volume output, so a noisy build or busy terminal never stalls my editor.

All frontend-to-kernel request-response communication SHALL enter through Tauri IPC at the Helix Host and be forwarded over typed internal RPC to the separate kernel process. High-frequency kernel streams SHALL use an authenticated local WebSocket whose endpoint is brokered by the host.

**Rationale:** Tauri Core owns application windows and terminates WebView IPC; a WebView cannot invoke commands directly in an unrelated `helix-kernel` executable. The host therefore validates the Tauri command envelope and forwards it without implementing domain behavior. Tauri's event system lacks the backpressure, delivery ordering guarantees, and typed envelope routing needed for high-throughput streams, so the kernel WebSocket remains the streaming path.

#### Acceptance Criteria
1. IPC uses one typed request/response contract generated from Rust structs across the WebView-to-host and host-to-kernel boundaries.
2. IPC supports command cancellation via correlation IDs.
3. IPC timeout is configurable (default 30s) with user-visible error handling.
4. IPC round-trip for simple commands is < 5ms (p95) on reference hardware.
5. WebSocket uses a typed envelope: `{ channel, correlationId?, sequence, payload }`.
6. WebSocket carries terminal output, agent progress, log tailing, diagnostics push, search results, and debug output.
7. WebSocket reconnects automatically with exponential backoff (100ms, 200ms, 400ms … max 10s).
8. WebSocket applies backpressure per channel (ring buffer, configurable depth, oldest-dropped with metric).
9. WebSocket heartbeat pings every 5s; the connection is considered dead after 3 missed pongs (15s).
10. Messages within a channel are delivered in monotonically increasing sequence order.
11. Tauri command handlers in the host contain forwarding, validation, cancellation, timeout, and error translation only; domain handlers execute in the kernel.
12. Internal RPC authenticates the host/kernel peer, preserves correlation IDs and typed errors, and rejects stale connections after a kernel restart.
13. Contract and integration tests exercise the complete WebView/Tauri-host/internal-RPC/kernel round trip rather than only an in-process dispatcher.

#### Failure Modes
- IPC timeout: command cancelled kernel-side, frontend shows error toast with retry.
- WebSocket disconnect: frontend shows "reconnecting" indicator, buffers user actions, replays on reconnect.
- Backpressure overflow: oldest messages dropped, frontend notified via control message, UI shows "output truncated".
- Malformed message: logged, discarded, counter incremented for observability.

---

### Requirement 4: Frontend Architecture

**ID:** REQ-ARCH-004 | **Category:** Architecture | **Tier:** 1 (MVP)

**User Story:** As a developer, I want one misbehaving panel to fail on its own, so a bug in a tool window never takes down the editor I am working in.

The frontend SHALL be a React application with unidirectional data flow, treating the kernel as authoritative.

#### Acceptance Criteria
1. React app renders in the Tauri webview.
2. A lightweight state manager (Zustand) holds UI-only state (panel sizes, focus, selection, scroll).
3. All domain mutations flow through kernel commands; the frontend holds read-only projections.
4. Panels and views are lazy-loaded (code-split at route/panel level).
5. Frontend shell renders in < 500ms; full hydration < 1.5s.
6. Each panel has an error boundary: a panel crash shows a fallback and does not take down the IDE.
7. Local projections are reconciled against kernel state every 30s by comparing a projection hash; mismatch triggers re-fetch.
8. A webview crash is detected by the kernel, the webview is restarted, and kernel state is re-pushed.

#### Failure Modes
- React render crash: error boundary shows panel-level fallback with "reload panel" action.
- Projection desync: periodic reconciliation corrects it without user action.
- WebView crash (renderer OOM): kernel detects loss of heartbeat, restarts webview, re-pushes state.

---

### Requirement 5: Process Supervision and Recovery

**ID:** REQ-ARCH-005 | **Category:** Architecture | **Tier:** 1 (MVP)

The application SHALL supervise the kernel process so that a kernel crash is detected and recovered without user intervention.

**User Story:** As a developer, I want the IDE to survive a backend crash without losing my work or requiring me to relaunch it, so an internal fault never costs me my session.

**Rationale:** REQ-ARCH-001 and REQ-NFR-002 promise automatic kernel restart and a bounded Recovery Point Objective. A crashed process cannot restart itself, so an explicit supervisor is required. This requirement names the owner of that behaviour.

#### Acceptance Criteria
1. The Helix Host, which is the Tauri Core process and supervisor, launches the separate kernel and monitors it for abnormal exit (non-zero code, signal, or missed heartbeat).
2. The host is minimal: window lifecycle, Tauri capabilities, typed command forwarding, kernel supervision, and recovery UI only; it owns no IDE business state, loads no plugins, and makes no application network requests.
3. On abnormal kernel exit, the kernel is restarted within 2s.
4. After restart the kernel loads the last snapshot and replays WAL entries; the frontend re-attaches and re-subscribes to its channels.
5. Restart storms are damped: max 5 restarts in 5 minutes, then the supervisor stops and presents a recovery UI offering "retry", "start without session restore", and "open logs".
6. The supervisor distinguishes a user-initiated quit from a crash and does not restart on clean exit.
7. Kernel heartbeat to the frontend every 5s; the frontend shows a recovery indicator if 3 consecutive beats are missed.
8. Crash cause is captured before restart (exit code, signal, panic message, last 20 log lines) and handed to crash reporting (REQ-OBS-002) when enabled.
9. Recovery is verified by an automated test that kills the kernel mid-edit and asserts unsaved content returns.

#### Failure Modes
- Helix Host itself dies: the OS reports application exit; on next launch the stale lock file triggers snapshot + WAL recovery, bounded by the REQ-NFR-002 RPO, with a manual relaunch.
- Kernel crashes during startup (crash loop before ready): after 3 failed starts, launch in safe mode with plugins and session restore disabled.
- Restart succeeds but state is corrupt: fall back to last valid snapshot, notify user which session data was dropped.

---

### Requirement 6: Window Management

**ID:** REQ-ARCH-006 | **Category:** Architecture | **Tier:** 1 (MVP)

The system SHALL support multiple application windows over a single kernel.

**User Story:** As a developer working across several projects, I want to open folders in separate windows that share one backend, so I can work side by side without paying the memory cost of a second IDE.

#### Acceptance Criteria
1. Multiple windows are supported, each bound to one workspace (which may itself be multi-root per REQ-FS-001).
2. One kernel process serves all windows; per-window state is scoped by window ID.
3. Commands: New Window, Open Folder in New Window, Duplicate Workspace in New Window, Close Window.
4. Moving an editor tab between windows is supported via drag-out (tab detach) or command.
5. Window geometry, monitor placement, and maximized/fullscreen state persist per workspace and restore on reopen.
6. Closing the last window shuts down the kernel gracefully; closing a non-last window releases only that window's resources (watchers, terminals, LSP servers not shared).
7. Services with per-workspace scope (LSP, watchers, terminals, search index) are keyed by workspace and reference-counted so a shared root is not torn down while another window uses it.
8. Global singletons (settings, keybindings, secrets, theme, AI providers) resolve once and apply to all windows; a settings change propagates to every open window.
9. Window-scoped notifications appear in the originating window; global notifications appear in the focused window.
10. Reopening the app restores the previous window set when session restore is enabled.

#### Failure Modes
- A single window's webview crashes: only that window restarts (see REQ-ARCH-004.8); other windows are unaffected.
- Kernel restart with multiple windows open: all windows re-attach and re-subscribe; each restores its own scoped state.
- Workspace already open in another window: focus the existing window instead of opening a duplicate, unless the user explicitly requests a duplicate.

---

### Requirement 7: Workbench Layout

**ID:** REQ-WB-001 | **Category:** Workbench | **Tier:** 1 (MVP)

The workbench SHALL provide a VS Code-style layout with customizable panels.

**User Story:** As a developer, I want a familiar IDE layout with customizable tool windows so I can organize my workspace efficiently.

#### Acceptance Criteria
1. Activity bar supports icon-based view switching (icons per REQ-ICON-001).
2. Primary sidebar, positionable left or right.
3. Secondary sidebar, optional, on the opposite side.
4. Panel area, positionable bottom or right.
5. Editor area with split views (horizontal/vertical, up to 4 groups).
6. Layout state persists across sessions (stored in the kernel, synced on startup).
7. All panels are resizable with drag handles honouring min/max constraints (sidebar min 200px).
8. User-defined layout profiles can be saved, restored, switched, renamed, and deleted via the command palette.
9. Zen/distraction-free mode hides all chrome (Ctrl+K Z).
10. Status bar shows contextual segments (left: branch, errors; right: language, encoding, line ending, line/col).
11. Layout degrades gracefully at the minimum supported window size (1024x600).

#### Failure Modes
- Corrupted layout state: reset to default layout with notification.
- Panel render failure: error boundary replaces the panel with a "failed to load — reload" placeholder.
- Layout profile references a view that no longer exists (plugin uninstalled): profile loads with that slot empty and a one-time notice.

---

### Requirement 8: Command Palette and Quick Open

**ID:** REQ-WB-002 | **Category:** Workbench | **Tier:** 1 (MVP)

**User Story:** As a developer, I want to reach any command or file with a few keystrokes, so I can navigate a large codebase without hunting through menus or trees.

The system SHALL provide a command palette and quick file open.

#### Acceptance Criteria
1. Command palette (Ctrl/Cmd+Shift+P): fuzzy search over all registered commands, recent commands first, keyboard shortcut display, category grouping, plugin-contributed commands.
2. Quick open (Ctrl/Cmd+P): fuzzy filename matching across all workspace roots with recent-file prioritization.
3. Mode prefixes in quick open: `@` document symbols, `#` workspace symbols, `:` line navigation, `>` command mode.
4. Results appear within 50ms of keystroke for workspaces under 100k files.
5. Search is cancellable: a new keystroke cancels the previous query.
6. Commands unavailable in the current context are hidden or shown disabled with the reason.
7. Enter opens the selection; Ctrl+Enter opens it in a split group.

#### Failure Modes
- Index not yet built: fall back to on-demand directory scan, show a subtle "indexing" hint.
- Symbol provider unavailable for the active language: `@`/`#` modes report "no symbol provider" rather than returning empty silently.

---

### Requirement 9: Notifications

**ID:** REQ-WB-003 | **Category:** Workbench | **Tier:** 1 (MVP)

**User Story:** As a developer, I want to be told when something needs my attention and to be left alone otherwise, so notifications inform me without interrupting my focus.

The system SHALL provide a notification system.

#### Acceptance Criteria
1. Toast notifications in four kinds: info, warning, error, progress.
2. Auto-dismiss: info 5s, warning 10s; errors require manual dismissal.
3. Up to 3 action buttons per notification (e.g. "Retry", "Open File", "Dismiss").
4. Notification center retains history for the session (max 500 entries).
5. Do-not-disturb mode suppresses toasts and accumulates them in the center.
6. Progress notifications show determinate or indeterminate progress and a cancel action where the operation supports it.
7. Every notification attributes its source (service or plugin).
8. Notifications are announced to screen readers via an ARIA live region (per REQ-NFR-005).

---

### Requirement 10: Welcome and Onboarding

**ID:** REQ-WB-004 | **Category:** Workbench | **Tier:** 2

The system SHALL provide a welcome experience for first run and empty state.

**User Story:** As a developer opening Helix for the first time, I want an obvious path to my code and to configuring AI, so I am productive without reading documentation.

#### Acceptance Criteria
1. Welcome tab on first launch and when no workspace is open, with: Open Folder, Clone Repository, Recent Workspaces, and New File.
2. Setup checklist covering the steps that gate core value: choose theme, configure an AI provider (REQ-AI-001), install language support, import keybindings from another editor (REQ-CONFIG-002.8).
3. Checklist items reflect real state (a configured provider shows as complete) and persist across restarts.
4. Recent workspaces list with pinning and removal.
5. "What's New" content shown once after an update, sourced from the release notes bundled with the build (no network required).
6. Welcome tab is dismissible and its reappearance is configurable (`workbench.startupEditor`).
7. Keyboard-navigable and screen-reader-labelled per REQ-NFR-005.
8. No telemetry or network call is required to render the welcome experience (per REQ-NFR-003).

---

### Requirement 11: Localization and Internationalization

**ID:** REQ-WB-005 | **Category:** Workbench | **Tier:** 2

The system SHALL support localization of its user interface and correct handling of international text.

**User Story:** As a developer on a non-English team, I want the IDE in my language with correct text handling, so language is not a barrier to adoption.

**Rationale:** Retrofitting i18n after UI strings are scattered across a codebase is expensive. The extraction mechanism and string discipline are therefore Tier 1 engineering practice even though shipped translations are Tier 2.

#### Acceptance Criteria
1. No user-visible string is hardcoded in components; all strings resolve through a message catalog keyed by message ID (enforced by lint rule from the first UI task onward).
2. Message catalogs are JSON, one per locale, loaded at startup based on OS locale with user override (`helix.locale`).
3. Missing translation falls back to the base locale (en) per key, never to a blank or raw key.
4. Pluralization and interpolation support (ICU MessageFormat or equivalent); no string concatenation for grammatical construction.
5. Locale-aware formatting for dates, times, numbers, and relative times ("3 minutes ago") in all UI surfaces.
6. Shipped locales for v1.0: English plus at least 4 additional locales.
7. Plugins can contribute their own catalogs and are resolved with the same fallback rules.
8. RTL layout support: mirrored layout, mirrored directional icons (REQ-ICON-001.14), correct bidirectional text rendering in UI chrome.
9. Editor content handling is locale-independent and correct regardless of UI locale: full Unicode support, grapheme-cluster-correct cursor movement and deletion, combining marks, CJK wide characters, emoji and ZWJ sequences.
10. Ambiguous and invisible character detection: warn on bidirectional control characters and confusable/invisible Unicode in source files (a security consideration, not only a display one).
11. Locale switch takes effect without reinstall; a restart may be required and is prompted clearly.

#### Failure Modes
- Catalog file malformed: load base locale, report the parse error, do not block startup.
- Locale requested but not shipped: fall back to base locale with a one-time notice.
- Font missing glyphs for the active locale: fall back through the configured font stack; never render tofu without a diagnosable log entry.

---

### Requirement 12: Core Editor

**ID:** REQ-ED-001 | **Category:** Editor | **Tier:** 1 (MVP)

The editor SHALL be based on Monaco Editor with full text editing capabilities.

**User Story:** As a developer, I want a fast, feature-rich code editor with syntax highlighting, intelligent completions, and multi-cursor support.

#### Acceptance Criteria
1. Multi-cursor and multi-selection editing.
2. Find and replace within a file, with regex support.
3. Code folding (language-aware plus region-based).
4. Minimap, configurable: on/off, scale, characters vs blocks.
5. Bracket matching and colorization with configurable nesting depth and colors.
6. Indentation guides.
7. Line numbers: absolute, relative, or off.
8. Word wrap modes: off, on, wordWrapColumn, bounded.
9. Whitespace rendering: none, boundary, selection, all.
10. Linked editing (e.g. HTML tag pairs).
11. Editor tabs support drag-and-drop reordering, pinning, preview mode (italic title), modified indicator, close/close-others/close-all/close-to-the-right, and split-to-group.
12. Tab overflow menu lists all open editors when tabs do not fit.
13. Files over 5MB open in large-file mode (no tokenization, minimap, or folding) with user notification.
14. Binary files are detected and shown as a hex preview or "binary file" notice; binary content is never corrupted.
15. Editor state (cursor, selection, scroll, fold state) persists per file across sessions.

#### Failure Modes
- Monaco crash or freeze: detected via heartbeat; instance destroyed and recreated, content restored from the kernel buffer.
- File too large for memory: refuse to open with a clear error, offer streaming read-only mode.

---

### Requirement 13: File Lifecycle and Buffer Management

**ID:** REQ-ED-006 | **Category:** Editor | **Tier:** 1 (MVP)

The system SHALL manage the full lifecycle of editable buffers, including buffers with no file on disk.

**User Story:** As a developer, I want to jot code in a scratch buffer, save it where I choose, and control saving, encoding, and line endings, so the editor does not force a workflow on me.

**Rationale:** Previously implied by the status bar (which displayed encoding and line endings) and by the WAL (which persisted "unsaved buffers") without any requirement defining what an unsaved buffer is, how it is named, or how it is saved.

#### Acceptance Criteria
1. New untitled buffers can be created (Ctrl+N), edited, and persist unsaved across restarts via the WAL (REQ-NFR-002.1).
2. Untitled buffers carry a language mode, settable manually or inferred on first save.
3. Save As: choose path and filename via the native file dialog; the buffer becomes a normal file editor afterwards.
4. Save All, with per-file error reporting that does not abort the remaining saves.
5. Auto-save with modes: off, afterDelay (configurable, default 1000ms), onFocusChange, onWindowChange.
6. Auto-save never runs while a file has unresolved merge conflict markers, and never triggers format-on-save unless explicitly enabled.
7. Encoding: detected on open (BOM plus heuristics), displayed in the status bar, and changeable via "Reopen with Encoding" and "Save with Encoding".
8. Line endings: detected on open (LF, CRLF, mixed), displayed in the status bar, changeable per file, with a default for new files configurable per platform and honouring `.editorconfig`.
9. Mixed line endings are reported, and normalization is offered as an explicit action rather than applied silently.
10. Trailing-whitespace trim and final-newline insertion on save, configurable, honouring `.editorconfig`.
11. Files and folders dropped onto the window from the OS open as editors or workspace roots; the choice is presented when ambiguous.
12. Read-only files are detected and the editor blocks edits with a clear indicator and an "override" action where the OS permits.
13. External deletion of an open file marks the editor dirty-with-no-file and offers Save As or Close.
14. Closing a dirty buffer prompts Save / Don't Save / Cancel; Cancel always aborts the close.

#### Failure Modes
- Save fails (permissions, disk full, read-only volume): original file untouched, specific OS error surfaced, buffer stays dirty.
- Encoding conversion is lossy: warn with the count of unrepresentable characters before writing, and offer to keep the original encoding.
- Save As to a path already open in another editor: focus the conflict and refuse rather than creating two buffers over one file.

---

### Requirement 14: Snippets

**ID:** REQ-ED-007 | **Category:** Editor | **Tier:** 1 (MVP)

The system SHALL support snippet definition and expansion.

**User Story:** As a developer, I want reusable code templates with tab stops, so repetitive boilerplate costs me a few keystrokes.

#### Acceptance Criteria
1. Snippet syntax with tab stops (`$1`, `$2`, `$0`), placeholders with defaults, choices, mirrored/duplicated stops, and nested snippets.
2. Variable substitution: selection, clipboard, filename, file path, workspace name, date/time, random, and comment markers for the current language.
3. Snippet sources: user snippets (global and per-language files), workspace snippets (`.helix/snippets/`), LSP-provided completion snippets, and plugin-contributed snippets.
4. Snippets appear in the completion list alongside LSP items, marked with a snippet icon, and are also insertable by name via the command palette.
5. Tab and Shift+Tab navigate stops; Escape leaves snippet mode; typing over a placeholder replaces it and updates mirrors.
6. Indentation is normalized to the target file's indent settings on insertion.
7. Snippet editor command opens the relevant snippet file with schema completion and validation.
8. Snippet expansion is a single undo step.

#### Failure Modes
- Malformed snippet body: reject with a clear parse error identifying the snippet and position; other snippets still load.
- Conflicting prefixes across sources: workspace overrides user, user overrides plugin, plugin overrides built-in; the effective source is shown in the completion detail.

---

### Requirement 15: Code Structure Navigation

**ID:** REQ-ED-008 | **Category:** Editor | **Tier:** 1 (MVP)

The system SHALL provide structural navigation affordances for the active file.

**User Story:** As a developer reading an unfamiliar file, I want to see and jump through its structure, so I can orient myself without scrolling.

**Rationale:** Breadcrumbs and the outline view were previously side effects of an LSP task with no requirement defining their behaviour, fallback, or configurability.

#### Acceptance Criteria
1. Breadcrumb bar above the editor showing workspace-relative path segments plus the symbol path at the cursor.
2. Each breadcrumb segment is clickable, opening a filterable picker of siblings (files for path segments, symbols for symbol segments).
3. Breadcrumbs are keyboard-navigable (focus command, arrow keys, Enter) and configurable: off, path only, symbols only, both.
4. Outline view in the sidebar showing the document symbol tree, with filter, sort by position/name/kind, and follow-cursor.
5. Symbol source precedence: LSP `documentSymbol` when available, Tree-sitter-derived structure otherwise (REQ-LANG-003), nothing shown for unsupported languages rather than a misleading empty tree.
6. Sticky scroll: pin enclosing scope headers at the top of the viewport, configurable with a maximum line count.
7. Symbol icons come from the icon system's `SymbolKind` set (REQ-ICON-001.9).
8. Breadcrumb and outline updates are debounced and never block typing (per REQ-NFR-001.3).

---

### Requirement 16: Workspace Find and Replace

**ID:** REQ-ED-002 | **Category:** Editor | **Tier:** 1 (MVP)

**User Story:** As a developer refactoring across a large repository, I want to search and replace project-wide with a preview and a reliable undo, so bulk edits are fast without being risky.

The system SHALL provide project-wide Find in Files and Replace in Files.

#### Acceptance Criteria
1. Text, regex, case-sensitive, and whole-word matching.
2. Include/exclude glob patterns, plus a toggle to respect or ignore `.gitignore`.
3. Streaming results grouped by file with configurable context lines (0-5).
4. Clicking a result opens the file at that line with the match highlighted.
5. Result groups can be collapsed and expanded; individual results can be dismissed from the set.
6. Replace preview shows a diff of pending changes before execution.
7. Replace in a single file, in a selected subset, or across all results.
8. A workspace-level undo stack covers multi-file replacements, retaining the last 10 operations, each undoable as one step.
9. File versions are validated immediately before replacement to detect concurrent external changes.
10. Replacement is atomic per file (write temp, fsync, rename); no partial file is ever observable.
11. Progress is reported for large replacements and the operation is cancellable.
12. Handles repositories with 100,000+ files.
13. First results are visible within 200ms for projects under 50k files.
14. Search result sets can be pinned so a new search does not discard them.
15. Search history retains the last 50 queries across sessions.

#### Failure Modes
- File locked by another process: skip it, report to user, continue with the rest.
- Disk full during replace: atomic write fails, original intact, operation halts with a clear error.
- External modification detected: abort replace for that file, notify, offer re-search.

---

### Requirement 17: Diff Editor

**ID:** REQ-ED-003 | **Category:** Editor | **Tier:** 1 (MVP)

**User Story:** As a developer reviewing changes, I want one consistent diff view everywhere differences are shown, so comparing files, revisions, and AI proposals all feel the same.

The system SHALL provide a reusable diff editor component.

#### Acceptance Criteria
1. Side-by-side view.
2. Inline (unified) view.
3. Next/previous change navigation.
4. Staged vs working tree comparison.
5. Arbitrary file/revision comparison, including compare-with-clipboard and compare-with-saved.
6. Virtualized rendering for files over 10k lines.
7. Read-only and editable modes.
8. Gutter indicators with added/removed/modified line counts.
9. Whitespace-insensitive comparison toggle.
10. Diff colors resolve from theme tokens (REQ-THEME-002.4).

---

### Requirement 18: Merge Editor

**ID:** REQ-ED-004 | **Category:** Editor | **Tier:** 2

**User Story:** As a developer resolving a merge, I want to see both sides against their common ancestor and build the result deliberately, so I stop guessing at conflict markers.

The system SHALL provide a three-way merge editor.

#### Acceptance Criteria
1. Base, ours (current), and theirs (incoming) panes.
2. Result pane with live preview.
3. Conflict navigation (next/previous).
4. Per-conflict actions: accept current, accept incoming, accept both, accept none.
5. Manual editing of the result pane.
6. Completion validation: all-conflicts-resolved indicator; the merge cannot be completed until none remain.
7. Opens automatically from the Git merge workflow on conflict (REQ-GIT-001, REQ-GIT-003).
8. Conflict count and resolution progress indicator ("3 of 7 resolved").
9. Minimap markers show conflict locations.

**Tier 1 fallback:** until this requirement is delivered, conflicted files open in the standard editor with conflict markers, next/previous-conflict navigation, and accept-ours/accept-theirs/accept-both commands.

#### Failure Modes
- Merge editor closed with conflicts remaining: prompt the user (save partial / discard / continue editing).

---

### Requirement 19: Formatting

**ID:** REQ-ED-005 | **Category:** Editor | **Tier:** 1 (MVP)

**User Story:** As a developer on a team with agreed style, I want formatting to happen automatically from the project's own configuration, so code style is never a review topic.

The system SHALL provide a formatting provider service.

#### Acceptance Criteria
1. Format Document (whole file).
2. Format Selection.
3. Format Modified Lines (changed regions only, via git diff).
4. Format on Save, configurable per language.
5. Format on Paste, configurable.
6. Format on Type, triggered by characters.
7. Multiple formatter providers per language with a user-selectable default.
8. Formatting timeout of 2s, cancelled with notification on expiry.
9. LSP formatters register through this service as the primary source.
10. Plugin-contributed formatters register through the same interface.
11. `.editorconfig` is respected when present (indent_style, indent_size, end_of_line, trim_trailing_whitespace).
12. Formatter results are rejected if empty or more than 10x the original size.

#### Failure Modes
- Formatter process crash: report the error, leave the file unchanged, offer to disable that formatter.
- Invalid output: rejected, file unchanged, error logged with the offending provider named.

---

### Requirement 20: Multi-Root Workspaces

**ID:** REQ-FS-001 | **Category:** File System | **Tier:** 1 (MVP)

**User Story:** As a developer whose service spans several repositories, I want them open together in one window, so I can navigate and search across them as one project.

The system SHALL support multi-root workspaces.

#### Acceptance Criteria
1. Multiple project folders in one window.
2. Workspace configuration file (`.helix/workspace.json`) with JSON schema validation, carrying a stable `id` used as the state and cache key (REQ-NFR-002.11).
3. Per-folder settings override workspace settings (REQ-CONFIG-001.1).
4. Folders can be added and removed at runtime via command palette or explorer context menu.
5. Maximum 20 roots per workspace (configurable), with a warning at the threshold.
6. Recent workspaces are tracked (last 20) and surfaced in the welcome experience (REQ-WB-004.4).

#### Failure Modes
- Workspace config parse error: open with available roots, notify about the invalid config, offer to reset.
- Root folder deleted externally: detected via watcher, warn, offer removal from the workspace.
- Root on an unmounted drive: marked unavailable with periodic retry; other roots are unaffected.

---

### Requirement 21: Monorepo Awareness

**ID:** REQ-FS-002 | **Category:** File System | **Tier:** 1 (MVP)

The system SHALL provide monorepo awareness.

**User Story:** As an enterprise developer working in a monorepo, I want the IDE to understand my project structure so navigation, search, and build commands respect project boundaries.

#### Acceptance Criteria
1. Detection of monorepo tooling: Nx, Turborepo, Lerna, pnpm/npm/yarn workspaces, Cargo workspaces, Go workspaces, Maven multi-module, Gradle multi-project, .NET solution files.
2. Project graph extraction: the set of projects, their root paths, and their inter-project dependencies.
3. Graph is exposed to the rest of the IDE as a queryable service (which project owns this file, what depends on this project).
4. Scoped operations: "Run Task in Project" and "Search in Project" restrict to the selected project's roots.
5. Affected-project detection: given a set of changed files, determine which projects are impacted, using the tool's own affected API where one exists and the extracted graph otherwise.
6. Project switcher in the status bar for quick-switching the active project context.
7. Per-project task and run configurations.
8. Graph is cached on disk and invalidated when the tool's config files or lockfiles change.
9. Graph extraction runs in the background and never blocks workspace open.

#### Failure Modes
- Tool not installed: fall back to treating each root as an independent project.
- Graph extraction timeout (> 10s): use the cached graph if present, otherwise skip with notification.
- Graph extraction fails or returns malformed output: log, keep the last good graph, degrade to per-root behaviour.

---

### Requirement 22: File Explorer

**ID:** REQ-FS-003 | **Category:** File System | **Tier:** 1 (MVP)

**User Story:** As a developer, I want to browse and reorganize my project tree fluidly even when it holds a hundred thousand files, so the explorer never becomes the slow part of my workflow.

The system SHALL provide a file explorer tree view.

#### Acceptance Criteria
1. Virtual rendering for large trees (100,000+ files) at 60fps scroll.
2. File and folder CRUD: new file, new folder, rename, delete with confirmation, duplicate.
3. Drag-and-drop move and copy, distinguished by modifier key.
4. Multi-select via Ctrl+click (toggle) and Shift+click (range), with multi-target delete and move.
5. File decoration: diagnostics count badge, and Git status colors when a VCS provider is active (REQ-GIT-004).
6. File and folder icons resolved via the file icon theme service (REQ-ICON-002).
7. Filter/focus mode: type to filter visible nodes.
8. Collapse all.
9. Reveal in explorer, from an editor tab or the command palette.
10. Context menu covering open, open to side, copy path, copy relative path, reveal in OS file manager, rename, delete.
11. Explorer functions fully without a VCS provider; Git decoration is an optional overlay, not a prerequisite.

#### Failure Modes
- Permission denied: show the specific OS error (access denied, read-only, etc.).
- Rename conflict: prompt for an alternative name.
- Delete of an open file: close the editor tab first, warning if unsaved.

---

### Requirement 23: File Watching

**ID:** REQ-FS-004 | **Category:** File System | **Tier:** 1 (MVP)

**User Story:** As a developer who also uses git and build tools on the command line, I want the IDE to notice changes made outside it, so what I see always matches what is on disk.

The kernel SHALL watch the filesystem for external changes.

#### Acceptance Criteria
1. Detect create/modify/delete originating outside the IDE.
2. Silently reload editor buffers for unmodified files.
3. Prompt for modified files with external changes ("File changed on disk. Reload?").
4. Respect `.gitignore` and configured exclusion patterns for performance.
5. Configurable watch depth and exclusion globs; `node_modules`, `.git`, and build directories excluded by default.
6. Watcher budget of 10,000 watched paths per root, warning and suggesting exclusions when exceeded.
7. Debounce rapid changes in a 50ms window to avoid event storms.
8. Watched-path count and event rate are reported to health monitoring (REQ-OBS-004.1).

#### Failure Modes
- OS watch limit exhausted (e.g. inotify): warn, suggest raising the limit or adding exclusions, fall back to 5s polling for overflow paths.
- Watcher crash: restart and perform a full directory diff to catch missed events.
- Network filesystem detected (latency > 500ms): switch to polling automatically.

---

### Requirement 24: Workspace Trust

**ID:** REQ-FS-005 | **Category:** File System | **Tier:** 1 (MVP)

The system SHALL require explicit user trust before executing workspace-supplied code or configuration.

**User Story:** As a developer who reviews unfamiliar repositories, I want the IDE to not execute anything from a folder until I say it is trusted, so cloning a repo cannot compromise my machine.

**Rationale:** Task auto-detection (REQ-TASK-001.3), language servers, debug adapters, formatters, and MCP servers all execute code or launch processes described by files inside the workspace. Without a trust gate, opening a repository is equivalent to running it.

#### Acceptance Criteria
1. On first open of a folder, the user chooses Trust or Restricted mode; the decision is remembered per path.
2. Parent-folder trust can be granted so subfolders inherit it.
3. In Restricted mode the following are blocked: task execution and auto-detection, language server launch, debug adapter launch, plugin activation for workspace-recommended plugins, MCP server launch, workspace-defined formatters, agent execution, and workspace settings that specify executable paths.
4. In Restricted mode the following remain fully available: reading and editing files, syntax highlighting via Tree-sitter, search, Git read operations, and the AI chat.
5. Restricted mode is visibly indicated in the status bar and in a dismissible banner, with a one-click path to trust the folder.
6. Trust can be revoked; revocation terminates the processes that trust permitted and returns to Restricted mode.
7. A trust manager UI lists trusted folders and allows removal.
8. Trust state is global user data, never stored inside the workspace itself (a workspace cannot declare itself trusted).
9. Multi-root workspaces require trust for every root; a single untrusted root places the window in Restricted mode.
10. Trust prompts are suppressible for users who opt into trusting everything, with an explicit warning at the time they opt in.

#### Failure Modes
- Trust store unreadable or corrupt: default to Restricted for all folders (fail closed), notify the user.
- A feature is invoked that requires trust: it fails with a clear explanation and a link to grant trust, never a generic error.

---

### Requirement 25: LSP Host Manager

**ID:** REQ-LANG-001 | **Category:** Language | **Tier:** 1 (MVP)

**User Story:** As a polyglot developer, I want language servers managed and recovered for me, so a crashed or memory-hungry server is a momentary blip rather than a broken session.

The kernel SHALL manage language server lifecycle.

#### Acceptance Criteria
1. Start, stop, and restart language servers per workspace root.
2. Process isolation: one process per language server instance.
3. Capability negotiation per LSP 3.17+, tracking newer revisions.
4. Auto-restart on crash with exponential backoff (1s, 2s, 4s, 8s, max 30s); after 5 restarts in 3 minutes the server is marked Failed and requires manual restart.
5. Resource monitoring: memory warning at 512MB, kill at 1GB per server; CPU sampled for health reporting.
6. Multiple servers per language (e.g. TypeScript + ESLint + Tailwind).
7. Server startup under 3s for a typical project (TypeScript, 10k files).
8. Server status visible in the status bar: running, starting, crashed, disabled.
9. Circuit breaker: 5 failures in 60s opens the breaker for 10s.
10. Graceful shutdown: `shutdown` request, 5s grace, `exit`, then SIGKILL.
11. Servers are not launched in Restricted mode (REQ-FS-005.3).

#### Failure Modes
- Binary not found: clear error with install instructions, degrade to Tree-sitter-only.
- Memory limit exceeded: SIGKILL and restart; if recurring (3x in 10 min), mark unstable and notify.
- Invalid responses: log, ignore the malformed message, never crash the client.
- Initialization timeout (> 30s): kill, retry once, then mark Failed.

---

### Requirement 26: LSP 3.17+ Feature Support

**ID:** REQ-LANG-002 | **Category:** Language | **Tier:** 1 (MVP)

**User Story:** As a developer, I want every capability my language server offers to be available in the editor, so choosing Helix never costs me language features I already had.

The LSP client SHALL support all LSP 3.17 features with forward-compatibility for newer versions.

#### Acceptance Criteria
1. Text document synchronization (incremental, full as fallback).
2. Completion with snippets, commit characters, resolve, and label details.
3. Hover with markdown rendering.
4. Signature help with trigger and retrigger characters.
5. Go to definition, declaration, type definition, and implementation.
6. Find references with configurable include-declaration.
7. Document symbols and workspace symbols.
8. Code actions (quick fix, refactor, source action) with resolve.
9. Code lens with resolve.
10. Document formatting, range formatting, on-type formatting.
11. Rename with prepare-rename validation.
12. Folding ranges.
13. Selection ranges.
14. Linked editing ranges.
15. Semantic tokens (full, delta, range).
16. Inlay hints with resolve.
17. Call hierarchy (incoming and outgoing).
18. Type hierarchy (supertypes and subtypes).
19. Document highlights.
20. Document links.
21. Document colors with an inline picker.
22. Diagnostics via both push and pull models.
23. Workspace edit application: text edits plus file create, rename, and delete operations.
24. Dynamic registration and unregistration of server capabilities at runtime.
25. Work-done progress and partial result streaming for long-running requests.
26. File operation notifications sent to the server (willCreate/didCreate, willRename/didRename, willDelete/didDelete).
27. Completions appear within 100ms of trigger (p95).
28. Go-to-definition resolves within 200ms for typical files (p95).
29. Unknown or future LSP methods are logged and ignored gracefully.
30. Notebook document support is out of scope for v1.0 and explicitly deferred.

#### Failure Modes
- Capability unsupported by the server: the feature is silently unavailable for that language, with no error shown.
- Request timeout (10s navigation, 30s workspace operations): cancel and show an unobtrusive status-bar notice.

---

### Requirement 27: Tree-sitter Integration

**ID:** REQ-LANG-003 | **Category:** Language | **Tier:** 1 (MVP)

**User Story:** As a developer opening a file in a language with no server installed, I want highlighting, folding, and structural selection to still work, so unsupported languages degrade rather than break.

The system SHALL use Tree-sitter for fallback syntax and structure.

#### Acceptance Criteria
1. Fallback syntax highlighting when semantic tokens are unavailable.
2. Structural navigation: expand and shrink selection by AST node.
3. Code folding fallback at function, class, and block boundaries.
4. Bracket pair detection accurate inside strings and comments.
5. Scope-aware text objects (function body, parameter list, etc.).
6. Structural source for the outline and breadcrumbs when no LSP is available (REQ-ED-008.5).
7. Enclosing-block detection for inline AI edit with no selection (REQ-AI-020.10).
8. Bundled grammars for the top 20 languages; dynamic grammar loading for others.
9. Parse under 50ms for files below 10k lines; incremental re-parse under 10ms per edit.

#### Failure Modes
- Grammar unavailable: fall back to a TextMate/regex tokenizer, then to plain text.
- Parse error: best-effort partial tree, highlight what parses, mark error regions.

---

### Requirement 28: Diagnostics

**ID:** REQ-LANG-004 | **Category:** Language | **Tier:** 1 (MVP)

**User Story:** As a developer, I want every error and warning from every tool in one place with a path to a fix, so I do not have to consult several panels to learn what is broken.

The system SHALL provide a unified diagnostics experience.

#### Acceptance Criteria
1. Problems panel with filtering by severity, source, file, and workspace root.
2. Inline squiggly decorations with severity coloring (error, warning, info, hint).
3. Diagnostic peek: inline detail with the full message, related information, and source links.
4. Quick fix access from a diagnostic (lightbulb, Ctrl+.).
5. Diagnostic navigation with F8 / Shift+F8, scoped to file or workspace.
6. Status bar summary of error and warning counts, opening the Problems panel on click.
7. Source attribution for every diagnostic (which server or linter produced it).
8. Stale diagnostics from crashed or stopped servers are cleared.
9. Diagnostics aggregate from all sources (LSP servers, task problem matchers, plugins) into one model.
10. Diagnostic counts are exposed for file decoration (REQ-FS-003.5) and announced via a live region (REQ-NFR-005.2).

---

### Requirement 29: Integrated Terminal

**ID:** REQ-TERM-001 | **Category:** Terminal | **Tier:** 1 (MVP)

The system SHALL provide an integrated terminal.

**User Story:** As a developer, I want an integrated terminal that feels as responsive as a native terminal application.

#### Acceptance Criteria
1. Multiple terminal instances as tabs, max 20 per window.
2. Split terminals (horizontal/vertical), up to 4 per tab.
3. Shell profiles configurable per platform: PowerShell, cmd, bash, zsh, fish, nushell.
4. Default shell auto-detected from the OS environment.
5. Terminal profiles can be saved and reused.
6. Link detection: file paths open in the editor, URLs open in the browser.
7. Copy/paste with configurable behaviour (select-to-copy, right-click paste).
8. Search within the terminal buffer.
9. Resize and reflow on width change.
10. ANSI support: 256-color, true color, bold, italic, underline, strikethrough.
11. Configurable font family, size, line height, letter spacing.
12. Shell integration: CWD tracking, command boundary detection, command decorations.
13. Input latency under 16ms from keypress to character render.
14. Scrollback of 10,000 lines by default, configurable to 100,000.
15. PTY cleanup on close: SIGHUP, then SIGKILL after 5s.

#### Failure Modes
- Shell process crash: show "Process exited with code X" and offer relaunch.
- PTY allocation failure: OS-specific guidance (e.g. "ConPTY not available").
- Renderer crash: destroy and recreate the terminal view, reconnecting to the existing PTY.

---

### Requirement 30: Task System

**ID:** REQ-TASK-001 | **Category:** Tasks | **Tier:** 1 (MVP)

**User Story:** As a developer, I want my project's build and script commands discovered and runnable from the IDE with their output turned into clickable errors, so I stop retyping commands in a terminal.

The system SHALL provide a task system for running and managing development tasks.

#### Acceptance Criteria
1. Task definitions in `.helix/tasks.json`: shell tasks, process tasks, plugin tasks, with schema validation.
2. Task dependency graphs (`dependsOn`, `preLaunchTask`) with sequential and parallel ordering.
3. Auto-detected tasks: npm/pnpm/yarn scripts, Makefile targets, Cargo commands, Gradle tasks, .NET targets.
4. Problem matchers parse task output into diagnostics via regex patterns.
5. Background tasks with begin/end patterns for watchers (e.g. `tsc --watch`).
6. Terminal management per task: shared, dedicated, or new terminal per run.
7. Commands: Run Task, Re-run Last Task, Stop Task, Restart Task.
8. Script explorer panel: tree of detected scripts per workspace root and per project (REQ-FS-002.4).
9. One-click script execution.
10. Package manager auto-detected from the lockfile.
11. Auto-detection completes within 2s of workspace open.
12. Task output appears in the terminal within 500ms of execution start.
13. Task variables: `${workspaceFolder}`, `${file}`, `${fileDirname}`, `${env:VAR}`, and others.
14. Tasks do not run and are not auto-detected in Restricted mode (REQ-FS-005.3).

#### Failure Modes
- Binary not found: clear error including PATH information and an install suggestion.
- Task timeout: optional hard kill after a user-defined duration (no default).
- Problem matcher regex error: log, skip the matcher, show raw output.
- Circular dependency: detected at definition time, refused with a clear error.

---

### Requirement 31: Test Explorer

**ID:** REQ-TEST-001 | **Category:** Testing | **Tier:** 2

**User Story:** As a developer practising test-driven development, I want to run and debug individual tests from the editor and see coverage inline, so the feedback loop stays inside my flow.

The system SHALL provide a test explorer extensible via plugins.

#### Acceptance Criteria
1. Tree view of discovered tests by file, suite, and test case.
2. Run and debug individual tests, suites, or all tests.
3. Status decorations in the tree and editor gutter: pass, fail, skip, running, queued.
4. Test output panel with stdout/stderr per test.
5. Failure diff view: expected vs actual.
6. Re-run failed tests.
7. Watch mode: auto-run affected tests on file change, debounced.
8. Coverage overlay: line highlighting for covered/uncovered plus branch indicators.
9. Coverage summary per file and per project.
10. Extensible via plugins for different runners (Jest, Vitest, pytest, JUnit, Go test, Cargo test).
11. Test discovery under 5s for projects with fewer than 10,000 tests.
12. Test status icons come from the icon system (REQ-ICON-001.9).

#### Failure Modes
- Runner not installed: setup guidance for the detected framework.
- Test process crash: affected tests marked errored, process output shown.
- Discovery timeout (> 30s): partial results with a manual refresh option.

---

### Requirement 32: DAP Client and Debug UI

**ID:** REQ-DEBUG-001 | **Category:** Debugging | **Tier:** 2

**User Story:** As a developer diagnosing a defect, I want to set conditional breakpoints, inspect state, and step through execution, so I can reason about running code instead of adding print statements.

The system SHALL implement a full DAP client with complete debug UI.

#### Acceptance Criteria
1. Launch and attach configurations in `.helix/launch.json` with schema validation and completion.
2. Compound launch configurations that start multiple sessions together.
3. Breakpoints: line, conditional, hit-count, log (tracepoint), function, exception (caught/uncaught), and data breakpoints where supported.
4. Variable inspection: locals, globals, closure scopes, expandable complex types with lazy child loading.
5. Watch expressions panel: add, edit, remove, evaluate on stop.
6. Call stack navigation across multiple threads, with async stack traces where supported.
7. Step operations: into, over, out, and step-back for adapters supporting reverse debugging.
8. Debug console: REPL with completions and syntax highlighting.
9. Inline values shown as decorations beside code, and value tooltips on hover.
10. Debug toolbar: continue, pause, step into/over/out, restart, stop, disconnect.
11. Breakpoint management panel listing all breakpoints with individual enable/disable and a condition editor.
12. Configuration picker for selecting among defined configurations.
13. Multi-session debugging of several processes simultaneously.
14. Debugger attaches within 3s; variable inspection responds within 200ms; stepping updates the UI within 100ms (p95).
15. Debug adapters are not launched in Restricted mode (REQ-FS-005.3).

#### Failure Modes
- Adapter crash: show the error and offer to restart the session.
- Adapter not found: clear error with install guidance, linking to the marketplace when the adapter is a plugin.
- Breakpoint unbound (e.g. missing source maps): warning decoration with an explanation on hover.
- Session hangs: detected via 10s heartbeat, force-stop offered.

---

### Requirement 33: Core Git Operations

**ID:** REQ-GIT-001 | **Category:** Version Control | **Tier:** 1 (MVP)

**User Story:** As a developer, I want to stage precisely the lines I intend and commit them without leaving the editor, so building a clean history is easy rather than a chore.

The system SHALL provide core Git operations.

#### Acceptance Criteria
1. Repository discovery across workspace roots, including nested repositories.
2. Status by state: staged, unstaged, untracked, conflicts, ignored, with file counts.
3. Stage and unstage at file, hunk, and individual line granularity.
4. Discard changes at file and hunk granularity, with confirmation.
5. Commit with a message editor, warning when the subject line exceeds 72 characters.
6. Amend the last commit with the message pre-filled.
7. Branch operations: create, switch, delete, rename.
8. Stash: save with message, pop, apply, drop, list, show diff.
9. Implementation uses gitoxide (`gix`) for read and performance-critical operations, and shells out to the git CLI for writes and complex operations (rebase, merge, cherry-pick).
10. Status updates are pushed to the frontend within 500ms of a file save.
11. Read-only Git operations remain available in Restricted mode (REQ-FS-005.4).

#### Failure Modes
- Uncommitted changes on branch switch: prompt to stash, commit, or abort.
- Merge conflicts on pull/merge: open the merge editor per conflicted file (REQ-ED-004), or the Tier 1 conflict-marker fallback.
- Stale `.git/index.lock` (older than 1 hour): detected, removal offered.
- Corrupt repository: clear error suggesting `git fsck`; no automatic repair attempted.

---

### Requirement 34: Remote Operations

**ID:** REQ-GIT-002 | **Category:** Version Control | **Tier:** 2

**User Story:** As a developer collaborating through a shared remote, I want fetch, pull, and push with working authentication and honest error messages, so syncing is routine rather than a source of anxiety.

The system SHALL provide Git remote operations.

#### Acceptance Criteria
1. Remote management: add, remove, rename, list with URLs.
2. Fetch all remotes or a specific remote, with prune of stale tracking branches.
3. Pull via merge or rebase, configurable default per branch.
4. Push with upstream tracking, force-with-lease, and an option to push tags.
5. Authentication: SSH keys (agent and key file), credential helpers, personal access tokens, HTTP basic auth, with secrets held per REQ-SEC-002.
6. Progress reporting for network operations over the streaming channel.
7. Cancellation of long-running network operations.
8. Ahead/behind counts per branch, refreshed by auto-fetch on a configurable interval (default 5 minutes, 0 disables).

#### Failure Modes
- Auth failure: error distinguishing key, password, and token problems, with a path to reconfigure.
- Network timeout (default 60s): cancel and report.
- Push rejected (non-fast-forward): explain, suggest pull; never force-push without explicit confirmation.
- SSH host key verification: prompt to accept or reject, and store the decision.

---

### Requirement 35: Advanced Git Workflows

**ID:** REQ-GIT-003 | **Category:** Version Control | **Tier:** 2

**User Story:** As a developer preparing work for review, I want to reorder and squash commits and trace a file's history visually, so I can shape a readable history without memorising git incantations.

The system SHALL provide advanced Git workflows.

#### Acceptance Criteria
1. Merge with conflict detection, opening the merge editor on conflict.
2. Interactive rebase with visual reorder: pick, squash, fixup, edit, drop, and drag-to-reorder.
3. Cherry-pick of a single commit or a range.
4. Tags: create lightweight and annotated, delete, push to remote.
5. Worktrees: create, switch, remove, list (also the mechanism behind agent isolation, REQ-AI-042.1).
6. Git log with graphical branch (DAG) visualization.
7. Log filtering by author, date range, path, and message text.
8. File history: commits touching a file, with inline diff preview and compare-with-previous-revision.
9. Blame: inline annotations (author, date, subject) with full commit details on hover and diff on click.

#### Failure Modes
- Rebase conflict: pause, open the merge editor per conflict, offer continue/abort/skip.
- Cherry-pick conflict: same handling as rebase.
- Worktree creation failure (path exists): clear error with an alternative path suggested.

---

### Requirement 36: Source Control UI

**ID:** REQ-GIT-004 | **Category:** Version Control | **Tier:** 1 (MVP)

The system SHALL provide a source control view.

**User Story:** As a developer, I want complete Git workflow support so I never need to leave the IDE for version control operations.

#### Acceptance Criteria
1. Source Control sidebar grouping changed files by state (staged, changes, untracked, merge conflicts).
2. Clicking a file opens an inline diff preview.
3. Stage, unstage, and discard actions per file and per group.
4. Commit message editor with conventional-commit helpers: type prefix dropdown and scope autocomplete.
5. Branch indicator in the status bar, click to switch or create.
6. Sync status with ahead/behind counts (populated once remotes are configured, REQ-GIT-002.8).
7. Quick actions in the status bar: commit, pull, push, sync, publish branch.
8. Commit message templates, configurable.
9. Commit signing via git config (GPG or SSH).
10. Git decoration colors are published for consumption by the explorer (REQ-FS-003.5) and theming (REQ-THEME-002.5).
11. Blame annotations appear within 1s of activation.
12. Push and pull operations show progress and are cancellable.

---

### Requirement 37: VCS Abstraction Layer

**ID:** REQ-GIT-005 | **Category:** Version Control | **Tier:** 3

**User Story:** As a platform developer, I want source control behind an interface, so supporting a second VCS later is an added implementation rather than a UI rewrite.

The architecture SHALL provide a VCS abstraction layer for future extensibility.

#### Acceptance Criteria
1. A common trait covering status, diff, commit, branch, merge, and log operations.
2. Git as the primary and initially only implementation.
3. A plugin registration point for alternative VCS providers.
4. Source control UI renders from the abstraction, not from Git directly.
5. No Git-specific assumptions leak into core UI components.

**Implementation Note:** This is a refactoring target after Git is fully implemented. Do not over-engineer upfront; extract the interface when a second provider is actually needed.

---

### Requirement 38: Workspace Search and Indexing

**ID:** REQ-SEARCH-001 | **Category:** Search | **Tier:** 1 (MVP)

**User Story:** As a developer in a very large repository, I want search and file-open results to appear as fast as I can type, so exploring unfamiliar code is immediate.

The system SHALL provide fast workspace search with persistent indexing.

#### Acceptance Criteria
1. Text and regex search with ripgrep-level performance, integrated once and consumed by all search surfaces (workspace find, quick open, symbol search) with no duplicate engine.
2. Symbol search across the workspace, sourced from LSP `workspace/symbol`.
3. File path fuzzy matching for quick open, backed by a trigram index.
4. File content trigram index for fast full-text search.
5. Symbol index populated from LSP responses and cached in memory and on disk.
6. Incremental index update within 100ms of a single-file change event.
7. Index persisted to the per-workspace OS cache directory, not inside the workspace, surviving restart and invalidated on file hash mismatch.
8. Index build under 30s for a 100k-file workspace and under 3 minutes for a 500k-file workspace, always in the background and never blocking the UI.
9. Search remains usable during index build, degrading to direct scan for un-indexed paths.
10. Exclude patterns respected (`.gitignore`, `node_modules`, `.git`, build output).
11. Index size cap, configurable (default 200MB), with LRU eviction of rarely-accessed entries.
12. Search history of the last 50 queries, persisted across sessions.
13. Result sets can be pinned for reference (REQ-ED-002.14).

#### Failure Modes
- Index corruption: detected via checksum, rebuilt in the background with notification.
- Index exceeds the configured cap: LRU eviction, with a notice if search quality is affected.
- Search process crash: restart and retry the query once.

---

### Requirement 39: Settings System

**ID:** REQ-CONFIG-001 | **Category:** Configuration | **Tier:** 1 (MVP)

**User Story:** As a developer with personal preferences working on a team with project standards, I want settings to layer predictably, so project conventions win where they should and my preferences apply everywhere else.

The system SHALL provide a layered settings system.

#### Acceptance Criteria
1. Layer precedence, highest first: folder settings, workspace settings, user settings, defaults.
2. Language-specific overrides within any layer (e.g. `[typescript].editor.tabSize`).
3. User settings at `~/.helix/settings.json`.
4. Workspace settings at `.helix/settings.json` per root.
5. JSON editor with schema validation, completion, and inline documentation.
6. GUI settings editor with search, categories, and a modified indicator.
7. Toggle between GUI and JSON views.
8. Changes apply immediately; no restart required for the majority of settings, and settings that do require restart are labelled as such.
9. Reset to default per setting.
10. Settings changes propagate to all open windows (REQ-ARCH-006.8).
11. Settings that specify executable paths are ignored in Restricted mode (REQ-FS-005.3).
12. Secrets are never accepted in settings files; a detected secret is rejected with a warning (REQ-SEC-002.4).
13. Cloud settings sync is explicitly out of scope for v1.0.

#### Failure Modes
- Invalid JSON: load last-known-good, show the parse error at the top of the settings editor, highlight the location.
- Unknown setting key: warn but preserve (forward-compat for plugin settings).
- Type mismatch: use the default, show a warning in the settings editor.

---

### Requirement 40: Keybinding System

**ID:** REQ-CONFIG-002 | **Category:** Configuration | **Tier:** 1 (MVP)

**User Story:** As a developer migrating from another editor, I want to bring my muscle memory with me and rebind anything that differs, so I am productive on day one.

The system SHALL provide a keybinding system.

#### Acceptance Criteria
1. Platform-specific default keybindings for Windows, macOS, and Linux.
2. User overrides at `~/.helix/keybindings.json`, supporting both addition and removal of bindings.
3. When-clause contexts for conditional bindings (`editorTextFocus`, `terminalFocus`, `panelFocus`, `sidebarFocus`, `inSearch`, `debugActive`, and others).
4. Multi-chord keybindings (e.g. Ctrl+K Ctrl+C) with a chord timeout of 1.5s.
5. Conflict detection with a resolution UI showing the competing commands.
6. Keybinding editor with search, filter by command, conflict filter, and shortcut recording.
7. Plugin-contributed keybindings.
8. Importable preset schemes: VS Code, JetBrains, Vim (basic motions), Emacs (basic).
9. Resolution precedence: user, then plugin, then default; within the same precedence the last definition wins.
10. Every command reachable by keyboard, supporting REQ-NFR-005.3.

#### Failure Modes
- Invalid keybinding definition: skipped with a warning; remaining bindings still load.
- Unresolvable conflict: last-defined wins and the conflict is flagged in the keybinding editor.

---

### Requirement 41: Command-Line Interface

**ID:** REQ-CLI-001 | **Category:** Platform | **Tier:** 2

The system SHALL provide a command-line interface for launching and controlling the application.

**User Story:** As a developer who lives in the terminal, I want to open files and folders in Helix from the shell, so the IDE fits my existing workflow.

**Rationale:** REQ-DIST-002 already references `helix --rollback` without any requirement defining the CLI that provides it.

#### Acceptance Criteria
1. `helix [path...]` opens folders and files; multiple paths open as multi-root or multiple editors as appropriate.
2. Flags: `--new-window`, `--reuse-window`, `--goto file:line:col`, `--diff a b`, `--wait` (block until the editor closes, for use as `$EDITOR`/`$GIT_EDITOR`), `--add` (add folder to the current workspace).
3. Diagnostic flags: `--version`, `--status`, `--log-level`, `--verbose`, `--user-data-dir`, `--disable-extensions`, `--safe-mode`.
4. Maintenance flags: `--rollback` (REQ-DIST-002.4), `--install-plugin`, `--uninstall-plugin`, `--list-plugins`.
5. Launching when an instance is already running forwards the request to that instance rather than starting a second kernel (per REQ-ARCH-006).
6. Shell command installation: an "Install 'helix' command in PATH" action per platform, plus documented manual installation.
7. Exit codes are meaningful and documented (0 success, non-zero for specific failure classes), so the CLI is scriptable.
8. `--wait` integrates correctly as a git editor: closing the editor returns control and the correct exit status.
9. Shell completions for bash, zsh, fish, and PowerShell.
10. All CLI output is plain text and honours `--json` where structured output is useful (`--status`, `--list-plugins`).

#### Failure Modes
- Path does not exist: clear message and non-zero exit; no empty window created for an obvious typo.
- Existing instance unreachable (stale socket/lock): detect, clean up, start fresh.
- `--wait` with the instance killed externally: return a distinct non-zero exit code so callers can distinguish abort from success.

---

### Requirement 42: LLM Provider Architecture

**ID:** REQ-AI-001 | **Category:** AI | **Tier:** 1 (MVP)

**User Story:** As a developer, I want to choose my own model provider and swap it later, so I am not locked into one vendor's pricing, privacy terms, or availability.

The system SHALL provide a provider-agnostic LLM interface with capability-based routing.

#### Acceptance Criteria
1. A unified provider trait exposing chat, completion, and embedding operations, each with a streaming variant.
2. Supported providers: OpenAI, Anthropic, Google (Gemini), Azure OpenAI, Ollama, llama.cpp, and any OpenAI-compatible endpoint.
3. Providers are registered via configuration and can be added, removed, and reordered in settings.
4. API keys are stored in the OS keychain and referenced by name from configuration, never stored in plain text (REQ-SEC-002).
5. Connection testing and per-provider health check commands.
6. Model capability declarations: context window, tool support, vision support, JSON mode, speed tier, cost tier.
7. Automatic model selection based on task requirements (REQ-AI-002).
8. Provider health status shown in the status bar: connected, degraded, offline.
9. Circuit breaker per provider: 3 failures in 30s opens the breaker for 30s.

#### Failure Modes
- Provider unreachable: retry with backoff (1s, 2s, 4s), mark degraded after 3 failures, fall through to the next provider in the chain.
- Invalid API key: clear error linking to settings; the provider is disabled until the key is updated.
- Rate limited (429): respect `Retry-After`, queue requests, show a rate-limited status.
- Malformed response: log, retry once, then surface a user-facing error.

---

### Requirement 43: Model Routing and Budget

**ID:** REQ-AI-002 | **Category:** AI | **Tier:** 1 (MVP)

**User Story:** As a developer paying per token, I want fast cheap models for routine work and strong models for hard problems, with a spending cap I control, so AI assistance never produces a surprise bill.

The system SHALL route AI requests to appropriate models based on task characteristics and budget.

#### Acceptance Criteria
1. The router selects a model based on task type (completion, chat, planning, embedding, inline edit), required capabilities, latency requirement, and cost constraint.
2. Per-task-type user override in settings.
3. Fallback chain: if the preferred model is unavailable, try the next in configured order.
4. Budget limits, all configurable: per-session tokens, daily tokens, monthly cost.
5. Warning at 80% of any limit.
6. Hard stop at 100%: AI features disabled until reset or a new period, with a clear notification offering to raise the limit.
7. Token usage tracked per request, session, day, and month.
8. Token usage displayed in the AI panel and status bar.
9. Graceful degradation when all cloud providers are unavailable, falling back to a local model if configured.
10. Non-AI IDE functionality is unaffected by any AI outage or budget exhaustion (REQ-NFR-003.2).

#### Failure Modes
- Budget exhausted: AI disabled with an actionable notification; nothing else degrades.
- All providers down: "AI offline" indicator; AI features disabled, no error spam.
- Local model OOM: detected via exit code; reduce context window and retry, or switch to a cloud provider.

---

### Requirement 44: Local Model Management

**ID:** REQ-AI-003 | **Category:** AI | **Tier:** 2

The system SHALL provide management of locally executed models.

**User Story:** As a developer with confidentiality constraints, I want to run models on my own machine and manage them from the IDE, so I get AI assistance without sending code to a third party.

#### Acceptance Criteria
1. Local runtime integration via Ollama or a compatible runtime.
2. Hardware detection and reporting: CUDA (NVIDIA), Metal (Apple), Vulkan/ROCm (AMD), CPU fallback, with available VRAM and system RAM.
3. Model catalog UI listing available local models with size, quantization, context window, and hardware suitability for the detected machine.
4. Download with progress, pause, resume, and cancel; integrity verified by checksum.
5. Model deletion with disk space reclaimed and reported.
6. Per-model configuration: context window, quantization variant, GPU layer offload.
7. Guidance when a chosen model exceeds available memory, with a recommended alternative rather than a failed load.
8. Runtime lifecycle managed by the kernel: start on demand, stop when idle for a configurable period.
9. Local models participate in routing and fallback identically to cloud providers (REQ-AI-002.1).
10. Fully functional with no internet connection once models are downloaded (REQ-NFR-003.2).

#### Failure Modes
- Download interrupted: resume from the partial file on retry.
- Runtime not installed: clear installation guidance for the platform; cloud providers keep working.
- Insufficient disk space: refuse the download before starting, reporting required vs available space.

---

### Requirement 45: Inline AI Completion

**ID:** REQ-AI-010 | **Category:** AI | **Tier:** 1 (MVP)

The system SHALL provide Copilot-style inline completions.

**User Story:** As a developer, I want AI-powered code suggestions that appear naturally as I type, without disrupting my flow.

#### Acceptance Criteria
1. Ghost-text suggestions rendered as dimmed text ahead of the cursor, visually distinct from the autocomplete popup.
2. Accept full suggestion with Tab.
3. Accept next word with Ctrl/Cmd+Right.
4. Accept line with a configurable binding.
5. Dismiss with Escape.
6. Cycle suggestions with Alt+] and Alt+[.
7. Multi-line, indentation-aware completions.
8. Request fires after a 300ms typing pause.
9. Suggestions are suppressed if the response takes longer than a configurable latency budget (default 500ms).
10. Context gathering: a focused window around the cursor, open tabs ranked by relevance, imports, and project structure hints.
11. Disable per language, per file glob, or globally.
12. No UI flicker or layout shift when a suggestion appears or disappears.
13. Local-only telemetry of acceptance and dismissal rates, for the user's own insight.
14. Coexists with LSP completions: ghost text is dismissed while the autocomplete popup is visible.
15. In-flight requests are cancelled on new keystrokes.

#### Failure Modes
- Request timeout: silently dismissed, no error shown.
- Syntactically invalid completion: still displayed for the user to judge; never auto-accepted.
- Provider switch mid-stream: cancel the in-flight request and re-debounce.

---

### Requirement 46: Inline AI Edit

**ID:** REQ-AI-020 | **Category:** AI | **Tier:** 1 (MVP)

**User Story:** As a developer, I want to describe a change to selected code and review the proposed diff before it lands, so AI edits are always something I approve rather than something I discover.

The system SHALL provide inline AI-assisted editing.

#### Acceptance Criteria
1. Triggered by Ctrl/Cmd+K with a selection.
2. Natural language instruction entered in an inline field above the editor, not a modal.
3. Patch previewed as diff decorations in the editor, toggleable between inline and side-by-side.
4. Accept, Reject, and Iterate actions.
5. Acceptance is a single undo step.
6. Context includes surrounding code, file imports, and related type definitions.
7. Streaming diff display as the patch generates.
8. Inline edit UI appears within 200ms of trigger.
9. Generated patch displays within 3s for typical requests (under 100 lines of context).
10. With no selection, the enclosing logical block is detected via Tree-sitter (REQ-LANG-003.7) and used as the target.
11. Multiple sequential edits without closing the input.
12. Instruction history accessible with the up arrow.

#### Failure Modes
- Generation fails: inline error with retry.
- File changed during generation: regenerate with updated context and warn.
- Response over 500 lines changed: warn before applying and offer a full diff view first.

---

### Requirement 47: AI Chat

**ID:** REQ-AI-030 | **Category:** AI | **Tier:** 1 (MVP)

**User Story:** As a developer, I want to discuss my actual code with a model by attaching the exact files, errors, and diffs in question, so the answers are about my project rather than generic advice.

The system SHALL provide an AI chat panel with context attachments and conversation management.

#### Acceptance Criteria
1. Conversational interface with GFM markdown rendering, including tables and task lists.
2. Syntax-highlighted code blocks with language labels.
3. Apply-to-editor action on each code block (insert or replace selection).
4. Copy action on each code block.
5. Multi-turn conversation with context retention within the model's window.
6. Model selection per conversation.
7. Token usage per message and per session.
8. Streaming responses rendered token-by-token.
9. Stop-generation and regenerate-last-response actions.
10. Context attachments via `@` mentions: `@file`, `@folder`, `@selection`, `@symbol`, `@diagnostics`, `@terminal`, `@test`, `@git-diff`, `@workspace`.
11. Drag-and-drop of files into the chat input.
12. Autocomplete for `@` mentions with fuzzy search.
13. Attachment chips showing what is attached, expandable and removable.
14. Stale-attachment indication when an attached file changes during the conversation.
15. Multiple chat sessions listed in a sidebar, with rename and delete.
16. Conversation persistence managed by the kernel, stored in the per-workspace OS state directory and encrypted at rest.
17. Branch a conversation from any message.
18. Export a conversation as markdown.
19. Conversations are never sent to telemetry.
20. Maximum conversation length configurable (default 100 messages) with a warning at 80.
21. Chat remains available in Restricted mode (REQ-FS-005.4).

#### Failure Modes
- Context exceeds the model window: auto-truncate oldest messages with a visible indicator, prioritizing recent messages and explicit attachments.
- Streaming interrupted: show the partial response with an interrupted indicator and offer retry.
- Attached file deleted: show a "no longer available" placeholder.
- Storage over 1GB: LRU deletion of the oldest conversations.

---

### Requirement 48: Autonomous Agent

**ID:** REQ-AI-040 | **Category:** AI Agent | **Tier:** 3

The system SHALL provide an autonomous coding agent.

**User Story:** As a developer, I want to describe a task in natural language and have the AI plan and execute it, so I can focus on higher-level decisions while the agent handles implementation details.

#### Acceptance Criteria
1. Natural language task description leads to planning and then execution.
2. Multi-step plan generation with a visible plan (numbered steps, estimated effort per step).
3. Plan is editable before approval: reorder, remove, and amend steps.
4. File creation, modification, and deletion within the agent worktree.
5. Terminal command execution within the sandboxed environment.
6. Test running with result interpretation.
7. Build error detection with a self-repair loop, max 3 retries per step.
8. Iterative refinement until the task completes or the budget is exhausted.
9. Clarifying questions pause execution and prompt the user.
10. Activity log with every action timestamped and reviewable.
11. One agent task per workspace at a time; additional requests queue.
12. Rate limited to 10 actions per second to prevent runaway loops.
13. The agent does not run in Restricted mode (REQ-FS-005.3).

---

### Requirement 49: Agent Trust and Approval Model

**ID:** REQ-AI-041 | **Category:** AI Agent | **Tier:** 3

The agent SHALL implement configurable trust levels.

**User Story:** As a developer, I want to configure how much autonomy the AI agent has so I can balance speed with control based on the sensitivity of my project.

#### Acceptance Criteria
1. Full autonomy: all steps execute without interruption; the user reviews on completion.
2. Gated autonomy: execution pauses at configured checkpoint categories (file write, file delete, terminal command, git operation, network request, dependency install).
3. Supervised: execution pauses after every action.
4. Per-project trust configuration in `.helix/agent.json`.
5. Per-task trust override selected at task start.
6. Trust escalation: the agent may request elevated permission for a specific action with a justification.
7. Approval UI showing the intended action, its details (command text, file path, diff preview), and Approve / Reject / Modify.
8. Audit trail recording every approval and rejection with a timestamp.
9. Emergency stop via a global shortcut (triple Escape) or a dedicated button, halting the agent immediately.

#### Failure Modes
- Approval prompt unanswered for 5 minutes: the agent pauses gracefully and can resume later.
- Emergency stop during a file write: atomic writes guarantee no partial corruption.

---

### Requirement 50: Agent Workspace Isolation

**ID:** REQ-AI-042 | **Category:** AI Agent | **Tier:** 3

**User Story:** As a developer delegating work to an agent, I want it confined to its own branch and sandbox with hard resource limits, so a confused agent cannot damage my working tree or my machine.

The agent SHALL operate in an isolated environment to protect user work.

#### Acceptance Criteria
1. A Git worktree on a separate branch holds all agent changes.
2. Sandboxed shell execution with filesystem restriction to the worktree, enforced by OS mechanism: namespaces/seccomp on Linux, App Sandbox on macOS, Job Objects with a restricted token on Windows.
3. Network egress whitelist, defaulting to package registries only and denying all other hosts, with user-configurable additions.
4. Process limits: max 10 child processes, 512MB memory per process, 60s per command, all configurable.
5. Kernel-side path validation before every agent file operation, rejecting traversal and symlink escape.
6. Per-task resource budgets: tokens (default 100k), wall-clock time (default 30 min), file writes (default 100), commands (default 50).
7. Budget warnings at 80% and hard stop at 100%.
8. Worktree deleted on task abandonment or after merge.

#### Failure Modes
- Out-of-scope file access: blocked, logged, and counted; 3 violations auto-pause the task for review.
- Non-whitelisted network request: blocked, logged, agent informed so it can adapt.
- Memory limit exceeded: process killed, agent notified, approach adjusted or user consulted.
- OS sandbox unavailable: fall back to path-prefix validation with an explicit warning that security is reduced.

---

### Requirement 51: Agent State and Recovery

**ID:** REQ-AI-043 | **Category:** AI Agent | **Tier:** 3

**User Story:** As a developer, I want to interrupt a long agent task and resume or rewind it later, so a restart or a change of mind does not throw away completed work.

The system SHALL provide agent state management and crash recovery.

#### Acceptance Criteria
1. Agent state persisted across sessions: task context, plan state, current step.
2. Checkpoint at each significant action (file write, command execution, test run), implemented as a commit in the worktree.
3. Rollback to any checkpoint, restoring the worktree to that state.
4. Interrupted tasks are detected on restart and the user is offered resume or discard.
5. Timeline view: chronological actions with status, duration, and diff summary; clicking an action shows its detail.
6. Cumulative diff view of all agent changes since task start.
7. State stored in the per-workspace OS state directory, not inside the workspace, capped at 100MB per workspace with LRU cleanup.

#### Failure Modes
- State corruption: discard the corrupted state and offer restart from the last valid checkpoint.
- Checkpoint creation fails (disk full): warn and continue without a checkpoint, marking the task non-resumable from that point.
- App crash mid-execution: incomplete task detected on restart, resume offered from the last checkpoint.

---

### Requirement 52: Agent Review and Merge

**ID:** REQ-AI-044 | **Category:** AI Agent | **Tier:** 3

**User Story:** As a developer, I want to review agent output file by file and hunk by hunk, with tests run before I accept, so I remain the author of record for what enters my branch.

The system SHALL provide review of agent work before merging into the user's working tree.

#### Acceptance Criteria
1. Review panel with a full diff of agent changes against the user's branch.
2. Per-file accept and reject toggles.
3. Per-hunk accept and reject within files.
4. Inline commenting on agent changes, stored locally.
5. Run Tests against the agent worktree before merge.
6. Run Build to verify the build passes with agent changes.
7. Test and build results shown in the review panel with a link to full output.
8. Merge applies accepted changes to the working tree, either as a commit with a generated message or as uncommitted changes.
9. Discard All deletes the worktree and abandons the changes.
10. Partial merge applies accepted content and retains rejected content in the worktree for later.

#### Failure Modes
- Merge conflicts because the user edited the same files: open the merge editor per conflict.
- Test failures in the worktree: highlighted in review with a warning before merge; the merge is not blocked, the user decides.

---

### Requirement 53: AI-Enhanced Workflows

**ID:** REQ-AI-050 | **Category:** AI | **Tier:** 2

**User Story:** As a developer, I want AI help with the writing that surrounds coding — commit messages, PR descriptions, docs, test scaffolding — so the routine prose costs me less time.

The system SHALL provide AI-enhanced development workflows.

#### Acceptance Criteria
1. Commit message generation from the staged diff, producing a subject and body the user edits before committing.
2. PR description generation from the branch diff against its base.
3. Code review assistance: highlight potential issues, suggest improvements, explain complex changes.
4. Test generation from function signatures and implementations.
5. Documentation generation (JSDoc, docstrings, README sections).
6. Error explanation from diagnostics or terminal output, in plain language with a suggested fix.
7. Refactoring suggestions from detected code smells.
8. All generated content is presented as a suggestion requiring confirmation; nothing is auto-applied.
9. Thumbs up/down feedback stored locally as a quality signal.

---

### Requirement 54: MCP Support

**ID:** REQ-AI-060 | **Category:** AI | **Tier:** 2

**User Story:** As a developer with organization-specific tools, I want to plug them into the IDE's AI through a standard protocol, so the agent can use my systems without me building a bespoke integration.

The system SHALL support Model Context Protocol for extensible AI tool use.

#### Acceptance Criteria
1. MCP client: connect to external MCP servers and discover their tools, resources, and prompts.
2. MCP server hosting: expose IDE context (files, symbols, diagnostics, workspace structure) as MCP resources.
3. Tool registration: MCP tools appear in the agent's tool palette.
4. Resource access: MCP resources available as chat context attachments.
5. Prompt template loading from MCP servers, surfaced in the command palette.
6. Server lifecycle managed by the kernel: start, stop, restart.
7. Server configuration in `.helix/mcp.json` (command, args, env, disabled flag).
8. Multiple servers running simultaneously.
9. Health monitoring with restart on crash.
10. MCP servers are not launched in Restricted mode (REQ-FS-005.3).
11. Tool output is treated as untrusted input in prompt construction (REQ-SEC-003.3).

#### Failure Modes
- Server crash: auto-restart with backoff; its tools are temporarily unavailable.
- Server timeout: per-server configurable timeout, then cancel and report.
- Incompatible protocol version: log and disable the server with a notification.

---

### Requirement 55: Project Genesis / Greenfield Agent

**ID:** REQ-AI-070 | **Category:** AI Agent | **Tier:** 3

**User Story:** As a developer with only a product idea, I want Helix to create a viable project baseline before normal agent development begins, so I do not have to prepare a repository, select every tool, or scaffold each application manually.

The system SHALL turn a greenfield idea into a verified, version-controlled workspace that can enter the normal isolated agent workflow.

#### Acceptance Criteria
1. A natural-language idea is converted into an editable product specification containing scope, user-visible capabilities, modules, non-functional constraints, and testable acceptance criteria.
2. Helix proposes an architecture and stack with explicit versions, explains material trade-offs, and honours user-mandated technologies and local-only/privacy constraints.
3. Genesis works when the target is empty, absent, or not a Git repository; it does not assume an existing workspace or worktree.
4. Before changing the machine or target directory, Helix produces a preflight plan covering environment requirements (REQ-AI-074), selected skills (REQ-AI-075), downloads, commands, ports, services, and estimated resource use.
5. Scaffolding runs in a dedicated temporary sandbox outside the final target. The target is populated only after required scaffold and baseline verification steps succeed.
6. The workflow can create multi-project systems, including frontend, backend, database, shared packages, infrastructure configuration, and a root-level developer entry point.
7. Helix initializes version control, writes an appropriate ignore file, records the generated specification and architecture decision summary, and creates an initial baseline commit.
8. Generated projects contain no production credentials. Secret values are requested through secret management; committed files contain only documented placeholders or environment-variable names.
9. The baseline runs its declared build, lint, and smoke-test checks before handoff. A failing baseline is repaired within the configured budget or presented as a failed genesis with complete diagnostics.
10. After baseline creation, Helix registers the workspace and hands it to the standard planner, worktree isolation, execution, verification, review, and merge workflow (REQ-AI-040 through REQ-AI-044).
11. Genesis state is checkpointed and resumable. Re-running after interruption detects completed deterministic steps and does not duplicate projects, dependencies, or commits.
12. Local-only mode can perform the workflow with an Ollama or llama.cpp model and cached skills/toolchains; unavailable network dependencies are reported before mutation begins.

#### Failure Modes
- Required runtime or SDK unavailable: pause before scaffolding and offer an environment plan; never improvise an unapproved global installation.
- Scaffold command partially fails: retain the sandbox and diagnostics for repair, leaving the final target absent or unchanged.
- Target becomes non-empty or changes during generation: stop and request a new target or explicit merge strategy.
- Baseline verification cannot converge within budget: preserve the sandbox and specification, report every failed check, and offer resume, revise architecture, or discard.

---

### Requirement 56: Native Agent Tool Protocol

**ID:** REQ-AI-071 | **Category:** AI | **Tier:** 1 (MVP)

**User Story:** As a platform developer, I want every provider to expose tool use through one typed protocol, so agent behavior is independent of vendor response formats and never relies on parsing executable intent from ordinary model prose.

The system SHALL normalize native model tool calling and structured output into a provider-independent Helix protocol.

#### Acceptance Criteria
1. The protocol defines canonical `ToolDefinition`, `ToolCall`, `ToolResult`, `ToolError`, and streamed `ModelEvent` types, with a unique call ID linking every result to its request.
2. Tool inputs and structured model outputs are described by JSON Schema and validated in the kernel before dispatch or consumption.
3. Provider adapters map native OpenAI, Anthropic, Gemini, Ollama, llama.cpp-compatible, and OpenAI-compatible tool events into the canonical types without exposing provider-specific payloads to the agent runtime.
4. Streaming preserves ordered text, reasoning-summary where exposed, tool-call delta, completed tool-call, usage, and terminal events without requiring consumers to concatenate arbitrary JSON fragments.
5. The protocol supports single, parallel, and multi-turn tool calls, while the runtime remains free to serialize calls whose tools conflict or require approval.
6. A model response containing tool-shaped JSON in ordinary text is treated as text, never as executable intent.
7. Unknown tools, malformed arguments, schema violations, duplicate call IDs, and results for unknown calls are rejected as typed errors and recorded in the audit trail.
8. Tool definitions declare risk category, required trust capability, timeout, idempotency, concurrency policy, and maximum output size.
9. Tool results are size-bounded, marked as trusted system output or untrusted external data, and may carry structured content, text, artifacts, and retryability.
10. Capability negotiation makes models without reliable native tool support ineligible for autonomous execution while still allowing chat, completion, or planning where suitable.
11. Contract tests replay provider fixtures, including streaming and parallel calls, and prove that equivalent provider responses produce identical canonical events.

#### Failure Modes
- Provider emits an incomplete streamed call: discard the incomplete call, preserve any safe text, and return a typed interrupted error.
- Model repeatedly violates a tool schema: retry once with the validation error, then route to a capable fallback or pause.
- Provider claims tool support but fails conformance tests: mark that model capability degraded and exclude it from agent routing.

---

### Requirement 57: Context Engine

**ID:** REQ-AI-072 | **Category:** AI | **Tier:** 2

**User Story:** As a developer using a finite-context model, I want Helix to retrieve the smallest relevant view of my project automatically, so local and cloud models can reason about large repositories without receiving the whole repository.

The system SHALL provide a shared, budgeted context engine for chat, inline AI, planning, execution, and verification.

#### Acceptance Criteria
1. Context sources include repository map, file and symbol indexes, project/dependency graph, lexical and semantic retrieval, open and recently modified files, diagnostics, test and terminal failures, Git diff/history, explicit attachments, and agent memory.
2. Every context item carries provenance, source revision/hash, trust classification, retrieval reason, token estimate, and freshness state.
3. Retrieval combines deterministic signals and semantic similarity; semantic retrieval has a local embedding option and a lexical-only fallback when embeddings are unavailable.
4. The engine constructs context against an explicit token budget, reserving space for system instructions, conversation, tool schemas, tool results, and model output.
5. Repository maps and summaries are hierarchical, incrementally updated, and invalidated when their source files, symbols, project graph, or configuration change.
6. Long-running tasks compact old observations and tool results into checkpointed summaries while retaining links to original evidence and recent modifications verbatim.
7. Full file contents are fetched only when selected by retrieval, explicitly attached, or requested by a tool; the default strategy never sends the complete repository.
8. Secret, exclusion, binary, workspace-trust, and provider-privacy rules are enforced before any context leaves the kernel or reaches a remote model.
9. The engine exposes one API used by chat, completion, inline edit, project genesis, planners, executors, and specialist agents rather than separate retrieval implementations.
10. A context-inspection view shows what was selected, omitted, summarized, stale, or blocked and why.
11. Tests measure retrieval relevance on checked-in fixtures, enforce token budgets, detect stale-summary invalidation, and verify that excluded or secret content never enters prompts.

#### Failure Modes
- Index unavailable or rebuilding: fall back to direct lexical search, open files, and explicit context with an indexing indicator.
- Context exceeds budget after mandatory items: summarize lower-priority items, then request a larger-context model or narrower task rather than truncating system/tool contracts.
- Embedding provider unavailable: continue with deterministic and lexical ranking; autonomous execution does not stop solely because semantic retrieval is offline.

---

### Requirement 58: Verification Agent

**ID:** REQ-AI-073 | **Category:** AI Agent | **Tier:** 3

**User Story:** As a developer delegating a runnable application, I want Helix to interact with and inspect the result like a user, so successful compilation is not mistaken for a working product.

The system SHALL expose runtime, browser, visual, and accessibility verification as bounded native agent tools.

#### Acceptance Criteria
1. The agent can launch a declared application or preview, wait for readiness, open a page, navigate, inspect the accessibility tree and DOM, click, type, select, scroll, and read visible text.
2. Verification tools capture screenshots, console messages, uncaught errors, failed requests, HTTP status, selected network timing, and page accessibility violations.
3. Browser sessions use an isolated profile with configurable viewport, locale, color scheme, reduced-motion setting, and deterministic storage/cookie reset.
4. Screenshots may be routed only to a vision-capable model; DOM/accessibility evidence remains available to non-vision and local models.
5. A verification plan derives from product acceptance criteria and records each action, assertion, artifact, and result as reviewable evidence.
6. Failed verification feeds a bounded diagnose-repair-rebuild-reverify loop through the normal agent runtime and budget controls.
7. The agent can invoke existing unit, integration, E2E, lint, build, and accessibility harnesses and correlate their failures with browser evidence.
8. External navigation, authentication with real accounts, file uploads, downloads, clipboard access, camera/microphone, and destructive actions require explicit capability grants.
9. Secrets used during verification are injected from secret management, masked in screenshots/logs where possible, and never written into generated tests or prompts.
10. Final review links every claimed acceptance criterion to passing automated evidence or a clearly marked manual verification gap.

#### Failure Modes
- Application never becomes ready: capture process output and health probes, stop leaked processes, and return a typed launch failure.
- Browser crashes or hangs: terminate the isolated session, preserve available artifacts, retry once, then pause.
- Visual model unavailable: continue DOM, accessibility, console, network, and screenshot-capture checks; mark visual interpretation as not run.
- Flaky assertion: rerun under the configured retry policy and label instability rather than silently treating a retry as clean success.

---

### Requirement 59: Development Environment Manager

**ID:** REQ-AI-074 | **Category:** AI Agent | **Tier:** 3

**User Story:** As a developer asking for a new application, I want Helix to understand and prepare the required toolchains safely, so project creation does not fail at the first missing SDK or mutate my machine without consent.

The system SHALL detect, plan, configure, and verify development environments as an explicit managed capability.

#### Acceptance Criteria
1. Environment discovery reports installed versions, executable paths, architecture, package managers, version managers, containers, available disk, memory, ports, and relevant accelerators without modifying the machine.
2. Supported tool families include Node.js and npm/pnpm/yarn, Java and Maven/Gradle, Python, Go, Rust, .NET, Android SDK, Docker/Podman, database runtimes, and framework CLIs.
3. A declarative environment plan lists required versions, compatibility constraints, source, download size, install scope, commands, environment variables, ports, services, and rollback steps.
4. Helix prefers existing compatible tools, then project-local or version-manager installations, then containers; global machine changes are a last resort and always require approval.
5. Downloads and installations use the trust and approval system, verify integrity or package-manager provenance, and never invoke an unreviewed command assembled from model text.
6. The resolved environment is captured as an immutable task snapshot so every tool execution uses the same paths and versions.
7. Runtime and service lifecycle includes start, readiness probe, log capture, stop, cleanup, port-conflict handling, and orphan recovery after a crash.
8. Environment state is shareable through generated project declarations such as lockfiles, tool-version files, dev containers, or documented prerequisites without committing machine-specific paths or secrets.
9. Offline mode reports which cached runtimes, packages, images, and skills are sufficient before execution begins.
10. Health checks distinguish missing, incompatible, installable, ready, degraded, and externally managed tools and provide actionable remediation.

#### Failure Modes
- Required install needs elevation: show the exact user-run command and stop; Helix never requests or captures an administrator password.
- Version conflict with an existing project: isolate the requested version instead of replacing the user's global default.
- Download or integrity check fails: remove or quarantine the partial artifact and leave the previous environment usable.
- Service port occupied: identify the owner where permitted and choose an approved alternate or ask the user; never terminate an unrelated process automatically.

---

### Requirement 60: Skills and Project Recipes

**ID:** REQ-AI-075 | **Category:** AI Agent | **Tier:** 3

**User Story:** As a developer using a smaller local model, I want common project operations backed by deterministic recipes, so reliability comes from known procedures rather than the model reinventing framework commands and conventions.

The system SHALL provide versioned, composable skills that models select and the kernel executes through native tools.

#### Acceptance Criteria
1. A skill manifest declares identity/version, purpose, supported platforms/stacks, inputs, prerequisites, required capabilities, ordered steps, expected outputs, verification, rollback, and compatible Helix API range.
2. Built-in skills cover at minimum Angular, React/Vite, Next.js, Spring Boot, FastAPI, PostgreSQL, Dockerization, authentication, and unit/integration/E2E test setup.
3. Skill steps invoke typed native tools or other skills with schema-validated inputs; arbitrary shell fragments generated in model prose are not executable skill definitions.
4. Skill selection considers the requested stack, detected environment, project graph, existing files, user policy, offline availability, and pinned recipe version.
5. Skills are composable through declared outputs and prerequisites, with cycle detection and a rendered execution plan before mutation.
6. Deterministic steps are idempotent or declare a safe precondition and rollback; rerunning a completed step cannot silently duplicate configuration.
7. User and plugin-contributed skills are treated as untrusted executable content, require signature/source attribution and capability review, and are disabled in Restricted mode.
8. A dry-run reports files, commands, downloads, services, and trust gates without executing them.
9. Skill versions used by a genesis or agent task are checkpointed for reproducibility; upgrades never alter an in-progress task.
10. Tests execute each built-in skill against clean fixtures and supported platform matrices, verifying expected project structure and declared checks.

#### Failure Modes
- No compatible skill exists: fall back to a visible model-generated plan using native tools, clearly marked as non-recipe execution and subject to stricter approval.
- Skill precondition fails: do not execute later steps; explain the detected state and offer compatible alternatives.
- Contributed skill requests undeclared capability: deny the step and record a security violation.

---

### Requirement 61: Specialist Agents and Delegation

**ID:** REQ-AI-076 | **Category:** AI Agent | **Tier:** 3

**User Story:** As a developer delegating a large task, I want one accountable orchestrator to assign bounded specialist work when useful, so complex projects gain focused review without becoming an uncontrolled swarm.

The system SHALL support optional specialist delegation under one orchestrator and one shared agent runtime.

#### Acceptance Criteria
1. Helix ships with one orchestrator; specialist delegation is optional and disabled unless the task complexity or user policy enables it.
2. Initial specialist roles are Architect, Implementation, Test, UI Review, Security, and Documentation, each defined by a versioned role contract rather than a free-form persona.
3. Every delegation has a bounded objective, input context manifest, allowed tools, path scope, model route, token/time/action budget, expected artifact, and completion criteria.
4. Specialists use the same sandbox, worktree, context engine, tool protocol, trust decisions, audit log, and emergency stop as the orchestrator; they cannot create independent privileges or hidden workspaces.
5. Specialists return structured findings, patches, test evidence, decisions, or questions with provenance. The orchestrator validates and accepts or rejects the handoff before continuing.
6. Concurrent read-only specialists are allowed. Concurrent writes require disjoint declared path scopes and conflict detection; otherwise work is serialized.
7. Delegation depth defaults to one and recursive delegation is denied unless explicitly configured with a global hard limit.
8. The UI shows active and completed delegations, budgets, model choice, evidence, and ownership of every change.
9. Cancelling or pausing the parent task propagates immediately to every specialist and child process.
10. Tests cover budget containment, permission inheritance, conflicting writes, cancellation propagation, malformed handoffs, and complete audit attribution.

#### Failure Modes
- Specialist fails or exhausts its budget: return partial evidence to the orchestrator, which retries with a revised bounded task, chooses another model, proceeds itself, or asks the user.
- Two specialists propose conflicting changes: stop automatic application and present both artifacts with the conflict identified.
- Specialist requests broader permissions: route the request through the parent task's approval policy; never grant it implicitly.

---

### Requirement 62: Plugin Architecture

**ID:** REQ-PLUG-001 | **Category:** Plugins | **Tier:** 4

**User Story:** As a developer, I want to extend the IDE with third-party plugins without accepting that any one of them can crash it or read my whole disk, so extensibility does not cost me stability or safety.

The system SHALL support a hybrid plugin isolation model.

#### Acceptance Criteria
1. WASM-sandboxed plugins for lightweight extensions (themes, icon themes, formatters, linters, snippets).
2. Process-isolated plugins for heavy extensions (language servers, debug adapters, AI agents, runtimes).
3. Plugin API surface covering: editor (content, decorations, selections), workspace (files, roots, settings), languages (provider registration), debug (adapter registration), terminal (create, input, output), AI (context providers, tools, prompt templates), UI (views, tree items, status bar, webviews, toolbar), commands, configuration, icons and themes, and events.
4. Manifest format (`plugin.json`) declaring name, version, publisher, description, license, repository, API version, entry point, activation events, capabilities, contributions, and dependencies.
5. Lifecycle: install, enable, activate, deactivate, disable, uninstall.
6. Dependency resolution by topological sort with circular detection.
7. Hot-reload for WASM plugins without IDE restart.
8. Plugin settings contributions appear in the settings UI.
9. Plugin keybinding contributions.
10. Resource limits: WASM 64MB memory and 5s per call; process plugins configurable.
11. Plugin API coverage parity: plugins can do everything bundled plugins do, with no privileged internal APIs (REQ-NFR-004.5).
12. Workspace-recommended plugins are not activated in Restricted mode (REQ-FS-005.3).

#### Failure Modes
- WASM panic: trapped, plugin disabled with notification, IDE unaffected.
- Process plugin crash: detected on exit, auto-restarted once, disabled if recurring.
- Activation timeout (> 10s): abort, disable, notify.
- Incompatible API version: refuse activation with the version requirement stated.
- Missing dependency: refuse activation naming the needed dependency.

---

### Requirement 63: Plugin Marketplace

**ID:** REQ-PLUG-002 | **Category:** Plugins | **Tier:** 4

**User Story:** As a developer, I want to find and install verified plugins from inside the IDE, and as an enterprise administrator I want to serve an approved internal catalogue instead.

The system SHALL support plugin distribution and discovery.

#### Acceptance Criteria
1. Public marketplace with browse by category, featured listings, and full-text search.
2. Install, update, and uninstall flows.
3. Ratings and written reviews, both readable and submittable from the IDE.
4. Private and enterprise registries via configurable registry URL with authentication.
5. Plugin signing (Ed25519) with verification on install and update.
6. Automatic updates, configurable as auto, notify, or manual.
7. Version compatibility checks against the declared minimum Helix version.
8. Offline installation from a local `.helix-plugin` bundle.
9. Size limits: WASM under 10MB, process plugins under 100MB, with a warning above.
10. Metrics displayed: install count, rating, last updated, compatibility status.
11. Changelogs viewable before updating.
12. Rollback to the previous plugin version.

#### Failure Modes
- Marketplace unreachable: show cached listings; local bundle install still works.
- Signature verification failure: refuse install with a security warning.
- Download corruption: verify hash, retry up to 3 times.
- Registry auth failure: prompt to re-authenticate, fall back to the public marketplace.

---

### Requirement 64: Bundled First-Party Plugins

**ID:** REQ-PLUG-003 | **Category:** Plugins | **Tier:** 1 (MVP)

**User Story:** As a developer installing Helix for the first time, I want my everyday languages to work immediately, so I do not have to assemble a working setup before writing code.

The system SHALL ship with a curated first-party plugin bundle providing baseline language support.

#### Acceptance Criteria
1. TypeScript/JavaScript: syntax, LSP integration, debugging, test runner.
2. HTML, CSS, SCSS, Less.
3. JSON, YAML, TOML with schema validation.
4. Markdown with live-reload preview and editing assistance.
5. Rust via rust-analyzer.
6. Python via pylsp or pyright.
7. Go via gopls.
8. Java via Eclipse JDT.
9. C/C++ via clangd.
10. Docker and Dockerfile.
11. Shell scripts (bash, zsh, fish).
12. Git integration built on the core Git service.
13. All bundled plugins are consumers of the same plugin API as third-party plugins once REQ-PLUG-001 lands; migration off internal APIs is a tracked deliverable.
14. Bundled plugins can be disabled but not uninstalled.

---

### Requirement 65: Plugin Development Kit

**ID:** REQ-PLUG-004 | **Category:** Plugins | **Tier:** 4

The system SHALL provide tooling for third-party plugin development.

**User Story:** As a plugin author, I want scaffolding, types, and a test harness, so I can build and publish a Helix plugin without reverse-engineering the platform.

**Rationale:** REQ-NFR-004 asserts a documented, stable, testable plugin API. The artifacts that make that real (SDK, CLI, templates, harness) previously had no requirement or task.

#### Acceptance Criteria
1. SDK libraries with full type definitions for the plugin API, published for both WASM and process plugin targets.
2. CLI scaffolding tool that generates a working plugin from templates: language support, theme, icon theme, formatter, tree view, AI tool.
3. Local development loop: build, install into a development instance, hot-reload, and inspect logs without publishing.
4. Integration test harness that runs a plugin against a real kernel with a temporary workspace and asserts on IDE state.
5. API reference documentation generated from the type definitions, versioned alongside the API.
6. Tutorials and worked examples for each template category.
7. Packaging command producing a signed `.helix-plugin` bundle.
8. Publishing command targeting the public marketplace or a configured private registry.
9. API version compatibility linting: warn when a plugin uses APIs newer than its declared minimum version.
10. Migration guides published for every breaking API change, per the deprecation policy (REQ-NFR-004.2).

---

### Requirement 66: Plugin Sandbox

**ID:** REQ-SEC-001 | **Category:** Security | **Tier:** 4

**User Story:** As a developer installing a plugin, I want to see and later revoke exactly what it is permitted to do, so trust is a decision I make rather than one made for me.

The system SHALL enforce plugin isolation at runtime.

#### Acceptance Criteria
1. WASM plugins cannot access filesystem, network, or OS APIs without an explicit capability grant.
2. Process-isolated plugins run with minimal OS permissions (restricted token or sandbox profile).
3. Capability declarations are shown to the user at install time as a permission list.
4. Permissions are revocable after install; the plugin degrades gracefully on denial.
5. Capability audit log records every capability use and denial per plugin, viewable in developer tools.
6. Plugin-contributed SVG assets are sanitized before entering the DOM (REQ-ICON-001.16).

---

### Requirement 67: Secret Management

**ID:** REQ-SEC-002 | **Category:** Security | **Tier:** 1 (MVP)

**User Story:** As a developer, I want my API keys and git credentials held by the operating system's credential store and never written to a config file or a log, so I cannot leak them by sharing my settings.

The system SHALL store credentials in the OS secure credential store.

#### Acceptance Criteria
1. OS keychain integration: Windows Credential Manager, macOS Keychain, Linux Secret Service/libsecret.
2. Service API: store, get, delete, list.
3. Plugin access is namespaced; plugins cannot read secrets outside their namespace.
4. No secrets in configuration files; a secret detected in settings is rejected with a warning.
5. Secret redaction in all log output, terminal captures, telemetry, and diagnostic reports.
6. API key rotation: updating a key in place causes all consumers to pick up the new value.
7. Secrets are accessible only to the kernel service, never exposed to the frontend or plugins without an explicit grant.
8. Git credential integration through the credential helper interface.

#### Failure Modes
- Keychain locked or unavailable: prompt to unlock, or fall back to an encrypted file protected by a master password.
- Secret not found: clear error with guidance to configure the credential.
- Keychain access denied: explain the required system permission.

---

### Requirement 68: Agent Security

**ID:** REQ-SEC-003 | **Category:** Security | **Tier:** 3

**User Story:** As a security-conscious developer, I want the agent's every action logged and its instructions insulated from content it reads, so untrusted text in a file cannot redirect it.

The system SHALL enforce security controls on agent execution.

#### Acceptance Criteria
1. Filesystem scope enforcement for every agent path (REQ-AI-042.5).
2. Network egress control against the configured whitelist (REQ-AI-042.3).
3. Prompt injection defense: system prompts stored separately from user context and marked immutable; content from files, terminals, and MCP tools treated as untrusted data with role and delimiter enforcement; agent actions validated against the approved plan with unexpected actions flagged.
4. Append-only audit log of all agent actions, recording file reads and writes, commands executed, API calls, and tokens consumed.
5. Audit log queryable from the command palette, with configurable retention (default 30 days).
6. Token and cost budget enforcement as hard limits (REQ-AI-002.6).
7. Agent rate limiting at 10 actions per second.
8. Three sandbox violations auto-pause the task for user review.

---

### Requirement 69: Supply Chain Security

**ID:** REQ-SEC-004 | **Category:** Security | **Tier:** 4

**User Story:** As an enterprise security reviewer, I want a bill of materials, signed artifacts, and reproducible builds, so I can approve Helix for use on a managed fleet.

The system SHALL protect the integrity of its own build and its plugin ecosystem.

#### Acceptance Criteria
1. Signed plugin packages (Ed25519) with publisher keys registered with the marketplace.
2. Plugin integrity verification on install and on every update (hash plus signature).
3. SBOM generated for every Helix release in SPDX format and published with the artifacts.
4. Dependency vulnerability scanning integrated into CI, checking advisory databases for both Rust and npm dependency trees, failing the build on unpatched critical advisories.
5. Reproducible builds: building from source with the same inputs produces an identical binary, verified by a CI job that builds twice and compares.
6. Plugin dependency audit flagging plugins with known-vulnerable dependencies, surfaced in the marketplace listing and to installed users.
7. Provenance attestation at SLSA Level 2 or higher (REQ-DIST-001.7).
8. Dependency pinning: exact versions in lockfiles, with a documented review process for updates.

---

### Requirement 70: Cross-Platform Distribution

**ID:** REQ-DIST-001 | **Category:** Distribution | **Tier:** 2

**User Story:** As a developer, I want to install Helix the normal way for my platform and have the OS trust it, so installation is not a security warning I have to click past.

The system SHALL be distributed as signed native packages for all supported platforms.

#### Acceptance Criteria
1. Windows: MSI installer, NSIS installer, and portable zip requiring no administrator rights.
2. macOS: DMG with drag-to-Applications, Homebrew cask, universal binary (ARM and Intel).
3. Linux: AppImage, `.deb`, `.rpm`, Flatpak, and Snap.
4. Auto-updater with staged rollouts (1%, 10%, 50%, 100% over 72 hours).
5. Signed binaries: Authenticode (EV certificate) on Windows, Developer ID plus notarization on macOS, GPG signatures on Linux packages.
6. SHA-256 checksums published alongside every artifact.
7. Provenance attestation at SLSA Level 2 or higher.
8. Installer size under 100MB compressed.
9. Each installer is verified in CI by an install, launch, and uninstall cycle on its target platform.

---

### Requirement 71: Update System

**ID:** REQ-DIST-002 | **Category:** Distribution | **Tier:** 2

**User Story:** As a developer, I want updates to arrive quietly and apply when I choose, with a way back if one goes wrong, so upgrading is never a risk to my working day.

The system SHALL update itself safely and reversibly.

#### Acceptance Criteria
1. Update check on startup, configurable as auto-download, notify-only, or disabled.
2. Background download that is non-blocking and resumes after network interruption.
3. Updates apply on restart; the user chooses when, and restart is never forced.
4. Rollback to the previous version, retained on disk, invokable from the command palette or `helix --rollback` (REQ-CLI-001.4).
5. Release channels: stable (monthly, with critical patches), beta (bi-weekly), nightly (daily).
6. Channel switching without reinstall.
7. Offline and air-gapped update from a downloaded bundle.
8. Delta updates where possible to reduce download size.
9. Update integrity verified by hash before applying.

#### Failure Modes
- Download interrupted: resume on the next check.
- Corrupted update: discard and re-download.
- Update breaks the IDE: rollback available from the command line.
- Update server unreachable: skip silently, retry next startup, no error spam.

---

### Requirement 72: Performance

**ID:** REQ-NFR-001 | **Category:** Non-Functional | **Tier:** 1 (MVP) — performance is not optional

**User Story:** As a developer, I want the editor to feel instant on ordinary hardware and in a huge repository, so the tool never becomes the reason I lose my train of thought.

The system SHALL meet defined performance budgets on reference hardware (Appendix B).

#### Acceptance Criteria
1. Startup: launch to usable editor under 3s.
2. File open: open-to-rendered under 200ms for files below 1MB.
3. Typing latency: keyboard-to-screen under 16ms.
4. Workspace scale: 500,000+ files without UI degradation across tree view, search, and file watching.
5. Memory baseline: under 300MB for an empty workspace with no plugins; under 500MB for a typical 10k-file workspace with no language servers.
6. Memory growth under 50MB/hour during active use, regression-tested in CI.
7. Configurable kernel memory ceiling (default 2GB) with a warning at 80%.
8. IPC round-trip under 5ms (p95) for simple commands.
9. Opening a 500k-file workspace reaches a usable state within 10s, with deferred indexing.
10. Editor and tree views hold 60fps during fast scroll.
11. Search returns first results within 200ms for repositories under 50k files.
12. Automated CI benchmarks cover startup, file open, typing latency, search, and memory baseline.
13. CI fails when any tracked metric regresses more than 10% from the recorded baseline.
14. The reference benchmark workspace is a 50k-file monorepo generated by a checked-in script.

---

### Requirement 73: Reliability

**ID:** REQ-NFR-002 | **Category:** Non-Functional | **Tier:** 1 (MVP)

**User Story:** As a developer, I want a bounded and stated worst case for losing unsaved work when the application dies, so I can trust the editor with hours of thinking and know exactly what that trust costs.

The system SHALL preserve user work within a defined Recovery Point Objective under all failure conditions.

**Recovery Point Objective (RPO).** The maximum loss of unsaved editor content after abrupt termination is one WAL flush interval, configurable as `files.walIntervalMs`, default 1000ms. A literal zero-keystroke guarantee is deliberately not claimed: it would require a synchronous durable write per edit, which is incompatible with the typing latency budget in REQ-NFR-001.3. Content already written to a file is a separate and stronger guarantee, covered by atomic writes in criterion 1.

#### Acceptance Criteria
1. Unsaved buffer content is persisted to the WAL and restored on restart; saved files are written atomically, so no file is ever observed partially written regardless of how the process dies.
2. Kernel crash recovery within 2s via the supervisor (REQ-ARCH-005.3).
3. Frontend crash recovery by webview restart with state re-push (REQ-ARCH-004.8).
4. Session restore after a crash reopens editors, terminals, and panels to their pre-crash state.
5. Graceful degradation: a single service failure does not cascade (an LSP crash does not affect the terminal).
6. Watchdog: kernel heartbeat every 5s; the frontend shows recovery UI after 15s of silence.
7. Durability is stated per failure class, and each class is asserted separately in test:

   | Failure class | Guarantee |
   |---|---|
   | Graceful shutdown (quit handshake completes) | No loss. All buffers flushed before exit. |
   | Kernel panic or webview crash after a completed WAL flush | No loss of flushed content. |
   | Hard kill (SIGKILL, OS crash, power loss) | Loss bounded by the RPO above. |
   | Any class, for content already saved to a file | No loss and no partial write. |

8. Recovery paths are covered by automated crash-simulation tests that assert the RPO for each failure class above, not manual verification alone.
9. The measured RPO is recorded in the MVP gate report (REQ-NFR-001.12) as a number, not asserted.
10. WAL, snapshots, and crash reports are stored in the OS state directory keyed by workspace, never inside the workspace. `.helix/` holds shareable workspace configuration only. Consequences that are part of the requirement: recovery works in a read-only or permission-restricted checkout, Helix creates no version-control noise, session and terminal history is never committable, and a multi-root workspace has exactly one state location.
11. The workspace key is stable across symlinked paths and root reordering, and is derived from the `id` in `.helix/workspace.json` when present, otherwise from a hash over the sorted set of canonicalized root paths.

---

### Requirement 74: Offline Capability

**ID:** REQ-NFR-003 | **Category:** Non-Functional | **Tier:** 1 (MVP) — standing obligation, re-verified in every tier

**User Story:** As a developer working on a plane or behind an air-gapped network, I want every part of the IDE that exists, except cloud AI, to work exactly as usual, so connectivity is not a prerequisite for my job.

The system SHALL operate without internet access for every capability it has implemented.

**Scope rule.** This requirement is Tier 1, but the capabilities it governs arrive across several tiers. It is therefore a standing obligation rather than a one-time gate: each capability SHALL be offline-verified by the task that implements it, in the phase that implements it. No capability is exempt, and none is verified before it exists. Criteria below name the owning requirement where the capability is not Tier 1.

#### Acceptance Criteria
1. All implemented Tier-1 functionality operates offline: editing, terminal, local git, tasks, workspace search, and indexing.
2. AI degrades predictably: full features with a configured local model; cleanly disabled with an offline indicator and no error spam otherwise.
3. Plugin installation from a local bundle requires no network. Verified with the plugin runtime (REQ-PLUG-001, Tier 4). No Tier-1 capability depends on plugin installation.
4. No telemetry or phone-home is required for operation.
5. Search indexes and caches operate entirely offline.
6. Welcome and What's New content is bundled, not fetched (REQ-WB-004.8). Verified with the Welcome experience (Tier 2).
7. Update checks fail silently offline (REQ-DIST-002.9). Verified with the updater (Tier 2).
8. Offline behaviour is verified by an automated suite that runs the application with network access denied. The suite is built in Tier 1 covering criteria 1, 2, 4, and 5, and each later capability extends it rather than being verified by hand.
9. Debugging and test execution operate offline. Verified with the DAP host and test explorer (REQ-DEBUG-001, REQ-TEST-001, Tier 2).

---

### Requirement 75: API Stability

**ID:** REQ-NFR-004 | **Category:** Non-Functional | **Tier:** 4 (API discipline applies from Tier 1)

**User Story:** As a plugin author, I want the API I build against to keep working across releases and to be warned well before anything is removed, so maintaining my plugin is not a treadmill.

The plugin API SHALL be stable and versioned.

#### Acceptance Criteria
1. Semantic versioning, with breaking changes only on a major version.
2. Deprecation policy: a minimum of 2 minor versions between deprecation and removal.
3. Extension development documentation: API reference, tutorials, examples (delivered via REQ-PLUG-004).
4. Plugin development kit: SDK, CLI, templates, and test harness (REQ-PLUG-004).
5. No privileged internal APIs: plugins can do everything bundled plugins can do.
6. API surface changes are gated by a CI check that detects unintended breaking changes.

---

### Requirement 76: Accessibility

**ID:** REQ-NFR-005 | **Category:** Non-Functional | **Tier:** 1 (MVP) — accessibility is not optional

**User Story:** As a developer who uses a screen reader and works entirely from the keyboard, I want every part of the IDE reachable and announced, so I can do my job with the same tool as my colleagues.

The system SHALL be usable without sight, without a mouse, and with assistive technology.

**Scope note:** Helix is a desktop application. Pointer targets follow desktop conventions; there is no touch or mobile target for v1.0.

#### Acceptance Criteria
1. WCAG 2.1 AA as the compliance target. Full validation requires manual testing with assistive technologies and expert review; automated checks are necessary but not sufficient.
2. Screen reader support: ARIA landmarks on major regions, ARIA live regions for dynamic content (notifications, diagnostics counts, build status), meaningful labels on all interactive elements.
3. Full keyboard navigation with no mouse-only operations: Tab cycles regions, arrows navigate within regions, Enter/Space activates, Escape closes and restores focus.
4. Visible focus indicators on every interactive element, at least 2px and meeting contrast requirements.
5. Logical tab order and skip-to-content landmarks.
6. Focus management: focus trapped in modals and restored on close.
7. High contrast themes, at least one dark and one light, meeting 7:1 for body text and 4.5:1 for large text.
8. Non-text contrast of at least 3:1 for icons, focus indicators, and control boundaries.
9. Configurable font sizes and UI zoom from 50% to 200% without loss of function or clipping.
10. Reduced motion honoured from the OS setting, disabling all non-essential animation.
11. Color is never the sole indicator of state; always paired with icon, text, shape, or position.
12. Pointer targets are at least 24x24px with adequate spacing, per WCAG 2.2 target-size guidance for pointer inputs.
13. Screen reader verification on NVDA (Windows), VoiceOver (macOS), and Orca (Linux).
14. Automated accessibility checks run in CI on every component.

---

### Requirement 77: Theme Architecture

**ID:** REQ-THEME-001 | **Category:** Theming | **Tier:** 1 (MVP)

**User Story:** As a developer who stares at this screen all day, I want to control how it looks, including porting the theme I already use, so the IDE is comfortable for long sessions.

The system SHALL provide a comprehensive theming system.

#### Acceptance Criteria
1. Design token system: colors, spacing, typography, radii, and shadows all defined as tokens.
2. Token layers: component tokens reference semantic tokens, which reference palette tokens.
3. Built-in themes: Light, Dark, High Contrast Dark, High Contrast Light.
4. Theme switching under 100ms with no layout shift or flicker.
5. OS preference detection with user override.
6. Editor token colors: full TextMate and semantic token customization per theme.
7. UI component colors: all panels, buttons, inputs, borders, and scrollbars themed.
8. Icon themes (product and file) are separate axes, swappable independently (REQ-ICON-001, REQ-ICON-002).
9. Theme file format: JSON with schema validation, accepting VS Code color themes for straightforward porting.
10. User overrides can customize any token from settings without authoring a full theme.
11. Theme preview: hovering a theme in the selector previews it live without committing the choice.
12. Third-party themes distributed via the plugin system.
13. Theme hot-reload for authors: the theme file is watched and changes apply instantly.

#### Failure Modes
- Theme parse error: fall back to the default dark theme and show the parse error.
- Missing token: fall back to the nearest semantic parent, then to the default theme's token.
- OS high-contrast mode detected: auto-switch to a high-contrast theme.

---

### Requirement 78: Syntax Theme Colors

**ID:** REQ-THEME-002 | **Category:** Theming | **Tier:** 1 (MVP)

**User Story:** As a theme author, I want every meaningful state in code presentation to be a nameable color, so I can build a coherent theme without fighting hardcoded values.

The system SHALL define themeable colors for all code presentation.

#### Acceptance Criteria
1. TextMate scope mapping for Tree-sitter and regex tokenizers.
2. Semantic token type and modifier colors for LSP semantic tokens.
3. Bracket pair colors, configurable per nesting level, minimum 6 levels.
4. Diff colors: added, removed, and modified line backgrounds and gutters.
5. Git decoration colors: untracked, modified, staged, conflict, ignored.
6. Diagnostic colors for error, warning, info, and hint, covering both squiggle and background tint.
7. Search highlight colors distinguishing the current match from other matches.
8. Selection and highlight colors: editor selection, word highlight, bracket highlight.
9. Icon color tokens consumed by the icon system (REQ-ICON-001.11).

---

### Requirement 79: Product Icon System

**ID:** REQ-ICON-001 | **Category:** Theming | **Tier:** 1 (MVP)

The system SHALL provide a unified product icon system for all UI chrome.

**User Story:** As a developer, I want consistent, crisp, legible icons throughout the IDE so I can recognize actions and states at a glance, at any zoom level or DPI.

**Rationale:** Icons are the primary affordance in an IDE's chrome. Activity bar, tabs, gutters, trees, toolbars, status bar, and diagnostics all depend on them. Without a single owned system, icons get added ad hoc per component, producing inconsistent sizing, broken theming, missing accessibility labels, and no plugin extension path.

#### Acceptance Criteria
1. A single icon registry addressing every icon by stable ID (e.g. `helix.file`, `helix.debug.start`).
2. ID namespacing: `helix.*` reserved for first-party; plugins use `<publisher>.<plugin>.*`.
3. Delivery as SVG, monochrome by default, using `currentColor` so icons inherit theme foreground.
4. First-party icons compiled into a single sprite at build time; no per-icon fetch at runtime.
5. Component API: `<Icon id size label? spin? />`.
6. Size scale of three tokens (12, 16, 20px) that scales with UI zoom from 50% to 200% without blurring.
7. Crisp rendering at 1x, 1.5x, 2x, and 3x DPI, authored on a 16px pixel grid.
8. Coverage of a defined MVP set of approximately 150 icons: activity bar, editor tabs, tree, diagnostics, source control, debug, test, terminal, AI, and common actions.
9. Symbol icons covering the full LSP `SymbolKind` and `CompletionItemKind` sets, used by the completion popup, outline, breadcrumbs, and symbol search.
10. State variants (default, hover, active, disabled, selected) expressed via CSS, not duplicate assets.
11. Icon color resolves from semantic theme tokens (`icon.foreground`, `icon.disabled`, plus context tokens such as git and diagnostic colors); never hardcoded.
12. Animated icons (spinner, progress) fall back to static under `prefers-reduced-motion`.
13. Product icon themes: the entire chrome icon set is swappable, independently of color theme and file icon theme.
14. Directional icons (chevrons, arrows, indent) mirror automatically under RTL layout (REQ-WB-005.8).
15. Plugins contribute icons via `contributes.icons` and may reference built-in icon IDs in their own views.
16. Plugin-contributed SVG is parsed and sanitized (script, external references, and event handlers stripped) before entering the DOM.
17. Accessibility: decorative icons are `aria-hidden`; icon-only controls carry an accessible name and a tooltip; icons never carry state alone (REQ-NFR-005.11).
18. High-contrast themes meet 3:1 non-text contrast for every icon (REQ-NFR-005.8).
19. Performance: icons add no measurable cost to 60fps virtualized scrolling; the first-party sprite is under 150KB gzipped.
20. Per-plugin icon size budget of 8KB.

#### Failure Modes
- Unknown icon ID: render a visible placeholder glyph, never blank, and log a warning once per ID.
- Icon theme fails to load or is partially defined: fall back per icon to the built-in set, notify once.
- Malformed plugin SVG: reject at install or activation with a clear error; never inject unsanitized markup.
- Plugin icon over budget: reject with a clear error naming the offending asset.

---

### Requirement 80: File Icon Themes

**ID:** REQ-ICON-002 | **Category:** Theming | **Tier:** 1 (MVP)

**User Story:** As a developer scanning a file tree, I want file types identifiable at a glance in whichever icon style I prefer, so I can find what I am looking for by shape rather than by reading every name.

The system SHALL provide file and folder icon themes.

#### Acceptance Criteria
1. Resolution order, first match wins: exact filename, compound extension (`.spec.ts`, `.d.ts`), simple extension, detected language ID, generic file icon.
2. Folder icons with closed and open variants, plus named-folder icons (`src`, `test`, `node_modules`, `.git`, `dist`).
3. Special node icons: workspace root, symlink, git submodule, unavailable root.
4. Built-in file icon themes: a colored default set, a monochrome minimal set, and None.
5. Coverage for at least the top 40 languages and formats plus every bundled first-party language (REQ-PLUG-003).
6. All consumers resolve through one service: explorer, editor tabs, quick open, search results, diff titles, breadcrumbs. No per-component mapping tables.
7. Theme file format: JSON with schema validation, using a VS Code-compatible mapping shape.
8. Third-party file icon themes installable via the plugin system.
9. Icon theme selection is independent of color theme and persisted in settings (`workbench.iconTheme`, `workbench.productIconTheme`).
10. Hot-reload for icon theme authors.
11. Lookup is O(1) amortized against a precomputed map, resolved during tree row render with no additional IPC round-trip.

#### Failure Modes
- No mapping matches: generic file icon, never blank.
- Referenced asset missing: generic fallback for that entry, warning logged once.
- Very large theme (over 2000 mappings): accepted, with a memory warning in the developer log.

---

### Requirement 81: Structured Logging

**ID:** REQ-OBS-001 | **Category:** Observability | **Tier:** 1 (MVP)

**User Story:** As a developer reporting a problem, I want to see and export what the IDE was doing, so a bug report can contain evidence rather than a description from memory.

The system SHALL provide structured logging for debugging and diagnostics.

#### Acceptance Criteria
1. Structured format: JSON lines with timestamp, level, source, message, and structured fields.
2. Levels trace, debug, info, warn, error, configurable per service and module.
3. Kernel and frontend logs unified into a single viewable stream.
4. Log viewer panel with filtering by level, source, and time range, plus full-text search and follow-tail.
5. Log viewer supports copying entries, exporting the filtered set, and jumping from a service in the health dashboard to its logs (REQ-OBS-004.5).
6. Log rotation: 50MB per file, 5 rotated files, configurable.
7. Output targets: file always, developer panel when open, stdout when launched from the CLI.
8. Zero cost on performance-critical paths when the level is disabled.
9. Correlation IDs shared between an IPC command and its kernel-side processing.
10. No PII by default: file paths are acceptable, file contents are not.
11. Secrets are redacted from all log output (REQ-SEC-002.5).

---

### Requirement 82: Crash Reporting

**ID:** REQ-OBS-002 | **Category:** Observability | **Tier:** 2

The system SHALL capture and optionally report crashes.

**User Story:** As a maintainer, I want actionable crash data from consenting users, so defects that only appear in the field can be diagnosed and fixed.

#### Acceptance Criteria
1. Opt-in with explicit consent on first run, changeable in settings at any time; nothing is transmitted before consent.
2. Report contents: stack trace, OS and hardware information, Helix version, active plugins, and the last 20 log lines. No file contents, no PII.
3. Kernel panics captured via a Rust panic hook, producing a minidump.
4. Frontend crashes captured via global error handlers and React error boundaries.
5. Crash cause captured by the supervisor before restart is included (REQ-ARCH-005.8).
6. Local crash dump storage in the OS state directory keyed by workspace, never inside the workspace, listable and viewable by the user before anything is sent.
7. Configurable destination, defaulting to the Helix endpoint, with an enterprise-configurable internal endpoint.
8. Crash-free session rate tracked as a local metric.
9. Previous-session crash detection on startup, offering to send the report.
10. Reports are queued when offline and sent later, or discarded on user request.
11. Secrets and tokens are redacted from all report content before storage or transmission.

#### Failure Modes
- Report upload fails: retain locally and retry later; never block startup.
- Minidump write fails: still record the textual stack trace and log tail.

---

### Requirement 83: Performance Telemetry

**ID:** REQ-OBS-003 | **Category:** Observability | **Tier:** 2

**User Story:** As a developer who suspects the IDE is slow on my machine, I want to measure it myself and capture a profile, so I can report something specific instead of "it feels sluggish".

The system SHALL measure its own performance and expose the data locally.

#### Acceptance Criteria
1. Opt-in transmission using the same consent gate as crash reporting; local collection requires no consent because it never leaves the machine.
2. Metrics collected: startup time, file open time, completion latency, build time, memory peaks, IPC latency distribution.
3. Local performance dashboard (Developer: Show Performance), always available regardless of the transmission opt-in.
4. Performance marks for key lifecycle events: app start, kernel ready, first paint, editor ready, LSP ready.
5. Latency distributions recorded as histograms, reported as p50, p95, and p99 rather than averages.
6. On-demand CPU profiling exporting a standard profile format for external analysis.
7. On-demand heap snapshot for memory analysis.
8. Metrics exportable as JSON for local analysis.
9. Rolling in-memory window with periodic aggregates persisted to disk.
10. The dashboard surfaces the same metrics the CI benchmarks gate on (REQ-NFR-001.12), so field data and CI data are directly comparable.

---

### Requirement 84: Health Monitoring

**ID:** REQ-OBS-004 | **Category:** Observability | **Tier:** 1 (MVP)

**User Story:** As a developer whose IDE has become slow, I want to see which component is misbehaving and fix it in one click, so I can resolve the problem instead of restarting the application.

The system SHALL expose the health of all internal services.

#### Acceptance Criteria
1. Service health dashboard covering: kernel process (memory, CPU, uptime), each language server (status, memory, restart count, last error), WebSocket (connection status, message rate, backpressure events), file watcher (watched path count, event rate, errors), AI providers (status, latency, token usage), and plugins (status, memory).
2. Status bar health indicator: healthy, degraded, or critical, opening the dashboard on click.
3. Automatic degradation detection with actionable remediation offered (restart a server, reduce watched paths).
4. Health state changes pushed to the frontend rather than polled.
5. Clicking a service filters the log viewer to that source (REQ-OBS-001.5).
6. Every kernel service implements the health contract (REQ-ARCH-002.6); a service that cannot report health is itself a defect.

---

### Requirement 85: Embedded Web Preview

**ID:** REQ-PREVIEW-001 | **Category:** Preview | **Tier:** 2

The system SHALL provide an optional web preview panel.

**User Story:** As a frontend developer, I want to preview my application side-by-side with my code without switching to an external browser.

#### Acceptance Criteria
1. Dev-server detection by scanning localhost ports for common dev servers.
2. Embedded localhost preview in a webview separate from the main window webview.
3. Auto-reload on file change via HMR detection, with a manual refresh fallback.
4. Open-externally action launching the system browser at the same URL.
5. Responsive mode with preset device widths.
6. Manual URL configuration when auto-detection fails.
7. Multiple preview tabs for multiple running servers.
8. Panel is resizable and dockable like any other tool panel.
9. Helpful empty state when no dev server is running.

#### Failure Modes
- Dev server not running: empty state with guidance, not an error.
- Port conflict: report which process holds the port.
- Preview webview crash: error state with a reload button; the main IDE is unaffected.

---

### Requirement 86: Remote Development

**ID:** REQ-REMOTE-001 | **Category:** Remote | **Tier:** Future (post-v1.0)

**User Story:** As a developer whose code lives on a remote host, container, or WSL distribution, I want to edit it with the same IDE experience as local code, so where the code runs does not dictate which tools I can use.

**Scope note:** This requirement is a deliberate placeholder. It is recorded because the kernel/frontend split (REQ-ARCH-001) exists partly to make remote operation possible without re-architecture, and that constraint must remain visible to anyone changing the communication layer. Detailed criteria will be written when the feature enters active planning; no task implements it in this plan.

#### Acceptance Criteria
1. Detailed acceptance criteria are deferred until this requirement enters active planning, and this requirement is explicitly excluded from the v1.0 scope and from the traceability matrix's coverage obligation.
2. Until then, the architecture SHALL not introduce assumptions that prevent relocating the kernel to another host: no shared-memory coupling between frontend and kernel, no reliance on local filesystem paths in IPC contracts, and no assumption of sub-millisecond transport latency.
3. Any change to the communication layer (REQ-ARCH-003) SHALL be reviewed against criterion 2.

**Anticipated Capabilities:**
- SSH remote: kernel on the remote machine, frontend connected over a tunnel.
- Container remote: kernel inside a Docker or Podman container.
- WSL remote: kernel in WSL, frontend on the Windows host.
- Cloud development environments: connection to pre-provisioned remote workspaces.
- Latency compensation: predictive typing, local echo, offline queue.

---

## Appendix A: Requirement Coverage

Every requirement in this document is implemented by at least one task in `tasks.md`, and every task in `tasks.md` cites at least one requirement. The mapping is maintained in the Traceability Matrix in `design.md`. A requirement with no task, or a task with no requirement, is a spec defect.

## Appendix B: Reference Hardware

Performance targets (REQ-NFR-001) are measured against this configuration:

- CPU: 4-core / 8-thread, 2020-era (Intel i5-10400, AMD Ryzen 5 3600, or Apple M1)
- RAM: 16GB
- Storage: NVMe SSD (sequential read above 2GB/s)
- OS: Windows 11, macOS 13, Ubuntu 22.04
- Display: 1080p at 60Hz. No GPU requirement for the IDE; a GPU is optional and only relevant to local LLM inference.

## Appendix C: Deferred Scope

Recorded so that absence is a decision rather than an oversight.

| Item | Status | Rationale |
|------|--------|-----------|
| Cloud settings sync | Deferred post-v1.0 | Requires account infrastructure; local settings are complete without it |
| Real-time collaborative editing | Deferred post-v1.0 | CRDT editing is a large independent workstream |
| Notebook editing (LSP notebook documents) | Deferred post-v1.0 | Distinct editor surface, not required by the target user |
| Mobile or tablet client | Out of scope | Helix is a desktop application (REQ-NFR-005 scope note) |
| Remote development | Future (REQ-REMOTE-001) | Architecture permits it; requirements not yet specified |
| Touch input optimization | Out of scope | Desktop pointer and keyboard are the supported inputs |
