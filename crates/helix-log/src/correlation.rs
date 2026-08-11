//! Correlation ID propagation (REQ-OBS-001.9).
//!
//! An IPC command arrives with a correlation ID. Everything the kernel does
//! in response — a file read, a config lookup, an LSP request — should be
//! attributable to it, so that "what happened when I hit save?" is a filter
//! in the log viewer rather than a reconstruction from timestamps.
//!
//! Threading the ID through every function signature would work and would be
//! unbearable: every service method would grow a parameter it only forwards.
//! A tokio task-local carries it instead. The dispatcher wraps each handler
//! invocation in [`scope`], and [`crate::Logger::log`] stamps any record that
//! does not already carry one.
//!
//! Task-local rather than thread-local because a handler is an async task
//! that may resume on a different worker thread than it started on; a
//! thread-local would attribute records to whichever request last ran on that
//! thread, which is worse than having no correlation at all.

use std::future::Future;

tokio::task_local! {
    static CORRELATION_ID: Option<String>;
}

/// Run a future with `correlation_id` in scope. Nested scopes shadow the
/// outer one, which is what a kernel-initiated sub-operation with its own ID
/// needs.
pub async fn scope<F>(correlation_id: impl Into<String>, future: F) -> F::Output
where
    F: Future,
{
    CORRELATION_ID
        .scope(Some(correlation_id.into()), future)
        .await
}

/// Run a future with correlation explicitly cleared, for background work
/// started inside a command that outlives it and should not be attributed to
/// it.
pub async fn without_correlation<F>(future: F) -> F::Output
where
    F: Future,
{
    CORRELATION_ID.scope(None, future).await
}

/// The correlation ID in scope, if any. Returns `None` outside a scope,
/// including outside a tokio task entirely, so a synchronous caller does not
/// have to guard the call.
pub fn current() -> Option<String> {
    CORRELATION_ID
        .try_with(|value| value.clone())
        .unwrap_or(None)
}

/// Whether a correlation ID is currently in scope.
pub fn is_active() -> bool {
    current().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_id_is_visible_inside_the_scope_and_not_outside_it() {
        assert_eq!(current(), None);
        scope("cmd-1", async {
            assert_eq!(current().as_deref(), Some("cmd-1"));
            assert!(is_active());
        })
        .await;
        assert_eq!(current(), None);
    }

    #[tokio::test]
    async fn the_id_survives_an_await_point() {
        scope("cmd-2", async {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            assert_eq!(current().as_deref(), Some("cmd-2"));
        })
        .await;
    }

    #[tokio::test]
    async fn a_nested_scope_shadows_the_outer_one() {
        scope("outer", async {
            scope("inner", async {
                assert_eq!(current().as_deref(), Some("inner"));
            })
            .await;
            assert_eq!(current().as_deref(), Some("outer"));
        })
        .await;
    }

    #[tokio::test]
    async fn correlation_can_be_cleared_for_detached_work() {
        scope("outer", async {
            without_correlation(async {
                assert_eq!(current(), None);
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn a_spawned_task_does_not_inherit_the_id() {
        // Documenting the boundary rather than asserting a wish: a task
        // spawned inside a command is independent work, and attributing its
        // records to a command that may already have returned would be a
        // lie. Kernel code that wants the link wraps the spawned future in
        // `scope` explicitly.
        scope("cmd-3", async {
            let inner = tokio::spawn(async { current() }).await.unwrap();
            assert_eq!(inner, None);
            assert_eq!(current().as_deref(), Some("cmd-3"));
        })
        .await;
    }

    #[test]
    fn current_is_none_outside_a_tokio_task() {
        assert_eq!(current(), None);
    }
}
