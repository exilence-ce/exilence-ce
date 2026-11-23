//! Everything about GGG tokens and the login (ADR 0001): the OAuth client id,
//! scopes and redirect; each login attempt's PKCE S256 verifier and `ng-`
//! state (D18, D20); the `exilence-ce-ng://` deep link; the backend's
//! `SignIn`, `RefreshGggToken` (D6) and `SignOut` (D19); and the stored
//! Login with its GGG access token. The GGG refresh token never reaches the
//! client.

use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use ipc::command::SettingsPatch;
use ipc::slices::{LoginState, NotificationKind, Platform, Session, Settings};
use ipc::{BackendReason, IpcError, Message, MessageKey};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime, Url};
use tokio::sync::watch;
use wire::v1::{Account, RefreshGggTokenRequest, SignInRequest, SignOutRequest};

use crate::backend::{self, Backend};
use crate::desktop::Desktop;
use crate::scheduler::{Deadline, Scheduler};
use crate::state::{self, Store};
use crate::{auto_snapshot, group, internal, logging, session_start, settings, shell, slot};

/// The confidential OAuth client (ADR 0001).
pub const CLIENT_ID: &str = "exilencece";
/// The scopes of every login (F1).
pub const SCOPES: &str = "account:stashes account:profile account:characters";
/// GGG's registered redirect: the backend, which forwards to the deep link.
pub const REDIRECT_URI: &str = "https://api.exilence.de/api/authentication/redirect";
/// GGG's consent page.
pub const AUTHORIZE_URL: &str = "https://www.pathofexile.com/oauth/authorize";
/// The deep link scheme the backend forwards the authorization to (D20).
pub const DEEP_LINK_SCHEME: &str = "exilence-ce-ng";
/// Every `state` starts with it, so HAProxy sends the redirect to the NG
/// backend (D20).
pub const STATE_PREFIX: &str = "ng-";
/// The contact GGG's User-Agent rule asks for; the one legacy sends.
pub const CONTACT: &str = "exilencece@gmail.com";

/// The User-Agent of every GGG request, in GGG's required format.
pub fn user_agent() -> String {
    format!(
        "OAuth {CLIENT_ID}/{} (contact: {CONTACT}) StrictMode",
        env!("CARGO_PKG_VERSION")
    )
}

/// One login attempt: what `start_login` or `copy_login_link` handed to the
/// browser, and the verifier only this app knows.
struct Attempt {
    state: String,
    verifier: String,
    url: String,
}

impl Attempt {
    /// A fresh attempt: a new PKCE verifier and a new `state`.
    #[expect(
        clippy::expect_used,
        reason = "AUTHORIZE_URL is a constant, parsed by every login test"
    )]
    fn new() -> Self {
        let verifier = random_token::<32>();
        let state = format!("{STATE_PREFIX}{}", random_token::<16>());
        let challenge = BASE64URL.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = Url::parse(AUTHORIZE_URL).expect("a valid URL");
        url.query_pairs_mut()
            .append_pair("client_id", CLIENT_ID)
            .append_pair("response_type", "code")
            .append_pair("scope", SCOPES)
            .append_pair("state", &state)
            .append_pair("redirect_uri", REDIRECT_URI)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        logging::secret(&state);
        logging::secret(&verifier);
        Self {
            state,
            verifier,
            url: url.into(),
        }
    }
}

/// `N` random bytes, base64url without padding.
#[expect(
    clippy::expect_used,
    reason = "without OS randomness no secure token exists"
)]
fn random_token<const N: usize>() -> String {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("the OS has randomness");
    BASE64URL.encode(bytes)
}

/// A running refresh's result, once known: `Err(SessionExpired)` when it
/// sent the user to login.
type Refreshed = Option<Result<(), IpcError>>;

/// The login state the core holds, managed by the app: the pending attempt,
/// the running token refresh and the OS integrations a login uses.
pub struct Auth {
    attempt: Mutex<Option<Attempt>>,
    /// One refresh at a time: a trigger that finds one running waits for its
    /// result (D6).
    refresh: Mutex<Option<watch::Receiver<Refreshed>>>,
    desktop: Arc<dyn Desktop>,
}

