# Channel link and storage claim acceptance

Status: coverage for the
[channel link and storage claim contract](channel-link-and-storage-claim-contracts.md).
Real-backend tests use the shared NATS test support. Studio tests use the
actual codecs without a browser.

## Executable coverage

| Requirement | Test |
| --- | --- |
| Link target, parameter and self-key admission | `resource_association_backend::channel_links_admit_exact_targets_and_reject_invalid_parameter_shapes` |
| Claims on every owner kind, attachment and port rules | `resource_association_backend::storage_claims_admit_exact_stores_attachments_and_ports_for_every_owner_kind` |
| Another exact version selectable, same exact Channel rejected | `resource_association_backend::channel_links_select_another_exact_version_of_the_same_slug_but_never_the_same_channel` |
| Unchanged dangling links stay editable, new or edited ones reject | `resource_association_backend::unchanged_dangling_channel_links_stay_editable_while_new_or_edited_unresolved_links_reject` |
| Unchanged dangling claims stay editable on every owner kind | `resource_association_backend::unchanged_dangling_storage_claims_stay_editable_for_every_owner_kind` |
| Hidden and missing targets fail identically | `resource_association_backend::hidden_and_missing_storage_targets_fail_identically` |
| Occurrence IDs, delete and retarget guards | `resource_association_backend::references_carry_occurrence_ids_and_guard_delete_and_retarget` |
| Shared occurrence budgets | `resource_association_backend::mixed_owner_inventories_share_the_occurrence_budget_across_links_and_claims` |
| Hidden referrers, final batch overlay | `resource_association_backend::hidden_referrers_block_safe_delete_without_leaking_and_batch_overlay_is_final` |
| Port removal with a dependent claim | `resource_association_backend::port_removal_must_carry_its_dependent_claim_in_the_same_guarded_mutation` |
| Source create, clear and title preservation | `source_integration_backend::association_source_title_only_create_rejects_without_native_or_mapping_base_publication` and `association_source_existing_owner_title_update_preserves_native_content_and_whole_owner_is_rejected` |
| Manifest schema equality | `committed_schema_file_matches_generator` and `trogon-atlas-client/tests/association_manifest.rs` |
| CLI authoring, export, no-op reapply | `trogon-atlas-cli/tests/association_cli_integration.rs` |
| MCP authoring, inspect, diff, export, reapply | `association_mcp_native_authoring_inspect_diff_export_and_reapply_preserve_identity` |
| Studio read-only codecs and Inspector | `ChannelStorageDetails.test.tsx` |

## Remaining acceptance

- Branch-specific visibility and hydration order for links and claims.
- Explicit CatalogSubject listing tests for link and claim connection subjects.
- A Studio-visible diff and an end-to-end diff test of links and claims.
- Deployed browser evidence.
