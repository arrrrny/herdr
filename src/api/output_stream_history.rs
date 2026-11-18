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
            bytes: 0,
        }
    }

    /// Publishes `event`, stamps it with its sequence, and returns it.
    pub(crate) fn push(
        &mut self,
        pane_id: &str,
        mut event: PaneOutputStreamEvent,
    ) -> PaneOutputStreamEvent {
        let state = self
            .panes
            .entry(pane_id.to_string())
            .or_default();
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
        if self.panes.len() > Self::MAX_PANES {
            // BTreeMap order is stable, so the same pane always goes first.
            let victim = self
                .panes
                .keys()
                .find(|candidate| candidate.as_str() != pane_id)
                .cloned();
            if let Some(victim) = victim {
                if let Some(state) = self.panes.remove(&victim) {
                    self.bytes = self.bytes.saturating_sub(state.bytes);
                }
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
