//! An in-process backend: tonic servers of `wire`'s AuthService,
//! SessionService, ProfileService and GroupService, scripted per test,
//! recording what the core sent (parity verification, fakes) and which
//! Snapshots it holds a copy of. The test sends the Group stream's events
//! after its scripted first one.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use prost::Message;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::codec::CompressionEncoding;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic::{Code, Request, Response, Status};
use wire::v1::auth_service_server::{AuthService, AuthServiceServer};
use wire::v1::group_event::Event;
use wire::v1::group_service_server::{GroupService, GroupServiceServer};
use wire::v1::profile_service_server::{ProfileService, ProfileServiceServer};
use wire::v1::session_service_server::{SessionService, SessionServiceServer};
use wire::v1::{
    Account, CheckGroupNameRequest, CheckGroupNameResponse, CreateGroupRequest,
    CreateGroupResponse, CreateProfileRequest, CreateProfileResponse, ErrorDetail, ErrorReason,
    GetItemDefinitionsRequest, GetItemDefinitionsResponse, GggAccessToken, GroupEvent, GroupSync,
    ItemDefinition, JoinGroupRequest, JoinGroupResponse, LeaveGroupRequest, LeaveGroupResponse,
    Member, MemberCursor, OpenGroupStreamRequest, Profile, RefreshGggTokenRequest,
    RefreshGggTokenResponse, RemoveProfileRequest, RemoveProfileResponse, RemoveSnapshotsRequest,
    RemoveSnapshotsResponse, SetActiveProfileRequest, SetActiveProfileResponse, SignInRequest,
    SignInResponse, SignOutRequest, SignOutResponse, StartSessionRequest, StartSessionResponse,
    UpdateProfileRequest, UpdateProfileResponse, UploadSnapshotRequest, UploadSnapshotResponse,
};

pub const ACCOUNT_ID: &str = "4b0c2f7e-6b1a-4c3d-9e8f-0a1b2c3d4e5f";
pub const ACCOUNT_NAME: &str = "Foo#1234";
pub const SESSION_TOKEN: &str = "c2Vzc2lvbi10b2tlbi1mb3ItdGhlLWZha2UtYmFja2VuZA";
pub const ACCESS_TOKEN: &str = "ggg-access-token-a1b2c3d4";
/// 2030-03-17T17:46:40Z.
pub const ACCESS_TOKEN_EXPIRES_AT: i64 = 1_900_000_000_000;

fn access_token(token: &str, expires_at: i64) -> GggAccessToken {
    GggAccessToken {
        access_token: token.to_owned(),
        expires_at: Some(prost_types::Timestamp {
            seconds: expires_at / 1000,
            nanos: 0,
        }),
    }
}

/// A successful `SignIn` of [`ACCOUNT_NAME`].
pub fn signed_in() -> Result<SignInResponse, Status> {
    Ok(SignInResponse {
        session_token: SESSION_TOKEN.to_owned(),
        ggg_token: Some(access_token(ACCESS_TOKEN, ACCESS_TOKEN_EXPIRES_AT)),
        account: Some(account()),
    })
}

/// A successful `RefreshGggToken` handing out `token`.
pub fn refreshed(token: &str, expires_at: i64) -> Result<RefreshGggTokenResponse, Status> {
    Ok(RefreshGggTokenResponse {
        ggg_token: Some(access_token(token, expires_at)),
    })
}

/// A successful `StartSession` of [`ACCOUNT_NAME`] with `profiles`.
pub fn session(
    profiles: Vec<wire::v1::Profile>,
    active_profile_id: &str,
) -> Result<StartSessionResponse, Status> {
    Ok(StartSessionResponse {
        account: Some(account()),
        profiles,
        active_profile_id: active_profile_id.to_owned(),
        group_name: None,
    })
}

/// A successful `StartSession` of an account with [`default_profile`] in
/// the Group named `name`.
pub fn session_in_group(name: &str) -> Result<StartSessionResponse, Status> {
    let mut answer = session(vec![default_profile()], &default_profile().id)?;
    answer.group_name = Some(name.to_owned());
    Ok(answer)
}

