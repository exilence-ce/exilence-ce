//! The `items` slice of the active Profile (F11): the newest Snapshot's rows,
//! decoded from its body and item definitions and merged across its tabs
//! ([`domain::items::merge`]), and the Comparison rows of the two newest
//! ([`domain::items::compare`]), with tab names and, from the current price
//! table of the Snapshot's price league, each item's trend and poe.ninja page
//! (D3). In a Group, the selected players' newest Snapshots merge into one
//! table without tabs and the Comparison is each player's, merged
//! ([`domain::items::merge_comparisons`], D14). Pushed after a Snapshot,
//! whenever the active Profile, a price table or the Group changes. The UI
//! sorts, pages and filters the rows.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use domain::item_definition::ItemDefinition;
use domain::items::{self, Difference, MergedRow, TabRow};
use domain::pricing::{PriceEntry, PriceTable, PricingOptions, ninja};
use ipc::slices::{ItemRow, ItemRowTab, ItemTab, Items, Profiles};
use prost::Message as _;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};
use wire::v1::SnapshotBody;

use crate::db;
use crate::group::Groups;
use crate::prices::{self, PriceTables};
use crate::session_start::signed_in;
use crate::settings;
use crate::state::{Pushing, Store};

/// Why the item detail of a Snapshot could not be read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("cannot decompress the Snapshot body: {0}")]
    Decompress(#[source] std::io::Error),
    #[error("cannot decode the Snapshot body or an item definition: {0}")]
    Decode(#[from] prost::DecodeError),
}

/// What the `items` pushes reuse, managed by the app: the item detail of
/// the Snapshots they show, by Snapshot id, each decoded once, and the price
/// table their items match, rebuilt only for new prices or other pricing
/// options. Item detail no push used since the previous one is dropped.
#[derive(Default)]
pub struct Cache {
    details: Mutex<HashMap<String, CachedDetail>>,
    prices: Mutex<Option<Arc<ItemPrices>>>,
    decoded: AtomicU64,
    tables_built: AtomicU64,
}

/// A Snapshot's item detail in the [`Cache`].
struct CachedDetail {
    detail: Arc<Detail>,
    /// Used since the previous push.
    used: bool,
}

/// The price entries of an [`ItemPrices`] table and the pricing options it
/// was built under.
struct PriceSource {
    entries: Arc<Vec<PriceEntry>>,
    options: PricingOptions,
}

self_cell::self_cell!(
    /// A price table holding the entries it indexes.
    struct ItemPrices {
        owner: PriceSource,
        #[covariant]
        dependent: PriceTable,
    }
);

impl Cache {
    /// How many stored Snapshot bodies the pushes decoded.
    pub fn decoded(&self) -> u64 {
        self.decoded.load(Ordering::Relaxed)
    }

    /// How many price tables the pushes built.
    pub fn tables_built(&self) -> u64 {
        self.tables_built.load(Ordering::Relaxed)
    }

    /// Holds `detail`, the item detail of Snapshot `id`, for the next push,
    /// unless definitions it references are missing: a Group member's may be
    /// stored later, so a later push loads them again.
    pub fn hold(&self, id: String, detail: Detail) -> Arc<Detail> {
        let detail = Arc::new(detail);
        if detail.complete() {
            let cached = CachedDetail {
                detail: detail.clone(),
                used: true,
            };
            self.details().insert(id, cached);
        }
        detail
    }

    /// The held item detail of Snapshot `id`.
    fn get(&self, id: &str) -> Option<Arc<Detail>> {
        let mut details = self.details();
        let held = details.get_mut(id)?;
        held.used = true;
        Some(held.detail.clone())
    }

    /// Drops the item detail no push used since the previous call.
    fn drop_unused(&self) {
        self.details()
            .retain(|_, held| std::mem::take(&mut held.used));
    }

    /// The price table of `entries` under `options`: the one held unless
    /// either changed.
    fn prices(&self, entries: Arc<Vec<PriceEntry>>, options: PricingOptions) -> Arc<ItemPrices> {
        let mut held = self.prices.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prices) = held.as_ref() {
            let priced = prices.borrow_owner();
            if Arc::ptr_eq(&priced.entries, &entries) && priced.options == options {
                return prices.clone();
            }
        }
        let prices = Arc::new(ItemPrices::new(PriceSource { entries, options }, |p| {
            PriceTable::new(&p.entries, &p.options, &[])
        }));
        self.tables_built.fetch_add(1, Ordering::Relaxed);
        *held = Some(prices.clone());
        prices
    }

    fn details(&self) -> MutexGuard<'_, HashMap<String, CachedDetail>> {
        self.details.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Decompresses and decodes a stored Snapshot body.
    fn decode(&self, body: &[u8]) -> Result<SnapshotBody, Error> {
        let body = zstd::decode_all(body).map_err(Error::Decompress)?;
        let body = SnapshotBody::decode(body.as_slice())?;
        self.decoded.fetch_add(1, Ordering::Relaxed);
        Ok(body)
    }
}

