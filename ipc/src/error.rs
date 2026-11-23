//! [`IpcError`], the error of every command.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::message::{Message, MessageKey};

/// Why a command failed. The UI uses it only for inline form errors (Group
/// name and password, Profile name); everything else reaches the user through
/// the core's notifications and toasts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IpcError {
    /// A GGG request would exceed the rate limit (F7).
    RateLimited { seconds_left: u64 },
    /// The operation slot is taken, e.g. by a running Snapshot.
    Busy,
    /// A Snapshot cannot start: the session is not ready, the league has
    /// no tabs, or prices of the price league are missing or updating (F3).
    NotReady,
    /// The backend cannot be reached.
    Offline,
    /// The Login is no longer valid; the user has to sign in again.
    SessionExpired,
    /// The same account connected elsewhere (F13, D25).
    Superseded,
    /// A form field holds an invalid value.
    Validation { field: Field, key: MessageKey },
    /// The backend refused the call.
    Backend { reason: BackendReason },
    /// Anything else; the correlation ID finds it in the log (F20).
    Internal { correlation_id: String },
}

/// A form field with inline validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    GroupName,
    GroupPassword,
    ProfileName,
    /// Settings (F4, F5, F17).
    AutoSnapshotMinutes,
    PriceThreshold,
    TotalPriceThreshold,
    UiScale,
    /// The item table's page size (F11).
    ItemTablePageSize,
    /// The custom-price dialog (F6).
    CustomPrice,
}

/// The wire's `ErrorReason` (`exilence.proto`), mirrored because `wire` does
/// not build for WASM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendReason {
    Unspecified,
    SessionExpired,
    Superseded,
    GggRefreshFailed,
    GroupNameTaken,
    GroupNotFound,
    WrongPassword,
    NotInGroup,
    InvalidArgument,
    GggAuthFailed,
}

impl IpcError {
    /// The message the UI shows for this error.
    pub fn message(&self) -> Message {
        match self {
            Self::RateLimited { seconds_left } => {
                Message::new(MessageKey::ErrorRateLimited).arg("seconds", *seconds_left)
            }
            Self::Busy => MessageKey::ErrorBusy.into(),
            Self::NotReady => MessageKey::ErrorNotReady.into(),
            Self::Offline => MessageKey::ErrorOffline.into(),
            Self::SessionExpired => MessageKey::ErrorSessionExpired.into(),
            Self::Superseded => MessageKey::ErrorSuperseded.into(),
            Self::Validation { key, .. } => (*key).into(),
            Self::Backend { reason } => reason.message_key().into(),
            Self::Internal { correlation_id } => Message::new(MessageKey::ErrorInternal)
                .arg("correlation_id", correlation_id.as_str()),
        }
    }
}

impl BackendReason {
    fn message_key(self) -> MessageKey {
        match self {
            Self::SessionExpired => MessageKey::ErrorSessionExpired,
            Self::Superseded => MessageKey::ErrorSuperseded,
            Self::GggRefreshFailed => MessageKey::ErrorGggRefreshFailed,
            Self::GroupNameTaken => MessageKey::ErrorGroupNameTaken,
            Self::GroupNotFound => MessageKey::ErrorGroupNotFound,
            Self::WrongPassword => MessageKey::ErrorWrongPassword,
            Self::NotInGroup => MessageKey::ErrorNotInGroup,
            Self::GggAuthFailed => MessageKey::ErrorGggAuthFailed,
            Self::Unspecified | Self::InvalidArgument => MessageKey::ErrorBackend,
        }
    }
}

/// For logs, which are English only (F20); never shown to the user.
impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited { seconds_left } => write!(f, "rate limited for {seconds_left} s"),
            Self::Busy => f.write_str("busy"),
            Self::NotReady => f.write_str("not ready for a Snapshot"),
            Self::Offline => f.write_str("offline"),
            Self::SessionExpired => f.write_str("session expired"),
            Self::Superseded => f.write_str("superseded"),
            Self::Validation { field, key } => write!(f, "invalid {field:?}: {}", key.id()),
            Self::Backend { reason } => write!(f, "backend refused: {reason:?}"),
            Self::Internal { correlation_id } => write!(f, "internal error {correlation_id}"),
        }
    }
}

impl std::error::Error for IpcError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_tagged_data_without_text() {
        let error = IpcError::Validation {
            field: Field::GroupPassword,
            key: MessageKey::ErrorWrongPassword,
        };
        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"validation","field":"group_password","key":"error-wrong-password"}"#
        );
        assert_eq!(serde_json::from_str::<IpcError>(&json).unwrap(), error);
        assert_eq!(
            serde_json::to_string(&IpcError::RateLimited { seconds_left: 12 }).unwrap(),
            r#"{"kind":"rate_limited","seconds_left":12}"#
        );
    }
}
