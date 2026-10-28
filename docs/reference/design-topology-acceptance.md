# Design graph topology acceptance requirements

Status: proposed requirements for the
[design topology contract](design-topology-contracts.md), not implemented or
executed coverage. No passing fixture, schema change or deployed readiness is
claimed. Logical names below are not manifest syntax, predicted owner UIDs or
submitted occurrence IDs.

## Fixture anchors

Prepare native entities in namespace `topology-fixture`, version `1`, reusing
patterns from existing server fixtures without modifying them:

| Anchor | Role in topology tests |
| --- | --- |
| Component `checkout-api` | Service-style step target and subflow parent subject. |
| Channel `order-events` | Channel step target. |
| DataStore `order-db` | Container-style step target. |
| DataProduct `orders-product` | Data product step target. |
| Specification `order-contract` with returned MESSAGE member `OrderPlaced` | Contract member message target. |
| Event `order-placed`, Command `place-order`, ReadModel `order-view` | Event/Command message targets; ReadModel is a negative message-kind control. |
| Document `checkout-layout` with retained Markdown bytes | Optional `layout_document` link only; no layout parser claims. |
| Storyboard `checkout-story` with two traces revisiting a slice | Control proving Storyboard traces are not DesignGraph steps. |

Create steps and retain returned IDs before the guarded write that adds entry
markers and transitions. Create DesignGraph `checkout-flow@1` profile `FLOW` with at least: generic
custom step, message step on `OrderPlaced` member, component step on
`checkout-api`, subflow step referencing child graph `payment-flow@1`, loop
transition back to an earlier step ID, and two outgoing transitions from one
step with distinct labels to different targets. Create child `payment-flow@1`
with two steps and a transition between them.

Create DesignGraph `system-context@1` profile `DIAGRAM` with actor and
external_system inline targets plus optional `layout_document` on
`checkout-layout`.

Retain actual owner UIDs, every `step_id`, transition `connection_id`, subject
association IDs and etags from authoritative reads. Record branch, authorization
context and query completeness metadata with each assertion.

Establish native persistence and exact reference discovery before client UI
checks. Preserve causal public RED/GREEN evidence for discovered defects.
Component or mocked HTTP fixtures cannot replace real Store, authenticated
transport, CLI/MCP or deployed-browser receipts.

## Admission and step identity

| Case | Required public outcome |
| --- | --- |
| Complete fixture create/read | Proposed fields preserve profile, Unicode titles, step order, transition order and distinct step IDs for equal catalog targets. |
| Generic step | Step with title and absent `target` admits; queries return a generic node without fabricated catalog ref. |
| Mutual exclusion | Step with more than one `TopologyStepTarget` alternative rejects atomically. |
| Self-subflow | `subflow` selecting the same DesignGraph `id` rejects. |
| Missing transition target | Transition referencing unknown `step_id` rejects. |
| Duplicate step IDs | Reject. |
| Equal parallel transitions | Equal target and equal labels retain distinct returned connection IDs; repeated supplied IDs reject across the whole owner. |
| Collection ceilings | Accept 512 steps and 64 transitions on one step at boundaries; 513th step or 65th transition rejects without truncation. |
| Whole owner size | 1 MiB encoded boundary rejects atomically including IDs and collections. |
| Wrong target kind/version | Component step pointing at wrong kind or version rejects without title or namespace fallback. |
| Hidden subflow target | Write referencing unreadable child graph rejects without leaking hidden titles. |
| Entry markers | Entry referencing missing step ID rejects; valid entries resolve in detail queries. |

## Transitions, branches and loops

| Case | Required public outcome |
| --- | --- |
| Linear chain | Single transition per step traverses in authored order when `follow_subflows` is false. |
| Branch | Step with two labeled transitions exposes both outgoing edges and distinct transition IDs. |
| Loop | Transition to earlier `step_id` admits; traversal reports cycle record without execution semantics. |
| Repeated catalog target | Two steps selecting the same Component ref remain distinct nodes with different `step_id` values. |
| Label edit | Changing transition label preserves `connection_id`; retargeting `target_step_id` renews transition ID. |
| Same-write unknown step | Referencing an omitted-ID newly added step before its authoritative ID is returned rejects; two guarded writes succeed. |
| Step stripping | Reintroducing an unmatched identical node value without its active ID rejects; adding an equal sibling while retaining the existing node IDs succeeds. |
| Transition source edit | Move a transition to another source step with the same target and label: old ID rejects; omitted ID receives a fresh ID. |
| Step reorder | Reordering `steps[]` preserves all step IDs and transition IDs. |
| Remove/re-add transition | Removed connection ID stops resolving; equal transition later receives fresh ID. |

