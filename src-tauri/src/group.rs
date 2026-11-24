//! Groups (F13, client-background-work.md, in a Group): `check_group_name`,
//! `create_group`, `join_group` and `leave_group` under the operation slot,
//! and `set_player_selected`. The client holds the Group stream while in a
//! Group (ADR 0004): opened right after a create or join, and when the
//! session start finds the account in a Group; closed on leave and when the
//! account's background work stops. Its events are applied in order: the
//! members, their active Profiles and the Snapshots they hold, with bodies
//! decoded and the item definitions they reference fetched into
//! `item_definitions` (`client-schema.sql`). Group state lives in memory
//! only. Each change pushes the `group` slice and the Group views: the
//! combined `net_worth`, `charts` and `items` of the selected players (D4,
//! D14, D15). A lost stream and its reconnect are [`group_connection`]'s; a
//! reopened stream keeps the Group and its player selection (D22).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use domain::net_worth::SnapshotTotal;
use ipc::command::GroupForm;
use ipc::slices::{Group, GroupMember, NotificationKind};
use ipc::{BackendReason, Field, IpcError, MessageKey};
use prost::Message as _;
use sqlx::SqlitePool;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Manager, Runtime};
use tonic::Streaming;
use wire::v1::group_event::Event;
use wire::v1::{
    CheckGroupNameRequest, CreateGroupRequest, GetItemDefinitionsRequest, GroupEvent, GroupSync,
    ItemDefinition, JoinGroupRequest, LeaveGroupRequest, MemberCursor, OpenGroupStreamRequest,
    SnapshotBody,
};

use crate::backend::{self, Backend};
use crate::snapshot::BODIES_KEPT;
use crate::state::{self, Pushing, Store};
use crate::{
    auth, charts, db, group_connection, group_upload_queue, internal, items, logging, net_worth,
    slot,
};

/// The most ids one `GetItemDefinitions` may ask for (`exilence.proto`).
pub const DEFINITIONS_PER_CALL: usize = 10_000;

/// The shortest Group password; empty means none (F13).
pub const MIN_PASSWORD_CHARS: usize = 6;

/// The Group the account is in, managed by the app; memory only.
#[derive(Default)]
pub struct Groups {
    joined: Mutex<Option<Joined>>,
    /// Numbers each stream: events of a closed one are dropped.
    next_stream: AtomicU64,
    /// A create, join or leave runs: it may end the stream on purpose.
    changing: AtomicBool,
}

struct Joined {
    stream: u64,
    name: String,
    /// In joining order.
    members: Vec<Member>,
    /// Reads the stream; aborted when it closes.
    reader: Option<JoinHandle<()>>,
    /// The stream ended while a create, join or leave ran.
    ended: bool,
}

/// A player of the Group as the client holds it.
pub struct Member {
    account_id: String,
    pub account_name: String,
    /// In the player selection: counts in the Group views.
    pub selected: bool,
    /// The signed-in account: its Snapshots come from the database.
    pub me: bool,
    profile: Option<wire::v1::Profile>,
    /// Of the active Profile, oldest first.
    pub snapshots: Vec<Held>,
}

/// A member's Snapshot: its totals, and its decoded body for the newest
/// [`BODIES_KEPT`].
pub struct Held {
    pub id: String,
    pub created_at: i64,
    pub total: f64,
    pub body: Option<SnapshotBody>,
}

/// The selected players, for the Group-combined `net_worth`.
pub struct Selection {
    /// The signed-in account counts.
    pub me: bool,
    /// The other selected players' Snapshot totals, newest first.
    pub others: Vec<Vec<SnapshotTotal>>,
}

impl Groups {
    fn lock(&self) -> MutexGuard<'_, Option<Joined>> {
        self.joined.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// In a Group: the selected players; `None` outside one.
    pub fn selection(&self) -> Option<Selection> {
        let joined = self.lock();
        let joined = joined.as_ref()?;
        let selected = || joined.members.iter().filter(|m| m.selected);
        Some(Selection {
            me: selected().any(|m| m.me),
            others: selected()
                .filter(|m| !m.me)
                .map(|m| {
                    m.snapshots
                        .iter()
                        .rev()
                        .map(|s| SnapshotTotal {
                            created_at: s.created_at,
                            total: s.total,
                        })
                        .collect()
                })
                .collect(),
        })
    }

