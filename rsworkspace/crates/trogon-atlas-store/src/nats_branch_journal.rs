use std::collections::BTreeSet;

use super::{
    branch_delta_key, journal, kv_entry_timeout, live_entry, MutationOp, MutationOutcome,
    NatsStore, Store, WriteContext, KV_OP_TIMEOUT,
};
use crate::{StoreError, StoreResult};

pub(super) struct BranchMutation {
    pub(super) key: String,
    pub(super) revision: u64,
    pub(super) value: Option<bytes::Bytes>,
}

impl NatsStore {
    pub(super) async fn batch_apply_branch(
        &self,
        ops: &[MutationOp],
        ctx: WriteContext<'_>,
    ) -> StoreResult<Vec<MutationOutcome>> {
        let branch = ctx
            .branch_name()
            .ok_or_else(|| StoreError::InvalidArgument("branch context required".into()))?;
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        let mut priors = Vec::new();
        let mut seen = BTreeSet::new();
        for (index, op) in ops.iter().enumerate() {
            let (kind, id) = match op {
                MutationOp::Create { kind, entity } | MutationOp::Put { kind, entity, .. } => (
                    *kind,
                    Self::validate(*kind, entity)
                        .map_err(|source| StoreError::BatchFailed {
                            index,
                            source: Box::new(source),
                        })?
                        .clone(),
                ),
                MutationOp::Delete { kind, id, .. } => (*kind, id.clone()),
            };
            let key = branch_delta_key(branch, kind, &id);
            if !seen.insert(key.clone()) {
                continue;
            }
            let entry = kv_entry_timeout(&self.branches_kv, key.clone()).await?;
            let before = match Store::get(self, kind, &id, Some(branch)).await {
                Ok(stored) => Some(stored.entity),
                Err(StoreError::NotFound) => None,
                Err(error) => return Err(error),
            };
            priors.push(journal::JournalPrior::branch(key, entry, before));
        }
        let mut journal = self.begin_journal(ctx, priors.clone()).await?;
        let write_ctx = ctx.without_changeset();
        let mut outcomes = Vec::with_capacity(ops.len());
        let mut changes = Vec::new();
        let mut applied = BTreeSet::new();
        let faults = self.faults();
        for (index, op) in ops.iter().enumerate() {
            if faults.crash_after_writes == Some(index) {
                return Err(journal::injected("crash"));
            }
            let (kind, id) = match op {
                MutationOp::Create { kind, entity } | MutationOp::Put { kind, entity, .. } => {
                    (*kind, Self::validate(*kind, entity)?.clone())
                }
                MutationOp::Delete { kind, id, .. } => (*kind, id.clone()),
            };
            let key = branch_delta_key(branch, kind, &id);
            let result = if faults.fail_write_at == Some(index) {
                Err(journal::injected("write"))
            } else {
                match op {
                    MutationOp::Create { kind, entity } => {
                        Store::create(self, *kind, entity, write_ctx)
                            .await
                            .map(|written| MutationOutcome::Wrote {
                                etag: written.etag,
                                entity: written.entity,
                            })
                    }
                    MutationOp::Put {
                        kind,
                        entity,
                        if_match,
                        force,
                    } => {
                        let written = match if_match {
                            Some(etag) => {
                                Store::update(self, *kind, entity, etag, *force, write_ctx).await
                            }
                            None => Store::put(self, *kind, entity, *force, write_ctx).await,
                        };
                        written.map(|written| {
                            if written.wrote {
                                MutationOutcome::Wrote {
                                    etag: written.etag,
                                    entity: written.entity,
                                }
                            } else {
                                MutationOutcome::Noop {
                                    etag: written.etag,
                                    entity: written.entity,
                                }
                            }
                        })
                    }
                    MutationOp::Delete { kind, id, if_match } => {
                        Store::delete(self, *kind, id, if_match.as_deref(), write_ctx)
                            .await
                            .map(|()| MutationOutcome::Deleted)
                    }
                }
            };
            match result {
                Ok(outcome) => {
                    if !matches!(outcome, MutationOutcome::Noop { .. }) {
                        applied.insert(key.clone());
                        let row = if faults.fail_branch_post_write_read_at == Some(index) {
                            Err(journal::injected("branch post-write read"))
                        } else {
                            kv_entry_timeout(&self.branches_kv, key.clone()).await
                        };
                        let row = match row {
                            Ok(row) => live_entry(row),
                            Err(error) => {
                                let source = self
                                    .undo_batch(journal, &priors, &applied)
                                    .await
                                    .unwrap_or(error);
                                return Err(StoreError::BatchFailed {
                                    index,
                                    source: Box::new(source),
                                });
                            }
                        };
                        let revision = super::Revision {
                            kind,
                            id,
                            change: if matches!(outcome, MutationOutcome::Deleted) {
                                super::ChangeKind::Delete
                            } else {
                                super::ChangeKind::Put
                            },
                            before: None,
                            after: None,
                        };
                        changes.push(journal::JournalChange::landed(
                            key,
                            &revision,
                            row.map_or(0, |row| row.revision),
                        ));
                    }
                    outcomes.push(outcome);
                }
                Err(source) => {
                    if matches!(source, StoreError::Backend(_) | StoreError::Unavailable(_)) {
                        applied.insert(key);
                    }
                    let source = self
                        .undo_batch(journal, &priors, &applied)
                        .await
                        .unwrap_or(source);
                    return Err(StoreError::BatchFailed {
                        index,
                        source: Box::new(source),
                    });
                }
            }
        }
        if faults.crash_after_writes == Some(ops.len()) {
            return Err(journal::injected("crash"));
        }
        if changes.is_empty() {
            journal.finish().await?;
            return Ok(outcomes);
        }
        if let Err(error) = journal.commit(changes).await {
            return Err(self
                .undo_batch(journal, &priors, &applied)
                .await
                .unwrap_or(error));
        }
        if faults.crash_after_commit {
            return Err(journal::injected("crash"));
        }
        if let Err(error) = self.publish_owned_journal(journal, &priors).await {
            self.config.metrics.record_batch_pending_recovery();
            tracing::warn!(%error, "branch batch history remains pending recovery");
        }
        Ok(outcomes)
    }

