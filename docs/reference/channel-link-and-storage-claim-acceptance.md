# Channel link and storage claim acceptance

Status: coverage for the
[channel link and storage claim contract](channel-link-and-storage-claim-contracts.md).
Real-backend tests use the shared NATS test support. Studio tests use the
actual codecs without a browser.

## Executable coverage

| Requirement | Test |
| --- | --- |
| Link target, parameter and self-key admission | `channel_storage_backend::channel_links_admit_exact_targets_and_reject_invalid_parameter_shapes` |
| Claims on every owner kind, attachment and port rules | `channel_storage_backend::storage_claims_admit_exact_stores_attachments_and_ports_for_every_owner_kind` |
| Another exact version selectable, same exact Channel rejected | `channel_storage_backend::channel_links_select_another_exact_version_of_the_same_slug_but_never_the_same_channel` |
| Unchanged dangling links stay editable, new or edited ones reject | `channel_storage_backend::unchanged_dangling_channel_links_stay_editable_while_new_or_edited_unresolved_links_reject` |
| Unchanged dangling claims stay editable on every owner kind | `channel_storage_backend::unchanged_dangling_storage_claims_stay_editable_for_every_owner_kind` |
| Hidden and missing targets fail identically | `channel_storage_backend::hidden_and_missing_storage_targets_fail_identically` |
| Occurrence IDs, delete and retarget guards | `channel_storage_backend::references_carry_occurrence_ids_and_guard_delete_and_retarget` |
| Shared occurrence budgets | `channel_storage_backend::mixed_owner_inventories_share_the_occurrence_budget_across_links_and_claims` |
| Hidden referrers, final batch overlay | `channel_storage_backend::hidden_referrers_block_safe_delete_without_leaking_and_batch_overlay_is_final` |
| Port removal with a dependent claim | `channel_storage_backend::port_removal_must_carry_its_dependent_claim_in_the_same_guarded_mutation` |
| Source create, clear and title preservation | `source_integration_backend::association_source_title_only_create_rejects_without_native_or_mapping_base_publication` and `association_source_existing_owner_title_update_preserves_native_content_and_whole_owner_is_rejected` |
| Manifest schema equality | `committed_schema_file_matches_generator` and `trogon-atlas-client/tests/association_manifest.rs` |
| CLI authoring, export, no-op reapply | `trogon-atlas-cli/tests/association_cli_integration.rs` |
| MCP authoring, inspect, diff, export, reapply | `association_mcp_native_authoring_inspect_diff_export_and_reapply_preserve_identity` |
| Native CLI exact u64 authoring, attachments, parameter maps, changed diff, atomic refusals | `association_cli_authors_exact_u64_attachments_and_parameter_maps_diffs_real_changes_and_refuses_invalid_input` |
| Native MCP attachments, parameter maps, real changed diff, refusals | `association_mcp_round_trips_attachments_and_parameter_maps_diffs_real_changes_and_refuses_invalid_input` |
| CatalogSubject listing selects exact link and claim occurrences through add, remove and equal siblings, on every owner kind | `channel_storage_backend::catalog_subject_listing_selects_exact_link_and_claim_occurrences_through_their_lifecycle` |
| Responsibility discovery selects only the exact occurrence, retains unresolved assignments after removal, and does not inherit from the owner | `channel_storage_backend::catalog_subject_listing_selects_exact_link_and_claim_occurrences_through_their_lifecycle` |
| Branch-authored occurrences resolve only on the branch, hidden namespace and hidden branch answer like missing ones, native DiffBranch reports changed owners with kept ids | `channel_storage_backend::branch_authored_links_and_claims_resolve_only_on_the_branch_and_diff_as_real_changes` |
| Studio read-only codecs and Inspector | `ChannelStorageDetails.test.tsx` |
| Composed MessageClaims and Product association IDs cannot make an ambiguous storage selector resolve | `ChannelStorageDetails.test.tsx` |
| Decoded attachment hydration preserves exact identity in either owner order and becomes unavailable when the target is unloaded | `ChannelStorageDetails.test.tsx` |
| Studio diff component shows changed associations (component evidence only, not a native receipt) | `ConflictInspector.test.tsx` |

## Remaining acceptance

- Deployed browser evidence.

Hydration assertions exercise the actual protobuf codec and component model.
They do not establish deployed-browser behavior.