    /// In a Group: `read` of each player, in joining order; `None` outside
    /// one. For the Group views, which follow the selection (D14, D15).
    pub fn read_members<T>(&self, read: impl FnMut(&Member) -> T) -> Option<Vec<T>> {
        let joined = self.lock();
        let joined = joined.as_ref()?;
        Some(joined.members.iter().map(read).collect())
    }

    fn slice(&self) -> Group {
        self.lock()
            .as_ref()
            .map_or_else(Group::default, |joined| Group {
                name: Some(joined.name.clone()),
                members: joined
                    .members
                    .iter()
                    .map(|m| GroupMember {
                        account_id: m.account_id.clone(),
                        account_name: m.account_name.clone(),
                        selected: m.selected,
                    })
                    .collect(),
            })
    }

    /// Changes the Group of stream `stream`; nothing once that stream closed.
    fn change(&self, stream: u64, change: impl FnOnce(&mut Joined)) -> bool {
        match self.lock().as_mut() {
            Some(joined) if joined.stream == stream => {
                change(joined);
                true
            }
            _ => false,
        }
    }

    /// Forgets the Group and stops reading its stream.
    fn forget(&self) -> bool {
        let Some(joined) = self.lock().take() else {
            return false;
        };
        if let Some(reader) = joined.reader {
            reader.abort();
        }
        true
    }

    /// The newest Snapshot held per other member: a rejoin's cursors;
    /// `None` outside a Group.
    fn cursors(&self) -> Option<Vec<MemberCursor>> {
        let joined = self.lock();
        Some(
            joined
                .as_ref()?
                .members
                .iter()
                .filter(|m| !m.me)
                .filter_map(|m| {
                    Some(MemberCursor {
                        account_id: m.account_id.clone(),
                        newest_snapshot_id: m.snapshots.last()?.id.clone(),
                    })
                })
                .collect(),
        )
    }

    /// Stops reading the stream but keeps the Group for a rejoin; false
    /// outside a Group.
    fn detach(&self) -> bool {
        let mut joined = self.lock();
        let Some(joined) = joined.as_mut() else {
            return false;
        };
        if let Some(reader) = joined.reader.take() {
            reader.abort();
        }
        true
    }

    /// Stream `stream`, read by the calling task, ended. True when it is
    /// the Group's and no create, join or leave runs, which may end it on
    /// purpose; such a change learns of it in [`Groups::finish_change`].
    fn ended(&self, stream: u64) -> bool {
        let mut joined = self.lock();
        match joined.as_mut() {
            Some(joined) if joined.stream == stream => {
                // The calling task ends by itself.
                joined.reader = None;
                if self.changing.load(Ordering::SeqCst) {
                    joined.ended = true;
                    false
                } else {
                    true
                }
            }
            _ => false,
        }
    }

    /// A create, join or leave starts.
    fn begin_change(&self) {
        self.changing.store(true, Ordering::SeqCst);
    }

    /// The create, join or leave is over: true when the Group it kept saw
    /// its stream end meanwhile.
    fn finish_change(&self) -> bool {
        self.changing.store(false, Ordering::SeqCst);
        self.lock()
            .as_mut()
            .is_some_and(|joined| std::mem::take(&mut joined.ended))
    }
}

/// Whether the account is in a Group: holding its stream, or reconnecting
/// to it (D22).
pub fn in_group<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<Groups>().lock().is_some() || group_connection::reconnecting(app)
}

/// Stops reading the Group stream, keeping the Group for the reconnect;
/// false outside a Group.
pub(crate) fn detach<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<Groups>().detach()
}

