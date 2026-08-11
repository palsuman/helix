# Helix Platform — Design Document

## Overview

Helix is a desktop-first, cross-platform IDE built on Tauri 2, React, TypeScript 7, Monaco Editor, and an authoritative Rust kernel. It targets polyglot enterprise developers and treats AI as an autonomous peer with configurable trust levels.

This design document specifies the architecture, component interfaces, data models, error handling, and testing strategy required to build a production-grade IDE that competes with VS Code while offering first-class AI agent integration.

**Key differentiators from VS Code:**
- Rust kernel owns all state (crash-safe, faster, memory-efficient)
- AI agent with OS-level sandboxing and configurable trust
- Native performance for all core operations (no Electron overhead)
- Plugin sandboxing via WASM (memory-safe by default)

**Cross-references.** Requirements are cited by stable ID (`REQ-ARCH-001`) rather than by number, so references remain valid as `requirements.md` evolves. Tasks are cited by phase number (`Task 4.5`) from `tasks.md`. Coverage in both directions is tabulated in the Requirements Traceability Matrix at the end of this document.

**Technology Stack:**

The Supported line column records the architectural compatibility commitment. The Chosen version column records the exact Task 1.1 selection; `Cargo.lock` and the npm lockfiles remain authoritative for transitive resolution. Components assigned to later tasks remain unselected until their implementation lands. `latest` is not a version and appears nowhere in this table or in a manifest.

| Layer | Technology | Supported line | Chosen version | Purpose |
|-------|-----------|----------------|----------------|---------|
| Desktop framework | Tauri | 2.x | Core 2.11.5, CLI 2.11.4 | Window management, IPC, native APIs, bundling |
| Kernel language | Rust | 2024 edition, pinned stable toolchain | 1.97.1 | Performance, safety, concurrency |
| Build runtime | Node.js | 24.x | 24.0.2 | Frontend builds and repository checks |
| Frontend framework | React | 19.x | 19.2.8 | UI rendering |
| State management | Zustand | 5.x | 5.0.14 | Lightweight frontend state (UI-only) |
| Code editor | Monaco Editor | 0.x, exact pin required | Not selected (Task 4.1) | Text editing, syntax, completions |
| Terminal | xterm.js | 5.x | Not selected (Task 6.1) | Terminal rendering in webview |
| TypeScript compiler | tsc (native) | 7.x | 7.0.2 | Native Go-based compiler (8-12x faster) |
| Bundler | Vite | 8.x | 8.2.1 | Frontend build tooling |
| Frontend quality | ESLint + Prettier | 10.x / 3.x | 10.8.0 / 3.9.6 | Static analysis and deterministic formatting |
| Styling | CSS Modules + CSS Variables | — | Built in | Scoped styles, theming via custom properties |
| Type generation | ts-rs | 12.x | 12.0.1 | Rust types → TypeScript interfaces |
| File watching | notify | 6.x | 6.1.1 | Cross-platform filesystem events |
| Git | gitoxide (gix) | 0.x, exact pin required | Not selected (Task 7.1) | High-performance git operations |
| Search | ripgrep (`grep` crate) | 0.x, exact pin required | Not selected (Task 4.5) | Fast text search |
| Tree-sitter | tree-sitter (WASM) | 0.x, exact pin required | Not selected (Task 5.7) | Syntax parsing in frontend |
| WASM runtime | wasmtime | Single security-supported major | Not selected (Task 17.1) | Plugin sandbox |
| Serialization | serde + JSON | 1.x | serde 1.0.228, serde_json 1.0.145 | IPC, config, state persistence |
| Async runtime | tokio | 1.x | 1.53.1 | Async I/O in kernel |
| HTTP client | reqwest | 0.x, exact pin required | Not selected (Task 8.1) | LLM API calls, marketplace |
| Crypto | ring / ed25519-dalek | Single security-supported major | Not selected (Task 15.3) | Plugin signing, checksums |
| Credential store | keyring / platform APIs | Single major per platform crate | Not selected (Task 1.12) | OS keychain access per platform |
| Localization | ICU MessageFormat (fluent or icu4x) | One selected major | Not selected (Task 2.9) | Message catalogs, pluralization, formatting |
| Text segmentation | unicode-segmentation | 1.x | Not selected (Task 4.2) | Grapheme-cluster cursor movement |
| Token counting | tiktoken-rs | 0.x, exact pin required | Not selected (Task 8.1) | Budget accounting for OpenAI-family models |
| Metrics | hdrhistogram | 7.x | Not selected (Task 9.7) | Latency percentiles for telemetry and gates |
| Crash capture | minidump-writer | 0.x, exact pin required | Not selected (Task 13.1) | Kernel panic minidumps |
| Testing (Rust) | cargo test + criterion | criterion 0.5.x or newer | cargo 1.97.1; criterion not selected | Unit, integration, benchmarks |
| Testing (TS) | Vitest | 4.x | 4.1.10 | Frontend tests |
| E2E testing (packaged app) | WebdriverIO + `@wdio/tauri-service` | 9.x | Not selected (Task 3.5) | Real binary, all three platforms |
| E2E testing (renderer only) | WebdriverIO browser mode, or Playwright | Current selected major | Not selected (Task 3.5) | Fast frontend-only scenarios against Vite |

**Dependency pinning policy.**

Rows marked "exact pin required" are pre-1.0 crates and packages where a minor bump is a breaking change by semver convention. Pinning them exactly is not caution, it is the only correct reading of `0.x`.

1. Every external dependency is declared with an exact version: `x.y.z` in npm manifests and `=x.y.z` in Cargo manifests. No ranges or floating tags are allowed.
2. Lockfiles are committed for both trees and are the single source of truth for what actually builds.
3. A CI check fails the build on any floating specifier, so the policy cannot decay quietly.
4. Version bumps are a reviewed change with a stated reason, batched on a schedule rather than applied ad hoc, and gated by the existing benchmark and vulnerability-scanning jobs.
5. Security advisories override the schedule, which is why `wasmtime` and the crypto crates are recorded as tracking security releases: they are the two places where being behind is worse than being unstable.

Task 15.3 already requires exact pinning with a documented update review process for release engineering. This policy is the same rule applied from the first commit rather than from the first release.

---

## Architecture

Helix follows a strict frontend/host/kernel split. The React frontend is a pure rendering layer. The Helix Host is the thin Tauri Core process: it owns windows and Tauri capabilities, terminates WebView invokes, forwards typed commands, and supervises the kernel. The separate Rust kernel owns all IDE state and business logic. This matches Tauri's process model without moving domain authority out of the kernel.

```
┌─────────────────────────────────────────────────────────────────────┐
│       Helix Host / Supervisor (Tauri Core, one per application)      │
│  windows + capabilities │ invoke gateway │ kernel monitor/recovery  │
│  No IDE business logic, plugins, provider calls, or domain state.    │
└──────────────┬─────────────────────────────┬────────────────────────┘
               │ owns WebView(s)             │ typed authenticated RPC
               ▼                             │ spawns + monitors
┌────────────────────────────────────────────┼────────────────────────┐
│              Frontend (React + TypeScript) │ one per window         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────────────┐  │
│  │Workbench │  │  Monaco  │  │  Panels  │  │  State (Zustand)  │  │
│  │  Shell   │  │  Editor  │  │  (Tools) │  │  (UI-only cache)  │  │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────────┬──────────┘  │
│       │              │              │                  │             │
│  ─────┴──────────────┴──────────────┴──────────────────┴─────────── │
│  │ Tauri invoke client │ authenticated WS client (streams)       │ │
│  └──────────┬──────────┘  └──────────────────┬───────────────────┘ │
└─────────────┼────────────────────────────────┼─────────────────────┘
              │ terminates at Host             │ direct loopback only;
              └──────────────► Host ───────────┐│ endpoint/token brokered
                                               ▼▼
┌─────────────────────────────────────────────────────────────────────┐
│                  Kernel (separate Rust process)                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │                    Service Container (DI)                      │  │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌────────┐ │  │
│  │  │   FS    │ │Workspace│ │   LSP   │ │   DAP   │ │Terminal│ │  │
│  │  │ Service │ │ Manager │ │  Host   │ │  Host   │ │Manager │ │  │
│  │  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └────────┘ │  │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌────────┐ │  │
│  │  │   Git   │ │   AI    │ │ Plugin  │ │ Search  │ │ Config │ │  │
│  │  │ Service │ │ Engine  │ │ Runtime │ │ /Index  │ │Service │ │  │
│  │  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └────────┘ │  │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌────────┐ │  │
│  │  │  Audit  │ │  Theme  │ │  State  │ │ Secrets │ │ Trust  │ │  │
│  │  │ Service │ │ /Icons  │ │Persister│ │ (keychain)│ Service│ │  │
│  │  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └────────┘ │  │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐            │  │
│  │  │ Command │ │ Context │ │ Project │ │  Agent  │            │  │
│  │  │Registry │ │ Engine  │ │ Genesis │ │ Runtime │            │  │
│  │  └─────────┘ └─────────┘ └─────────┘ └─────────┘            │  │
│  │                                                              │  │
│  │  Scope: ■ global  ▲ workspace (refcounted)  ● window         │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                      │
│  ┌────────────────── External Processes ─────────────────────────┐  │
│  │ Language Servers │ Debug Adapters │ WASM Sandboxes │ LLM APIs │  │
│  │ Process Plugins  │ PTY Shells     │ MCP Servers    │          │  │
│  └───────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────┘
```

### Key Architectural Decisions

**1. Kernel-Authoritative Model**

All mutations go through Rust kernel commands. Frontend holds read-only cached projections.

- Single source of truth eliminates state sync bugs
- Kernel persists/recovers state independently of frontend lifecycle
- Frontend crash (webview OOM) doesn't lose data — kernel holds everything
- Enables future remote-frontend scenarios (SSH, browser client) with zero architecture changes
- Frontend must never store persistent state (only ephemeral UI in Zustand: panel sizes, focus, scroll)
- Every user action that changes data must be an IPC command, never a local mutation
- The Host may retain only process/window routing state needed to operate Tauri and recover the kernel; it cannot answer a domain command without forwarding it to the kernel

**2. Dual-Channel Communication (IPC + WebSocket)**

