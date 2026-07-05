use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use futures_util::future::{BoxFuture, FutureExt, Shared};

use crate::error::AppError;
use crate::locs::Locs;

type JobResult = Result<Arc<Locs>, AppError>;
type JobFuture = Shared<BoxFuture<'static, JobResult>>;

// Deduplicates concurrent work per key: joins callers onto one job that always runs to completion, regardless of caller cancellation.
#[derive(Clone, Default)]
pub struct Inflight {
    jobs: Arc<Mutex<HashMap<String, JobFuture>>>,
}

impl Inflight {
    pub async fn run<F>(&self, key: String, job: F) -> JobResult
    where
        F: Future<Output = JobResult> + Send + 'static,
    {
        let (shared, is_new) = {
            let mut jobs = self.jobs.lock().unwrap();
            match jobs.entry(key.clone()) {
                Entry::Occupied(entry) => (entry.get().clone(), false),
                Entry::Vacant(entry) => {
                    let handle = tokio::spawn(job);
                    let shared: JobFuture = async move {
                        match handle.await {
                            Ok(result) => result,
                            Err(e) => Err(AppError::Upstream(format!("locs task panicked: {e}"))),
                        }
                    }
                    .boxed()
                    .shared();

                    entry.insert(shared.clone());
                    (shared, true)
                }
            }
        };

        // Only the creator cleans up, so a stale clone can't evict a newer job reusing this key.
        if is_new {
            let jobs = Arc::clone(&self.jobs);
            let shared = shared.clone();
            tokio::spawn(async move {
                let _ = shared.await;
                jobs.lock().unwrap().remove(&key);
            });
        }

        shared.await
    }
}
