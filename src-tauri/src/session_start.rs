//! The session start (client-background-work.md, session start): at launch
//! with a stored Login and right after sign-in. After the token check (D6)
//! the GGG part and the backend part run side by side, neither waiting for
//! the other, except that the GGG part of an account with no local Profile
//! yet waits until its Profiles are known ([`ProfilesKnown`]).
//!
//! - GGG part: loads the account's characters and their leagues through the
//!   GGG queue (F7). A failure shows a toast and retries after 45 s, unless
//!   the user was sent to login (F1). Without a local Profile it then waits
//!   for the `StartSession` answer and, while the legacy import is offered,
//!   for the user's answer.
//! - Backend part, silent outside a Group (D25, D26): `StartSession`, then
//!   its answer applied under the operation slot, then the pending Snapshot
//!   removals sent again (D5); an account in a Group then opens its Group
//!   stream as on a first join (F13). A failed `StartSession` retries with the
//!   backoff, without indicator or toast; meanwhile the app runs on its
//!   local Profile list.
//!
//! When `StartSession` finds no Profile, `profile 1` is created once both
//! parts are done (F2, [`profiles::create_first`]); when the legacy import
//! is offered, only after the user skips it (F21). When it finds Profiles
//! but no active one (`profile 1`'s activation failed), the backend part
//! makes the first one active ([`profiles::activate_first`]).
//!
//! After the characters, the GGG part loads the stash tab list of the
//! active Profile's league (F3), then prices of its price league (F5); a
//! price failure keeps the session going and the minute check retries it.
//! Then the session is ready (`session.ready`, F3).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};

use ipc::slices::{LoginState, NotificationKind, Session};
use ipc::{IpcError, Message, MessageKey};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::watch;
use wire::v1::{StartSessionRequest, StartSessionResponse};

use crate::auth::{self, TokenCheck};
use crate::backend::{self, Backend};
use crate::ggg::{Character, Ggg};
use crate::prices::{self, CHECK_MS};
use crate::profiles::{self, FirstProfile};
use crate::scheduler::{Deadline, Scheduler};
use crate::slot;
use crate::snapshot_removal;
use crate::stash_tabs;
use crate::state::{self, Store};
use crate::{auto_snapshot, group, internal, legacy_import};

/// The wait before the GGG part runs again after a failure (F1).
pub const GGG_RETRY_MS: i64 = 45_000;

/// The account's characters as GGG last sent them; memory only
/// (`client-schema.sql`).
#[derive(Default)]
pub struct Characters(Mutex<Vec<Character>>);

impl Characters {
    fn lock(&self) -> MutexGuard<'_, Vec<Character>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list(&self) -> Vec<Character> {
        self.lock().clone()
    }

    pub fn set(&self, characters: Vec<Character>) {
        *self.lock() = characters;
    }

    /// The leagues of the characters, each once, in character order.
    pub fn leagues(&self) -> Vec<String> {
        let mut leagues: Vec<String> = Vec::new();
        for league in self.lock().iter().filter_map(|c| c.league.as_ref()) {
            if !leagues.contains(league) {
                leagues.push(league.clone());
            }
        }
        leagues
    }
}

/// The backend part's failed attempts since its last success, managed by
/// the app: n of the backoff.
#[derive(Default)]
pub struct BackendAttempts(AtomicU32);

/// The current session start, counted, and whether it knows the account's
/// Profiles: `StartSession` brought Profiles, or `profile 1` is due, or the
/// legacy import offer was answered (F2, F21); managed by the app. The GGG
/// part of an account with no local Profile waits for this
/// ([`wait_for_profiles`]) before its Profile's GGG data.
#[derive(Default)]
pub struct ProfilesKnown(watch::Sender<Start>);

#[derive(Clone, Copy, Default)]
struct Start {
    id: u64,
    profiles_known: bool,
}

impl ProfilesKnown {
    /// A new session start, or the end of the current one at sign-out: a
    /// GGG part still waiting for the previous one stands down.
    fn next(&self) -> u64 {
        let mut id = 0;
        self.0.send_modify(|start| {
            *start = Start {
                id: start.id + 1,
                profiles_known: false,
            };
            id = start.id;
        });
        id
    }

    fn current(&self) -> u64 {
        self.0.borrow().id
    }

    /// Session start `id` knows the account's Profiles; nothing once a
    /// newer one began.
    fn set(&self, id: u64) {
        self.0.send_if_modified(|start| {
            let set = start.id == id && !start.profiles_known;
            start.profiles_known |= set;
            set
        });
    }

    /// Whether session start `id` knows the account's Profiles, once it
    /// does; `false` once a newer one began.
    async fn wait(&self, id: u64) -> bool {
        let mut started = self.0.subscribe();
        let waited = started
            .wait_for(|start| start.id != id || start.profiles_known)
            .await
            .map(|start| start.id == id);
        // The sender lives as long as the app.
        waited.unwrap_or(false)
    }
}

