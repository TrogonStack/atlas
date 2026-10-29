# Design graph topology contracts

Status: native contract implemented on the integration branch, with the
exceptions listed under [Shipped versus proposed summary](#shipped-versus-proposed-summary).
This document defines a bounded authored flow and design topology separate from
behavioral storyboards, document body prose, diagram layout bytes and workflow
execution. The [acceptance requirements](design-topology-acceptance.md) map each
requirement to executed tests and name the boundaries that remain unproved.

## Scope and existing representations

| Representation | Shipped meaning and boundary |
| --- | --- |
| [Storyboard](../../proto/trogonatlas/eventmodel/v1alpha1/storyboards.proto) | Ordered Event Modeling slices, entry/outcome edges and concrete [StoryboardTrace](../../proto/trogonatlas/eventmodel/v1alpha1/storyboards.proto) steps pinned to slice execution shapes. Trace `steps[]` may repeat the same slice with distinct concrete data. Fork semantics stay on Swimlane transitions, not storyboard-authored branch graphs. |
| [Document](../../proto/trogonatlas/eventmodel/v1alpha1/decision_document.proto) | Retained text or inert external link, declared anchors and identified navigation. No typed step graph or transition inventory. |
| [Specification](../../proto/trogonatlas/eventmodel/v1alpha1/specification.proto) | Artifacts and selectable message/operation members. A member is not a flow step occurrence. |
| [CatalogSubject](../../proto/trogonatlas/eventmodel/v1alpha1/decision_document.proto) | Exact entity, connection, contract member or data port selection used by Decision, Document and responsibility surfaces. |

EventCatalog pinned inventory (commit
[`2736a2272207cf1b1ee09e6a440401d2215c6380`](https://github.com/event-catalog/eventcatalog/tree/2736a2272207cf1b1ee09e6a440401d2215c6380))
defines flow frontmatter as `steps[]` where each step has stable `id`, optional
`title`/`summary`, at most one resource pointer among message, agent, service,
system(s), channel, flow, container, dataProduct, actor, custom or
externalSystem, and either `next_step` or `next_steps` with optional transition
labels. Designs in `.ecstudio` retain opaque `nodes`, `edges` and `viewport`
JSON without a typed catalog graph in that revision. This contract covers the
flow step and transition semantics and a logical design graph profile. It does
not import `.ecstudio`, parse diagram files, render layouts or execute steps.

## New native resource versus extension

Propose a new top-level entity kind `DesignGraph` rather than extending
Storyboard, Document or registered metadata. Coordinated implementation reserves
Entity union field `39` and EntityKind enum value `38` for DesignGraph. Document
already occupies Entity union field `38` and enum value `37`; the numbering
domains differ. Recheck the reservation when integrating parallel schema work.

| Alternative | Rejection |
| --- | --- |
| Extend Storyboard | Storyboard steps are behavioral slice pins and trace execution records, not optional catalog resource selections or generic titled boxes. Mixing generic topology into Storyboard would blur Event Modeling narrative with catalog flow diagrams. |
| Extend Document | Document body remains prose or link; anchors are navigation declarations, not graph nodes with labeled transitions. Typed traversal requires a dedicated inventory. |
| Metadata-only extension | Would forfeit exact reference discovery, guarded occurrence identity, Source ownership boundaries and typed queries. |

Storyboard and DesignGraph may reference the same catalog subjects through
independent `subjects` or step targets. Neither resource implies the other,
executable order or Swimlane lifecycle.

## Entity shape

Propose `proto/trogonatlas/eventmodel/v1alpha1/design_topology.proto`
(with generated code following existing entity patterns). Implementation must
recheck union tag and field availability before controlled generation.

| DesignGraph field | Type and rule |
| --- | --- |
| `id = 1` | Required existing `Id`. |
| `title = 2` | Required `CatalogTitle`. |
| `doc = 3` | Optional `CatalogText`. |
| `profile = 4` | Required `DesignGraphProfile`: `FLOW` for catalog-style flows, `DIAGRAM` for logical design graphs without claiming layout. |
| `steps = 5` | Nonempty identified `TopologyStep` collection at creation; may become empty only if the whole graph is withdrawn through deletion, not partial truncation. |
| `entry_steps = 6` | Optional repeated `LocalValueId` values selecting existing step IDs, without separate occurrence IDs. Empty means no authored entry set; queries may derive candidates (steps with no incoming transition targets) without treating derivation as execution entry. |
| `subjects = 7` | Optional identified `SubjectAssociation` collection for catalog grouping, at most 1,024. |
| `layout_document = 8` | Optional exact `DocumentRef` pointing at retained layout or diagram bytes. Logical topology and visual layout remain separate; this field does not parse or validate layout structure. |
| `metadata = 9` | Existing registered typed annotations. |
| `supersedes = 10` | Optional exact prior version; existing terminating single-parent version rules apply. |

Whole prepared entity limit is 1 MiB encoded, including assigned IDs and all
collections. Lower transport or storage limits still win; rejection never
truncates.

### Step identity and optional target

Each `TopologyStep` is an identified graph node whose identity is independent of
any optional catalog target.

| TopologyStep field | Type and rule |
| --- | --- |
| `step_id = 1` | Server-assigned `LocalValueId`, required in persisted state and unique within the owner incarnation. Omission requests assignment; explicit empty or malformed IDs reject before normalization. Stable across title, summary, target and transition label edits and list reorder. Removing a step retires its ID; re-adding an equal-looking step requires a fresh ID. |
| `title = 2` | Required `CatalogTitle`. Generic titled steps are first-class even when `target` is absent. |
| `summary = 3` | Optional `CatalogText`. |
| `target = 4` | Optional `TopologyStepTarget` oneof; at most one alternative. Absent means a generic step with no catalog selection, matching EventCatalog custom boxes without inventing a pseudo resource. |
| `transitions = 5` | Identified outgoing `TopologyTransition` collection, at most 64 per step. Order is authored display order only, not execution priority. |

EventCatalog mutual exclusion of resource pointers maps to the oneof. EventCatalog
ignored `type` frontmatter is not stored. Equal catalog targets require distinct
`step_id` values to preserve repeated occurrences and parallel references.

### TopologyStepTarget alternatives

Each alternative selects an exact native identity. No latest-version, title or
namespace-derived fallback.

| Alternative | Exact selection | EventCatalog pointer |
| --- | --- | --- |
| `catalog_message = 1` | `CatalogSubject` with MESSAGE `ContractMemberRef` or behavioral `EntityRef` to Event or Command. A query message remains a MESSAGE member; it does not become a ReadModel. | `message` |
| `component = 2` | `EntityRef` to Component. | `service` |
| `channel = 3` | `EntityRef` to Channel. | `channel` |
| `data_store = 4` | `EntityRef` to DataStore. | `container` |
| `data_product = 5` | `EntityRef` to DataProduct. | `dataProduct` |
| `subflow = 6` | `DesignGraphRef` to another DesignGraph at exact version. | `flow` |
| `actor = 7` | Inline `TopologyActor`: required `CatalogTitle name`, optional `CatalogText summary`. Not a Persona or Person ref. | `actor` |
| `custom = 8` | Inline `TopologyCustomStep`: required `CatalogTitle title`, optional summary, icon token, external HTTPS URL and string properties map with at most 32 entries. | `custom` |
| `external_system = 9` | Inline `TopologyExternalSystem`: required `CatalogTitle name`, optional summary and external HTTPS URL. Not an ExternalSystem entity ref. | `externalSystem` |

Agent and higher-level system targets depend on their dedicated contracts.
They must receive explicit supported target alternatives during coordinated
schema work; neither is temporarily represented as a deployable Component.
Domain, Team, Person and Decision kinds use `subjects` for grouping.

Subflow selection references another DesignGraph owner. It does not inline the
child graph. Traversal queries may expand subflows explicitly within a budget;
storage retains only the ref.

### Transitions, branches and loops

| TopologyTransition field | Type and rule |
| --- | --- |
| `connection_id = 1` | Required `ConnectionId` in persisted state; omission on create requests assignment. Distinct transitions may share the same target step ID with different labels or distinct connection IDs. |
| `label = 2` | Optional `CatalogTitle` transition label. |
| `target_step_id = 3` | Required `LocalValueId` referencing another step in the same DesignGraph owner. |

EventCatalog `next_step` versus `next_steps` maps to one or many `transitions`
entries. Reject a step that mixes a deprecated singular field with the repeated
collection if both were ever introduced; the proposed shape uses only
`transitions`.

Intra-graph cycles are allowed: a transition may target an earlier `step_id`
(loop). Validation does not require acyclic graphs because no execution is
claimed. Subflow cycles across DesignGraph owners are allowed at rest; bounded
traversal detects cycles and reports them without implying deadlock or runtime.

Reject at write time:

- `subflow` targeting the same DesignGraph `id` (self-subflow on the same version).
- Transition `target_step_id` missing from `steps`.
- Duplicate `step_id` values.
- Duplicate supplied transition IDs anywhere in the complete owner inventory.
  Equal-endpoint transitions, including equal labels, remain distinct authored
  occurrences and must preserve multiplicity.
- More than 512 steps per owner, more than 64 transitions on one step, or
  more than 2,048 step, transition and subject IDs across the whole owner.

Entry markers must reference existing step IDs. An entry step may also have
incoming transitions from loops. To build a graph without supplying fresh IDs on
initial creation, create its steps first, retain returned IDs, then add transitions
and entries under the returned etag. On existing owners, new steps omit IDs and
receive IDs before a later guarded write can reference them. No temporary handle
or predicted ID is accepted. Fresh explicit IDs on initial owner creation follow
existing new-owner rules.

Step-ID preparation needs a dedicated helper rather than reuse of Decision option
or Document anchor preparation. First match retained active IDs. For unmatched
old steps and omitted-ID additions, compare complete authored step values except
step ID and transition collection: an identical removed value reintroduced
without its ID is observable stripping and rejects. Equal-looking additional
steps are allowed when existing occurrences retain their IDs. A deliberate
same-value replacement uses separate remove and add writes. Nodes have no unique
natural key; changed content added without an ID is a new node, never proof of
lineage to an old node. Returned IDs alone establish node continuity.

Transition fingerprints include both source step ID and target step ID.
Implement descriptor-scoped ancestor identity for TopologyStep; the current
member_id/port_id wrapper handling is insufficient. Moving a transition between
steps therefore renews its connection ID even when target and label are equal.

## Identity, references and lifecycle

Reuse [connection identity](connection-identity.md) for `TopologyTransition`
occurrences. Step identity uses `LocalValueId`, not `ConnectionId`, because steps
are nodes rather than directed catalog edges.

| Operation | Required outcome |
| --- | --- |
| Create | Assign fresh owner UID and omitted UUIDv7 IDs for transitions and subject associations. Valid supplied transition IDs follow existing new-owner rules. |
| Existing-owner edit | Preserve active step IDs and transition IDs when fingerprints unchanged. Reject unknown supplied IDs and identity stripping. Changing a transition source step or target step requires a fresh transition ID; labels are excluded from the fingerprint. |
| Wording, reorder | Preserve step and transition IDs, including equal-target siblings. |
| Remove/re-add step or transition | Retired IDs stop resolving; equal content later receives fresh IDs. |
| Subflow retarget | Changing `DesignGraphRef` slug, namespace or version renews no step ID; only transition IDs follow transition rules when transition rows change. |
| Preview/apply, branch, land | Same rules as Decision Document associations and architectural occurrences: preserve same-incarnation identities, explicit deleted-ID renewal on conflict resolution, no partial publication. |
| Safe delete | Extend incoming reference extraction for subflow targets and subject associations. Hidden referrers block deletion. Force delete leaves subflow selectors unresolved. |

Extend outbound reference extraction for:

- `steps[i].target.*` entity, member, port and subflow refs.
- `steps[i].transitions[j].target_step_id` as an internal same-owner link (no
  external entity ref, but impact/closure should surface intra-owner structure).
- `layout_document`, `subjects`, `supersedes`.

Reference rows retain owner UID, connection or step identity, diagnostic field
path and multiplicity. Intra-owner transition targets use a stable internal
target descriptor in impact results rather than fabricating cross-entity refs.

Extend the closed `CatalogSubject` inventory to allow DesignGraph entity refs
where other catalog resources already allow exact entity kinds. DesignGraph step
targets do not automatically become `CatalogSubject` entries.

## Public queries

Propose `proto/trogonatlas/api/eventmodel/v1alpha1/design_topology_queries.proto`
with read-only RPCs on `EventModelService` (exact names may shift during
implementation; behavior must not).

| RPC | Input and result |
| --- | --- |
| `GetDesignGraphDetail` | Exact `DesignGraphRef`. Return authored profile, steps with resolved/unresolved targets, transitions with labels, entry markers, subjects, optional layout document resolution state, branch and checked scope metadata. |
| `ListSubjectDesignGraphs` | Exact `CatalogSubject`. Return authorized DesignGraph owners that declare the subject, with association IDs and unchanged selectors. No recursive expansion. |
| `TraverseDesignGraphTopology` | Required root `DesignGraphRef`, optional `follow_subflows`, `max_depth` (default 8, hard cap 32) and no role filter; authorization alone governs target summaries. Walk intra-graph transitions and, when enabled, expand `subflow` targets one level per hop. Return nodes, edges, unresolved subflow markers, cycle records and completeness flags. No execution order or reachability proof beyond authored graph. |
| `ListIncomingDesignGraphSubflows` | Exact `DesignGraphRef` as subflow target. List referring steps with owning graph ref, step ID and transition provenance. |

Traversal returns every root-owner step, including disconnected steps; entry
markers do not hide authored topology. Root owner has subflow depth zero. A
subflow expansion adds one to depth; intra-owner transitions do not. A
max_depth of one includes direct child owners but no grandchildren. If a deeper
subflow is present, return its exact authored boundary without expansion and set
incomplete=true with a depth-limit diagnostic. follow_subflows=false is an
explicit unexpanded view, not a failed resolution. Cycles are returned as cycle
diagnostics and do not cause repeated expansion. Hard row/work/output budget
overflow remains RESOURCE_EXHAUSTED with no successful partial receipt.

Resolution diagnostics must not distinguish hidden endpoints from missing
endpoints. Return the same generic unavailable state without endpoint title, UID
or existence details. Operational failures remain RPC errors.

Role-aware selection: callers with Reader role receive the same graph structure
as Writers; mutation remains on guarded entity writes. Traversal does not infer
additional steps from Component membership, communication intent, Storyboard
order or Document navigation. `InferDataFlow` and storyboard projections remain
separate analysis surfaces.

## Authorization, bounds and Source isolation

DesignGraph writes require authorized create or update on the entity namespace
and reference access to every selected target kind in the prepared graph.
Subflow refs require read access to the child graph for admission validation on
write; hidden child graphs cause write rejection when the subflow target cannot
be validated, without disclosing hidden titles.

Extend existing shared ceilings wherever these surfaces apply:

| Ceiling | Application |
| --- | --- |
| 5,000 resource rows | Visible DesignGraph discovery lists. |
| 10,000 examined occurrences | Traversal subflow expansion, subject resolution and nested owner hydration share one request budget. |
| 4,096 emitted records | Traversal and incoming-subflow listing outputs. |
| 1 MiB prepared owner | Whole DesignGraph entity. |
| 512 steps, 64 transitions per step, 1,024 subjects, 2,048 total local IDs | Graph-specific limits within the owner ceiling; all limits apply together. |

Charge each distinct owner hydrated during traversal once per request. Repeated
queries into the same subflow owner consume work again on a new request but not
through nested counter reset within one request. Overflow returns
RESOURCE_EXHAUSTED, not a silently truncated graph.

DesignGraph authoring is native-only. Extend Source closed selection and
ownership inventories to reject step IDs, transition IDs, step collections,
transition collections and entry markers. Reject Source creates carrying step
topology under every ownership policy, including TITLE-only: initial create
copies incoming content. Reject WholeOwner writes when current or incoming step
inventories are nonempty, including proposed clears. Supported unrelated
field-specific updates preserve local topology exactly. EventCatalog flow
frontmatter mapping, `.ecstudio` import and remote sync remain outside this
contract.

## Relationship to Storyboard traces

| Concern | Storyboard trace (shipped) | DesignGraph (proposed) |
| --- | --- | --- |
| Step key | `TraceStep.slice` plus execution oneof | `TopologyStep.step_id` plus optional catalog target |
| Repetition | Same slice ref across trace steps | Same catalog target across distinct step IDs |
| Transitions | Implicit trace step order only | Explicit labeled `TopologyTransition` targets |
| Subflows | Not represented | `subflow` DesignGraphRef |
| Data pinning | CommandExample, ReadModelExample | None; no executable payload |
| Analysis | Projections, InferDataFlow, completeness | TraverseDesignGraphTopology only |

Converting between representations is a separate tool concern. Neither resource
validates the other automatically.

## Shipped versus proposed summary

| Item | Status |
| --- | --- |
| Storyboard, Document, Specification, CatalogSubject | Shipped |
| DesignGraph entity, topology messages, reference extraction, four read RPCs, Source guards, CLI and MCP surfaces, Studio gateway routes | Implemented, with executed coverage in the acceptance map |
| System grouping step target (union arm 10) | Depends on the separate system grouping contract; not integrated here |
| Authenticated browser rendering of a native graph | Maintained `design-graph.native.spec.ts` uses the real authenticated backend fixture; exact persisted FLOW/DIAGRAM identities and deleted-target safety are asserted. This is inspection evidence, not deployed readiness. |
| EventCatalog import, `.ecstudio` parser, layout renderer, workflow execution | Explicitly out of scope |

## Open objections

- Agent and system target alternatives await their native contracts; no Component fallback is permitted.
- The proposed entry list is authored and independent of derived zero-incoming candidates; pure cycles can name entries explicitly.
- Layout content does not attest logical node equality and is not parsed.
- Parent reserves DesignGraph union `39`/enum `38` and system grouping union `40`/enum `39` independently; integration must preserve both allocations.

## Exact dependencies for implementation

- Proto: new `design_topology.proto`, entity union and `EntityKind` entry,
  `DesignGraphRef`, contract revision and minimum mutation revision bump.
- Core: validation, reference extraction and rewiring in
  [`refs.rs`](../../rsworkspace/crates/trogon-atlas-core/src/refs.rs), connection
  persistence for transitions, CatalogSubject inventory, responsibility contexts.
- Server: admission and per-kind content validation in conv.rs; resource and
  complete occurrence accounting in architecture_resources.rs and
  decision_document_resources.rs; new-kind reference snapshot inclusion in
  architecture_admission.rs; graph provenance, RPC handlers and auth required_role
  mapping; traversal cycle and budget enforcement.
- Client: manifest typing, CLI/MCP parity, uint64 and occurrence identity in
  export and diff.
- Source: closed inventory updates documented in
  [source integration](source-integration-contracts.md).
- Acceptance: executable fixtures described in
  [design-topology-acceptance.md](design-topology-acceptance.md).

Parent owns code generation, Cargo and JS builds for this proposal. This document
is the first implementation artifact for the flow and design topology checklist
item in [eventcatalog-coverage.md](eventcatalog-coverage.md); closing that item
still requires linked schema, validation and query evidence.
