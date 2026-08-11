//! Health reporting model shared by every kernel service.
//!
//! Full implementation of the service container and its `HealthCheck` trait
//! lands in Task 1.2. This module defines the data shapes so `helix-ipc` and
//! `helix-kernel` can be scaffolded against a stable contract now.

use serde::{Deserialize, Serialize};

/// The health state of a single kernel service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ServiceHealth {
    Healthy,
    Degraded { reason: String, since_ms: u64 },
    Failed { reason: String, since_ms: u64 },
}

impl ServiceHealth {
    pub fn is_healthy(&self) -> bool {
        matches!(self, ServiceHealth::Healthy)
    }
}

/// Point-in-time metrics for a kernel service.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServiceMetrics {
    pub memory_bytes: u64,
    pub uptime_ms: u64,
    pub request_count: u64,
    pub error_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_state_reports_healthy() {
        assert!(ServiceHealth::Healthy.is_healthy());
    }

    #[test]
    fn degraded_state_is_not_healthy() {
        let state = ServiceHealth::Degraded {
            reason: "high memory".into(),
            since_ms: 1000,
        };
        assert!(!state.is_healthy());
    }

    #[test]
    fn metrics_default_to_zero() {
        let m = ServiceMetrics::default();
        assert_eq!(m.memory_bytes, 0);
        assert_eq!(m.request_count, 0);
    }
}
