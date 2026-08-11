//! Per-module level configuration (REQ-OBS-001.2) and the viewer's query
//! filter (REQ-OBS-001.4).
//!
//! Two different kinds of filtering live here on purpose. [`LevelConfig`]
//! decides what is *recorded*, and is consulted on the hot path before a
//! record is built. [`LogQuery`] decides what is *shown*, and is applied to
//! records already in the ring. Conflating them would mean the viewer's
//! filters silently changed what reaches the log file.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::record::{LogLevel, LogRecord};

/// Which levels are recorded, globally and per module (REQ-OBS-001.2).
///
/// A module name is dot-separated (`kernel.fs.watcher`) and resolution is a
/// longest-prefix match, so setting `kernel.fs` to `trace` turns on tracing
/// for the watcher too without naming it. That is what makes the setting
/// usable: a developer debugging the file system does not have to enumerate
/// its submodules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LevelConfig {
    /// Applied to any source without a matching module override.
    pub default_level: LogLevel,
    /// Module prefix to level. `BTreeMap` so the serialized form (and the
    /// viewer's display of it) is deterministic.
    pub modules: BTreeMap<String, LogLevel>,
}

impl Default for LevelConfig {
    fn default() -> Self {
        Self {
            default_level: LogLevel::Info,
            modules: BTreeMap::new(),
        }
    }
}

impl LevelConfig {
    pub fn new(default_level: LogLevel) -> Self {
        Self {
            default_level,
            modules: BTreeMap::new(),
        }
    }

    pub fn with_module(mut self, module: impl Into<String>, level: LogLevel) -> Self {
        self.modules.insert(module.into(), level);
        self
    }

    pub fn set_module(&mut self, module: impl Into<String>, level: LogLevel) {
        self.modules.insert(module.into(), level);
    }

    pub fn clear_module(&mut self, module: &str) -> bool {
        self.modules.remove(module).is_some()
    }

    /// The effective minimum level for a source: the most specific matching
    /// module prefix, else the default.
    pub fn level_for(&self, source: &str) -> LogLevel {
        if self.modules.is_empty() {
            return self.default_level;
        }
        if let Some(level) = self.modules.get(source) {
            return *level;
        }
        let mut best: Option<(usize, LogLevel)> = None;
        for (module, level) in &self.modules {
            // Prefix match on a segment boundary only, so `kernel.fs` does
            // not capture `kernel.fsevents`.
            if source.len() > module.len()
                && source.starts_with(module.as_str())
                && source.as_bytes()[module.len()] == b'.'
                && best.map(|(len, _)| module.len() > len).unwrap_or(true)
            {
                best = Some((module.len(), *level));
            }
        }
        best.map(|(_, level)| level).unwrap_or(self.default_level)
    }

    /// The most verbose level any source could be enabled at. The logger
    /// caches this in an atomic and rejects anything below it without
    /// taking a lock, which is what makes a disabled level cost one atomic
    /// load (REQ-OBS-001.8).
    pub fn min_enabled_level(&self) -> LogLevel {
        self.modules
            .values()
            .copied()
            .chain(std::iter::once(self.default_level))
            .min()
            .unwrap_or(self.default_level)
    }

    pub fn enabled(&self, level: LogLevel, source: &str) -> bool {
        level >= self.level_for(source)
    }
}

/// A viewer query: level, source, time range, full-text search, and
/// correlation ID (REQ-OBS-001.4, .9).
///
/// Every field is optional and they compose with AND. `Default` therefore
/// means "everything", which is the state the viewer opens in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct LogQuery {
    /// Records at this level or above.
    pub min_level: Option<LogLevel>,
    /// Records at exactly one of these levels. Combined with `min_level` by
    /// AND, which lets the viewer offer both a threshold and a set of
    /// checkboxes without them contradicting each other.
    pub levels: Option<Vec<LogLevel>>,
    /// Sources to include. A source matches if it equals an entry or is a
    /// dot-separated descendant of one, so filtering to `kernel` includes
    /// `kernel.ipc`.
    pub sources: Option<Vec<String>>,
    /// Inclusive RFC 3339 lower bound on `ts`.
    pub from_ts: Option<String>,
    /// Inclusive RFC 3339 upper bound on `ts`.
    pub to_ts: Option<String>,
    /// Case-insensitive substring searched across message, source, and the
    /// serialized fields.
    pub search: Option<String>,
    /// Exact correlation ID, which is how the viewer answers "what did the
    /// kernel do for this command" (REQ-OBS-001.9).
    pub correlation_id: Option<String>,
    /// Maximum number of records returned, newest first.
    pub limit: Option<u32>,
}