WebViews use Tauri invoke for request-response, which necessarily terminates in Tauri Core. The Helix Host validates the transport envelope and forwards the unchanged correlation, cancellation, timeout, payload, and typed error semantics over authenticated internal RPC to the kernel. The kernel exposes an authenticated loopback WebSocket for streaming; the Host brokers its endpoint and launch-scoped token.

Why not Tauri events for streaming:
- No backpressure (events fire-and-forget; slow frontend drops events silently)
- No delivery ordering guarantees across concurrent emitters
- No per-channel subscription (all listeners receive all events)
- No binary frame support (JSON overhead for high-throughput terminal data)

Why not WebSocket for commands:
- No built-in request-response correlation
- Harder to type-check (envelope parsing vs. Tauri's generated command types)
- Higher overhead for single-shot queries

| Channel | Use Case | Characteristics |
|---------|----------|----------------|
| Tauri invoke → Host → internal RPC | File read/write, config, git, LSP requests | Typed end to end, correlated, cancellable, timeout-able; domain handler only in kernel |
| WebSocket | Terminal output, agent progress, logs, search streaming, diagnostics push | High-throughput, backpressure-managed, channel-multiplexed |

**3. Plugin Isolation (Hybrid Model)**

| Plugin Type | Isolation | Memory | Performance | Use Cases |
|-------------|-----------|--------|-------------|-----------|
| WASM | Memory-safe sandbox | 64MB limit | Fast (in-process) | Themes, formatters, linters, snippets |
| Process | Separate OS process | Configurable | ~1ms IPC | Language servers, debug adapters, AI agents |

**4. LSP/DAP: Kernel-Managed Process Hosting**

- Kernel multiplexes JSON-RPC messages between frontend and server processes
- Frontend never communicates directly with language servers
- Enables resource limits, crash recovery, multi-root routing, future remote scenarios

**5. AI Engine: Provider-Agnostic Router**

- Capability-based model selection (tools, vision, speed, cost)
- Budget enforcement (token, time, cost limits)
- Circuit breaker per provider for resilience
- Agent isolation via git worktree + OS sandbox

---

## Components and Interfaces

### IPC Protocol

The same generated envelope crosses both request-response hops. The Host may validate size, capability, authentication, and schema version, but it must not deserialize a command into a host-owned domain handler. Internal RPC is local, authenticated with a launch-scoped secret, and epoch-tagged so a stale Host connection cannot send commands to a restarted kernel.

```typescript
// Frontend → Kernel (request)
interface IpcRequest<T> {
  command: string;
  correlationId: string;
  payload: T;
  timeoutMs?: number;
}

// Kernel → Frontend (response)
interface IpcResponse<T> {
  correlationId: string;
  result: T | null;
  error: IpcError | null;
}

interface IpcError {
  code: string;          // e.g., "FILE_NOT_FOUND", "TIMEOUT", "CANCELLED"
  category: "transient" | "permanent" | "cancelled" | "timeout";
  message: string;
  details?: unknown;
}
```

### WebSocket Protocol

```typescript
// Kernel → Frontend (data stream)
interface WsMessage<T> {
  channel: string;        // e.g., "terminal:output", "agent:progress"
  correlationId?: string; // links to originating command if applicable
  sequence: number;       // monotonic per channel for ordering
  payload: T;
}

// Bidirectional control
interface WsControl {
  type: "subscribe" | "unsubscribe" | "backpressure_warning" | "heartbeat";
  channels?: string[];
}
```

### Service Container Interface

```rust
trait Service: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn dependencies(&self) -> &[&'static str];
    async fn start(&mut self, ctx: &ServiceContext) -> Result<(), ServiceError>;
    async fn stop(&mut self) -> Result<(), ServiceError>;
}

trait HealthCheck {
    fn health(&self) -> ServiceHealth; // Healthy | Degraded(reason) | Failed(reason)
    fn metrics(&self) -> ServiceMetrics;
}

enum ServiceHealth {
    Healthy,
    Degraded { reason: String, since: Instant },
    Failed { reason: String, since: Instant },
}

struct ServiceMetrics {
    memory_bytes: u64,
    uptime: Duration,
    request_count: u64,
    error_count: u64,
}
```

### LLM Provider Interface

```rust
#[async_trait]
trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> ModelCapabilities;
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError>;
  async fn chat_stream(&self, req: ChatRequest) -> Result<Pin<Box<dyn Stream<Item = ModelEvent>>>, LlmError>;
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError>;
    async fn embed(&self, req: EmbedRequest) -> Result<EmbedResponse, LlmError>;
    async fn health_check(&self) -> ProviderHealth;
}

struct ModelCapabilities {
    context_window: u32,
  native_tool_protocol: Option<ToolProtocolVersion>,
    supports_vision: bool,
  supports_schema_output: bool,
  supports_parallel_tools: bool,
    speed_tier: SpeedTier,    // Fast | Medium | Slow
    cost_tier: CostTier,      // Free | Cheap | Moderate | Expensive
}

struct ChatRequest {
  messages: Vec<ModelMessage>,
  tools: Vec<ToolDefinition>,
  response_schema: Option<JsonSchema>,
  context_manifest_id: Option<ContextManifestId>,
  limits: GenerationLimits,
}

struct ToolDefinition {
  name: ToolName,
  description: String,
  input_schema: JsonSchema,
  risk: ToolRisk,
  required_capability: Capability,
  timeout: Duration,
  idempotency: Idempotency,
  concurrency: ConcurrencyPolicy,
  max_output_bytes: u64,
}

struct ToolCall {
  id: ToolCallId,
  name: ToolName,
  arguments: serde_json::Value, // validated against input_schema before dispatch
}

struct ToolResult {
  call_id: ToolCallId,
  content: ToolContent,
  trust: ContentTrust,
  artifacts: Vec<ArtifactRef>,
  retryable: bool,
}

enum ModelEvent {
  TextDelta(String),
  ReasoningSummaryDelta(String),
  ToolCallDelta { call_id: ToolCallId, fragment: ToolCallFragment },
  ToolCallReady(ToolCall),
  Usage(ModelUsage),
  Completed(StopReason),
}
```

#### Native Agent Tool Protocol

`ModelEvent::ToolCallReady` is the only model-originated event that can request execution. Provider adapters produce it from their native function/tool-call protocol after assembling and validating the complete call. JSON that merely resembles a tool call inside `TextDelta` remains inert text. This is the security and portability boundary: the agent runtime never parses executable intent from prose and never sees an OpenAI-, Anthropic-, Gemini-, or Ollama-specific response shape.

Ollama participates through its native tool-call and JSON-schema structured-output capabilities, including streamed and parallel tool calls where the selected model supports them. It is one adapter, not the architecture. The same conformance fixtures are replayed through every provider adapter, and only conformant models are eligible for autonomous tool execution.

### AI Development Orchestration

The ordinary autonomous agent assumes a workspace and Git baseline. Application creation needs an earlier orchestration layer that can safely turn an idea into that baseline, then hand control to the same planner and execution runtime used for existing projects.

```
idea
  │
  ▼
Product specification ──► Architecture + stack decision
  │                                  │
  └──────────────┬───────────────────┘
         ▼
    Project Genesis preflight
     ┌─────────┼──────────┐
     ▼         ▼          ▼
 Context     Environment   Skills / recipes
 Engine      Manager       (deterministic)
     └─────────┼──────────┘
         ▼
    temporary genesis sandbox
    scaffold → build → smoke
         │ success only
         ▼
     target + Git baseline commit
         │
         ▼
     normal isolated agent workflow
  plan → worktree → implement → diagnostics
         │
         ▼
 unit/integration/build → launch → browser verify
         │ failure
         └──── diagnose → repair → repeat
         │ success
         ▼
    review evidence + merge
```

Genesis and autonomous execution share provider routing, native tools, context, budgets, trust, audit, and persistence. They do not share mutation targets: Genesis operates in a temporary sandbox and creates the baseline; normal execution operates in a Git worktree created from that baseline. A failed Genesis cannot leave a half-populated final target.

#### Context Engine

The Context Engine is a kernel service, not a prompt helper embedded in each AI feature. Every consumer requests a manifest against a token and privacy budget.

```rust
struct ContextRequest {
  task: AiTaskType,
  query: String,
  workspace: WorkspaceKey,
  explicit: Vec<ContextRef>,
  token_budget: u32,
  provider_policy: ProviderDataPolicy,
  changed_since: Option<CheckpointId>,
}

struct ContextItem {
  id: ContextItemId,
  source: ContextSource,
  revision: ContentHash,
  reason: RetrievalReason,
  trust: ContentTrust,
  freshness: Freshness,
  token_estimate: u32,
  content: ContextContent,
}

struct ContextManifest {
  id: ContextManifestId,
  items: Vec<ContextItem>,
  summaries: Vec<SummaryRef>,
  omitted: Vec<Omission>,
  total_tokens: u32,
  reserved_output_tokens: u32,
}
```

Retrieval combines deterministic ownership/import/dependency/recent-change signals with lexical search and optional semantic similarity. Embeddings may be local, remote when policy permits, or absent; lexical-only mode is a supported degradation path. Hierarchical repository maps and summaries are caches with source hashes, never authorities. A source change invalidates the dependent summary before it can be selected again.

For local agent workloads the router considers both model capability and configured runtime context. Ollama's runtime context can be much smaller than a model's advertised maximum and consumes more memory as it grows, so hardware detection and the actual allocated context participate in routing. The default agent suitability profile targets at least 64K context when hardware permits, but the Context Engine still minimizes prompts and decomposes work; a large window is not permission to send the repository wholesale.

#### Environment Plans and Skills

Environment preparation is declarative and inspectable. Discovery is read-only; mutation begins only from an approved plan.

```rust
struct EnvironmentPlan {
  requirements: Vec<ToolchainRequirement>,
  resolutions: Vec<ToolchainResolution>,
  downloads: Vec<VerifiedArtifact>,
  services: Vec<ManagedServicePlan>,
  environment: BTreeMap<String, SecretSafeValue>,
  rollback: Vec<EnvironmentAction>,
}

struct SkillManifest {
  id: SkillId,
  version: Version,
  input_schema: JsonSchema,
  prerequisites: Vec<SkillRequirement>,
  capabilities: Vec<Capability>,
  steps: Vec<SkillStep>,
  outputs: Vec<DeclaredOutput>,
  verification: Vec<VerificationStep>,
  rollback: Vec<SkillStep>,
}
```

Resolution order is compatible existing tool, project-local/version-manager install, container, then an explicitly approved global installation. Helix never captures elevation credentials. Skills are versioned deterministic procedures over native tools; they may compose other skills through declared outputs and prerequisites, with cycle detection. A model chooses and parameterizes skills but cannot alter their executable steps by emitting prose.

Built-in recipes establish the common paths first: Angular, React/Vite, Next.js, Spring Boot, FastAPI, PostgreSQL, Dockerization, authentication, and test setup. This is especially important for smaller local models: model reasoning chooses the recipe and handles project-specific gaps while framework conventions remain deterministic.

#### Project Genesis State Machine

```
DraftSpec
  → ArchitectureProposed
  → PreflightReady
  → AwaitingApproval
  → ScaffoldingInSandbox
  → BaselineVerifying
  → TargetMaterializing
  → BaselineCommitted
  → HandedToAgent

Any mutable state → Paused | FailedRetained | Discarded
```

Every transition is checkpointed with the target fingerprint, environment snapshot, skill versions, completed deterministic steps, and artifacts. `TargetMaterializing` uses an expected-empty or expected-hash precondition. If the target changes, Genesis stops rather than merging generated content into an unknown directory. Secrets are references resolved only at execution time; generated repositories contain documented variable names and safe templates, never credential values.

#### Verification Runtime

Browser verification is a native tool service layered on the embedded preview and E2E infrastructure, not unrestricted browser automation supplied directly to a model.

```rust
struct VerificationPlan {
  acceptance_criteria: Vec<CriterionId>,
  environment: RuntimeSnapshotId,
  scenarios: Vec<VerificationScenario>,
  limits: VerificationLimits,
}

struct VerificationEvidence {
  criterion: CriterionId,
  actions: Vec<BrowserActionRecord>,
  assertions: Vec<AssertionResult>,
  screenshots: Vec<ArtifactRef>,
  console: Vec<ConsoleRecord>,
  network_failures: Vec<NetworkFailure>,
  accessibility: Vec<AccessibilityViolation>,
  result: VerificationResult,
}
```

Each browser session has an isolated profile and deterministic viewport, locale, color scheme, reduced-motion state, and storage reset. Screenshots are artifacts that may be sent only to a privacy-eligible vision model. Text-only and local models retain DOM, accessibility-tree, console, network, and test evidence, so visual-model availability is an enhancement rather than a correctness dependency.

Browser capabilities are individually gated. Local application navigation and inspection can be pre-approved; external origins, real-account authentication, uploads/downloads, clipboard, devices, and destructive actions require explicit grants. Verification failures enter the same bounded repair loop as compiler and test failures and retain evidence for final review.

#### Specialist Delegation

Helix starts with one orchestrator. Optional specialists are bounded child executions, not independent agents:

```rust
struct Delegation {
  id: DelegationId,
  role: SpecialistRole,
  objective: String,
  context: ContextManifestId,
  allowed_tools: Vec<ToolName>,
  path_scope: Vec<WorkspacePath>,
  budget: AgentBudget,
  expected_artifact: ArtifactSchema,
  completion_criteria: Vec<Criterion>,
}
```

Architect, Implementation, Test, UI Review, Security, and Documentation roles use versioned contracts. They inherit the parent sandbox, worktree, trust policy, audit log, emergency stop, and global budget ceiling. They cannot grant capabilities or create private worktrees. Read-only delegations may run concurrently; write delegations require disjoint declared path scopes or are serialized. The orchestrator validates every structured handoff before incorporating it, and delegation depth is bounded.

**Validates:** REQ-AI-070, REQ-AI-071, REQ-AI-072, REQ-AI-073, REQ-AI-074, REQ-AI-075, REQ-AI-076

### Plugin API Surface

```rust
// WASM plugin host functions (imports available to plugins)
trait PluginHostApi {
    // Editor
    fn editor_get_content(&self, uri: &str) -> Result<String, PluginError>;
    fn editor_set_content(&self, uri: &str, content: &str) -> Result<(), PluginError>;
    fn editor_get_selection(&self) -> Result<Selection, PluginError>;
    fn editor_set_decorations(&self, uri: &str, decorations: &[Decoration]) -> Result<(), PluginError>;

    // Workspace
    fn workspace_list_files(&self, glob: &str) -> Result<Vec<FilePath>, PluginError>;
    fn workspace_read_file(&self, path: &str) -> Result<String, PluginError>;

    // Config (scoped to plugin namespace)
    fn config_get(&self, key: &str) -> Result<Value, PluginError>;

    // Commands
    fn command_register(&self, id: &str, title: &str) -> Result<(), PluginError>;
    fn command_execute(&self, id: &str, args: &Value) -> Result<Value, PluginError>;

    // UI
    fn ui_show_notification(&self, level: NotifLevel, message: &str) -> Result<(), PluginError>;
    fn ui_status_bar_item(&self, id: &str, text: &str, tooltip: &str) -> Result<(), PluginError>;
}
```

### Theming System Interface

```
Token Resolution (3-layer):

Layer 3: Component Tokens (most specific)
  button.primary.background = semantic.accent
  editor.lineHighlight = semantic.highlight
  panel.border = semantic.border
      │ references
Layer 2: Semantic Tokens
  semantic.background = palette.gray.900
  semantic.foreground = palette.gray.100
  semantic.accent = palette.blue.400
  semantic.error = palette.red.400
      │ references
Layer 1: Palette Tokens (raw colors)
  palette.gray.900 = #1a1a2e
  palette.blue.400 = #60a5fa
  palette.red.400 = #f87171
```

Theme file format:
```json
{
  "name": "Helix Dark",
  "type": "dark",
  "palette": { "gray": { "900": "#1a1a2e" }, "blue": { "400": "#60a5fa" } },
  "semantic": { "background": "palette.gray.900", "accent": "palette.blue.400" },
  "editor": { "tokenColors": [], "semanticTokenColors": {} },
  "ui": { "button.primary.background": "semantic.accent" }
}
```

Runtime: tokens compile to CSS custom properties; theme switch = swap variables + update Monaco theme (< 100ms).

### Icon System

Three independent theme axes, each swappable without affecting the others:

| Axis | Setting | Scope |
|------|---------|-------|
| Color theme | `workbench.colorTheme` | All colors (UI + editor tokens) |
| Product icon theme | `workbench.productIconTheme` | UI chrome icons (toolbars, activity bar, gutters, states) |
| File icon theme | `workbench.iconTheme` | File/folder icons (explorer, tabs, pickers) |

**Ownership split.** Icons are a frontend concern; the kernel owns only the *mapping data*, not rendering.

- Kernel: parses icon theme manifests, validates against schema, resolves the file/folder mapping table, serves the resolved table over IPC (`icons.theme`) and pushes changes over WebSocket (`icons:changed`). Same lifecycle as the color theme service, so plugin-contributed themes work through one code path.
- Frontend: owns the sprite, the `<Icon>` component, and per-row lookup. File icon resolution happens synchronously during virtualized tree render against a precomputed map — no IPC in the render path (required for 60fps at 100k nodes).

**Delivery: build-time SVG sprite.**

```
assets/icons/*.svg  ──[build step]──►  sprite.svg (<symbol id="helix.file">…)
                                       + icons.gen.ts (union type of all IDs)
```

- Monochrome SVGs authored on a 16px pixel grid, `fill="currentColor"` so color comes from CSS, and state variants (hover/active/disabled) cost zero extra assets.
- The generated union type makes an unknown icon ID a compile error for first-party code; the runtime placeholder path exists only for plugin-supplied and theme-supplied IDs.
- Rejected alternatives: icon fonts (no multicolor path, ligature/a11y problems, FOUT), one-file-per-icon (hundreds of requests, no tree-shaking win at this count), React-component-per-icon (bundle bloat, no runtime ID indirection needed for themes and plugins).

**Resolution order.**

```
Product icon:  active product icon theme → built-in default set → placeholder glyph
File icon:     exact filename → compound ext (.spec.ts) → simple ext → language ID → generic file
Folder icon:   named folder (src, test, …) → generic folder (open | closed variant)
Color:         context token (e.g. gitDecoration.modified) → icon.foreground → semantic.foreground
```

Every fallback is per-icon, not per-theme: a theme defining 3 of 200 icons renders correctly, with the built-in set filling the rest.

**Component contract.**

```typescript
interface IconProps {
  id: IconId;                    // generated union of known IDs
  size?: "sm" | "md" | "lg";     // 12 | 16 | 20 px, scales with UI zoom
  label?: string;                // present => aria-label; absent => aria-hidden="true"
  spin?: boolean;                // suppressed under prefers-reduced-motion
  className?: string;            // color via token classes, never inline hex
}

// Kernel-served mapping consumed by the file icon hook
interface FileIconTheme {
  id: string;
  fileNames: Record<string, IconId>;
  fileExtensions: Record<string, IconId>;
  languageIds: Record<string, IconId>;
  folderNames: Record<string, { closed: IconId; open: IconId }>;
  defaults: { file: IconId; folder: { closed: IconId; open: IconId } };
}
```

Accessibility falls out of the `label` prop: an icon is either labeled or explicitly hidden, with no third state where a screen reader announces raw SVG. Icon-only buttons are lint-enforced to pass `label`.

**Plugin-contributed icons.** Declared in the manifest, validated at install time, and namespaced to the plugin. SVGs are parsed and re-serialized through a sanitizer (strip `<script>`, external refs, event handlers) before entering the sprite — plugin SVG is untrusted input injected into the DOM, so this is the security boundary.

```json
{ "contributes": { "icons": { "acme.lint.fix": { "default": "./icons/fix.svg" } } } }
```

Budgets: ≤ 8KB per plugin icon, ≤ 150KB gzipped for the first-party sprite.

### Process Supervision

The kernel cannot restart itself, so the Helix Host owns that job. The Host is already required because Tauri Core owns windows and routes WebView IPC; making that same thin process the supervisor avoids a fourth application process while keeping business logic in the kernel. Its own failure has no in-process recovery path, so its surface remains deliberately narrow.

```
┌──────────────────────┐  spawn + monitor   ┌──────────────────────┐
│ Helix Host           │───────────────────►│ Kernel               │
│ (Tauri Core + thin   │◄──── heartbeat ────│ (domain services)    │
│  supervisor)         │  exit code/signal  └──────────┬───────────┘
└──────────┬───────────┘                               │ local WS
      │ owns windows + Tauri invoke               │ streams
      │ forwards typed internal RPC               │
      ▼                                           ▼
     ┌───────────┐                              authenticated client
     │ WebView(s)│
     └───────────┘
```

Supervisor responsibilities and non-responsibilities:

| Does | Does not |
|------|----------|
| Spawn/monitor the kernel and own Tauri windows | Hold IDE domain state |
| Terminate Tauri invoke and forward typed internal RPC | Implement file, workspace, editor, Git, AI, or other domain handlers |
| Detect abnormal exit (code, signal, missed heartbeat) | Load plugins or language servers |
| Apply restart policy and storm damping | Make network requests |
| Capture crash cause and hand it to the reporter | Parse workspace files |
| Enter safe mode after repeated start failures | Own any business logic |

Restart policy:

```
abnormal exit ──► capture cause ──► restart within 2s
   │                                    │
   │  5 restarts in 5 min?              │  3 failed starts?
   ▼                                    ▼
recovery UI                        safe mode
(retry / no-session-restore /      (no plugins,
 open logs)                         no session restore)
```

Clean exit is distinguished from a crash by an explicit shutdown handshake: the kernel acknowledges a quit request before exiting. Any exit without that acknowledgement is treated as a crash. Without this distinction the supervisor would resurrect the application every time the user closed it.

Recovery depends on the durability model already described under State Persistence: the supervisor restores nothing itself, it only restarts the kernel, which then loads its snapshot and replays its WAL.

**Validates:** REQ-ARCH-005, REQ-NFR-002

### Window and Workspace Scoping

One kernel serves every window. This is what makes a second window cheap, and it is the reason service scope has to be explicit.

```
                    ┌─────────── Kernel ───────────┐
Window A ──IPC/WS──►│  Global singletons           │
(workspace 1)       │  settings, keybindings,      │
                    │  secrets, theme, icons,      │
Window B ──IPC/WS──►│  AI providers, command reg   │
(workspace 2)       ├──────────────────────────────┤
                    │  Workspace-scoped (refcount) │
Window C ──IPC/WS──►│  ws1: LSP, watcher, index,   │
(workspace 1)       │       terminals, git         │
                    │  ws2: LSP, watcher, index... │
                    ├──────────────────────────────┤
                    │  Window-scoped               │
                    │  A: layout, editors, focus   │
                    │  B: layout, editors, focus   │
                    └──────────────────────────────┘
```

Three scopes, and every service declares which one it belongs to:

| Scope | Lifetime | Examples |
|-------|----------|----------|
| Global | Kernel lifetime | Settings, keybindings, secrets, theme, icon themes, AI providers, command registry |
| Workspace | Reference-counted across windows | LSP servers, file watcher, search index, git service, terminals, project graph |
| Window | Window lifetime | Layout, open editor set, focus, selection, per-window notifications |

Windows A and C above share workspace 1. Closing A must not stop workspace 1's language servers, so workspace-scoped services are reference-counted by window. This is the single most likely source of bugs in multi-window support, which is why scope is a declared property of every service rather than an emergent one.

Settings changes fan out to every window because settings are global. Layout changes do not, because layout is window-scoped. A change to a workspace setting reaches only the windows bound to that workspace.

**Validates:** REQ-ARCH-006, REQ-ARCH-002

### Command Registry and Keybinding Resolution

Commands are the single entry point for every user action. The palette, keybindings, menus, toolbar buttons, and plugin invocations all dispatch through the same registry, so an action is implemented once and reachable every way.

```
Keybinding  ─┐
Palette     ─┤
Menu item   ─┼──► Command Registry ──► handler
Toolbar     ─┤     (id, title, category,
Plugin call ─┘      enablement, handler)
```

Keybinding resolution runs on the frontend against a context set the kernel does not need to know about:

```
keypress
  ├── chord in progress? ──► accumulate (1.5s timeout)
  ├── collect candidate bindings for the key
  ├── filter by when-clause against current context
  │     editorTextFocus, terminalFocus, panelFocus,
  │     sidebarFocus, inSearch, debugActive, ...
  ├── sort by precedence: user > plugin > default
  │     (last definition wins within a level)
  └── first surviving candidate ──► command dispatch
        none survive ──► keypress falls through to the focused control
```

Enablement is separate from the when-clause. A when-clause decides whether a *binding* applies; enablement decides whether a *command* can run. The palette uses enablement to grey out commands with a reason, which is why an unavailable command explains itself instead of silently doing nothing.

**Validates:** REQ-WB-002, REQ-CONFIG-002

### Search and Index Architecture

One engine, one index, three consumers. The alternative — each surface integrating its own search — was the single largest duplication risk in the original plan.

```
                  ┌──────────── Kernel ────────────┐
                  │                                │
 Workspace find ─►│  ┌──────────────────────────┐  │
 Quick open     ─►│  │   Search Service         │  │
 Symbol search  ─►│  │   ┌────────┐ ┌────────┐  │  │
 Chat @mentions ─►│  │   │ripgrep │ │ index  │  │  │
                  │  │   │(live)  │ │(cached)│  │  │
                  │  └───┴────────┴─┴────────┴──┘  │
                  │         │            │         │
                  └─────────┼────────────┼─────────┘
                     stream results   ┌──┴───────────┐
                     over WS channel  │ path trigram │
                                      │ content trigram│
                                      │ symbol (LSP) │
                                      └──────────────┘
```

Two retrieval paths, chosen by query kind:

| Query | Path | Why |
|-------|------|-----|
| Text/regex content search | ripgrep, live | Faster than maintaining a full inverted index, and always current |
| Fuzzy file path | Path trigram index | Must answer within 50ms per keystroke; a scan cannot |
| Symbol lookup | Symbol index from LSP | Servers are the only source of semantic symbols |
| Content prefilter on huge repos | Content trigram index | Narrows ripgrep's candidate set on 500k-file trees |

The index is a cache, never a source of truth. Every index answer is verifiable against the filesystem, so corruption degrades speed rather than correctness: a checksum mismatch triggers a background rebuild while queries fall back to direct scan. This is why search works during the initial build instead of blocking on it.

**Validates:** REQ-SEARCH-001, REQ-WB-002, REQ-ED-002, REQ-NFR-001

### Monorepo Project Graph

The graph turns "which files are in this repository" into "which project owns this file and what depends on it", which is what scoped search, scoped tasks, and affected-test selection all need.

```
detect tooling ──► extract graph ──► cache to disk
  nx.json                │              │
  turbo.json             │              │ invalidated by
  pnpm-workspace.yaml    │              │ config/lockfile change
  Cargo.toml [workspace] │              │
  go.work, pom.xml, *.sln│              ▼
                         │        ┌──────────────┐
                         └───────►│ Graph Service│
                                  │ ownerOf(path)│
                                  │ dependents(p)│
                                  │ affected(Δ)  │
                                  └──────────────┘
```

Affected-project computation prefers the tool's own answer (`nx affected`, `turbo --filter`) because the tool knows about implicit dependencies the file graph cannot see. The extracted graph is the fallback, not the primary. Extraction is always background and always time-boxed: a monorepo tool that hangs degrades the IDE to per-root behaviour rather than delaying workspace open.

**Validates:** REQ-FS-002, REQ-TASK-001

### Buffer and File Lifecycle

A buffer is not a file. Keeping the two concepts separate is what makes untitled buffers, Save As, encoding conversion, and crash recovery all fall out of one model instead of being special cases.

```
                  ┌──────────────── Buffer ────────────────┐
                  │ id, content, language, dirty, encoding,│
                  │ lineEnding, readOnly, backing?         │
                  └───────────────────┬────────────────────┘
                                      │ backing
        ┌─────────────────────────────┼─────────────────────────┐
        ▼                             ▼                         ▼
   None (untitled)              File(path)              File(path, deleted)
   Save As required        normal save path         dirty-with-no-file:
   WAL-persisted           atomic write             Save As or Close
```

State transitions:

```
new ──► untitled ──Save As──► file
                                │
  external delete ──► file(deleted) ──Save As──► file
                                │
  external change + clean ──► silent reload
  external change + dirty ──► prompt (reload / keep / diff)
```

Encoding and line endings are buffer properties, not file properties, which is what allows "reopen with encoding" and "save with encoding" to be different operations. Conversion is always explicit: a lossy encoding change reports the count of unrepresentable characters before writing, because silently substituting characters corrupts source files in ways that surface much later.

Auto-save composes with this model rather than bypassing it: it triggers the same save path, and it is suppressed while conflict markers are present so a half-resolved merge is never written to disk automatically.

**Validates:** REQ-ED-006, REQ-ED-001, REQ-NFR-002

### Workspace Trust Enforcement

Trust is a gate on execution, not on reading. The design question is where the gate sits, and the answer is: in the kernel, at the point of process launch, not in the UI.

```
                        ┌─── Trust Service (global) ───┐
                        │ trusted paths (user data)    │
                        │ fail closed on unreadable    │
                        └──────────────┬───────────────┘
                                       │ consulted before every launch
   ┌───────────────────────────────────┼───────────────────────────────┐
   ▼               ▼           ▼       ▼        ▼          ▼           ▼
 LSP host      Task runner  DAP host  MCP   Formatters  Plugin      Agent
                                            (workspace) activation
   └──────────── all refuse in Restricted mode, with an actionable reason ────┘

   Always permitted: read, edit, Tree-sitter highlighting, search, git read, chat
```

Enforcement is centralized because the alternative — each subsystem checking trust itself — guarantees that the next subsystem to be added forgets. Any component that spawns a process or evaluates workspace-supplied configuration resolves the trust service first, and the check is part of the launch path rather than a caller responsibility.

Trust state lives in user data, never in the workspace. A workspace that could declare itself trusted would make the entire mechanism decorative.

**Validates:** REQ-FS-005, REQ-SEC-002

### Localization Architecture

The expensive part of internationalization is not translation, it is discovering two years later that strings are scattered through the codebase. The lint rule is therefore the load-bearing component.

```
source ──► extraction ──► base catalog (en) ──► translation ──► locale catalogs
  │                            │                                     │
  │ lint: no literal                                                 │
  │ user-visible strings                                             ▼
  │ (CI failure)                                        ┌────────────────────┐
  └─────────────────────────────────────────────────────► Message Resolver   │
                                                        │ key ──► locale     │
                                                        │   ──► base (en)    │
                                                        │   ──► never blank  │
                                                        └────────────────────┘
```

Two distinct concerns that are easy to conflate:

| Concern | Scope | Independent of |
|---------|-------|----------------|
| UI localization | Chrome strings, formats, layout direction | Editor content |
| Text correctness | Grapheme clusters, combining marks, CJK width, emoji, bidi | UI locale |

Editor text handling must be correct regardless of UI locale. A user running an English UI still edits files containing Arabic, Japanese, and emoji, so cursor movement and deletion operate on grapheme clusters rather than code points or bytes. Bidirectional control character detection is treated as a security feature, not a rendering nicety: invisible reordering characters can make source code read differently from how it compiles.

**Validates:** REQ-WB-005, REQ-NFR-005

---

## Data Models

### Configuration Model

```
Layer Precedence (highest wins):
  Folder settings (.helix/settings.json per folder)
  > Workspace settings (.helix/settings.json at workspace root)
  > User settings (~/.helix/settings.json)
  > Defaults (hardcoded in kernel)

Language-specific overrides: [typescript].editor.tabSize = 2

File format: JSON with comments (JSONC), validated against JSON Schema
```

### Storage Locations

Three storage domains, separated by two questions: would a colleague want this in the repository, and can it be rebuilt if lost?

| Domain | Location | Contents | Committed |
|--------|----------|----------|-----------|
| Workspace configuration | `.helix/` inside the workspace | `settings.json`, `workspace.json`, `tasks.json`, `launch.json`, `mcp.json`, `agent.json`, `snippets/` | Yes, deliberately shareable |
| Session state | OS state directory, keyed by workspace | WAL, snapshots, crash reports, agent task state, agent audit log, conversations | No, never |
| Caches | OS cache directory, keyed by workspace | Search and symbol index, monorepo project graph, resolved theme and icon tables | No, rebuildable |

```
Windows   state  %LOCALAPPDATA%\Helix\state\<workspaceKey>\
          cache  %LOCALAPPDATA%\Helix\cache\<workspaceKey>\
macOS     state  ~/Library/Application Support/Helix/state/<workspaceKey>/
          cache  ~/Library/Caches/Helix/<workspaceKey>/
Linux     state  ${XDG_STATE_HOME:-~/.local/state}/helix/state/<workspaceKey>/
          cache  ${XDG_CACHE_HOME:-~/.cache}/helix/<workspaceKey>/
```

`workspaceKey` is a stable opaque identifier: the `id` field in `.helix/workspace.json` when one exists, otherwise a hash over the sorted set of canonicalized root paths. Hashing the sorted set rather than a single path is what gives multi-root workspaces one unambiguous home instead of one per root, and canonicalizing first means a workspace reached through a symlink resolves to the same key.

Session state deliberately does not live in the workspace. Putting it there creates four problems at once: Helix generates Git noise in every repository it touches, terminal and agent history becomes committable, a read-only or permission-restricted checkout cannot be edited at all, and a multi-root workspace has no principled answer for which root owns the state. The trust model already relies on this separation for the same reason — a workspace that could supply its own trust decision would make trust decorative — and session state deserves the same boundary.

State directories are keyed, not permanent. A directory whose workspace roots no longer exist is pruned after a configurable retention period, default 30 days, so the state root does not grow without bound.

### State Persistence Model

```
State Persistence Strategy:
├── WAL (Write-Ahead Log)
│   ├── Unsaved editor buffers (coalesced, files.walIntervalMs, default 1s)
│   ├── Terminal scrollback (every 5s)
│   ├── Agent task state (every action)
│   └── Location: <stateDir>/wal/
├── Snapshots (periodic, every 5 minutes)
│   ├── Open editors list + cursor positions
│   ├── Panel layout
│   ├── Workspace state (open roots, active file)
│   ├── Terminal sessions (shell + CWD, not scrollback)
│   └── Location: <stateDir>/snapshot.json
└── On Startup:
    ├── Resolve workspaceKey, then load last snapshot
    ├── Replay WAL entries after snapshot timestamp
    └── Restore to pre-crash state
```

A consequence worth stating: because state lives outside the workspace, an unavailable root no longer blocks recovery. The kernel can restore unsaved buffers for a workspace whose network share has not mounted yet, and surface them as dirty buffers awaiting a reachable target.

### Agent State Model

```json
{
  "taskId": "uuid",
  "description": "Add login form with validation",
  "plan": [
    { "step": 1, "action": "file_create", "target": "src/Login.tsx", "status": "completed" },
    { "step": 2, "action": "file_modify", "target": "src/App.tsx", "status": "in_progress" }
  ],
  "currentStep": 2,
  "worktreeBranch": "agent/task-abc123",
  "checkpoints": ["sha1", "sha2"],
  "budget": {
    "tokensUsed": 45000,
    "tokensLimit": 100000,
    "filesWritten": 3,
    "commandsExecuted": 5,
    "elapsedMs": 120000
  }
}
```

### Log Record Model

```json
{
  "ts": "2026-08-07T10:30:00.123Z",
  "level": "info",
  "source": "lsp_host",
  "correlationId": "cmd-abc123",
  "message": "Server started",
  "fields": { "language": "typescript", "pid": 12345, "startupMs": 1200 }
}
```

### Performance Metrics Model

```
Metric Categories:
├── Startup: app_start_to_editor_ready_ms, kernel_init_ms, frontend_hydrate_ms
├── Editor: keystroke_to_render_ms, file_open_ms, completion_latency_ms
├── Search: query_to_first_result_ms, total_search_ms, index_build_ms
├── AI: completion_latency_ms, chat_first_token_ms, agent_step_ms
├── Memory: kernel_rss_mb, frontend_heap_mb, lsp_total_rss_mb
└── IPC: command_roundtrip_ms (p50, p95, p99), ws_message_rate_per_sec

Collection: in-process counters + HDR histograms for latencies
Storage: rolling 1-hour window in memory, hourly aggregates to disk
Export: JSON on demand, optional telemetry endpoint (opt-in)
```

### Workspace Model

```json
{
  "version": 1,
  "id": "01J8ZC4K7Q9V2M0X",
  "roots": [
    { "path": "/home/user/project", "name": "my-project" },
    { "path": "/home/user/shared-lib", "name": "shared-lib" }
  ],
  "settings": {},
  "monorepo": {
    "tool": "nx",
    "projects": ["app", "lib-core", "lib-ui"],
    "graph": { "app": ["lib-core", "lib-ui"], "lib-ui": ["lib-core"] }
  }
}
```

---

## Correctness Properties

### Property 1: State Ownership
The kernel is the single source of truth. Frontend never holds persistent state. Any frontend-displayed data is a projection of kernel state, never a local mutation.

**Validates: Requirements 1.1, 1.2, 1.5, 4.3**
→ REQ-ARCH-001, REQ-ARCH-004

### Property 2: Atomic File Writes
Every file write uses write-to-temp + fsync + atomic rename. No partial file content is ever observable on disk, even under crash.

**Validates: Requirements 16.10, 73.1**
→ REQ-ED-002, REQ-NFR-002

### Property 3: Bounded Recovery Point
Unsaved editor content is recoverable after any crash scenario (kernel panic, webview crash, OS crash, power loss) up to a stated Recovery Point Objective: one WAL flush interval, default 1s. Graceful shutdown flushes before exit and loses nothing. Content already saved to a file is never lost and never partially written, in any failure class.

The RPO is a deliberate trade, not a limitation to be fixed later. Driving it to zero requires a synchronous durable write per edit, which cannot coexist with the 16ms keystroke-to-screen budget in REQ-NFR-001.3. The interval is therefore configurable, so a user who values durability over latency can lower it.

**Validates: Requirements 73.1, 73.7, 13.1**
→ REQ-NFR-002, REQ-ED-006

### Property 4: IPC Correlation
Every IPC request has a unique correlation ID. Every response references the originating correlation ID. Orphan responses are discarded. Timed-out commands are cancelled kernel-side.

**Validates: Requirements 3.1, 3.2, 3.3**
→ REQ-ARCH-003

### Property 5: WebSocket Ordering
Messages within a channel are delivered in monotonically increasing sequence order. Gaps indicate dropped messages (backpressure).

**Validates: Requirements 3.5, 3.8, 3.10**
→ REQ-ARCH-003

### Property 6: Agent Isolation
Agent file operations are restricted to the worktree directory. Path validation occurs in the kernel before every file I/O syscall. No path traversal (../) or symlink escape is possible.

**Validates: Requirements 50.1, 50.2, 50.5, 68.1**
→ REQ-AI-042, REQ-SEC-003

### Property 7: Plugin Sandboxing
WASM plugins cannot access any OS resource without explicit capability grant. Denied operations return errors, never silently succeed.

**Validates: Requirements 62.10, 66.1, 66.2**
→ REQ-PLUG-001, REQ-SEC-001

### Property 8: Secret Safety
API keys and credentials are never stored in configuration files, never logged, never exposed to the frontend or plugins without explicit kernel grant.

**Validates: Requirements 67.4, 67.5, 67.7**
→ REQ-SEC-002

### Property 9: Service Independence
A failure in one kernel service does not cascade to unrelated services. Each service has independent health state and restart capability.

**Validates: Requirements 2.6, 2.7, 73.5, 84.6**
→ REQ-ARCH-002, REQ-NFR-002, REQ-OBS-004

### Property 10: Typing Latency
Under no circumstance does a background operation (indexing, search, AI completion, LSP request) block the editor's keystroke processing pipeline.

**Validates: Requirements 72.3, 72.10**
→ REQ-NFR-001

### Property 11: Icon Resolution Totality
Every icon request resolves to something renderable. Unknown IDs, partial icon themes, and missing theme assets fall back per-icon to the built-in set, then to a visible placeholder. No code path yields a blank slot, a layout shift, or a crash.

**Validates: Requirements 79.1, 80.1**
→ REQ-ICON-001, REQ-ICON-002

### Property 12: Supervised Recovery

An abnormally terminated kernel is always restarted or always surfaces a recovery UI. There is no state in which the kernel is dead and the user is not told. A clean shutdown is never mistaken for a crash, and a crash is never mistaken for a clean shutdown.

**Validates: Requirements 5.1, 5.3, 5.5, 5.6**
→ REQ-ARCH-005, REQ-NFR-002

### Property 13: Trust Fails Closed

No process is launched from workspace-supplied configuration without a positive trust decision for that path. An unreadable or corrupt trust store yields Restricted mode for every folder, never Trusted. Trust state is never sourced from inside the workspace it governs.

**Validates: Requirements 24.3, 24.8, 24.9**
→ REQ-FS-005

### Property 14: Window Isolation

A window's failure, restart, or closure affects no other window. Workspace-scoped services survive the closure of any single window that shares their workspace, and are torn down exactly when the last such window closes.

**Validates: Requirements 6.6, 6.7**
→ REQ-ARCH-006

### Property 15: Message Resolution Totality

Every message key resolves to displayable text. A missing translation falls back to the base locale per key; a missing base entry is a build-time failure, not a runtime blank. No user-visible string bypasses the catalog.

**Validates: Requirements 11.1, 11.3**
→ REQ-WB-005

### Property 16: Index Non-Authority

The search index is never a source of truth. Every answer derived from it is verifiable against the filesystem, so index corruption or staleness degrades performance or completeness but never correctness, and never blocks search availability.

**Validates: Requirements 38.7, 38.9**
→ REQ-SEARCH-001

### Property 17: Suggestion Consent

No AI-generated content modifies a file, a commit, or a configuration without an explicit user action. Streaming previews, ghost text, and generated messages are proposals until accepted. The agent is the sole exception, and only within its worktree under its configured trust level.

**Validates: Requirements 45.1, 46.3, 46.4, 49.2, 53.8**
→ REQ-AI-010, REQ-AI-020, REQ-AI-041, REQ-AI-050

### Property 18: Executable Tool Intent Authenticity

Only a schema-valid canonical `ToolCallReady` event produced by a conformant provider adapter can request execution. Tool-shaped JSON in ordinary model text is inert. Every result references one known call ID, and every dispatch passes capability and approval checks before reaching a tool.

**Validates: Requirements 56.1, 56.2, 56.6, 56.7, 56.8**
→ REQ-AI-071, REQ-AI-041, REQ-SEC-003

### Property 19: Context Is Bounded and Provenanced

Every model request has an explicit context budget. Every selected item names its source revision, retrieval reason, trust class, and freshness. Summaries cannot outlive their source hashes, and excluded, secret, or privacy-ineligible content cannot enter a prompt.

**Validates: Requirements 57.2, 57.4, 57.5, 57.8**
→ REQ-AI-072, REQ-SEC-002

### Property 20: Genesis Target Integrity

Project Genesis never scaffolds directly into the final target. The target is created or populated only from a verified sandbox under an expected-empty or expected-hash precondition. Failure before materialization leaves the target unchanged; failure after materialization remains recoverable from a checkpoint and baseline commit.

**Validates: Requirements 55.3, 55.5, 55.7, 55.9, 55.11**
→ REQ-AI-070, REQ-NFR-002

### Property 21: Environment Mutation Requires a Plan

Environment discovery is read-only. Every download, installation, service launch, global mutation, and rollback is represented in a declarative plan and passes trust/approval before execution. Helix never captures elevation credentials and never replaces a global default to satisfy one project.

**Validates: Requirements 59.1, 59.3, 59.4, 59.5**
→ REQ-AI-074, REQ-FS-005, REQ-AI-041

### Property 22: Recipe Execution Is Deterministic

An executing skill is a pinned, cycle-free sequence of schema-valid native tool calls with declared preconditions, outputs, verification, and rollback. The model may select and parameterize a skill but cannot rewrite its executable steps through prose.

**Validates: Requirements 60.1, 60.3, 60.5, 60.6, 60.9**
→ REQ-AI-075, REQ-AI-071

### Property 23: Verification Claims Have Evidence

Every acceptance criterion reported as verified links to a passing assertion and retained evidence from tests, browser state, logs, network, accessibility, or an explicitly identified manual check. A screenshot without an assertion and a successful build without runtime inspection cannot satisfy a user-facing criterion by themselves.

**Validates: Requirements 58.2, 58.5, 58.7, 58.10**
→ REQ-AI-073

### Property 24: Delegation Cannot Expand Authority

A specialist's tools, paths, budget, models, and capabilities are subsets of the parent task's grants. Specialists share the parent worktree, audit log, pause/cancel signal, and emergency stop. No delegation can recursively create unbounded work or conceal a mutation from the orchestrator.

**Validates: Requirements 61.3, 61.4, 61.7, 61.9**
→ REQ-AI-076, REQ-AI-041, REQ-SEC-003

### Pre/Post Conditions

**File Save:**
- Pre: path is within workspace, content is valid UTF-8 (or binary mode)
- Post: file on disk matches content byte-for-byte, watcher event emitted, editor dirty state cleared

**Agent Task Execution:**
- Pre: plan approved by user, worktree created, budget > 0
- Post: either all steps complete OR task paused at gate/budget/failure with state checkpointed

**Native Tool Dispatch:**
- Pre: provider adapter conformant, complete canonical call, schema valid, call ID unique, tool registered, capability granted, budget available
- Post: exactly one bounded result or typed error is recorded against the call ID; no prose path can bypass dispatch validation

**Project Genesis:**
- Pre: editable specification and architecture accepted as required by trust mode, target fingerprint captured, environment/skill preflight complete, budget available
- Post: either a verified Git baseline is registered and handed to the agent OR the final target is unchanged and a resumable/inspectable sandbox explains the failure

**Verification Run:**
- Pre: runtime snapshot ready, application launch plan declared, browser capabilities granted, acceptance criteria selected
- Post: every criterion has passing evidence, a failed evidence bundle, or an explicit not-run/manual status; all managed processes and browser profiles are stopped or retained by an approved debug action

**Specialist Delegation:**
- Pre: parent task active, bounded objective/context/tools/path/budget declared as subsets of parent grants
- Post: structured handoff accepted/rejected by orchestrator or partial evidence retained; no child process, lock, or independent permission survives parent cancellation

**Plugin Activation:**
- Pre: manifest valid, API version compatible, dependencies resolved, capabilities granted
- Post: plugin running with declared capabilities, registered commands/providers accessible

---

## Error Handling

### Principles

1. **No silent failures.** Every error is either handled (with recovery) or surfaced to the user.
2. **Blast radius containment.** A failure in one service/panel/plugin must not cascade to unrelated functionality.
3. **Prefer degradation over crash.** If a subsystem fails, the rest of the IDE continues working.
4. **User work is sacred, and the worst case is stated.** Unsaved buffers survive any crash scenario up to the RPO in Property 3; saved files survive unconditionally.
5. **Actionable errors.** Every user-facing error includes: what happened, what was affected, and what the user can do.

### Error Categories

| Category | Examples | Handling |
|----------|----------|----------|
| Transient | Network timeout, rate limit | Retry with backoff, user-visible progress |
| Recoverable | LSP crash, plugin crash, webview OOM | Auto-restart affected component, notify user |
| Data integrity | Corrupted config, bad state file | Fall back to defaults/last-known-good, warn user |
| Resource exhaustion | Disk full, memory limit, inotify limit | Degrade gracefully, explain remediation |
| Permanent | Missing binary, invalid license | Clear error with actionable guidance |
| Security | Sandbox violation, invalid signature | Block action, log, alert user |

### Circuit Breaker Pattern

Services interacting with external processes (LSP, DAP, LLM, MCP) use circuit breakers:

```
States: Closed (normal) → Open (failing) → Half-Open (probing)

Closed:  requests flow normally; if error_count > threshold in window → Open
Open:    all requests fail immediately; after cooldown → Half-Open
Half-Open: allow one probe request; success → Closed, failure → Open

Thresholds:
- LSP server: 5 failures in 60s → open for 10s
- LLM provider: 3 failures in 30s → open for 30s
- MCP server: 3 failures in 60s → open for 15s
```

### Recovery Scenarios

| Failure | Detection | Recovery |
|---------|-----------|----------|
| Kernel panic | Process exit code != 0 | Auto-restart, load snapshot + WAL replay |
| Frontend crash (webview OOM) | Kernel detects WS disconnect + no heartbeat | Restart webview, re-push state |
| OS crash / power loss | Stale lock file on startup | Load snapshot + WAL replay (loss bounded by the RPO, default ≤1s of keystrokes) |
| State file corruption | CRC mismatch | Discard corrupted entry, use last valid snapshot |
| Disk full | Write fails | Alert user, degrade (no persistence), prioritize WAL flush |
| LSP server crash | Process exit detected | Auto-restart with exponential backoff (1s → 30s max) |
| LLM provider down | HTTP error / timeout | Circuit breaker opens, try next provider in chain |
| Plugin panic (WASM) | WASM trap | Plugin disabled, IDE unaffected, notification shown |
| Plugin crash (Process) | Process exit | Auto-restart once, disable if recurring |

### Security Architecture

```
Layer 1: OS-Level Isolation
├── Agent sandbox: namespaces (Linux), App Sandbox (macOS), Job Objects (Windows)
├── Process plugins: restricted tokens, minimal permissions
└── WASM plugins: no OS access by design

Layer 2: Application-Level Enforcement
├── Kernel validates all file paths (no traversal)
├── Capability system gates all privileged operations
├── Network whitelist for agent shell
├── Plugin-contributed SVG sanitized before DOM injection (no script, no external refs)
└── Rate limiting on all external operations

Layer 3: Data Protection
├── Secrets in OS keychain (never in files)
├── Secret redaction in all output channels
├── Encrypted conversation storage (AES-256)
└── No PII in logs or telemetry

Layer 4: Supply Chain
├── Plugin signing (Ed25519)
├── Signature verification on install/update
├── SBOM for every release
└── Dependency vulnerability scanning
```

Agent prompt injection defense:
- System prompts immutable (kernel-owned, not in user-accessible files)
- File/terminal content marked as untrusted data in prompt construction
- Output validation: agent actions compared against plan (unexpected actions flagged)
- 3 sandbox violations → auto-pause task for user review

---

## Testing Strategy

### Testing Pyramid

```
                    ┌──────────┐
                    │   E2E    │  ~20 tests
                    │  (WDIO   │  Real packaged binary, real processes
                    │  Tauri)  │  Verify critical user journeys
                    ├──────────┤
                  ┌─┤Integration├─┐  ~200 tests
                  │ │  Tests   │ │  IPC contracts, service interactions
                  │ └──────────┘ │  Real kernel, mock externals
                  ├──────────────┤
               ┌──┤  Component  ├──┐  ~500 tests
               │  │   Tests     │  │  React components (Vitest + Testing Library)
               │  └─────────────┘  │  Kernel services (Rust unit tests)
               ├───────────────────┤
            ┌──┤    Unit Tests     ├──┐  ~2000+ tests
            │  │  (Rust + TS)      │  │  Pure functions, algorithms, parsers
            │  └───────────────────┘  │
            └──────────────────────────┘
```

### Test Categories

**1. Rust Unit Tests (cargo test)**
- Every service has unit tests with mock dependencies
- Coverage target: 80% line coverage for kernel crate
- Run time: < 30s for full suite
- CI gate: must pass on every commit

**2. TypeScript Unit Tests (Vitest)**
- UI components tested with Vitest + @testing-library/react
- State management logic tested in isolation
- Coverage target: 70% for frontend
- Run time: < 20s for full suite

**3. IPC Contract Tests**
- Verify TypeScript IPC client types match Rust command signatures
- Auto-generated from shared schema (build-time check)
- Catch frontend/kernel contract drift before runtime

**4. Integration Tests (Rust, with real kernel)**
- Spin up kernel service container with real services but mock external processes
- Test service interactions (e.g., file save → watcher event → editor update)
- Test WebSocket streaming contracts
- Test state persistence and recovery (crash simulation)
- Run time: < 2 minutes

**5. End-to-End Tests (WebdriverIO + `@wdio/tauri-service`)**
- Real application binary, real filesystem, real processes
- The service's embedded WebDriver server is the default provider, which is what makes macOS work. Driving `tauri-driver` directly supports only Windows and Linux, so it is not an option for a cross-platform gate.
- `tauri-plugin-wdio` gives the tests backend access: `browser.tauri.execute()`, IPC command mocking, and frontend plus backend log capture. Command mocking is what allows an E2E test to exercise the AI journeys without a live LLM.
- Renderer-only scenarios use WDIO browser mode against the Vite dev server, with no binary or driver needed. Playwright remains acceptable for that narrow case, but never for the packaged-app gate.
- Critical user journeys:
  - Open workspace → edit file → save → verify on disk
  - Open terminal → run command → see output
  - Git stage → commit → verify git log
  - AI chat → send message → receive response (mock LLM)
  - Plugin install → activate → verify feature available
- Run time: < 10 minutes
- CI gate: run on merge to main (not every commit)

**6. Performance Regression Tests**
- Benchmark suite in CI (criterion for Rust, custom harness for E2E)
- Metrics: startup time, file open, typing latency, search speed, memory baseline
- Fail CI if any metric degrades > 10% from baseline
- Baseline updated on releases

**7. Fuzz Testing (cargo-fuzz)**
- IPC message parsing, LSP message parsing, config file parsing, WebSocket envelope parsing
- Run: continuous in separate CI job (not gating)
- Crash corpus stored, crashes become regression tests

### Test Infrastructure

| Tool | Purpose | Scope |
|------|---------|-------|
| cargo test | Rust unit + integration | Kernel |
| Vitest | TypeScript unit + component | Frontend |
| WebdriverIO + `@wdio/tauri-service` | E2E against the packaged binary | Full app, Windows / macOS / Linux |
| criterion | Rust benchmarks | Performance |
| cargo-fuzz | Fuzz testing | Parsers, IPC |
| cargo-tarpaulin | Code coverage | Kernel |
| istanbul/c8 | Code coverage | Frontend |
| axe-core | Accessibility testing | Frontend components |

---

## Data Flow Patterns

### Command Pattern (IPC)
```
Frontend                          Kernel
   │                                │
   │── invoke("file.save", {       │
   │      correlationId: "abc",    │
   │      path: "/src/main.rs",    │
   │      content: "..."           │
   │   }) ────────────────────────►│
   │                                │── validate path
   │                                │── write to temp file
   │                                │── atomic rename
   │                                │── emit watcher event
   │   ◄────────────────────────── │── return Ok({ bytesWritten })
   │                                │
   │   [on timeout: cancel kernel- │
   │    side via correlationId]    │
```

### Stream Pattern (WebSocket)
```
Kernel                            Frontend
   │                                │
   │── { channel: "terminal:out",  │
   │     sequence: 42,             │
   │     payload: { id: "t1",     │
   │       data: "Hello\n" }      │
   │   } ────────────────────────►│── route to terminal panel
   │                                │── append to buffer
   │                                │── render
   │                                │
   │   [if frontend slow:]         │
   │── { type: "backpressure",    │
   │     channel: "terminal:out"  │
   │   } ────────────────────────►│── show "output truncated"
```

### LSP Proxy Pattern
```
Frontend                  Kernel                  Language Server
   │                        │                          │
   │── IPC: lsp.completion  │                          │
   │   { uri, position }   │                          │
   │ ─────────────────────►│                          │
   │                        │── route to TS server     │
   │                        │── JSON-RPC request ────►│
   │                        │                          │── compute
   │                        │   ◄──── JSON-RPC resp ──│
   │   ◄────────────────── │── transform + return     │
   │                        │                          │
   │   [if server crashed:] │                          │
   │                        │── detect exit            │
   │                        │── restart with backoff   │
   │   ◄── degraded status │── notify frontend        │
```

### Agent Execution Pattern
```
User                    Frontend              Kernel/Agent Engine
  │                        │                        │
  │── "Add login form"    │                        │
  │ ─────────────────────►│── IPC: agent.start     │
  │                        │ ─────────────────────►│
  │                        │                        │── create worktree
  │                        │                        │── generate plan
  │   ◄── WS: plan ready │◄── WS: agent:plan ──── │
  │── approve plan        │                        │
  │ ─────────────────────►│── IPC: agent.approve   │
  │                        │ ─────────────────────►│
  │                        │                        │── execute step 1
  │   ◄── WS: progress   │◄── WS: agent:progress─│
  │                        │                        │── [gated: file write]
  │   ◄── WS: approval?  │◄── WS: agent:gate ─── │
  │── approve             │                        │
  │ ─────────────────────►│── IPC: agent.gate_resp │
  │                        │ ─────────────────────►│── write file
  │                        │                        │── checkpoint
  │                        │                        │── continue...
```

---

## Observability Architecture

### Structured Logging Pipeline

```
┌─────────────────────────────────────────────────────────┐
│ Sources                                                   │
│ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ │
│ │ Kernel │ │Frontend│ │  LSP   │ │ Agent  │ │Plugins │ │
│ │Services│ │  Logs  │ │Servers │ │Actions │ │        │ │
│ └───┬────┘ └───┬────┘ └───┬────┘ └───┬────┘ └───┬────┘ │
│     └──────────┴──────────┴──────────┴──────────┘       │
│                        │                                  │
│              ┌─────────┴──────────┐                      │
│              │  Log Aggregator     │                      │
│              │  (ring buffer 10k)  │                      │
│              └──┬──────────┬──────┘                      │
│                 │          │                              │
│         ┌──────┴──┐  ┌────┴─────┐                       │
│         │Log File │  │Developer │                       │
│         │(rotate) │  │  Panel   │                       │
│         └─────────┘  └──────────┘                       │
└─────────────────────────────────────────────────────────┘
```

### Health Monitoring

Every kernel service implements the `HealthCheck` trait. Aggregated health is exposed to the frontend via IPC `health.summary` and pushed via WebSocket channel `health:status` on state change. Status bar shows green/yellow/red indicator.

### Crash Reporting Pipeline

Crash data has to be captured by something that outlives the crash, which is why the supervisor is part of this path rather than only the kernel.

```
kernel panic ──► panic hook ──► minidump + context ──┐
                                                     │
abnormal exit ──► supervisor captures ───────────────┤
  exit code, signal, last 20 log lines               │
                                                     ▼
frontend error ──► error boundary / onerror ──► ┌──────────────┐
                                                │  Redaction   │  strip secrets,
                                                │              │  tokens, PII
                                                └──────┬───────┘
                                                       ▼
                                                ┌──────────────┐
                                                │ Local store  │  user can inspect
                                                │ (<stateDir>/ │  the full report
                                                │  crashes/)   │  before sending
                                                └──────┬───────┘
                                        consent? ──no──► stays local
                                            │yes
                                            ▼
                                     configurable endpoint
                                     (default or enterprise)
                                     queued when offline
```

Two ordering constraints matter. Redaction happens before the report touches disk, not before transmission, so a report that is never sent still cannot leak a token to a local file that gets shared later. And consent is checked at send time against current settings rather than at capture time, so revoking consent suppresses reports already queued.

Reports land in the per-workspace state directory described under Storage Locations, never in `.helix/`. A crash report contains stack frames, file paths, and log context, which is the last thing that should be one `git add -A` away from a shared branch.

**Validates:** REQ-OBS-002, REQ-SEC-002, REQ-ARCH-005

### Performance Telemetry

Local collection and remote transmission are deliberately decoupled. Collection is unconditional because the local dashboard is a debugging tool the user owns; only transmission is gated by consent.

The dashboard reads the same counters the CI benchmark gate measures, so a user reporting "startup is slow on my machine" produces a number directly comparable to the recorded baseline. Latencies are held as HDR histograms and reported at p50, p95, and p99, because an average hides exactly the tail that users notice.

**Validates:** REQ-OBS-003, REQ-NFR-001

---

## Open Questions / Future Decisions

### Still open

| # | Question | Blocks | Decision needed by |
|---|----------|--------|--------------------|
| 1 | Plugin API versioning: one semver line for the whole API, or per-surface versioning? | Task 17.3 | Before the API surface is frozen |
| 2 | Marketplace hosting: self-hosted, cloud provider, or federated? | Task 17.6 | Before marketplace launch |
| 3 | License model: open core with proprietary AI, or fully open? | Public release | Before any public release |
| 4 | Remote development transport: SSH tunnel, WebSocket relay, or gRPC? | REQ-REMOTE-001 | Post-v1.0 |
| 5 | Real-time collaboration: CRDT-based co-editing scope and timeline? | Nothing current | Post-v1.0 |

### Decided

| Question | Decision |
|----------|----------|
| gitoxide vs git CLI for advanced workflows | gitoxide (`gix`) for reads and performance-critical paths, git CLI for writes and complex operations. REQ-GIT-001.9, Task 7.1 |
| Who owns Tauri IPC and restarts a crashed kernel | The thin Helix Host is the Tauri Core process and supervisor. It owns windows/capabilities, forwards typed internal RPC, and restarts the separate authoritative kernel; it owns no IDE business logic. REQ-ARCH-003, REQ-ARCH-005, Tasks 1.3 and 1.11 |
| One kernel per window, or one kernel for all windows | One kernel for all windows, with services declaring global, workspace, or window scope. REQ-ARCH-006, Task 2.3 |
| Where the trust gate lives | In the kernel at process-launch points, centralized rather than per-subsystem. REQ-FS-005, Task 1.13 |
| How many search engines | One. ripgrep plus a cache index in a single service consumed by all surfaces. REQ-SEARCH-001, Task 4.5 |
| Icon delivery mechanism | Build-time SVG sprite with a generated ID union, not an icon font and not per-icon files. REQ-ICON-001, Task 2.5 |
| Which driver runs E2E against the packaged app | WebdriverIO with `@wdio/tauri-service` on its embedded provider. Driving `tauri-driver` directly covers only Windows and Linux, which cannot gate a tri-platform release. Task 3.3 |
| Where transient session state lives | OS application-data directory keyed by workspace, not `.helix/`. REQ-NFR-002, Task 1.10 |
| Mobile companion app | Out of scope. Helix is a desktop application (requirements Appendix C) |
| Cloud settings sync | Deferred post-v1.0; requires account infrastructure (requirements Appendix C) |

---

## Requirements Traceability Matrix

Two invariants, enforced by review:

1. Every requirement except REQ-REMOTE-001 has at least one implementing task.
2. Every task cites at least one requirement.

REQ-REMOTE-001 is intentionally unimplemented: it constrains the communication layer without being built in this plan.

| Requirement | Design coverage | Tasks |
|-------------|-----------------|-------|
| REQ-ARCH-001 Authoritative kernel | Architecture, Property 1 | 1.1, 1.2 |
| REQ-ARCH-002 Service container | Service Container Interface, Window Scoping, Property 9 | 1.2, 2.3, 3.1 |
| REQ-ARCH-003 IPC + WebSocket | Architecture, IPC Protocol, WebSocket Protocol, Properties 4-5 | 1.3, 1.4, 3.5, 18.3 |
| REQ-ARCH-004 Frontend architecture | Architecture, Property 1 | 1.1, 2.1, 3.2, 9.4 |
| REQ-ARCH-005 Process supervision | Architecture, Process Supervision, Property 12 | 1.11, 13.1 |
| REQ-ARCH-006 Window management | Window and Workspace Scoping, Property 14 | 2.3, 9.4, 14.3 |
| REQ-WB-001 Workbench layout | Theming, Icon System | 2.1, 2.2 |
| REQ-WB-002 Palette and quick open | Command Registry, Search Architecture | 2.7, 4.7 |
| REQ-WB-003 Notifications | Error Handling principles | 2.6 |
| REQ-WB-004 Welcome and onboarding | — (UI only) | 14.1 |
| REQ-WB-005 Localization | Localization Architecture, Property 15 | 2.9, 14.2 |
| REQ-ED-001 Core editor | Buffer and File Lifecycle | 4.1, 4.2, 4.4 |
| REQ-ED-002 Workspace find/replace | Search Architecture, Property 2 | 4.6 |
| REQ-ED-003 Diff editor | — (component) | 4.9 |
| REQ-ED-004 Merge editor | — (component) | 7.3 (fallback), 10.4 |
| REQ-ED-005 Formatting | — (provider registry) | 4.10, 5.4 |
| REQ-ED-006 File lifecycle | Buffer and File Lifecycle, Properties 2-3 | 1.7, 1.10, 4.3 |
| REQ-ED-007 Snippets | — (component) | 4.11 |
| REQ-ED-008 Structure navigation | — (component) | 5.9 |
| REQ-FS-001 Multi-root workspaces | Workspace Model | 1.8 |
| REQ-FS-002 Monorepo awareness | Monorepo Project Graph | 1.9, 6.2 |
| REQ-FS-003 File explorer | Icon System | 4.8, 7.3 |
| REQ-FS-004 File watching | — (service) | 1.7 |
| REQ-FS-005 Workspace trust | Trust Enforcement, Property 13 | 1.13 |
| REQ-LANG-001 LSP host | LSP Proxy Pattern, Circuit Breaker | 5.1 |
| REQ-LANG-002 LSP features | LSP Proxy Pattern | 5.2, 5.3, 5.4, 5.5, 5.6 |
| REQ-LANG-003 Tree-sitter | — (frontend runtime) | 5.7 |
| REQ-LANG-004 Diagnostics | — (aggregation service) | 5.8 |
| REQ-TERM-001 Terminal | Stream Pattern | 6.1 |
| REQ-TASK-001 Task system | Monorepo Project Graph | 6.2 |
| REQ-TEST-001 Test explorer | — (provider framework) | 10.5 |
| REQ-DEBUG-001 DAP client | Circuit Breaker, Recovery Scenarios | 10.1, 10.2, 10.3 |
| REQ-GIT-001 Core git | — (service) | 7.1, 7.3 |
| REQ-GIT-002 Remote operations | — (service) | 11.1 |
| REQ-GIT-003 Advanced git | — (service) | 11.2, 11.3 |
| REQ-GIT-004 Source control UI | — (component) | 7.2 |
| REQ-GIT-005 VCS abstraction | — (refactoring target) | 16.8 |
| REQ-SEARCH-001 Search and index | Search Architecture, Property 16 | 4.5 |
| REQ-CONFIG-001 Settings | Configuration Model | 1.6, 9.1 |
| REQ-CONFIG-002 Keybindings | Keybinding Resolution | 2.8 |
| REQ-CLI-001 Command-line interface | Window Scoping (single-instance) | 14.3 |
| REQ-AI-001 LLM providers | LLM Provider Interface, Circuit Breaker | 8.1 |
| REQ-AI-002 Routing and budget | LLM Provider Interface | 8.2 |
| REQ-AI-003 Local models | LLM Provider Interface | 12.3 |
| REQ-AI-010 Inline completion | Property 17 | 8.3 |
| REQ-AI-020 Inline edit | Property 17 | 8.4 |
| REQ-AI-030 AI chat | Stream Pattern | 8.5, 8.6, 8.7 |
| REQ-AI-040 Autonomous agent | AI Development Orchestration, Agent Execution Pattern, Agent State Model | 16.2, 16.3 |
| REQ-AI-041 Agent trust | Agent Execution Pattern, Property 17 | 16.4 |
| REQ-AI-042 Agent isolation | Security Architecture, Property 6 | 16.1, 18.3 |
| REQ-AI-043 Agent state | Agent State Model | 16.5 |
| REQ-AI-044 Agent review | — (component) | 16.6 |
| REQ-AI-050 AI workflows | Property 17 | 12.1 |
| REQ-AI-060 MCP support | Circuit Breaker | 12.2 |
| REQ-AI-070 Project Genesis | AI Development Orchestration, Project Genesis State Machine, Property 20 | 12.7, 16.3 |
| REQ-AI-071 Native agent tools | LLM Provider Interface, Native Agent Tool Protocol, Property 18 | 8.1, 16.3 |
| REQ-AI-072 Context engine | AI Development Orchestration, Context Engine, Property 19 | 12.4, 16.2, 16.3 |
| REQ-AI-073 Verification agent | AI Development Orchestration, Verification Runtime, Property 23 | 12.8, 16.3 |
| REQ-AI-074 Environment manager | Environment Plans and Skills, Property 21 | 12.5, 12.7 |
| REQ-AI-075 Skills and recipes | Environment Plans and Skills, Property 22 | 12.6, 12.7, 16.3 |
| REQ-AI-076 Specialist delegation | Specialist Delegation, Property 24 | 16.9 |
| REQ-PLUG-001 Plugin architecture | Plugin API Surface, Property 7 | 17.1, 17.2, 17.3, 17.4 |
| REQ-PLUG-002 Marketplace | Security Architecture (supply chain) | 17.6 |
| REQ-PLUG-003 Bundled plugins | — | 9.5, 17.8 |
| REQ-PLUG-004 Development kit | Plugin API Surface | 17.7 |
| REQ-SEC-001 Plugin sandbox | Security Architecture, Property 7 | 17.5 |
| REQ-SEC-002 Secret management | Security Architecture, Property 8 | 1.12, 1.5, 13.1 |
| REQ-SEC-003 Agent security | Security Architecture, Property 6 | 16.7 |
| REQ-SEC-004 Supply chain | Security Architecture (layer 4) | 15.3 |
| REQ-DIST-001 Distribution | — (build pipeline) | 15.1 |
| REQ-DIST-002 Update system | Recovery Scenarios | 15.2 |
| REQ-NFR-001 Performance | Performance Metrics Model, Property 10 | 3.4, 9.7, 18.2, 18.4 |
| REQ-NFR-002 Reliability | State Persistence, Properties 2-3, 12 | 1.10, 1.11, 9.4, 9.7 |
| REQ-NFR-003 Offline capability | — (standing obligation; verified per capability) | 9.6, 10.1, 10.5, 14.1, 15.2, 17.4 |
| REQ-NFR-004 API stability | Plugin API Surface | 17.3, 17.7, 17.8 |
| REQ-NFR-005 Accessibility | Icon System (a11y contract), Localization | 3.6, 9.2 |
| REQ-THEME-001 Theme architecture | Theming System Interface | 2.4, 9.1 |
| REQ-THEME-002 Syntax colors | Theming System Interface | 2.4 |
| REQ-ICON-001 Product icons | Icon System, Property 11 | 2.5 |
| REQ-ICON-002 File icon themes | Icon System, Property 11 | 2.5, 9.1 |
| REQ-OBS-001 Structured logging | Logging Pipeline, Log Record Model | 1.5 |
| REQ-OBS-002 Crash reporting | Crash Reporting Pipeline | 13.1 |
| REQ-OBS-003 Performance telemetry | Performance Telemetry, Metrics Model | 13.2 |
| REQ-OBS-004 Health monitoring | Health Monitoring | 1.2, 9.3 |
| REQ-PREVIEW-001 Web preview | — (component) | 14.4 |
| REQ-REMOTE-001 Remote development | Architecture (constraint only) | none — deferred by design |

Requirements marked "—" in the design column are deliberately not architected in detail: they are self-contained components or build pipeline concerns whose acceptance criteria fully determine their implementation. Requirements with design coverage are those where a structural decision, a shared contract, or a failure-mode guarantee had to be made before implementation could start.
