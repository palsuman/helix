//! `helix-supervisor` — the thin process that launches and monitors the
//! kernel.
//!
//! Per REQ-ARCH-005 and the design document's Process Supervision section,
//! this binary is deliberately minimal: no business logic, no plugin
//! loading, no network access. Its own crash surface must stay small because
//! nothing supervises the supervisor.
//!
//! Full restart policy, crash-cause capture, and recovery UI land in Task
//! 1.11. This is a placeholder entry point so the workspace builds and CI
//! can exercise it from Task 1.1 onward.

fn main() {
    println!("helix-supervisor starting (placeholder — full supervision lands in Task 1.11)");
}

#[cfg(test)]
mod tests {
    // Placeholder to confirm the crate compiles and its test harness runs
    // under `cargo test`. Real supervisor behavior tests land in Task 1.11.
    #[test]
    fn placeholder_binary_compiles() {
        let version = env!("CARGO_PKG_VERSION");
        assert!(!version.is_empty());
    }
}