/// Pushes the `group` slice and the Group views.
async fn push<R: Runtime>(app: &AppHandle<R>) {
    let groups = app.state::<Groups>();
    {
        let pushing = app.state::<Pushing<Group>>();
        let _pushing = pushing.lock().await;
        let group = groups.slice();
        if let Err(error) = app.state::<Store>().update::<Group, _>(app, |g| *g = group) {
            log::error!("cannot push the group: {error}");
        }
    }
    tokio::join!(net_worth::push(app), charts::push(app), items::push(app),);
}

/// The stored Login's session token, after the superseded check (D25).
async fn session_token<R: Runtime>(app: &AppHandle<R>) -> Result<String, IpcError> {
    app.state::<Backend>().check_superseded()?;
    let token: Option<String> = sqlx::query_scalar("SELECT session_token FROM login")
        .fetch_optional(&*app.state::<SqlitePool>())
        .await
        .map_err(internal)?;
    token.ok_or(IpcError::SessionExpired)
}

/// `check_group_name(name)`: whether a Group of that name exists (F13).
/// Nobody is told about a failure; the field shows no error then.
pub async fn check_name<R: Runtime>(app: &AppHandle<R>, name: String) -> Result<bool, IpcError> {
    let token = session_token(app).await?;
    let request = backend::request(
        "CheckGroupName",
        CheckGroupNameRequest {
            name: name.trim().to_owned(),
        },
        Some(&token),
    );
    match app
        .state::<Backend>()
        .group()
        .check_group_name(request)
        .await
    {
        Ok(response) => Ok(response.into_inner().exists),
        Err(status) => Err(backend::failure(app, &status)),
    }
}

/// `form` with its name trimmed, if the name is not empty and the password
/// is empty or at least [`MIN_PASSWORD_CHARS`] long (F13).
fn valid(mut form: GroupForm) -> Result<GroupForm, IpcError> {
    form.name = form.name.trim().to_owned();
    if form.name.is_empty() {
        return Err(IpcError::Validation {
            field: Field::GroupName,
            key: MessageKey::ErrorGroupNameRequired,
        });
    }
    if !form.password.is_empty() && form.password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(IpcError::Validation {
            field: Field::GroupPassword,
            key: MessageKey::ErrorGroupPasswordTooShort,
        });
    }
    Ok(form)
}

/// A refusal the form shows on its field (F13): the name taken or not
/// found, a wrong password; other errors as they are.
fn on_field(error: IpcError) -> IpcError {
    let (field, key) = match error {
        IpcError::Backend {
            reason: BackendReason::GroupNameTaken,
        } => (Field::GroupName, MessageKey::ErrorGroupNameTaken),
        IpcError::Backend {
            reason: BackendReason::GroupNotFound,
        } => (Field::GroupName, MessageKey::ErrorGroupNotFound),
        IpcError::Backend {
            reason: BackendReason::WrongPassword,
        } => (Field::GroupPassword, MessageKey::ErrorWrongPassword),
        error => return error,
    };
    IpcError::Validation { field, key }
}

/// Create or join.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Create,
    Join,
}

/// `create_group(group)` and `join_group(group)` (F13): under the operation
/// slot, `CreateGroup` or `JoinGroup` (which leave any previous Group), then
/// the stream opens with all members selected. A refusal for the name or
/// password shows on its field; a wrong password is told nowhere else, any
/// other failure (a refused name too) is a notification with a toast, an
/// expired session goes to login.
pub async fn enter<R: Runtime>(
    app: &AppHandle<R>,
    entry: Entry,
    form: GroupForm,
) -> Result<(), IpcError> {
    let done = async {
        let _held = slot::try_hold(app)?;
        let _change = Change::begin(app);
        let GroupForm { name, password } = valid(form)?;
        // Never logged (F20).
        logging::secret(&password);
        let token = session_token(app).await?;
        let mut client = app.state::<Backend>().group();
        let entered = match entry {
            Entry::Create => {
                let request = CreateGroupRequest { name, password };
                let request = backend::request("CreateGroup", request, Some(&token));
                client.create_group(request).await.map(drop)
            }
            Entry::Join => {
                let request = JoinGroupRequest { name, password };
                let request = backend::request("JoinGroup", request, Some(&token));
                client.join_group(request).await.map(drop)
            }
        };
        entered.map_err(|status| backend::failure(app, &status))?;
        // The previous Group, if any, is left.
        close(app).await;
        open(app, &token).await
    }
    .await;
    match done {
        Ok(()) => {
            state::push(
                app,
                NotificationKind::Success,
                MessageKey::NotificationGroupJoined.into(),
                None,
            );
            Ok(())
        }
        // A wrong password shows on its field only; every other failure also
        // toasts, a refused name too (F18).
        Err(
            error @ IpcError::Backend {
                reason: BackendReason::WrongPassword,
            },
        ) => {
            log::warn!("{}: {error}", MessageKey::NotificationGroupJoinFailed.id());
            Err(on_field(error))
        }
        Err(error) => {
            let error =
                auth::action_failed(app, MessageKey::NotificationGroupJoinFailed, error).await;
            Err(on_field(error))
        }
    }
}