/// A Snapshot's item detail: its body and the definitions the body
/// references.
pub struct Detail {
    body: SnapshotBody,
    definitions: HashMap<u64, ItemDefinition>,
}

impl Detail {
    /// `body` with `definitions`, every definition it references by id.
    pub fn new(body: SnapshotBody, definitions: HashMap<u64, ItemDefinition>) -> Self {
        Self { body, definitions }
    }

    /// Whether every definition the body references is there.
    fn complete(&self) -> bool {
        self.body
            .tabs
            .iter()
            .flat_map(|t| &t.definition_ids)
            .all(|id| self.definitions.contains_key(id))
    }
}

/// The two newest Snapshots of a Profile, newest first; `None` for one
/// without item detail (imported, F21, or past retention, F12), which
/// counts as having no items.
#[derive(Default)]
pub struct Newest(Vec<Option<Arc<Detail>>>);

impl Newest {
    /// The price league whose prices valued the newest Snapshot with item
    /// detail.
    pub fn price_league(&self) -> Option<&str> {
        self.0
            .iter()
            .flatten()
            .next()
            .map(|d| d.body.price_league_id.as_str())
    }
}

/// The item detail of the two newest Snapshots of `profile_id` of
/// `account_id`, from `cache` where it holds them.
pub async fn newest(
    pool: &SqlitePool,
    cache: &Cache,
    account_id: &str,
    profile_id: &str,
) -> Result<Newest, Error> {
    let snapshots: Vec<(String, bool)> = sqlx::query_as(
        "SELECT id, body IS NOT NULL FROM snapshots WHERE account_id = ?1 AND profile_id = ?2 \
         ORDER BY created_at DESC, id DESC LIMIT 2",
    )
    .bind(account_id)
    .bind(profile_id)
    .fetch_all(pool)
    .await?;
    let mut newest = Vec::with_capacity(snapshots.len());
    for (id, has_body) in snapshots {
        newest.push(match (has_body, cache.get(&id)) {
            (false, _) => None,
            (true, Some(detail)) => Some(detail),
            (true, None) => stored(pool, cache, account_id, id).await?,
        });
    }
    Ok(Newest(newest))
}

/// Decodes the stored body of Snapshot `id` of `account_id`, loads the
/// definitions it references and holds them in `cache`; `None` once
/// retention dropped the body (F12).
async fn stored(
    pool: &SqlitePool,
    cache: &Cache,
    account_id: &str,
    id: String,
) -> Result<Option<Arc<Detail>>, Error> {
    let body: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT body FROM snapshots WHERE account_id = ?1 AND id = ?2 AND body IS NOT NULL",
    )
    .bind(account_id)
    .bind(&id)
    .fetch_optional(pool)
    .await?;
    let Some(body) = body else {
        return Ok(None);
    };
    let detail = with_definitions(pool, cache.decode(&body)?).await?;
    Ok(Some(cache.hold(id, detail)))
}

