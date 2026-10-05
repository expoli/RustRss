//! Bounded, drain-on-cancel extraction. Only item extraction uses this limiter;
//! synthesis, prompts and cache identity remain unchanged.
use super::AiError;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::Instant;

#[derive(Default)]
struct Budget {
    tasks: HashMap<u64, usize>,
    next_task: u64,
    in_flight: usize,
    level: usize,
    rate_cap: usize,
    successes: usize,
    throttles: u32,
    cooldown: Option<Instant>,
}

#[derive(Default)]
struct Limiter {
    budget: Mutex<Budget>,
    changed: Notify,
}

fn endpoint_limiter(endpoint: &str) -> Arc<Limiter> {
    static LIMITERS: OnceLock<Mutex<HashMap<String, Arc<Limiter>>>> = OnceLock::new();
    LIMITERS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(endpoint.trim().trim_end_matches('/').to_owned())
        .or_default()
        .clone()
}

fn retryable_throttle(error: &AiError) -> bool {
    let AiError::Provider {
        status: 429,
        message,
        ..
    } = error
    else {
        return false;
    };
    // Some OpenAI-compatible services report billing exhaustion as HTTP 429.
    // Explicit permanent billing codes are not request-rate throttles.
    ![
        "insufficient_quota",
        "insufficient_balance",
        "credit_balance_too_low",
    ]
    .iter()
    .any(|code| message.contains(code))
}

fn backoff(exponent: u32) -> Duration {
    // No RNG dependency: a small positive jitter prevents lockstep retries.
    let jitter = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
        % 251;
    Duration::from_millis((1u64 << exponent.min(5)) * 1000 + jitter)
}

struct Registration {
    limiter: Arc<Limiter>,
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.limiter.budget.lock().unwrap().tasks.remove(&self.id);
        self.limiter.changed.notify_waiters();
    }
}

struct Permit(Arc<Limiter>);

impl Permit {
    fn finish(self, result: &Result<String, AiError>) {
        let mut b = self.0.budget.lock().unwrap();
        match result {
            Err(AiError::Provider {
                status: 429,
                retry_after,
                ..
            }) if result.as_ref().err().is_some_and(retryable_throttle) => {
                let delay = retry_after.unwrap_or_else(|| backoff(b.throttles));
                b.throttles = b.throttles.saturating_add(1);
                // Even an unrepresentably distant header must not panic.
                let until = Instant::now()
                    .checked_add(delay)
                    .unwrap_or_else(|| Instant::now() + Duration::from_secs(365 * 24 * 3600));
                b.cooldown = Some(b.cooldown.map_or(until, |old| old.max(until)));
                b.rate_cap = (b.rate_cap / 2).max(1);
                b.level = 1;
                b.successes = 0;
            }
            Ok(_) => {
                b.throttles = 0;
                b.successes += 1;
                if b.successes == 3 {
                    b.level = (b.level + 1).min(b.rate_cap);
                    b.successes = 0;
                }
            }
            _ => b.successes = 0,
        }
        drop(b);
        // Drop releases the shared slot even if a request panics/unwinds.
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.budget.lock().unwrap().in_flight -= 1;
        self.0.changed.notify_waiters();
    }
}

impl Limiter {
    async fn acquire(self: &Arc<Self>, cancel: &AtomicBool) -> Option<Permit> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            {
                let mut b = self.budget.lock().unwrap();
                let ceiling = b.tasks.values().copied().min().unwrap_or(1);
                let cooling = b.cooldown.is_some_and(|until| until > Instant::now());
                if !cooling && b.in_flight < b.level.min(b.rate_cap).min(ceiling) {
                    // No await between cancellation check, admission and HTTP start.
                    if cancel.load(Ordering::Relaxed) {
                        return None;
                    }
                    b.in_flight += 1;
                    return Some(Permit(self.clone()));
                }
            }
            tokio::select! {
                _ = changed => {},
                // Atomic cancellation has no notification; polling is bounded.
                _ = tokio::time::sleep(Duration::from_millis(20)) => {},
            }
        }
    }
}

pub struct DigestExtractionScheduler {
    endpoint: String,
    concurrency: usize,
    cancel: Arc<AtomicBool>,
}

impl DigestExtractionScheduler {
    /// Use the service base URL, not the model-dependent cache identity. Active
    /// tasks with different settings share the smallest requested endpoint cap.
    pub fn new(endpoint: &str, concurrency: usize, cancel: Arc<AtomicBool>) -> Self {
        Self {
            endpoint: endpoint.into(),
            concurrency: concurrency.clamp(1, 8),
            cancel,
        }
    }

