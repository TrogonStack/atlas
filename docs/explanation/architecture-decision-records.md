# Architecture decision records

Status: proposed product capability and architecture. The record profile,
Markdown planner, decision log queries and Studio view described here require
implementation. Names below specify the intended model, not existing protobuf
fields, RPCs, commands or manifest syntax. Publishing this page does not
implement them or settle their wire shapes.

The existing [Decision and Document contract](../reference/decision-and-document-contracts.md)
and its [catalog context additions](../reference/catalog-context-contracts.md)
are implemented and stay authoritative for every field they define.

## Product intent

Teams keep architecture decision records (ADRs) as numbered Markdown files next
to their code. Atlas should let a team bring that decision log into the catalog
and keep it there: each record stays tied to the file it came from, links to
the catalog resources it governs, and is answerable through the same backend
that people and agents already use for the model.

A person or agent should be able to ask:

- Which records exist in a decision log, in which state?
- Which record is `0007`, and where is its source?
- Which decisions govern this Event, Component or DataProduct?
- Which record replaced or amended this one?
- Has the source file changed since the record was last mapped?

Atlas's own source is a useful acceptance fixture. Its comments cite numbered
decisions such as `Decision #21`, and no committed decision log defines them.

## Existing foundation

A Decision already carries most of an ADR. The gap is the record's identity
within a log, its tie to source material, a reviewed way in from Markdown and
a log-wide view.

| ADR concept | Existing capability | Boundary |
| --- | --- | --- |
| Status | Decision `status`: PROPOSED, ACCEPTED, REJECTED, WITHDRAWN, SUPERSEDED. | No deprecated native status. `LifecycleAnnotation` can label a Decision deprecated without changing it. A source label is prose and cannot establish native status. |
| Context and decision | `context`, `rationale`, `options` and `selected_option`. | `context`, `rationale` and a nonempty `options` are required. A selected option is required for ACCEPTED and SUPERSEDED and rejected for every other status. Each text value is limited to 16 KiB. |
| Consequences | `consequences`, each BENEFIT, COST or RISK. | Required nonempty for ACCEPTED and SUPERSEDED. Every consequence needs a kind. |
| Supersession and amendment | `replaced_by` and `amends` between exact Decisions. | SUPERSEDED requires a replacement whose chain ends at an ACCEPTED Decision. |
| Dates and participants | `authored_on`, `decided_on` and `decision_makers`. | Authored assertions. No approval authority is implied. |
| Affected resources | `subjects` select exact entities, connections, contract members or data ports. `ListSubjectDecisions` answers the reverse question. | Required nonempty. Decision and Document cannot be subjects. |
| Source text | A Document retains exact Markdown bytes, at most 256 KiB, with a verified SHA-256 digest. A Decision selects it through `documents`. | No field says which supporting Document is the record's source. |
| Source location | `CodeRefAnnotation` records a logical repository, path and optional commit on any entity. | Descriptive. The server never opens the repository. |
| Revision history | Entity revisions and changesets retain earlier state. | No source revision is tied to a Decision. |
| Listing | `ListEntities` filters by kind, namespace and lifecycle annotation. | No filter by Decision status, date or maker. No record number. |
| Review | Studio inspects one Decision or Document from the entity inspector. | No decision log view. |

Bringing a log into Atlas changes the [backing report](../reference/backing-report.md).
An ACCEPTED Decision counts as evidence for each exact entity among its
subjects; other statuses do not. A Document counts for its exact entity
subjects whatever the Decision's status, so every retained source file backs
the subjects it names.

## Scope

In scope:

- A typed record profile that marks a Decision as one record of a named
  decision log.
- A pin from the Decision to the retained source it was mapped from.
- An offline planner that turns supplied ADR Markdown into ordinary manifests
  and a mapping report.
- Log-wide queries through the backend, CLI and MCP.
- A read-only Studio decision log.

Out of scope:

- A second decision resource. A record is a Decision. It does not copy
  Decision fields into another entity kind.
- Replacing a team's Markdown authoring workflow.
- Reading a repository, following links or watching for changes from the
  server. Source material is always supplied by the caller.
- Inferring a status, consequence kind or subject that the source does not
  state.