/// `leave_group` (F13): under the operation slot, `LeaveGroup`, then the
/// stream closes and the views are solo again. `NOT_IN_GROUP` means the
/// backend already let the account go: left all the same.
pub async fn leave<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let done = async {
        let _held = slot::try_hold(app)?;
        let _change = Change::begin(app);
        let token = session_token(app).await?;
        let request = backend::request("LeaveGroup", LeaveGroupRequest {}, Some(&token));
        match app.state::<Backend>().group().leave_group(request).await {
            Ok(_) => {}
            Err(status) => match backend::failure(app, &status) {
                IpcError::Backend {
                    reason: BackendReason::NotInGroup,
                } => log::info!("LeaveGroup: not in a Group any more"),
                error => return Err(error),
            },
        }
        close(app).await;
        Ok(())
    }
    .await;
    match done {
        Ok(()) => {
            log::info!("left the Group");
            state::push(
                app,
                NotificationKind::Success,
                MessageKey::NotificationGroupLeft.into(),
                None,
            );
            Ok(())
        }
        Err(error) => {
            Err(auth::action_failed(app, MessageKey::NotificationGroupLeaveFailed, error).await)
        }
    }
}

/// `set_player_selected(account_id, selected)` (F13).
pub async fn set_player_selected<R: Runtime>(app: &AppHandle<R>, account_id: &str, selected: bool) {
    let changed = match app.state::<Groups>().lock().as_mut() {
        Some(joined) => joined
            .members
            .iter_mut()
            .find(|m| m.account_id == account_id)
            .map(|m| m.selected = selected)
            .is_some(),
        None => false,
    };
    if changed {
        push(app).await;
    }
}

/// The session start found the account in a Group (`StartSession`'s
/// `group_name`): opens its stream as on a first join, all members
/// selected (F13). Without a connection the client is offline and
/// reconnects (D22).
pub async fn rejoin<R: Runtime>(app: &AppHandle<R>) {
    match open_stream(app).await {
        Ok(()) => log::info!("rejoined the Group"),
        Err(IpcError::Backend {
            reason: BackendReason::NotInGroup,
        }) => log::info!("the account is no longer in its Group"),
        Err(IpcError::SessionExpired) => auth::send_to_login(app).await,
        Err(IpcError::Superseded) => {}
        Err(error) => group_connection::lost(app, &format!("the rejoin failed: {error}")),
    }
}

/// Opens the Group stream: as on a first join, or, while the client holds
/// the Group, with its cursors and keeping the player selection (D22).
pub(crate) async fn open_stream<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let token = session_token(app).await?;
    open(app, &token).await
}

/// Closes the Group stream and forgets the Group (on leave, when the grace
/// ran out, and when the account's background work stops); clears the
/// Group upload queue (D27) and pushes the solo views.
pub async fn close<R: Runtime>(app: &AppHandle<R>) {
    group_connection::closed(app);
    group_upload_queue::clear(app);
    if app.state::<Groups>().forget() {
        log::info!("Group stream closed");
        push(app).await;
    }
}