/// The account's Profiles are known: the waiting GGG part of the current
/// session start goes on ([`ProfilesKnown`]).
pub fn profiles_known<R: Runtime>(app: &AppHandle<R>) {
    let known = app.state::<ProfilesKnown>();
    known.set(known.current());
}

/// Ends the current session start (sign-out, sent to login): its GGG part
/// no longer waits for the account's Profiles.
pub fn end<R: Runtime>(app: &AppHandle<R>) {
    app.state::<ProfilesKnown>().next();
}

/// Starts a session: the token check, then the GGG part in the background
/// and the backend part.
pub async fn run<R: Runtime>(app: &AppHandle<R>) {
    let start = app.state::<ProfilesKnown>().next();
    let scheduler = app.state::<Scheduler>();
    scheduler.clear(Deadline::GggSessionRetry);
    scheduler.clear(Deadline::BackendSessionRetry);
    scheduler.set_in(Deadline::PriceCheck, CHECK_MS);
    set_ready(app, false);
    app.state::<BackendAttempts>().0.store(0, Ordering::SeqCst);
    let token = auth::check_token(app).await;
    if token == TokenCheck::SentToLogin {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move { ggg_part(&handle, token, start).await });
    backend_part(app).await;
}

/// The GGG part again, with its own token check: the 45 s retry.
pub async fn retry_ggg<R: Runtime>(app: &AppHandle<R>) {
    let start = app.state::<ProfilesKnown>().current();
    let token = auth::check_token(app).await;
    if token != TokenCheck::SentToLogin {
        ggg_part(app, token, start).await;
    }
}

/// The GGG part of session start `start`: loads the characters through the
/// GGG queue; without a local Profile, waits until the account's Profiles
/// are known ([`wait_for_profiles`]). A failure shows a toast and retries
/// after [`GGG_RETRY_MS`], unless the user was sent to login meanwhile.
async fn ggg_part<R: Runtime>(app: &AppHandle<R>, token: TokenCheck, start: u64) {
    let loaded = match token {
        TokenCheck::Proceed => app
            .state::<Ggg>()
            .characters(app)
            .await
            .map_err(|error| IpcError::from(error).message()),
        TokenCheck::Expired | TokenCheck::SentToLogin => {
            Err(MessageKey::ErrorGggTokenExpired.into())
        }
    };
    match loaded {
        Ok(characters) => {
            log::info!("loaded {} characters", characters.len());
            app.state::<Characters>().set(characters);
            if !wait_for_profiles(app, start).await {
                return;
            }
            // The leagues are known: the `Standard` fallback applies, and
            // `profile 1` may be due (F2).
            profiles::push(app).await;
            profiles::create_first(app).await;
            if let Some(league) = stash_tabs::active_league(app)
                && let Err(error) = stash_tabs::load(app, &league).await
            {
                return ggg_failed(app, IpcError::from(error).message());
            }
            prices::ensure_active(app).await;
            // Auto-snapshot is due first: whoever sees `session.ready` sees
            // the session start's due time.
            auto_snapshot::session_ready(app);
            set_ready(app, true);
            // The active Profile may have changed meanwhile.
            stash_tabs::follow_active(app);
        }
        Err(description) => ggg_failed(app, description),
    }
}

/// Whether the GGG part of session start `start` goes on to the active
/// Profile's data: at once when the account has a local Profile, otherwise
/// once its Profiles are known ([`ProfilesKnown`]). `false` when a newer
/// session start or a sign-out ended `start` meanwhile.
async fn wait_for_profiles<R: Runtime>(app: &AppHandle<R>, start: u64) -> bool {
    match profiles::any_stored(&app.state::<SqlitePool>()).await {
        Ok(true) => return true,
        Ok(false) => {}
        Err(error) => log::error!("cannot load the profiles: {error}"),
    }
    app.state::<ProfilesKnown>().wait(start).await
}

/// A failed GGG part: a toast and another attempt after [`GGG_RETRY_MS`],
/// unless the user was sent to login meanwhile.
fn ggg_failed<R: Runtime>(app: &AppHandle<R>, description: Message) {
    if !signed_in(app) {
        return;
    }
    log::warn!(
        "the session's GGG data did not load ({}); retrying in 45 s",
        description.key.id()
    );
    state::push(
        app,
        NotificationKind::Error,
        MessageKey::NotificationSessionStartFailed.into(),
        Some(description),
    );
    app.state::<Scheduler>()
        .set_in(Deadline::GggSessionRetry, GGG_RETRY_MS);
}

pub(crate) fn signed_in<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<Store>()
        .read(|s: &Session| s.login == LoginState::SignedIn)
}

/// Pushes whether the session is ready (F3); never while signed out.
fn set_ready<R: Runtime>(app: &AppHandle<R>, ready: bool) {
    let pushed = app.state::<Store>().update::<Session, _>(app, |s| {
        s.ready = ready && s.login == LoginState::SignedIn;
    });
    if let Err(error) = pushed {
        log::error!("cannot push the session: {error}");
    }
}