    pub(super) async fn apply_branch_maintenance(
        &self,
        branch: &str,
        pending: Vec<BranchMutation>,
    ) -> StoreResult<()> {
        if pending.is_empty() {
            return Ok(());
        }
        let mut priors = Vec::with_capacity(pending.len());
        for mutation in &pending {
            let entry = kv_entry_timeout(&self.branches_kv, mutation.key.clone()).await?;
            let found = live_entry(entry.clone()).map_or(0, |entry| entry.revision);
            if found != mutation.revision {
                return Err(StoreError::EtagMismatch {
                    expected: mutation.revision.to_string(),
                    found: found.to_string(),
                });
            }
            priors.push(journal::JournalPrior::branch(
                mutation.key.clone(),
                entry,
                None,
            ));
        }
        let mut journal = self
            .begin_scoped_journal(
                WriteContext::branch(branch),
                priors.clone(),
                true,
                Vec::new(),
            )
            .await?;
        let mut applied = BTreeSet::new();
        let faults = self.faults();
        for (index, mutation) in pending.iter().enumerate() {
            if faults.crash_after_writes == Some(index) {
                return Err(journal::injected("crash"));
            }
            applied.insert(mutation.key.clone());
            let write = async {
                if let Some(bytes) = &mutation.value {
                    self.branches_kv
                        .update(&mutation.key, bytes.clone(), mutation.revision)
                        .await
                        .map(|_| ())
                        .map_err(|error| StoreError::Backend(error.to_string()))
                } else {
                    self.branches_kv
                        .purge_expect_revision(&mutation.key, Some(mutation.revision))
                        .await
                        .map_err(|error| StoreError::Backend(error.to_string()))
                }
            };
            let result = tokio::time::timeout(KV_OP_TIMEOUT, write)
                .await
                .map_err(|_| StoreError::Unavailable("branch maintenance timed out".into()))
                .and_then(|result| result);
            if let Err(error) = result {
                return Err(self
                    .undo_batch(journal, &priors, &applied)
                    .await
                    .unwrap_or(error));
            }
        }
        if faults.crash_after_writes == Some(pending.len()) {
            return Err(journal::injected("crash"));
        }
        if let Err(error) = journal.commit(Vec::new()).await {
            return Err(self
                .undo_batch(journal, &priors, &applied)
                .await
                .unwrap_or(error));
        }
        if faults.crash_after_commit {
            return Err(journal::injected("crash"));
        }
        journal.finish().await
    }
}
