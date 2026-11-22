use std::collections::{BTreeMap, VecDeque};

use crate::api::schema::PaneOutputStreamEvent;

/// One retained stream event with the sequence it was published under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetainedOutputStreamEvent {
    pub sequence: u64,
    pub event: PaneOutputStreamEvent,
}

/// Verdict for a resumption request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutputStreamReplay {
    /// Retained events the client has not applied yet, oldest first.
    Events(Vec<RetainedOutputStreamEvent>),
    /// The requested sequence fell out of the retained window.
    Lost,
}

#[derive(Debug, Default)]
struct PaneOutputStreamState {
    next_sequence: u64,
    /// Tick of this pane's most recent publish, so eviction drops the least
    /// recently published pane instead of the first key in `BTreeMap` order.
    last_publish: u64,
    events: VecDeque<RetainedOutputStreamEvent>,
    bytes: usize,
}

/// Bounded per-pane replay buffer for `pane.output_stream`.
///
/// Deliberately separate from the `EventHub` lifecycle ring: a pane that floods
/// output must not push lifecycle events out of that ring, so stream chunks are
/// retained here under their own caps and dropped oldest-first.
#[derive(Debug)]
pub(crate) struct OutputStreamHistory {
    panes: BTreeMap<String, PaneOutputStreamState>,
    /// How far each evicted pane's sequence had reached. A pane that publishes
    /// again resumes there instead of restarting at 1, so a reconnecting client
    /// never sees a backwards sequence. Bounded to the live pane cap: a
    /// tombstone is sequence bookkeeping, not retained output.
    retired: BTreeMap<String, u64>,
    /// Publish counter behind each pane's `last_publish` tick.
    tick: u64,
    bytes: usize,
}

impl OutputStreamHistory {
    /// Panes retained at once; the least recently published pane is evicted.
    const MAX_PANES: usize = 64;
    /// Events retained per pane.
    const MAX_EVENTS_PER_PANE: usize = 256;
    /// Retained bytes per pane.
    const MAX_BYTES_PER_PANE: usize = 256 * 1024;
    /// Retained bytes across every pane.
    const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

    pub(crate) fn new() -> Self {
        Self {
            panes: BTreeMap::new(),
            retired: BTreeMap::new(),
            tick: 0,
            bytes: 0,
        }
    }

    /// Publishes `event`, stamps it with its sequence, and returns it.
    pub(crate) fn push(
        &mut self,
        pane_id: &str,
        mut event: PaneOutputStreamEvent,
    ) -> PaneOutputStreamEvent {
        self.tick = self.tick.saturating_add(1);
        let state = self.panes.entry(pane_id.to_string()).or_default();
        if let Some(sequence) = self.retired.remove(pane_id) {
            state.next_sequence = sequence;
        }
        state.last_publish = self.tick;
        state.next_sequence = state.next_sequence.saturating_add(1);
        let sequence = state.next_sequence;
        event.sequence = sequence;
        state.bytes = state.bytes.saturating_add(event.text.len());
        state.events.push_back(RetainedOutputStreamEvent {
            sequence,
            event: event.clone(),
        });
        self.bytes = self.bytes.saturating_add(event.text.len());
        self.trim(pane_id);
        event
    }

    /// The next sequence a pane would publish under.
    #[allow(dead_code)] // part of the output stream API surface
    pub(crate) fn next_sequence(&self, pane_id: &str) -> u64 {
        self.panes
            .get(pane_id)
            .map_or(0, |state| state.next_sequence)
    }

    /// Events published after `sequence`, or `Lost` when they were dropped.
    pub(crate) fn replay(&self, pane_id: &str, sequence: u64) -> OutputStreamReplay {
        let Some(state) = self.panes.get(pane_id) else {
            return OutputStreamReplay::Lost;
        };
        let oldest = state
            .events
            .front()
            .map_or(state.next_sequence.saturating_add(1), |event| {
                event.sequence
            });
        if sequence.saturating_add(1) < oldest {
            return OutputStreamReplay::Lost;
        }
        OutputStreamReplay::Events(
            state
                .events
                .iter()
                .filter(|event| event.sequence > sequence)
                .cloned()
                .collect(),
        )
    }

    fn trim(&mut self, pane_id: &str) {
        while self.panes.len() > Self::MAX_PANES {
            // Least recently published goes first; `pane_id` just published, so
            // it is never the victim.
            let victim = self
                .panes
                .iter()
                .filter(|(candidate, _)| candidate.as_str() != pane_id)
                .min_by_key(|(_, state)| state.last_publish)
                .map(|(candidate, _)| candidate.clone());
            let Some(victim) = victim else {
                break;
            };
            if let Some(state) = self.panes.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(state.bytes);
                self.retire(&victim, state.next_sequence);
            }
        }

        let Some(state) = self.panes.get_mut(pane_id) else {
            return;
        };
        while state.events.len() > Self::MAX_EVENTS_PER_PANE
            || state.bytes > Self::MAX_BYTES_PER_PANE
            || self.bytes > Self::MAX_TOTAL_BYTES
        {
            let Some(dropped) = state.events.pop_front() else {
                break;
            };
            state.bytes = state.bytes.saturating_sub(dropped.event.text.len());
            self.bytes = self.bytes.saturating_sub(dropped.event.text.len());
        }
    }

    /// Remembers how far an evicted pane's sequence had reached, so the pane
    /// keeps its sequence space when it publishes again. Bounded to the live
    /// pane cap; `BTreeMap` order is stable, so the same key always goes first.
    fn retire(&mut self, pane_id: &str, sequence: u64) {
        self.retired.insert(pane_id.to_string(), sequence);
        while self.retired.len() > Self::MAX_PANES {
            let Some(victim) = self.retired.keys().next().cloned() else {
                break;
            };
            self.retired.remove(&victim);
        }
    }
}

