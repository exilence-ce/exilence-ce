//! Taking a Snapshot (F3, client-background-work.md, snapshot). Never needs
//! the backend (D26):
//!
//! 1. Start conditions: the session is ready, the active league has tabs,
//!    prices of the price league are loaded and not updating, no rate-limit
//!    cooldown runs (F7), and the operation slot is free.
//! 2. Token check (D6).
//! 3. Fetch through the GGG queue: the stash tab list, each selected tab
//!    with its child tabs counted under it, and the character's inventory
//!    and/or equipment as the Character pseudo-tab. Status "Fetching stash
//!    tab i / N".
//! 4. Price against the price table in memory at that moment.
//! 5. Commit: one SQLite transaction with the Snapshot, its tabs, new item
//!    definitions and retention (F12). Cancel is disabled from here.
//! 6. Upload: one `UploadSnapshot` attempt with a 30 s deadline, status
//!    "Saving snapshot". Outside a Group a failure is logged and ignored.
//!
//! Cancel, Reset data, Quit and installing an update stop it until the
//! commit: the in-flight GGG request is dropped and nothing is saved. Any GGG
//! error fails it with a toast.

use std::time::{Duration, Instant};

use domain::pricing::{PriceTable, PricedSnapshot, TabId, TabItems};
use ipc::slices::{
    NotificationKind, PriceAvailability, Prices, Profile, Profiles, Session, SnapshotProgress,
    Status,
};
use ipc::{IpcError, Message, MessageKey};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::{oneshot, watch};
use wire::v1::{SnapshotBody, SnapshotTab, TabKind, TabValue, UploadSnapshotRequest};

use crate::auth::{self, TokenCheck};
use crate::backend::{self, Backend};
use crate::ggg::limiter::seconds_left;
use crate::ggg::{self, CharacterItems, Ggg, StashTab};
use crate::group_upload_queue;
use crate::prices::{self, PriceTables};
use crate::scheduler::Scheduler;
use crate::slot::{self, OperationSlot};
use crate::state::{self, Store};
use crate::{auto_snapshot, charts, db, group, internal, items, net_worth, settings, stash_tabs};

/// Item detail (bodies) is kept for this many of a Profile's newest
/// Snapshots (F12).
pub const BODIES_KEPT: i64 = 10;
/// At most this many Snapshots are kept per Profile; the oldest go first
/// (F12).
pub const SNAPSHOTS_KEPT: i64 = 1000;
/// The one upload attempt's deadline.
pub const UPLOAD_DEADLINE: Duration = Duration::from_secs(30);
/// The name of the Character pseudo-tab (F3).
pub const CHARACTER_TAB: &str = "Character";
/// `SnapshotBody` is compressed with zstd at this level (`exilence.proto`).
const ZSTD_LEVEL: i32 = 3;

/// Cancel of the running Snapshot, managed by the app.
pub struct Control {
    cancel: watch::Sender<bool>,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            cancel: watch::Sender::new(false),
        }
    }
}

/// `take_snapshot`: checks the start conditions, takes the operation slot and
/// runs the Snapshot in the background. Returns once it holds the slot.
pub async fn start<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let (started, result) = oneshot::channel();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move { run(&handle, started).await });
    result
        .await
        .unwrap_or_else(|_| Err(internal("the Snapshot ended before it started")))
}

/// `cancel_snapshot`: stops the running Snapshot unless it is being saved.
pub fn cancel<R: Runtime>(app: &AppHandle<R>) {
    let cancellable = app
        .state::<Store>()
        .read(|s: &Status| s.snapshot_running && !s.snapshot_saving);
    if cancellable {
        log::debug!("Snapshot: cancel requested");
        app.state::<Control>().cancel.send_replace(true);
    }
}

/// The Profile to snapshot, if every start condition but the free slot
/// holds (F3, F7).
fn check_conditions<R: Runtime>(app: &AppHandle<R>) -> Result<Profile, IpcError> {
    let now = app.state::<Scheduler>().now();
    if let Some(until) = app.state::<Ggg>().rate_limit().cooldown_until {
        return Err(IpcError::RateLimited {
            seconds_left: seconds_left(until, now),
        });
    }
    let store = app.state::<Store>();
    let session = store.read(|s: &Session| s.ready && s.league_has_tabs);
    let profile = store.read(|p: &Profiles| {
        let active = p.active_profile_id.as_ref()?;
        p.profiles
            .iter()
            .find(|profile| &profile.id == active)
            .cloned()
    });
    let profile = profile.filter(|_| session).ok_or(IpcError::NotReady)?;
    let prices = store.read(|p: &Prices| {
        p.price_league_id.as_ref() == Some(&profile.price_league_id)
            && p.availability == PriceAvailability::Available
    });
    if !prices {
        return Err(IpcError::NotReady);
    }
    Ok(profile)
}