/// [`close`] for code that cannot wait: the views follow in the background.
pub fn close_now<R: Runtime>(app: &AppHandle<R>) {
    group_connection::closed(app);
    group_upload_queue::clear(app);
    if app.state::<Groups>().forget() {
        log::info!("Group stream closed");
        let app = app.clone();
        tauri::async_runtime::spawn(async move { push(&app).await });
    }
}

/// A create, join or leave in progress, which may end the stream on
/// purpose ([`Groups::ended`]). Once it is over, a Group it kept whose
/// stream ended meanwhile has lost its connection.
struct Change<'a, R: Runtime>(&'a AppHandle<R>);

impl<'a, R: Runtime> Change<'a, R> {
    fn begin(app: &'a AppHandle<R>) -> Self {
        app.state::<Groups>().begin_change();
        Self(app)
    }
}

impl<R: Runtime> Drop for Change<'_, R> {
    fn drop(&mut self) {
        if self.0.state::<Groups>().finish_change() {
            group_connection::lost(self.0, "the Group stream ended");
        }
    }
}

/// Opens the stream and applies its first event, the `GroupSync`: as on a
/// first join with every member selected, or, while the client holds the
/// Group, after the newest Snapshot held per member and keeping the
/// selection, new members unselected (D22). Then reads the rest in the
/// background.
async fn open<R: Runtime>(app: &AppHandle<R>, token: &str) -> Result<(), IpcError> {
    let groups = app.state::<Groups>();
    let cursors = groups.cursors();
    let rejoin = cursors.is_some();
    let request = backend::request(
        "OpenGroupStream",
        OpenGroupStreamRequest {
            cursors: cursors.unwrap_or_default(),
        },
        Some(token),
    );
    let mut events = app
        .state::<Backend>()
        .group()
        .open_group_stream(request)
        .await
        .map_err(|status| backend::failure(app, &status))?
        .into_inner();
    let sync = match events.message().await {
        Ok(Some(GroupEvent {
            event: Some(Event::Sync(sync)),
        })) => sync,
        Ok(other) => return Err(internal(format!("the Group stream began with {other:?}"))),
        Err(status) => return Err(backend::failure(app, &status)),
    };
    let stream = groups.next_stream.fetch_add(1, Ordering::Relaxed);
    {
        let mut joined = groups.lock();
        match joined.as_mut() {
            Some(joined) => {
                if let Some(reader) = joined.reader.take() {
                    reader.abort();
                }
                joined.stream = stream;
                joined.ended = false;
            }
            None if rejoin => return Err(internal("the Group closed meanwhile")),
            None => {
                *joined = Some(Joined {
                    stream,
                    name: sync.group_name.clone(),
                    members: Vec::new(),
                    reader: None,
                    ended: false,
                });
            }
        }
    }
    log::debug!("Group stream {stream}: GroupSync");
    group_connection::opened(app);
    apply_sync(app, stream, token, sync, !rejoin).await;
    let reader = {
        let app = app.clone();
        let token = token.to_owned();
        tauri::async_runtime::spawn(async move { read(&app, stream, &token, events).await })
    };
    let mut reader = Some(reader);
    groups.change(stream, |joined| joined.reader = reader.take());
    // Closed meanwhile.
    if let Some(reader) = reader {
        reader.abort();
    }
    push(app).await;
    Ok(())
}

/// Applies the stream's events in order until it ends. Every event keeps
/// it alive; `Superseded` takes the client offline until restarted; an end
/// or error otherwise means the connection is lost (D22, D25).
async fn read<R: Runtime>(
    app: &AppHandle<R>,
    stream: u64,
    token: &str,
    mut events: Streaming<GroupEvent>,
) {
    let why = loop {
        match events.message().await {
            Ok(Some(GroupEvent {
                event: Some(Event::Superseded(_)),
            })) => {
                log::warn!("the Group stream says the account connected elsewhere");
                return backend::superseded(app);
            }
            Ok(Some(GroupEvent { event })) => {
                group_connection::heard(app);
                // `None`: a variant this client does not know
                // (`exilence.proto`).
                if let Some(event) = event {
                    log::debug!("Group stream {stream}: {}", event_name(&event));
                    apply(app, stream, token, event).await;
                }
            }
            Ok(None) => break "the Group stream ended".to_owned(),
            Err(status) => match backend::failure(app, &status) {
                IpcError::Superseded => return,
                IpcError::SessionExpired => return auth::send_to_login(app).await,
                error => break format!("the Group stream failed: {error}"),
            },
        }
    };
    if app.state::<Groups>().ended(stream) {
        group_connection::lost(app, &why);
    }
}

