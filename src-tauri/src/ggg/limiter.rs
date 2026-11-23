//! The limiter of the GGG queue (F7, client-background-work.md, GGG requests
//! and rate limits): GGG's rules learnt from each response's `X-Rate-Limit-*`
//! headers, a log of hits per window, pacing with one hit spare, and the 429
//! cooldown. Pure: every time is passed in, Unix ms.

use std::collections::{BTreeMap, BTreeSet};

use ipc::slices::{Allowance, RateLimit};

/// Every `x-rate-limit-*` header of a response, name (lowercase) → value, as
/// received; `rate_limit_policies.headers`.
pub type Headers = BTreeMap<String, String>;

/// The prefix of the headers [`Headers`] keeps.
pub const HEADER_PREFIX: &str = "x-rate-limit-";

/// A window counts hits this much longer than its period, so clock skew and
/// latency never let a hit land in GGG's window early.
const MARGIN_MS: i64 = 2_000;

/// `max:period:restriction`: at most `max` hits per `period` s, else
/// restricted for `restriction` s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Limit {
    max: u32,
    period: u32,
    restriction: u32,
}

/// `hits:period:active`: GGG counts `hits` within `period` s; `active` s of
/// restriction are left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct State {
    hits: u32,
    period: u32,
    active: u32,
}

/// One rule of a policy as the headers state it.
struct Rule {
    name: String,
    limits: Vec<Limit>,
    states: Vec<State>,
}

/// A policy as one response's headers state it.
struct Learnt {
    policy: String,
    rules: Vec<Rule>,
}

impl Learnt {
    /// Reads `X-Rate-Limit-Policy`, `-Rules`, `-<rule>` and `-<rule>-State`;
    /// `None` without a policy. Malformed entries are skipped.
    fn parse(headers: &Headers) -> Option<Self> {
        let policy = headers.get("x-rate-limit-policy")?.trim();
        if policy.is_empty() {
            return None;
        }
        let names = headers.get("x-rate-limit-rules").map_or("", String::as_str);
        let rules = names
            .split(',')
            .map(|n| n.trim().to_ascii_lowercase())
            .filter(|n| !n.is_empty())
            .map(|name| {
                let header = |suffix: &str| {
                    headers
                        .get(&format!("{HEADER_PREFIX}{name}{suffix}"))
                        .map_or("", String::as_str)
                };
                let limits = triples(header(""))
                    .map(|[max, period, restriction]| Limit {
                        max,
                        period,
                        restriction,
                    })
                    .filter(|l| l.period > 0)
                    .collect();
                let states = triples(header("-state"))
                    .map(|[hits, period, active]| State {
                        hits,
                        period,
                        active,
                    })
                    .collect();
                Rule {
                    name,
                    limits,
                    states,
                }
            })
            .collect();
        Some(Self {
            policy: policy.to_owned(),
            rules,
        })
    }

    /// The largest active restriction in the `-State` headers, if any.
    fn active_restriction(&self) -> Option<u32> {
        self.rules
            .iter()
            .flat_map(|r| &r.states)
            .map(|s| s.active)
            .filter(|&a| a > 0)
            .max()
    }

    /// The hits GGG reports for `rule` within `period`.
    fn reported_hits(&self, rule: &str, period: u32) -> Option<u32> {
        self.rules
            .iter()
            .find(|r| r.name == rule)?
            .states
            .iter()
            .find(|s| s.period == period)
            .map(|s| s.hits)
    }
}

/// The `a:b:c` triples of a comma list.
fn triples(value: &str) -> impl Iterator<Item = [u32; 3]> + '_ {
    value.split(',').filter_map(|triple| {
        let mut numbers = triple.split(':').map(|n| n.trim().parse().ok());
        let parsed = [numbers.next()??, numbers.next()??, numbers.next()??];
        numbers.next().is_none().then_some(parsed)
    })
}