/// Why a Snapshot ended without being saved.
enum Failure {
    /// Shown as a toast whose description says why.
    Toast(Message),
    /// The user was sent to login or signed out meanwhile; that told them.
    Silent,
}

impl From<ggg::Error> for Failure {
    fn from(error: ggg::Error) -> Self {
        match error {
            ggg::Error::SessionExpired | ggg::Error::SignedOut => Self::Silent,
            error => {
                log::warn!("Snapshot: GGG request failed: {error}");
                Self::Toast(IpcError::from(error).message())
            }
        }
    }
}

/// The Snapshot: start conditions, slot, fetch and price (cancellable),
/// commit, upload. Reports through `started` whether it started.
async fn run<R: Runtime>(app: &AppHandle<R>, started: oneshot::Sender<Result<(), IpcError>>) {
    let begun = check_conditions(app).and_then(|profile| Ok((profile, slot::try_hold(app)?)));
    let (profile, _held) = match begun {
        Ok(begun) => begun,
        Err(error) => {
            log::debug!("Snapshot refused: {error}");
            let _ = started.send(Err(error));
            return;
        }
    };
    let control = app.state::<Control>();
    control.cancel.send_replace(false);
    let mut cancelled_by_user = control.cancel.subscribe();
    let mut cancelled_by_slot = app.state::<OperationSlot>().cancellation();
    let _running = Running::start(app);
    let _ = started.send(Ok(()));
    log::debug!(
        "Snapshot of profile {} started ({} tabs selected)",
        profile.id,
        profile.stash_tab_ids.len()
    );

    let taken = tokio::select! {
        taken = fetch_and_price(app, &profile) => taken,
        _ = cancelled_by_user.wait_for(|c| *c) => return log::info!("Snapshot cancelled"),
        _ = cancelled_by_slot.wait_for(|c| *c) => {
            return log::info!("Snapshot cancelled for another operation");
        }
    };
    let (taken, detail) = match taken {
        Ok(taken) => taken,
        Err(Failure::Silent) => return log::info!("Snapshot stopped: signed out"),
        Err(Failure::Toast(description)) => return failed(app, description),
    };

    set_status(app, |s| {
        s.snapshot_saving = true;
        s.snapshot_progress = None;
        s.message = Some(MessageKey::StatusSavingSnapshot.into());
    });
    let committing = Instant::now();
    let new_definitions = match commit(app, &profile, &taken).await {
        Ok(new_definitions) => {
            log::debug!(
                "Snapshot committed: {} new definitions in {} ms",
                new_definitions.len(),
                committing.elapsed().as_millis()
            );
            new_definitions
        }
        Err(error) => {
            return failed(
                app,
                internal(format!("cannot save the Snapshot: {error}")).message(),
            );
        }
    };
    log::info!(
        "Snapshot {} saved: {} tabs, total {:.2}c",
        taken.snapshot.id,
        taken.snapshot.tab_values.len(),
        taken.snapshot.total_value
    );
    net_worth::snapshot_taken(app).await;
    app.state::<items::Cache>()
        .hold(taken.snapshot.id.clone(), detail);
    items::push(app).await;
    charts::push(app).await;
    state::push(
        app,
        NotificationKind::Success,
        MessageKey::NotificationSnapshotSucceeded.into(),
        None,
    );

    // Cancel is over; Reset data, Quit and an update install still stop the
    // upload, which outside a Group is never retried anyway.
    tokio::select! {
        () = upload(app, taken, new_definitions) => {}
        _ = cancelled_by_slot.wait_for(|c| *c) => {
            log::info!("Snapshot upload dropped for another operation");
        }
    }
}

/// `status.snapshot_running` while alive; when dropped, sets the next
/// auto-snapshot due time (F4) and clears the Snapshot's status.
struct Running<'a, R: Runtime>(&'a AppHandle<R>);

impl<'a, R: Runtime> Running<'a, R> {
    fn start(app: &'a AppHandle<R>) -> Self {
        set_status(app, |s| {
            s.snapshot_running = true;
            s.snapshot_saving = false;
            s.snapshot_progress = None;
            s.message = None;
        });
        Self(app)
    }
}