/// The proto name of a stream event, for Debug mode's connection log (F20).
fn event_name(event: &Event) -> &'static str {
    match event {
        Event::Sync(_) => "GroupSync",
        Event::MemberJoined(_) => "MemberJoined",
        Event::MemberLeft(_) => "MemberLeft",
        Event::ActiveProfileChanged(_) => "ActiveProfileChanged",
        Event::SnapshotAdded(_) => "SnapshotAdded",
        Event::SnapshotsRemoved(_) => "SnapshotsRemoved",
        Event::Heartbeat(_) => "Heartbeat",
        Event::Superseded(_) => "Superseded",
    }
}

/// Applies one event of stream `stream`, then pushes the views.
async fn apply<R: Runtime>(app: &AppHandle<R>, stream: u64, token: &str, event: Event) {
    let groups = app.state::<Groups>();
    let changed = match event {
        Event::Sync(sync) => return apply_sync(app, stream, token, sync, false).await,
        Event::MemberJoined(joined) => {
            let Some(member) = joined.member else {
                return;
            };
            let member = arrived(app, token, member).await;
            groups.change(stream, |joined| {
                match joined
                    .members
                    .iter_mut()
                    .find(|m| m.account_id == member.account_id)
                {
                    Some(known) => {
                        known.profile = member.profile;
                        known.snapshots = member.snapshots;
                    }
                    // Players who join later appear unselected (F13).
                    None => joined.members.push(member),
                }
            })
        }
        Event::MemberLeft(left) => groups.change(stream, |joined| {
            joined.members.retain(|m| m.account_id != left.account_id);
        }),
        Event::ActiveProfileChanged(changed) => {
            let latest = match changed.latest_snapshot {
                Some(snapshot) => held(app, token, vec![snapshot], &[])
                    .await
                    .into_iter()
                    .map(|(held, _)| held)
                    .collect(),
                None => Vec::new(),
            };
            groups.change(stream, |joined| {
                let Some(member) = joined
                    .members
                    .iter_mut()
                    .find(|m| m.account_id == changed.account_id)
                else {
                    return;
                };
                let same = member.profile.as_ref().map(|p| &p.id)
                    == changed.profile.as_ref().map(|p| &p.id);
                // A switch shows the new Profile's latest alone (D4); an
                // edit, or setting the active Profile again, keeps the
                // Snapshots held.
                if same {
                    for held in latest {
                        merge(&mut member.snapshots, held);
                    }
                } else {
                    member.snapshots = latest;
                }
                member.profile = changed.profile;
            })
        }
        Event::SnapshotAdded(added) => {
            let Some(snapshot) = added.snapshot else {
                return;
            };
            let new = held(app, token, vec![snapshot], &added.new_definitions).await;
            groups.change(stream, |joined| {
                let Some(member) = joined
                    .members
                    .iter_mut()
                    .find(|m| m.account_id == added.account_id)
                else {
                    return;
                };
                let active = member.profile.as_ref().map(|p| p.id.as_str());
                for (held, profile_id) in new {
                    if Some(profile_id.as_str()) == active {
                        merge(&mut member.snapshots, held);
                    }
                }
            })
        }
        Event::SnapshotsRemoved(removed) => groups.change(stream, |joined| {
            if let Some(member) = joined
                .members
                .iter_mut()
                .find(|m| m.account_id == removed.account_id)
            {
                if removed.snapshot_ids.len() > 16 {
                    let removed_ids: std::collections::HashSet<&str> =
                        removed.snapshot_ids.iter().map(String::as_str).collect();
                    member
                        .snapshots
                        .retain(|s| !removed_ids.contains(s.id.as_str()));
                } else {
                    member
                        .snapshots
                        .retain(|s| !removed.snapshot_ids.contains(&s.id));
                }
            }
        }),
        // `Superseded` never gets here: [`read`] handles it.
        Event::Heartbeat(_) | Event::Superseded(_) => false,
    };
    if changed {
        push(app).await;
    }
}

