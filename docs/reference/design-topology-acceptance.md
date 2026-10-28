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
| Source guards, inventory | `rsworkspace/crates/trogon-atlas-core/tests/source_integration.rs` (`design_topology_steps_are_native_only_for_source_ownership`) |
| Source guards, server | `rsworkspace/crates/trogon-atlas-server/tests/design_topology_source_backend.rs` |
| Branch restoration | `rsworkspace/crates/trogon-atlas-server/tests/design_topology_branch_backend.rs` |
| Contract revision floor | `rsworkspace/crates/trogon-atlas-client/src/compat.rs` tests and the pinned revision tests in `rsworkspace/crates/trogon-atlas-core/tests/` |
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
| Initial Source create carrying topology rejects under title-only, unrelated-field, whole-owner and no-field policies | `source_create_of_design_topology_rejects_under_every_ownership_policy` |
| Source selections cannot address steps, entry steps or transitions | `source_selections_cannot_address_steps_or_transitions_under_any_policy` |
| WholeOwner rejects current topology and a proposed clear | `whole_owner_source_preview_rejects_current_and_cleared_incoming_topology` |
| Title, Profile and layout document updates preserve local topology exactly | `source_title_and_disposed_unrelated_fields_preserve_local_topology_exactly` |
| Branch keep-ours and take-theirs restoration renews only the restored step and its transitions | `keep_ours_restoration_renews_only_the_restored_step_and_transitions`, `take_theirs_restoration_renews_only_the_restored_step_and_transitions` |
| Prepared owner accepts exactly 1 MiB and rejects one byte more without changing the stored owner | `prepared_owner_accepts_exactly_one_mebibyte_and_rejects_one_byte_more` |
| Emitted incoming-subflow records stop at the output ceiling | `incoming_subflow_output_stops_at_the_emitted_record_ceiling` |
| Writers from before DesignGraph are refused in both directions | `server_from_before_design_graph_is_refused_for_every_intent`, `writer_from_before_design_graph_is_refused_by_a_current_server` |

## Source field disposition

`PROFILE` and `LAYOUT_DOCUMENT` are permitted field-specific Source units:
they reconcile without touching local steps, entry markers or transitions, and
the server test shows the topology unchanged after each. `STEPS`,
`ENTRY_STEPS` and transitions stay unsupported. The server disposition follows
the parent rejection inventory rather than assuming that a new enum value
implies permit or reject.

## Known gaps

- The system-group step target arm (field 10) remains an integration
  dependency until the frozen system-group schema arrives. It is not replaced
  by a Component target.
- The stored owner ceiling is lower than the validated 1 MiB. The store rejects
  a write whose encoded owner approaches the NATS message payload limit with an
  internal storage error, while validation accepts up to 1 MiB. The exact
  boundary is proven through validate-only requests, and a stored owner of just
  over 1,044,000 bytes is proven to persist. This affects every entity kind, so
  it needs a shared storage decision rather than a design-graph change.
- Composed membership with the sibling system-group change is verified at
  integration, not here.
- No deployed browser receipt exists for Studio.
