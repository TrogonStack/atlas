# Documentation

Start with the [Atlas principles](explanation/principles/README.md) for the
shared direction behind the product, its modeling language, and its codebase.

## Tutorials

Learn by doing:

- [Run Atlas locally with Docker Compose](tutorials/run-atlas-locally.md)

## How-to guides

Solve a specific problem:

- [Assess the blast radius of a change](how-to/assess-change-impact.md)
- [Check code against the model](how-to/check-code-drift.md)
- [Declare an SLO](how-to/declare-an-slo.md)
- [Draft and land a new entity version](how-to/draft-then-land.md)
- [Issue an API key to a client](how-to/issue-an-api-key.md)
- [Migrate legacy entity fields into `schema`](how-to/migrate-legacy-entity-fields.md)
- [Model runtime code as an Event Model](how-to/model-runtime-code-as-an-event-model.md)
- [Register a type library](how-to/register-a-type-library.md)
- [Repair a validation finding](how-to/repair-validation-findings.md)
- [Replan after a stale refusal](how-to/replan-after-a-stale-refusal.md)
- [Ship a breaking proto change](how-to/ship-a-breaking-proto-change.md)
- [Start an event model from scratch](how-to/start-a-model-from-scratch.md)
- [Track a version migration](how-to/track-a-migration.md)
- [Verify a connected architecture graph](how-to/verify-connected-architecture.md)

## Reference

Look up exact behavior:

- [Failure reasons](reference/failure-reasons.md)
- [Connection identity](reference/connection-identity.md)
- [Reliability validation rules](reference/reliability-rules.md)
- [`trogon-atlas` exit codes](reference/trogon-atlas-exit-codes.md)

Resource contracts and acceptance:

- [Architecture resource coverage](reference/architecture-resource-coverage.md)
- [Domain-scoped listing](reference/domain-scoped-listing.md)
- [System relationship contracts](reference/system-relationship-contracts.md)
- [System relationship acceptance requirements](reference/system-relationship-acceptance.md)
- [Communication intent contracts](reference/communication-intent-contracts.md)
- [Communication intent acceptance requirements](reference/communication-intent-acceptance.md)
- [Human interaction contracts (proposed)](reference/human-interaction-contracts.md)
- [Human interaction acceptance requirements (proposed)](reference/human-interaction-acceptance.md)
- [Channel and DataStore contracts](reference/channel-and-datastore-contracts.md)
- [Channel and DataStore acceptance fixture](reference/channel-and-datastore-acceptance.md)
- [Specification and API contracts](reference/specification-and-api-contracts.md)
- [Specification and API acceptance criteria](reference/specification-and-api-acceptance.md)
- [DataProduct contracts](reference/data-product-contracts.md)
- [DataProduct acceptance criteria](reference/data-product-acceptance.md)
- [Responsibility contracts](reference/responsibility-contracts.md)
- [Responsibility acceptance criteria](reference/responsibility-acceptance.md)
- [Decision and Document contracts](reference/decision-and-document-contracts.md)
- [Decision and Document acceptance requirements](reference/decision-and-document-acceptance.md)
- [Supplied source bundle profile](reference/source-bundle-profile.md)
- [Source integration contracts](reference/source-integration-contracts.md)
- [Source control API](reference/source-control-api.md)
- [Source integration acceptance requirements](reference/source-integration-acceptance.md)

## Explanation

Understand how Atlas works and why:

- [Atlas principles](explanation/principles/README.md)
- [Architecture resource roadmap](explanation/architecture-resource-roadmap.md)
- [Authorization design](explanation/authorization.md)
- [Board renderer performance](explanation/board-renderer-performance.md)
- [Branching](explanation/branching.md)
- [Changesets](explanation/changesets.md)
- [Entity protection](explanation/entity-protection.md)
- [Reliability overlays](explanation/reliability-overlays.md)
- [Single-writer fencing](explanation/single-writer.md)
- [Snapshot identity](explanation/snapshot-identity.md)
- [Stale plans](explanation/stale-plans.md)
- [Where tenant types live](explanation/tenant-types.md)
- [Validation coverage](explanation/validation-coverage.md)
- [Validation repair patterns](explanation/validation-repair-patterns.md)
- [Wire compatibility and mixed-version operation](explanation/wire-compatibility.md)

Mapping existing code into an Event Model:

- [Translation layers](explanation/translation-layers.md)
- [Temporal and Event Modeling](explanation/temporal-and-event-modeling.md)
- [gRPC handler is not a command](explanation/grpc-handler-is-not-a-command.md)
- [Pub/sub message is not an event](explanation/pubsub-message-is-not-an-event.md)