impl Auth {
    pub fn new(desktop: Arc<dyn Desktop>) -> Self {
        Self {
            attempt: Mutex::new(None),
            refresh: Mutex::new(None),
            desktop,
        }
    }

    fn attempt(&self) -> std::sync::MutexGuard<'_, Option<Attempt>> {
        self.attempt.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The URL of a fresh attempt, which replaces any pending one.
    fn fresh_url(&self) -> String {
        let attempt = Attempt::new();
        let url = attempt.url.clone();
        *self.attempt() = Some(attempt);
        url
    }

    /// The URL of the pending attempt, or of a fresh one.
    fn pending_url(&self) -> String {
        self.attempt().get_or_insert_with(Attempt::new).url.clone()
    }

    /// The pending attempt if `state` is its own; it is then used up. A
    /// mismatch leaves it pending: a forged link must not cancel it.
    fn take_matching(&self, state: &str) -> Option<Attempt> {
        let mut attempt = self.attempt();
        attempt
            .as_ref()
            .is_some_and(|a| a.state == state)
            .then(|| attempt.take())
            .flatten()
    }
}

/// `start_login(platform)`: remembers the platform, then opens a fresh
/// attempt's consent page in the system browser (F1). If the browser does
/// not open, a toast points to the copy-link control.
pub async fn start_login<R: Runtime>(
    app: &AppHandle<R>,
    platform: Platform,
) -> Result<(), IpcError> {
    let patch = SettingsPatch {
        platform: Some(platform),
        ..SettingsPatch::default()
    };
    let entries = settings::save(&app.state::<SqlitePool>(), &patch).await?;
    let store = app.state::<Store>();
    let mut merged = Ok(());
    store
        .update::<Settings, _>(app, |s| merged = settings::merge(s, entries))
        .map_err(internal)?;
    merged.map_err(internal)?;
    store
        .update::<Session, _>(app, |s| s.platform = platform)
        .map_err(internal)?;

    let auth = app.state::<Auth>();
    let url = auth.fresh_url();
    log::info!("login started, opening the consent page");
    if let Err(error) = auth.desktop.open_url(&url) {
        log::warn!("the browser did not open: {error}");
        state::push(
            app,
            NotificationKind::Error,
            MessageKey::ErrorBrowserNotOpened.into(),
            None,
        );
    }
    Ok(())
}

/// `copy_login_link`: puts the pending attempt's authorization URL on the
/// clipboard (F1).
pub fn copy_login_link<R: Runtime>(app: &AppHandle<R>) {
    let auth = app.state::<Auth>();
    let url = auth.pending_url();
    match auth.desktop.copy_text(&url) {
        Ok(()) => log::info!("login link copied"),
        Err(error) => {
            log::warn!("the login link could not be copied: {error}");
            state::push(
                app,
                NotificationKind::Error,
                MessageKey::ErrorClipboard.into(),
                None,
            );
        }
    }
}

/// What the backend's redirect forwarded: `code` and `state`, or GGG's
/// `error` when the user refused (D20).
struct Callback {
    state: String,
    code: Option<String>,
}

impl Callback {
    fn parse(link: &str) -> Option<Self> {
        let url = Url::parse(link).ok()?;
        if url.scheme() != DEEP_LINK_SCHEME {
            return None;
        }
        let mut callback = Self {
            state: String::new(),
            code: None,
        };
        for (name, value) in url.query_pairs() {
            match &*name {
                "state" => callback.state = value.into_owned(),
                "code" => callback.code = Some(value.into_owned()),
                _ => {}
            }
        }
        logging::secret(&callback.state);
        if let Some(code) = &callback.code {
            logging::secret(code);
        }
        Some(callback)
    }
}

