//! The account's Profiles (F2) as this device holds them
//! (`client-schema.sql`, `profiles`): a mirror of the backend's list,
//! replaced by every `StartSession` answer (client-background-work.md,
//! session start). A Profile change goes to the backend first, under the
//! operation slot, and is written here only after it succeeded; when the
//! backend cannot be reached it fails with a toast (D26).

use std::sync::atomic::{AtomicBool, Ordering};

use ipc::command::{NewProfile, ProfileOptions, StashTab};
use ipc::slices::{NotificationKind, Profile, Profiles};
use ipc::{Field, IpcError, Message, MessageKey};
use ninja::Ninja;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};
use tonic::{Request, Response, Status};
use uuid::Uuid;
use wire::v1::profile_service_client::ProfileServiceClient;
use wire::v1::{
    CreateProfileRequest, RemoveProfileRequest, SetActiveProfileRequest, UpdateProfileRequest,
};

use crate::backend::{self, Backend, LoggedChannel};
use crate::ggg::{self, Ggg};
use crate::scheduler::{Deadline, Scheduler};
use crate::session_start::{self, Characters};
use crate::slot;
use crate::state::{self, Pushing, Store};
use crate::{auth, charts, db, internal, items, net_worth, prices, stash_tabs};

/// Where a Profile whose league no longer exists goes (F2).
pub const STANDARD: &str = "Standard";

/// The name of the Profile created at first login (F2).
pub const FIRST_PROFILE_NAME: &str = "profile 1";

/// Whether the signed-in account has a stored Profile.
pub async fn any_stored(pool: &SqlitePool) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM profiles p JOIN login l ON l.account_id = p.account_id)",
    )
    .fetch_one(pool)
    .await
}

/// The `profiles` slice of `account_id` as stored: Profiles in creation
/// order and the active Profile, read in one transaction, so a [`replace`]
/// never pairs one list with another's active Profile. Price leagues are
/// not stored.
pub async fn load(pool: &SqlitePool, account_id: &str) -> sqlx::Result<Profiles> {
    type Row = (
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        bool,
        bool,
    );
    let mut tx = pool.begin().await?;
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, name, league_id, price_league_id, stash_tab_ids, character_name, \
         include_equipment, include_inventory \
         FROM profiles WHERE account_id = ?1 ORDER BY position",
    )
    .bind(account_id)
    .fetch_all(&mut *tx)
    .await?;
    let active_profile_id: Option<String> =
        sqlx::query_scalar("SELECT active_profile_id FROM accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await?
            .flatten();
    tx.commit().await?;
    let profiles = rows
        .into_iter()
        .map(
            |(id, name, league_id, price_league_id, tabs, character_name, equipment, inventory)| {
                Profile {
                    id,
                    name,
                    league_id,
                    price_league_id,
                    // The column is checked to be a JSON array.
                    stash_tab_ids: serde_json::from_str(&tabs).unwrap_or_default(),
                    character_name,
                    include_equipment: equipment,
                    include_inventory: inventory,
                }
            },
        )
        .collect();
    Ok(Profiles {
        profiles,
        active_profile_id,
        ..Profiles::default()
    })
}