impl<R: Runtime> Drop for Running<'_, R> {
    fn drop(&mut self) {
        auto_snapshot::snapshot_ended(self.0);
        set_status(self.0, |s| {
            s.snapshot_running = false;
            s.snapshot_saving = false;
            s.snapshot_progress = None;
            s.message = None;
        });
    }
}

fn set_status<R: Runtime>(app: &AppHandle<R>, change: impl FnOnce(&mut Status)) {
    if let Err(error) = app.state::<Store>().update::<Status, _>(app, change) {
        log::error!("cannot push the status: {error}");
    }
}

/// A failed Snapshot: a toast saying why (F3, F18).
fn failed<R: Runtime>(app: &AppHandle<R>, description: Message) {
    log::warn!("Snapshot failed: {}", description.key.id());
    state::push(
        app,
        NotificationKind::Error,
        MessageKey::NotificationSnapshotFailed.into(),
        Some(description),
    );
}

/// A selected stash tab with its items, child tabs' items included.
struct FetchedTab {
    tab: StashTab,
    items: Vec<domain::ggg::Item>,
}

/// A priced Snapshot ready to save and upload.
struct Taken {
    /// As uploaded: totals, tab values and the compressed body.
    snapshot: wire::v1::Snapshot,
    /// Each tab's name, in `tab_values` order.
    tab_names: Vec<String>,
    /// Every definition the body references, `id` set to its content id.
    definitions: Vec<wire::v1::ItemDefinition>,
}

/// Steps 2–4: token check, fetch, price. With the Snapshot's item detail,
/// which the `items` push after the commit reuses.
async fn fetch_and_price<R: Runtime>(
    app: &AppHandle<R>,
    profile: &Profile,
) -> Result<(Taken, items::Detail), Failure> {
    match auth::check_token(app).await {
        TokenCheck::Proceed => {}
        TokenCheck::Expired => {
            return Err(Failure::Toast(MessageKey::ErrorGggTokenExpired.into()));
        }
        TokenCheck::SentToLogin => return Err(Failure::Silent),
    }
    let fetching = Instant::now();
    let (tabs, character) = fetch(app, profile).await?;
    log::debug!(
        "Snapshot fetched: {} tabs, {} items in {} ms",
        tabs.len(),
        tabs.iter().map(|t| t.items.len()).sum::<usize>(),
        fetching.elapsed().as_millis()
    );
    let pricing = Instant::now();
    let priced_at = app.state::<Scheduler>().now();
    let Some((entries, _)) = app.state::<PriceTables>().get(&profile.price_league_id) else {
        return Err(Failure::Toast(MessageKey::ErrorNotReady.into()));
    };
    let options = app.state::<Store>().read(settings::pricing_options);
    let custom = prices::custom_prices(&app.state::<SqlitePool>(), &profile.price_league_id)
        .await
        .map_err(|error| Failure::Toast(internal(error).message()))?;
    let mut items: Vec<TabItems> = tabs
        .iter()
        .map(|t| TabItems {
            id: TabId::Stash(t.tab.id.clone()),
            items: t.items.iter().collect(),
        })
        .collect();
    if let Some(character) = &character {
        items.push(TabItems::character(
            &character.inventory,
            &character.equipment,
            profile.include_inventory,
            profile.include_equipment,
        ));
    }
    let priced = PriceTable::new(&entries, &options, &custom).price_tabs(&items);
    log::debug!(
        "Snapshot priced: {} definitions against {} prices of {} in {} ms",
        priced.definitions.len(),
        entries.len(),
        profile.price_league_id,
        pricing.elapsed().as_millis()
    );
    Ok(snapshot(profile, &tabs, priced, priced_at))
}