/// A deep link reached the app (F1): brings the main window to the front and,
/// if its `state` is the pending attempt's, signs in with its code in the
/// background. Any other link is rejected (D18).
pub fn open_deep_link<R: Runtime>(app: &AppHandle<R>, link: &str) {
    shell::show_main_window(app);
    let Some(callback) = Callback::parse(link) else {
        log::warn!("ignored a link that is not an {DEEP_LINK_SCHEME}:// link");
        return;
    };
    let Some(attempt) = app.state::<Auth>().take_matching(&callback.state) else {
        log::warn!("login deep link rejected: its state is not the pending attempt's (D18)");
        failed(app, MessageKey::ErrorLoginRejected.into());
        return;
    };
    let Some(code) = callback.code else {
        log::info!("the authorization was refused on pathofexile.com");
        failed(app, MessageKey::ErrorLoginRefused.into());
        return;
    };
    set_login_state(app, LoginState::SigningIn);
    let app = app.clone();
    tauri::async_runtime::spawn(async move { sign_in(&app, code, attempt.verifier).await });
}

async fn sign_in<R: Runtime>(app: &AppHandle<R>, code: String, verifier: String) {
    match exchange(app, code, verifier).await {
        Ok(account_name) => {
            log::info!("signed in as {account_name}");
            let signed_in = app.state::<Store>().update::<Session, _>(app, |s| {
                s.login = LoginState::SignedIn;
                s.account_name = Some(account_name);
            });
            if let Err(error) = signed_in {
                log::error!("cannot push the session: {error}");
            }
            state::push(
                app,
                NotificationKind::Success,
                MessageKey::NotificationLoginSucceeded.into(),
                None,
            );
            session_start::run(app).await;
        }
        Err(error) => {
            log::warn!("sign-in failed: {error}");
            failed(app, error.message());
        }
    }
}

/// `SignIn` with the code and verifier, then stores the Login. Returns the
/// account name.
async fn exchange<R: Runtime>(
    app: &AppHandle<R>,
    code: String,
    code_verifier: String,
) -> Result<String, IpcError> {
    let request = backend::request(
        "SignIn",
        SignInRequest {
            code,
            code_verifier,
        },
        None,
    );
    let response = app
        .state::<Backend>()
        .auth()
        .sign_in(request)
        .await
        .map_err(|status| backend::ipc_error(&status))?
        .into_inner();
    logging::secret(&response.session_token);
    let (Some(account), Some(token)) = (response.account, response.ggg_token) else {
        return Err(internal("SignIn answered without account or GGG token"));
    };
    logging::secret(&token.access_token);
    let expires_at = token
        .expires_at
        .as_ref()
        .map(backend::unix_ms)
        .ok_or_else(|| internal("SignIn answered without the token's expiry"))?;
    store_login(
        &app.state::<SqlitePool>(),
        &account,
        &response.session_token,
        &token.access_token,
        expires_at,
    )
    .await
    .map_err(internal)?;
    Ok(account.name)
}

/// Before a session starts or a Snapshot runs, the GGG access token is
/// refreshed when it expires within this many ms (D6).
pub const REFRESH_WITHIN_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// What the token check (D6) leaves the action to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenCheck {
    /// Go on with the stored token: fresh, refreshed, or kept because it is
    /// still valid although the refresh failed.
    Proceed,
    /// The token has expired and cannot be refreshed now: the action fails
    /// with a toast; the user stays signed in.
    Expired,
    /// The user was sent to login, or is signed out.
    SentToLogin,
}

/// The token check before a session starts or a Snapshot runs (D6): has the
/// backend refresh the GGG access token when it expires within
/// [`REFRESH_WITHIN_MS`].
pub async fn check_token<R: Runtime>(app: &AppHandle<R>) -> TokenCheck {
    let expires_at: Option<i64> = match sqlx::query_scalar("SELECT ggg_expires_at FROM login")
        .fetch_optional(&*app.state::<SqlitePool>())
        .await
    {
        Ok(expires_at) => expires_at,
        Err(error) => {
            // The action's own read of the token reports the failure.
            log::error!("cannot read the Login: {error}");
            return TokenCheck::Proceed;
        }
    };
    let Some(expires_at) = expires_at else {
        return TokenCheck::SentToLogin;
    };
    let now = app.state::<Scheduler>().now();
    if expires_at - now > REFRESH_WITHIN_MS {
        return TokenCheck::Proceed;
    }
    match refresh_ggg_token(app).await {
        Ok(()) => TokenCheck::Proceed,
        Err(IpcError::SessionExpired) => TokenCheck::SentToLogin,
        Err(error) if expires_at > now => {
            log::warn!("keeping the GGG token, valid until {expires_at}: {error}");
            TokenCheck::Proceed
        }
        Err(error) => {
            log::warn!("the GGG token expired at {expires_at} and cannot be refreshed: {error}");
            TokenCheck::Expired
        }
    }
}