/// Applies a `GroupSync`. The first one of a join selects every member;
/// a later one keeps the selection, new members unselected (F13).
async fn apply_sync<R: Runtime>(
    app: &AppHandle<R>,
    stream: u64,
    token: &str,
    sync: GroupSync,
    first: bool,
) {
    let me: Option<String> = match sqlx::query_scalar("SELECT account_id FROM login")
        .fetch_optional(&*app.state::<SqlitePool>())
        .await
    {
        Ok(me) => me,
        Err(error) => {
            log::error!("cannot read the Login: {error}");
            None
        }
    };
    let mut members = Vec::with_capacity(sync.members.len());
    for member in sync.members {
        let mut member = arrived(app, token, member).await;
        member.me = me.as_deref() == Some(member.account_id.as_str());
        members.push(member);
    }
    let changed = app.state::<Groups>().change(stream, |joined| {
        joined.name = sync.group_name;
        for member in &mut members {
            member.selected = first
                || joined
                    .members
                    .iter()
                    .find(|m| m.account_id == member.account_id)
                    .is_some_and(|m| m.selected);
            if let Some(known) = joined
                .members
                .iter_mut()
                .find(|m| m.account_id == member.account_id)
                && known.profile.as_ref().map(|p| &p.id) == member.profile.as_ref().map(|p| &p.id)
            {
                // Snapshots after a rejoin's cursors add to those held.
                for held in std::mem::take(&mut member.snapshots) {
                    merge(&mut known.snapshots, held);
                }
                member.snapshots = std::mem::take(&mut known.snapshots);
            }
        }
        joined.members = members;
    });
    if changed && !first {
        push(app).await;
    }
}

/// A member as it arrived, unselected, with their Snapshots held.
async fn arrived<R: Runtime>(app: &AppHandle<R>, token: &str, member: wire::v1::Member) -> Member {
    let account = member.account.unwrap_or_default();
    let mut snapshots = Vec::new();
    for (held, _) in held(app, token, member.snapshots, &[]).await {
        merge(&mut snapshots, held);
    }
    Member {
        account_id: account.id,
        account_name: account.name,
        selected: false,
        me: false,
        profile: member.active_profile,
        snapshots,
    }
}

/// Keeps `held` in `snapshots`, oldest first, once; drops the bodies of all
/// but the newest [`BODIES_KEPT`].
fn merge(snapshots: &mut Vec<Held>, held: Held) {
    if snapshots.iter().any(|s| s.id == held.id) {
        return;
    }
    let at = snapshots.partition_point(|s| (s.created_at, &s.id) < (held.created_at, &held.id));
    snapshots.insert(at, held);
    let kept = snapshots.len().saturating_sub(BODIES_KEPT as usize);
    for old in &mut snapshots[..kept] {
        old.body = None;
    }
}

/// Wire Snapshots as held, each with its Profile id: bodies decoded, the
/// `provided` definitions stored and the missing ones fetched (failures
/// are logged; the totals still count).
async fn held<R: Runtime>(
    app: &AppHandle<R>,
    token: &str,
    snapshots: Vec<wire::v1::Snapshot>,
    provided: &[ItemDefinition],
) -> Vec<(Held, String)> {
    let pool = app.state::<SqlitePool>();
    if let Err(error) = store_definitions(&pool, provided).await {
        log::error!("cannot store the relayed item definitions: {error}");
    }
    let held: Vec<(Held, String)> = snapshots
        .into_iter()
        .map(|snapshot| {
            let body = (!snapshot.body.is_empty())
                .then(|| decode(&snapshot.body))
                .and_then(|decoded| match decoded {
                    Ok(body) => Some(body),
                    Err(error) => {
                        log::warn!(
                            "cannot decode the body of Snapshot {}: {error}",
                            snapshot.id
                        );
                        None
                    }
                });
            let created_at = snapshot.created_at.as_ref().map_or(0, backend::unix_ms);
            let held = Held {
                id: snapshot.id,
                created_at,
                total: snapshot.total_value,
                body,
            };
            (held, snapshot.profile_id)
        })
        .collect();
    let referenced: BTreeSet<u64> = held
        .iter()
        .filter_map(|(h, _)| h.body.as_ref())
        .flat_map(|body| {
            body.tabs
                .iter()
                .flat_map(|tab| tab.definition_ids.iter())
                .chain(&body.priced_definition_ids)
                .copied()
        })
        .collect();
    if let Err(error) = fetch_missing(app, &pool, token, referenced).await {
        log::warn!("cannot fetch the missing item definitions: {error}");
    }
    held
}

