# Architecture resources

Status: the native Channel and DataStore resource slice is implemented. Its
[contract reference](../reference/channel-and-datastore-contracts.md) describes
the supported fields and lifecycle. The
[acceptance fixture](../reference/channel-and-datastore-acceptance.md) links the
executable coverage and records its verification boundaries.

## Start with the behavioral model

Architecture resources describe the same model's transport, persistence,
contracts and responsibility. They build on Event Modeling's existing causal
structure. Integration input is a ReadModel. An AutomationSlice observes that
ReadModel through a Processor, which can issue a local Command. A Channel
describes the selected observation. A DataStore describes storage attached to
an exact ReadModel.

Keeping those anchors explicit prevents a transport label from introducing an
additional behavioral grammar. Broker delivery claims do not establish
end-to-end processing guarantees, and a storage attachment does not establish
which component reads or writes the store.

## Connection identity is the implemented foundation

[Connection identity](../reference/connection-identity.md) already defines
persisted IDs on explicit typed edges and navigation/lifecycle occurrences.
An exact owner reference, its incarnation UID and an occurrence ID identify a
connection. Reorder preserves identity; retargeting creates a new occurrence.
Ordinary writes use bounded active state without a deleted-ID ledger.

That foundation does not identify every reference in the model. Raw reference
lists, repeated storyboard moments, additional architecture associations and
the fitness host's reference adapter still need their own supported representations.
Implementing a new association requires extending reference discovery,
validation and lifecycle handling together.

## Deliver a connected resource slice

Channel and DataStore include typed values, identified associations and nested
reference handling. The acceptance fixture covers repeated observations, exact
versions, reference discovery, guarded mutations and branch behavior.

Implemented scope:

- [x] Add entity kinds, typed references and generated bindings.
- [x] Admit writes and resolve associations through the authorized backend.
- [x] Index nested references for queries, reverse references and impact.
- [x] Preserve identity through edit, copy, retarget and branch operations.
- [x] Exercise the connected fixture through public service and CLI/MCP paths.
- [x] Expose the resources and diagnostics in Studio's review surfaces.

Runtime evidence comes from the linked executable tests. Component review
coverage does not establish deployed-browser or production readiness.

## Follow-on contracts

Specification/API resources need explicit operation, message and schema-member
identity, with request and response roles kept separate. Team and Person need
exact-version versus family responsibility, retained directory data and a clear
separation from authorization principals. DataProduct needs ReadModel output
ports, lineage and selected access paths tied to the same output.

Those resources also need per-relation cycle rules, cross-namespace permissions
and deletion/copy/merge semantics. They should extend the connected fixture as
they become implementable rather than silently inheriting undefined behavior.

Fitness functions use the WebAssembly Component Model and WIT. Their remaining
contracts include the authorized snapshot projection, lossless entity and exact
connection subjects, component/configuration storage, registration,
run/cancel/report operations, retention, budget profiles and CI failure policy.
The connection-ID foundation alone does not provide that host or its adapter.

Source import, synchronization, export and additional schema-format validation
are later milestones with explicit acceptance criteria. Native Channel and
DataStore support does not require a source connector, a source overlay,
Specification resources or format-specific compatibility checking.