/// Replaces the Profile list of `account_id` with `profiles` (in creation
/// order) and its active Profile with `active_profile_id` (empty: none), in
/// one transaction. A Profile missing from the list goes, with its
/// Snapshots. The caller holds the operation slot.
pub async fn replace(
    pool: &SqlitePool,
    account_id: &str,
    profiles: &[wire::v1::Profile],
    active_profile_id: &str,
) -> sqlx::Result<()> {
    let ids = db::json_text(&profiles.iter().map(|p| &p.id).collect::<Vec<_>>())?;
    let mut tx = pool.begin().await?;
    // Unset first: the active Profile may be among those that go.
    sqlx::query("UPDATE accounts SET active_profile_id = NULL WHERE id = ?1")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM profiles WHERE account_id = ?1 \
         AND id NOT IN (SELECT value FROM json_each(?2))",
    )
    .bind(account_id)
    .bind(&ids)
    .execute(&mut *tx)
    .await?;
    for (position, profile) in profiles.iter().enumerate() {
        let tabs = db::json_text(&profile.stash_tab_ids)?;
        sqlx::query(
            "INSERT INTO profiles (account_id, id, position, name, league_id, price_league_id, \
             stash_tab_ids, character_name, include_equipment, include_inventory) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT (account_id, id) DO UPDATE SET position = excluded.position, \
             name = excluded.name, league_id = excluded.league_id, \
             price_league_id = excluded.price_league_id, \
             stash_tab_ids = excluded.stash_tab_ids, character_name = excluded.character_name, \
             include_equipment = excluded.include_equipment, \
             include_inventory = excluded.include_inventory",
        )
        .bind(account_id)
        .bind(&profile.id)
        .bind(position as i64)
        .bind(&profile.name)
        .bind(&profile.league_id)
        .bind(&profile.price_league_id)
        .bind(tabs)
        .bind(&profile.character_name)
        .bind(profile.include_equipment)
        .bind(profile.include_inventory)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE accounts SET active_profile_id = ?2 WHERE id = ?1 \
         AND ?2 IN (SELECT id FROM profiles WHERE account_id = ?1)",
    )
    .bind(account_id)
    .bind(active_profile_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// Stores `profile` for `account_id`: a new one after the others, an
/// existing one in its place.
async fn save(pool: &SqlitePool, account_id: &str, profile: &Profile) -> sqlx::Result<()> {
    let tabs = db::json_text(&profile.stash_tab_ids)?;
    sqlx::query(
        "INSERT INTO profiles (account_id, id, position, name, league_id, price_league_id, \
         stash_tab_ids, character_name, include_equipment, include_inventory) \
         VALUES (?1, ?2, (SELECT coalesce(max(position) + 1, 0) FROM profiles \
         WHERE account_id = ?1), ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
         ON CONFLICT (account_id, id) DO UPDATE SET name = excluded.name, \
         league_id = excluded.league_id, price_league_id = excluded.price_league_id, \
         stash_tab_ids = excluded.stash_tab_ids, character_name = excluded.character_name, \
         include_equipment = excluded.include_equipment, \
         include_inventory = excluded.include_inventory",
    )
    .bind(account_id)
    .bind(&profile.id)
    .bind(&profile.name)
    .bind(&profile.league_id)
    .bind(&profile.price_league_id)
    .bind(tabs)
    .bind(&profile.character_name)
    .bind(profile.include_equipment)
    .bind(profile.include_inventory)
    .execute(pool)
    .await
    .map(drop)
}

async fn save_active(pool: &SqlitePool, account_id: &str, profile_id: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE accounts SET active_profile_id = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind(profile_id)
        .execute(pool)
        .await
        .map(drop)
}

/// Deletes the Profile with its Snapshots (by cascade) and the removals of
/// its Snapshots the backend has not confirmed: `RemoveProfile` took the
/// backend's copies (`client-schema.sql`, `pending_removals`).
async fn delete(pool: &SqlitePool, account_id: &str, profile_id: &str) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM pending_removals WHERE account_id = ?1 AND profile_id = ?2")
        .bind(account_id)
        .bind(profile_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM profiles WHERE account_id = ?1 AND id = ?2")
        .bind(account_id)
        .bind(profile_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Puts each Profile whose league none of the account's characters is in
/// any more into `Standard` (F2). `leagues` are the characters' leagues;
/// nothing changes before they are known.
pub fn fall_back(profiles: &mut [Profile], leagues: &[String]) {
    if leagues.is_empty() {
        return;
    }
    for profile in profiles {
        if !leagues.contains(&profile.league_id) {
            profile.league_id = STANDARD.to_owned();
        }
    }
}

/// Pushes the signed-in account's stored Profiles as the `profiles` slice,
/// with the `Standard` fallback applied, then follows the active Profile:
/// its league's stash tabs ([`stash_tabs::follow_active`]), its `net_worth`
/// (F8), `items` (F11) and `charts` (F10), and its price league (F5,
/// [`prices::follow_active`]). A failure is only logged.
pub async fn push<R: Runtime>(app: &AppHandle<R>) {
    {
        let pushing = app.state::<Pushing<Profiles>>();
        // From before the stored Profiles and the leagues are read until the
        // slice went out: a push that read them before a change (a
        // `StartSession` answer, the characters) never lands after the push
        // of that change.
        let _pushing = pushing.lock().await;
        let pool = app.state::<SqlitePool>();
        let loaded = async {
            let account_id: Option<String> = sqlx::query_scalar("SELECT account_id FROM login")
                .fetch_optional(&*pool)
                .await?;
            match account_id {
                Some(account_id) => load(&pool, &account_id).await.map(Some),
                None => Ok(None),
            }
        }
        .await;
        let loaded = match loaded {
            Ok(Some(loaded)) => loaded,
            Ok(None) => return,
            Err(error) => {
                log::error!("cannot load the profiles: {error}");
                return;
            }
        };
        let leagues = app.state::<Characters>().leagues();
        let pushed = app.state::<Store>().update::<Profiles, _>(app, |p| {
            p.profiles = loaded.profiles;
            p.active_profile_id = loaded.active_profile_id;
            fall_back(&mut p.profiles, &leagues);
        });
        if let Err(error) = pushed {
            log::error!("cannot push the profiles: {error}");
        }
    }
    stash_tabs::follow_active(app);
    tokio::join!(net_worth::push(app), items::push(app), charts::push(app),);
    prices::follow_active(app).await;
}

/// The price leagues of the `profiles` slice, loaded from poe.ninja first
/// when it has none (D21).
pub async fn price_leagues<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<String>, ninja::Error> {
    let store = app.state::<Store>();
    let known = store.read(|p: &Profiles| p.price_leagues.clone());
    if !known.is_empty() {
        return Ok(known);
    }
    let loaded = app.state::<Ninja>().price_leagues().await?;
    log::info!("loaded {} price leagues", loaded.len());
    let leagues = loaded.clone();
    if let Err(error) = store.update::<Profiles, _>(app, |p| p.price_leagues = leagues) {
        log::error!("cannot push the profiles: {error}");
    }
    Ok(loaded)
}

/// The tab picker's tabs: folder tabs replaced by their child tabs (F3).
pub fn picker_tabs(tabs: Vec<ggg::StashTab>) -> Vec<StashTab> {
    tabs.into_iter()
        .flat_map(|tab| {
            if tab.children.is_empty() {
                vec![tab]
            } else {
                tab.children
            }
        })
        .map(|tab| StashTab {
            id: tab.id,
            name: tab.name,
            index: tab.index,
            colour: tab.metadata.colour,
        })
        .collect()
}

/// The account's characters: those the session start loaded, else loaded
/// now through the GGG queue.
async fn characters<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<ggg::Character>, IpcError> {
    let known = app.state::<Characters>().list();
    if !known.is_empty() {
        return Ok(known);
    }
    let loaded = app.state::<Ggg>().characters(app).await?;
    app.state::<Characters>().set(loaded.clone());
    push(app).await;
    Ok(loaded)
}

/// `get_profile_options(league)`: the leagues of the account's characters,
/// and the stash tabs and characters of `league` (the first league when
/// `None`), each GGG request through the queue (F7). Loads the price leagues
/// when the slice has none (D21). A failure shows a toast.
pub async fn options<R: Runtime>(
    app: &AppHandle<R>,
    league: Option<String>,
) -> Result<ProfileOptions, IpcError> {
    let title = MessageKey::NotificationProfileOptionsFailed;
    let characters = match characters(app).await {
        Ok(characters) => characters,
        Err(error) => return Err(auth::action_failed(app, title, error).await),
    };
    if let Err(error) = price_leagues(app).await {
        log::warn!("the price leagues did not load: {error}");
        state::push(
            app,
            NotificationKind::Error,
            title.into(),
            Some(MessageKey::ErrorPriceLeagues.into()),
        );
        return Err(internal(error));
    }
    let leagues = app.state::<Characters>().leagues();
    let Some(league) = league.or_else(|| leagues.first().cloned()) else {
        return Ok(ProfileOptions::default());
    };
    let stash_tabs = match stash_tabs::load(app, &league).await {
        Ok(tabs) => picker_tabs(tabs),
        Err(error) => return Err(auth::action_failed(app, title, error.into()).await),
    };
    let characters = characters
        .into_iter()
        .filter(|c| c.league.as_ref() == Some(&league))
        .map(|c| c.name)
        .collect();
    Ok(ProfileOptions {
        league: Some(league),
        leagues,
        stash_tabs,
        characters,
    })
}

/// The signed-in Login as the ProfileService sees it; also used by the
/// legacy import (F21).
pub(crate) struct Remote<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    pub(crate) account_id: String,
    session_token: String,
}

impl<'a, R: Runtime> Remote<'a, R> {
    /// The stored Login; `SessionExpired` without one.
    pub(crate) async fn signed_in(app: &'a AppHandle<R>) -> Result<Self, IpcError> {
        let login: Option<(String, String)> =
            sqlx::query_as("SELECT account_id, session_token FROM login")
                .fetch_optional(&*app.state::<SqlitePool>())
                .await
                .map_err(internal)?;
        let (account_id, session_token) = login.ok_or(IpcError::SessionExpired)?;
        Ok(Self {
            app,
            account_id,
            session_token,
        })
    }

    /// Sends `message` as the request of `rpc` with `send`. Fails at once
    /// once superseded (D25); a refusal goes through [`backend::failure`].
    async fn call<T, U, F>(
        &self,
        rpc: &str,
        message: T,
        send: impl FnOnce(ProfileServiceClient<LoggedChannel>, Request<T>) -> F,
    ) -> Result<(), IpcError>
    where
        F: Future<Output = Result<Response<U>, Status>>,
    {
        let backend = self.app.state::<Backend>();
        backend.check_superseded()?;
        let request = backend::request(rpc, message, Some(&self.session_token));
        match send(backend.profile(), request).await {
            Ok(_) => Ok(()),
            Err(status) => Err(backend::failure(self.app, &status)),
        }
    }

    pub(crate) async fn create(&self, profile: &Profile) -> Result<(), IpcError> {
        let message = CreateProfileRequest {
            profile: Some(to_wire(profile)),
        };
        self.call("CreateProfile", message, |mut client, request| async move {
            client.create_profile(request).await
        })
        .await
    }

    async fn update(&self, profile: &Profile) -> Result<(), IpcError> {
        let message = UpdateProfileRequest {
            profile: Some(to_wire(profile)),
        };
        self.call("UpdateProfile", message, |mut client, request| async move {
            client.update_profile(request).await
        })
        .await
    }

    pub(crate) async fn set_active(&self, profile_id: &str) -> Result<(), IpcError> {
        let message = SetActiveProfileRequest {
            profile_id: profile_id.to_owned(),
        };
        self.call(
            "SetActiveProfile",
            message,
            |mut client, request| async move { client.set_active_profile(request).await },
        )
        .await
    }

    async fn remove(&self, profile_id: &str) -> Result<(), IpcError> {
        let message = RemoveProfileRequest {
            profile_id: profile_id.to_owned(),
        };
        self.call("RemoveProfile", message, |mut client, request| async move {
            client.remove_profile(request).await
        })
        .await
    }
}

fn to_wire(profile: &Profile) -> wire::v1::Profile {
    wire::v1::Profile {
        id: profile.id.clone(),
        name: profile.name.clone(),
        league_id: profile.league_id.clone(),
        price_league_id: profile.price_league_id.clone(),
        stash_tab_ids: profile.stash_tab_ids.clone(),
        character_name: profile.character_name.clone(),
        include_equipment: profile.include_equipment,
        include_inventory: profile.include_inventory,
    }
}

/// `profile` with its name trimmed, if it has a name (F2) and both leagues.
fn valid(mut profile: Profile) -> Result<Profile, IpcError> {
    profile.name = profile.name.trim().to_owned();
    if profile.name.is_empty() {
        return Err(IpcError::Validation {
            field: Field::ProfileName,
            key: MessageKey::ErrorProfileNameRequired,
        });
    }
    if profile.league_id.is_empty() || profile.price_league_id.is_empty() {
        return Err(internal("a profile needs a league and a price league"));
    }
    Ok(profile)
}

/// The stored Profiles of the Login's account.
async fn stored<R: Runtime>(remote: &Remote<'_, R>) -> Result<Profiles, IpcError> {
    load(&remote.app.state::<SqlitePool>(), &remote.account_id)
        .await
        .map_err(internal)
}

/// The outcome of a Profile command for the user: a success is noted in the
/// notification list; a failure also shows a toast (F18), except an inline
/// validation error and an expired session, which sends the user to login
/// with its own toast (F1).
async fn finish<R: Runtime>(
    app: &AppHandle<R>,
    done: Result<(), IpcError>,
    succeeded: MessageKey,
    title: MessageKey,
) -> Result<(), IpcError> {
    match done {
        Ok(()) => {
            state::push(app, NotificationKind::Success, succeeded.into(), None);
            Ok(())
        }
        Err(error) => Err(auth::action_failed(app, title, error).await),
    }
}

/// `create_profile(profile)`: creates the Profile on the backend, stores it,
/// then makes it the active one there and here (F2).
pub async fn create<R: Runtime>(app: &AppHandle<R>, new: NewProfile) -> Result<(), IpcError> {
    let done = async {
        let _held = slot::try_hold(app)?;
        let profile = valid(new.with_id(Uuid::now_v7().to_string()))?;
        let remote = Remote::signed_in(app).await?;
        let pool = app.state::<SqlitePool>();
        remote.create(&profile).await?;
        save(&pool, &remote.account_id, &profile)
            .await
            .map_err(internal)?;
        let activated = remote.set_active(&profile.id).await;
        if activated.is_ok() {
            save_active(&pool, &remote.account_id, &profile.id)
                .await
                .map_err(internal)?;
        }
        push(app).await;
        activated
    }
    .await;
    finish(
        app,
        done,
        MessageKey::NotificationProfileCreated,
        MessageKey::NotificationProfileCreateFailed,
    )
    .await
}

/// `update_profile(profile)`: replaces the stored Profile with the same id,
/// on the backend first (F2).
pub async fn update<R: Runtime>(app: &AppHandle<R>, profile: Profile) -> Result<(), IpcError> {
    let done = async {
        let _held = slot::try_hold(app)?;
        let profile = valid(profile)?;
        let remote = Remote::signed_in(app).await?;
        if !stored(&remote)
            .await?
            .profiles
            .iter()
            .any(|p| p.id == profile.id)
        {
            return Err(internal(format!("no profile {}", profile.id)));
        }
        remote.update(&profile).await?;
        save(&app.state::<SqlitePool>(), &remote.account_id, &profile)
            .await
            .map_err(internal)?;
        push(app).await;
        Ok(())
    }
    .await;
    finish(
        app,
        done,
        MessageKey::NotificationProfileUpdated,
        MessageKey::NotificationProfileUpdateFailed,
    )
    .await
}

/// `set_active_profile(profile_id)`: switches the active Profile, on the
/// backend first (F2).
pub async fn set_active<R: Runtime>(
    app: &AppHandle<R>,
    profile_id: String,
) -> Result<(), IpcError> {
    let done = async {
        let _held = slot::try_hold(app)?;
        let remote = Remote::signed_in(app).await?;
        if !stored(&remote)
            .await?
            .profiles
            .iter()
            .any(|p| p.id == profile_id)
        {
            return Err(internal(format!("no profile {profile_id}")));
        }
        remote.set_active(&profile_id).await?;
        save_active(&app.state::<SqlitePool>(), &remote.account_id, &profile_id)
            .await
            .map_err(internal)?;
        push(app).await;
        Ok(())
    }
    .await;
    finish(
        app,
        done,
        MessageKey::NotificationProfileChanged,
        MessageKey::NotificationProfileChangeFailed,
    )
    .await
}

/// `remove_profile(profile_id)`: switches to another Profile first when it
/// is the active one, then removes it on the backend and here, with its
/// Snapshots (F2). The only Profile is never removed.
pub async fn remove<R: Runtime>(app: &AppHandle<R>, profile_id: String) -> Result<(), IpcError> {
    let done = async {
        let _held = slot::try_hold(app)?;
        let remote = Remote::signed_in(app).await?;
        let pool = app.state::<SqlitePool>();
        let current = stored(&remote).await?;
        if !current.profiles.iter().any(|p| p.id == profile_id) {
            return Err(internal(format!("no profile {profile_id}")));
        }
        let Some(other) = current.profiles.iter().find(|p| p.id != profile_id) else {
            return Err(internal("the only profile cannot be removed"));
        };
        if current.active_profile_id.as_ref() == Some(&profile_id) {
            remote.set_active(&other.id).await?;
            save_active(&pool, &remote.account_id, &other.id)
                .await
                .map_err(internal)?;
            push(app).await;
        }
        remote.remove(&profile_id).await?;
        delete(&pool, &remote.account_id, &profile_id)
            .await
            .map_err(internal)?;
        push(app).await;
        Ok(())
    }
    .await;
    finish(
        app,
        done,
        MessageKey::NotificationProfileRemoved,
        MessageKey::NotificationProfileRemoveFailed,
    )
    .await
}

/// Whether `StartSession` found no Profile on the backend while none is
/// stored here, so `profile 1` is due (F2); managed by the app.
#[derive(Default)]
pub struct FirstProfile(AtomicBool);

impl FirstProfile {
    pub fn set_due(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn due(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Created, or handed to the backend part's retry.
    fn clear(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Why `profile 1` could not be created.
enum FirstProfileError {
    /// GGG or poe.ninja; the description says why.
    Data(Message),
    Backend(IpcError),
}

/// Creates `profile 1` (F2) once it is due and the characters are known,
/// under the operation slot: the first two stash tabs of the first league,
/// the first price league. Runs after the backend part of the session start
/// and after the GGG part, whichever comes second does it. A GGG or
/// poe.ninja failure is the GGG part's: a toast and the 45 s retry (F1); a
/// backend failure is the backend part's: a silent retry with the backoff,
/// and `profile 1` is no longer due, so an attempt waiting for the slot
/// stands down until the retry's `StartSession` makes it due again.
pub async fn create_first<R: Runtime>(app: &AppHandle<R>) {
    let first = app.state::<FirstProfile>();
    if !first.due() || app.state::<Characters>().leagues().is_empty() {
        return;
    }
    let _held = slot::hold(app).await;
    if !first.due() || !session_start::signed_in(app) {
        return;
    }
    match create_first_held(app).await {
        Ok(()) => first.clear(),
        Err(FirstProfileError::Data(description)) => {
            if !session_start::signed_in(app) {
                return;
            }
            log::warn!(
                "{FIRST_PROFILE_NAME} was not created ({}); retrying in 45 s",
                description.key.id()
            );
            state::push(
                app,
                NotificationKind::Error,
                MessageKey::NotificationSessionStartFailed.into(),
                Some(description),
            );
            app.state::<Scheduler>()
                .set_in(Deadline::GggSessionRetry, session_start::GGG_RETRY_MS);
        }
        Err(FirstProfileError::Backend(error)) => {
            first.clear();
            session_start::backend_failed(app, error).await;
        }
    }
}

async fn create_first_held<R: Runtime>(app: &AppHandle<R>) -> Result<(), FirstProfileError> {
    let remote = Remote::signed_in(app)
        .await
        .map_err(FirstProfileError::Backend)?;
    let data = |error: IpcError| FirstProfileError::Data(error.message());
    if !stored(&remote).await.map_err(data)?.profiles.is_empty() {
        return Ok(());
    }
    let Some(league) = app.state::<Characters>().leagues().into_iter().next() else {
        return Ok(());
    };
    let tabs = stash_tabs::load(app, &league)
        .await
        .map_err(|error| data(error.into()))?;
    let price_league = match price_leagues(app).await {
        Ok(leagues) => leagues.into_iter().next(),
        Err(error) => {
            log::warn!("the price leagues did not load: {error}");
            None
        }
    };
    let Some(price_league_id) = price_league else {
        return Err(FirstProfileError::Data(
            MessageKey::ErrorPriceLeagues.into(),
        ));
    };
    let profile = Profile {
        id: Uuid::now_v7().to_string(),
        name: FIRST_PROFILE_NAME.to_owned(),
        league_id: league,
        price_league_id,
        stash_tab_ids: picker_tabs(tabs)
            .into_iter()
            .take(2)
            .map(|t| t.id)
            .collect(),
        character_name: None,
        include_equipment: false,
        include_inventory: false,
    };
    let pool = app.state::<SqlitePool>();
    remote
        .create(&profile)
        .await
        .map_err(FirstProfileError::Backend)?;
    save(&pool, &remote.account_id, &profile)
        .await
        .map_err(|error| data(internal(error)))?;
    remote
        .set_active(&profile.id)
        .await
        .map_err(FirstProfileError::Backend)?;
    save_active(&pool, &remote.account_id, &profile.id)
        .await
        .map_err(|error| data(internal(error)))?;
    push(app).await;
    log::info!("created {FIRST_PROFILE_NAME}");
    state::push(
        app,
        NotificationKind::Success,
        MessageKey::NotificationProfileCreated.into(),
        None,
    );
    Ok(())
}

/// Makes the first stored Profile the active one, on the backend first and
/// under the operation slot, when `StartSession` found Profiles but no
/// active one: `profile 1`'s `SetActiveProfile` failed after its
/// `CreateProfile` (F2). Nothing when a Profile is active by then. Fails
/// with the backend's error, for the backend part to retry; a local
/// failure is only logged.
pub async fn activate_first<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let _held = slot::hold(app).await;
    if !session_start::signed_in(app) {
        return Ok(());
    }
    let remote = Remote::signed_in(app).await?;
    let pool = app.state::<SqlitePool>();
    let first = match load(&pool, &remote.account_id).await {
        Ok(stored) if stored.active_profile_id.is_none() => stored.profiles.into_iter().next(),
        Ok(_) => None,
        Err(error) => {
            log::error!("cannot load the profiles: {error}");
            None
        }
    };
    let Some(first) = first else {
        return Ok(());
    };
    remote.set_active(&first.id).await?;
    if let Err(error) = save_active(&pool, &remote.account_id, &first.id).await {
        log::error!("cannot store the active profile: {error}");
        return Ok(());
    }
    log::info!("activated profile {}, as none was active", first.id);
    push(app).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(league: &str) -> Profile {
        Profile {
            id: league.to_lowercase(),
            name: league.to_owned(),
            league_id: league.to_owned(),
            price_league_id: league.to_owned(),
            stash_tab_ids: Vec::new(),
            character_name: Some("Hero".to_owned()),
            include_equipment: true,
            include_inventory: false,
        }
    }

    #[test]
    fn f2_a_league_without_characters_falls_back_to_standard() {
        let mut profiles = vec![profile("Settlers"), profile("Necropolis")];
        let leagues = ["Settlers".to_owned(), "Hardcore".to_owned()];

        fall_back(&mut profiles, &leagues);

        assert_eq!(profiles[0], profile("Settlers"));
        assert_eq!(
            profiles[1],
            Profile {
                league_id: STANDARD.to_owned(),
                ..profile("Necropolis")
            },
            "only the league changes"
        );
        let mut unknown = vec![profile("Necropolis")];
        fall_back(&mut unknown, &[]);
        assert_eq!(unknown, [profile("Necropolis")], "characters not known yet");
    }

    fn tab(id: &str, children: Vec<ggg::StashTab>) -> ggg::StashTab {
        ggg::StashTab {
            id: id.to_owned(),
            name: id.to_uppercase(),
            index: 0,
            metadata: ggg::StashTabMetadata {
                colour: Some("ff0000".to_owned()),
            },
            children,
        }
    }

    #[test]
    fn f2_f3_folder_tabs_are_replaced_by_their_children_in_the_picker() {
        let tabs = vec![
            tab("a", Vec::new()),
            tab("folder", vec![tab("b", Vec::new()), tab("c", Vec::new())]),
            tab("d", Vec::new()),
        ];
        let ids: Vec<String> = picker_tabs(tabs).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["a", "b", "c", "d"]);
    }
}
