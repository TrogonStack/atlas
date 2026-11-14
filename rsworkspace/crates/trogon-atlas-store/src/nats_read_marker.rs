use async_nats::jetstream::{kv::Store as KvStore, stream::Info};

use super::{writer_lease, NatsStore, KV_OP_TIMEOUT};
use crate::{
    store::{BucketReadRevision, StoreNamespaceMarker, StoreReadMarker},
    StoreError, StoreResult,
};

async fn info(kv: &KvStore) -> StoreResult<Info> {
    tokio::time::timeout(KV_OP_TIMEOUT, kv.stream.get_info())
        .await
        .map_err(|_| StoreError::Unavailable("read marker timed out".into()))?
        .map_err(|_| StoreError::Unavailable("read marker unavailable".into()))
}

fn stamp(info: &Info) -> BucketReadRevision {
    BucketReadRevision {
        incarnation: info.created.to_string(),
        sequence: info.state.last_sequence,
    }
}

fn busy() -> StoreError {
    StoreError::Unavailable("model publication is in flight or awaiting recovery".into())
}

impl NatsStore {
    pub(super) async fn capture_read_marker(&self) -> StoreResult<StoreReadMarker> {
        let (source_before, gate) = self.load_source_gate().await?;
        if gate.busy {
            return Err(busy());
        }
        let journal_before = info(&self.batches_kv).await?;
        if journal_before.state.messages != 0 {
            return Err(busy());
        }
        let mut revisions = Vec::new();
        for kv in [
            &self.kv,
            &self.branches_kv,
            &self.namespaces_kv,
            &self.source_kv,
            &self.changesets_kv,
            &self.revisions_kv,
            &self.protections_kv,
        ] {
            revisions.push(stamp(&info(kv).await?));
        }
        let namespace = StoreNamespaceMarker(revisions[2].clone());
        let lease = info(&self.lease_kv).await?;
        revisions.push(BucketReadRevision {
            incarnation: lease.created.to_string(),
            sequence: 0,
        });
        let writer = writer_lease::WriterLease::read_identity(&self.lease_kv).await?;
        let journal_after = info(&self.batches_kv).await?;
        let (source_after, gate) = self.load_source_gate().await?;
        if gate.busy
            || source_before != source_after
            || journal_after.state.messages != 0
            || stamp(&journal_before) != stamp(&journal_after)
        {
            return Err(busy());
        }
        revisions.push(stamp(&journal_after));
        Ok(StoreReadMarker {
            revisions,
            namespace,
            writer,
        })
    }
}
