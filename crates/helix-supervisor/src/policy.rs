use std::collections::VecDeque;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const RESTART_LIMIT: usize = 5;
pub const RESTART_WINDOW: Duration = Duration::from_secs(5 * 60);
pub const SAFE_MODE_FAILURES: u32 = 3;
pub const RESTART_DEADLINE: Duration = Duration::from_secs(2);
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const MISSED_HEARTBEAT_LIMIT: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    Retry,
    StartWithoutSessionRestore,
    OpenLogs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SupervisorStatus {
    Starting {
        safe_mode: bool,
    },
    Running {
        safe_mode: bool,
        epoch: String,
    },
    Recovering {
        attempt: usize,
        safe_mode: bool,
        cause: CrashCause,
    },
    RecoveryRequired {
        safe_mode: bool,
        cause: CrashCause,
    },
    Stopped,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashCause {
    pub timestamp_ms: u64,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub panic_message: Option<String>,
    pub missed_heartbeats: u32,
    pub last_log_lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitDecision {
    Stop,
    Restart { safe_mode: bool, attempt: usize },
    ShowRecovery { safe_mode: bool },
}

/// Pure restart policy. Process I/O lives in the host adapter; time and exit
/// classification stay deterministic here so storm damping is exhaustively tested.
#[derive(Debug, Default)]
pub struct RestartPolicy {
    restarts_ms: VecDeque<u64>,
    consecutive_start_failures: u32,
    safe_mode_latched: bool,
    quit_acknowledged: bool,
}

impl RestartPolicy {
    pub fn acknowledge_quit(&mut self) {
        self.quit_acknowledged = true;
    }

    pub fn record_ready(&mut self) {
        self.consecutive_start_failures = 0;
    }

    pub fn record_start_failure(&mut self) -> bool {
        self.consecutive_start_failures = self.consecutive_start_failures.saturating_add(1);
        if self.consecutive_start_failures >= SAFE_MODE_FAILURES {
            self.safe_mode_latched = true;
        }
        self.safe_mode()
    }

    pub fn safe_mode(&self) -> bool {
        self.safe_mode_latched
    }

    pub fn classify_exit(&mut self, now_ms: u64, success: bool) -> ExitDecision {
        if self.quit_acknowledged || success {
            self.quit_acknowledged = false;
            return ExitDecision::Stop;
        }

        let window_ms = RESTART_WINDOW.as_millis() as u64;
        while self
            .restarts_ms
            .front()
            .is_some_and(|timestamp| now_ms.saturating_sub(*timestamp) > window_ms)
        {
            self.restarts_ms.pop_front();
        }
        if self.restarts_ms.len() >= RESTART_LIMIT {
            return ExitDecision::ShowRecovery {
                safe_mode: self.safe_mode(),
            };
        }
        self.restarts_ms.push_back(now_ms);
        ExitDecision::Restart {
            safe_mode: self.safe_mode(),
            attempt: self.restarts_ms.len(),
        }
    }

    pub fn retry_from_ui(&mut self, without_restore: bool) -> bool {
        self.restarts_ms.clear();
        self.quit_acknowledged = false;
        if without_restore {
            self.safe_mode_latched = true;
        }
        self.safe_mode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abnormal_exit_restarts_until_the_storm_limit_then_shows_recovery() {
        let mut policy = RestartPolicy::default();
        for attempt in 1..=RESTART_LIMIT {
            assert_eq!(
                policy.classify_exit(attempt as u64 * 1_000, false),
                ExitDecision::Restart {
                    safe_mode: false,
                    attempt
                }
            );
        }
        assert_eq!(
            policy.classify_exit(6_000, false),
            ExitDecision::ShowRecovery { safe_mode: false }
        );
    }

    #[test]
    fn old_restarts_age_out_of_the_five_minute_window() {
        let mut policy = RestartPolicy::default();
        for second in 0..RESTART_LIMIT {
            let _ = policy.classify_exit(second as u64 * 1_000, false);
        }
        assert!(matches!(
            policy.classify_exit(RESTART_WINDOW.as_millis() as u64 + 1, false),
            ExitDecision::Restart { attempt: 5, .. }
        ));
    }

    #[test]
    fn acknowledged_or_successful_exit_never_restarts() {
        let mut policy = RestartPolicy::default();
        policy.acknowledge_quit();
        assert_eq!(policy.classify_exit(1, false), ExitDecision::Stop);
        assert_eq!(policy.classify_exit(2, true), ExitDecision::Stop);
    }

    #[test]
    fn three_failed_starts_enable_safe_mode() {
        let mut policy = RestartPolicy::default();
        assert!(!policy.record_start_failure());
        assert!(!policy.record_start_failure());
        assert!(policy.record_start_failure());
        assert!(matches!(
            policy.classify_exit(1, false),
            ExitDecision::Restart {
                safe_mode: true,
                ..
            }
        ));
    }

    #[test]
    fn recovery_ui_can_clear_the_storm_and_force_no_restore() {
        let mut policy = RestartPolicy::default();
        for second in 0..=RESTART_LIMIT {
            let _ = policy.classify_exit(second as u64, false);
        }
        assert!(policy.retry_from_ui(true));
        assert!(matches!(
            policy.classify_exit(10, false),
            ExitDecision::Restart { attempt: 1, .. }
        ));
    }
}