/// The item definition sweep (F12, `client-schema.sql`), once at startup
/// before any Group rejoin or upload: decodes every stored Snapshot body,
/// collects the definition ids they reference and deletes every other
/// definition. A body that cannot be decoded stops the sweep with nothing
/// deleted. How many definitions went.
pub async fn sweep(pool: &SqlitePool) -> Result<u64, Error> {
    let bodies: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT body FROM snapshots WHERE body IS NOT NULL")
            .fetch_all(pool)
            .await?;
    let mut referenced = std::collections::HashSet::new();
    for body in &bodies {
        let body = zstd::decode_all(body.as_slice()).map_err(Error::Decompress)?;
        let body = SnapshotBody::decode(body.as_slice())?;
        referenced.extend(
            body.tabs
                .iter()
                .flat_map(|t| &t.definition_ids)
                .map(|&id| id as i64),
        );
    }
    let ids = db::json_text(&referenced)?;
    let removed = sqlx::query(
        "DELETE FROM item_definitions WHERE id NOT IN (SELECT value FROM json_each(?1))",
    )
    .bind(ids)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(removed)
}

/// `body` with the definitions it references.
async fn with_definitions(pool: &SqlitePool, body: SnapshotBody) -> Result<Detail, Error> {
    let ids: std::collections::HashSet<i64> = body
        .tabs
        .iter()
        .flat_map(|t| &t.definition_ids)
        .map(|&id| id as i64)
        .collect();
    let ids = db::json_text(&ids)?;
    let encoded: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT id, encoded FROM item_definitions \
         WHERE id IN (SELECT value FROM json_each(?1))",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    let definitions = encoded
        .into_iter()
        .map(|(id, encoded)| {
            let definition = wire::v1::ItemDefinition::decode(encoded.as_slice())?;
            Ok((id as u64, from_wire(definition)))
        })
        .collect::<Result<_, prost::DecodeError>>()?;
    Ok(Detail { body, definitions })
}

/// The domain definition of a stored one.
fn from_wire(definition: wire::v1::ItemDefinition) -> ItemDefinition {
    ItemDefinition {
        name: definition.name,
        type_line: definition.type_line,
        frame_type: definition.frame_type,
        icon: definition.icon,
        ilvl: definition.ilvl,
        tier: definition.tier,
        links: definition.links,
        sockets: definition.sockets,
        quality: definition.quality,
        level: definition.level,
        variant: definition.variant,
        corrupted: definition.corrupted,
        elder: definition.elder,
        shaper: definition.shaper,
        blighted: definition.blighted,
        beast: definition.beast,
    }
}

/// The tab rows of `detail`, each with its tab's position in the Snapshot.
fn tab_rows(detail: &Detail) -> Vec<TabRow<'_>> {
    let body = &detail.body;
    let price_of = |id: &u64| {
        body.priced_definition_ids
            .binary_search(id)
            .ok()
            .and_then(|i| body.prices.get(i).copied())
    };
    let mut tab_rows = Vec::new();
    for (tab, snapshot_tab) in body.tabs.iter().enumerate() {
        for (id, &stack_size) in snapshot_tab
            .definition_ids
            .iter()
            .zip(&snapshot_tab.stack_sizes)
        {
            match (detail.definitions.get(id), price_of(id)) {
                (Some(definition), Some(price)) => tab_rows.push(TabRow {
                    tab,
                    definition,
                    stack_size,
                    price,
                }),
                _ => log::warn!("Snapshot row {id:016x} lacks its definition or price"),
            }
        }
    }
    tab_rows
}

/// The tab rows of `detail`, merged.
fn merged(detail: &Detail) -> Vec<MergedRow<'_>> {
    items::merge(tab_rows(detail))
}

/// Builds item rows with the trend and poe.ninja page of the entry each item
/// matches in `table`, the price table of `league`, if loaded (D3).
struct Rows<'a> {
    table: Option<&'a PriceTable<'a>>,
    league: &'a str,
}

impl<'a> Rows<'a> {
    fn new(league: &'a str, table: Option<&'a PriceTable<'a>>) -> Self {
        Self { table, league }
    }

    fn row(
        &self,
        key: String,
        definition: &ItemDefinition,
        price: f64,
        quantity: i64,
        total: f64,
        tabs: Vec<ItemRowTab>,
    ) -> ItemRow {
        let entry = self.table.and_then(|t| t.matching_price(definition));
        ItemRow {
            key,
            name: definition.name.clone(),
            icon: definition.icon.clone(),
            frame_type: definition.frame_type,
            ilvl: definition.ilvl,
            tier: definition.tier,
            corrupted: definition.corrupted,
            links: definition.links,
            quality: definition.quality,
            level: definition.level,
            quantity,
            price,
            total,
            tabs,
            trend: entry.and_then(prices::trend),
            ninja_url: entry
                .and_then(|e| e.details_page.as_deref())
                .map(|page| ninja::details_url(self.league, page)),
        }
    }

