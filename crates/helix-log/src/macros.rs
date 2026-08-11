//! The logging macros (REQ-OBS-001.8).
//!
//! A macro rather than a method for one reason: the level check has to
//! surround the *arguments*, not just the record. A function call would
//! evaluate every field expression before the callee could decide the level
//! is disabled, and "zero cost when the level is disabled" would only be true
//! of the storage, not of the formatting and allocation the call site did to
//! produce the values.
//!
//! Usage:
//!
//! ```
//! use helix_log::{Logger, log_info};
//! # use helix_log::LogLevel;
//! let logger = Logger::in_memory(LogLevel::Info);
//! log_info!(logger, "lsp_host", "Server started", "language" => "typescript", "startup_ms" => 1200);
//! ```
//!
//! The first argument is anything that derefs to a [`crate::Logger`], so an
//! `Arc<Logger>` held by a service works without an explicit deref.

/// Emit a record at an explicit level, skipping all argument evaluation when
/// that level is disabled for the source.
#[macro_export]
macro_rules! log_event {
    ($logger:expr, $level:expr, $source:expr, $message:expr $(, $key:expr => $value:expr)* $(,)?) => {{
        // Annotated rather than inferred so a `Logger`, a `&Logger`, and an
        // `Arc<Logger>` all work at the call site through deref coercion.
        let __helix_logger: &$crate::Logger = &$logger;
        let __helix_level = $level;
        let __helix_source: &str = $source;
        if __helix_logger.enabled(__helix_level, __helix_source) {
            #[allow(unused_mut)]
            let mut __helix_fields = $crate::record::Fields::new();
            $(
                __helix_fields.insert(
                    ::std::string::String::from($key),
                    $crate::record::to_field($value),
                );
            )*
            __helix_logger.log(
                $crate::record::LogRecord::new(__helix_level, __helix_source, $message)
                    .with_fields(__helix_fields),
            );
        }
    }};
}

/// Emit a `trace` record.
#[macro_export]
macro_rules! log_trace {
    ($logger:expr, $source:expr, $message:expr $(, $key:expr => $value:expr)* $(,)?) => {
        $crate::log_event!($logger, $crate::LogLevel::Trace, $source, $message $(, $key => $value)*)
    };
}

/// Emit a `debug` record.
#[macro_export]
macro_rules! log_debug {
    ($logger:expr, $source:expr, $message:expr $(, $key:expr => $value:expr)* $(,)?) => {
        $crate::log_event!($logger, $crate::LogLevel::Debug, $source, $message $(, $key => $value)*)
    };
}

/// Emit an `info` record.
#[macro_export]
macro_rules! log_info {
    ($logger:expr, $source:expr, $message:expr $(, $key:expr => $value:expr)* $(,)?) => {
        $crate::log_event!($logger, $crate::LogLevel::Info, $source, $message $(, $key => $value)*)
    };
}

/// Emit a `warn` record.
#[macro_export]
macro_rules! log_warn {
    ($logger:expr, $source:expr, $message:expr $(, $key:expr => $value:expr)* $(,)?) => {
        $crate::log_event!($logger, $crate::LogLevel::Warn, $source, $message $(, $key => $value)*)
    };
}

/// Emit an `error` record.
#[macro_export]
macro_rules! log_error {
    ($logger:expr, $source:expr, $message:expr $(, $key:expr => $value:expr)* $(,)?) => {
        $crate::log_event!($logger, $crate::LogLevel::Error, $source, $message $(, $key => $value)*)
    };
}

#[cfg(test)]
mod tests {
    use crate::{LogLevel, LogQuery, Logger};
    use std::sync::Arc;

    #[test]
    fn a_macro_accepts_a_bare_logger_a_reference_and_an_arc() {
        let logger = Logger::in_memory(LogLevel::Trace);
        log_info!(logger, "s", "bare");

        let by_reference = &logger;
        log_info!(by_reference, "s", "reference");

        let shared = Arc::new(Logger::in_memory(LogLevel::Trace));
        log_info!(shared, "s", "arc");

        assert_eq!(logger.query(&LogQuery::new()).entries.len(), 2);
        assert_eq!(shared.query(&LogQuery::new()).entries.len(), 1);
    }

    #[test]
    fn a_macro_accepts_a_formatted_message_and_no_fields() {
        let logger = Logger::in_memory(LogLevel::Trace);
        let port = 4242u16;
        log_warn!(logger, "stream", format!("bound on {port}"));
        assert_eq!(
            logger.query(&LogQuery::new()).entries[0].message,
            "bound on 4242"
        );
    }

    #[test]
    fn a_macro_accepts_a_trailing_comma_after_the_last_field() {
        let logger = Logger::in_memory(LogLevel::Trace);
        log_info!(logger, "s", "m", "a" => 1, "b" => 2,);
        let fields = &logger.query(&LogQuery::new()).entries[0].fields;
        assert_eq!(fields["a"], 1);
        assert_eq!(fields["b"], 2);
    }

    #[test]
    fn a_field_key_may_be_a_computed_string() {
        let logger = Logger::in_memory(LogLevel::Trace);
        let key = String::from("dynamic");
        log_info!(logger, "s", "m", key => 7);
        assert_eq!(
            logger.query(&LogQuery::new()).entries[0].fields["dynamic"],
            7
        );
    }
}
