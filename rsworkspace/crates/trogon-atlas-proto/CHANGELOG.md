# Changelog

## [Unreleased]

### Added
- `FailureModeAnnotation` added: documents an entity's (typically a UI's) behavior under a foreseeable failure mode (network disconnect, load timeout, stale/partial response): `trigger`, `behavior`, and optional `recovery`. Purely descriptive; no validator rule keys off it.
- `PersonalDataAnnotation` added: marks a FieldSpec as personal data, with a required `doc`, an `Erasure` strategy (crypto-shredding, external reference, projection delete, retention expiry), and an optional `subject_field` naming the sibling field that identifies the person. Backs new validator rules `PII_DOC_MISSING`, `PII_EVENT_ERASURE_UNSPECIFIED`, `PII_ERASURE_NOT_APPLICABLE`, `PII_SUBJECT_FIELD_UNKNOWN`, and `PII_FLOW_UNMARKED` (a downstream field inferred to come from a personal-data field but carrying no annotation of its own).
- `repeated google.protobuf.Any metadata` added to `CommandScenario`, `ReadModelScenario`, and `AutomationScenario`, so individual scenarios can carry annotations (e.g. `NoteAnnotation`, `OpenQuestionAnnotation`) instead of only the entity owning them.
- `AssumptionAnnotation` added: names a belief the design relies on that nobody has confirmed, with `statement`, `basis`, `author`, and `created_at`. Distinct from `OpenQuestionAnnotation` (undecided) in that the model already proceeds as if the assumption were true. Backs new validator rule `ASSUMPTION_PRESENT` (Info). `OpenQuestionAnnotation` now also backs a validator rule, `OPEN_QUESTION_PRESENT` (Info), where previously nothing read it.

### Changed
- Proto package renamed `eventmodel.v1` → `trogonatlas.eventmodel.v1`; every `.proto` file moved under `proto/trogonatlas/eventmodel/v1/`. Every stored `google.protobuf.Any` type url, error domain, and gRPC method path carries the new package; there is no compatibility alias for the old name.
- `namespace` field documentation updated: field is now explicitly documented as required and non-empty, validated server-side. Previous docs described it as a free-form optional disambiguator; that was incorrect as the server enforces non-empty namespace on write.
- `DerivedFieldAnnotation` comment added clarifying `doc` is required; suppresses `EVENT_FIELD_UNSOURCED` and `AUTOMATION_COMMAND_FIELD_UNSOURCED` for annotated fields.
- Various inline field and message comments enriched across `annotations.proto`, `fields.proto`, `service.proto` for clarity.

## [0.1.0] - Initial schema state

### Summary
Initial v1 proto schema for the Event Modeling gRPC service under package `trogonatlas.eventmodel.v1`. Covers:

- **common.proto** - `EventModelId`, `EntityRef`, `Pagination`, `PageInfo`, shared scalar helpers.
- **fields.proto** - `FieldSpec`, `FieldValue`, `FieldKind` enum, per-field annotation support.
- **annotations.proto** - `DerivedFieldAnnotation` with `Kind` enum (TIMESTAMP, UUID, SEQUENCE, CONTEXT).
- **moments.proto** - `Command`, `Event`, `ReadModel` message types.
- **slices.proto** - `Automation`, `Translation`, `Trigger` slice types.
- **storyboards.proto** - `Storyboard`, `StoryboardStep` for user-journey modelling.
- **scenarios.proto** - `Scenario`, `GivenStep`, `WhenStep`, `ThenStep` for BDD-style specifications.
- **infrastructure.proto** - `Domain`, `BoundedContext`, `Subdomain` structural hierarchy.
- **strategic.proto** - `Project` top-level container.
- **event_model.proto** - `EventModel` aggregate plus `Entity` oneof container.
- **service.proto** - `EventModelingService` gRPC service definition with full CRUD, validation, bulk-delete, reference-graph, change-streaming, and domain-scoped listing RPCs.

### Breaking-change protection
`buf.yaml` (v2, `breaking: use: [FILE]`) is colocated with the proto files. Any wire-breaking change (field removal, number renumbering, type change, RPC removal) will be caught by `buf breaking` against a committed baseline image.