## Subflows and traversal

| Case | Required public outcome |
| --- | --- |
| One-level subflow | `TraverseDesignGraphTopology` with `follow_subflows=true` and `max_depth>=1` includes child graph nodes and marks subflow boundary. |
| Depth cap | At `max_depth=1`, direct child owners are included and grandchildren remain explicit unexpanded boundaries with incomplete=true and a depth-limit diagnostic. |
| Subflow cycle | Graph A subflows to B and B subflows to A: traversal returns cycle diagnostics within budget, not stack overflow or execution error. |
| Incoming subflow listing | `ListIncomingDesignGraphSubflows` on `payment-flow` returns `checkout-flow` step selector and transition provenance. |
| Unfollowed subflow | `follow_subflows=false` returns an unexpanded subflow marker on the step without child node expansion. |
| Budget exhaustion | Fixture exceeding 10,000 examined occurrences or 4,096 emitted traversal records returns RESOURCE_EXHAUSTED, not partial graph posing as complete. |

## References, lifecycle and Storyboard boundary

| Case | Required public outcome |
| --- | --- |
| Outgoing refs | Reference discovery lists Component, Channel, DataStore, DataProduct, Specification member, subflow DesignGraph and Document layout refs with owner UID, field paths and parallel multiplicity. |
| Intra-owner transitions | Impact or closure surfaces same-owner transition structure without inventing external entity refs. |
| Safe delete | Subflow referrer or subject owner blocks deletion of referenced graph; force delete leaves subflow selectors unresolved. |
| Subflow retarget | Changing subflow slug/version preserves step ID; transitions unchanged unless edited. |
| Branch lifecycle | Same-incarnation branch edits preserve step and transition IDs; conflict renewal follows existing deleted-ID rules. |
| New owner incarnation | Clone or recreation assigns new owner UID; contained step/transition selectors remain stale. Plain exact-key subflow refs follow a same-key replacement and report its checked UID/etag. |
| Storyboard control | Storyboard trace steps and slice order remain unchanged when DesignGraph fixtures are added; no automatic cross-link. |

## Subjects, queries and role behavior

| Case | Required public outcome |
| --- | --- |
| ListSubjectDesignGraphs | Selecting Component `checkout-api` returns only graphs with an explicit subject association to that ref; component step targets alone do not appear unless also listed in `subjects`. |
| CatalogSubject inventory | DesignGraph entity ref may appear as subject where inventory allows; step inline actor/custom does not auto-create CatalogSubject. |
| GetDesignGraphDetail | Returns resolved and unresolved targets separately; link-only layout document stays unverified bytes. |
| Reader versus Writer | Reader receives traversal and detail results; DesignGraph mutation returns PERMISSION_DENIED for Reader. |
| Hidden endpoint | Unreadable message or component target yields the same generic state as a missing endpoint, without title, UID or existence leakage. |
| Operational failure | Store, decode, timeout and authorization-service failures remain RPC errors, not empty graphs. |

## Source isolation

| Case | Required public outcome |
| --- | --- |
| Source create | Incoming step or transition inventory rejects under every ownership policy, including TITLE-only. |
| Source whole owner | Nonempty current or incoming step collections reject, including proposed clear of steps. |
| Source field update | Supported unrelated field-specific writes preserve local topology exactly. Step or transition occurrence selection rejects through closed inventories. |

## Client and evidence boundaries

Native manifests and generated schema require typed step and transition shapes,
lossless uint64s, present/absent documentation and full step and connection
identity. CLI/MCP must demonstrate real native authoring, traversal, incoming
subflow query, reference and subject listing, export/diff/reapply and branch or
authorization forwarding. Validate-only preview proves no write, not persisted
reapply.

Studio may display loaded graph structure from backend reads and must not infer
topology from Storyboard canvas order, Document prose or Component membership
alone. Mark loaded-only correlation separately from traversal receipts.
Authenticated browser evidence names the actual backend fixture; component
tests and mocked gateway results do not establish deployment, native persistence
or runtime execution.

Implementation completion requires maintained executable fixtures and retained
commands and results at the candidate SHA. This proposal does not claim
EventCatalog import, `.ecstudio` parsing, diagram layout validation, workflow
execution, portable selector rebinding or atomic multi-owner snapshots.

## Coverage linkage

When implemented, add a dedicated server test module (for example
`design_topology_acceptance.rs`) and link it here. Until then, treat every case
above as pending. Do not check off the master checklist in
[eventcatalog-coverage.md](eventcatalog-coverage.md) from documentation alone.