/// A Snapshot body as uploaded: zstd-compressed `SnapshotBody`.
fn decode(body: &[u8]) -> Result<SnapshotBody, String> {
    let decompressed = zstd::stream::decode_all(body).map_err(|e| e.to_string())?;
    SnapshotBody::decode(&decompressed[..]).map_err(|e| e.to_string())
}

/// Stores definitions that came from the backend, each after checking its
/// id is the hash of its content; the backend has them (`uploaded`).
async fn store_definitions(pool: &SqlitePool, definitions: &[ItemDefinition]) -> sqlx::Result<()> {
    if definitions.is_empty() {
        return Ok(());
    }
    let valid: Vec<(i64, Vec<u8>)> = definitions
        .iter()
        .filter_map(|definition| {
            let (bytes, id) = definition.content_bytes_and_id();
            if definition.id != id {
                log::warn!(
                    "ignored item definition {:016x}: not the hash of its content",
                    definition.id
                );
                None
            } else {
                Some((definition.id as i64, bytes))
            }
        })
        .collect();
    if valid.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for chunk in valid.chunks(500) {
        let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> =
            sqlx::QueryBuilder::new("INSERT INTO item_definitions (id, encoded, uploaded) ");
        qb.push_values(chunk, |mut b, (id, bytes)| {
            b.push_bind(*id).push_bind(bytes).push_bind(1i64);
        });
        qb.push(" ON CONFLICT (id) DO UPDATE SET uploaded = 1");
        qb.build().execute(&mut *tx).await?;
    }
    tx.commit().await
}

/// Fetches the `referenced` definitions not stored yet, at most
/// [`DEFINITIONS_PER_CALL`] per `GetItemDefinitions`, and stores them.
async fn fetch_missing<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    token: &str,
    referenced: BTreeSet<u64>,
) -> Result<(), IpcError> {
    if referenced.is_empty() {
        return Ok(());
    }
    let ids = db::json_text(&referenced.iter().map(|&id| id as i64).collect::<Vec<_>>())
        .map_err(internal)?;
    // One row: SQLx hands each row from its SQLite thread to this task, so
    // a Snapshot of 10,000 new definitions would cost 10,000 hand-offs.
    let missing_json: String = sqlx::query_scalar(
        "SELECT json_group_array(value) FROM json_each(?1) \
         WHERE value NOT IN (SELECT id FROM item_definitions)",
    )
    .bind(ids)
    .fetch_one(pool)
    .await
    .map_err(internal)?;
    let missing: Vec<i64> = serde_json::from_str(&missing_json).map_err(internal)?;
    for chunk in missing.chunks(DEFINITIONS_PER_CALL) {
        app.state::<Backend>().check_superseded()?;
        let request = GetItemDefinitionsRequest {
            ids: chunk.iter().map(|&id| id as u64).collect(),
        };
        let request = backend::request("GetItemDefinitions", request, Some(token));
        let definitions = app
            .state::<Backend>()
            .group()
            .get_item_definitions(request)
            .await
            .map_err(|status| backend::failure(app, &status))?
            .into_inner()
            .definitions;
        store_definitions(pool, &definitions)
            .await
            .map_err(internal)?;
    }
    Ok(())
}
