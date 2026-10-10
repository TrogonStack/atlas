// Branch lifecycle (Phase 1: Isolation, see docs/explanation/branching.md).
// These three RPCs manage branch metadata only; branch *scope* for every
// other RPC travels via the `x-trogon-atlas-branch` metadata header injected
// by `ConnectOptions::branch` / `BearerAuth`, never through these functions.
//
// Review and merge (Phase 2, same doc) lives below: DiffBranch/MergeBranch/
// UpdateBranch/ResolveBranchEntry all take the branch `name` as an explicit
// request field rather than via the header, matching the three lifecycle
// RPCs above.

use anyhow::{Context as _, Result};
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    failure::invalid_input,
    manifest::entity_to_json,
    precondition::{BranchEntryState, StaleResolution},
};

/// Validate a branch name the same way the server does (Phase 1: Isolation):
/// at most one `/`-separated segment pair, each segment a safe id component,
/// and not the reserved value `"meta"` (case-insensitive). Surfaces authoring
/// errors before a gRPC connect so an unreachable endpoint cannot mask them.
pub fn validate_branch_name(name: &str) -> Result<()> {
    if name.is_empty() {
        crate::invalid_input!("branch name must not be empty");
    }
    if name.eq_ignore_ascii_case("meta") {
        crate::invalid_input!("branch name {name:?} is reserved");
    }
    let mut segments = name.splitn(3, '/');
    let first = segments.next().unwrap_or_default();
    let second = segments.next();
    if segments.next().is_some() {
        crate::invalid_input!("branch name must contain at most one '/'");
    }
    trogon_atlas_core::validate_id_component(first)
        .map_err(|e| invalid_input(format!("branch name segment {e}")))?;
    if let Some(second) = second {
        trogon_atlas_core::validate_id_component(second)
            .map_err(|e| invalid_input(format!("branch name segment {e}")))?;
    }
    Ok(())
}

/// Create a new branch. Fails if a branch with this name already exists.
pub async fn create_branch(client: &mut Client, name: &str, doc: &str) -> Result<pb::BranchInfo> {
    let resp = client
        .create_branch(pb::CreateBranchRequest {
            name: name.to_string(),
            doc: doc.to_string(),
        })
        .await
        .context("CreateBranch failed")?
        .into_inner();
    resp.branch
        .context("CreateBranch response did not include a branch")
}

/// List every registered branch, in server-defined order.
pub async fn list_branches(client: &mut Client) -> Result<Vec<pb::BranchInfo>> {
    let resp = client
        .list_branches(pb::ListBranchesRequest {})
        .await
        .context("ListBranches failed")?
        .into_inner();
    Ok(resp.branches)
}

/// Delete a branch and every delta it holds. Entities previously visible
/// only through this branch's deltas become unreachable (or fall through to
/// baseline, if baseline has the key) once this returns.
pub async fn delete_branch(client: &mut Client, name: &str) -> Result<()> {
    client
        .delete_branch(pb::DeleteBranchRequest {
            name: name.to_string(),
        })
        .await
        .context("DeleteBranch failed")?;
    Ok(())
}

/// Diff a branch's live deltas against the current baseline. Read-only:
/// never mutates branch or baseline state.
pub async fn diff_branch(client: &mut Client, name: &str) -> Result<Vec<pb::BranchDiffEntry>> {
    let resp = client
        .diff_branch(pb::DiffBranchRequest {
            name: name.to_string(),
        })
        .await
        .context("DiffBranch failed")?
        .into_inner();
    Ok(resp.entries)
}

/// How a merge should behave, separate from which branch it merges. Every
/// knob defaults to the conservative reading: persist the result, delete the
/// merged branch, and treat every conflict class as a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MergeOptions {
    /// Run full detection and validation, persist nothing even on success.
    pub dry_run: bool,
    /// Keep the branch (now empty of deltas) after a successful merge.
    pub keep_branch: bool,
    /// Land the structural three-way merge for edit/edit conflicts whose two
    /// sides touched disjoint field paths, instead of reporting them as
    /// conflicts. Same-path collisions stay conflicts regardless.
    pub auto_merge: bool,
}

/// Merge a branch onto baseline. All-or-nothing: any conflict (see
/// `resp.conflicts`) or post-merge validation Error (see `resp.validation`)
/// persists nothing. `operation_id` is dropped when `options.dry_run` is
/// set: the server rejects an `operation_id` paired with `dry_run`, since a
/// dry run never produces a write for a later replay to recover.
pub async fn merge_branch(
    client: &mut Client,
    name: &str,
    options: MergeOptions,
    operation_id: Option<&str>,
) -> Result<pb::MergeBranchResponse> {
    let operation_id = if options.dry_run {
        String::new()
    } else {
        operation_id.unwrap_or_default().to_string()
    };
    let resp = client
        .merge_branch(pb::MergeBranchRequest {
            name: name.to_string(),
            dry_run: options.dry_run,
            keep_branch: options.keep_branch,
            auto_merge: options.auto_merge,
            operation_id,
        })
        .await
        .context("MergeBranch failed")?
        .into_inner();
    Ok(resp)
}

