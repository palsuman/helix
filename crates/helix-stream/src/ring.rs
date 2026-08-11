//! Per-channel ring buffer with oldest-dropped backpressure
//! (REQ-ARCH-003.8).
//!
//! The buffer holds the most recent `depth` messages for one channel and
//! records every eviction. A subscriber is a *cursor* into this buffer
//! rather than a queue of its own, which is what makes two otherwise
//! separate requirements fall out of one structure:
//!
//! - A subscriber that reconnects resumes from its last sequence, so long
//!   as the messages it missed are still buffered (REQ-ARCH-003.10, and the
//!   Task 1.4 demo criterion "the stream resumes with no gap").
//! - A subscriber that reads too slowly falls off the back of the buffer.
//!   The gap between its cursor and the oldest retained sequence is the
//!   exact number of dropped messages, which is what the
//!   `backpressure_warning` control message reports.

use std::collections::VecDeque;

/// Default per-channel buffer depth (REQ-ARCH-003.8). Configurable per
/// channel via [`crate::hub::HubConfig`].
pub const DEFAULT_BUFFER_DEPTH: usize = 1000;

/// A bounded, sequence-tagged history of the most recent messages on one
/// channel.
#[derive(Debug)]
pub struct RingBuffer<T> {
    depth: usize,
    items: VecDeque<(u64, T)>,
    dropped: u64,
}

impl<T> RingBuffer<T> {
    /// A buffer holding at most `depth` items. A depth of zero is coerced
    /// to one: a buffer that drops everything it is given would make every
    /// channel silently dead, which is worse than a misconfiguration that
    /// merely retains less than intended.
    pub fn new(depth: usize) -> Self {
        let depth = depth.max(1);
        Self {
            depth,
            items: VecDeque::with_capacity(depth),
            dropped: 0,
        }
    }

    /// Append an item, evicting the oldest if the buffer is full. Returns
    /// the sequence number of the evicted item, if any.
    pub fn push(&mut self, sequence: u64, item: T) -> Option<u64> {
        let evicted = if self.items.len() == self.depth {
            self.dropped += 1;
            self.items.pop_front().map(|(seq, _)| seq)
        } else {
            None
        };
        self.items.push_back((sequence, item));
        evicted
    }

    /// Sequence of the oldest retained item.
    pub fn oldest_sequence(&self) -> Option<u64> {
        self.items.front().map(|(seq, _)| *seq)
    }

    /// Sequence of the newest retained item.
    pub fn newest_sequence(&self) -> Option<u64> {
        self.items.back().map(|(seq, _)| *seq)
    }

    /// Every retained item newer than `cursor`, oldest first.
    ///
    /// Ordering is inherent: items are appended in sequence order and never
    /// reordered, so iteration satisfies the monotonic-delivery guarantee
    /// (REQ-ARCH-003.10) without a sort.
    pub fn iter_after(&self, cursor: u64) -> impl Iterator<Item = (u64, &T)> {
        self.items
            .iter()
            .filter(move |(seq, _)| *seq > cursor)
            .map(|(seq, item)| (*seq, item))
    }

    /// How many messages a reader at `cursor` has already lost to
    /// eviction. Zero when nothing was missed.
    pub fn gap_after(&self, cursor: u64) -> u64 {
        match self.oldest_sequence() {
            Some(oldest) if oldest > cursor.saturating_add(1) => oldest - cursor - 1,
            _ => 0,
        }
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Total items evicted over the buffer's lifetime.
    pub fn dropped_count(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(depth: usize, count: u64) -> RingBuffer<u64> {
        let mut ring = RingBuffer::new(depth);
        for seq in 1..=count {
            ring.push(seq, seq);
        }
        ring
    }

    #[test]
    fn retains_up_to_depth_and_drops_the_oldest() {
        let ring = filled(3, 5);
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.oldest_sequence(), Some(3));
        assert_eq!(ring.newest_sequence(), Some(5));
        assert_eq!(ring.dropped_count(), 2);
    }

    #[test]
    fn push_reports_the_evicted_sequence_only_when_full() {
        let mut ring = RingBuffer::new(2);
        assert_eq!(ring.push(1, "a"), None);
        assert_eq!(ring.push(2, "b"), None);
        assert_eq!(ring.push(3, "c"), Some(1));
    }

    #[test]
    fn iteration_after_a_cursor_is_ordered_and_exclusive() {
        let ring = filled(10, 5);
        let seen: Vec<u64> = ring.iter_after(2).map(|(seq, _)| seq).collect();
        assert_eq!(seen, vec![3, 4, 5]);
    }

    #[test]
    fn a_cursor_ahead_of_the_buffer_yields_nothing() {
        let ring = filled(10, 3);
        assert_eq!(ring.iter_after(3).count(), 0);
    }

    #[test]
    fn gap_is_zero_while_the_reader_keeps_up() {
        let ring = filled(10, 5);
        assert_eq!(ring.gap_after(2), 0);
        assert_eq!(
            ring.gap_after(0),
            0,
            "cursor 0 expects sequence 1, retained"
        );
    }

    #[test]
    fn gap_counts_exactly_the_messages_lost_to_eviction() {
        // Depth 3 holding 8..10; a reader still at 4 lost 5, 6 and 7.
        let ring = filled(3, 10);
        assert_eq!(ring.gap_after(4), 3);
    }

    #[test]
    fn an_empty_buffer_reports_no_gap() {
        let ring: RingBuffer<u64> = RingBuffer::new(4);
        assert!(ring.is_empty());
        assert_eq!(ring.gap_after(7), 0);
    }

    #[test]
    fn zero_depth_is_coerced_to_one_rather_than_dropping_everything() {
        let mut ring = RingBuffer::new(0);
        assert_eq!(ring.depth(), 1);
        ring.push(1, "a");
        assert_eq!(ring.newest_sequence(), Some(1));
    }

    #[test]
    fn default_depth_matches_the_requirement() {
        assert_eq!(DEFAULT_BUFFER_DEPTH, 1000);
    }
}
