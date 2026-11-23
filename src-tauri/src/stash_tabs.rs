//! The stash tab lists of the account's leagues as GGG last sent them,
//! memory only (`client-schema.sql`). The session start loads the active
//! Profile's league; a Snapshot refreshes it (F3); the Profile dialog
//! refreshes the league it shows. Whether the active league has tabs is the
//! `session.league_has_tabs` start condition of a Snapshot (F3).

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

use ipc::slices::{Profiles, Session};
use tauri::{AppHandle, Manager, Runtime};

use crate::ggg::{self, Ggg, StashTab};
use crate::state::Store;

/// The lists by GGG league, managed by the app.
#[derive(Default)]
pub struct StashTabs {
    lists: Mutex<HashMap<String, Vec<StashTab>>>,
    /// The leagues [`follow_active`] loads in the background: the session
    /// start's Profile changes and its own call may follow at once.
    following: Mutex<HashSet<String>>,
}

impl StashTabs {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Vec<StashTab>>> {
        self.lists.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn following(&self) -> MutexGuard<'_, HashSet<String>> {
        self.following.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The list of `league`; `None` before it was loaded.
    pub fn get(&self, league: &str) -> Option<Vec<StashTab>> {
        self.lock().get(league).cloned()
    }
}

/// The active Profile's league, the `Standard` fallback applied (F2).
pub fn active_league<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    app.state::<Store>().read(|p: &Profiles| {
        let active = p.active_profile_id.as_ref()?;
        p.profiles
            .iter()
            .find(|profile| &profile.id == active)
            .map(|profile| profile.league_id.clone())
    })
}

/// Loads the stash tab list of `league` through the GGG queue, keeps it and
/// pushes `session.league_has_tabs`.
pub async fn load<R: Runtime>(
    app: &AppHandle<R>,
    league: &str,
) -> Result<Vec<StashTab>, ggg::Error> {
    let tabs = app.state::<Ggg>().stash_tabs(app, league).await?;
    log::debug!("loaded {} stash tabs of {league}", tabs.len());
    app.state::<StashTabs>()
        .lock()
        .insert(league.to_owned(), tabs.clone());
    push(app);
    Ok(tabs)
}

/// After the `profiles` slice changed: pushes `session.league_has_tabs`, and
/// once the session is ready loads the active league's list in the
/// background when it is neither known nor loading yet. Before that the
/// session start loads it.
pub fn follow_active<R: Runtime>(app: &AppHandle<R>) {
    push(app);
    let Some(league) = active_league(app) else {
        return;
    };
    if !app.state::<Store>().read(|s: &Session| s.ready) {
        return;
    }
    {
        let tabs = app.state::<StashTabs>();
        // Under `following`: a load inserts its list before it leaves it.
        let mut following = tabs.following();
        if tabs.get(&league).is_some() || !following.insert(league.clone()) {
            return;
        }
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = load(&app, &league).await {
            log::warn!("the stash tabs of {league} did not load: {error}");
        }
        app.state::<StashTabs>().following().remove(&league);
    });
}

/// Pushes whether the active Profile's league has stash tabs (F3).
pub fn push<R: Runtime>(app: &AppHandle<R>) {
    let has_tabs = active_league(app)
        .and_then(|league| app.state::<StashTabs>().get(&league))
        .is_some_and(|tabs| !tabs.is_empty());
    let store = app.state::<Store>();
    if store.read(|s: &Session| s.league_has_tabs == has_tabs) {
        return;
    }
    if let Err(error) = store.update::<Session, _>(app, |s| s.league_has_tabs = has_tabs) {
        log::error!("cannot push the session: {error}");
    }
}
