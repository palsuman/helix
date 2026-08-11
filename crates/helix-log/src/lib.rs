//! `helix-log` — structured logging for the whole application (Task 1.5,
//! REQ-OBS-001, REQ-SEC-002.5).
//!
//! One aggregator, several sinks, one record format. Kernel services, the
//! frontend, LSP servers, the agent, and plugins all emit the same
//! [`LogRecord`] through the same [`Logger`], which is what makes a single
//! viewable stream possible (REQ-OBS-001.3) and what makes redaction
//! unconditional (REQ-OBS-001.11) rather than a thing each sink remembers.
//!
//! ```text
//!  kernel services ─┐
//!  frontend (IPC) ──┤          ┌─ ring buffer (10k) ─► log viewer queries
//!  LSP / agent ─────┼─► Logger ─┼─ rotating file (50MB × 5)
//!  plugins ─────────┘  ▲       ├─ stdout (CLI launches)
//!                      │       └─ registered sinks ─► live stream, crash reports
//!               redaction +
//!               correlation
//! ```
//!
//! Module map:
//!
//! - [`record`] — the record model and level ordering.
//! - [`filter`] — per-module level configuration and the viewer's query.
//! - [`ring`] — the bounded in-memory window the viewer reads.
//! - [`rotate`] — the size-based rotating file sink.
//! - [`redact`] — secret and content removal, applied before any sink.
//! - [`correlation`] — task-local correlation ID propagation.
//! - [`logger`] — the aggregator that ties those together.
//! - [`commands`] — the `log.*` IPC payloads and the streaming channel name.
//!
//! Like `helix-ipc` and `helix-stream`, this crate has no Tauri dependency:
//! `helix-kernel` wires it to commands and to the stream hub, while tests
//! drive the identical code path with no process around it.

pub mod commands;
pub mod correlation;
pub mod filter;
pub mod logger;
pub mod macros;
pub mod record;
pub mod redact;
pub mod ring;
pub mod rotate;
pub mod time;

pub use commands::{
    LogAppendRequest, LogAppendResponse, LogExportRequest, LogExportResponse, LogLevelsRequest,
    LogLevelsResponse, LogQueryRequest, LogQueryResponse, LogSetLevelRequest,
};
pub use filter::{LevelConfig, LogQuery};
pub use logger::{LogSink, Logger, LoggerConfig, LoggerMetrics, QueryResult};
pub use record::{Fields, LogLevel, LogRecord, to_field};
pub use redact::{OMITTED_CONTENT, REDACTED, Redactor};
pub use ring::{DEFAULT_RING_CAPACITY, RecordRing};
pub use rotate::{DEFAULT_FILE_NAME, DEFAULT_MAX_FILE_BYTES, DEFAULT_MAX_FILES, RotatingFileSink};
