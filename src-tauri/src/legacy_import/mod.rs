//! Legacy import (F21): Exilence CE's history, custom prices and settings from this device's
//! legacy folders into the signed-in account. Spec:
//! <https://github.com/exilence-ce/migration/blob/main/docs/spec/legacy-import.md>.
//!
//! - Offer: after a successful `StartSession`, when the account has no `legacy_import` row and
//!   the scan finds data. An account without a Profile waits for the answer before `profile 1`
//!   (F2): Import brings the legacy Profiles instead, Skip creates it.
//! - Import, under the operation slot: `CreateProfile` for each ticked legacy Profile the account
//!   lacks, `SetActiveProfile` when it had none, then one local transaction ([`store`]). Any
//!   failure writes nothing locally and shows a toast; the offer stays open. Imported Snapshots
//!   are never uploaded (D4, ADR 0002).

pub mod folders;
pub mod mapping;
pub mod model;
pub mod reader;
mod store;

use ipc::slices::{LegacyImport, NotificationKind, Profile, Settings, Status, Toast};
use ipc::{IpcError, Message, MessageKey};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::profiles::{self, FirstProfile, Remote};
use crate::scheduler::Scheduler;
use crate::state::{self, Store};
use crate::{auth, internal, prices, session_start, settings, shell, slot};
use folders::LegacyFolder;
use mapping::Scan;
use reader::ReadError;

/// The legacy folders the scan reads, managed by the app: this OS's, or a test's.
pub struct LegacyFolders(pub Vec<LegacyFolder>);

/// Why a scan or an import failed.
enum Failure {
    Read(ReadError),
    Command(IpcError),
}

impl From<IpcError> for Failure {
    fn from(error: IpcError) -> Self {
        Self::Command(error)
    }
}

/// Reads every existing legacy folder and merges them, off the async runtime. Fails as a whole
/// while Exilence CE holds one open.
async fn scan<R: Runtime>(app: &AppHandle<R>) -> Result<Scan, ReadError> {
    let folders = app.state::<LegacyFolders>().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut read = Vec::new();
        for folder in folders.iter().filter(|f| f.path.is_dir()) {
            read.push((folder.app, reader::read_folder(&folder.path)?));
        }
        Ok(mapping::scan(read))
    })
    .await
    .map_err(|error| ReadError::Io(std::io::Error::other(error)))?
}

/// The signed-in account's id and name.
async fn signed_in_account(pool: &SqlitePool) -> sqlx::Result<Option<(String, String)>> {
    sqlx::query_as("SELECT a.id, a.name FROM login l JOIN accounts a ON a.id = l.account_id")
        .fetch_optional(pool)
        .await
}

fn update_slice<R: Runtime>(app: &AppHandle<R>, change: impl FnOnce(&mut LegacyImport)) {
    if let Err(error) = app.state::<Store>().update::<LegacyImport, _>(app, change) {
        log::error!("cannot push the legacy import: {error}");
    }
}

/// Closes a pending offer, e.g. at sign-out.
pub fn close_offer<R: Runtime>(app: &AppHandle<R>) {
    update_slice(app, |l| l.offer = None);
}

/// After a successful `StartSession`: offers the import when the account has no
/// `legacy_import` row and the scan finds data. Whether an offer is pending: an account without
/// a Profile then waits for the answer before `profile 1` (F2). A failed scan shows a toast and
/// offers nothing; the offer returns at the next session start.
pub async fn offer_after_session_start<R: Runtime>(app: &AppHandle<R>) -> bool {
    if app
        .state::<Store>()
        .read(|l: &LegacyImport| l.offer.is_some())
    {
        return true;
    }
    let pool = app.state::<SqlitePool>();
    let answered = async {
        let Some((account_id, name)) = signed_in_account(&pool).await? else {
            return Ok(None);
        };
        let row: Option<String> =
            sqlx::query_scalar("SELECT state FROM legacy_import WHERE account_id = ?1")
                .bind(&account_id)
                .fetch_optional(&*pool)
                .await?;
        Ok::<_, sqlx::Error>(Some((name, row.is_some())))
    }
    .await;
    let name = match answered {
        Ok(Some((name, false))) => name,
        Ok(_) => return false,
        Err(error) => {
            log::error!("cannot read the legacy import marker: {error}");
            return false;
        }
    };
    match scan(app).await {
        Ok(scan) if scan.is_empty() => false,
        Ok(scan) => {
            log::info!(
                "offering the legacy import of {} legacy accounts",
                scan.accounts.len()
            );
            let offer = mapping::summary(&scan, &name);
            update_slice(app, |l| l.offer = Some(offer));
            true
        }
        Err(error) => {
            failed(app, Failure::Read(error)).await;
            false
        }
    }
}