    /// Results retain input order; the single coordinator calls `completed` in
    /// completion order. None means never started (cancelled). The callback can
    /// persist paid results before cancellation is reported by the caller.
    pub async fn run<T, F, Fut, C>(
        &self,
        items: Vec<T>,
        extract: F,
        mut completed: C,
    ) -> Vec<Option<Result<String, AiError>>>
    where
        T: Send + 'static,
        F: Fn(T) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = Result<String, AiError>> + Send + 'static,
        C: FnMut(usize, &Result<String, AiError>),
        T: Clone,
    {
        let limiter = endpoint_limiter(&self.endpoint);
        let id = {
            let mut b = limiter.budget.lock().unwrap();
            if b.tasks.is_empty() {
                b.level = 1;
                b.successes = 0;
                if b.cooldown.is_none_or(|until| until <= Instant::now()) {
                    b.rate_cap = self.concurrency;
                    b.throttles = 0;
                }
            }
            b.next_task += 1;
            let id = b.next_task;
            b.tasks.insert(id, self.concurrency);
            id
        };
        let _registration = Registration {
            limiter: limiter.clone(),
            id,
        };
        let mut results: Vec<_> = (0..items.len()).map(|_| None).collect();
        let mut pending = items.into_iter().enumerate();
        let mut running = JoinSet::new();
        let mut level = 1;
        let mut successes = 0;
        loop {
            while running.len() < level && !self.cancel.load(Ordering::Relaxed) {
                let Some((index, item)) = pending.next() else {
                    break;
                };
                let limiter = limiter.clone();
                let cancel = self.cancel.clone();
                let extract = extract.clone();
                running.spawn(async move {
                    let mut previous = None;
                    for attempt in 0..2 {
                        let Some(permit) = limiter.acquire(&cancel).await else {
                            return (index, previous);
                        };
                        let result = extract(item.clone()).await;
                        permit.finish(&result);
                        // Transport is the existing coarse network category. It
                        // cannot distinguish transient failures from TLS errors.
                        let retry = result.as_ref().err().is_some_and(|error| {
                            retryable_throttle(error) || matches!(error, AiError::Transport(_))
                        });
                        let transport = matches!(&result, Err(AiError::Transport(_)));
                        previous = Some(result);
                        if !retry || attempt == 1 || cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        if transport {
                            tokio::time::sleep(backoff(0)).await;
                        }
                    }
                    (index, previous)
                });
            }
            let Some(result) = running.join_next().await else {
                break;
            };
            // Extraction must not panic. Propagate a panic rather than silently
            // treating a missing paid result as successful completion.
            let (index, output) = result.expect("digest extraction worker panicked");
            if let Some(output) = output {
                if output.is_ok() {
                    successes += 1;
                    if successes == 3 {
                        level = (level + 1).min(self.concurrency);
                        successes = 0;
                    }
                } else {
                    successes = 0;
                    if matches!(&output, Err(AiError::Provider { status: 429, .. })) {
                        level = 1;
                    }
                }
                completed(index, &output);
                results[index] = Some(output);
            }
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn throttling_halves_cap_resets_ramp_and_uses_exponential_jitter() {
        let limiter = Arc::new(Limiter::default());
        {
            let mut b = limiter.budget.lock().unwrap();
            b.tasks.insert(0, 4);
            b.rate_cap = 4;
            b.level = 4;
            b.successes = 2;
        }
        for (expected_cap, seconds) in [(2, 1), (1, 2)] {
            limiter.budget.lock().unwrap().in_flight = 1;
            Permit(limiter.clone()).finish(&Err(AiError::Provider {
                status: 429,
                message: "rate".into(),
                retry_after: None,
            }));
            let b = limiter.budget.lock().unwrap();
            let delay = b.cooldown.unwrap().duration_since(Instant::now());
            assert!(delay >= Duration::from_secs(seconds));
            assert!(delay <= Duration::from_millis(seconds * 1000 + 250));
            assert_eq!(b.rate_cap, expected_cap);
            assert_eq!(b.level, 1);
            assert_eq!(b.successes, 0);
            assert_eq!(b.in_flight, 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn shared_budget_uses_smallest_active_task_cap_and_waiting_cancel_stops() {
        let limiter = Arc::new(Limiter::default());
        {
            let mut b = limiter.budget.lock().unwrap();
            b.tasks.insert(0, 4);
            b.tasks.insert(1, 1);
            b.rate_cap = 4;
            b.level = 4;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let first = limiter.acquire(&cancel).await.unwrap();
        let waiting = tokio::spawn({
            let limiter = limiter.clone();
            let cancel = cancel.clone();
            async move { limiter.acquire(&cancel).await }
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "two tasks share one active slot");
        cancel.store(true, Ordering::Relaxed);
        assert!(waiting.await.unwrap().is_none());
        drop(first);
        assert_eq!(limiter.budget.lock().unwrap().in_flight, 0);
    }
}
