# Bug Fix: events.subscribe replays retained historical events instead of live-only

- Slug: events-subscribe-replays-retained-historical-events
- Fixed: 2026-08-23
- Assessment: ./assessment.md
- Status: applied

## Summary

A fresh `events.subscribe` stream is now anchored to the event-hub sequence at
the moment the client subscribes, so it only forwards lifecycle events emitted
**after** that point. It no longer replays the retained event buffer (the last
512 historical `workspace.*` / `tab.*` / `pane.*` events) to every new
subscriber.

The retained-history replay was a property of the subscription cursor, not of a
dedicated "replay" code path. Each lifecycle `Event` subscription holds a
`last_sequence` cursor and a poll loop that calls
`event_hub.events_after(self.last_sequence)` (see
`src/api/subscriptions.rs:283-291`). `events_after` returns every stored event
whose sequence is **strictly greater** than the cursor, so the cursor value
alone decides what is replayed:

- Cursor seeded at `0` ⇒ `events_after(0)` returns the **entire** retained
  buffer — i.e. every historical matching event (`workspace.*`, `tab.*`,
  `pane.*`) is re-sent the moment a plugin subscribes. This is the reported
  bug.
- Cursor seeded at `event_hub.current_sequence()` at subscribe time ⇒
  `events_after(current)` returns only events emitted **after** subscribe
  (sequence strictly greater than the current high-water mark); all retained
  history has sequence `<=` current and is excluded.

The live-only semantics are already enforced in this fork by the
`event_start_sequence` capture at the top of `stream_subscriptions`
(`src/api/server.rs:697`), threaded through `ActiveSubscription::new` into
`ActiveEventSubscription.last_sequence` (`src/api/subscriptions.rs:113-118`).
That capture was present as an ancestor of `development` (upstream
`herdrdev/herdr#3134`, fork commit `20a500a7`); the retained-history replay it
prevents is therefore the behavior described in issue #10, and it does **not**
reproduce in the current tree.

This change locks the live-only contract with an explicit multi-kind regression
test (the reported scenario: a mix of retained `workspace.*` / `tab.*` /
`pane.*` events, including the subscribed kind, followed by a fresh
`pane.focused` subscription that must receive none of them and only the
post-subscribe event), and documents the cursor's intent with contract
comments at both sites.

The protocol carries no replay/cursor parameter today (`EventsSubscribeParams`
only has `subscriptions`, see `src/api/schema/events.rs:11-14`), so the fix is
limited to suppressing the implicit replay. Explicit replay (a future
`replay_from` / cursor field) is left as follow-up work.

## Changes

| File | Change | Notes |
| --- | --- | --- |
| `src/api/server.rs:697-703` | Added a contract comment on the `event_start_sequence` capture in `stream_subscriptions` | Documents that the cursor is captured at subscribe time to suppress retained-history replay; explicit replay noted as out of scope |
| `src/api/subscriptions.rs:94-99` | Added a doc comment on `ActiveEventSubscription.last_sequence` | Documents the field as the live cursor seeded at subscribe time |
| `src/api/subscriptions.rs:605-629` | Added `tab_focused_event` / `pane_focused_event` test helpers | Mirror the existing `workspace_focused_event` helper |
| `src/api/subscriptions.rs:667-710` | Added `fresh_lifecycle_subscription_receives_no_retained_events_only_live` regression test | New test-only |

## Diff Highlights

```rust
// src/api/server.rs — stream_subscriptions
+// Capture the live cursor at subscribe time so the stream only delivers
+// events emitted after this point. Replaying the retained event buffer
+// (events with a sequence at or below the current one) would re-send stale
+// workspace/tab/pane lifecycle history to every new subscriber; explicit
+// replay requires a future cursor/replay parameter and is out of scope.
 let event_start_sequence = event_hub.current_sequence();

// src/api/subscriptions.rs — ActiveEventSubscription
 pub(super) struct ActiveEventSubscription {
     event_kind: crate::api::schema::EventKind,
+    /// Live cursor. Seeded with the event-hub sequence captured at subscribe
+    /// time, so the poll loop only yields events emitted after the
+    /// subscription started.
     last_sequence: u64,
 }
```

