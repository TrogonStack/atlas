# Architecture resources

Status: proposed resource contracts. Channel and DataStore are not yet entity
kinds accepted by the schema, server, CLI or MCP. The
[contract reference](../reference/channel-and-datastore-contracts.md) specifies
their first implementation boundary; the
[acceptance fixture](../reference/channel-and-datastore-acceptance.md) defines
the behavior that implementation must demonstrate.

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
lists, repeated storyboard moments, architecture associations and the fitness
host's reference adapter still need their own supported representations.
Implementing a new association requires extending reference discovery,
validation and lifecycle handling together.

## Deliver a connected resource slice

The next implementation includes Channel and DataStore together with their
typed values and reference handling. Its own contracts must close before
schema tags and generated bindings are introduced. The acceptance fixture
covers repeated observations, exact versions, reference discovery, guarded
mutations and branch behavior.

Implementation remains open:

- [ ] Add entity kinds, typed references and generated bindings.
- [ ] Admit writes and resolve associations through the authorized backend.
- [ ] Index nested references for queries, reverse references and impact.
- [ ] Preserve identity through edit, copy, retarget and branch operations.
- [ ] Exercise the connected fixture through public service and CLI/MCP paths.
- [ ] Expose the resources and diagnostics in Studio's review surfaces.

Completing the contract documents does not complete these implementation gates.

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