/// `scan_legacy_import`: Settings → Advanced → Import from Exilence CE. Scans now and sets the
/// offer, whatever the marker says; an empty scan says no data was found. `Busy` while an
/// import runs.
pub async fn scan_command<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    if app.state::<Store>().read(|l: &LegacyImport| l.running) {
        return Err(failed(app, IpcError::Busy.into()).await);
    }
    let name = match signed_in_account(&app.state::<SqlitePool>()).await {
        Ok(Some((_, name))) => name,
        Ok(None) => return Err(IpcError::SessionExpired),
        Err(error) => return Err(failed(app, internal(error).into()).await),
    };
    match scan(app).await {
        Ok(scan) if scan.is_empty() => {
            log::info!("no legacy data found");
            close_offer(app);
            let toast = Toast {
                kind: NotificationKind::Info,
                message: MessageKey::LegacyImportNoData.into(),
                description: None,
            };
            if let Err(error) = state::toast(app, toast) {
                log::error!("cannot show a toast: {error}");
            }
            Ok(())
        }
        Ok(scan) => {
            let offer = mapping::summary(&scan, &name);
            update_slice(app, |l| l.offer = Some(offer));
            Ok(())
        }
        Err(error) => Err(failed(app, Failure::Read(error)).await),
    }
}

/// `skip_legacy_import`: records `declined` (a finished import's `done` stays) and closes the
/// offer; an account without a Profile then gets `profile 1` (F2).
pub async fn skip<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    if app.state::<Store>().read(|l: &LegacyImport| l.running) {
        return Err(IpcError::Busy);
    }
    let pool = app.state::<SqlitePool>();
    let account_id = match signed_in_account(&pool).await.map_err(internal)? {
        Some((account_id, _)) => account_id,
        None => return Err(IpcError::SessionExpired),
    };
    let at = app.state::<Scheduler>().now();
    sqlx::query(
        "INSERT INTO legacy_import (account_id, state, at) VALUES (?1, 'declined', ?2) \
         ON CONFLICT (account_id) DO NOTHING",
    )
    .bind(&account_id)
    .bind(at)
    .execute(&*pool)
    .await
    .map_err(internal)?;
    log::info!("legacy import skipped");
    close_offer(app);
    first_profile_if_none(app, &account_id).await;
    Ok(())
}

/// `profile 1` for an account the answer left without a Profile (F2). The
/// account's Profiles are known then: a session start's GGG part waiting
/// for the answer goes on ([`session_start::ProfilesKnown`]).
async fn first_profile_if_none<R: Runtime>(app: &AppHandle<R>, account_id: &str) {
    match profiles::load(&app.state::<SqlitePool>(), account_id).await {
        Ok(stored) if stored.profiles.is_empty() => {
            app.state::<FirstProfile>().set_due();
            session_start::profiles_known(app);
            profiles::create_first(app).await;
        }
        Ok(_) => session_start::profiles_known(app),
        Err(error) => {
            log::error!("cannot load the profiles: {error}");
            session_start::profiles_known(app);
        }
    }
}

/// `legacy_import.running` and the status "Importing Exilence CE history" while alive.
struct Running<'a, R: Runtime>(&'a AppHandle<R>);

impl<'a, R: Runtime> Running<'a, R> {
    fn start(app: &'a AppHandle<R>) -> Self {
        update_slice(app, |l| l.running = true);
        set_status(app, Some(MessageKey::StatusImportingLegacy.into()));
        Self(app)
    }
}

impl<R: Runtime> Drop for Running<'_, R> {
    fn drop(&mut self) {
        set_status(self.0, None);
        update_slice(self.0, |l| l.running = false);
    }
}

fn set_status<R: Runtime>(app: &AppHandle<R>, message: Option<Message>) {
    if let Err(error) = app
        .state::<Store>()
        .update::<Status, _>(app, |s| s.message = message)
    {
        log::error!("cannot push the status: {error}");
    }
}

/// `run_legacy_import(account_names)`: imports the ticked legacy accounts into the signed-in
/// account under the operation slot. Success notes "Imported N Profiles and M Snapshots" in the
/// list and closes the offer; a failure shows a toast and keeps it open.
pub async fn run<R: Runtime>(
    app: &AppHandle<R>,
    account_names: Vec<String>,
) -> Result<(), IpcError> {
    let imported = async {
        let _held = slot::try_hold(app)?;
        let _running = Running::start(app);
        import(app, &account_names).await
    }
    .await;
    let (account_id, imported) = match imported {
        Ok(imported) => imported,
        Err(failure) => return Err(failed(app, failure).await),
    };
    log::info!(
        "legacy import done: {} profiles created, {} snapshots added",
        imported.profiles,
        imported.snapshots
    );
    close_offer(app);
    let message = Message::new(MessageKey::NotificationLegacyImportSucceeded)
        .arg("profiles", imported.profiles)
        .arg("snapshots", imported.snapshots);
    state::push(app, NotificationKind::Success, message, None);
    first_profile_if_none(app, &account_id).await;
    Ok(())
}

