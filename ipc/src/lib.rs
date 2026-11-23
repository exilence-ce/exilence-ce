//! The contract between the Tauri core and the Leptos UI: command arguments,
//! results and errors, the event enum, the state slice types, message keys,
//! the locale resources and the currency format helper. Compiles for native
//! and WASM.
//!
//! No English text crosses IPC (D10): user-facing text travels as
//! [`Message`]s, which the UI resolves through [`Locale`].

pub mod command;
pub mod currency;
mod error;
mod event;
pub mod links;
mod locale;
mod message;
pub mod slices;

pub use error::{BackendReason, Field, IpcError};
pub use event::{Event, Slice, SliceEvent, SliceName, Slices, TOAST_EVENT};
pub use locale::{ENGLISH, Locale};
pub use message::{Message, MessageArg, MessageKey};