/// Rebase every non-conflicting delta on `branch` forward to the current
/// baseline (CONVERGED entries collapse; others advance `base`/`base_etag`).
/// Conflicting entries are left untouched and returned in `conflicts` for
/// `resolve_branch_entry` to handle.
pub async fn update_branch(client: &mut Client, name: &str) -> Result<pb::UpdateBranchResponse> {
    let resp = client
        .update_branch(pb::UpdateBranchRequest {
            name: name.to_string(),
        })
        .await
        .context("UpdateBranch failed")?
        .into_inner();
    Ok(resp)
}

/// Resolution for a single conflicting branch entry, mirroring
/// `ResolveBranchEntryRequest.Resolution` without exposing the raw i32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Overwrite the branch's delta with the current baseline content (or
    /// remove the delta entirely if baseline no longer has the key).
    TakeTheirs,
    /// Keep the branch's current value, rebasing `base`/`base_etag`
    /// forward to the current baseline so the conflict clears.
    KeepOurs,
    /// Take the entry's structural three-way merge as the branch's new
    /// value, rebasing forward so the conflict clears. Only available for
    /// an edit/edit conflict whose two sides touched disjoint field paths.
    TakeMerged,
}

impl From<Resolution> for pb::resolve_branch_entry_request::Resolution {
    fn from(r: Resolution) -> Self {
        match r {
            Resolution::TakeTheirs => pb::resolve_branch_entry_request::Resolution::TakeTheirs,
            Resolution::KeepOurs => pb::resolve_branch_entry_request::Resolution::KeepOurs,
            Resolution::TakeMerged => pb::resolve_branch_entry_request::Resolution::TakeMerged,
        }
    }
}

/// Resolve a single conflicting entry on `branch`, bound to the state the
/// caller inspected (`BranchDiffEntry.state`). If any side moved since, the
/// server refuses and this returns a `StaleResolution` naming both states.
pub async fn resolve_branch_entry(
    client: &mut Client,
    name: &str,
    entity_ref: pb::EntityRef,
    resolution: Resolution,
    expected: &BranchEntryState,
    operation_id: Option<&str>,
) -> Result<()> {
    let entity = entity_key(&entity_ref)?;
    let result = client
        .resolve_branch_entry(pb::ResolveBranchEntryRequest {
            name: name.to_string(),
            r#ref: Some(entity_ref.clone()),
            resolution: pb::resolve_branch_entry_request::Resolution::from(resolution) as i32,
            expected_state: Some(expected.to_wire()),
            operation_id: operation_id.unwrap_or_default().to_string(),
        })
        .await;
    match result {
        Ok(_) => Ok(()),
        Err(status) if status.code() == tonic::Code::Aborted => {
            let actual = current_entry_state(client, name, &entity_ref).await?;
            Err(anyhow::Error::new(StaleResolution {
                branch: name.to_string(),
                entity,
                expected: expected.clone(),
                actual,
            }))
        }
        Err(status) => Err(anyhow::Error::new(status).context("ResolveBranchEntry failed")),
    }
}

fn entity_key(entity_ref: &pb::EntityRef) -> Result<pb::EntityKey> {
    let kind = pb::EntityKind::try_from(entity_ref.kind)
        .map_err(|_| anyhow::anyhow!("unknown entity kind {}", entity_ref.kind))?;
    let id = entity_ref.id.as_ref().context("entity ref has no id")?;
    Ok(pb::EntityKey::new(kind, id))
}

async fn current_entry_state(
    client: &mut Client,
    name: &str,
    entity_ref: &pb::EntityRef,
) -> Result<Option<BranchEntryState>> {
    diff_branch(client, name)
        .await?
        .iter()
        .find(|entry| entry.r#ref.as_ref() == Some(entity_ref))
        .and_then(|entry| entry.state.as_ref())
        .map(BranchEntryState::from_wire)
        .transpose()
}

fn diff_entry_label(entry: &pb::BranchDiffEntry) -> String {
    entry
        .r#ref
        .as_ref()
        .and_then(|r| {
            let kind = pb::EntityKind::try_from(r.kind).ok()?;
            let id = r.id.as_ref()?;
            Some(pb::canonical::id_string(kind, id))
        })
        .unwrap_or_else(|| "<unknown>".to_string())
}

fn status_label(status: i32) -> &'static str {
    match pb::branch_diff_entry::Status::try_from(status) {
        Ok(pb::branch_diff_entry::Status::Added) => "added",
        Ok(pb::branch_diff_entry::Status::Changed) => "changed",
        Ok(pb::branch_diff_entry::Status::Deleted) => "deleted",
        Ok(pb::branch_diff_entry::Status::Converged) => "converged",
        Ok(pb::branch_diff_entry::Status::ConflictEditEdit) => "conflict (edit/edit)",
        Ok(pb::branch_diff_entry::Status::ConflictEditDelete) => "conflict (edit/delete)",
        Ok(pb::branch_diff_entry::Status::ConflictDeleteEdit) => "conflict (delete/edit)",
        _ => "unspecified",
    }
}

