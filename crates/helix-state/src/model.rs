use serde::{Deserialize, Serialize};

/// A durable session mutation. Buffer content is intentionally stored here,
/// outside the workspace, so recovery also works for untitled and unavailable files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateMutation {
    Buffer(BufferState),
    Terminal(TerminalState),
    Agent(AgentState),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferState {
    pub id: String,
    pub content: String,
    pub language: String,
    pub target: Option<String>,
    pub dirty: bool,
    pub cursor_line: u32,
    pub cursor_column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalState {
    pub id: String,
    pub shell: String,
    pub cwd: String,
    pub scrollback: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentState {
    pub id: String,
    pub state: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutState {
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub timestamp_ms: u64,
    pub workspace_key: String,
    pub roots: Vec<String>,
    pub buffers: Vec<BufferState>,
    pub terminals: Vec<TerminalState>,
    pub agents: Vec<AgentState>,
    pub layout: LayoutState,
    pub active_file: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub session: SessionSnapshot,
    pub discarded_entries: u64,
    pub snapshot_corrupt: bool,
}