/// Step 3: the stash tab list, each selected tab with its child tabs, and
/// the character when the Profile includes its inventory or equipment.
async fn fetch<R: Runtime>(
    app: &AppHandle<R>,
    profile: &Profile,
) -> Result<(Vec<FetchedTab>, Option<CharacterItems>), Failure> {
    let league = profile.league_id.as_str();
    set_status(app, |s| {
        s.message = Some(MessageKey::StatusRefreshingStashTabs.into());
    });
    let list = stash_tabs::load(app, league).await?;
    let selected = selected_tabs(list, &profile.stash_tab_ids);
    if selected.is_empty() {
        return Err(Failure::Toast(MessageKey::ErrorNoStashTabsSelected.into()));
    }
    let count = selected.len() as u32;
    let ggg = app.state::<Ggg>();
    let mut tabs = Vec::with_capacity(selected.len());
    for (i, tab) in selected.into_iter().enumerate() {
        let progress = SnapshotProgress {
            tab: i as u32 + 1,
            count,
        };
        set_status(app, |s| {
            s.message = None;
            s.snapshot_progress = Some(progress);
        });
        log::debug!("Snapshot: fetching stash tab {} / {count}", progress.tab);
        let mut contents = ggg.stash(app, league, &tab.id).await?;
        for child in std::mem::take(&mut contents.children) {
            log::debug!(
                "Snapshot: fetching a child tab of stash tab {}",
                progress.tab
            );
            let child = ggg.child_stash(app, league, &tab.id, &child.id).await?;
            contents.items.extend(child.items);
        }
        tabs.push(FetchedTab {
            tab,
            items: contents.items,
        });
    }
    let character = match profile.character_name.as_deref() {
        Some(name)
            if !name.is_empty() && (profile.include_inventory || profile.include_equipment) =>
        {
            log::debug!("Snapshot: fetching the character");
            Some(ggg.character(app, name).await?)
        }
        _ => None,
    };
    Ok((tabs, character))
}

/// The tabs of `list` the Profile selected, in the list's order, folder tabs
/// replaced by their child tabs as in the tab picker (F3).
fn selected_tabs(list: Vec<StashTab>, selected: &[String]) -> Vec<StashTab> {
    list.into_iter()
        .flat_map(|tab| {
            if tab.children.is_empty() {
                vec![tab]
            } else {
                tab.children
            }
        })
        .filter(|tab| selected.contains(&tab.id))
        .collect()
}

/// The wire form of a priced Snapshot: tab values, and the body with its
/// tabs, rows and prices, compressed once. With its item detail: the body
/// and the definitions it references.
#[expect(
    clippy::expect_used,
    reason = "zstd compressing into a Vec cannot fail"
)]
fn snapshot(
    profile: &Profile,
    tabs: &[FetchedTab],
    priced: PricedSnapshot,
    created_at: i64,
) -> (Taken, items::Detail) {
    let definitions: Vec<wire::v1::ItemDefinition> = priced
        .definitions
        .iter()
        .map(|d| {
            let mut definition = to_wire(&d.definition);
            definition.id = definition.content_id();
            definition
        })
        .collect();
    let mut priced_ids: Vec<(u64, f64)> = definitions
        .iter()
        .zip(&priced.definitions)
        .map(|(definition, d)| (definition.id, d.price))
        .collect();
    priced_ids.sort_by_key(|(id, _)| *id);
    let mut body = SnapshotBody {
        price_league_id: profile.price_league_id.clone(),
        priced_definition_ids: priced_ids.iter().map(|(id, _)| *id).collect(),
        prices: priced_ids.iter().map(|(_, price)| *price).collect(),
        tabs: Vec::with_capacity(priced.tabs.len()),
    };
    let mut tab_values = Vec::with_capacity(priced.tabs.len());
    let mut tab_names = Vec::with_capacity(priced.tabs.len());
    for priced_tab in &priced.tabs {
        let mut tab = match &priced_tab.id {
            TabId::Stash(id) => {
                let fetched = tabs.iter().find(|t| &t.tab.id == id).map(|t| &t.tab);
                SnapshotTab {
                    tab_id: id.clone(),
                    kind: TabKind::Stash.into(),
                    name: fetched.map(|t| t.name.clone()).unwrap_or_default(),
                    index: fetched.map_or(0, |t| t.index),
                    colour: fetched
                        .and_then(|t| t.metadata.colour.clone())
                        .unwrap_or_default(),
                    ..SnapshotTab::default()
                }
            }
            TabId::Character => SnapshotTab {
                kind: TabKind::Character.into(),
                name: CHARACTER_TAB.to_owned(),
                ..SnapshotTab::default()
            },
        };
        tab.definition_ids = priced_tab
            .rows
            .iter()
            .map(|row| definitions[row.definition].id)
            .collect();
        tab.stack_sizes = priced_tab.rows.iter().map(|row| row.stack_size).collect();
        tab_values.push(TabValue {
            tab_id: tab.tab_id.clone(),
            value: priced_tab.value,
        });
        tab_names.push(tab.name.clone());
        body.tabs.push(tab);
    }
    let encoded = prost::Message::encode_to_vec(&body);
    let compressed =
        zstd::bulk::compress(&encoded, ZSTD_LEVEL).expect("zstd compresses into memory");
    let total_value = priced.total_value();
    let detail = items::Detail::new(
        body,
        definitions
            .iter()
            .map(|d| d.id)
            .zip(priced.definitions.into_iter().map(|d| d.definition))
            .collect(),
    );
    let taken = Taken {
        snapshot: wire::v1::Snapshot {
            id: uuid::Uuid::now_v7().to_string(),
            profile_id: profile.id.clone(),
            created_at: Some(prost_types::Timestamp {
                seconds: created_at.div_euclid(1000),
                nanos: (created_at.rem_euclid(1000) * 1_000_000) as i32,
            }),
            total_value,
            tab_values,
            body: compressed,
        },
        tab_names,
        definitions,
    };
    (taken, detail)
}