/// One (policy, rule, period) with the hits counted against it: the
/// client's own send times and hits spent elsewhere.
#[derive(Debug)]
struct Window {
    rule: String,
    limit: Limit,
    /// Ascending.
    hits: Vec<i64>,
}

impl Window {
    /// How far back hits count when pacing.
    fn span(&self) -> i64 {
        i64::from(self.limit.period) * 1000 + MARGIN_MS
    }

    /// Hits a window may hold before the next send: `max − 1` keeps one
    /// hit spare. A rule of one hit allows that one.
    fn allowed(&self) -> usize {
        self.limit.max.saturating_sub(1).max(1) as usize
    }

    fn add(&mut self, at: i64, count: usize) {
        let i = self.hits.partition_point(|&h| h <= at);
        self.hits.splice(i..i, std::iter::repeat_n(at, count));
    }

    /// The earliest time from `now` at which the window holds fewer than
    /// [`Window::allowed`] hits within the last [`Window::span`].
    fn ready_at(&self, now: i64) -> i64 {
        let span = self.span();
        let recent = &self.hits[self.hits.partition_point(|&h| h <= now - span)..];
        match recent.len().checked_sub(self.allowed()) {
            None => now,
            // Once that hit drops out, `allowed − 1` remain.
            Some(oldest_to_drop) => (recent[oldest_to_drop] + span).max(now),
        }
    }

    fn prune(&mut self, now: i64) {
        let cutoff = now - self.span();
        self.hits.retain(|&h| h > cutoff);
    }
}

#[derive(Default)]
struct Policy {
    windows: Vec<Window>,
    /// The endpoint classes seen under this policy.
    endpoints: BTreeSet<String>,
    headers: Headers,
    observed_at: i64,
    restricted_until: Option<i64>,
}

impl Policy {
    /// Replaces the windows with `learnt`'s, keeping the hits of each (rule,
    /// period) that stays.
    fn set_rules(&mut self, name: &str, learnt: &Learnt) {
        let mut old = std::mem::take(&mut self.windows);
        for rule in &learnt.rules {
            for &limit in &rule.limits {
                let hits = old
                    .iter_mut()
                    .find(|w| w.rule == rule.name && w.limit.period == limit.period)
                    .map(|w| std::mem::take(&mut w.hits))
                    .unwrap_or_default();
                self.windows.push(Window {
                    rule: rule.name.clone(),
                    limit,
                    hits,
                });
            }
        }
        let rules = |windows: &[Window]| -> Vec<(String, Limit)> {
            windows.iter().map(|w| (w.rule.clone(), w.limit)).collect()
        };
        if rules(&old) != rules(&self.windows) {
            log::debug!(
                "rate limit policy {name}: rules {}",
                describe(&self.windows)
            );
        }
    }
}

fn describe(windows: &[Window]) -> String {
    let rules: Vec<String> = windows
        .iter()
        .map(|w| {
            let l = w.limit;
            format!("{} {}:{}:{}", w.rule, l.max, l.period, l.restriction)
        })
        .collect();
    rules.join(", ")
}

/// A policy as `rate_limit_policies` stores it, and as the Diagnostics
/// bundle shows it (F20).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Row {
    pub policy: String,
    pub headers: Headers,
    pub endpoints: BTreeSet<String>,
    pub observed_at: i64,
    pub restricted_until: Option<i64>,
}

/// Every policy the client has seen, with its windows.
#[derive(Default)]
pub struct Limiter {
    policies: BTreeMap<String, Policy>,
}

