//! The backend's gRPC clients (`exilence.proto`): one lazily connected
//! channel, zstd in both directions, a fresh correlation ID per call (F20) and
//! the session token as bearer. Once superseded, no call leaves until the app
//! restarts (D25). In Debug mode every call is logged with its correlation
//! ID, status and duration (F20).

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use http_body::{Frame, SizeHint};
use ipc::slices::{NotificationKind, Session};
use ipc::{BackendReason, IpcError, MessageKey};
use prost::Message;
use tauri::{AppHandle, Manager, Runtime};
use tonic::body::Body;
use tonic::codec::CompressionEncoding;
use tonic::codegen::{Bytes, Service};
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::{Code, Request, Status};
use uuid::Uuid;
use wire::v1::auth_service_client::AuthServiceClient;
use wire::v1::group_service_client::GroupServiceClient;
use wire::v1::profile_service_client::ProfileServiceClient;
use wire::v1::session_service_client::SessionServiceClient;
use wire::v1::{ErrorDetail, ErrorReason};

use crate::state::{self, Store};

/// The production backend.
pub const URL: &str = "https://api.exilence.de";

/// The backend, managed by the app.
pub struct Backend {
    channel: LoggedChannel,
    /// Another Login of the account started a session later (D25).
    superseded: AtomicBool,
}

impl Backend {
    /// A channel to `url` that connects on first use; `https` trusts the
    /// webpki roots. Needs a tokio runtime.
    pub fn new(url: &str) -> Result<Self, tonic::transport::Error> {
        let mut endpoint = Endpoint::from_shared(url.to_owned())?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30));
        if url.starts_with("https://") {
            endpoint = endpoint.tls_config(ClientTlsConfig::new().with_webpki_roots())?;
        }
        Ok(Self {
            channel: LoggedChannel {
                channel: endpoint.connect_lazy(),
                url: url.trim_end_matches('/').into(),
            },
            superseded: AtomicBool::new(false),
        })
    }

    /// Fails at once with `Superseded` once another Login took over: the
    /// client stays offline until restarted (client-background-work.md,
    /// outside a Group). Every call but `SignIn`, which makes a new Login,
    /// checks it first.
    pub fn check_superseded(&self) -> Result<(), IpcError> {
        if self.superseded.load(Ordering::SeqCst) {
            Err(IpcError::Superseded)
        } else {
            Ok(())
        }
    }

    pub fn auth(&self) -> AuthServiceClient<LoggedChannel> {
        AuthServiceClient::new(self.channel.clone())
            .send_compressed(CompressionEncoding::Zstd)
            .accept_compressed(CompressionEncoding::Zstd)
    }

    pub fn session(&self) -> SessionServiceClient<LoggedChannel> {
        SessionServiceClient::new(self.channel.clone())
            .send_compressed(CompressionEncoding::Zstd)
            .accept_compressed(CompressionEncoding::Zstd)
    }

    pub fn profile(&self) -> ProfileServiceClient<LoggedChannel> {
        ProfileServiceClient::new(self.channel.clone())
            .send_compressed(CompressionEncoding::Zstd)
            .accept_compressed(CompressionEncoding::Zstd)
    }

    pub fn group(&self) -> GroupServiceClient<LoggedChannel> {
        GroupServiceClient::new(self.channel.clone())
            .send_compressed(CompressionEncoding::Zstd)
            .accept_compressed(CompressionEncoding::Zstd)
    }
}

/// The backend channel. In Debug mode it logs each call once it ends: its
/// URL and correlation ID, gRPC status, duration and rate-limit headers, and
/// a failed call's message up to [`ninja::http::BODY_LIMIT`] (F20).
#[derive(Clone)]
pub struct LoggedChannel {
    channel: Channel,
    url: Arc<str>,
}

type ResponseFuture =
    Pin<Box<dyn Future<Output = Result<http::Response<Body>, tonic::transport::Error>> + Send>>;