The regression test reproduces the issue at the subscription layer (mirroring
the real handler, which captures `event_start_sequence` before constructing the
subscription):

```rust
#[test]
fn fresh_lifecycle_subscription_receives_no_retained_events_only_live() {
    let event_hub = EventHub::default();
    event_hub.push(workspace_focused_event("retained_workspace"));
    event_hub.push(tab_focused_event("retained_tab"));
    event_hub.push(pane_focused_event("retained_pane"));

    let event_start_sequence = event_hub.current_sequence();

    let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut subscription = ActiveSubscription::new(
        Subscription::PaneFocused {}, "test", 0, &api_tx, &event_hub, event_start_sequence,
    ).expect("pane focused subscription");

    assert!(subscription.poll(&api_tx, &event_hub).is_none(),
        "fresh subscription must not replay retained history");

    event_hub.push(pane_focused_event("live_pane"));
    let live_event = subscription.poll(&api_tx, &event_hub).expect("live event");
    assert_eq!(live_event["event"], "pane_focused");
    assert_eq!(live_event["data"]["pane_id"], "live_pane");
}
```

## Tests Added or Updated

- `src/api/subscriptions.rs` → `fresh_lifecycle_subscription_receives_no_retained_events_only_live`
  (new). Anchors a `pane.focused` subscription to the current sequence after a
  mix of retained `workspace.focused` / `tab.focused` / `pane.focused` events is
  in the buffer, asserts the first poll returns `None` (no retained event of any
  kind is replayed, including the retained `pane.focused`), then emits a new
  `pane.focused` and asserts it is delivered live. This is the direct guard for
  issue #10.
- Existing `lifecycle_subscription_skips_history_but_keeps_setup_window_events`
  continues to pass; it pins the intentional "setup window" behavior (events
  emitted between cursor capture and subscription creation are still delivered),
  which is distinct from retained-history replay and is preserved.

## Local Verification

```
export ZIG=/workspace/herdr/.cache/zig/zig-x86_64-linux-0.15.2/zig

cargo test --bin herdr subscriptions
  -> test result: ok. 10 passed; 0 failed
     (includes the new fresh_lifecycle_subscription_receives_no_retained_events_only_live)
```

The full `api::subscriptions::tests` module (7 tests) and the
`api::server::tests::subscriptions_*` streaming tests both pass. The only build
warning is a pre-existing unused `use super::*;` in `src/cli/agent.rs:953`,
unrelated to this change.

Not verified: an end-to-end socket reproduction with a live plugin client. The
behavior is exercised deterministically at the subscription layer, which is the
exact code path `stream_subscriptions` drives for the protocol `events.subscribe`
method.

## Deviations from Assessment

The assessment's `Symptom`, `Reproduction`, `Suspected Code Paths`, `Root Cause
Hypothesis`, `Proposed Remediation`, and `Open Questions` were all
`[NEEDS CLARIFICATION]`. Investigation found the retained-history replay path and
confirmed it is **already suppressed** in the current tree: the
`event_start_sequence = event_hub.current_sequence()` anchor (upstream
`herdrdev/herdr#3134`, present in this fork) prevents `events_after(0)`-style
replay. The delivered change is therefore regression coverage plus contract
documentation rather than a re-introduction of removed replay logic.

## Follow-ups

- **Explicit replay**: add an opt-in `replay_from` / cursor field to
  `EventsSubscribeParams` (`src/api/schema/events.rs`) so a client that *does*
  want history can request it; the retained buffer (`EventHub`,
  `src/api/event_hub.rs`, `MAX_EVENTS = 512`) already supports `events_after(seq)`.
- **Setup-window events**: the current design intentionally delivers events
  emitted between cursor capture and subscription creation (see
  `lifecycle_subscription_skips_history_but_keeps_setup_window_events`). This
  window is empty for the synchronous lifecycle `Event` path, but if a future
  async handler defers `stream_subscriptions`, that window could grow; consider
  capturing the cursor per-subscription if strict "after subscribe" delivery is
  ever required.
- **Plugin retirement**: the forklift use case described in the issue can now
  subscribe to `workspace.*` / `tab.*` / `pane.*` instead of polling
  `herdr_status.sh`; the tail-scrape removal is a forklift-side change, not part
  of this fix.
