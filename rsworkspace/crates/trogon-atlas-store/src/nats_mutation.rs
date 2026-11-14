use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use super::NatsStore;
use crate::{StoreError, StoreResult};

tokio::task_local! {
    static MUTATION_HELD: Arc<tokio::sync::Mutex<()>>;
}

impl NatsStore {
    pub(super) async fn lock_native_mutation(
        &self,
    ) -> StoreResult<tokio::sync::MutexGuard<'_, ()>> {
        tokio::time::timeout(Duration::from_secs(60), self.native_mutation_lock.lock())
            .await
            .map_err(|_| StoreError::Unavailable("native mutation timed out".into()))
    }

    pub(super) fn mutation_scope<'a, T: Send + 'a>(
        &'a self,
        _guard: &'a tokio::sync::MutexGuard<'_, ()>,
        future: impl Future<Output = T> + Send + 'a,
    ) -> Pin<Box<dyn Future<Output = T> + Send + 'a>> {
        Box::pin(MUTATION_HELD.scope(self.native_mutation_lock.clone(), future))
    }

    pub(super) fn model_write_guarded<'a, T: Send + 'a>(
        &'a self,
        future: impl Future<Output = StoreResult<T>> + Send + 'a,
    ) -> Pin<Box<dyn Future<Output = StoreResult<T>> + Send + 'a>> {
        Box::pin(async move {
            if MUTATION_HELD
                .try_with(|held| Arc::ptr_eq(held, &self.native_mutation_lock))
                .unwrap_or(false)
            {
                return future.await;
            }
            let guard = self.lock_native_mutation().await?;
            self.mutation_scope(
                &guard,
                self.source_guarded(async {
                    self.journal_writes_ready().await?;
                    future.await
                }),
            )
            .await
        })
    }
}