impl Default for OutputStreamHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::PaneOutputStreamPhase;

    fn event(text: &str) -> PaneOutputStreamEvent {
        PaneOutputStreamEvent {
            pane_id: "pane_1".into(),
            workspace_id: "workspace_1".into(),
            sequence: 0,
            phase: PaneOutputStreamPhase::Delta,
            text: text.into(),
            truncated: false,
        }
    }

    #[test]
    fn push_stamps_monotonic_per_pane_sequences() {
        let mut history = OutputStreamHistory::new();
        assert_eq!(history.push("pane_1", event("a")).sequence, 1);
        assert_eq!(history.push("pane_1", event("b")).sequence, 2);
        // A second pane has its own sequence space.
        assert_eq!(history.push("pane_2", event("c")).sequence, 1);
        assert_eq!(history.next_sequence("pane_1"), 2);
        assert_eq!(history.next_sequence("pane_2"), 1);
        assert_eq!(history.next_sequence("pane_3"), 0);
    }

    #[test]
    fn replay_returns_only_events_after_the_requested_sequence() {
        let mut history = OutputStreamHistory::new();
        for text in ["a", "b", "c"] {
            history.push("pane_1", event(text));
        }
        let OutputStreamReplay::Events(events) = history.replay("pane_1", 1) else {
            panic!("expected retained events");
        };
        assert_eq!(
            events
                .iter()
                .map(|event| event.event.text.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert_eq!(events[0].sequence, 2);
    }

    #[test]
    fn replay_reports_loss_once_events_are_dropped() {
        let mut history = OutputStreamHistory::new();
        for index in 0..=OutputStreamHistory::MAX_EVENTS_PER_PANE {
            history.push("pane_1", event(&index.to_string()));
        }
        assert_eq!(history.replay("pane_1", 0), OutputStreamReplay::Lost);
        // The oldest retained sequence still resumes cleanly.
        assert!(matches!(
            history.replay("pane_1", 1),
            OutputStreamReplay::Events(_)
        ));
    }

    #[test]
    fn replay_of_an_unseen_pane_is_a_loss_not_an_empty_stream() {
        let history = OutputStreamHistory::new();
        assert_eq!(history.replay("pane_1", 7), OutputStreamReplay::Lost);
    }

    #[test]
    fn byte_caps_bound_a_flooding_pane() {
        let mut history = OutputStreamHistory::new();
        let chunk = "x".repeat(8 * 1024);
        for _ in 0..64 {
            history.push("pane_1", event(&chunk));
        }
        assert_eq!(
            history.replay("pane_1", 0),
            OutputStreamReplay::Lost,
            "a flood must not retain the whole pane history"
        );
        assert!(matches!(
            history.replay("pane_1", 40),
            OutputStreamReplay::Events(_)
        ));
    }

    #[test]
    fn eviction_drops_the_least_recently_published_pane() {
        let mut history = OutputStreamHistory::new();
        for index in 0..OutputStreamHistory::MAX_PANES {
            history.push(&format!("pane_{index}"), event("a"));
        }
        // Publishing pane_0 again makes it the most recent pane, not the first key.
        history.push("pane_0", event("b"));
        history.push("overflow", event("a"));

        assert!(
            matches!(history.replay("pane_0", 1), OutputStreamReplay::Events(_)),
            "a pane that just published must survive eviction"
        );
        assert_eq!(
            history.replay("pane_1", 0),
            OutputStreamReplay::Lost,
            "the least recently published pane is the eviction victim"
        );
    }

    #[test]
    fn an_evicted_pane_resumes_its_sequence_space() {
        let mut history = OutputStreamHistory::new();
        history.push("pane_0", event("a"));
        history.push("pane_0", event("b"));
        // Overflow the pane cap so the stale pane_0 is evicted.
        for index in 1..=OutputStreamHistory::MAX_PANES {
            history.push(&format!("pane_{index}"), event("x"));
        }
        assert_eq!(history.replay("pane_0", 0), OutputStreamReplay::Lost);

        // The pane comes back: it continues at 3 instead of restarting at 1, so a
        // client resuming at sequence 2 sees forward progress, never a rewind.
        assert_eq!(history.push("pane_0", event("c")).sequence, 3);
        assert!(matches!(
            history.replay("pane_0", 2),
            OutputStreamReplay::Events(_)
        ));
    }

    #[test]
    fn panes_beyond_the_cap_evict_the_oldest_entry() {
        let mut history = OutputStreamHistory::new();
        for index in 0..=OutputStreamHistory::MAX_PANES {
            history.push(&format!("pane_{index}"), event("a"));
        }
        assert_eq!(history.replay("pane_0", 0), OutputStreamReplay::Lost);
        assert!(matches!(
            history.replay(&format!("pane_{}", OutputStreamHistory::MAX_PANES), 0),
            OutputStreamReplay::Events(_)
        ));
    }
}