impl Limiter {
    /// Rebuilds the limiter from the stored policies at `now`: a future
    /// `restricted_until` restores the cooldown; each window's reported hits
    /// count as sent at `observed_at`.
    pub fn restore(rows: impl IntoIterator<Item = Row>, now: i64) -> Self {
        let mut limiter = Self::default();
        for row in rows {
            let mut policy = Policy {
                windows: Vec::new(),
                endpoints: row.endpoints,
                headers: row.headers,
                observed_at: row.observed_at,
                restricted_until: row.restricted_until,
            };
            if let Some(learnt) = Learnt::parse(&policy.headers) {
                policy.set_rules(&row.policy, &learnt);
                for window in &mut policy.windows {
                    let hits = learnt.reported_hits(&window.rule, window.limit.period);
                    window.add(row.observed_at, hits.unwrap_or(0) as usize);
                    window.prune(now);
                }
            }
            if let Some(until) = policy.restricted_until.filter(|&u| u > now) {
                log::debug!(
                    "rate limit policy {} restored restricted for {} s",
                    row.policy,
                    seconds_left(until, now)
                );
            }
            limiter.policies.insert(row.policy, policy);
        }
        limiter
    }

    /// The end of the latest restriction still running at `now`.
    pub fn cooldown_until(&self, now: i64) -> Option<i64> {
        self.policies
            .values()
            .filter_map(|p| p.restricted_until)
            .filter(|&u| u > now)
            .max()
    }

    fn policy_of(&self, class: &str) -> Option<(&String, &Policy)> {
        self.policies
            .iter()
            .find(|(_, p)| p.endpoints.contains(class))
    }

    /// When a request of `class` may be sent, from `now`: once every window
    /// of its policy holds fewer than `max − 1` hits within the last
    /// `period + 2 s`. A class seen for the first time goes at once.
    pub fn ready_at(&self, class: &str, now: i64) -> i64 {
        self.policy_of(class).map_or(now, |(_, p)| {
            p.windows
                .iter()
                .map(|w| w.ready_at(now))
                .fold(now, i64::max)
        })
    }

    /// Logs a request of `class` sent at `at`.
    pub fn sent(&mut self, class: &str, at: i64) {
        if let Some(policy) = self
            .policies
            .values_mut()
            .find(|p| p.endpoints.contains(class))
        {
            for window in &mut policy.windows {
                window.prune(at);
                window.add(at, 1);
            }
        }
    }

    /// Learns from the headers of a response to a `class` request received
    /// at `now`: the class's policy, its rules, and hits spent elsewhere.
    /// Returns the policies whose stored row changed.
    pub fn observe(&mut self, class: &str, headers: &Headers, now: i64) -> Vec<String> {
        let Some(learnt) = Learnt::parse(headers) else {
            return Vec::new();
        };
        let mut changed = vec![learnt.policy.clone()];
        for (name, policy) in &mut self.policies {
            if *name != learnt.policy && policy.endpoints.remove(class) {
                changed.push(name.clone());
            }
        }
        let policy = self.policies.entry(learnt.policy.clone()).or_default();
        if policy.endpoints.insert(class.to_owned()) {
            log::debug!(
                "rate limit: endpoint class {class} uses policy {}",
                learnt.policy
            );
        }
        policy.set_rules(&learnt.policy, &learnt);
        for window in &mut policy.windows {
            window.prune(now);
            let Some(reported) = learnt.reported_hits(&window.rule, window.limit.period) else {
                continue;
            };
            let since = now - i64::from(window.limit.period) * 1000;
            let held = window.hits.iter().filter(|&&h| h > since).count();
            if let Some(elsewhere) = (reported as usize).checked_sub(held).filter(|&n| n > 0) {
                log::debug!(
                    "rate limit policy {} {} {}s: {elsewhere} hits spent elsewhere",
                    learnt.policy,
                    window.rule,
                    window.limit.period
                );
                window.add(now, elsewhere);
            }
        }
        policy.headers = headers.clone();
        policy.observed_at = now;
        if policy.restricted_until.is_some_and(|u| u <= now) {
            policy.restricted_until = None;
        }
        changed
    }

