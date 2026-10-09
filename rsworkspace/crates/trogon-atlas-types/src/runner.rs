use std::{sync::Arc, time::Duration};

use tokio::sync::Semaphore;

use crate::compile::{compile, CompileError, CompiledLibrary, TypeLibrarySource};

/// Keeps protobuf compiles off the async executor and bounds how many run
/// at once and how long a caller waits for one.
#[derive(Debug, Clone)]
pub struct CompileRunner {
    permits: Arc<Semaphore>,
    timeout: Duration,
}

impl Default for CompileRunner {
    fn default() -> Self {
        Self::new(4, Duration::from_secs(2))
    }
}

impl CompileRunner {
    pub fn new(concurrency: usize, timeout: Duration) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(concurrency)),
            timeout,
        }
    }

    pub async fn compile(
        &self,
        library: TypeLibrarySource,
        dependencies: Vec<TypeLibrarySource>,
    ) -> Result<CompiledLibrary, CompileError> {
        let permits = Arc::clone(&self.permits);
        let run = async move {
            let permit = permits
                .acquire_owned()
                .await
                .map_err(|_| CompileError::Unavailable)?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                compile(&library, &dependencies)
            })
            .await
            .map_err(|_| CompileError::Unavailable)?
        };
        tokio::time::timeout(self.timeout, run)
            .await
            .map_err(|_| CompileError::TimedOut(self.timeout))?
    }
}