/// The account's Profile when `StartSession` is not scripted: an account
/// that has one, so no `profile 1` is created (F2).
pub fn default_profile() -> wire::v1::Profile {
    wire::v1::Profile {
        id: "01a1cd36-0000-7000-8000-000000000001".to_owned(),
        name: "profile 1".to_owned(),
        league_id: "Settlers".to_owned(),
        price_league_id: "Settlers".to_owned(),
        stash_tab_ids: Vec::new(),
        character_name: None,
        include_equipment: false,
        include_inventory: false,
    }
}

/// A status with `reason` in its details, as the backend refuses.
pub fn refused(code: Code, reason: ErrorReason) -> Status {
    let detail = ErrorDetail {
        reason: reason.into(),
    };
    Status::with_details(code, "refused", detail.encode_to_vec().into())
}

/// The fake backend's account, as a Group member.
pub fn account() -> Account {
    Account {
        id: ACCOUNT_ID.to_owned(),
        name: ACCOUNT_NAME.to_owned(),
    }
}

/// What the core sent.
#[derive(Default)]
pub struct Calls {
    pub sign_ins: Vec<SignInRequest>,
    /// `authorization` metadata and client version of each `StartSession`.
    pub start_sessions: Vec<(Option<String>, String)>,
    /// `authorization` metadata of each `RefreshGggToken`.
    pub refreshes: Vec<Option<String>>,
    /// `authorization` metadata of each `SignOut`.
    pub sign_outs: Vec<Option<String>>,
    /// `x-correlation-id` of every call.
    pub correlation_ids: Vec<String>,
    /// Every Profile call, in order.
    pub profile_calls: Vec<ProfileCall>,
    /// Every `UploadSnapshot`, in order.
    pub uploads: Vec<UploadSnapshotRequest>,
    /// `grpc-timeout` metadata of each `UploadSnapshot`.
    pub upload_timeouts: Vec<Option<String>>,
    /// Every Group call, in order.
    pub group_calls: Vec<GroupCall>,
    /// The cursors of each `OpenGroupStream`.
    pub open_cursors: Vec<Vec<MemberCursor>>,
}

/// A Group call the core sent.
#[derive(Clone, Debug, PartialEq)]
pub enum GroupCall {
    CheckName(String),
    Create {
        name: String,
        password: String,
    },
    Join {
        name: String,
        password: String,
    },
    Leave,
    /// `OpenGroupStream` with that many cursors.
    Open(usize),
    /// `GetItemDefinitions` with these ids.
    GetDefinitions(Vec<u64>),
}

/// A Profile call the core sent.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileCall {
    Create(Profile),
    Update(Profile),
    SetActive(String),
    Remove(String),
    /// `RemoveSnapshots`: Profile id and Snapshot ids.
    RemoveSnapshots(String, Vec<String>),
}

/// The answers of the next calls, in order. Without one, `StartSession`
/// succeeds with [`default_profile`], `RefreshGggToken` is UNIMPLEMENTED,
/// `SignOut`, a Profile call and `UploadSnapshot` succeed.
#[derive(Default)]
pub struct Script {
    pub start_sessions: VecDeque<Result<StartSessionResponse, Status>>,
    pub refreshes: VecDeque<Result<RefreshGggTokenResponse, Status>>,
    /// How long each `RefreshGggToken` takes.
    pub refresh_delay: Duration,
    /// The failures of the next Profile calls, any of them but
    /// `UploadSnapshot`; a status with `Code::Ok` lets its call succeed.
    pub profile_failures: VecDeque<Status>,
    pub sign_outs: VecDeque<Result<SignOutResponse, Status>>,
    /// How long each `SignOut` takes.
    pub sign_out_delay: Duration,
    /// The failures of the next `UploadSnapshot` calls.
    pub upload_failures: VecDeque<Status>,
    /// Whether the names of the next `CheckGroupName` calls exist.
    pub group_names: VecDeque<bool>,
    /// The failures of the next `CreateGroup`, `JoinGroup` and `LeaveGroup`
    /// calls, any of the three.
    pub group_failures: VecDeque<Status>,
    /// The first event of each next Group stream, or its failure.
    pub group_syncs: VecDeque<Result<GroupSync, Status>>,
    /// What `GetItemDefinitions` knows.
    pub definitions: HashMap<u64, ItemDefinition>,
    /// Holds the next `UploadSnapshot` after recording it.
    pub held_upload: Option<Hold>,
    /// Holds the next Profile call after recording it.
    pub held_profile_call: Option<Hold>,
}

