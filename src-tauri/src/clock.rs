//! Wall-clock time, read through [`Clock`] so tests advance time instead of
//! sleeping (parity verification, fakes). Deadlines are wall-clock times,
//! never tokio's monotonic clock, which stops while the machine sleeps
//! (client-background-work.md, scheduler).

use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A source of wall-clock time.
pub trait Clock: Send + Sync {
    /// Now, Unix ms UTC.
    fn now(&self) -> i64;

    /// Resolves once [`Clock::now`] has reached `at`.
    fn sleep_until(&self, at: i64) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// The system's clock.
pub struct SystemClock;

/// How long [`SystemClock`] sleeps at most before it reads the time again, so
/// a deadline passed while the machine slept fires right after the wake.
const RECHECK: Duration = Duration::from_secs(1);

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    }

    fn sleep_until(&self, at: i64) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            loop {
                let left = at - self.now();
                if left <= 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(left as u64).min(RECHECK)).await;
            }
        })
    }
}
