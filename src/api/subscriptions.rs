use regex::Regex;

use crate::api::event_hub::EventHistoryError;
use crate::api::output_stream_history::OutputStreamReplay;
use crate::api::schema::{
    ErrorBody, ErrorResponse, EventKind, Method, PaneAgentStatusChangedEvent,
    PaneOutputMatchedEvent, PaneOutputStreamEvent, PaneOutputStreamPhase, PaneScrollChangedEvent,
    PaneScrollInfo, ReadFormat, ReadSource, Request, Subscription, SubscriptionEventData,
    SubscriptionEventEnvelope, SubscriptionEventKind,
};
use crate::api::server::{dispatch_to_app_with_timeout, APP_RESPONSE_TIMEOUT};
use crate::api::{ApiRequestSender, EventHub};

/// Lines a `pane.output_stream` subscription reads per poll and per snapshot.
pub(super) const DEFAULT_OUTPUT_STREAM_LINES: u32 = 64;
/// Upper bound on the read window, matching `pane.read`'s own clamp.
pub(super) const MAX_OUTPUT_STREAM_LINES: u32 = 1000;
/// How many candidate split points the delta stitch tries before it gives up
/// and asks the client to resync. A normal append matches on the first try.
///
/// The budget is spent one byte at a time, not one UTF-8 boundary at a time, so
/// a full repaint of multi-byte text can consume it without ever testing a
/// meaningful split point. That only costs a resync, never correctness.
const MAX_OUTPUT_OVERLAP_ATTEMPTS: usize = 64;

pub(super) fn output_match_read_source(
    source: &crate::api::schema::ReadSource,
) -> crate::api::schema::ReadSource {
    match source {
        crate::api::schema::ReadSource::Recent => crate::api::schema::ReadSource::RecentUnwrapped,
        other => *other,
    }
}

/// Splits the newest pane read into what a client holding `previous` still needs.
///
/// Both reads are suffixes of the same growing buffer, so the client's text is a
/// suffix of the new read: the new output starts where the two overlap ends. A
/// zero-length overlap means the client's text is gone (a clear or a full
/// repaint), which is a resync rather than a delta.
pub(super) fn output_stream_delta<'a>(
    previous: &str,
    next: &'a str,
) -> (PaneOutputStreamPhase, &'a str) {
    if previous.is_empty() {
        return (PaneOutputStreamPhase::Snapshot, next);
    }

    let mut offset = previous.len().min(next.len());
    let mut attempts = 0;
    loop {
        while offset > 0 && !next.is_char_boundary(offset) {
            offset -= 1;
        }
        if previous.ends_with(&next[..offset]) {
            return if offset == 0 {
                (PaneOutputStreamPhase::Resync, next)
            } else {
                (PaneOutputStreamPhase::Delta, &next[offset..])
            };
        }
        if offset == 0 || attempts >= MAX_OUTPUT_OVERLAP_ATTEMPTS {
            return (PaneOutputStreamPhase::Resync, next);
        }
        attempts += 1;
        offset -= 1;
    }
}

pub(super) fn match_output(
    text: &str,
    matcher: &crate::api::schema::OutputMatch,
    regex: Option<&Regex>,
) -> Option<String> {
    match matcher {
        crate::api::schema::OutputMatch::Substring { value } => text
            .lines()
            .find(|line| line.contains(value))
            .map(|line| line.to_string()),
        crate::api::schema::OutputMatch::Regex { .. } => regex.and_then(|re| {
            text.lines()
                .find(|line| re.is_match(line))
                .map(|line| line.to_string())
        }),
    }
}

pub(super) struct ActiveOutputMatchedSubscription {
    pane_id: String,
    source: crate::api::schema::ReadSource,
    lines: Option<u32>,
    matcher: crate::api::schema::OutputMatch,
    regex: Option<Regex>,
    strip_ansi: bool,
    currently_matching: bool,
    request_prefix: String,
}

pub(super) struct ActiveAgentStatusChangedSubscription {
    pane_id: String,
    status_filter: Option<crate::api::schema::AgentStatus>,
    last_status: Option<crate::api::schema::AgentStatus>,
    last_presentation: Option<PanePresentationSnapshot>,
    last_sequence: u64,
    initial_event: Option<PaneAgentStatusChangedEvent>,
    request_prefix: String,
}

pub(super) struct ActiveScrollChangedSubscription {
    pane_id: String,
    last_scroll: Option<PaneScrollInfo>,
    request_prefix: String,
}

/// Streams one pane's output: a snapshot first, then coalesced deltas.
pub(super) struct ActiveOutputStreamSubscription {
    pane_id: String,
    workspace_id: String,
    source: ReadSource,
    lines: u32,
    format: ReadFormat,
    strip_ansi: bool,
    /// The exact text the client holds, so the next delta stitches against it.
    sent: String,
    /// Events replayed from the retained stream history after a resubscribe.
    replayed: std::collections::VecDeque<SubscriptionEventEnvelope>,
    /// The first event to emit: a snapshot, or a resync after a lost replay.
    initial_event: Option<PaneOutputStreamEvent>,
    request_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PanePresentationSnapshot {
    title: Option<String>,
    display_agent: Option<String>,
    state_labels: std::collections::HashMap<String, String>,
}

impl PanePresentationSnapshot {
    fn from(pane: &crate::api::schema::PaneInfo) -> Self {
        Self {
            title: pane.title.clone(),
            display_agent: pane.display_agent.clone(),
            state_labels: pane.state_labels.clone(),
        }
    }