impl LogQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_min_level(mut self, level: LogLevel) -> Self {
        self.min_level = Some(level);
        self
    }

    pub fn with_levels(mut self, levels: impl IntoIterator<Item = LogLevel>) -> Self {
        self.levels = Some(levels.into_iter().collect());
        self
    }

    pub fn with_sources(mut self, sources: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.sources = Some(sources.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_time_range(
        mut self,
        from_ts: Option<impl Into<String>>,
        to_ts: Option<impl Into<String>>,
    ) -> Self {
        self.from_ts = from_ts.map(Into::into);
        self.to_ts = to_ts.map(Into::into);
        self
    }

    pub fn with_search(mut self, search: impl Into<String>) -> Self {
        self.search = Some(search.into());
        self
    }

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }

    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Whether a record satisfies every populated criterion.
    pub fn matches(&self, record: &LogRecord) -> bool {
        if let Some(min_level) = self.min_level
            && record.level < min_level
        {
            return false;
        }
        if let Some(levels) = &self.levels
            && !levels.contains(&record.level)
        {
            return false;
        }
        if let Some(sources) = &self.sources
            && !sources.iter().any(|s| source_matches(s, &record.source))
        {
            return false;
        }
        // Fixed-width RFC 3339 makes a string comparison a chronological
        // one, so the range test needs no date parsing.
        if let Some(from) = &self.from_ts
            && record.ts.as_str() < from.as_str()
        {
            return false;
        }
        if let Some(to) = &self.to_ts
            && record.ts.as_str() > to.as_str()
        {
            return false;
        }
        if let Some(correlation_id) = &self.correlation_id
            && record.correlation_id.as_deref() != Some(correlation_id.as_str())
        {
            return false;
        }
        if let Some(search) = &self.search
            && !search.is_empty()
            && !full_text_match(record, search)
        {
            return false;
        }
        true
    }
}

/// A filter entry matches a source exactly, or as its dot-separated
/// ancestor.
fn source_matches(filter: &str, source: &str) -> bool {
    source == filter
        || (source.len() > filter.len()
            && source.starts_with(filter)
            && source.as_bytes()[filter.len()] == b'.')
}