/// The fake's ends of a [`HeldCall`].
pub struct Hold {
    /// The call answers once this receives.
    released: oneshot::Receiver<()>,
    /// Then it reports here that it has.
    answered: oneshot::Sender<()>,
}

/// `answer`, once `held`, if any, is released; then reports that the held
/// call has answered.
async fn answer_when_released<T>(held: Option<Hold>, answer: impl FnOnce() -> T) -> T {
    let Some(hold) = held else {
        return answer();
    };
    let _ = hold.released.await;
    let answer = answer();
    let _ = hold.answered.send(());
    answer
}

type Events = mpsc::UnboundedSender<Result<GroupEvent, Status>>;

#[derive(Clone)]
struct Fake {
    sign_in: Arc<Result<SignInResponse, Status>>,
    script: Arc<Mutex<Script>>,
    calls: Arc<Mutex<Calls>>,
    /// The open Group stream's sender.
    stream: Arc<Mutex<Option<Events>>>,
    /// The ids of the Snapshots the backend holds a copy of.
    copies: Arc<Mutex<BTreeSet<String>>>,
}

impl Fake {
    fn record<T>(&self, request: &Request<T>) {
        let id = metadata(request, "x-correlation-id").unwrap_or_default();
        self.calls.lock().unwrap().correlation_ids.push(id);
    }
}

