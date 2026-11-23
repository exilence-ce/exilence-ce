//! The scheduler (client-background-work.md, scheduler): one task owns every
//! deadline as a wall-clock time (Unix ms, UTC) and checks them once a
//! second. Time comes from the injectable [`Clock`], so after a wake or any
//! other clock jump every overdue deadline fires once, at once. The
//! backoff's random factor comes from the injectable [`Jitter`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};

use crate::clock::Clock;
use crate::ggg::Ggg;
use crate::{auto_snapshot, group_connection, group_upload_queue, prices, session_start, updater};

/// How often the task checks the deadlines.
pub const TICK: Duration = Duration::from_secs(1);

/// The longest backoff, so retries come at most 60 s apart (F18).
pub const BACKOFF_CAP_MS: i64 = 60_000;
/// The first backoff before the random factor.
pub const BACKOFF_BASE_MS: i64 = 5_000;

/// A deadline the scheduler owns; at most one of each is pending. When
/// several are due they fire in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Deadline {
    /// The next update check, 5 minutes after the last (F16).
    UpdateCheck,
    /// The GGG part of the session start again, 45 s after it failed (F1).
    GggSessionRetry,
    /// The backend part of the session start again, after a [backoff]
    /// (D26).
    ///
    /// [backoff]: backoff_ms
    BackendSessionRetry,
    /// The Group stream has been silent for 75 s: dead (D22).
    GroupStreamSilence,
    /// The next Group reconnect attempt, after a [backoff] (D22).
    ///
    /// [backoff]: backoff_ms
    GroupReconnect,
    /// The next attempt of the Group upload queue (D27).
    GroupUploadRetry,
    /// The end of the GGG rate-limit cooldown: the cleared `rate_limit`
    /// slice is pushed (F7).
    RateLimitCooldownEnd,
    /// The price check, every minute while signed in (F5).
    PriceCheck,
    /// The auto-snapshot due time (F4).
    AutoSnapshot,
}

/// The backoff's random factor `r`, drawn uniformly from 0.5–1 for every
/// wait (client-background-work.md, backoff); every other random wait is
/// drawn from it too.
pub trait Jitter: Send + Sync {
    fn factor(&self) -> f64;
}

/// `r` from the OS's randomness.
pub struct OsJitter;

impl Jitter for OsJitter {
    #[expect(
        clippy::expect_used,
        reason = "without OS randomness there is no jitter to fall back to"
    )]
    fn factor(&self) -> f64 {
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes).expect("the OS has randomness");
        // 53 random bits: uniform in [0, 1).
        let unit = (u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64;
        0.5 + unit / 2.0
    }
}

/// The wait before attempt n+1 of a backend retry loop after `failed` (n)
/// failed attempts: `min(60 s, 5 s × 2ⁿ) × r`, in ms.
pub fn backoff_ms(failed: u32, r: f64) -> i64 {
    let base = BACKOFF_BASE_MS
        .saturating_mul(1 << failed.min(62))
        .min(BACKOFF_CAP_MS);
    (base as f64 * r) as i64
}

/// The deadlines, managed by the app.
pub struct Scheduler {
    clock: Arc<dyn Clock>,
    jitter: Arc<dyn Jitter>,
    deadlines: Mutex<BTreeMap<Deadline, i64>>,
}

impl Scheduler {
    pub fn new(clock: Arc<dyn Clock>, jitter: Arc<dyn Jitter>) -> Self {
        Self {
            clock,
            jitter,
            deadlines: Mutex::default(),
        }
    }

    fn deadlines(&self) -> MutexGuard<'_, BTreeMap<Deadline, i64>> {
        self.deadlines.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Now, Unix ms UTC.
    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    /// Sets `deadline` to `at`, replacing a pending one.
    pub fn set(&self, deadline: Deadline, at: i64) {
        self.deadlines().insert(deadline, at);
    }

    /// Sets `deadline` to `ms` from now.
    pub fn set_in(&self, deadline: Deadline, ms: i64) {
        self.set(deadline, self.now() + ms);
    }

    pub fn clear(&self, deadline: Deadline) {
        self.deadlines().remove(&deadline);
    }

    /// When `deadline` fires, if pending.
    pub fn due_at(&self, deadline: Deadline) -> Option<i64> {
        self.deadlines().get(&deadline).copied()
    }

    /// The [backoff](backoff_ms) after `failed` failed attempts, with a
    /// fresh random factor.
    pub fn backoff(&self, failed: u32) -> i64 {
        backoff_ms(failed, self.jitter.factor())
    }