impl Service<http::Request<Body>> for LoggedChannel {
    type Response = http::Response<Body>;
    type Error = tonic::transport::Error;
    type Future = ResponseFuture;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.channel.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        if !log::log_enabled!(log::Level::Debug) {
            return Box::pin(self.channel.call(request));
        }
        let correlation_id = request
            .headers()
            .get("x-correlation-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("none");
        let mut call = Call {
            line: format!(
                "backend {} {}{}, correlation ID {correlation_id}",
                request.method(),
                self.url,
                request.uri().path()
            ),
            started: Instant::now(),
            limits: String::new(),
        };
        let response = self.channel.call(request);
        Box::pin(async move {
            let response = match response.await {
                Ok(response) => response,
                Err(error) => {
                    call.end(&format!("no answer: {error}"));
                    return Err(error);
                }
            };
            call.limits = ninja::http::rate_limit_headers(response.headers());
            // A failure without a body carries its status in the headers.
            if let Some(status) = Status::from_header_map(response.headers()) {
                call.end_with(Some(&status));
                return Ok(response);
            }
            Ok(response.map(|body| {
                Body::new(LoggedBody {
                    body,
                    call: Some(call),
                })
            }))
        })
    }
}

/// A call being logged.
struct Call {
    line: String,
    started: Instant,
    limits: String,
}

impl Call {
    fn end(&self, outcome: &str) {
        let ms = self.started.elapsed().as_millis();
        log::debug!("{}: {outcome} after {ms} ms{}", self.line, self.limits);
    }

    fn end_with(&self, status: Option<&Status>) {
        match status {
            Some(status) if status.code() == Code::Ok => self.end("OK"),
            Some(status) => self.end(&format!(
                "{:?}: {}",
                status.code(),
                ninja::http::excerpt(status.message().as_bytes())
            )),
            None => self.end("ended without a status"),
        }
    }
}

/// A response body that logs its call at its trailers, its end or when it is
/// dropped early.
struct LoggedBody {
    body: Body,
    call: Option<Call>,
}