/// Case-insensitive search across the message, the source, the correlation
/// ID, and the field values.
///
/// Fields are included because the interesting part of a structured record is
/// frequently in the fields rather than the message ("which language server
/// was that?"), and a search that could not see them would send users back to
/// grepping the file.
fn full_text_match(record: &LogRecord, needle: &str) -> bool {
    let needle = needle.to_ascii_lowercase();
    if record.message.to_ascii_lowercase().contains(&needle)
        || record.source.to_ascii_lowercase().contains(&needle)
    {
        return true;
    }
    if let Some(correlation_id) = &record.correlation_id
        && correlation_id.to_ascii_lowercase().contains(&needle)
    {
        return true;
    }
    if record.fields.is_empty() {
        return false;
    }
    serde_json::to_string(&record.fields)
        .map(|fields| fields.to_ascii_lowercase().contains(&needle))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::to_field;

    fn record(level: LogLevel, source: &str, message: &str) -> LogRecord {
        LogRecord::at("2026-01-01T12:00:00.000Z", level, source, message)
    }

    // ---- level configuration -------------------------------------------

    #[test]
    fn the_default_level_applies_to_an_unconfigured_source() {
        let config = LevelConfig::new(LogLevel::Info);
        assert_eq!(config.level_for("kernel.fs"), LogLevel::Info);
        assert!(config.enabled(LogLevel::Warn, "kernel.fs"));
        assert!(!config.enabled(LogLevel::Debug, "kernel.fs"));
    }

    #[test]
    fn a_module_level_overrides_the_default_for_that_module_only() {
        let config = LevelConfig::new(LogLevel::Info).with_module("kernel.fs", LogLevel::Trace);
        assert!(config.enabled(LogLevel::Trace, "kernel.fs"));
        assert!(
            !config.enabled(LogLevel::Trace, "kernel.ipc"),
            "an unrelated module must keep the default level"
        );
    }

    #[test]
    fn a_module_level_applies_to_descendants() {
        let config = LevelConfig::new(LogLevel::Warn).with_module("kernel.fs", LogLevel::Debug);
        assert_eq!(config.level_for("kernel.fs.watcher"), LogLevel::Debug);
    }

    #[test]
    fn prefix_matching_respects_segment_boundaries() {
        let config = LevelConfig::new(LogLevel::Warn).with_module("kernel.fs", LogLevel::Trace);
        assert_eq!(
            config.level_for("kernel.fsevents"),
            LogLevel::Warn,
            "a longer name that merely starts with the module must not match"
        );
    }

    #[test]
    fn the_most_specific_module_wins() {
        let config = LevelConfig::new(LogLevel::Error)
            .with_module("kernel", LogLevel::Warn)
            .with_module("kernel.fs", LogLevel::Debug)
            .with_module("kernel.fs.watcher", LogLevel::Trace);
        assert_eq!(config.level_for("kernel.ipc"), LogLevel::Warn);
        assert_eq!(config.level_for("kernel.fs.reader"), LogLevel::Debug);
        assert_eq!(config.level_for("kernel.fs.watcher"), LogLevel::Trace);
    }

    #[test]
    fn min_enabled_level_is_the_most_verbose_configured_level() {
        let config = LevelConfig::new(LogLevel::Warn).with_module("kernel.fs", LogLevel::Trace);
        assert_eq!(config.min_enabled_level(), LogLevel::Trace);
        assert_eq!(
            LevelConfig::new(LogLevel::Error).min_enabled_level(),
            LogLevel::Error
        );
    }

    #[test]
    fn clearing_a_module_restores_the_default() {
        let mut config = LevelConfig::new(LogLevel::Info).with_module("noisy", LogLevel::Error);
        assert!(config.clear_module("noisy"));
        assert!(!config.clear_module("noisy"));
        assert_eq!(config.level_for("noisy"), LogLevel::Info);
    }

    // ---- viewer query ---------------------------------------------------

    #[test]
    fn an_empty_query_matches_everything() {
        let query = LogQuery::new();
        assert!(query.matches(&record(LogLevel::Trace, "a", "hello")));
    }

    #[test]
    fn min_level_filters_out_quieter_records() {
        let query = LogQuery::new().with_min_level(LogLevel::Warn);
        assert!(query.matches(&record(LogLevel::Error, "a", "boom")));
        assert!(!query.matches(&record(LogLevel::Info, "a", "fine")));
    }

    #[test]
    fn an_explicit_level_set_filters_to_those_levels() {
        let query = LogQuery::new().with_levels([LogLevel::Warn, LogLevel::Error]);
        assert!(query.matches(&record(LogLevel::Warn, "a", "x")));
        assert!(!query.matches(&record(LogLevel::Debug, "a", "x")));
    }

    #[test]
    fn source_filtering_includes_descendants() {
        let query = LogQuery::new().with_sources(["kernel"]);
        assert!(query.matches(&record(LogLevel::Info, "kernel", "x")));
        assert!(query.matches(&record(LogLevel::Info, "kernel.ipc", "x")));
        assert!(!query.matches(&record(LogLevel::Info, "frontend.app", "x")));
    }

    #[test]
    fn a_time_range_is_inclusive_at_both_ends() {
        let query = LogQuery::new().with_time_range(
            Some("2026-01-01T12:00:00.000Z"),
            Some("2026-01-01T12:00:00.000Z"),
        );
        assert!(query.matches(&record(LogLevel::Info, "a", "x")));

        let earlier = LogRecord::at("2026-01-01T11:59:59.999Z", LogLevel::Info, "a", "x");
        let later = LogRecord::at("2026-01-01T12:00:00.001Z", LogLevel::Info, "a", "x");
        assert!(!query.matches(&earlier));
        assert!(!query.matches(&later));
    }

    #[test]
    fn full_text_search_is_case_insensitive_across_message_source_and_fields() {
        let entry = record(LogLevel::Info, "lsp_host", "Server started")
            .with_field("language", to_field("TypeScript"));
        assert!(LogQuery::new().with_search("server").matches(&entry));
        assert!(LogQuery::new().with_search("LSP_HOST").matches(&entry));
        assert!(LogQuery::new().with_search("typescript").matches(&entry));
        assert!(!LogQuery::new().with_search("python").matches(&entry));
    }

    #[test]
    fn an_empty_search_string_is_not_a_filter() {
        let query = LogQuery::new().with_search("");
        assert!(query.matches(&record(LogLevel::Info, "a", "x")));
    }

    #[test]
    fn correlation_id_filtering_is_exact() {
        let entry = record(LogLevel::Info, "fs", "write").with_correlation_id("cmd-1");
        assert!(LogQuery::new().with_correlation_id("cmd-1").matches(&entry));
        assert!(!LogQuery::new().with_correlation_id("cmd-2").matches(&entry));
        assert!(
            !LogQuery::new()
                .with_correlation_id("cmd-1")
                .matches(&record(LogLevel::Info, "fs", "write")),
            "a record with no correlation id must not match a correlated query"
        );
    }

    #[test]
    fn criteria_compose_with_and() {
        let entry = record(LogLevel::Error, "kernel.fs", "disk full")
            .with_correlation_id("cmd-9")
            .with_field("path", to_field("/tmp/x"));
        let query = LogQuery::new()
            .with_min_level(LogLevel::Warn)
            .with_sources(["kernel"])
            .with_search("disk")
            .with_correlation_id("cmd-9");
        assert!(query.matches(&entry));

        let narrowed = query.clone().with_search("network");
        assert!(!narrowed.matches(&entry));
    }

    #[test]
    fn a_query_round_trips_through_json_with_absent_fields_defaulted() {
        let query: LogQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(query, LogQuery::default());

        let populated = LogQuery::new()
            .with_min_level(LogLevel::Debug)
            .with_sources(["kernel"])
            .with_limit(50);
        let json = serde_json::to_string(&populated).unwrap();
        assert_eq!(serde_json::from_str::<LogQuery>(&json).unwrap(), populated);
    }
}