- Approval workflow, signatories or votes.
- EventCatalog-specific readers or conversion. That work stays deferred under
  the [native-first scope](../reference/eventcatalog-coverage.md#native-first-scope).
  The planner here reads plain ADR Markdown conventions.

## Package boundary

New record types live in their own protobuf packages:

| Package | Holds | May import |
| --- | --- | --- |
| `trogonatlas.adr.v1alpha1` | The record profile and its value types, including its own digest value. | `trogonatlas.type.v1alpha1` and the existing exact reference types in `identity.proto`. |
| `trogonatlas.api.adr.v1alpha1` | The decision log query service and its messages. | The record package, `type` and existing reference and status types. |

No message, enum or RPC is added to `trogonatlas.eventmodel.v1alpha1` or to
`trogonatlas.api.eventmodel.v1alpha1`, and neither imports the new packages.
The record package uses existing reference types, as the shared
`trogonatlas.annotation.v1alpha1` package does. A Decision carries the profile
the way it carries any typed metadata, as a `google.protobuf.Any` value in
`metadata`.

Keeping the profile outside the entity union leaves entity kinds and the
existing Decision fields unchanged. Decision logs are not Event Modeling
primitives, and their types should not widen that package.

Built-in metadata is recognized when a `trogonatlas` message name ends in
`Annotation`. Beyond that, each slice wires the new packages by hand: the Rust
re-export list in `trogon-atlas-proto`, the Studio protobuf load list, the
Studio native metadata type list and the closed lists that validate and decode
known annotations.

## Record profile

`trogonatlas.adr.v1alpha1.RecordAnnotation` is valid only in the top-level
`metadata` of a Decision, at most once.

| Field | Contract |
| --- | --- |
| `log` | Required logical decision log name, nonblank UTF-8, at most 256 bytes. It names a log, such as a repository's ADR directory. It is never a URL or filesystem path. |
| `record_id` | Required source-assigned identifier, kept verbatim, such as `0007`. Nonblank UTF-8 without control characters, at most 128 bytes. |
| `source_status` | Optional verbatim source status label, at most 256 bytes. Descriptive only. It never sets, implies or overrides the Decision's native status. |
| `template` | Optional source convention: NYGARD, MADR or OTHER. Descriptive only. |
| `source` | Optional `RecordSource`: an exact `DocumentRef`, plus the SHA-256 digest of the bytes the structured fields were mapped from. |

Rules:

- `(log, record_id)` is unique among Decision families in one namespace. Every
  version of one Decision slug shares its record identity. A second slug
  claiming the same pair rejects.
- `source.document` must equal the whole-document selection of one entry in
  the same Decision's `documents`. Removing that entry requires removing or
  repointing `source` in the same write.
- The selected Document must have a retained body when the write is admitted.
  An external link cannot be a record source, because its bytes are unverified.
- The profile's `DocumentRef` is decoded explicitly into reference discovery
  and retargeting, as `ResourceGroupsAnnotation` targets are, so a retarget
  moves the association and the pin together. No other payload value becomes a
  reference. Delete protection already follows from the `documents` entry.
- Invalid profiles reject the write atomically, like any other invalid
  Decision content.

Source location stays on the source Document as a `CodeRefAnnotation`: logical
repository, path and, when exactness matters, the commit. A changed source file
updates that Document's body at the same version, with its new digest. Entity
revision history retains the earlier bytes.

### Source state

A record's source state is computed on read, never stored:

| State | Meaning |
| --- | --- |
| `CURRENT` | The selected Document's digest equals `source.digest`. |
| `STALE` | The selected Document resolves and its digest differs. The structured fields were mapped from other bytes. |
| `UNAVAILABLE` | The selected Document is missing or hidden from the caller. The two cases are indistinguishable, as in existing detail queries. |
| `UNDECLARED` | The profile has no `source`. |

`CURRENT` proves that the stored bytes are the ones the mapping used. It does
not prove that the structured fields restate them faithfully, and it says
nothing about the file in the repository today.

## From Markdown to manifests

The first way in is a planner in the CLI, with a matching MCP tool:

```sh
trogon-atlas adr plan --log platform --namespace platform-decisions \
  --source docs/adr --output decisions.yaml
```

The planner runs offline and never contacts a server. The CLI reads the named
files, expanding a directory; the MCP tool takes the file contents as input.
It parses them and writes two things: ordinary Document and Decision manifests,
and a mapping report. Loading the result is the existing guarded path:
`apply --dry-run`, review, then `apply`, which sends the plan as one atomic
batch. Admission, authorization, graph checks and revision history stay in the
backend.

Structural mapping is deterministic:

| Source element | Planned value |
| --- | --- |
| File bytes | One Document with a retained `text/markdown` body, its digest and a `CodeRefAnnotation`. |
| File number | `record_id` verbatim. The Decision and Document slugs derive from the log name and a slug-safe form of the identifier, so retitling a record keeps its identity. |
| Title | `title`. |
| Status line or front matter status | `source_status` verbatim, and the native status from the status table below. |
| Context section | `context`. |
| Decision or decision outcome section | `rationale`. |
| Considered options and chosen option | `options`, and `selected_option` when the native status is ACCEPTED or SUPERSEDED. |
| Decision section without listed options | One option, titled with the record title, whose description is the decision text. The report marks it as planner-structured. |
| Consequences stated as good or bad | BENEFIT or COST consequences. |
| Date and deciders front matter | `decided_on` and `decision_makers`, when deciders resolve to exact Person or Team entities. |
| "Superseded by" and "Amends" links to another record in the log | `replaced_by` and `amends`. |

Native status is set only from an exact, case-insensitive label match:

| Source label | Native status |
| --- | --- |
| proposed, draft | PROPOSED |
| accepted | ACCEPTED |
| rejected | REJECTED |
| superseded | SUPERSEDED, when the replacement is a Decision in the plan or the store whose replacement chain ends at an ACCEPTED Decision |
| withdrawn | WITHDRAWN |
| anything else, including deprecated | Not mapped |

Some required Decision content has no source in a typical ADR. The planner
reports each gap and leaves that record out of the plan until a reviewer
supplies the value explicitly:

- **Subjects.** Decision and Document both require one. A reviewer names the
  subjects per record, or a default subject for the log, such as a Project,
  Domain or Component.
- **Consequence kinds.** Free prose consequences have no BENEFIT, COST or RISK
  kind until a reviewer classifies them.
- **Unmapped status.** A label outside the table needs an explicit reviewer
  choice.
- **Unresolved links.** A replacement or amendment target that is neither in
  the plan nor in the store, a superseded record whose replacement chain does
  not end at an ACCEPTED Decision, or a decider with no matching Person or
  Team. A record that depends on a record left out of the plan is left out too.
- **Missing required text.** No context, no decision text or a blank title.
- **Unusable identifier.** A record identifier with no slug-safe form.
- **Oversized content.** A section above the 16 KiB text limit, or a file above
  the 256 KiB retained content limit. Nothing is truncated.

Reviewer inputs live in a mapping file kept beside the source, so a later run
reuses them. The report marks every planned value as source-derived or
reviewer-supplied. An agent may propose reviewer inputs; a person inspects the
report and the dry run before anything lands.

A record left out of a plan is a visible gap, not a failure of the whole log.
Partial input never authorizes deleting a Decision that an earlier run created.

### Keeping records current

Re-running the planner on a changed file plans the same Document with its new
body and digest, and the same Decision with remapped fields and a new
`source.digest`. The `documents` entry is unchanged, so the batch needs no
server-assigned identity. Entity revision history retains the earlier mapping.
Editing a Decision in Atlas without touching its source leaves the source state
`CURRENT`; editing the source Document without remapping makes it `STALE`.

This first slice has no source overlay, drift reconciliation or scheduled
refresh. [Source integration](../reference/source-integration-contracts.md)
owns those concerns and currently rejects creates that carry the Decision's
catalog context fields. Running decision logs through source runs is a later
decision, listed below.

## Decision log queries

`trogonatlas.api.adr.v1alpha1.DecisionLogService` reads Decisions that carry
the record profile. It adds no stored index contract.

| RPC | Input and result |
| --- | --- |
| `ListDecisionLogs` | Namespaces. Returns each visible log name with its record count per native status. |
| `ListDecisionRecords` | Namespace and log, with optional filters for native status, `decided_on` range, exact decision maker and source state. Returns the exact Decision reference, owner UID, record profile, title, native status, dates and source state, ordered by `record_id`. |
| `GetDecisionRecord` | Namespace, log and `record_id`. Returns the same row, the resolved source Document selection and the replacement and amendment neighbors. |

These queries follow the existing typed-query rules: the caller's authorized
view, the branch header, bounded work with `RESOURCE_EXHAUSTED` on overflow,
authorized totals only and explicit completeness. A hidden or failed lookup is
never a verified empty log.

"Which decisions govern this resource" stays with `ListSubjectDecisions`. The
new service does not duplicate it.

CLI and MCP expose the three queries. Studio adds a read-only decision log:
records in query order with number, title, native status, source label,
decided date and source state, and a record page that shows the retained
Markdown inertly beside the structured Decision, with its replacement and
amendment chain. Edits stay in the CLI and MCP.

## Delivery slices

Each slice is reviewable alone and leaves Atlas coherent.

| Slice | Delivers | Complete when |
| --- | --- | --- |
| 1. Record profile | The `trogonatlas.adr.v1alpha1` package, admission rules, explicit reference decoding, manifest round trip and Studio decoding. | A Decision with a valid profile round-trips through manifest, CLI and MCP. Every rule above has a rejecting test. Reference and acceptance pages exist. |
| 2. Planner | `adr plan`, the mapping file, the report and a how-to. | A fixture log in Nygard and MADR form plans, dry-runs and applies. Every gap class above is reported and never guessed. |
| 3. Log queries | The query service with CLI and MCP surfaces. | Connected acceptance covers filters, ordering, source states, restricted views and budget exhaustion. |
| 4. Studio decision log | The log list and record page. | Component and connected evidence cover a log with replaced, amended, stale and unavailable records. |

## Decisions to settle before dependent implementation

- **Deprecated records.** An adopted decision that no longer governs new work
  and has no replacement has no native status. Mapping it to ACCEPTED keeps
  counting it as decision evidence. Mapping it to WITHDRAWN discards its
  selected option. A `LifecycleAnnotation` can already label the Decision
  deprecated with a date, and `ListEntities` filters on that label, but the
  label does not change native status. The preferred direction is a native
  deprecated status in the Decision contract, proposed and reviewed separately.
  Until then, the planner leaves such records unmapped unless a reviewer
  chooses a status.
- **Records without listed options.** The Decision contract requires an
  option, and many records state only the decision. Either the planner keeps
  emitting one option from the decision text, or the Decision contract allows
  an adopted decision with no options. The second is a separate Decision
  contract change.
- **Unclassified consequences.** Either reviewers always classify consequences,
  or the Decision contract gains a neutral consequence kind. The second option
  is also a separate Decision contract change.
- **Records with no catalog subject.** Some decisions concern the whole system.
  A default subject per log covers them without weakening the subject
  requirement. Confirm that this is acceptable before relaxing anything.
- **Record identity scope.** Uniqueness per namespace keeps the check inside
  one authorization boundary. A log spanning namespaces would need a different
  rule.
- **Record identifier ordering and slugs.** Zero-padded numbers order
  bytewise. Unpadded or mixed identifiers need a defined collation, and the
  slug-safe form needs an exact rule.
- **Decision log as a resource.** A log is only a name in this proposal. A log
  with its own title, owner and documentation would need an owning entity.
- **Source runs.** Whether decision logs should later use source registrations
  for drift and reconciliation, and what that requires from the catalog
  context restriction.

## Principles this follows

- [Backend authority](principles/backend-authority.md): the planner only
  produces manifests. Admission, uniqueness and source state are backend rules.
- [AI-first work](principles/ai-first.md): an agent runs the plan and proposes
  reviewer inputs; a person inspects the report.
- [Explicit intent and state](principles/explicit-state.md): unmapped records,
  reviewer-supplied values and stale sources are visible states, never silent
  defaults.
- [Layered information](principles/layered-information.md): records join the
  existing subject, replacement and amendment graph, and unresolved links stay
  inspectable.
