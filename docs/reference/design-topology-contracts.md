# Design graph topology contracts

Status: native schema, backend admission, typed queries, manifest/CLI/MCP
authoring and the Studio review surface are implemented. The
[protobuf contract](../../proto/trogonatlas/eventmodel/v1alpha1/design_topology.proto)
defines the wire representation and the
[query contract](../../proto/trogonatlas/api/eventmodel/v1alpha1/design_topology_queries.proto)
defines the read RPCs. The [acceptance reference](design-topology-acceptance.md)
links executable coverage and distinguishes it from exhaustive or deployed
evidence.

DesignGraph is a bounded authored flow and design topology. It is separate from
behavioral storyboards, document prose, diagram layout bytes and workflow
execution. It claims no execution order, reachability proof or runtime
behavior, and it has no importer, provider sync, federation or diagram parser.

## Identity allocation

DesignGraph uses Entity union field 39 and `ENTITY_KIND_DESIGN_GRAPH` value 38.
Document occupies union field 38 and enum value 37; the numbering domains
differ. Both allocations were verified free before generation. System grouping
owns union field 40 and enum value 39 in a sibling change, and the target oneof
leaves field 10 unused for its future arm. Integration must preserve all of
these allocations and report a conflict rather than overwrite one.

Contract revision and minimum client/server revisions are 19. Writers from
before DesignGraph cannot name the kind, and a server from before it cannot
store it, so clients refuse the mismatch before mutating in both directions.
Upgrade connected clients and servers together.

## Entity shape

| DesignGraph field | Rule |
| --- | --- |
| `id` | Required exact `Id`. |
| `title` | Required `CatalogTitle`. |
| `doc` | Optional string with presence. |
| `profile` | Required: `FLOW` for catalog-style flows, `DIAGRAM` for logical design graphs. Neither claims layout. |
| `steps` | Identified `TopologyStep` collection. |
| `entry_steps` | Optional authored entry markers naming existing step IDs. Empty means no authored entry set; derived zero-incoming candidates are never treated as execution entry. |
| `subjects` | Optional identified `SubjectAssociation` collection for catalog grouping. |
| `layout_document` | Optional exact `DocumentRef` to retained layout bytes. Linked bytes stay unverified and unparsed. |
| `metadata` | Registered typed annotations. Native metadata (including lifecycle) is supported on the kind. |
| `supersedes` | Optional exact prior version under the existing single-parent rules. |

### Steps

| TopologyStep field | Rule |
| --- | --- |
| `step_id` | Server-assigned `LocalValueId`, unique in the owner incarnation. Omission requests assignment; malformed values reject. Stable across title, summary, target and label edits and across reorder. |
| `title` | Required. A step with no target is a first-class generic node. |
| `summary` | Optional text. |
| `target` | Optional `TopologyStepTarget`; at most one alternative. |
| `transitions` | Identified outgoing `TopologyTransition` collection in authored display order. Order carries no priority. |

### Targets

Every selection is exact. There is no latest-version, title or namespace
fallback and no temporary agent or system fallback to Component.

| Alternative | Exact selection |
| --- | --- |
| `catalog_message` | `CatalogSubject`: a MESSAGE contract member or an Event or Command entity. A ReadModel is rejected. |
| `component` | `EntityRef` to a Component. Component service targets are legitimate. |
| `channel` | `EntityRef` to a Channel. |
| `data_store` | `EntityRef` to a DataStore. |
| `data_product` | `EntityRef` to a DataProduct. |
| `subflow` | `DesignGraphRef` at an exact version. |
| `actor` | Inline `TopologyActor` with a required name. Not a Persona or Person ref. |
| `custom` | Inline custom step with a required title and a bounded property map. |
| `external_system` | Inline name, summary and URL. Not an ExternalSystem entity ref. |

Agent targets remain Component with an `AgentAnnotation`. A system-group target
arrives with its own contract through the reserved field.

### Transitions

| TopologyTransition field | Rule |
| --- | --- |
| `connection_id` | Required `ConnectionId` in persisted state; omission requests assignment. |
| `label` | Optional `CatalogTitle`. Excluded from the identity fingerprint. |
| `target_step_id` | Required step ID in the same owner. |

The transition fingerprint covers source step ID and target step ID. Moving a
transition to another source, or retargeting it, renews its connection ID;
editing only the label preserves it. Parallel transitions with equal target and
equal label are legal and keep distinct IDs. Loops are legal because no
execution is claimed.

### Write-time rejections

- A subflow that selects the same graph `id`.
- A transition target or entry marker naming an unknown step ID.
- Duplicate step IDs, or duplicate supplied transition IDs anywhere in the owner.
- A step with more than one target alternative.
- More steps, transitions per step, subjects, local IDs or encoded bytes than the
  limits below.
- A target of the wrong kind or version, or one the writer cannot read, with the
  same generic message in both cases.

### Limits

| Limit | Value |
| --- | --- |
| Steps per owner | 512 |
| Transitions per step | 64 |
| Subjects per owner | 1,024 |
| Local IDs per owner | 2,048 |
| Encoded owner | 1 MiB |
| Properties on a custom step | 32 |