    /// A random wait uniformly between `low` and `high` ms, from the
    /// backoff's random source.
    pub fn random_ms(&self, low: i64, high: i64) -> i64 {
        let unit = (self.jitter.factor() - 0.5) * 2.0;
        low + ((high - low) as f64 * unit) as i64
    }

    /// Removes and returns every deadline due at `now`, in [`Deadline`]
    /// order.
    fn take_due(&self, now: i64) -> Vec<Deadline> {
        let mut deadlines = self.deadlines();
        let due: Vec<Deadline> = deadlines
            .iter()
            .filter(|&(_, &at)| at <= now)
            .map(|(&deadline, _)| deadline)
            .collect();
        for deadline in &due {
            deadlines.remove(deadline);
        }
        due
    }
}

/// Starts the scheduler task: [`tick`] once a second.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut ticks = tokio::time::interval(TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            tick(&app);
        }
    });
}

/// Fires every due deadline once, each starting its work in the background;
/// returns those fired.
pub fn tick<R: Runtime>(app: &AppHandle<R>) -> Vec<Deadline> {
    let scheduler = app.state::<Scheduler>();
    let due = scheduler.take_due(scheduler.now());
    for &deadline in &due {
        log::debug!("deadline {deadline:?} fired");
        match deadline {
            Deadline::UpdateCheck => updater::check_now(app),
            Deadline::GggSessionRetry => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { session_start::retry_ggg(&app).await });
            }
            Deadline::BackendSessionRetry => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { session_start::backend_part(&app).await });
            }
            Deadline::GroupStreamSilence => group_connection::silent(app),
            Deadline::GroupReconnect => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { group_connection::reconnect(&app).await });
            }
            Deadline::GroupUploadRetry => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { group_upload_queue::drain(&app).await });
            }
            Deadline::RateLimitCooldownEnd => app.state::<Ggg>().push(app),
            Deadline::PriceCheck => prices::check(app),
            Deadline::AutoSnapshot => auto_snapshot::fire(app),
        }
    }
    due
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use super::*;

    struct Fixed(i64);

    impl Clock for Fixed {
        fn now(&self) -> i64 {
            self.0
        }

        fn sleep_until(&self, _: i64) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            Box::pin(async {})
        }
    }

    #[test]
    fn d26_backoff_doubles_from_5_s_up_to_60_s_times_a_factor_of_one_half_to_one() {
        let bounds: Vec<(i64, i64)> = (0..6)
            .map(|n| (backoff_ms(n, 0.5), backoff_ms(n, 1.0)))
            .collect();
        assert_eq!(
            bounds,
            [
                (2_500, 5_000),
                (5_000, 10_000),
                (10_000, 20_000),
                (20_000, 40_000),
                (30_000, 60_000),
                (30_000, 60_000),
            ]
        );
        // Never past the cap, however many attempts failed.
        assert_eq!(backoff_ms(63, 1.0), 60_000);
        assert_eq!(backoff_ms(u32::MAX, 1.0), 60_000);
        for _ in 0..1000 {
            let r = OsJitter.factor();
            assert!((0.5..=1.0).contains(&r), "{r}");
        }
    }

    #[test]
    fn overdue_deadlines_are_taken_once_in_order() {
        let scheduler = Scheduler::new(Arc::new(Fixed(0)), Arc::new(OsJitter));
        scheduler.set(Deadline::RateLimitCooldownEnd, 60_000);
        scheduler.set(Deadline::GggSessionRetry, 45_000);
        scheduler.set(Deadline::UpdateCheck, 300_000);
        assert_eq!(scheduler.take_due(44_999), []);
        // Hours later, e.g. after a suspend: each fires once.
        assert_eq!(
            scheduler.take_due(10_000_000),
            [
                Deadline::UpdateCheck,
                Deadline::GggSessionRetry,
                Deadline::RateLimitCooldownEnd
            ]
        );
        assert_eq!(scheduler.take_due(10_000_000), []);
        assert_eq!(scheduler.due_at(Deadline::UpdateCheck), None);
    }

    struct Factor(f64);

    impl Jitter for Factor {
        fn factor(&self) -> f64 {
            self.0
        }
    }

    #[test]
    fn d27_the_first_upload_retry_waits_a_random_1_to_10_s() {
        let wait =
            |r| Scheduler::new(Arc::new(Fixed(0)), Arc::new(Factor(r))).random_ms(1_000, 10_000);
        assert_eq!([wait(0.5), wait(0.75), wait(1.0)], [1_000, 5_500, 10_000]);
    }
}