    /// The Latest rows of `merged`, keyed by position; with each row's tabs
    /// unless `tabless`.
    fn latest(&self, merged: &[MergedRow<'_>], tabless: bool) -> Vec<ItemRow> {
        merged
            .iter()
            .enumerate()
            .map(|(index, r)| {
                let tabs = if tabless {
                    Vec::new()
                } else {
                    r.tabs
                        .iter()
                        .map(|&(tab, quantity)| ItemRowTab {
                            tab: tab as u32,
                            quantity: signed(quantity),
                        })
                        .collect()
                };
                self.row(
                    index.to_string(),
                    r.definition,
                    r.price,
                    signed(r.quantity),
                    r.total(),
                    tabs,
                )
            })
            .collect()
    }

    /// The Comparison rows of `differences`, keyed by position.
    fn comparison(&self, differences: &[Difference<'_>]) -> Vec<ItemRow> {
        differences
            .iter()
            .enumerate()
            .map(|(index, d)| {
                let key = format!("c{index}");
                self.row(key, d.definition, d.price, d.quantity, d.total, Vec::new())
            })
            .collect()
    }
}

fn signed(quantity: u64) -> i64 {
    i64::try_from(quantity).unwrap_or(i64::MAX)
}

/// The Comparison of a player's two newest Snapshots; none with fewer than 2.
fn differences<'a>(newest: &'a Newest, latest: &[MergedRow<'a>]) -> Vec<Difference<'a>> {
    match newest.0.get(1) {
        Some(previous) => {
            let previous_rows = previous.as_deref().map(merged).unwrap_or_default();
            items::compare(&previous_rows, latest)
        }
        None => Vec::new(),
    }
}

/// The `items` slice of `newest`: the newest Snapshot's rows and the
/// comparison rows of the two newest (none with fewer than 2). `prices` is
/// the current price table of their price league under the pricing
/// options, `None` while not loaded: trends and pages stay blank then (D3).
/// They come from the entry each item matches.
pub fn slice(newest: &Newest, prices: Option<&PriceTable<'_>>) -> Items {
    let Some(league) = newest.price_league() else {
        return Items::default();
    };
    let rows = Rows::new(league, prices);
    let latest = newest.0[0].as_deref();
    let latest_rows = latest.map(merged).unwrap_or_default();
    Items {
        tabs: latest
            .into_iter()
            .flat_map(|l| &l.body.tabs)
            .map(|t| ItemTab {
                id: t.tab_id.clone(),
                name: t.name.clone(),
            })
            .collect(),
        rows: rows.latest(&latest_rows, false),
        comparison_rows: rows.comparison(&differences(newest, &latest_rows)),
        ..Items::default()
    }
}

/// The price league of the Group views: the first player's whose newest
/// Snapshots have item detail.
fn group_price_league(players: &[Newest]) -> Option<&str> {
    players.iter().find_map(Newest::price_league)
}

/// The Group `items` slice of the selected `players` (F11, D14): their
/// newest Snapshots merged into one table without tabs, and each player's
/// Comparison, merged. Trends and pages come from `prices`, the table of
/// [`group_price_league`].
pub fn group_slice(players: &[Newest], prices: Option<&PriceTable<'_>>) -> Items {
    let group = Items {
        group_combined: true,
        ..Items::default()
    };
    let Some(league) = group_price_league(players) else {
        return group;
    };
    let rows = Rows::new(league, prices);
    let latest: Vec<Vec<MergedRow<'_>>> = players
        .iter()
        .map(|p| {
            p.0.first()
                .and_then(Option::as_deref)
                .map(merged)
                .unwrap_or_default()
        })
        .collect();
    let latest_rows = items::merge(
        players
            .iter()
            .filter_map(|p| p.0.first().and_then(Option::as_deref))
            .flat_map(tab_rows),
    );
    let comparison = items::merge_comparisons(
        players
            .iter()
            .zip(&latest)
            .map(|(newest, latest)| differences(newest, latest)),
    );
    Items {
        rows: rows.latest(&latest_rows, true),
        comparison_rows: rows.comparison(&comparison),
        ..group
    }
}

/// A selected player's Snapshot as the Group holds it: its item detail held
/// in the [`Cache`], or its id and body.
enum MemberSnapshot {
    Cached(Arc<Detail>),
    Body(String, SnapshotBody),
}

/// Pushes the `items` slice of the signed-in account's active Profile;
/// empty without one; in a Group, of the selected players (F11, D14). When
/// the newest Snapshot was priced in another price league than the active
/// one and its prices are not loaded, loads them in the background, which
/// pushes again (D3). A failure is only logged.
pub async fn push<R: Runtime>(app: &AppHandle<R>) {
    let pushing = app.state::<Pushing<Items>>();
    let _pushing = pushing.lock().await;
    let pool = app.state::<SqlitePool>();
    let store = app.state::<Store>();
    let cache = app.state::<Cache>();
    let active = store.read(|p: &Profiles| p.active_profile_id.clone());
    // In a Group: each selected player, the account's own or the two newest
    // Snapshots held, newest first.
    let group = app.state::<Groups>().read_members(|m| {
        m.selected.then(|| {
            let snapshots: Vec<Option<MemberSnapshot>> = if m.me {
                Vec::new()
            } else {
                m.snapshots
                    .iter()
                    .rev()
                    .take(2)
                    .map(|s| {
                        let body = s.body.as_ref()?;
                        Some(match cache.get(&s.id) {
                            Some(detail) => MemberSnapshot::Cached(detail),
                            None => MemberSnapshot::Body(s.id.clone(), body.clone()),
                        })
                    })
                    .collect()
            };
            (m.me, snapshots)
        })
    });
    let in_group = group.is_some();
    let loaded = async {
        let account_id: Option<String> = sqlx::query_scalar("SELECT account_id FROM login")
            .fetch_optional(&*pool)
            .await?;
        let own = async || match (&account_id, &active) {
            (Some(account_id), Some(profile_id)) => {
                newest(&pool, &cache, account_id, profile_id).await
            }
            _ => Ok(Newest::default()),
        };
        let Some(group) = group else {
            return Ok(vec![own().await?]);
        };
        let mut players = Vec::new();
        for (me, snapshots) in group.into_iter().flatten() {
            players.push(if me {
                own().await?
            } else {
                let mut details = Vec::with_capacity(snapshots.len());
                for snapshot in snapshots {
                    details.push(match snapshot {
                        Some(MemberSnapshot::Cached(detail)) => Some(detail),
                        Some(MemberSnapshot::Body(id, body)) => {
                            Some(cache.hold(id, with_definitions(&pool, body).await?))
                        }
                        None => None,
                    });
                }
                Newest(details)
            });
        }
        Ok::<_, Error>(players)
    }
    .await;
    cache.drop_unused();
    let items = match loaded {
        Ok(players) => {
            let league = if in_group {
                group_price_league(&players)
            } else {
                players[0].price_league()
            };
            let table = league.and_then(|league| {
                let table = app.state::<PriceTables>().get(league);
                if table.is_none()
                    && signed_in(app)
                    && prices::active_price_league(app).as_deref() != Some(league)
                {
                    tauri::async_runtime::spawn(load_prices(app.clone(), league.to_owned()));
                }
                table
            });
            let options = store.read(settings::pricing_options);
            let prices = table.map(|(entries, _)| cache.prices(entries, options));
            let prices = prices.as_deref().map(ItemPrices::borrow_dependent);
            if in_group {
                group_slice(&players, prices)
            } else {
                slice(&players[0], prices)
            }
        }
        Err(error) => return log::error!("cannot load the items: {error}"),
    };
    if let Err(error) = store.update::<Items, _>(app, |i| *i = items) {
        log::error!("cannot push the items: {error}");
    }
}

/// Loads the prices of `league` (logged by `ensure`), which pushes `items`
/// again. Boxed, as the two pushes refer to each other.
fn load_prices<R: Runtime>(
    app: AppHandle<R>,
    league: String,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let _ = prices::ensure(&app, &league).await;
    })
}
