//! The in-memory ring buffer backing the log viewer (REQ-OBS-001.4).
//!
//! 10,000 entries by default, oldest evicted first. The viewer reads from
//! here rather than from the log file, for two reasons: a query never has to
//! touch the disk while the user is typing in the search box, and the buffer
//! is the one place kernel and frontend records are already interleaved in
//! arrival order (REQ-OBS-001.3).
//!
//! `helix-stream` has its own ring for channel history. That one is keyed by
//! sequence and exists to serve reconnect resume; this one is a plain
//! chronological window and exists to serve queries. Sharing an
//! implementation would mean one of the two carrying machinery it does not
//! need.

use std::collections::VecDeque;

use crate::record::LogRecord;

/// Default viewer buffer depth (REQ-OBS-001.4).
pub const DEFAULT_RING_CAPACITY: usize = 10_000;

/// A bounded, append-only-with-eviction window over the most recent
/// records.
#[derive(Debug)]
pub struct RecordRing {
    entries: VecDeque<LogRecord>,
    capacity: usize,
    /// Records evicted since start. Surfaced to the viewer so "the oldest
    /// entry here is not the oldest entry ever" is visible rather than
    /// implied.
    evicted: u64,
}

impl RecordRing {
    /// A ring with the given capacity. A capacity of zero is coerced to one:
    /// a buffer that silently retains nothing would make the viewer look
    /// broken rather than look empty.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            entries: VecDeque::with_capacity(capacity.min(1_024)),
            capacity,
            evicted: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Append a record, evicting the oldest if the ring is full.
    pub fn push(&mut self, record: LogRecord) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.evicted += 1;
        }
        self.entries.push_back(record);
    }

    /// Records in arrival order, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &LogRecord> {
        self.entries.iter()
    }

    /// Records in reverse arrival order, newest first. The viewer's default
    /// query direction, because a limit should keep the newest matches.
    pub fn iter_rev(&self) -> impl Iterator<Item = &LogRecord> {
        self.entries.iter().rev()
    }

    /// Distinct sources seen in the buffer, sorted. Drives the viewer's
    /// source filter, so it lists what actually logged rather than a
    /// hard-coded set of service names.
    pub fn sources(&self) -> Vec<String> {
        let mut sources: Vec<String> = self
            .entries
            .iter()
            .map(|record| record.source.clone())
            .collect();
        sources.sort_unstable();
        sources.dedup();
        sources
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Resize in place, keeping the newest records when shrinking.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
            self.evicted += 1;
        }
    }
}

impl Default for RecordRing {
    fn default() -> Self {
        Self::new(DEFAULT_RING_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::LogLevel;

    fn record(n: u32) -> LogRecord {
        LogRecord::at(
            format!("2026-01-01T00:00:{:02}.000Z", n % 60),
            LogLevel::Info,
            if n.is_multiple_of(2) {
                "kernel.a"
            } else {
                "kernel.b"
            },
            format!("message {n}"),
        )
    }

    #[test]
    fn the_default_capacity_is_ten_thousand_entries() {
        assert_eq!(RecordRing::default().capacity(), DEFAULT_RING_CAPACITY);
    }

    #[test]
    fn records_are_retained_in_arrival_order() {
        let mut ring = RecordRing::new(10);
        for n in 0..3 {
            ring.push(record(n));
        }
        let messages: Vec<&str> = ring.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(messages, vec!["message 0", "message 1", "message 2"]);
    }

    #[test]
    fn the_oldest_record_is_evicted_once_the_ring_is_full() {
        let mut ring = RecordRing::new(3);
        for n in 0..5 {
            ring.push(record(n));
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.evicted(), 2);
        let messages: Vec<&str> = ring.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(messages, vec!["message 2", "message 3", "message 4"]);
    }

    #[test]
    fn reverse_iteration_yields_the_newest_first() {
        let mut ring = RecordRing::new(10);
        for n in 0..3 {
            ring.push(record(n));
        }
        assert_eq!(ring.iter_rev().next().unwrap().message, "message 2");
    }

    #[test]
    fn sources_are_deduplicated_and_sorted() {
        let mut ring = RecordRing::new(10);
        for n in 0..4 {
            ring.push(record(n));
        }
        assert_eq!(ring.sources(), vec!["kernel.a", "kernel.b"]);
    }

    #[test]
    fn shrinking_keeps_the_newest_records() {
        let mut ring = RecordRing::new(5);
        for n in 0..5 {
            ring.push(record(n));
        }
        ring.set_capacity(2);
        let messages: Vec<&str> = ring.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(messages, vec!["message 3", "message 4"]);
    }

    #[test]
    fn a_zero_capacity_ring_still_retains_one_record() {
        let mut ring = RecordRing::new(0);
        ring.push(record(1));
        assert_eq!(ring.len(), 1);
    }
}