    /// A 429 to a `class` request at `now`: restricts the policy until
    /// `retry_after` s from now; without it, the largest active restriction
    /// in the `-State` headers; without that, the largest restriction of the
    /// policy's rules. Returns the policy and the end of its restriction.
    pub fn restrict(
        &mut self,
        class: &str,
        headers: &Headers,
        retry_after: Option<u64>,
        now: i64,
    ) -> Option<(String, i64)> {
        let learnt = Learnt::parse(headers);
        let name = match &learnt {
            Some(learnt) => learnt.policy.clone(),
            None => self.policy_of(class)?.0.clone(),
        };
        let policy = self.policies.get_mut(&name)?;
        let seconds = retry_after
            .or_else(|| learnt.as_ref()?.active_restriction().map(u64::from))
            .or_else(|| {
                policy
                    .windows
                    .iter()
                    .map(|w| u64::from(w.limit.restriction))
                    .filter(|&r| r > 0)
                    .max()
            })?;
        let until = now.saturating_add(i64::try_from(seconds.saturating_mul(1000)).ok()?);
        let until = policy.restricted_until.map_or(until, |u| u.max(until));
        policy.restricted_until = Some(until);
        log::debug!("rate limit policy {name} restricted for {seconds} s");
        Some((name, until))
    }

    /// The stored form of `policy`.
    pub fn row(&self, policy: &str) -> Option<Row> {
        self.policies.get(policy).map(|p| Row {
            policy: policy.to_owned(),
            headers: p.headers.clone(),
            endpoints: p.endpoints.clone(),
            observed_at: p.observed_at,
            restricted_until: p.restricted_until,
        })
    }

    /// The stored form of every policy.
    pub fn rows(&self) -> Vec<Row> {
        self.policies.keys().filter_map(|p| self.row(p)).collect()
    }

    /// The strictest window (fewest requests per second) of `class`'s
    /// policy, from the latest headers.
    pub fn allowance(&self, class: &str) -> Option<Allowance> {
        let (_, policy) = self.policy_of(class)?;
        policy
            .windows
            .iter()
            .map(|w| Allowance {
                max: w.limit.max,
                period: w.limit.period,
            })
            .min_by(|a, b| {
                (u64::from(a.max) * u64::from(b.period))
                    .cmp(&(u64::from(b.max) * u64::from(a.period)))
            })
    }

    /// The `rate_limit` slice at `now`; the hint names `hint_class`'s policy.
    pub fn slice(&self, now: i64, hint_class: &str) -> RateLimit {
        RateLimit {
            cooldown_until: self.cooldown_until(now),
            allowance: self.allowance(hint_class),
        }
    }
}