/// The wire definition of `definition`, `id` left 0.
fn to_wire(definition: &domain::item_definition::ItemDefinition) -> wire::v1::ItemDefinition {
    wire::v1::ItemDefinition {
        id: 0,
        name: definition.name.clone(),
        type_line: definition.type_line.clone(),
        frame_type: definition.frame_type,
        icon: definition.icon.clone(),
        ilvl: definition.ilvl,
        tier: definition.tier,
        links: definition.links,
        sockets: definition.sockets,
        quality: definition.quality,
        level: definition.level,
        variant: definition.variant.clone(),
        corrupted: definition.corrupted,
        elder: definition.elder,
        shaper: definition.shaper,
        blighted: definition.blighted,
        beast: definition.beast,
    }
}

/// Step 5: one transaction with the Snapshot, its tabs, its definitions not
/// stored yet and retention (F12). The ids of its definitions the backend
/// does not have yet (`new_definitions`).
async fn commit<R: Runtime>(
    app: &AppHandle<R>,
    profile: &Profile,
    taken: &Taken,
) -> sqlx::Result<Vec<u64>> {
    let pool = app.state::<SqlitePool>();
    let account_id: String = sqlx::query_scalar("SELECT account_id FROM login")
        .fetch_one(&*pool)
        .await?;
    let snapshot = &taken.snapshot;
    let created_at = snapshot.created_at.as_ref().map_or(0, backend::unix_ms);
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO snapshots (account_id, id, profile_id, created_at, total_value, body) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&account_id)
    .bind(&snapshot.id)
    .bind(&profile.id)
    .bind(created_at)
    .bind(snapshot.total_value)
    .bind(&snapshot.body)
    .execute(&mut *tx)
    .await?;
    let tab_rows: Vec<_> = snapshot
        .tab_values
        .iter()
        .zip(&taken.tab_names)
        .enumerate()
        .collect();
    for chunk in tab_rows.chunks(100) {
        let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
            "INSERT INTO snapshot_tabs (account_id, snapshot_id, position, tab_id, name, value) ",
        );
        qb.push_values(chunk, |mut b, &(position, (value, name))| {
            b.push_bind(&account_id)
                .push_bind(&snapshot.id)
                .push_bind(position as i64)
                .push_bind(&value.tab_id)
                .push_bind(name)
                .push_bind(value.value);
        });
        qb.build().execute(&mut *tx).await?;
    }
    for chunk in taken.definitions.chunks(500) {
        let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> =
            sqlx::QueryBuilder::new("INSERT INTO item_definitions (id, encoded) ");
        qb.push_values(chunk, |mut b, definition| {
            b.push_bind(definition.id as i64)
                .push_bind(definition.content_bytes());
        });
        qb.push(" ON CONFLICT (id) DO NOTHING");
        qb.build().execute(&mut *tx).await?;
    }
    retain(&mut tx, &account_id, &profile.id).await?;
    let ids = db::json_text(
        &taken
            .definitions
            .iter()
            .map(|d| d.id as i64)
            .collect::<Vec<_>>(),
    )?;
    let new: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM item_definitions \
         WHERE uploaded = 0 AND id IN (SELECT value FROM json_each(?1))",
    )
    .bind(ids)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(new.into_iter().map(|id| id as u64).collect())
}

