//! poe.ninja, the price source: the leagues it prices, which are the price
//! leagues (D21), and a price league's price table (F5). Every request
//! legacy makes per price league (`exilence-ce-legacy/src/services/poe-ninja.service.ts:44-95`)
//! is sent conditionally (ETag); the answers are mapped and deduplicated by
//! [`domain::pricing::ninja`].
//!
//! A crate of its own, free of Tauri, so the poe.ninja drift check
//! (`tools/ninja-drift`) runs the app's client without building the app.

pub mod http;

use std::collections::HashMap;

use domain::pricing::PriceEntry;
use domain::pricing::ninja::{
    self as mapping, CurrencyOverview, ExchangeOverview, ItemOverview, OverviewKind,
};
use reqwest::header::{ETAG, IF_NONE_MATCH};
use reqwest::{StatusCode, Url};
use serde::Deserialize;

/// poe.ninja's host.
pub const API_URL: &str = "https://poe.ninja";

/// Why a poe.ninja request failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("poe.ninja answered {0}")]
    Status(StatusCode),
    #[error("poe.ninja unreachable: {0}")]
    Transport(#[source] reqwest::Error),
    /// An answer that does not parse: poe.ninja changed its format.
    #[error("poe.ninja answered an unparsable body: {0}")]
    Parse(#[source] reqwest::Error),
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        if error.is_decode() {
            Self::Parse(error)
        } else {
            Self::Transport(error)
        }
    }
}

/// The poe.ninja client.
pub struct Ninja {
    http: reqwest::Client,
    base_url: Url,
}

#[derive(Deserialize)]
struct League {
    name: String,
}

/// One overview answer kept for the next conditional request.
#[derive(Clone, Debug)]
struct Cached {
    etag: String,
    entries: Vec<PriceEntry>,
}

/// The overview answers behind a price league's table, by request URL: a
/// 304 reuses them.
#[derive(Clone, Debug, Default)]
pub struct Overviews(HashMap<String, Cached>);

impl Ninja {
    /// The client of poe.ninja at `base_url` ([`API_URL`] in the app),
    /// sending `user_agent`.
    pub fn new(base_url: &str, user_agent: String) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            http: http::client(user_agent)?,
            base_url: Url::parse(base_url)?,
        })
    }

    /// `base_url` + `/poe1/api/economy/` + `path`.
    #[expect(
        clippy::expect_used,
        reason = "the base URL is checked as http(s) when the client is built"
    )]
    fn url(&self, path: &[&str]) -> Url {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .expect("the base URL is http(s)")
            .pop_if_empty()
            .extend(["poe1", "api", "economy"])
            .extend(path);
        url
    }

    /// The names of the leagues poe.ninja prices, in its order
    /// (`/poe1/api/economy/leagues`).
    pub async fn price_leagues(&self) -> Result<Vec<String>, Error> {
        let request = self.http.get(self.url(&["leagues"]));
        let response = http::send("poe.ninja", request).await?;
        if !response.status().is_success() {
            return Err(Error::Status(response.status()));
        }
        let leagues: Vec<League> = response.json().await?;
        Ok(leagues.into_iter().map(|l| l.name).collect())
    }

    /// The price table of `league`: every overview of
    /// [`mapping::overviews`], concatenated in legacy order and
    /// deduplicated. Each request carries the ETag of its answer in
    /// `overviews`; a 304 reuses that answer. A 404 is an overview without
    /// prices; a league whose overviews all 404 has an empty table (no
    /// prices). Any other failure fails the whole table and leaves
    /// `overviews` as it was.
    pub async fn price_table(
        &self,
        league: &str,
        overviews: &mut Overviews,
    ) -> Result<Vec<PriceEntry>, Error> {
        let mut answered = Overviews::default();
        let mut entries = Vec::new();
        for (kind, overview_type) in mapping::overviews() {
            let mut url = self.url(match kind {
                OverviewKind::Currency => &["stash", "current", "currency", "overview"],
                OverviewKind::Item => &["stash", "current", "item", "overview"],
                OverviewKind::Exchange => &["exchange", "current", "overview"],
            });
            url.query_pairs_mut()
                .append_pair("league", league)
                .append_pair("type", overview_type);
            let key = url.to_string();
            let cached = overviews.0.get(&key);
            let mut request = self.http.get(url);
            if let Some(cached) = cached {
                request = request.header(IF_NONE_MATCH, &cached.etag);
            }
            let response = http::send("poe.ninja", request).await?;
            let status = response.status();
            let fetched = match (status, cached) {
                (StatusCode::NOT_MODIFIED, Some(cached)) => Some(cached.clone()),
                (StatusCode::NOT_FOUND, _) => None,
                (status, _) if status.is_success() => {
                    let etag = response
                        .headers()
                        .get(ETAG)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    let mapped: Vec<PriceEntry> = match kind {
                        OverviewKind::Currency => response
                            .json::<CurrencyOverview>()
                            .await?
                            .price_entries(overview_type)
                            .collect(),
                        OverviewKind::Item => response
                            .json::<ItemOverview>()
                            .await?
                            .price_entries(overview_type)
                            .collect(),
                        OverviewKind::Exchange => response
                            .json::<ExchangeOverview>()
                            .await?
                            .price_entries(overview_type)
                            .collect(),
                    };
                    match etag {
                        Some(etag) => Some(Cached {
                            etag,
                            entries: mapped,
                        }),
                        None => {
                            entries.extend(mapped);
                            continue;
                        }
                    }
                }
                (status, _) => return Err(Error::Status(status)),
            };
            if let Some(cached) = fetched {
                entries.extend(cached.entries.iter().cloned());
                answered.0.insert(key, cached);
            }
        }
        *overviews = answered;
        Ok(mapping::dedupe(entries))
    }
}
