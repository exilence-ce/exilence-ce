//! GGG's API (F7, client-background-work.md, GGG requests and rate limits):
//! every request goes through one queue, one at a time, paced by the
//! [`limiter`] and carrying GGG's User-Agent and the Login's access token.
//! Learnt rules and cooldowns live in `rate_limit_policies`, so a restart
//! keeps them.

pub mod limiter;

use std::sync::{Arc, Mutex, MutexGuard};

use ipc::IpcError;
use ipc::slices::RateLimit;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use self::limiter::{HEADER_PREFIX, Headers, Limiter, Row, seconds_left};
use crate::clock::Clock;
use crate::scheduler::{Deadline, Scheduler};
use crate::state::Store;
use crate::{auth, db, internal};

/// GGG's API host.
pub const API_URL: &str = "https://api.pathofexile.com";

/// The endpoint classes the limiter learns a policy for
/// (`rate_limit_policies.endpoints`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Profile,
    Characters,
    Character,
    StashList,
    Stash,
    ChildStash,
}

impl Endpoint {
    /// The class's stored name.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Characters => "characters",
            Self::Character => "character",
            Self::StashList => "stash-list",
            Self::Stash => "stash",
            Self::ChildStash => "stash-child",
        }
    }
}

/// One GGG request.
#[derive(Clone, Copy, Debug)]
pub enum Request<'a> {
    /// `/profile`.
    Profile,
    /// `/character`: the account's characters.
    Characters,
    /// `/character/<name>`.
    Character { name: &'a str },
    /// `/stash/<league>`: the stash tab list.
    StashList { league: &'a str },
    /// `/stash/<league>/<id>`: a stash tab.
    Stash { league: &'a str, id: &'a str },
    /// `/stash/<league>/<parent>/<id>`: a child tab.
    ChildStash {
        league: &'a str,
        parent: &'a str,
        id: &'a str,
    },
}

impl Request<'_> {
    pub fn endpoint(&self) -> Endpoint {
        match self {
            Self::Profile => Endpoint::Profile,
            Self::Characters => Endpoint::Characters,
            Self::Character { .. } => Endpoint::Character,
            Self::StashList { .. } => Endpoint::StashList,
            Self::Stash { .. } => Endpoint::Stash,
            Self::ChildStash { .. } => Endpoint::ChildStash,
        }
    }

    /// The path segments, unencoded.
    fn segments(&self) -> Vec<&str> {
        match *self {
            Self::Profile => vec!["profile"],
            Self::Characters => vec!["character"],
            Self::Character { name } => vec!["character", name],
            Self::StashList { league } => vec!["stash", league],
            Self::Stash { league, id } => vec!["stash", league, id],
            Self::ChildStash { league, parent, id } => vec!["stash", league, parent, id],
        }
    }
}

/// A character of `/character`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Character {
    pub name: String,
    /// Absent for a character outside any league.
    #[serde(default)]
    pub league: Option<String>,
}

#[derive(Deserialize)]
struct CharacterList {
    characters: Vec<Character>,
}

/// A tab of `/stash/<league>`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct StashTab {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub metadata: StashTabMetadata,
    /// A folder's tabs.
    #[serde(default)]
    pub children: Vec<StashTab>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct StashTabMetadata {
    /// RGB hex without `#`.
    #[serde(default)]
    pub colour: Option<String>,
}

#[derive(Deserialize)]
struct StashList {
    #[serde(default)]
    stashes: Vec<StashTab>,
}

/// A stash tab of `/stash/<league>/<id>` with its items. A tab with child
/// tabs (e.g. a map or unique tab) lists them without items; each is fetched
/// on its own (F3).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StashContents {
    pub id: String,
    #[serde(default)]
    pub items: Vec<domain::ggg::Item>,
    #[serde(default)]
    pub children: Vec<StashContents>,
}

#[derive(Deserialize)]
struct StashResponse {
    stash: StashContents,
}

/// A character of `/character/<name>` with its items.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CharacterItems {
    #[serde(default)]
    pub inventory: Vec<domain::ggg::Item>,
    #[serde(default)]
    pub equipment: Vec<domain::ggg::Item>,
}

#[derive(Deserialize)]
struct CharacterResponse {
    character: CharacterItems,
}

/// Why a GGG request failed. Never carries the access token.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A cooldown is running, or this request started one (F7).
    #[error("rate limited for {seconds_left} s")]
    RateLimited { seconds_left: u64 },
    #[error("no Login is stored")]
    SignedOut,
    /// A second 401 after a refresh, or the refresh was refused: the user
    /// was sent to login (D6).
    #[error("the GGG token was refused after a refresh")]
    SessionExpired,
    #[error("GGG answered {0}")]
    Status(StatusCode),
    #[error("GGG unreachable: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("cannot read or store the rate limits: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<Error> for IpcError {
    fn from(error: Error) -> Self {
        match error {
            Error::RateLimited { seconds_left } => Self::RateLimited { seconds_left },
            Error::SessionExpired => Self::SessionExpired,
            error => internal(error),
        }
    }
}