Rejection never truncates.

## Two-phase authoring

On an existing owner, new steps omit IDs and receive them in the write response.
A later guarded write then references the returned IDs for entries and
transitions. No handle, alias or predicted ID is accepted. Referencing an
omitted-ID step in the same write rejects.

The dedicated step-ID preparation first matches retained active IDs. Among
unmatched values it compares the whole step value except step ID and transitions:

- An identical removed value reintroduced without its ID in the same write
  rejects as identity stripping.
- Equal siblings are allowed while the existing occurrences keep their IDs.
- Changed content without an ID is a new node. Returned IDs alone establish
  continuity.
- Removing a step in one write and adding an equal step in another is the
  deliberate replacement path. No resurrection ledger exists.
- Explicit active IDs are always preserved.

## References and lifecycle

Outbound references cover step targets (entity, member, port and subflow),
`layout_document`, `subjects` and `supersedes`. Reference rows retain owner UID,
step or connection identity, field path and multiplicity.

Same-owner transitions are not entity references. Reference discovery surfaces
them as self-references whose `step_id` and `target_step_id` carry the internal
descriptors and whose `connection_id` carries the transition identity. Impact
traversal never visits the owner twice, so these rows appear in reference
queries rather than as extra impact nodes.

| Operation | Outcome |
| --- | --- |
| Create | Assigns the owner UID and omitted transition and subject IDs. |
| Edit | Preserves step and transition IDs under unchanged fingerprints; rejects unknown supplied IDs and stripping. |
| Retarget subflow | Renews no step ID. |
| Safe delete | Subflow referrers and subject owners block deletion. |
| Force delete | Leaves exact subflow selectors unresolved. |
| Branch and restore | Follow the existing occurrence rules with no partial publication. |

## Read RPCs

| RPC | Result |
| --- | --- |
| `GetDesignGraphDetail` | The checked entity, etag and per-target resolution states with scope and completeness. |
| `ListSubjectDesignGraphs` | Authorized graphs that declare an exact subject, with association IDs. Step targets alone do not appear. |
| `TraverseDesignGraphTopology` | Nodes, edges, unexpanded subflow markers, cycle records and diagnostics. |
| `ListIncomingDesignGraphSubflows` | Referring steps with owning graph, step ID and provenance. |

Reader is the required role for all four.

Traversal rules:

- The root owner is depth 0 and every root step is returned, including
  disconnected ones. Entry markers never hide topology.
- Each subflow hop adds one to depth; intra-owner transitions add none.
- `max_depth` defaults to 8 and is clamped to a hard cap of 32.
- At the depth limit a deeper subflow is returned as an explicit boundary with
  `incomplete` set and a depth-limit diagnostic.
- `follow_subflows` false is an explicit unexpanded view, not a failed resolution.
- Cycles produce a cycle diagnostic without repeated expansion.
- Hidden and missing endpoints share one generic unavailable state with no title,
  UID or existence detail. Operational failures remain RPC errors.

### Request budgets

| Budget | Value |
| --- | --- |
| Resource rows | 5,000 |
| Examined occurrences | 10,000 |
| Emitted records | 4,096 |

Each distinct owner hydrated is charged once per request. Overflow returns
RESOURCE_EXHAUSTED with no partial receipt.

## Authorization

Create, update, delete, retarget, branch and restore require write access to the
namespace and read access to every selected target. A hidden child graph rejects
the write without disclosing its title. Read RPCs require Reader.

## Source isolation

DesignGraph authoring is native only. The closed Source ownership inventory
gains `PROFILE`, `STEPS`, `ENTRY_STEPS` and `LAYOUT_DOCUMENT` fields; see
[source integration](source-integration-contracts.md). Source rejects:

- Initial creates carrying steps or entry markers, under every ownership policy
  including title-only.
- WholeOwner writes when current or proposed step or entry inventories are
  nonempty, including proposed clears.
- Closed selections naming step IDs, transition IDs, step or transition
  collections and entry markers.

Unrelated field-specific updates preserve local topology exactly.

## Relationship to Storyboard traces

| Concern | Storyboard trace | DesignGraph |
| --- | --- | --- |
| Step key | Slice plus execution oneof | `step_id` plus optional catalog target |
| Transitions | Implicit trace order | Explicit labeled transitions |
| Subflows | Not represented | `subflow` reference |
| Payload pinning | CommandExample, ReadModelExample | None |
| Analysis | Projections and InferDataFlow | `TraverseDesignGraphTopology` only |

Neither resource validates the other, and conversion is a separate tool concern.

## Client surfaces

The manifest schema is generated from the canonical kind list and carries typed
steps and transitions with lossless uint64 values. CLI and MCP expose the four
read RPCs next to native apply, export, diff and reapply. Studio is a review
surface: it shows loaded authored topology next to backend query receipts and
never infers topology from Storyboard order, Document prose or Component
membership.

## Out of scope

EventCatalog import, `.ecstudio` parsing, layout rendering or validation,
workflow execution, portable selector rebinding and atomic multi-owner snapshots.