/// Whole seconds from `now` to `until`, rounded up.
pub fn seconds_left(until: i64, now: i64) -> u64 {
    u64::try_from(until - now).map_or(0, |ms| ms.div_ceil(1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_800_000_000_000;

    fn headers(pairs: &[(&str, &str)]) -> Headers {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn account(rules: &str, state: &str) -> Headers {
        headers(&[
            ("x-rate-limit-policy", "stash-request-limit"),
            ("x-rate-limit-rules", "Account"),
            ("x-rate-limit-account", rules),
            ("x-rate-limit-account-state", state),
        ])
    }

    #[test]
    fn f7_parses_rules_and_skips_malformed_entries() {
        let learnt =
            Learnt::parse(&account("15:10:60, x:1:2,30:300:1800", "1:10:0,1:300:0")).unwrap();
        assert_eq!(learnt.policy, "stash-request-limit");
        assert_eq!(learnt.rules[0].name, "account");
        assert_eq!(
            learnt.rules[0].limits,
            [
                Limit {
                    max: 15,
                    period: 10,
                    restriction: 60
                },
                Limit {
                    max: 30,
                    period: 300,
                    restriction: 1800
                }
            ]
        );
        assert_eq!(learnt.reported_hits("account", 300), Some(1));
        assert!(Learnt::parse(&headers(&[("x-rate-limit-rules", "account")])).is_none());
    }

    #[test]
    fn f7_paces_with_one_hit_spare_and_counts_hits_spent_elsewhere() {
        let mut limiter = Limiter::default();
        // A class seen for the first time goes at once, unlogged: its hit
        // counts from the response on.
        assert_eq!(limiter.ready_at("stash", T0), T0);
        limiter.sent("stash", T0);
        limiter.observe("stash", &account("5:10:60", "1:10:0"), T0 + 100);
        limiter.sent("stash", T0 + 200);
        // GGG counts 4 hits: 2 were spent elsewhere.
        limiter.observe("stash", &account("5:10:60", "4:10:0"), T0 + 300);
        // 4 = max − 1 hits within 12 s: wait until the oldest drops out.
        assert_eq!(limiter.ready_at("stash", T0 + 400), T0 + 12_100);
        // Other classes are not paced by this policy.
        assert_eq!(limiter.ready_at("characters", T0 + 300), T0 + 300);
    }

    #[test]
    fn f7_cooldown_sources_in_order() {
        let mut limiter = Limiter::default();
        let rules = "5:10:60,15:60:300";
        limiter.observe("stash", &account(rules, "6:10:45,6:60:0"), T0);
        assert_eq!(
            limiter.restrict("stash", &account(rules, "6:10:45,6:60:0"), Some(30), T0),
            Some(("stash-request-limit".to_owned(), T0 + 30_000))
        );
        let mut limiter = Limiter::default();
        limiter.observe("stash", &account(rules, "6:10:45,6:60:0"), T0);
        assert_eq!(
            limiter.restrict("stash", &account(rules, "6:10:45,6:60:0"), None, T0),
            Some(("stash-request-limit".to_owned(), T0 + 45_000))
        );
        let mut limiter = Limiter::default();
        limiter.observe("stash", &account(rules, "6:10:0,6:60:0"), T0);
        assert_eq!(
            limiter.restrict("stash", &account(rules, "6:10:0,6:60:0"), None, T0),
            Some(("stash-request-limit".to_owned(), T0 + 300_000))
        );
        assert_eq!(limiter.cooldown_until(T0), Some(T0 + 300_000));
        assert_eq!(limiter.cooldown_until(T0 + 300_000), None);
    }

    #[test]
    fn f7_restore_counts_reported_hits_at_observed_at() {
        let row = Row {
            policy: "stash-request-limit".to_owned(),
            headers: account("5:10:60", "4:10:0"),
            endpoints: BTreeSet::from(["stash".to_owned()]),
            observed_at: T0,
            restricted_until: Some(T0 + 60_000),
        };
        let limiter = Limiter::restore([row.clone()], T0 + 1_000);
        assert_eq!(limiter.cooldown_until(T0 + 1_000), Some(T0 + 60_000));
        assert_eq!(limiter.ready_at("stash", T0 + 1_000), T0 + 12_000);
        assert_eq!(limiter.row("stash-request-limit"), Some(row));
        // Hits drop out once observed_at + period + 2 s has passed.
        let limiter = Limiter::restore(limiter.row("stash-request-limit"), T0 + 12_000);
        assert_eq!(limiter.ready_at("stash", T0 + 12_000), T0 + 12_000);
    }

    #[test]
    fn f7_allowance_names_the_strictest_window() {
        let mut limiter = Limiter::default();
        assert_eq!(limiter.allowance("stash"), None);
        limiter.observe(
            "stash",
            &account("15:10:60,30:300:1800", "1:10:0,1:300:0"),
            T0,
        );
        assert_eq!(
            limiter.allowance("stash"),
            Some(Allowance {
                max: 30,
                period: 300
            })
        );
    }

    #[test]
    fn seconds_left_rounds_up() {
        assert_eq!(seconds_left(T0 + 1_001, T0), 2);
        assert_eq!(seconds_left(T0 + 1_000, T0), 1);
        assert_eq!(seconds_left(T0, T0 + 5), 0);
    }
}