impl http_body::Body for LoggedBody {
    type Data = Bytes;
    type Error = Status;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Status>>> {
        let polled = Pin::new(&mut self.body).poll_frame(cx);
        // Some once the call ended: with the status it ended with, if any.
        let ended = match &polled {
            Poll::Ready(Some(Ok(frame))) => frame.trailers_ref().map(Status::from_header_map),
            Poll::Ready(Some(Err(status))) => Some(Some(status.clone())),
            Poll::Ready(None) => Some(None),
            Poll::Pending => None,
        };
        if let Some(status) = ended
            && let Some(call) = self.call.take()
        {
            call.end_with(status.as_ref());
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

impl Drop for LoggedBody {
    fn drop(&mut self) {
        if let Some(call) = self.call.take() {
            call.end("dropped before its end");
        }
    }
}

/// `message` as the request of `call`, with a fresh correlation ID, logged
/// before sending (F20), and `session_token` as bearer when given.
#[expect(clippy::expect_used, reason = "a UUID is ASCII")]
pub fn request<T>(call: &str, message: T, session_token: Option<&str>) -> Request<T> {
    let id = Uuid::now_v7().to_string();
    log::info!("backend {call}, correlation ID {id}");
    let mut request = Request::new(message);
    let metadata = request.metadata_mut();
    metadata.insert("x-correlation-id", id.parse().expect("a UUID is ASCII"));
    if let Some(token) = session_token {
        match format!("Bearer {token}").parse() {
            Ok(value) => {
                metadata.insert("authorization", value);
            }
            // A stored token is base64url; anything else is refused as
            // expired by the backend.
            Err(_) => log::warn!("the stored session token is not a header value"),
        }
    }
    request
}

/// What a failed call means for the user. An unknown `ErrorReason` is a
/// generic backend error (`exilence.proto`, evolution).
pub fn ipc_error(status: &Status) -> IpcError {
    if let Ok(detail) = ErrorDetail::decode(status.details())
        && detail.reason != 0
    {
        return match ErrorReason::try_from(detail.reason) {
            Ok(ErrorReason::SessionExpired) => IpcError::SessionExpired,
            Ok(ErrorReason::Superseded) => IpcError::Superseded,
            Ok(reason) => IpcError::Backend {
                reason: backend_reason(reason),
            },
            Err(_) => IpcError::Backend {
                reason: BackendReason::Unspecified,
            },
        };
    }
    match status.code() {
        Code::Unavailable | Code::DeadlineExceeded | Code::Cancelled => IpcError::Offline,
        code => crate::internal(format!("backend answered {code:?}: {}", status.message())),
    }
}

/// [`ipc_error`] for a failed call of this app; `SUPERSEDED` goes through
/// [`superseded`].
pub fn failure<R: Runtime>(app: &AppHandle<R>, status: &Status) -> IpcError {
    let error = ipc_error(status);
    if error == IpcError::Superseded {
        superseded(app);
    }
    error
}

/// Another Login of the account started a session (`SUPERSEDED`, or the
/// Group stream's `Superseded` event): the client is offline until
/// restarted, the first time with `superseded` in the `session` slice and a
/// note in the notification list, without a toast (client-background-work.md,
/// backend connection), and its Group closed (D22).
pub fn superseded<R: Runtime>(app: &AppHandle<R>) {
    if app
        .state::<Backend>()
        .superseded
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    log::warn!("superseded by another login of the account: no backend calls until restart");
    if let Err(error) = app
        .state::<Store>()
        .update::<Session, _>(app, |s| s.superseded = true)
    {
        log::error!("cannot push the session: {error}");
    }
    state::push(
        app,
        NotificationKind::Warning,
        MessageKey::ErrorSuperseded.into(),
        None,
    );
    crate::group_connection::superseded(app);
}

fn backend_reason(reason: ErrorReason) -> BackendReason {
    match reason {
        ErrorReason::Unspecified => BackendReason::Unspecified,
        ErrorReason::SessionExpired => BackendReason::SessionExpired,
        ErrorReason::Superseded => BackendReason::Superseded,
        ErrorReason::GggRefreshFailed => BackendReason::GggRefreshFailed,
        ErrorReason::GroupNameTaken => BackendReason::GroupNameTaken,
        ErrorReason::GroupNotFound => BackendReason::GroupNotFound,
        ErrorReason::WrongPassword => BackendReason::WrongPassword,
        ErrorReason::NotInGroup => BackendReason::NotInGroup,
        ErrorReason::InvalidArgument => BackendReason::InvalidArgument,
        ErrorReason::GggAuthFailed => BackendReason::GggAuthFailed,
    }
}

/// A wire time as Unix milliseconds.
pub fn unix_ms(time: &prost_types::Timestamp) -> i64 {
    time.seconds * 1000 + i64::from(time.nanos) / 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_reason(code: Code, reason: i32) -> Status {
        let detail = ErrorDetail { reason }.encode_to_vec();
        Status::with_details(code, "refused", detail.into())
    }

    #[test]
    fn statuses_map_to_ipc_errors() {
        assert_eq!(
            ipc_error(&with_reason(
                Code::Unauthenticated,
                ErrorReason::GggAuthFailed.into()
            )),
            IpcError::Backend {
                reason: BackendReason::GggAuthFailed
            }
        );
        assert_eq!(
            ipc_error(&with_reason(
                Code::Unauthenticated,
                ErrorReason::SessionExpired.into()
            )),
            IpcError::SessionExpired
        );
        assert_eq!(
            ipc_error(&with_reason(Code::Unknown, 999)),
            IpcError::Backend {
                reason: BackendReason::Unspecified
            }
        );
        assert_eq!(ipc_error(&Status::unavailable("down")), IpcError::Offline);
        assert!(matches!(
            ipc_error(&Status::internal("boom")),
            IpcError::Internal { .. }
        ));
    }
}