/// `RefreshGggToken` (D6), one at a time: a trigger that finds a refresh
/// running waits for its result. `GGG_REFRESH_FAILED` and `SESSION_EXPIRED`
/// send the user to login and give `Err(SessionExpired)`; any other failure
/// keeps the token.
pub async fn refresh_ggg_token<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let auth = app.state::<Auth>();
    let (sender, mut receiver) = {
        let mut running = auth.refresh.lock().unwrap_or_else(|e| e.into_inner());
        match &*running {
            // A refresh whose task was dropped has no sender left.
            Some(receiver) if receiver.has_changed().is_ok() => (None, receiver.clone()),
            _ => {
                let (sender, receiver) = watch::channel(None);
                *running = Some(receiver.clone());
                (Some(sender), receiver)
            }
        }
    };
    let Some(sender) = sender else {
        return match receiver.wait_for(Option::is_some).await {
            Ok(result) => (*result).clone().unwrap_or_else(|| {
                Err(internal("the running token refresh ended without a result"))
            }),
            Err(_) => Err(internal("the running token refresh ended without a result")),
        };
    };
    let result = refresh(app).await;
    *auth.refresh.lock().unwrap_or_else(|e| e.into_inner()) = None;
    sender.send_replace(Some(result.clone()));
    result
}

async fn refresh<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let backend = app.state::<Backend>();
    backend.check_superseded()?;
    let pool = app.state::<SqlitePool>();
    let session_token: Option<String> = sqlx::query_scalar("SELECT session_token FROM login")
        .fetch_optional(&*pool)
        .await
        .map_err(internal)?;
    let session_token = session_token.ok_or(IpcError::SessionExpired)?;
    let request = backend::request(
        "RefreshGggToken",
        RefreshGggTokenRequest {},
        Some(&session_token),
    );
    let response = match backend.auth().refresh_ggg_token(request).await {
        Ok(response) => response.into_inner(),
        Err(status) => {
            return Err(match backend::failure(app, &status) {
                IpcError::SessionExpired
                | IpcError::Backend {
                    reason: BackendReason::GggRefreshFailed,
                } => {
                    log::warn!("the GGG token refresh was refused");
                    send_to_login(app).await;
                    IpcError::SessionExpired
                }
                error => {
                    log::warn!("the GGG token refresh failed: {error}");
                    error
                }
            });
        }
    };
    let token = response
        .ggg_token
        .ok_or_else(|| internal("RefreshGggToken answered without a token"))?;
    logging::secret(&token.access_token);
    let expires_at = token
        .expires_at
        .as_ref()
        .map(backend::unix_ms)
        .ok_or_else(|| internal("RefreshGggToken answered without the token's expiry"))?;
    // Only this Login's row: the user may have signed in again meanwhile.
    sqlx::query(
        "UPDATE login SET ggg_access_token = ?2, ggg_expires_at = ?3 WHERE session_token = ?1",
    )
    .bind(&session_token)
    .bind(&token.access_token)
    .bind(expires_at)
    .execute(&*pool)
    .await
    .map_err(internal)?;
    log::info!("GGG token refreshed, valid until {expires_at}");
    Ok(())
}

/// Tells the user why the action `title` failed: nothing for a field error,
/// login for an expired session, else a notification with a toast (F18).
/// Returns `error`.
pub(crate) async fn action_failed<R: Runtime>(
    app: &AppHandle<R>,
    title: MessageKey,
    error: IpcError,
) -> IpcError {
    log::warn!("{}: {error}", title.id());
    match &error {
        IpcError::Validation { .. } => {}
        IpcError::SessionExpired => send_to_login(app).await,
        _ => state::push(
            app,
            NotificationKind::Error,
            title.into(),
            Some(error.message()),
        ),
    }
    error
}