/// The GGG client with its queue, managed by the app.
pub struct Ggg {
    http: reqwest::Client,
    base_url: Url,
    clock: Arc<dyn Clock>,
    pool: SqlitePool,
    limiter: Mutex<Limiter>,
    /// Held for a whole request, pacing included: one request at a time.
    queue: tokio::sync::Mutex<()>,
}

impl Ggg {
    /// The client of the API at `base_url` ([`API_URL`] in the app), with the
    /// limiter rebuilt from `rate_limit_policies`.
    pub async fn open(
        pool: SqlitePool,
        base_url: &str,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let rows = load(&pool).await?;
        let limiter = Limiter::restore(rows, clock.now());
        Ok(Self {
            http: ninja::http::client(auth::user_agent())?,
            base_url: Url::parse(base_url)?,
            clock,
            pool,
            limiter: Mutex::new(limiter),
            queue: tokio::sync::Mutex::new(()),
        })
    }

    fn limiter(&self) -> MutexGuard<'_, Limiter> {
        self.limiter.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The `rate_limit` slice now.
    pub fn rate_limit(&self) -> RateLimit {
        self.limiter()
            .slice(self.clock.now(), Endpoint::Stash.class())
    }

    /// Every learnt policy with its rules and cooldown, for the Diagnostics
    /// bundle (F20).
    pub fn policies(&self) -> Vec<Row> {
        self.limiter().rows()
    }

    /// Fails at once while a cooldown runs.
    fn check_cooldown(&self, now: i64) -> Result<(), Error> {
        match self.limiter().cooldown_until(now) {
            Some(until) => Err(Error::RateLimited {
                seconds_left: seconds_left(until, now),
            }),
            None => Ok(()),
        }
    }

    /// Sends `request` through the queue and reads its JSON answer. Waits
    /// for its turn and its policy's windows; fails at once with
    /// [`Error::RateLimited`] while a cooldown runs. A 401 has the backend
    /// refresh the token once and sends the request once more; a second 401
    /// sends the user to login (D6). Nothing else is retried.
    pub async fn get<T: DeserializeOwned, R: Runtime>(
        &self,
        app: &AppHandle<R>,
        request: Request<'_>,
    ) -> Result<T, Error> {
        let response = match self.send(app, request).await {
            Err(Error::Status(StatusCode::UNAUTHORIZED)) => {
                match auth::refresh_ggg_token(app).await {
                    Ok(()) => {}
                    Err(IpcError::SessionExpired) => return Err(Error::SessionExpired),
                    Err(_) => return Err(Error::Status(StatusCode::UNAUTHORIZED)),
                }
                match self.send(app, request).await {
                    Err(Error::Status(StatusCode::UNAUTHORIZED)) => {
                        auth::send_to_login(app).await;
                        return Err(Error::SessionExpired);
                    }
                    response => response?,
                }
            }
            response => response?,
        };
        Ok(response.json().await?)
    }

