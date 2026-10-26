//! A rayon pool behind a tokio admission semaphore.

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use firespread_ensemble::panic_message;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_rayon::{AsyncThreadPool, rayon};

use crate::{AgentError, Result};

/// A rayon pool of `threads` threads and a semaphore of `max_in_flight`
/// permits. Each job holds one permit from before it is queued on the pool
/// until its result is observed, so at most `max_in_flight` jobs are queued
/// or running at once.
#[derive(Debug)]
pub struct Executor {
    pool: rayon::ThreadPool,
    permits: Arc<Semaphore>,
    threads: usize,
    max_in_flight: usize,
}

impl Executor {
    /// A pool of `threads` threads named `compute-<i>` and `max_in_flight`
    /// admission permits.
    ///
    /// # Errors
    /// [`AgentError::InvalidExecutor`] when either value is 0, and
    /// [`AgentError::ExecutorBuild`] when rayon cannot build the pool.
    pub fn new(threads: usize, max_in_flight: usize) -> Result<Self> {
        for (field, value) in [("threads", threads), ("max_in_flight", max_in_flight)] {
            if value == 0 {
                return Err(AgentError::InvalidExecutor { field, value });
            }
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("compute-{i}"))
            .build()
            .map_err(|source| AgentError::ExecutorBuild { threads, source })?;
        Ok(Self {
            pool,
            permits: Arc::new(Semaphore::new(max_in_flight)),
            threads,
            max_in_flight,
        })
    }

    /// [`Executor::new`] with `2 * threads` admission permits.
    ///
    /// # Errors
    /// The errors of [`Executor::new`].
    pub fn with_threads(threads: usize) -> Result<Self> {
        Self::new(threads, threads.saturating_mul(2))
    }

    /// Threads of the pool.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Admission permits.
    #[must_use]
    pub fn max_in_flight(&self) -> usize {
        self.max_in_flight
    }

    /// Waits for one admission permit.
    ///
    /// # Errors
    /// [`AgentError::AdmissionClosed`] when the semaphore is closed.
    pub async fn admit(&self, stage: &'static str) -> Result<OwnedSemaphorePermit> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| AgentError::AdmissionClosed { stage })
    }

    /// Queues `f` on the pool now and returns a future of its value that
    /// releases `permit` once the value is observed or the future is
    /// dropped. A panic in `f` resumes on the task that awaits the future.
    pub fn submit<F, R>(
        &self,
        permit: OwnedSemaphorePermit,
        f: F,
    ) -> impl Future<Output = R> + Send + 'static
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let handle = self.pool.spawn_async(f);
        async move {
            let value = handle.await;
            drop(permit);
            value
        }
    }

    /// Admits, runs `f` on the pool and returns its value, with a panic in
    /// `f` mapped to [`AgentError::Panicked`] carrying `stage`.
    ///
    /// # Errors
    /// [`AgentError::AdmissionClosed`] and [`AgentError::Panicked`].
    pub async fn run<F, R>(&self, stage: &'static str, f: F) -> Result<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let permit = self.admit(stage).await?;
        self.submit(permit, move || catch_unwind(AssertUnwindSafe(f)))
            .await
            .map_err(|payload| AgentError::Panicked {
                stage,
                message: panic_message(payload.as_ref()),
            })
    }
}
