# Design graph topology acceptance

Status: the cases below have maintained executable coverage in the named tests.
Coverage runs against the real shared store and authenticated gRPC, and the CLI
and MCP tests drive the public clients against that backend. No deployed browser
receipt exists, and Studio tests use mocked gateway results, so they establish
rendering and request shape only. The
[contract](design-topology-contracts.md) defines the behavior.

## Executable coverage

| Surface | Test |
| --- | --- |
| Server | `rsworkspace/crates/trogon-atlas-server/tests/design_topology_backend.rs` |
| Client transport | `rsworkspace/crates/trogon-atlas-client/tests/design_topology_transport.rs` |
| MCP | `rsworkspace/crates/trogon-atlas-mcp/tests/design_topology_mcp_integration.rs` |
| CLI | `rsworkspace/crates/trogon-atlas-cli/tests/design_topology_cli_integration.rs` |
| Source guards | `rsworkspace/crates/trogon-atlas-core/tests/source_integration.rs` (`design_topology_steps_are_native_only_for_source_ownership`) |
| Studio | `tsworkspace/webapps/studio/src/components/DesignGraphInspector.test.tsx`, `src/lib/design-topology-api.test.ts`, `server/design-topology-contract.test.mjs` |

## Case map

| Case | Test |
| --- | --- |
| Two-phase authoring, returned IDs, equal parallel transitions | `two_phase_authoring_returns_ids_and_allows_equal_parallel_transitions` |
| Unknown step IDs, self-subflow, Reader write denied | `authoring_rejects_unknown_step_ids_self_subflow_and_reader_writes` |
| Subflow depth, default and capped depth, unfollowed markers, disconnected root steps, incoming subflows | `traversal_counts_subflow_depth_reports_cycles_and_budgets` |
| Subflow cycle without infinite expansion | `subflow_cycles_give_a_diagnostic_without_infinite_expansion` |
| Hidden and missing targets share one state | `hidden_and_missing_targets_share_one_generic_state` |
| Safe and force delete | `safe_delete_protects_subflow_targets_and_force_delete_leaves_exact_refs` |
| Step stripping, changed content, separate writes | `stripped_step_id_is_rejected_while_changed_content_and_separate_writes_are_new_nodes` |
| Equal siblings keep distinct IDs | `equal_sibling_steps_keep_distinct_ids_across_resubmission` |
| Collection ceilings | `owner_limits_reject_oversized_step_and_transition_collections` |
| Native metadata on the kind | `lifecycle_metadata_is_supported_on_the_design_graph_kind` |
| Same-owner transition descriptors in reference discovery | `outgoing_references_surface_same_owner_transitions_with_internal_descriptors` |
| Branch, auth and failure-code forwarding | `design_topology_queries_preserve_branch_auth_and_operational_failure_codes` |
| MCP apply, inspect, traverse, incoming | `design_topology_mcp_apply_inspect_traverse_and_incoming` |
| CLI two-phase apply, get, query, idempotent reapply | `design_topology_cli_two_phase_apply_get_and_query` |
| Malformed query fails before connecting | `malformed_design_topology_query_fails_before_connecting` |

## Known gaps

- No server-level Source guard test exists; the guard is covered at the core
  inventory and by shared server code paths only.
- Branch merge conflict renewal of deleted step IDs has no dedicated
  design-graph fixture; it relies on the shared occurrence rules.
- Request budget overflow is enforced through the shared catalog query budget
  and its RESOURCE_EXHAUSTED mapping is asserted at the client transport, but no
  design-graph fixture drives a real overflow. The 1 MiB encoded boundary has
  no separate fixture beyond the collection ceilings.
- Composed membership with the sibling system-group change is verified at
  integration, not here.