    /// One attempt of [`Ggg::get`]: the response if GGG answered with
    /// success.
    #[expect(
        clippy::expect_used,
        reason = "the base URL is checked as http(s) when the client is built"
    )]
    async fn send<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        request: Request<'_>,
    ) -> Result<reqwest::Response, Error> {
        self.check_cooldown(self.clock.now())?;
        let _turn = self.queue.lock().await;
        let class = request.endpoint().class();
        loop {
            let now = self.clock.now();
            self.check_cooldown(now)?;
            let ready = self.limiter().ready_at(class, now);
            if ready <= now {
                break;
            }
            log::debug!("rate limit: a {class} request waits {} ms", ready - now);
            self.clock.sleep_until(ready).await;
        }
        let token: Option<String> = sqlx::query_scalar("SELECT ggg_access_token FROM login")
            .fetch_optional(&self.pool)
            .await?;
        let token = token.ok_or(Error::SignedOut)?;
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .expect("the base URL is http(s)")
            .pop_if_empty()
            .extend(request.segments());
        self.limiter().sent(class, self.clock.now());
        let response = ninja::http::send("GGG", self.http.get(url).bearer_auth(token)).await;
        let now = self.clock.now();
        let response = response?;
        let status = response.status();
        let headers = rate_limit_headers(response.headers());
        let mut changed = self.limiter().observe(class, &headers, now);
        let mut cooldown = None;
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok()?.trim().parse().ok());
            let restricted = self.limiter().restrict(class, &headers, retry_after, now);
            if let Some((policy, until)) = restricted {
                if !changed.contains(&policy) {
                    changed.push(policy);
                }
                cooldown = Some(until);
            }
        }
        self.store(&changed).await;
        self.push(app);
        if let Some(until) = cooldown {
            log::warn!("GGG rate limit exceeded by a {class} request");
            return Err(Error::RateLimited {
                seconds_left: seconds_left(until, now),
            });
        }
        if !status.is_success() {
            log::warn!("GGG answered {status} to a {class} request");
            return Err(Error::Status(status));
        }
        Ok(response)
    }

    /// The account's characters (`/character`).
    pub async fn characters<R: Runtime>(
        &self,
        app: &AppHandle<R>,
    ) -> Result<Vec<Character>, Error> {
        let list: CharacterList = self.get(app, Request::Characters).await?;
        Ok(list.characters)
    }

    /// The stash tabs of `league` as GGG lists them (`/stash/<league>`).
    pub async fn stash_tabs<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        league: &str,
    ) -> Result<Vec<StashTab>, Error> {
        let list: StashList = self.get(app, Request::StashList { league }).await?;
        Ok(list.stashes)
    }

    /// A stash tab with its items (`/stash/<league>/<id>`).
    pub async fn stash<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        league: &str,
        id: &str,
    ) -> Result<StashContents, Error> {
        let response: StashResponse = self.get(app, Request::Stash { league, id }).await?;
        Ok(response.stash)
    }

    /// A child tab with its items (`/stash/<league>/<parent>/<id>`).
    pub async fn child_stash<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        league: &str,
        parent: &str,
        id: &str,
    ) -> Result<StashContents, Error> {
        let request = Request::ChildStash { league, parent, id };
        let response: StashResponse = self.get(app, request).await?;
        Ok(response.stash)
    }

    /// A character's inventory and equipment (`/character/<name>`).
    pub async fn character<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        name: &str,
    ) -> Result<CharacterItems, Error> {
        let response: CharacterResponse = self.get(app, Request::Character { name }).await?;
        Ok(response.character)
    }

    /// Upserts the rows of `policies`; a failure is only logged; the limiter
    /// in memory still applies.
    async fn store(&self, policies: &[String]) {
        let rows: Vec<Row> = {
            let limiter = self.limiter();
            policies.iter().filter_map(|p| limiter.row(p)).collect()
        };
        for row in rows {
            if let Err(error) = store(&self.pool, &row).await {
                log::error!("cannot store rate limit policy {}: {error}", row.policy);
            }
        }
    }

    /// Pushes the `rate_limit` slice when it changed, and has the scheduler
    /// push it again when the cooldown ends (F7).
    pub fn push<R: Runtime>(&self, app: &AppHandle<R>) {
        let slice = self.rate_limit();
        if let Some(until) = slice.cooldown_until {
            app.state::<Scheduler>()
                .set(Deadline::RateLimitCooldownEnd, until);
        }
        let store = app.state::<Store>();
        if store.read(|s: &RateLimit| *s == slice) {
            return;
        }
        log::debug!(
            "rate limit: cooldown until {:?}, allowance {:?}",
            slice.cooldown_until,
            slice.allowance
        );
        if let Err(error) = store.update::<RateLimit, _>(app, |s| *s = slice) {
            log::error!("cannot push the rate limit: {error}");
        }
    }
}

/// The `x-rate-limit-*` headers of a response.
fn rate_limit_headers(headers: &HeaderMap) -> Headers {
    headers
        .iter()
        .filter(|(name, _)| name.as_str().starts_with(HEADER_PREFIX))
        .filter_map(|(name, value)| {
            Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
        })
        .collect()
}

/// Every stored policy; a row that cannot be read is skipped.
async fn load(pool: &SqlitePool) -> sqlx::Result<Vec<Row>> {
    let rows: Vec<(String, String, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT policy, headers, endpoints, observed_at, restricted_until \
         FROM rate_limit_policies",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(policy, headers, endpoints, observed_at, restricted_until)| {
                let parsed = serde_json::from_str(&headers)
                    .and_then(|h| Ok((h, serde_json::from_str(&endpoints)?)));
                match parsed {
                    Ok((headers, endpoints)) => Some(Row {
                        policy,
                        headers,
                        endpoints,
                        observed_at,
                        restricted_until,
                    }),
                    Err(error) => {
                        log::warn!("ignored stored rate limit policy {policy}: {error}");
                        None
                    }
                }
            },
        )
        .collect())
}

async fn store(pool: &SqlitePool, row: &Row) -> sqlx::Result<()> {
    let headers = db::json_text(&row.headers)?;
    let endpoints = db::json_text(&row.endpoints)?;
    sqlx::query(
        "INSERT INTO rate_limit_policies \
         (policy, headers, endpoints, observed_at, restricted_until) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT (policy) DO UPDATE SET headers = excluded.headers, \
         endpoints = excluded.endpoints, observed_at = excluded.observed_at, \
         restricted_until = excluded.restricted_until",
    )
    .bind(&row.policy)
    .bind(headers)
    .bind(endpoints)
    .bind(row.observed_at)
    .bind(row.restricted_until)
    .execute(pool)
    .await
    .map(drop)
}