fn metadata<T>(request: &Request<T>, name: &str) -> Option<String> {
    request
        .metadata()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

#[tonic::async_trait]
impl AuthService for Fake {
    async fn sign_in(
        &self,
        request: Request<SignInRequest>,
    ) -> Result<Response<SignInResponse>, Status> {
        self.record(&request);
        self.calls
            .lock()
            .unwrap()
            .sign_ins
            .push(request.into_inner());
        (*self.sign_in).clone().map(Response::new)
    }

    async fn refresh_ggg_token(
        &self,
        request: Request<RefreshGggTokenRequest>,
    ) -> Result<Response<RefreshGggTokenResponse>, Status> {
        self.record(&request);
        let authorization = metadata(&request, "authorization");
        self.calls.lock().unwrap().refreshes.push(authorization);
        let (delay, answer) = {
            let mut script = self.script.lock().unwrap();
            (script.refresh_delay, script.refreshes.pop_front())
        };
        tokio::time::sleep(delay).await;
        answer
            .unwrap_or_else(|| Err(Status::unimplemented("not scripted")))
            .map(Response::new)
    }

    async fn sign_out(
        &self,
        request: Request<SignOutRequest>,
    ) -> Result<Response<SignOutResponse>, Status> {
        self.record(&request);
        let authorization = metadata(&request, "authorization");
        self.calls.lock().unwrap().sign_outs.push(authorization);
        let (delay, answer) = {
            let mut script = self.script.lock().unwrap();
            (script.sign_out_delay, script.sign_outs.pop_front())
        };
        tokio::time::sleep(delay).await;
        answer.unwrap_or(Ok(SignOutResponse {})).map(Response::new)
    }
}

#[tonic::async_trait]
impl SessionService for Fake {
    async fn start_session(
        &self,
        request: Request<StartSessionRequest>,
    ) -> Result<Response<StartSessionResponse>, Status> {
        self.record(&request);
        let authorization = metadata(&request, "authorization");
        self.calls
            .lock()
            .unwrap()
            .start_sessions
            .push((authorization, request.into_inner().client_version));
        let answer = self.script.lock().unwrap().start_sessions.pop_front();
        answer
            .unwrap_or_else(|| session(vec![default_profile()], &default_profile().id))
            .map(Response::new)
    }
}

impl Fake {
    /// Records `call`, waits while it is held, then answers with the next
    /// scripted failure, if any.
    async fn profile_call<T: Default>(&self, call: ProfileCall) -> Result<Response<T>, Status> {
        self.calls.lock().unwrap().profile_calls.push(call);
        let held = self.script.lock().unwrap().held_profile_call.take();
        answer_when_released(held, || {
            match self.script.lock().unwrap().profile_failures.pop_front() {
                Some(status) if status.code() != Code::Ok => Err(status),
                _ => Ok(Response::new(T::default())),
            }
        })
        .await
    }
}

#[tonic::async_trait]
impl ProfileService for Fake {
    async fn create_profile(
        &self,
        request: Request<CreateProfileRequest>,
    ) -> Result<Response<CreateProfileResponse>, Status> {
        self.record(&request);
        let profile = request.into_inner().profile.unwrap_or_default();
        self.profile_call(ProfileCall::Create(profile)).await
    }

    async fn update_profile(
        &self,
        request: Request<UpdateProfileRequest>,
    ) -> Result<Response<UpdateProfileResponse>, Status> {
        self.record(&request);
        let profile = request.into_inner().profile.unwrap_or_default();
        self.profile_call(ProfileCall::Update(profile)).await
    }

    async fn set_active_profile(
        &self,
        request: Request<SetActiveProfileRequest>,
    ) -> Result<Response<SetActiveProfileResponse>, Status> {
        self.record(&request);
        self.profile_call(ProfileCall::SetActive(request.into_inner().profile_id))
            .await
    }

    async fn remove_profile(
        &self,
        request: Request<RemoveProfileRequest>,
    ) -> Result<Response<RemoveProfileResponse>, Status> {
        self.record(&request);
        self.profile_call(ProfileCall::Remove(request.into_inner().profile_id))
            .await
    }

    async fn upload_snapshot(
        &self,
        request: Request<UploadSnapshotRequest>,
    ) -> Result<Response<UploadSnapshotResponse>, Status> {
        self.record(&request);
        let timeout = metadata(&request, "grpc-timeout");
        let request = request.into_inner();
        let id = request.snapshot.as_ref().map(|s| s.id.clone());
        {
            let mut calls = self.calls.lock().unwrap();
            calls.upload_timeouts.push(timeout);
            calls.uploads.push(request);
        }
        let held = self.script.lock().unwrap().held_upload.take();
        answer_when_released(held, || {
            match self.script.lock().unwrap().upload_failures.pop_front() {
                Some(status) => Err(status),
                None => {
                    self.copies.lock().unwrap().extend(id);
                    Ok(Response::new(UploadSnapshotResponse {}))
                }
            }
        })
        .await
    }

    async fn remove_snapshots(
        &self,
        request: Request<RemoveSnapshotsRequest>,
    ) -> Result<Response<RemoveSnapshotsResponse>, Status> {
        self.record(&request);
        let RemoveSnapshotsRequest {
            profile_id,
            snapshot_ids,
        } = request.into_inner();
        let answer = self
            .profile_call(ProfileCall::RemoveSnapshots(
                profile_id,
                snapshot_ids.clone(),
            ))
            .await;
        if answer.is_ok() {
            let mut copies = self.copies.lock().unwrap();
            for id in &snapshot_ids {
                copies.remove(id);
            }
        }
        answer
    }
}

/// The account alone in a Group named `name`, as `GroupSync` sends it.
pub fn group_sync(name: &str, others: Vec<Member>) -> GroupSync {
    let mut members = vec![Member {
        account: Some(account()),
        active_profile: Some(default_profile()),
        snapshots: Vec::new(),
    }];
    members.extend(others);
    GroupSync {
        group_name: name.to_owned(),
        members,
    }
}

impl Fake {
    fn group_call(&self, call: GroupCall) {
        self.calls.lock().unwrap().group_calls.push(call);
    }

    /// The next scripted failure of a membership call, if any.
    fn group_failure(&self) -> Result<(), Status> {
        match self.script.lock().unwrap().group_failures.pop_front() {
            Some(status) => Err(status),
            None => Ok(()),
        }
    }
}

#[tonic::async_trait]
impl GroupService for Fake {
    async fn check_group_name(
        &self,
        request: Request<CheckGroupNameRequest>,
    ) -> Result<Response<CheckGroupNameResponse>, Status> {
        self.record(&request);
        self.group_call(GroupCall::CheckName(request.into_inner().name));
        let exists = self
            .script
            .lock()
            .unwrap()
            .group_names
            .pop_front()
            .unwrap_or(false);
        Ok(Response::new(CheckGroupNameResponse { exists }))
    }

    async fn create_group(
        &self,
        request: Request<CreateGroupRequest>,
    ) -> Result<Response<CreateGroupResponse>, Status> {
        self.record(&request);
        let CreateGroupRequest { name, password } = request.into_inner();
        self.group_call(GroupCall::Create { name, password });
        self.group_failure()?;
        Ok(Response::new(CreateGroupResponse {}))
    }

    async fn join_group(
        &self,
        request: Request<JoinGroupRequest>,
    ) -> Result<Response<JoinGroupResponse>, Status> {
        self.record(&request);
        let JoinGroupRequest { name, password } = request.into_inner();
        self.group_call(GroupCall::Join { name, password });
        self.group_failure()?;
        Ok(Response::new(JoinGroupResponse {}))
    }

    async fn leave_group(
        &self,
        request: Request<LeaveGroupRequest>,
    ) -> Result<Response<LeaveGroupResponse>, Status> {
        self.record(&request);
        self.group_call(GroupCall::Leave);
        self.group_failure()?;
        // The caller's stream ends.
        self.stream.lock().unwrap().take();
        Ok(Response::new(LeaveGroupResponse {}))
    }

    type OpenGroupStreamStream = UnboundedReceiverStream<Result<GroupEvent, Status>>;

    async fn open_group_stream(
        &self,
        request: Request<OpenGroupStreamRequest>,
    ) -> Result<Response<Self::OpenGroupStreamStream>, Status> {
        self.record(&request);
        let cursors = request.into_inner().cursors;
        let mut calls = self.calls.lock().unwrap();
        calls.group_calls.push(GroupCall::Open(cursors.len()));
        calls.open_cursors.push(cursors);
        drop(calls);
        let first = self.script.lock().unwrap().group_syncs.pop_front();
        let sync = first.unwrap_or_else(|| Ok(group_sync("Rich Exiles123", Vec::new())))?;
        let (events, received) = mpsc::unbounded_channel();
        events
            .send(Ok(GroupEvent {
                event: Some(Event::Sync(sync)),
            }))
            .unwrap();
        *self.stream.lock().unwrap() = Some(events);
        Ok(Response::new(UnboundedReceiverStream::new(received)))
    }

    async fn get_item_definitions(
        &self,
        request: Request<GetItemDefinitionsRequest>,
    ) -> Result<Response<GetItemDefinitionsResponse>, Status> {
        self.record(&request);
        let ids = request.into_inner().ids;
        self.group_call(GroupCall::GetDefinitions(ids.clone()));
        if ids.len() > 10_000 {
            return Err(refused(Code::InvalidArgument, ErrorReason::InvalidArgument));
        }
        let script = self.script.lock().unwrap();
        let definitions = ids
            .iter()
            .filter_map(|id| script.definitions.get(id).cloned())
            .collect();
        Ok(Response::new(GetItemDefinitionsResponse { definitions }))
    }
}

/// The fake backend, serving until the test process ends.
pub struct FakeBackend {
    pub url: String,
    pub script: Arc<Mutex<Script>>,
    pub calls: Arc<Mutex<Calls>>,
    stream: Arc<Mutex<Option<Events>>>,
    copies: Arc<Mutex<BTreeSet<String>>>,
}

/// A call the fake holds before answering ([`FakeBackend::hold_next_upload`],
/// [`FakeBackend::hold_next_profile_call`]).
pub struct HeldCall {
    release: oneshot::Sender<()>,
    answered: oneshot::Receiver<()>,
}

impl HeldCall {
    /// The fake's end, to put in the [`Script`], and the test's.
    fn new() -> (Hold, Self) {
        let (release, released) = oneshot::channel();
        let (answer, answered) = oneshot::channel();
        let hold = Hold {
            released,
            answered: answer,
        };
        (hold, Self { release, answered })
    }

    /// Lets the held call answer and waits until it has.
    pub fn release(self) {
        let _ = self.release.send(());
        let answered = self.answered;
        tauri::async_runtime::block_on(async { tokio::time::timeout(super::WAIT, answered).await })
            .expect("the held call answers")
            .unwrap();
    }
}

impl FakeBackend {
    /// Answers every `SignIn` with `sign_in`.
    pub fn start(sign_in: Result<SignInResponse, Status>) -> Self {
        let fake = Fake {
            sign_in: Arc::new(sign_in),
            script: Arc::default(),
            calls: Arc::default(),
            stream: Arc::default(),
            copies: Arc::default(),
        };
        let calls = Arc::clone(&fake.calls);
        let script = Arc::clone(&fake.script);
        let stream = Arc::clone(&fake.stream);
        let copies = Arc::clone(&fake.copies);
        let url = tauri::async_runtime::block_on(async move {
            let incoming = TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let url = format!("http://{}", incoming.local_addr().unwrap());
            let server = Server::builder()
                .add_service(
                    AuthServiceServer::new(fake.clone())
                        .accept_compressed(CompressionEncoding::Zstd)
                        .send_compressed(CompressionEncoding::Zstd),
                )
                .add_service(
                    SessionServiceServer::new(fake.clone())
                        .accept_compressed(CompressionEncoding::Zstd)
                        .send_compressed(CompressionEncoding::Zstd),
                )
                .add_service(
                    ProfileServiceServer::new(fake.clone())
                        .accept_compressed(CompressionEncoding::Zstd)
                        .send_compressed(CompressionEncoding::Zstd),
                )
                .add_service(
                    GroupServiceServer::new(fake)
                        .accept_compressed(CompressionEncoding::Zstd)
                        .send_compressed(CompressionEncoding::Zstd),
                )
                .serve_with_incoming(incoming);
            tauri::async_runtime::spawn(server);
            url
        });
        Self {
            url,
            script,
            calls,
            stream,
            copies,
        }
    }

    /// Answers the next `StartSession` calls with `answers`, in order.
    pub fn script_start_sessions(
        &self,
        answers: impl IntoIterator<Item = Result<StartSessionResponse, Status>>,
    ) {
        self.script.lock().unwrap().start_sessions.extend(answers);
    }

    /// Answers the next `RefreshGggToken` calls with `answers`, in order.
    pub fn script_refreshes(
        &self,
        answers: impl IntoIterator<Item = Result<RefreshGggTokenResponse, Status>>,
    ) {
        self.script.lock().unwrap().refreshes.extend(answers);
    }

    pub fn start_session_count(&self) -> usize {
        self.calls.lock().unwrap().start_sessions.len()
    }

    pub fn refresh_count(&self) -> usize {
        self.calls.lock().unwrap().refreshes.len()
    }

    /// Fails the next Profile calls with `failures`, in order.
    pub fn script_profile_failures(&self, failures: impl IntoIterator<Item = Status>) {
        self.script
            .lock()
            .unwrap()
            .profile_failures
            .extend(failures);
    }

    /// Fails the next `UploadSnapshot` calls with `failures`, in order.
    pub fn script_upload_failures(&self, failures: impl IntoIterator<Item = Status>) {
        self.script.lock().unwrap().upload_failures.extend(failures);
    }

    /// Every `UploadSnapshot` so far.
    pub fn uploads(&self) -> Vec<UploadSnapshotRequest> {
        self.calls.lock().unwrap().uploads.clone()
    }

    /// Holds the next `UploadSnapshot`, recorded but unanswered, until the
    /// returned [`HeldCall`] is released.
    pub fn hold_next_upload(&self) -> HeldCall {
        let (hold, held) = HeldCall::new();
        self.script.lock().unwrap().held_upload = Some(hold);
        held
    }

    /// Holds the next Profile call, recorded but unanswered, until the
    /// returned [`HeldCall`] is released.
    pub fn hold_next_profile_call(&self) -> HeldCall {
        let (hold, held) = HeldCall::new();
        self.script.lock().unwrap().held_profile_call = Some(hold);
        held
    }

    /// The ids of the Snapshots the backend holds a copy of, sorted: an
    /// `UploadSnapshot` stores one, `RemoveSnapshots` deletes them.
    pub fn copies(&self) -> Vec<String> {
        self.copies.lock().unwrap().iter().cloned().collect()
    }

    pub fn profile_calls(&self) -> Vec<ProfileCall> {
        self.calls.lock().unwrap().profile_calls.clone()
    }

    /// `authorization` metadata of each `SignOut` so far.
    pub fn sign_outs(&self) -> Vec<Option<String>> {
        self.calls.lock().unwrap().sign_outs.clone()
    }

    /// Every Group call so far.
    pub fn group_calls(&self) -> Vec<GroupCall> {
        self.calls.lock().unwrap().group_calls.clone()
    }

    /// Scripts the next streams' first events, in order.
    pub fn script_group_syncs(&self, syncs: impl IntoIterator<Item = Result<GroupSync, Status>>) {
        self.script.lock().unwrap().group_syncs.extend(syncs);
    }

    /// Fails the next `CreateGroup`, `JoinGroup` or `LeaveGroup` calls.
    pub fn script_group_failures(&self, failures: impl IntoIterator<Item = Status>) {
        self.script.lock().unwrap().group_failures.extend(failures);
    }

    /// Answers the next `CheckGroupName` calls, in order.
    pub fn script_group_names(&self, exists: impl IntoIterator<Item = bool>) {
        self.script.lock().unwrap().group_names.extend(exists);
    }

    /// Makes `GetItemDefinitions` know `definitions`.
    pub fn know_definitions(&self, definitions: impl IntoIterator<Item = ItemDefinition>) {
        let mut script = self.script.lock().unwrap();
        script
            .definitions
            .extend(definitions.into_iter().map(|d| (d.id, d)));
    }

    /// Sends `event` on the open Group stream; false without one.
    pub fn send_group_event(&self, event: Event) -> bool {
        self.stream
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|events| events.send(Ok(GroupEvent { event: Some(event) })).is_ok())
    }

    /// Ends the open Group stream as the backend does, without `Superseded`.
    pub fn end_group_stream(&self) {
        self.stream.lock().unwrap().take();
    }

    /// Fails the open Group stream with `status`.
    pub fn fail_group_stream(&self, status: Status) {
        if let Some(events) = self.stream.lock().unwrap().take() {
            let _ = events.send(Err(status));
        }
    }

    /// The cursors of each `OpenGroupStream` so far.
    pub fn open_cursors(&self) -> Vec<Vec<MemberCursor>> {
        self.calls.lock().unwrap().open_cursors.clone()
    }

    /// Whether the core holds a Group stream: its receiving end is open.
    pub fn group_stream_open(&self) -> bool {
        self.stream
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|events| !events.is_closed())
    }
}
