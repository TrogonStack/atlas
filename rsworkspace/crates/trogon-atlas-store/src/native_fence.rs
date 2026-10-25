use std::{future::Future, pin::Pin};

use bytes::Bytes;
use prost::Message;

use super::{kv_entry_timeout, live_entry, NatsStore, KV_OP_TIMEOUT};
use crate::{StoreError, StoreResult};

pub(super) const GATE_KEY: &str = "publication-gate";

tokio::task_local! {
    static ACCESS: Access;
}

#[derive(Clone)]
struct Access {
    store: String,
    source: bool,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct Gate {
    #[prost(bool, tag = "1")]
    pub(super) busy: bool,
    #[prost(string, tag = "2")]
    pub(super) transaction: String,
    #[prost(uint64, tag = "3")]
    pub(super) writer_epoch: u64,
}

pub(super) fn blocked() -> StoreError {
    StoreError::Unavailable("native state is fenced pending source publication recovery".into())
}

impl NatsStore {
    pub(super) fn source_internal(&self) -> bool {
        ACCESS
            .try_with(|a| a.store == self.config.bucket && a.source)
            .unwrap_or(false)
    }

    pub(super) fn source_scope<'a, T: Send + 'a>(
        &'a self,
        future: impl Future<Output = T> + Send + 'a,
    ) -> Pin<Box<dyn Future<Output = T> + Send + 'a>> {
        Box::pin(ACCESS.scope(
            Access {
                store: self.config.bucket.clone(),
                source: true,
            },
            future,
        ))
    }

    pub(super) fn source_finalize_scope<'a, T: Send + 'a>(
        &'a self,
        future: impl Future<Output = T> + Send + 'a,
    ) -> Pin<Box<dyn Future<Output = T> + Send + 'a>> {
        Box::pin(ACCESS.scope(
            Access {
                store: self.config.bucket.clone(),
                source: false,
            },
            future,
        ))
    }

    pub(super) async fn load_source_gate(&self) -> StoreResult<(u64, Gate)> {
        match live_entry(kv_entry_timeout(&self.source_kv, GATE_KEY.into()).await?) {
            None => Ok((0, Gate::default())),
            Some(e) => Ok((
                e.revision,
                Gate::decode(e.value.as_ref()).map_err(|_| blocked())?,
            )),
        }
    }

    pub(super) async fn set_source_gate(&self, expected: u64, gate: Gate) -> StoreResult<u64> {
        self.require_writer()?;
        self.lease
            .verify_epoch(&self.lease_kv, self.lease.current_epoch())
            .await?;
        let bytes = Bytes::from(gate.encode_to_vec());
        if expected == 0 {
            tokio::time::timeout(KV_OP_TIMEOUT, self.source_kv.create(GATE_KEY, bytes))
                .await
                .map_err(|_| blocked())?
                .map_err(|_| blocked())
        } else {
            tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.source_kv.update(GATE_KEY, bytes, expected),
            )
            .await
            .map_err(|_| blocked())?
            .map_err(|_| blocked())
        }
    }

    pub(super) fn source_guarded<'a, T: Send + 'a>(
        &'a self,
        future: impl Future<Output = StoreResult<T>> + Send + 'a,
    ) -> Pin<Box<dyn Future<Output = StoreResult<T>> + Send + 'a>> {
        Box::pin(async move {
            if ACCESS
                .try_with(|a| a.store == self.config.bucket)
                .unwrap_or(false)
            {
                return future.await;
            }
            let _guard = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                self.source_native_lock.read(),
            )
            .await
            .map_err(|_| blocked())?;
            let (epoch, gate) = self.load_source_gate().await?;
            if gate.busy {
                return Err(blocked());
            }
            if let Some(observer) = &self.source_observer {
                observer.native_access_admitted()?;
            }
            let result = ACCESS
                .scope(
                    Access {
                        store: self.config.bucket.clone(),
                        source: false,
                    },
                    future,
                )
                .await;
            let (after, gate) = self.load_source_gate().await?;
            if gate.busy || epoch != after {
                return Err(blocked());
            }
            result
        })
    }
}