/// The Login cannot be renewed (D6) or the backend no longer knows it
/// (`SESSION_EXPIRED`): forgets it and returns to login with "Session
/// expired, please login again" (F1). The session start's retries stop.
pub async fn send_to_login<R: Runtime>(app: &AppHandle<R>) {
    let store = app.state::<Store>();
    if store.read(|s: &Session| s.login != LoginState::SignedIn) {
        return;
    }
    log::warn!("session expired: back to login");
    stop_background_work(app);
    if let Err(error) = sqlx::query("DELETE FROM login")
        .execute(&*app.state::<SqlitePool>())
        .await
    {
        log::error!("cannot delete the Login: {error}");
    }
    signed_out(app);
    state::push(
        app,
        NotificationKind::Error,
        MessageKey::ErrorSessionExpired.into(),
        None,
    );
}

/// `sign_out` (F1, client-background-work.md, sign-out): refused with `Busy`
/// while the operation slot is taken. Stops the account's background work,
/// has the backend delete this Login and revoke its GGG tokens (best
/// effort, D19), forgets the Login and returns to login with a notification.
/// The account's local history stays. A local failure keeps the user signed
/// in, with an error toast.
pub async fn sign_out<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let _held = slot::try_hold(app)?;
    log::info!("signing out");
    stop_background_work(app);
    let forgotten = forget_login(app).await;
    if let Err(error) = &forgotten {
        log::error!("sign-out failed: {error}");
        state::push(
            app,
            NotificationKind::Error,
            MessageKey::NotificationSignOutFailed.into(),
            Some(error.message()),
        );
    }
    forgotten?;
    signed_out(app);
    state::push(
        app,
        NotificationKind::Success,
        MessageKey::NotificationSignedOut.into(),
        None,
    );
    Ok(())
}

/// `SignOut` with the stored Login's session token (best effort, D19), then
/// deletes the Login.
async fn forget_login<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    sign_out_at_backend(app).await?;
    let pool = app.state::<SqlitePool>();
    sqlx::query("DELETE FROM login")
        .execute(&*pool)
        .await
        .map_err(internal)?;
    Ok(())
}

/// `SignOut` with the stored Login's session token, if any (best effort,
/// D19): a failing `SignOut` is logged; only reading the Login can fail.
pub async fn sign_out_at_backend<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let session_token: Option<String> = sqlx::query_scalar("SELECT session_token FROM login")
        .fetch_optional(&*app.state::<SqlitePool>())
        .await
        .map_err(internal)?;
    if let Some(session_token) = session_token
        && let Err(error) = revoke(app, &session_token).await
    {
        log::warn!("SignOut failed, signing out locally anyway (D19): {error}");
    }
    Ok(())
}

/// The backend's `SignOut`: deletes this Login and revokes its GGG tokens.
async fn revoke<R: Runtime>(app: &AppHandle<R>, session_token: &str) -> Result<(), IpcError> {
    let backend = app.state::<Backend>();
    backend.check_superseded()?;
    let request = backend::request("SignOut", SignOutRequest {}, Some(session_token));
    match backend.auth().sign_out(request).await {
        Ok(_) => Ok(()),
        Err(status) => Err(backend::failure(app, &status)),
    }
}

/// Stops what runs for the signed-in account: the session start's retries
/// and a GGG part waiting for the account's Profiles, the auto-snapshot due
/// time and the Group stream (the membership stays, D22). The Group upload
/// queue (D27) clears here too.
pub fn stop_background_work<R: Runtime>(app: &AppHandle<R>) {
    session_start::end(app);
    let scheduler = app.state::<Scheduler>();
    scheduler.clear(Deadline::GggSessionRetry);
    scheduler.clear(Deadline::BackendSessionRetry);
    scheduler.clear(Deadline::PriceCheck);
    auto_snapshot::clear(app);
    group::close_now(app);
}