/// Server-owned metadata is stamped on write; strip it so a branch diff
/// matches `semantic_eq` / `apply::render_diff` and does not report
/// system-only noise as content drift.
fn strip_system(entity: &pb::Entity) -> pb::Entity {
    let mut e = entity.clone();
    e.system = None;
    e
}

/// Render a branch diff as a unified diff of canonical proto JSON per
/// entry, kubectl-diff style, reusing the same rendering convention as
/// `apply::render_diff`. `theirs` (or `base`, if baseline has no key) is
/// treated as "live"; `ours` is treated as the proposed change.
pub fn render_diff(entries: &[pb::BranchDiffEntry]) -> Result<String> {
    use std::fmt::Write as _;
    let mut out = String::new();
    for entry in entries {
        let label = diff_entry_label(entry);
        let before = match entry.theirs.as_ref().or(entry.base.as_ref()) {
            Some(e) => entity_to_json(&strip_system(e))? + "\n",
            None => String::new(),
        };
        let after = match entry.ours.as_ref() {
            Some(e) => entity_to_json(&strip_system(e))? + "\n",
            None => String::new(),
        };
        let _ = writeln!(out, "diff {label} ({})", status_label(entry.status));
        if let Some(state) = entry.state.as_ref() {
            let _ = writeln!(out, "  state: {}", BranchEntryState::from_wire(state)?);
        }
        if before == after {
            continue;
        }
        let diff = similar::TextDiff::from_lines(&before, &after);
        let _ = write!(
            out,
            "{}",
            diff.unified_diff()
                .context_radius(3)
                .header(&format!("baseline/{label}"), &format!("branch/{label}"))
        );
        if !entry.conflict_field_paths.is_empty() {
            let _ = writeln!(
                out,
                "  conflicting fields: {}",
                entry.conflict_field_paths.join(", ")
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn event(title: &str, system: Option<pb::SystemMeta>) -> pb::Entity {
        pb::Entity {
            system,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "shop".into(),
                    slug: "order.placed".into(),
                    version: 1,
                }),
                title: title.into(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn validate_branch_name_rejects_spaces_and_reserved_meta() {
        assert!(validate_branch_name("ok-branch").is_ok());
        assert!(validate_branch_name("team/feature-x").is_ok());
        assert!(validate_branch_name("").is_err());
        assert!(validate_branch_name("meta").is_err());
        assert!(validate_branch_name("META").is_err());
        assert!(validate_branch_name("bad name").is_err());
        assert!(validate_branch_name("a/b/c").is_err());
    }

    #[test]
    fn render_diff_ignores_system_meta_on_converged_entries() {
        // Classification uses semantic_eq (system-blind). Rendering must
        // match: otherwise CONVERGED entries with distinct stamps print a
        // spurious unified diff of server-owned metadata only.
        let entries = vec![pb::BranchDiffEntry {
            r#ref: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(pb::Id {
                    namespace: "shop".into(),
                    slug: "order.placed".into(),
                    version: 1,
                }),
            }),
            status: pb::branch_diff_entry::Status::Converged as i32,
            base: None,
            base_etag: String::new(),
            ours: Some(event(
                "Same",
                Some(pb::SystemMeta {
                    uid: "branch-uid".into(),
                    created_at: "2026-01-01T00:00:00Z".into(),
                }),
            )),
            theirs: Some(event(
                "Same",
                Some(pb::SystemMeta {
                    uid: "baseline-uid".into(),
                    created_at: "2026-02-01T00:00:00Z".into(),
                }),
            )),
            conflict_field_paths: vec![],
            auto_merged: None,
            state: None,
        }];
        let rendered = render_diff(&entries).unwrap();
        assert!(
            rendered.contains("converged"),
            "status line must remain: {rendered}"
        );
        assert!(
            !rendered.contains("@@") && !rendered.contains("uid"),
            "system-only noise must not appear in a converged diff: {rendered}"
        );
    }
}
