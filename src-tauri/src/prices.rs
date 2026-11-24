//! Prices (F5, client-background-work.md, prices): the price tables of the
//! price leagues, in memory only (`client-schema.sql`), and the custom
//! prices (F6), stored per price league.
//!
//! - The active price league's prices load at session start and when the
//!   active Profile changes to another price league ([`follow_active`]).
//! - A check every minute ([`check`]) refetches them when older than
//!   60 minutes; it skips while a Snapshot runs, and the next tick catches
//!   up. Only the active price league is checked.
//! - Other price leagues load on demand ([`ensure`], the custom-price table
//!   of F6), when missing or older than 60 minutes.
//! - A failure keeps the previous table and fetched-at time.
//! - A load replaces a table in one swap, so a Snapshot prices against the
//!   table in memory when it starts pricing ([`PriceTables::get`]).
//! - A custom-price change pushes `prices`, whose ex/div rates use the
//!   custom prices (F9); a Snapshot reads them with [`custom_prices`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use domain::pricing::{self, CustomPrice, PriceEntry, PriceKey};
use ipc::command::{self, PriceTableRow};
use ipc::slices::{PriceAvailability, PriceTrend, Prices, Profiles, Status};
use ipc::{Field, IpcError, MessageKey};
use ninja::{Ninja, Overviews};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::scheduler::{Deadline, Scheduler};
use crate::session_start::signed_in;
use crate::state::{Pushing, Store};
use crate::{internal, items, profiles};

/// Prices older than this are fetched again.
pub const MAX_AGE_MS: i64 = 60 * 60 * 1000;
/// How often the active price league's prices are checked.
pub const CHECK_MS: i64 = 60 * 1000;

/// A price league's prices as last fetched.
struct League {
    table: Arc<Vec<PriceEntry>>,
    fetched_at: i64,
    overviews: Overviews,
}

/// The price tables by price league, managed by the app.
#[derive(Default)]
pub struct PriceTables {
    leagues: Mutex<HashMap<String, League>>,
    /// The price league loading now.
    updating: Mutex<Option<String>>,
    /// One load at a time.
    loading: tokio::sync::Mutex<()>,
}

impl PriceTables {
    fn leagues(&self) -> MutexGuard<'_, HashMap<String, League>> {
        self.leagues.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn updating(&self) -> MutexGuard<'_, Option<String>> {
        self.updating.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The price table of `league` in memory now and when it was fetched;
    /// `None` before it first loaded. Empty: poe.ninja has no prices for it.
    pub fn get(&self, league: &str) -> Option<(Arc<Vec<PriceEntry>>, i64)> {
        self.leagues()
            .get(league)
            .map(|l| (l.table.clone(), l.fetched_at))
    }
}

/// The active Profile's price league.
pub fn active_price_league<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    app.state::<Store>().read(|p: &Profiles| {
        let active = p.active_profile_id.as_ref()?;
        p.profiles
            .iter()
            .find(|profile| &profile.id == active)
            .map(|profile| profile.price_league_id.clone())
    })
}

/// Why [`ensure`] loaded no prices; a previous table stays.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Ninja(#[from] ninja::Error),
    /// A refresh of a league that has prices came back empty: taken for a
    /// poe.ninja outage, not for prices that are gone.
    #[error("the refresh came back empty")]
    Empty,
}

/// The price table of `league`, fetched first when missing or older than
/// [`MAX_AGE_MS`]. A failure keeps the previous table and fetched-at time,
/// as does a refresh finding no prices for a league that has some. A
/// refresh with the same prices keeps the held table, so nothing built from
/// it is rebuilt. Pushes `prices`, and `items` after a load, whose trends
/// come from the table (D3).
pub async fn ensure<R: Runtime>(
    app: &AppHandle<R>,
    league: &str,
) -> Result<Arc<Vec<PriceEntry>>, LoadError> {
    let tables = app.state::<PriceTables>();
    let scheduler = app.state::<Scheduler>();
    let _loading = tables.loading.lock().await;
    let previous = tables.get(league);
    if let Some((table, fetched_at)) = &previous
        && scheduler.now() - fetched_at < MAX_AGE_MS
    {
        return Ok(table.clone());
    }
    *tables.updating() = Some(league.to_owned());
    push(app).await;
    let mut overviews = tables
        .leagues()
        .get(league)
        .map(|l| l.overviews.clone())
        .unwrap_or_default();
    let fetched = app
        .state::<Ninja>()
        .price_table(league, &mut overviews)
        .await;
    let mut changed = false;
    let result = match fetched {
        // No prices (e.g. 404 everywhere): right for a league that never had
        // any, but a refresh must not swap a table for nothing.
        Ok(entries)
            if entries.is_empty()
                && previous
                    .as_ref()
                    .is_some_and(|(table, _)| !table.is_empty()) =>
        {
            let error = LoadError::Empty;
            log::warn!("prices of {league} did not refresh, kept the previous ones: {error}");
            Err(error)
        }
        Ok(entries) => {
            log::info!("loaded {} prices of {league}", entries.len());
            let table = match previous {
                Some((held, _)) if *held == entries => held,
                _ => {
                    changed = true;
                    Arc::new(entries)
                }
            };
            tables.leagues().insert(
                league.to_owned(),
                League {
                    table: table.clone(),
                    fetched_at: scheduler.now(),
                    overviews,
                },
            );
            Ok(table)
        }
        Err(error) => {
            log::warn!("prices of {league} did not load: {error}");
            Err(error.into())
        }
    };
    *tables.updating() = None;
    push(app).await;
    if changed {
        items::push(app).await;
    }
    result
}