    fn from_event(
        title: &Option<String>,
        display_agent: &Option<String>,
        state_labels: &std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            title: title.clone(),
            display_agent: display_agent.clone(),
            state_labels: state_labels.clone(),
        }
    }
}

pub(super) struct ActiveEventSubscription {
    event_kind: crate::api::schema::EventKind,
    /// Live cursor. Seeded with the event-hub sequence captured at subscribe
    /// time, so the poll loop only yields events emitted after the
    /// subscription started.
    last_sequence: u64,
}

pub(super) enum ActiveSubscription {
    Event(ActiveEventSubscription),
    OutputMatched(ActiveOutputMatchedSubscription),
    AgentStatusChanged(Box<ActiveAgentStatusChangedSubscription>),
    ScrollChanged(ActiveScrollChangedSubscription),
    OutputStream(ActiveOutputStreamSubscription),
}

impl ActiveSubscription {
    pub(super) fn new(
        subscription: Subscription,
        request_id: &str,
        index: usize,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
        event_start_sequence: u64,
    ) -> Result<Self, ErrorResponse> {
        let event_subscription = |event_kind| {
            Self::Event(ActiveEventSubscription {
                event_kind,
                last_sequence: event_start_sequence,
            })
        };

        match subscription {
            Subscription::WorkspaceCreated {} => {
                Ok(event_subscription(EventKind::WorkspaceCreated))
            }
            Subscription::WorkspaceUpdated {} => {
                Ok(event_subscription(EventKind::WorkspaceUpdated))
            }
            Subscription::WorkspaceMetadataUpdated {} => {
                Ok(event_subscription(EventKind::WorkspaceMetadataUpdated))
            }
            Subscription::WorkspaceRenamed {} => {
                Ok(event_subscription(EventKind::WorkspaceRenamed))
            }
            Subscription::WorkspaceMoved {} => Ok(event_subscription(EventKind::WorkspaceMoved)),
            Subscription::WorkspaceReordered {} => {
                Ok(event_subscription(EventKind::WorkspaceReordered))
            }
            Subscription::WorkspaceClosed {} => Ok(event_subscription(EventKind::WorkspaceClosed)),
            Subscription::WorkspaceFocused {} => {
                Ok(event_subscription(EventKind::WorkspaceFocused))
            }
            Subscription::WorktreeCreated {} => Ok(event_subscription(EventKind::WorktreeCreated)),
            Subscription::WorktreeOpened {} => Ok(event_subscription(EventKind::WorktreeOpened)),
            Subscription::WorktreeRemoved {} => Ok(event_subscription(EventKind::WorktreeRemoved)),
            Subscription::TabCreated {} => Ok(event_subscription(EventKind::TabCreated)),
            Subscription::TabClosed {} => Ok(event_subscription(EventKind::TabClosed)),
            Subscription::TabFocused {} => Ok(event_subscription(EventKind::TabFocused)),
            Subscription::TabRenamed {} => Ok(event_subscription(EventKind::TabRenamed)),
            Subscription::TabMoved {} => Ok(event_subscription(EventKind::TabMoved)),
            Subscription::PaneCreated {} => Ok(event_subscription(EventKind::PaneCreated)),
            Subscription::PaneClosed {} => Ok(event_subscription(EventKind::PaneClosed)),
            Subscription::PaneUpdated {} => Ok(event_subscription(EventKind::PaneUpdated)),
            Subscription::PaneFocused {} => Ok(event_subscription(EventKind::PaneFocused)),
            Subscription::PaneMoved {} => Ok(event_subscription(EventKind::PaneMoved)),
            Subscription::PaneExited {} => Ok(event_subscription(EventKind::PaneExited)),
            Subscription::PaneAgentDetected {} => {
                Ok(event_subscription(EventKind::PaneAgentDetected))
            }
            Subscription::LayoutUpdated {} => Ok(event_subscription(EventKind::LayoutUpdated)),
            Subscription::PaneOutputMatched {
                pane_id,
                source,
                lines,
                r#match,
                strip_ansi,
            } => {
                let regex = match &r#match {
                    crate::api::schema::OutputMatch::Regex { value } => match Regex::new(value) {
                        Ok(regex) => Some(regex),
                        Err(err) => {
                            return Err(ErrorResponse {
                                id: request_id.to_string(),
                                error: ErrorBody {
                                    code: "invalid_regex".into(),
                                    message: err.to_string(),
                                },
                            });
                        }
                    },
                    crate::api::schema::OutputMatch::Substring { .. } => None,
                };

                let probe = pane_read(
                    format!("{request_id}:sub:{index}:probe"),
                    &pane_id,
                    source,
                    lines,
                    crate::api::schema::ReadFormat::Text,
                    strip_ansi,
                    api_tx,
                );
                probe?;

                Ok(Self::OutputMatched(ActiveOutputMatchedSubscription {
                    pane_id,
                    source,
                    lines,
                    matcher: r#match,
                    regex,
                    strip_ansi,
                    currently_matching: false,
                    request_prefix: format!("{request_id}:sub:{index}"),
                }))
            }
            Subscription::PaneAgentStatusChanged {
                pane_id,
                agent_status,
            } => {
                let probe = pane_get(format!("{request_id}:sub:{index}:probe"), &pane_id, api_tx)?;
                let last_sequence = event_hub.current_sequence();
                let last_status = probe.agent_status;
                let last_presentation = PanePresentationSnapshot::from(&probe);
                let initial_event = agent_status
                    .is_some_and(|wanted| wanted == probe.agent_status)
                    .then_some(PaneAgentStatusChangedEvent {
                        pane_id: probe.pane_id.clone(),
                        workspace_id: probe.workspace_id,
                        agent_status: probe.agent_status,
                        agent: probe.agent,
                        title: probe.title,
                        display_agent: probe.display_agent,
                        state_labels: probe.state_labels,
                    });

                Ok(Self::AgentStatusChanged(Box::new(
                    ActiveAgentStatusChangedSubscription {
                        pane_id: probe.pane_id,
                        status_filter: agent_status,
                        last_status: Some(last_status),
                        last_presentation: Some(last_presentation),
                        last_sequence,
                        initial_event,
                        request_prefix: format!("{request_id}:sub:{index}"),
                    },
                )))
            }
            Subscription::PaneScrollChanged { pane_id } => {
                let probe = pane_get(format!("{request_id}:sub:{index}:probe"), &pane_id, api_tx)?;

                Ok(Self::ScrollChanged(ActiveScrollChangedSubscription {
                    pane_id: probe.pane_id,
                    last_scroll: probe.scroll,
                    request_prefix: format!("{request_id}:sub:{index}"),
                }))
            }
            Subscription::PaneOutputStream {
                pane_id,
                source,
                lines,
                format,
                strip_ansi,
                since_sequence,
            } => {
                let lines = lines
                    .unwrap_or(DEFAULT_OUTPUT_STREAM_LINES)
                    .clamp(1, MAX_OUTPUT_STREAM_LINES);
                let request_prefix = format!("{request_id}:sub:{index}");
                let probe = pane_get(format!("{request_prefix}:probe"), &pane_id, api_tx)?;
                // Unwrapped history, like `pane.output_matched`: the client's own
                // emulator owns wrapping, so it must receive unwrapped lines.
                let source = output_match_read_source(&source);
                let read = pane_read(
                    format!("{request_prefix}:snapshot"),
                    &pane_id,
                    source,
                    Some(lines),
                    format,
                    strip_ansi,
                    api_tx,
                )?;

                let mut replayed = std::collections::VecDeque::new();
                let mut sent = read.text.clone();
                let initial_event = match since_sequence {
                    Some(since) => match event_hub.output_stream_replay(&pane_id, since) {
                        OutputStreamReplay::Events(events) => {
                            // Replayed chunks keep the sequences they were
                            // published under and are already retained, so they
                            // go straight onto the queue instead of back through
                            // `output_stream_push`.
                            for retained in events {
                                sent = retained.event.text.clone();
                                replayed.push_back(SubscriptionEventEnvelope {
                                    event: SubscriptionEventKind::PaneOutputStream,
                                    data: SubscriptionEventData::PaneOutputStream(retained.event),
                                });
                            }
                            None
                        }
                        OutputStreamReplay::Lost => Some(PaneOutputStreamEvent {
                            pane_id: pane_id.clone(),
                            workspace_id: probe.workspace_id.clone(),
                            sequence: 0,
                            phase: PaneOutputStreamPhase::Resync,
                            text: read.text.clone(),
                            truncated: read.truncated,
                        }),
                    },
                    None => Some(PaneOutputStreamEvent {
                        pane_id: pane_id.clone(),
                        workspace_id: probe.workspace_id.clone(),
                        sequence: 0,
                        phase: PaneOutputStreamPhase::Snapshot,
                        text: read.text.clone(),
                        truncated: read.truncated,
                    }),
                };

                Ok(Self::OutputStream(ActiveOutputStreamSubscription {
                    pane_id: probe.pane_id,
                    workspace_id: probe.workspace_id,
                    source,
                    lines,
                    format,
                    strip_ansi,
                    sent,
                    replayed,
                    initial_event,
                    request_prefix,
                }))
            }
        }
    }

    pub(super) fn poll(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Option<serde_json::Value> {
        match self {
            Self::Event(subscription) => subscription.poll(event_hub),
            Self::OutputMatched(subscription) => {
                serde_json::to_value(subscription.poll(api_tx)?).ok()
            }
            Self::AgentStatusChanged(subscription) => {
                serde_json::to_value(subscription.poll(api_tx, event_hub)?).ok()
            }
            Self::ScrollChanged(subscription) => {
                serde_json::to_value(subscription.poll(api_tx)?).ok()
            }
            Self::OutputStream(subscription) => {
                serde_json::to_value(subscription.poll(api_tx, event_hub)?).ok()
            }
        }
    }

    pub(super) fn poll_for_wait(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Option<serde_json::Value>, ErrorResponse> {
        match self {
            Self::AgentStatusChanged(subscription) => Ok(subscription
                .poll_result(api_tx, event_hub)?
                .and_then(|event| serde_json::to_value(event).ok())),
            _ => Ok(self.poll(api_tx, event_hub)),
        }
    }

    pub(super) fn poll_batch(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Vec<serde_json::Value>, ErrorBody> {
        match self {
            Self::Event(subscription) => {
                let events = subscription_events_after(event_hub, subscription.last_sequence)?;
                let mut matching = Vec::new();
                for (sequence, event) in events {
                    subscription.last_sequence = sequence;
                    if event.event == subscription.event_kind {
                        matching.push(serde_json::to_value(event).map_err(event_encoding_error)?);
                    }
                }
                Ok(matching)
            }
            Self::AgentStatusChanged(subscription) => {
                let events = subscription_events_after(event_hub, subscription.last_sequence)?;
                let mut matching = Vec::new();
                for (sequence, event) in events {
                    subscription.last_sequence = sequence;
                    if let Some(event) = subscription.event_from_history(event) {
                        matching.push(serde_json::to_value(event).map_err(event_encoding_error)?);
                    }
                }
                if matching.is_empty() {
                    if let Some(event) =
                        subscription.poll_snapshot(api_tx, event_hub).ok().flatten()
                    {
                        matching.push(serde_json::to_value(event).map_err(event_encoding_error)?);
                    }
                }
                Ok(matching)
            }
            // These subscriptions sample current state, not retained event history.
            // Keep their existing cadence even when a lifecycle batch was nonempty.
            Self::OutputMatched(_) | Self::ScrollChanged(_) => {
                Ok(self.poll(api_tx, event_hub).into_iter().collect())
            }
            Self::OutputStream(_) => Ok(self.poll(api_tx, event_hub).into_iter().collect()),
        }
    }
}

fn subscription_events_after(
    event_hub: &EventHub,
    sequence: u64,
) -> Result<Vec<(u64, crate::api::schema::EventEnvelope)>, ErrorBody> {
    event_hub.events_after_checked(sequence).map_err(|error| match error {
        EventHistoryError::Lost => ErrorBody {
            code: "events_lost".into(),
            message: "event subscription fell behind retained history; resubscribe and resync with session.snapshot".into(),
        },
        EventHistoryError::Unavailable => ErrorBody {
            code: "server_unavailable".into(),
            message: "event history is unavailable".into(),
        },
    })
}

fn event_encoding_error(error: serde_json::Error) -> ErrorBody {
    ErrorBody {
        code: "internal_error".into(),
        message: format!("failed to encode subscription event: {error}"),
    }
}

impl ActiveEventSubscription {
    fn poll(&mut self, event_hub: &EventHub) -> Option<serde_json::Value> {
        for (sequence, event) in event_hub.events_after(self.last_sequence) {
            self.last_sequence = sequence;
            if event.event == self.event_kind {
                return serde_json::to_value(event).ok();
            }
        }
        None
    }
}

impl ActiveOutputMatchedSubscription {
    fn poll(&mut self, api_tx: &ApiRequestSender) -> Option<SubscriptionEventEnvelope> {
        let read = pane_read(
            format!("{}:read", self.request_prefix),
            &self.pane_id,
            output_match_read_source(&self.source),
            self.lines,
            crate::api::schema::ReadFormat::Text,
            self.strip_ansi,
            api_tx,
        )
        .ok()?;

        let matched_line = match_output(&read.text, &self.matcher, self.regex.as_ref());
        match matched_line {
            Some(matched_line) => {
                if self.currently_matching {
                    return None;
                }
                self.currently_matching = true;
                Some(SubscriptionEventEnvelope {
                    event: SubscriptionEventKind::PaneOutputMatched,
                    data: SubscriptionEventData::PaneOutputMatched(PaneOutputMatchedEvent {
                        pane_id: read.pane_id.clone(),
                        matched_line,
                        read,
                    }),
                })
            }
            None => {
                self.currently_matching = false;
                None
            }
        }
    }
}

impl ActiveAgentStatusChangedSubscription {
    fn poll(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Option<SubscriptionEventEnvelope> {
        self.poll_result(api_tx, event_hub).ok().flatten()
    }

    fn poll_result(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Option<SubscriptionEventEnvelope>, ErrorResponse> {
        for (sequence, event) in event_hub.events_after(self.last_sequence) {
            self.last_sequence = sequence;
            if let Some(event) = self.event_from_history(event) {
                return Ok(Some(event));
            }
        }

        self.poll_snapshot(api_tx, event_hub)
    }

    fn event_from_history(
        &mut self,
        event: crate::api::schema::EventEnvelope,
    ) -> Option<SubscriptionEventEnvelope> {
        if event.event != EventKind::PaneAgentStatusChanged {
            return None;
        }
        let crate::api::schema::EventData::PaneAgentStatusChanged {
            pane_id,
            workspace_id,
            agent_status,
            agent,
            title,
            display_agent,
            state_labels,
        } = event.data
        else {
            return None;
        };
        if pane_id != self.pane_id {
            return None;
        }
        self.last_status = Some(agent_status);
        self.last_presentation = Some(PanePresentationSnapshot::from_event(
            &title,
            &display_agent,
            &state_labels,
        ));
        self.initial_event = None;
        if self
            .status_filter
            .is_some_and(|wanted| wanted != agent_status)
        {
            return None;
        }

        Some(SubscriptionEventEnvelope {
            event: SubscriptionEventKind::PaneAgentStatusChanged,
            data: SubscriptionEventData::PaneAgentStatusChanged(PaneAgentStatusChangedEvent {
                pane_id,
                workspace_id,
                agent_status,
                agent,
                title,
                display_agent,
                state_labels,
            }),
        })
    }

    fn poll_snapshot(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Result<Option<SubscriptionEventEnvelope>, ErrorResponse> {
        if event_hub.current_sequence() != self.last_sequence {
            return Ok(None);
        } else if let Some(event) = self.initial_event.take() {
            return Ok(Some(SubscriptionEventEnvelope {
                event: SubscriptionEventKind::PaneAgentStatusChanged,
                data: SubscriptionEventData::PaneAgentStatusChanged(event),
            }));
        }

        let before_snapshot_sequence = self.last_sequence;
        let pane = pane_get(
            format!("{}:pane", self.request_prefix),
            &self.pane_id,
            api_tx,
        );
        let after_snapshot_sequence = event_hub.current_sequence();
        if after_snapshot_sequence != before_snapshot_sequence {
            return Ok(None);
        }
        let pane = pane?;

        let event = self.event_from_snapshot(pane);
        if event.is_some() {
            self.last_sequence = after_snapshot_sequence;
        }
        Ok(event)
    }

    fn event_from_snapshot(
        &mut self,
        pane: crate::api::schema::PaneInfo,
    ) -> Option<SubscriptionEventEnvelope> {
        let current_status = pane.agent_status;
        let current_presentation = PanePresentationSnapshot::from(&pane);
        let previous_status = self.last_status.replace(current_status);
        let previous_presentation = self.last_presentation.replace(current_presentation.clone());
        let presentation_changed = previous_presentation
            .as_ref()
            .is_some_and(|previous| previous != &current_presentation);
        let status_changed = previous_status.is_some_and(|previous| previous != current_status);
        if !(status_changed || presentation_changed) {
            return None;
        }
        if self
            .status_filter
            .is_some_and(|wanted| wanted != current_status)
        {
            return None;
        }

        Some(SubscriptionEventEnvelope {
            event: SubscriptionEventKind::PaneAgentStatusChanged,
            data: SubscriptionEventData::PaneAgentStatusChanged(PaneAgentStatusChangedEvent {
                pane_id: pane.pane_id,
                workspace_id: pane.workspace_id,
                agent_status: current_status,
                agent: pane.agent,
                title: pane.title,
                display_agent: pane.display_agent,
                state_labels: pane.state_labels,
            }),
        })
    }
}

impl ActiveOutputStreamSubscription {
    fn poll(
        &mut self,
        api_tx: &ApiRequestSender,
        event_hub: &EventHub,
    ) -> Option<SubscriptionEventEnvelope> {
        if let Some(event) = self.initial_event.take() {
            return Some(self.publish(event, event_hub));
        }
        if let Some(event) = self.replayed.pop_front() {
            return Some(event);
        }

        let read = pane_read(
            format!("{}:read", self.request_prefix),
            &self.pane_id,
            self.source,
            Some(self.lines),
            self.format,
            self.strip_ansi,
            api_tx,
        )
        .ok()?;
        if read.text == self.sent {
            return None;
        }

        let previous = std::mem::replace(&mut self.sent, read.text.clone());
        let (phase, text) = output_stream_delta(&previous, &read.text);
        if text.is_empty() {
            return None;
        }

        // `read.truncated` describes the read window, not the chunk. Only a
        // whole-window phase (`snapshot`/`resync`) hands the client text that the
        // line cap could have clipped; a delta is complete relative to what the
        // client already holds, so it never reports a truncation it did not have.
        let truncated = !matches!(phase, PaneOutputStreamPhase::Delta) && read.truncated;

        Some(self.publish(
            PaneOutputStreamEvent {
                pane_id: self.pane_id.clone(),
                workspace_id: self.workspace_id.clone(),
                sequence: 0,
                phase,
                text: text.to_string(),
                truncated,
            },
            event_hub,
        ))
    }

    /// Publishes a chunk on the pane's retained stream and wraps it for the socket.
    fn publish(
        &self,
        event: PaneOutputStreamEvent,
        event_hub: &EventHub,
    ) -> SubscriptionEventEnvelope {
        let event = event_hub.output_stream_push(&self.pane_id, event);
        SubscriptionEventEnvelope {
            event: SubscriptionEventKind::PaneOutputStream,
            data: SubscriptionEventData::PaneOutputStream(event),
        }
    }
}

impl ActiveScrollChangedSubscription {
    fn poll(&mut self, api_tx: &ApiRequestSender) -> Option<SubscriptionEventEnvelope> {
        let pane = pane_get(
            format!("{}:pane", self.request_prefix),
            &self.pane_id,
            api_tx,
        )
        .ok()?;
        self.event_from_snapshot(pane)
    }

    fn event_from_snapshot(
        &mut self,
        pane: crate::api::schema::PaneInfo,
    ) -> Option<SubscriptionEventEnvelope> {
        let scroll = pane.scroll;
        if self.last_scroll == scroll {
            return None;
        }
        self.last_scroll = scroll;
        let scroll = scroll?;

        Some(SubscriptionEventEnvelope {
            event: SubscriptionEventKind::ScrollChanged,
            data: SubscriptionEventData::ScrollChanged(PaneScrollChangedEvent {
                pane_id: pane.pane_id,
                workspace_id: pane.workspace_id,
                scroll,
            }),
        })
    }
}

fn pane_read(
    request_id: String,
    pane_id: &str,
    source: crate::api::schema::ReadSource,
    lines: Option<u32>,
    format: crate::api::schema::ReadFormat,
    strip_ansi: bool,
    api_tx: &ApiRequestSender,
) -> Result<crate::api::schema::PaneReadResult, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: request_id.clone(),
            method: Method::PaneRead(crate::api::schema::PaneReadParams {
                pane_id: pane_id.to_string(),
                source,
                lines,
                format,
                strip_ansi,
                intent: crate::api::schema::ReadIntent::Passive,
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|_| ErrorResponse {
        id: request_id.clone(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane read response".into(),
        },
    })?;
    if value.get("error").is_some() {
        return serde_json::from_value(value).map_err(|_| ErrorResponse {
            id: request_id,
            error: ErrorBody {
                code: "internal_error".into(),
                message: "failed to decode pane read error".into(),
            },
        });
    }
    serde_json::from_value(value["result"]["read"].clone()).map_err(|_| ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane read result".into(),
        },
    })
}

fn pane_get(
    request_id: String,
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<crate::api::schema::PaneInfo, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: request_id.clone(),
            method: Method::PaneGet(crate::api::schema::PaneTarget {
                pane_id: pane_id.to_string(),
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|_| ErrorResponse {
        id: request_id.clone(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane get response".into(),
        },
    })?;
    if value.get("error").is_some() {
        let response =
            serde_json::from_value::<ErrorResponse>(value).map_err(|_| ErrorResponse {
                id: request_id,
                error: ErrorBody {
                    code: "internal_error".into(),
                    message: "failed to decode pane get error".into(),
                },
            })?;
        return Err(response);
    }
    serde_json::from_value(value["result"]["pane"].clone()).map_err(|_| ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode pane get result".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::api::schema::{AgentStatus, EventData, EventEnvelope, EventKind, PaneInfo};

    #[test]
    fn stream_delta_reports_the_whole_first_read_as_a_snapshot() {
        let (phase, text) = output_stream_delta("", "hello\n");
        assert_eq!(phase, PaneOutputStreamPhase::Snapshot);
        assert_eq!(text, "hello\n");
    }

    #[test]
    fn stream_delta_reports_only_appended_output() {
        let (phase, text) = output_stream_delta("hello\n", "hello\nworld\n");
        assert_eq!(phase, PaneOutputStreamPhase::Delta);
        assert_eq!(text, "world\n");
    }

    #[test]
    fn stream_delta_skips_lines_that_slid_out_of_the_read_window() {
        // The pane scrolled: the first lines of the previous window are gone.
        let previous = "one\ntwo\nthree\n";
        let next = "three\nfour\n";
        let (phase, text) = output_stream_delta(previous, next);
        assert_eq!(phase, PaneOutputStreamPhase::Delta);
        assert_eq!(text, "four\n");
    }

    #[test]
    fn stream_delta_resyncs_when_the_client_text_is_gone() {
        // A clear-screen leaves nothing the client can stitch onto.
        let (phase, text) = output_stream_delta("one\ntwo\n", "\u{1b}[2J\u{1b}[Hnew\n");
        assert_eq!(phase, PaneOutputStreamPhase::Resync);
        assert_eq!(text, "\u{1b}[2J\u{1b}[Hnew\n");
    }

    #[test]
    fn stream_delta_never_splits_a_multi_byte_character() {
        // "é" is two bytes; splitting it would emit invalid UTF-8.
        let (phase, text) = output_stream_delta("caf\u{e9}\n", "caf\u{e9}\nna\u{ef}ve\n");
        assert_eq!(phase, PaneOutputStreamPhase::Delta);
        assert_eq!(text, "na\u{ef}ve\n");
    }

    #[test]
    fn stream_delta_resyncs_instead_of_talking_through_a_full_repaint() {
        let previous = "a".repeat(4096);
        let next = "b".repeat(4096);
        let (phase, text) = output_stream_delta(&previous, &next);
        assert_eq!(phase, PaneOutputStreamPhase::Resync);
        assert_eq!(text, next);
    }

    fn presentation_event(title: Option<&str>) -> EventEnvelope {
        EventEnvelope {
            event: EventKind::PaneAgentStatusChanged,
            data: EventData::PaneAgentStatusChanged {
                pane_id: "pane_1".into(),
                workspace_id: "workspace_1".into(),
                agent_status: AgentStatus::Working,
                agent: Some("pi".into()),
                title: title.map(str::to_string),
                display_agent: None,
                state_labels: HashMap::new(),
            },
        }
    }

    fn workspace_focused_event(workspace_id: &str) -> EventEnvelope {
        EventEnvelope {
            event: EventKind::WorkspaceFocused,
            data: EventData::WorkspaceFocused {
                workspace_id: workspace_id.into(),
            },
        }
    }

    fn tab_focused_event(tab_id: &str) -> EventEnvelope {
        EventEnvelope {
            event: EventKind::TabFocused,
            data: EventData::TabFocused {
                tab_id: tab_id.into(),
                workspace_id: "workspace_1".into(),
            },
        }
    }

    fn pane_focused_event(pane_id: &str) -> EventEnvelope {
        EventEnvelope {
            event: EventKind::PaneFocused,
            data: EventData::PaneFocused {
                pane_id: pane_id.into(),
                workspace_id: "workspace_1".into(),
            },
        }
    }

    fn pane_info_with_scroll(scroll: Option<PaneScrollInfo>) -> PaneInfo {
        PaneInfo {
            node_id: "test-node".into(),
            pane_id: "pane_1".into(),
            terminal_id: "terminal_1".into(),
            workspace_id: "workspace_1".into(),
            tab_id: "tab_1".into(),
            focused: true,
            cwd: None,
            foreground_cwd: None,
            restore_error: None,
            label: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            display_agent: None,
            agent_status: AgentStatus::Unknown,
            state_labels: HashMap::new(),
            tokens: HashMap::new(),
            agent_session: None,
            scroll,
            revision: 0,
        }
    }

    #[test]
    fn lifecycle_subscription_skips_history_but_keeps_setup_window_events() {
        let event_hub = EventHub::default();
        event_hub.push(workspace_focused_event("before_subscription"));
        let event_start_sequence = event_hub.current_sequence();
        event_hub.push(workspace_focused_event("during_setup"));

        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subscription = ActiveSubscription::new(
            Subscription::WorkspaceFocused {},
            "test",
            0,
            &api_tx,
            &event_hub,
            event_start_sequence,
        )
        .expect("workspace focus subscription");

        let setup_event = subscription
            .poll(&api_tx, &event_hub)
            .expect("setup-window event");
        assert_eq!(setup_event["data"]["workspace_id"], "during_setup");
        assert!(subscription.poll(&api_tx, &event_hub).is_none());

        event_hub.push(workspace_focused_event("after_setup"));
        let live_event = subscription.poll(&api_tx, &event_hub).expect("live event");
        assert_eq!(live_event["data"]["workspace_id"], "after_setup");
    }

    #[test]
    fn fresh_lifecycle_subscription_receives_no_retained_events_only_live() {
        let event_hub = EventHub::default();
        // Retained history: a mix of lifecycle events, including the subscribed
        // kind. A plugin subscribing to pane.* should not be replayed any of
        // these when a fresh events.subscribe stream starts.
        event_hub.push(workspace_focused_event("retained_workspace"));
        event_hub.push(tab_focused_event("retained_tab"));
        event_hub.push(pane_focused_event("retained_pane"));

        // Mirror the real events.subscribe handler (src/api/server.rs): the
        // live cursor is captured at subscribe time, not at zero.
        let event_start_sequence = event_hub.current_sequence();

        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subscription = ActiveSubscription::new(
            Subscription::PaneFocused {},
            "test",
            0,
            &api_tx,
            &event_hub,
            event_start_sequence,
        )
        .expect("pane focused subscription");

        // No retained event is replayed, including the retained pane.focused.
        assert!(
            subscription.poll(&api_tx, &event_hub).is_none(),
            "fresh subscription must not replay retained history"
        );

        // A matching event emitted after the subscription starts is delivered
        // live, and only then.
        event_hub.push(pane_focused_event("live_pane"));
        let live_event = subscription.poll(&api_tx, &event_hub).expect("live event");
        assert_eq!(live_event["event"], "pane_focused");
        assert_eq!(live_event["data"]["pane_id"], "live_pane");
    }

    #[test]
    fn workspace_metadata_subscription_uses_dedicated_event_kind() {
        let event_hub = EventHub::default();
        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let subscription = ActiveSubscription::new(
            Subscription::WorkspaceMetadataUpdated {},
            "test",
            0,
            &api_tx,
            &event_hub,
            event_hub.current_sequence(),
        )
        .expect("workspace metadata subscription");

        assert!(matches!(
            subscription,
            ActiveSubscription::Event(ActiveEventSubscription {
                event_kind: EventKind::WorkspaceMetadataUpdated,
                ..
            })
        ));
    }

    #[test]
    fn lifecycle_batch_drains_in_order_and_advances_past_unmatched_events() {
        let event_hub = EventHub::default();
        event_hub.push(workspace_focused_event("old"));
        let start = event_hub.current_sequence();
        event_hub.push(workspace_focused_event("setup"));
        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subscription = ActiveSubscription::new(
            Subscription::WorkspaceFocused {},
            "batch",
            0,
            &api_tx,
            &event_hub,
            start,
        )
        .unwrap();
        event_hub.push(presentation_event(None));
        event_hub.push(workspace_focused_event("live"));
        let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["data"]["workspace_id"], "setup");
        assert_eq!(events[1]["data"]["workspace_id"], "live");
        assert!(subscription
            .poll_batch(&api_tx, &event_hub)
            .unwrap()
            .is_empty());
        let ActiveSubscription::Event(subscription) = subscription else {
            panic!("expected lifecycle subscription");
        };
        assert_eq!(subscription.last_sequence, event_hub.current_sequence());
    }

    #[test]
    fn agent_status_batch_preserves_transitions_filters_and_initial_state_ordering() {
        for filtered in [false, true] {
            let event_hub = EventHub::default();
            let mut subscription = ActiveSubscription::AgentStatusChanged(Box::new(
                ActiveAgentStatusChangedSubscription {
                    pane_id: "pane_1".into(),
                    status_filter: filtered.then_some(AgentStatus::Working),
                    last_status: Some(AgentStatus::Working),
                    last_presentation: None,
                    last_sequence: event_hub.current_sequence(),
                    initial_event: Some(PaneAgentStatusChangedEvent {
                        pane_id: "pane_1".into(),
                        workspace_id: "workspace_1".into(),
                        agent_status: AgentStatus::Working,
                        agent: Some("pi".into()),
                        title: Some("stale initial snapshot".into()),
                        display_agent: None,
                        state_labels: HashMap::new(),
                    }),
                    request_prefix: "batch".into(),
                },
            ));
            for (status, title) in [
                (AgentStatus::Working, "started"),
                (AgentStatus::Blocked, "approval"),
                (AgentStatus::Idle, "finished"),
                (AgentStatus::Working, "restarted"),
            ] {
                let mut event = presentation_event(Some(title));
                let EventData::PaneAgentStatusChanged { agent_status, .. } = &mut event.data else {
                    panic!("expected status data");
                };
                *agent_status = status;
                event_hub.push(event);
            }
            let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
            let events = subscription.poll_batch(&api_tx, &event_hub).unwrap();
            let titles = events
                .iter()
                .map(|event| event["data"]["title"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                titles,
                if filtered {
                    vec!["started", "restarted"]
                } else {
                    vec!["started", "approval", "finished", "restarted"]
                }
            );
            let ActiveSubscription::AgentStatusChanged(subscription) = subscription else {
                panic!("expected agent subscription");
            };
            assert_eq!(subscription.last_sequence, event_hub.current_sequence());
            assert!(subscription.initial_event.is_none());
        }
    }

    #[test]
    fn scroll_subscription_emits_when_scroll_snapshot_changes() {
        let at_bottom = PaneScrollInfo {
            offset_from_bottom: 0,
            max_offset_from_bottom: 40,
            viewport_rows: 20,
        };
        let scrolled_back = PaneScrollInfo {
            offset_from_bottom: 8,
            max_offset_from_bottom: 40,
            viewport_rows: 20,
        };
        let mut subscription = ActiveScrollChangedSubscription {
            pane_id: "pane_1".into(),
            last_scroll: Some(at_bottom),
            request_prefix: "test".into(),
        };

        assert!(subscription
            .event_from_snapshot(pane_info_with_scroll(Some(at_bottom)))
            .is_none());

        let event = subscription
            .event_from_snapshot(pane_info_with_scroll(Some(scrolled_back)))
            .expect("scroll event");
        assert_eq!(event.event, SubscriptionEventKind::ScrollChanged);
        let SubscriptionEventData::ScrollChanged(data) = event.data else {
            panic!("wrong event data");
        };
        assert_eq!(data.pane_id, "pane_1");
        assert_eq!(data.workspace_id, "workspace_1");
        assert_eq!(data.scroll, scrolled_back);
    }

    #[test]
    fn agent_status_subscription_replays_queued_metadata_set_and_expiry_events() {
        let event_hub = EventHub::default();
        let mut subscription = ActiveAgentStatusChangedSubscription {
            pane_id: "pane_1".into(),
            status_filter: None,
            last_status: Some(AgentStatus::Working),
            last_presentation: Some(PanePresentationSnapshot {
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            last_sequence: event_hub.current_sequence(),
            initial_event: None,
            request_prefix: "test".into(),
        };

        event_hub.push(presentation_event(Some("short lived")));
        event_hub.push(presentation_event(None));

        let set_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("set event");
        let SubscriptionEventData::PaneAgentStatusChanged(set_data) = set_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(set_data.title.as_deref(), Some("short lived"));

        let expiry_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("expiry event");
        let SubscriptionEventData::PaneAgentStatusChanged(expiry_data) = expiry_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(expiry_data.title, None);
    }

    #[test]
    fn agent_status_subscription_prefers_setup_window_events_over_initial_snapshot() {
        let event_hub = EventHub::default();
        let mut subscription = ActiveAgentStatusChangedSubscription {
            pane_id: "pane_1".into(),
            status_filter: Some(AgentStatus::Working),
            last_status: Some(AgentStatus::Working),
            last_presentation: Some(PanePresentationSnapshot {
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            last_sequence: event_hub.current_sequence(),
            initial_event: Some(PaneAgentStatusChangedEvent {
                pane_id: "pane_1".into(),
                workspace_id: "workspace_1".into(),
                agent_status: AgentStatus::Working,
                agent: Some("pi".into()),
                title: None,
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            request_prefix: "test".into(),
        };

        event_hub.push(presentation_event(Some("short lived")));
        event_hub.push(presentation_event(None));

        let set_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("set event");
        let SubscriptionEventData::PaneAgentStatusChanged(set_data) = set_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(set_data.title.as_deref(), Some("short lived"));

        let expiry_event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("expiry event");
        let SubscriptionEventData::PaneAgentStatusChanged(expiry_data) = expiry_event.data else {
            panic!("wrong event data");
        };
        assert_eq!(expiry_data.title, None);
    }

    #[test]
    fn agent_status_subscription_emits_setup_window_event_already_reflected_by_probe() {
        let event_hub = EventHub::default();
        let mut subscription = ActiveAgentStatusChangedSubscription {
            pane_id: "pane_1".into(),
            status_filter: Some(AgentStatus::Working),
            last_status: Some(AgentStatus::Working),
            last_presentation: Some(PanePresentationSnapshot {
                title: Some("short lived".into()),
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            last_sequence: event_hub.current_sequence(),
            initial_event: Some(PaneAgentStatusChangedEvent {
                pane_id: "pane_1".into(),
                workspace_id: "workspace_1".into(),
                agent_status: AgentStatus::Working,
                agent: Some("pi".into()),
                title: Some("short lived".into()),
                display_agent: None,
                state_labels: HashMap::new(),
            }),
            request_prefix: "test".into(),
        };

        event_hub.push(presentation_event(Some("short lived")));

        let event = subscription
            .poll(&tokio::sync::mpsc::unbounded_channel().0, &event_hub)
            .expect("setup-window event");
        let SubscriptionEventData::PaneAgentStatusChanged(data) = event.data else {
            panic!("wrong event data");
        };
        assert_eq!(data.title.as_deref(), Some("short lived"));
        assert!(subscription.initial_event.is_none());
    }
}
