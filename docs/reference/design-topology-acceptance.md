# Design graph topology acceptance requirements

Status: requirements for the
[design topology contract](design-topology-contracts.md), mapped to executed
tests under [Coverage linkage](#coverage-linkage). Deployed readiness and
authenticated browser rendering are not claimed. Logical names below are not
manifest syntax, predicted owner UIDs or submitted occurrence IDs.

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
| Complete fixture create/read | Fields preserve profile, Unicode titles, step order, transition order and distinct step IDs for equal catalog targets. |
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

Server public-service acceptance lives in
[`design_topology_acceptance.rs`](../../rsworkspace/crates/trogon-atlas-server/tests/design_topology_acceptance.rs).
It drives the public service with the shared NATS-backed store and complements
the preserved backend, branch and Source suites
(`design_topology_backend.rs`, `design_topology_branch_backend.rs`,
`design_topology_source_backend.rs`).

| Requirement area | Executed test |
| --- | --- |
| Unicode, order, presence, equal catalog selections with distinct step IDs | `complete_fixture_preserves_unicode_order_presence_and_distinct_ids_for_equal_catalog_selections` |
| Every adopted target arm, wrong kind and version, dangling and inline targets | `every_adopted_target_arm_admits_exact_selections_and_rejects_wrong_kind_version_and_dangling_refs` |
| Layout document present, absent, unavailable and missing | `layout_document_state_is_absent_resolved_or_unavailable_and_missing_layouts_are_rejected` |
| Invalid local IDs, transitions, entries and same-write unknowns leave state unchanged | `invalid_local_ids_transitions_entries_and_same_write_unknowns_leave_stored_state_unchanged` |
| Two-write staging and stale etags | `two_write_staging_returns_ids_first_then_accepts_transitions_and_rejects_stale_etags` |
| Label retention, endpoint renewal, reorder, remove and re-add | `transition_identity_follows_label_retention_endpoint_renewal_reorder_and_remove_readd` |
| Linear, branch and loop traversal receipts | `linear_branch_and_loop_traversals_return_exact_nodes_edges_entries_and_cycles` |
| Depth default, cap and exact completion | `subflow_depth_boundaries_default_cap_and_exact_completion_are_reported` |
| Subflow cycles | `subflow_cycles_report_their_graph_path_and_both_edges` |
| Authorization lens outage fails every query | `authorization_lens_outage_fails_all_four_queries_instead_of_returning_empty_graphs` |
| Hidden and missing endpoints share one state and diagnostic | `hidden_and_missing_endpoints_share_one_state_and_one_diagnostic_across_every_read` |
| Store, decode and authorizer failures are operational | `store_decode_faults_and_outages_are_operational_failures_not_empty_or_unavailable_targets` |
| Step, transition and local identity maxima, then overflow by one | `step_transition_and_local_identity_maxima_are_accepted_then_overflow_by_one` |
| Subject maximum and local identity ceiling | `subject_maximum_counts_toward_the_local_identity_ceiling_and_overflows_by_one` |
| Traversal work budget, owner charged once | `traversal_work_budget_charges_each_distinct_owner_once_and_exhausts_on_the_tenth` |
| Traversal output budget | `traversal_output_budget_exhausts_on_the_expansion_that_crosses_4096_rows` |
| Explicit subjects, target-only negative, Reader query and denied mutation | `subject_listing_matches_explicit_subjects_only_and_readers_query_but_cannot_mutate` |
| Reference multiplicity, hidden safe delete, recreation staleness | `references_keep_parallel_multiplicity_hidden_referrers_block_delete_and_recreation_is_stale` |
| Storyboard control unchanged | `storyboard_slices_and_traces_stay_unchanged_when_design_graphs_are_added` |

Client and surface evidence:

| Surface | Executed test |
| --- | --- |
| CLI authenticated apply, exact detail, subject, traversal and incoming queries, export, diff, reapply, owned branch apply and merge | `authenticated_design_topology_cli_returns_exact_queries_exports_without_drift_and_follows_branches` in [`design_topology_cli_integration.rs`](../../rsworkspace/crates/trogon-atlas-cli/tests/design_topology_cli_integration.rs) |
| MCP apply, exact detail, subject listing, traversal and incoming subflows | `design_topology_mcp_apply_inspect_traverse_and_incoming` in [`design_topology_mcp_integration.rs`](../../rsworkspace/crates/trogon-atlas-mcp/tests/design_topology_mcp_integration.rs) |
| Studio gateway response identities, presence and unavailable states through the production codec | `design topology gateway response codec` in [`index.test.mjs`](../../tsworkspace/webapps/studio/server/index.test.mjs) |

## Proof boundaries

- `prepared_owner_accepts_exactly_one_mebibyte_and_rejects_one_byte_more`
  proves validate-only admission. Persisted complete-owner capacity is covered
  separately by
  [`complete_prepared_graph_capacity_persists_exact_boundary_and_refuses_one_byte_more`](../../rsworkspace/crates/trogon-atlas-server/tests/document_catalog_context_backend.rs):
  the authenticated public service stores exactly 1,048,576 encoded bytes,
  including assigned identities and server metadata, and rejects the next byte
  without changing the owner, etag or change sequence. That fixture covers
  baseline publication; broader branch-publication capacity assertions use
  Document owners.
- The resource-row budget cannot be reached through traversal because the work
  budget is exhausted first. Only the work and output budgets are exercised.
- The owned branch lifecycle through the CLI stops at apply and branch-scoped
  reads: `branch create`, `diff` and `merge` reject owned branch names on the
  client, so creation, diff and merge are driven through the public gRPC
  service in the same test.
- The MCP test runs against an unauthenticated server. MCP Reader discovery of
  the four tools depends on the tool policy rows in the MCP server and is
  proved separately by that crate's role map test.
- System grouping (union arm 10) is a dependency of the separate system
  grouping contract and is not integrated or tested here.
- Browser rendering with an authenticated backend is not proved. No maintained
  authenticated browser fixture exists, and component tests with mocked
  gateways do not establish it.

Do not check off the master checklist in
[eventcatalog-coverage.md](eventcatalog-coverage.md) from this document alone.
