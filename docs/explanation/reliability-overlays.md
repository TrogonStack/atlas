# Reliability overlays

`ServiceLevelIndicator`, `ServiceLevelObjective`, `AlertPolicy`, and
`AlertNotificationTarget` (`reliability.proto`) let the model state what a
slice or a journey must deliver and whether anyone gets paged when it
does not. Before these entities existed, that information lived in prose
or in a vendor tool disconnected from the graph: an SLO described as a
sentence in a runbook, or configured directly in a monitoring product,
has no way to prove it still measures something the model actually does.
An indicator that points at a `Connection` the validator can resolve
closes that gap: the objective is provably attached to a real edge, not a
description of one.

## Why a connection, not a metadata tag

An indicator could have been a free-text label on an edge, or a
`(from, to)` pair naming any two entities. `Connection` is neither. It is
a oneof over the kinds of edge the graph already has: `CommandHandling`
(a command slice's emitted events), `Projection` (a read model slice's
source events), `Reaction` (an automation slice), `Display` (a UI
slice), `Integration` (a processor's call to an external system), and
`Journey` (a start and end event correlated across a storyboard's
scope). Naming the edge kind instead of a generic pair means a
connection that does not exist in the model cannot be expressed; a typo'd
slice or an event not actually on that slice's edges is rejected as
`SLI_CONNECTION_UNRESOLVED` rather than silently measuring nothing. The
alternative, a free `(from, to)` pair, would let an SLI point at two
entities that are not actually connected, and the validator would have no
graph to check it against.

## Overlay posture

Reliability entities carry the same posture as `Tracker`: delivery status
is an opinion about the model, not a property of it, so it lives in a
denormalized entity that points at design entities rather than adding
status fields to them. The same reasoning applies here. A target, a burn
rate threshold, or an on-call rotation is an operational decision that
changes on its own schedule, not the schedule of the design it measures.
Retuning a target from 99% to 99.9%, or rotating a notification target
from one on-call tool to another, never touches the command slice or
storyboard underneath. If reliability fields lived directly on
`CommandSlice` or `Storyboard`, every retune would be a design-entity
mutation competing with actual design changes for the same revision
history.

## Signal is a pointer, not a vendor query

The schema models decisions and data, never implementation: nothing about
classes, services, or vendor APIs belongs in it, only a pointer out when
the model needs to name something outside itself (`Component.tech`,
`UI.design_ref`). `Signal` follows the same rule. It names where a
measurement comes from logically, `EventTimestamps` (the span between two
events already in the model) or `Telemetry` (a named span or metric by
string identifier), never a Datadog query, a PromQL expression, or a
vendor's metric ID. A schema that embedded vendor queries would go stale
every time a team changed monitoring vendors, and would make the model
unreadable to a team that uses a different one. The query itself belongs
to export configuration: the bindings file handed to `trogon-atlas openslo
export` maps a `Signal` to a `DataSource` and a set of per-role metric
queries, entirely outside the stored model. Changing vendors means
changing a bindings file, not the model.

## SLO or SLA

The schema does not have a separate SLA entity; the same
`ServiceLevelObjective` serves both, distinguished by `Commitment`. An
`Internal` commitment is an objective a team holds itself to with no
external party able to enforce it; an `External` commitment is what a
counterparty, a customer or a partner, can hold the team to, which is
what "SLA" usually means in practice. The distinction matters to
validation: an externally committed objective with no alert policy is a
warning (`SLO_EXTERNAL_UNALERTED`), since nothing pages before a
commitment that someone outside the team can enforce is breached, while
an internal or uncommitted objective with no alert policy is only
informational (`SLO_UNALERTED`). `SLO_EXTERNAL_UNALERTED` and
`SLO_UNALERTED` stay separate so one objective is never reported by both.

## Vendor-neutral export, not a vendor-specific one

`trogon-atlas openslo export` and the MCP `export_openslo` tool render the
model's reliability entities as OpenSLO v1, a vendor-neutral format,
rather than directly as a Datadog monitor or a Prometheus rule. OpenSLO
describes the same shape the model already has, service, indicator,
objective, alert policy, without committing the model itself to one
vendor's dialect. The bindings file is where a specific vendor enters:
it supplies the `DataSource` and the metric query template for each role
a measure needs (`threshold` for latency, `good`/`total` or `bad`/`total`
for a ratio or completion), so the same stored model can export against
different vendors by swapping the bindings file, never the model.

## Known limitations

Journey reachability is structural, not an attempt to model everything a
human could do. An edge exists from a source event to a command's
emitted events when a read model slice projects the source event and
either an automation slice observes that read model and emits the
command (a reaction chain), or a UI slice renders that read model for a
persona who then issues the command from the same UI (a human handoff on
one screen). It deliberately does not model a persona navigating to a
different screen first. A journey that only a cross-screen handoff can
complete is reported as unreachable (`SLI_JOURNEY_UNREACHABLE`) even
though a human could still complete it by hand; the fix is to narrow the
journey's scope or add the missing UI-to-slice pairing, not to treat the
finding as wrong.

Outcome-ratio measures are not checked against connection kinds with no
defined notion of termination: `Projection`, `Reaction`, `Display`, and
`Integration` connections can carry a `Latency` or `Completion` measure,
but `SLI_OUTCOME_UNREACHABLE` only applies where a connection actually
ends in events the model can enumerate, `CommandHandling`'s emitted
events or a `Journey`'s reachable events.

An indicator or objective only gets validated when it is listed as a
member of an `EventModel`; one that exists in the store but is not a
member of any `EventModel` is never checked (see
`docs/reference/reliability-rules.md` and
`docs/how-to/declare-an-slo.md`).