/// The active price league's prices, loaded when missing or old; a failure
/// is only logged, the minute check retries.
pub async fn ensure_active<R: Runtime>(app: &AppHandle<R>) {
    if let Some(league) = active_price_league(app) {
        // Logged by `ensure`.
        let _ = ensure(app, &league).await;
    }
}

/// After the `profiles` slice changed: when the active price league is
/// another than `prices` shows, shows it and loads its prices in the
/// background.
pub async fn follow_active<R: Runtime>(app: &AppHandle<R>) {
    let league = active_price_league(app);
    let shown = app
        .state::<Store>()
        .read(|p: &Prices| p.price_league_id.clone());
    if league == shown {
        return;
    }
    push(app).await;
    if league.is_some() && signed_in(app) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { ensure_active(&app).await });
    }
}

/// The minute check: sets the next one, then, unless a Snapshot runs,
/// loads the active price league's prices when older than 60 minutes. Stops
/// once signed out.
pub fn check<R: Runtime>(app: &AppHandle<R>) {
    if !signed_in(app) {
        return;
    }
    app.state::<Scheduler>()
        .set_in(Deadline::PriceCheck, CHECK_MS);
    if app.state::<Store>().read(|s: &Status| s.snapshot_running) {
        log::debug!("price check skipped: a Snapshot runs");
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move { ensure_active(&app).await });
}

/// Pushes `prices` for the active price league. The slice is computed and
/// pushed under one lock, so a push never overwrites a newer one.
pub async fn push<R: Runtime>(app: &AppHandle<R>) {
    let tables = app.state::<PriceTables>();
    // Read before the state the slice is computed from, so a later push
    // sees custom prices at least as new.
    let custom_league = active_price_league(app);
    let custom = match &custom_league {
        Some(league) => custom_prices(&app.state::<SqlitePool>(), league)
            .await
            .unwrap_or_else(|error| {
                log::error!("cannot load the custom prices: {error}");
                Vec::new()
            }),
        None => Vec::new(),
    };
    let pushing = app.state::<Pushing<Prices>>();
    let _pushing = pushing.lock().await;
    let league = active_price_league(app);
    let custom: &[CustomPrice] = if league == custom_league {
        &custom
    } else {
        &[]
    };
    let loaded = league.as_deref().and_then(|l| tables.get(l));
    let updating = league.is_some() && *tables.updating() == league;
    let availability = match &loaded {
        _ if updating => PriceAvailability::Loading,
        None => PriceAvailability::NotLoaded,
        Some((table, _)) if table.is_empty() => PriceAvailability::Unavailable,
        Some(_) => PriceAvailability::Available,
    };
    let rate = |name| {
        loaded
            .as_ref()
            .and_then(|(table, _)| pricing::currency_rate(table, custom, name))
    };
    let prices = Prices {
        fetched_at: loaded.as_ref().map(|(_, at)| *at),
        availability,
        exalted_rate: rate("Exalted Orb"),
        divine_rate: rate("Divine Orb"),
        price_league_id: league,
    };
    if let Err(error) = app
        .state::<Store>()
        .update::<Prices, _>(app, |p| *p = prices)
    {
        log::error!("cannot push the prices: {error}");
    }
}

type CustomPriceRow = (
    String,
    String,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
    f64,
);

/// The custom prices of `league` (F6), as `domain::pricing::PriceTable::new`
/// takes them: a Snapshot reads them when it starts pricing.
pub async fn custom_prices(pool: &SqlitePool, league: &str) -> sqlx::Result<Vec<CustomPrice>> {
    let rows: Vec<CustomPriceRow> = sqlx::query_as(
        "SELECT name, icon, frame_type, variant, links, quality, level, ilvl, tier, \
         corrupted, elder, shaper, value FROM custom_prices WHERE price_league_id = ?1",
    )
    .bind(league)
    .fetch_all(pool)
    .await?;
    let number = |v: Option<i64>| v.and_then(|v| u32::try_from(v).ok());
    Ok(rows
        .into_iter()
        .map(
            |(
                name,
                icon,
                frame_type,
                variant,
                links,
                quality,
                level,
                ilvl,
                tier,
                corrupted,
                elder,
                shaper,
                value,
            )| {
                CustomPrice {
                    key: PriceKey {
                        name,
                        icon,
                        frame_type: number(frame_type),
                        variant,
                        links: number(links),
                        quality: number(quality),
                        level: number(level),
                        ilvl: number(ilvl),
                        tier: number(tier),
                        corrupted,
                        elder,
                        shaper,
                    },
                    value,
                }
            },
        )
        .collect())
}