/// The backend part: `StartSession`, then its answer applied under the
/// operation slot, then the first Profile made active when the answer has
/// Profiles but no active one (F2), then the account's pending Snapshot
/// removals sent again (D5); the Group stream when the account is in a
/// Group (F13); the legacy import offer (F21); `profile 1` when the
/// account has no Profile and no offer waits for an answer (F2). The
/// Profiles are known ([`ProfilesKnown`]) once the answer with Profiles is
/// applied and the first one made active if needed, or `profile 1` is due.
/// A backend failure of `StartSession` or of the activation goes through
/// [`backend_failed`].
pub async fn backend_part<R: Runtime>(app: &AppHandle<R>) {
    let known = app.state::<ProfilesKnown>();
    let start = known.current();
    let response = match start_session(app).await {
        Ok(Some(response)) => response,
        // Signed out meanwhile.
        Ok(None) => return,
        Err(error) => return backend_failed(app, error).await,
    };
    app.state::<BackendAttempts>().0.store(0, Ordering::SeqCst);
    log::info!("session started on the backend");
    let no_profile = response.profiles.is_empty();
    let no_active = response.active_profile_id.is_empty();
    let in_group = response.group_name.is_some();
    if let Err(error) = apply(app, response).await {
        log::error!("cannot apply the session start: {error}");
        // The GGG part goes on with the local Profiles.
        return known.set(start);
    }
    if !no_profile {
        let activated = match no_active {
            true => profiles::activate_first(app).await,
            false => Ok(()),
        };
        // Also when the activation failed: the retry activates one.
        known.set(start);
        if let Err(error) = activated {
            return backend_failed(app, error).await;
        }
    }
    snapshot_removal::send_pending(app).await;
    // A restarted client rejoins as on a first join (F13).
    if in_group && signed_in(app) {
        group::rejoin(app).await;
    }
    let offered = legacy_import::offer_after_session_start(app).await;
    if no_profile && !offered {
        app.state::<FirstProfile>().set_due();
        known.set(start);
        profiles::create_first(app).await;
    }
}

/// A failed backend call of the backend part: `SESSION_EXPIRED` sends the
/// user to login; `SUPERSEDED` leaves the client offline until restarted;
/// any other failure is only logged and the backend part retried after the
/// backoff.
pub(crate) async fn backend_failed<R: Runtime>(app: &AppHandle<R>, error: IpcError) {
    match error {
        IpcError::SessionExpired => auth::send_to_login(app).await,
        IpcError::Superseded => {}
        error => {
            let failed = app
                .state::<BackendAttempts>()
                .0
                .fetch_add(1, Ordering::SeqCst);
            let scheduler = app.state::<Scheduler>();
            let wait = scheduler.backoff(failed);
            log::info!("session start on the backend failed ({error}); retrying in {wait} ms");
            scheduler.set_in(Deadline::BackendSessionRetry, wait);
        }
    }
}

/// `StartSession` with the stored Login's session token; `None` without a
/// Login.
pub(crate) async fn start_session<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Option<StartSessionResponse>, IpcError> {
    let backend = app.state::<Backend>();
    backend.check_superseded()?;
    let session_token: Option<String> = sqlx::query_scalar("SELECT session_token FROM login")
        .fetch_optional(&*app.state::<SqlitePool>())
        .await
        .map_err(internal)?;
    let Some(session_token) = session_token else {
        return Ok(None);
    };
    let request = backend::request(
        "StartSession",
        StartSessionRequest {
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        Some(&session_token),
    );
    match backend.session().start_session(request).await {
        Ok(response) => Ok(Some(response.into_inner())),
        Err(status) => Err(backend::failure(app, &status)),
    }
}

/// Applies a `StartSession` answer under the operation slot: the account's
/// name, and its Profile list and active Profile replace the local ones
/// (`client-schema.sql`). Pushes `session` and `profiles`. Nothing once the
/// user was sent to login meanwhile.
pub(crate) async fn apply<R: Runtime>(
    app: &AppHandle<R>,
    response: StartSessionResponse,
) -> Result<(), sqlx::Error> {
    let Some(account) = response.account else {
        log::warn!("StartSession answered without the account");
        return Ok(());
    };
    let _held = slot::hold(app).await;
    if !signed_in(app) {
        return Ok(());
    }
    let pool = app.state::<SqlitePool>();
    sqlx::query("UPDATE accounts SET name = ?2 WHERE id = ?1")
        .bind(&account.id)
        .bind(&account.name)
        .execute(&*pool)
        .await?;
    profiles::replace(
        &pool,
        &account.id,
        &response.profiles,
        &response.active_profile_id,
    )
    .await?;
    let pushed = app.state::<Store>().update::<Session, _>(app, |s| {
        if s.login == LoginState::SignedIn {
            s.account_name = Some(account.name);
        }
    });
    if let Err(error) = pushed {
        log::error!("cannot push the session start: {error}");
    }
    profiles::push(app).await;
    Ok(())
}