/// Steps 1–4 of the import (legacy-import.md, order and failure); the caller holds the slot.
async fn import<R: Runtime>(
    app: &AppHandle<R>,
    account_names: &[String],
) -> Result<(String, store::Imported), Failure> {
    let remote = Remote::signed_in(app).await?;
    let pool = app.state::<SqlitePool>();
    let scan = scan(app).await.map_err(Failure::Read)?;
    let selection = mapping::select(&scan, account_names);
    let existing = profiles::load(&pool, &remote.account_id)
        .await
        .map_err(internal)?;
    let mut position: i64 = sqlx::query_scalar(
        "SELECT coalesce(max(position) + 1, 0) FROM profiles WHERE account_id = ?1",
    )
    .bind(&remote.account_id)
    .fetch_one(&*pool)
    .await
    .map_err(internal)?;
    let mut new_profiles = Vec::new();
    for legacy in &selection.profiles {
        if existing.profiles.iter().any(|p| p.id == legacy.id) {
            continue;
        }
        new_profiles.push((position, profile(legacy, position)));
        position += 1;
    }
    for (_, profile) in &new_profiles {
        remote.create(profile).await?;
    }
    let active = if existing.profiles.is_empty() {
        selection
            .active_profile_id
            .clone()
            .filter(|id| selection.profiles.iter().any(|p| &p.id == id))
            .or_else(|| selection.profiles.first().map(|p| p.id.clone()))
    } else {
        None
    };
    if let Some(active) = &active {
        remote.set_active(active).await?;
    }
    let at = app.state::<Scheduler>().now();
    let write = store::Write {
        account_id: &remote.account_id,
        scan: &scan,
        selection: &selection,
        new_profiles: &new_profiles,
        active_profile_id: active.as_deref(),
        at,
    };
    let imported = store::write(&pool, &write).await.map_err(internal)?;
    push(app, &pool).await;
    Ok((remote.account_id.clone(), imported))
}

/// The NG Profile of `legacy` at `position`; a missing league becomes `Standard`, which F2's
/// fallback would choose too.
fn profile(legacy: &mapping::Profile, position: i64) -> Profile {
    let or_standard = |league: &Option<String>| {
        league
            .clone()
            .unwrap_or_else(|| profiles::STANDARD.to_owned())
    };
    Profile {
        id: legacy.id.clone(),
        name: mapping::profile_name(&legacy.name, position),
        league_id: or_standard(&legacy.league_id),
        price_league_id: or_standard(&legacy.price_league_id),
        stash_tab_ids: legacy.stash_tab_ids.clone(),
        character_name: legacy.character_name.clone(),
        include_equipment: legacy.include_equipment,
        include_inventory: legacy.include_inventory,
    }
}

/// Pushes what the import changed: Profiles with their net worth, settings, and prices whose
/// rates custom prices change (F9).
async fn push<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool) {
    profiles::push(app).await;
    match settings::load(pool).await {
        Ok(loaded) => {
            if let Err(error) = app
                .state::<Store>()
                .update::<Settings, _>(app, |s| *s = loaded)
            {
                log::error!("cannot push the settings: {error}");
            }
            shell::apply_ui_scale(app);
        }
        Err(error) => log::error!("cannot load the settings: {error}"),
    }
    prices::push(app).await;
}

/// Tells the user why a scan or an import failed: a toast saying why, except an expired session,
/// which sends the user to login with its own toast (F1).
async fn failed<R: Runtime>(app: &AppHandle<R>, failure: Failure) -> IpcError {
    let (error, description) = match failure {
        Failure::Read(ReadError::Locked) => {
            log::info!("legacy import: Exilence CE is running");
            (
                internal("Exilence CE holds its data open"),
                MessageKey::ErrorLegacyRunning.into(),
            )
        }
        Failure::Read(error) => (
            internal(format!("legacy data unreadable: {error}")),
            MessageKey::ErrorLegacyUnreadable.into(),
        ),
        Failure::Command(error) => {
            log::warn!("legacy import failed: {error}");
            let description = error.message();
            (error, description)
        }
    };
    if error == IpcError::SessionExpired {
        auth::send_to_login(app).await;
        return error;
    }
    state::push(
        app,
        NotificationKind::Error,
        MessageKey::NotificationLegacyImportFailed.into(),
        Some(description),
    );
    error
}