/// Pushes the login screen; the signed-out account's legacy import offer
/// closes (F21).
fn signed_out<R: Runtime>(app: &AppHandle<R>) {
    let pushed = app.state::<Store>().update::<Session, _>(app, |s| {
        s.login = LoginState::SignedOut;
        s.account_name = None;
        s.ready = false;
        s.league_has_tabs = false;
    });
    if let Err(error) = pushed {
        log::error!("cannot push the session: {error}");
    }
    crate::legacy_import::close_offer(app);
}

/// Stores the Login of `account`, replacing any other (`client-schema.sql`:
/// at most one row).
async fn store_login(
    pool: &SqlitePool,
    account: &Account,
    session_token: &str,
    ggg_access_token: &str,
    ggg_expires_at: i64,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO accounts (id, name) VALUES (?1, ?2) \
         ON CONFLICT (id) DO UPDATE SET name = excluded.name",
    )
    .bind(&account.id)
    .bind(&account.name)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO login (id, account_id, session_token, ggg_access_token, ggg_expires_at) \
         VALUES (1, ?1, ?2, ?3, ?4) \
         ON CONFLICT (id) DO UPDATE SET account_id = excluded.account_id, \
         session_token = excluded.session_token, ggg_access_token = excluded.ggg_access_token, \
         ggg_expires_at = excluded.ggg_expires_at",
    )
    .bind(&account.id)
    .bind(session_token)
    .bind(ggg_access_token)
    .bind(ggg_expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// The account of the stored Login, if any; its tokens are redacted from the
/// log from now on.
pub async fn restore(pool: &SqlitePool) -> sqlx::Result<Option<Account>> {
    let login: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT a.id, a.name, l.session_token, l.ggg_access_token \
         FROM login l JOIN accounts a ON a.id = l.account_id",
    )
    .fetch_optional(pool)
    .await?;
    Ok(login.map(|(id, name, session_token, access_token)| {
        logging::secret(&session_token);
        logging::secret(&access_token);
        Account { id, name }
    }))
}

/// A rejected or failed authorization: back to the login screen, unless
/// already signed in, with an error toast (F1).
fn failed<R: Runtime>(app: &AppHandle<R>, description: Message) {
    let pushed = app.state::<Store>().update::<Session, _>(app, |s| {
        if s.login != LoginState::SignedIn {
            s.login = LoginState::SignedOut;
        }
    });
    if let Err(error) = pushed {
        log::error!("cannot push the session: {error}");
    }
    state::push(
        app,
        NotificationKind::Error,
        MessageKey::NotificationLoginFailed.into(),
        Some(description),
    );
}

fn set_login_state<R: Runtime>(app: &AppHandle<R>, login: LoginState) {
    if let Err(error) = app
        .state::<Store>()
        .update::<Session, _>(app, |s| s.login = login)
    {
        log::error!("cannot push the session: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d18_d20_attempts_differ_and_carry_pkce_s256() {
        let first = Attempt::new();
        let second = Attempt::new();
        assert_ne!(first.state, second.state);
        assert_ne!(first.verifier, second.verifier);
        assert!(first.state.starts_with("ng-"));
        assert_eq!(first.verifier.len(), 43, "RFC 7636: 43 to 128 characters");
        let url = Url::parse(&first.url).unwrap();
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let challenge = BASE64URL.encode(Sha256::digest(first.verifier.as_bytes()));
        let expected = [
            ("client_id", CLIENT_ID),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("state", &first.state),
            ("redirect_uri", REDIRECT_URI),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
        assert_eq!(query, expected);
        assert!(first.url.starts_with(AUTHORIZE_URL));
    }

    #[test]
    fn callback_reads_code_state_and_refusals() {
        let callback = Callback::parse("exilence-ce-ng://?code=a1b2%2Bc3&state=ng-x").unwrap();
        assert_eq!(
            (callback.state.as_str(), callback.code.as_deref()),
            ("ng-x", Some("a1b2+c3"))
        );
        let refused = Callback::parse("exilence-ce-ng://?error=access_denied&state=ng-y").unwrap();
        assert_eq!((refused.state.as_str(), refused.code), ("ng-y", None));
        assert!(Callback::parse("exilence://?code=a&state=ng-x").is_none());
        assert!(Callback::parse("not a url").is_none());
    }
}