/// Retention (F12, `client-schema.sql`): item detail for the Profile's
/// newest [`BODIES_KEPT`] Snapshots, at most [`SNAPSHOTS_KEPT`] Snapshots.
pub async fn retain(
    tx: &mut sqlx::SqliteConnection,
    account_id: &str,
    profile_id: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE snapshots SET body = NULL \
         WHERE account_id = ?1 AND body IS NOT NULL \
         AND id IN ( \
             SELECT id FROM snapshots \
             WHERE account_id = ?1 AND profile_id = ?2 \
             ORDER BY created_at DESC, id DESC LIMIT -1 OFFSET ?3 \
         )",
    )
    .bind(account_id)
    .bind(profile_id)
    .bind(BODIES_KEPT)
    .execute(&mut *tx)
    .await?;
    let removed = sqlx::query(
        "DELETE FROM snapshots \
         WHERE account_id = ?1 \
         AND id IN ( \
             SELECT id FROM snapshots \
             WHERE account_id = ?1 AND profile_id = ?2 \
             ORDER BY created_at DESC, id DESC LIMIT -1 OFFSET ?3 \
         )",
    )
    .bind(account_id)
    .bind(profile_id)
    .bind(SNAPSHOTS_KEPT)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if removed > 0 {
        log::debug!("retention removed {removed} Snapshots of profile {profile_id}");
    }
    Ok(())
}

/// Step 6: one `UploadSnapshot` attempt with [`UPLOAD_DEADLINE`], carrying
/// the definitions the backend does not have yet. Outside a Group any
/// failure, transport or status (`SUPERSEDED` included), is logged and
/// ignored: no retry, no toast, no state change (D26). In a Group a failed
/// upload goes to the Group upload queue (D27, [`group_upload_queue::upload_failed`]).
async fn upload<R: Runtime>(app: &AppHandle<R>, taken: Taken, new_ids: Vec<u64>) {
    let id = taken.snapshot.id.clone();
    if app.state::<Backend>().check_superseded().is_err() {
        return log::info!("UploadSnapshot of {id} skipped: superseded (D26)");
    }
    let session_token: Option<String> = match sqlx::query_scalar("SELECT session_token FROM login")
        .fetch_optional(&*app.state::<SqlitePool>())
        .await
    {
        Ok(token) => token,
        Err(error) => return log::error!("cannot read the Login for UploadSnapshot: {error}"),
    };
    let Some(session_token) = session_token else {
        return log::info!("UploadSnapshot of {id} skipped: signed out");
    };
    let new_ids: std::collections::HashSet<u64> = new_ids.into_iter().collect();
    let new_definitions: Vec<_> = taken
        .definitions
        .into_iter()
        .filter(|d| new_ids.contains(&d.id))
        .collect();
    let message = UploadSnapshotRequest {
        snapshot: Some(taken.snapshot),
        new_definitions,
    };
    // In a Group, kept for the Group upload queue should this attempt fail.
    let kept = group::in_group(app).then(|| message.clone());
    let uploading = Instant::now();
    match (send(app, &session_token, message).await, kept) {
        (Ok(()), _) => log::debug!(
            "Snapshot {id} uploaded in {} ms",
            uploading.elapsed().as_millis()
        ),
        (Err(status), Some(message)) => group_upload_queue::upload_failed(app, message, &status),
        (Err(status), None) => log::info!(
            "UploadSnapshot of {id} failed ({:?}: {}); ignored outside a Group (D26)",
            status.code(),
            status.message()
        ),
    }
}

/// One `UploadSnapshot` with [`UPLOAD_DEADLINE`]; once it succeeded, the
/// definitions it carried are marked as the backend's.
pub(crate) async fn send<R: Runtime>(
    app: &AppHandle<R>,
    session_token: &str,
    message: UploadSnapshotRequest,
) -> Result<(), tonic::Status> {
    let uploaded_ids: Vec<i64> = message
        .new_definitions
        .iter()
        .map(|d| d.id as i64)
        .collect();
    let mut request = backend::request("UploadSnapshot", message, Some(session_token));
    request.set_timeout(UPLOAD_DEADLINE);
    app.state::<Backend>()
        .profile()
        .upload_snapshot(request)
        .await?;
    mark_uploaded(&app.state::<SqlitePool>(), &uploaded_ids).await;
    Ok(())
}

/// The backend has these definitions now (`client-schema.sql`).
async fn mark_uploaded(pool: &SqlitePool, ids: &[i64]) {
    if ids.is_empty() {
        return;
    }
    let marked = async {
        let ids = db::json_text(ids)?;
        sqlx::query(
            "UPDATE item_definitions SET uploaded = 1 WHERE id IN (SELECT value FROM json_each(?1))",
        )
        .bind(ids)
        .execute(pool)
        .await
    };
    if let Err(error) = marked.await {
        log::error!("cannot mark the uploaded item definitions: {error}");
    }
}
