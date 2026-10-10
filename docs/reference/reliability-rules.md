# Reliability validation rules

`ServiceLevelIndicator` and `ServiceLevelObjective` entities are checked by
`validate_model`, the same engine behind `validate_event_model` and the
per-model pass of `validate_project` (see
`docs/how-to/repair-validation-findings.md`). These checks only run for an
indicator or objective listed in the `EventModel`'s `members`; one that
exists in the store but is not a member of any `EventModel` is never
checked.

`list_validation_rules` is the live source of truth for titles, categories,
and default severities; this page exists so the trigger conditions read
without decoding the catalog from source.

| Code | Severity | Subject | Trigger |
| --- | --- | --- | --- |
| `SLI_CONNECTION_UNRESOLVED` | Error | `ServiceLevelIndicator.connection` | The connection points at a missing slice, the wrong slice kind for the `Connection` variant, events not on that slice's edges, or an integration system the processor does not call. |
| `SLI_JOURNEY_UNREACHABLE` | Error | `ServiceLevelIndicator.connection.journey` | A journey's end event cannot be reached from its start event within the journey's scope. |
| `SLI_CORRELATION_UNSOURCED` | Error | `ServiceLevelIndicator.connection.journey.correlate_on` | A journey's `correlate_on` field is absent from the start event's or the end event's field list. |
| `SLI_OUTCOME_UNREACHABLE` | Error | `ServiceLevelIndicator.measure.outcome_ratio` | An outcome-ratio measure's good or bad event cannot terminate the connection, or either side is empty. |
| `SLI_SIGNAL_MISMATCH` | Error | `ServiceLevelIndicator.signal` | The signal cannot measure the connection or the indicator has no signal or measure at all. |
| `SLO_THRESHOLD_MISMATCH` | Error | `ServiceLevelObjective.objectives` | An objective carries a threshold for a non-latency indicator, or carries none for a latency indicator. |
| `SLO_TARGET_RANGE` | Error | `ServiceLevelObjective.objectives` | An objective's target ratio is not strictly between 0 and 1. |
| `SLO_EXTERNAL_UNALERTED` | Warning | `ServiceLevelObjective.alert_policies` | An externally committed objective has no alert policy watching it. |
| `SLO_UNALERTED` | Info | `ServiceLevelObjective.alert_policies` | An objective with no commitment, or an internal one, has no alert policy watching it. |

## `SLI_CONNECTION_UNRESOLVED`

A `ServiceLevelIndicator`'s connection points at a slice that is missing or
is the wrong slice kind for the `Connection` variant, names events not on
that slice's edges (`CommandHandling.events` must be a subset of the
command slice's `emitted_events`; `Projection.source_events` must be a
subset of the read model slice's `source_events`), or names an integration
system the processor does not list in `calls`. The measurement has nowhere
real to attach.

## `SLI_JOURNEY_UNREACHABLE`

A journey connection's end event cannot be reached from its start event
within its scope (one storyboard, or the pooled slices of an `EventModel`'s
member storyboards). Reachability is structural, not storyboard order: an
edge exists from a source event to a command's emitted events when a read
model slice projects the source event and either an automation slice
observes that read model and emits the command (a reaction chain) or a UI
slice renders that read model for a persona who then issues the command
from the same UI (a human handoff on one screen). This deliberately does
not model a persona navigating to a different screen first, so a journey
that only a cross-screen handoff can complete is conservatively reported as
unreachable even though a human could still complete it by hand; narrow the
journey or add the missing UI-to-slice pairing to clear the finding.

## `SLI_CORRELATION_UNSOURCED`

A journey connection's `correlate_on` names a field absent from the
`schema` of the start event or the end event. Without the field on both
ends the two cannot be matched into one instance.

## `SLI_OUTCOME_UNREACHABLE`

An outcome-ratio measure's good or bad event is not among the events the
connection can end on: for command handling, the command slice's
`emitted_events`; for a journey, the end event or any event reachable from
the same start by `SLI_JOURNEY_UNREACHABLE`'s reachability rule. Also fires
when good or bad is empty, since a ratio with no numerator or no
denominator side measures nothing. Connection kinds with no defined notion
of termination (projection, reaction, display, integration) are not
checked.

## `SLI_SIGNAL_MISMATCH`

An event-timestamps signal needs an event marking the end of the span;
display and integration connections render a screen or call outward with
no such event, so event timestamps cannot source them. Also fires when the
indicator has no signal or no measure at all.

## `SLO_THRESHOLD_MISMATCH`

A `ServiceLevelObjective`'s objective must carry a threshold when its
indicator measures latency (the threshold says how slow is too slow) and
must not carry one otherwise, since a ratio or completion measure has
nothing to threshold.

## `SLO_TARGET_RANGE`

An objective's `target` or `time_slice_target` ratio must be strictly
between 0 and 1; 0 commits to nothing and 1 (or above) commits to
perfection, neither of which is a budget.

## `SLO_EXTERNAL_UNALERTED` and `SLO_UNALERTED`

Both report an objective with no `alert_policies` watching it; which one
fires depends on `commitment`. `SLO_EXTERNAL_UNALERTED` is a warning for an
objective with an `External` commitment, what a counterparty can hold the
team to, since nothing pages before that commitment is breached.
`SLO_UNALERTED` is informational for an objective with no commitment or an
`Internal` one. The two are kept separate so an externally committed
objective is not double-reported by both rules.
