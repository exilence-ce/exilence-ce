//! Slice names, the [`Event`] enum and the [`Slice`] trait.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::slices::*;

/// A state slice: pushed whole as an [`Event`].
pub trait Slice: Clone + Default + PartialEq + Serialize + DeserializeOwned + 'static {
    /// This slice in `slices`.
    fn get(slices: &Slices) -> &Self;

    /// This slice in `slices`, mutably.
    fn get_mut(slices: &mut Slices) -> &mut Self;

    /// The event that carries this slice, borrowing it.
    fn as_event(&self) -> SliceEvent<'_>;
}

/// Declares [`SliceName`], [`Event`], [`Slices`] and the [`Slice`] impls from
/// one list of `Variant => field: Type`.
macro_rules! slices {
    ($($variant:ident => $field:ident: $ty:ty,)*) => {
        /// The name of a state slice. Its `snake_case` form is also the name of
        /// the Tauri event that carries it.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum SliceName {
            $($variant,)*
        }

        impl SliceName {
            /// Every slice, in declaration order.
            pub const ALL: &[SliceName] = &[$(SliceName::$variant,)*];

            /// The Tauri event name of this slice.
            pub const fn event_name(self) -> &'static str {
                match self {
                    $(SliceName::$variant => stringify!($field),)*
                }
            }
        }

        /// What the core pushes to the webviews: one slice, whole, or a toast.
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(tag = "event", content = "value", rename_all = "snake_case")]
        pub enum Event {
            $($variant($ty),)*
            Toast(Toast),
        }

        impl Event {
            /// The Tauri event name this event is emitted under.
            pub const fn name(&self) -> &'static str {
                match self {
                    $(Event::$variant(_) => SliceName::$variant.event_name(),)*
                    Event::Toast(_) => TOAST_EVENT,
                }
            }

            /// The slice this event carries; `None` for a toast.
            pub const fn slice(&self) -> Option<SliceName> {
                match self {
                    $(Event::$variant(_) => Some(SliceName::$variant),)*
                    Event::Toast(_) => None,
                }
            }
        }

        /// An [`Event`] carrying a borrowed slice, serialised the same: what
        /// the core emits without copying the slice.
        #[derive(Clone, Copy, Debug, Serialize)]
        #[serde(tag = "event", content = "value", rename_all = "snake_case")]
        pub enum SliceEvent<'a> {
            $($variant(&'a $ty),)*
        }

        impl SliceEvent<'_> {
            /// The Tauri event name this event is emitted under.
            pub const fn name(&self) -> &'static str {
                match self {
                    $(SliceEvent::$variant(_) => SliceName::$variant.event_name(),)*
                }
            }
        }

        /// Every slice; the core's whole state.
        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
        pub struct Slices {
            $(pub $field: $ty,)*
        }

        impl Slices {
            /// The event carrying the current value of slice `name`.
            pub fn event(&self, name: SliceName) -> Event {
                match name {
                    $(SliceName::$variant => Event::$variant(self.$field.clone()),)*
                }
            }

            /// The event borrowing slice `name`.
            #[cfg(test)]
            fn slice_event(&self, name: SliceName) -> SliceEvent<'_> {
                match name {
                    $(SliceName::$variant => self.$field.as_event(),)*
                }
            }
        }

        $(
            impl Slice for $ty {
                fn get(slices: &Slices) -> &Self {
                    &slices.$field
                }

                fn get_mut(slices: &mut Slices) -> &mut Self {
                    &mut slices.$field
                }

                fn as_event(&self) -> SliceEvent<'_> {
                    SliceEvent::$variant(self)
                }
            }
        )*
    };
}

slices! {
    Session => session: Session,
    Status => status: Status,
    RateLimit => rate_limit: RateLimit,
    Profiles => profiles: Profiles,
    Prices => prices: Prices,
    NetWorth => net_worth: NetWorth,
    Items => items: Items,
    Charts => charts: Charts,
    Group => group: Group,
    Notifications => notifications: Notifications,
    Settings => settings: Settings,
    Update => update: Update,
    LegacyImport => legacy_import: LegacyImport,
}

/// The Tauri event name of [`Event::Toast`].
pub const TOAST_EVENT: &str = "toast";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_match_slice_names() {
        for &name in SliceName::ALL {
            let event = Slices::default().event(name);
            assert_eq!(event.name(), name.event_name());
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["event"], name.event_name());
            assert_eq!(serde_json::from_value::<Event>(json).unwrap(), event);
        }
    }

    #[test]
    fn a_borrowed_slice_event_serialises_as_its_event() {
        let slices = Slices::default();
        for &name in SliceName::ALL {
            let borrowed = slices.slice_event(name);
            assert_eq!(borrowed.name(), name.event_name());
            assert_eq!(
                serde_json::to_string(&borrowed).unwrap(),
                serde_json::to_string(&slices.event(name)).unwrap()
            );
        }
    }
}