/// `get_price_table(price_league)`: the prices of `league` with their custom
/// prices, loaded first when missing or old; older map generations are left
/// out (F6). When the load fails the previous table serves, if any. Loads
/// the price leagues for the league selector when the `profiles` slice has
/// none (D21).
pub async fn price_table<R: Runtime>(
    app: &AppHandle<R>,
    league: &str,
) -> Result<Vec<PriceTableRow>, IpcError> {
    if let Err(error) = profiles::price_leagues(app).await {
        log::warn!("the price leagues did not load: {error}");
    }
    let table = match ensure(app, league).await {
        Ok(table) => table,
        // Logged by `ensure`.
        Err(error) => match app.state::<PriceTables>().get(league) {
            Some((table, _)) => table,
            None => return Err(internal(error)),
        },
    };
    let custom = custom_prices(&app.state::<SqlitePool>(), league)
        .await
        .map_err(internal)?;
    let custom: HashMap<&PriceKey, f64> = custom.iter().map(|c| (&c.key, c.value)).collect();
    Ok(pricing::newest_map_generation(table.iter())
        .into_iter()
        .map(|entry| PriceTableRow {
            key: ipc_key(&entry.key),
            price: entry.calculated,
            custom_price: custom.get(&entry.key).copied(),
            trend: trend(entry),
            ninja_url: entry
                .details_page
                .as_deref()
                .map(|page| pricing::ninja::details_url(league, page)),
        })
        .collect())
}

/// The "Price last 7 days" of `entry`, if poe.ninja has one (D3).
pub fn trend(entry: &PriceEntry) -> Option<PriceTrend> {
    entry
        .sparkline
        .as_ref()
        .filter(|s| !s.data.is_empty())
        .map(|s| PriceTrend {
            points: s.data.clone(),
            change: s.total_change,
        })
}

/// `set_custom_price(price_league, key, value)`: stores `value` as the
/// custom price of the entry `key` in `league`, replacing any (F6); then
/// pushes `prices`, whose ex/div rates may change (F9).
pub async fn set_custom_price<R: Runtime>(
    app: &AppHandle<R>,
    league: &str,
    key: command::PriceKey,
    value: f64,
) -> Result<(), IpcError> {
    if !(value.is_finite() && value >= 0.0) {
        return Err(IpcError::Validation {
            field: Field::CustomPrice,
            key: MessageKey::ErrorCustomPriceInvalid,
        });
    }
    // The unique index `custom_prices_entry` makes this replace the entry's
    // custom price.
    sqlx::query(
        "INSERT OR REPLACE INTO custom_prices (price_league_id, name, icon, frame_type, \
         variant, links, quality, level, ilvl, tier, corrupted, elder, shaper, value) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )
    .bind(league)
    .bind(key.name)
    .bind(key.icon)
    .bind(key.frame_type)
    .bind(key.variant)
    .bind(key.links)
    .bind(key.quality)
    .bind(key.level)
    .bind(key.ilvl)
    .bind(key.tier)
    .bind(key.corrupted)
    .bind(key.elder)
    .bind(key.shaper)
    .bind(value)
    .execute(&*app.state::<SqlitePool>())
    .await
    .map_err(internal)?;
    push(app).await;
    Ok(())
}

/// `remove_custom_price(price_league, key)`: removes the custom price of the
/// entry `key` in `league`, if any (F6); then pushes `prices` (F9).
pub async fn remove_custom_price<R: Runtime>(
    app: &AppHandle<R>,
    league: &str,
    key: command::PriceKey,
) -> Result<(), IpcError> {
    // `IS`: an absent field matches only another absent field.
    sqlx::query(
        "DELETE FROM custom_prices WHERE price_league_id = ?1 AND name = ?2 AND icon = ?3 \
         AND frame_type IS ?4 AND variant IS ?5 AND links IS ?6 AND quality IS ?7 \
         AND level IS ?8 AND ilvl IS ?9 AND tier IS ?10 AND corrupted IS ?11 \
         AND elder IS ?12 AND shaper IS ?13",
    )
    .bind(league)
    .bind(key.name)
    .bind(key.icon)
    .bind(key.frame_type)
    .bind(key.variant)
    .bind(key.links)
    .bind(key.quality)
    .bind(key.level)
    .bind(key.ilvl)
    .bind(key.tier)
    .bind(key.corrupted)
    .bind(key.elder)
    .bind(key.shaper)
    .execute(&*app.state::<SqlitePool>())
    .await
    .map_err(internal)?;
    push(app).await;
    Ok(())
}

fn ipc_key(key: &PriceKey) -> command::PriceKey {
    command::PriceKey {
        name: key.name.clone(),
        icon: key.icon.clone(),
        frame_type: key.frame_type,
        variant: key.variant.clone(),
        links: key.links,
        quality: key.quality,
        level: key.level,
        ilvl: key.ilvl,
        tier: key.tier,
        corrupted: key.corrupted,
        elder: key.elder,
        shaper: key.shaper,
    }
}
