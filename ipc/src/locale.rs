//! The locale resources (D10): English only, in Fluent, read by the UI for
//! everything it shows and by the core for the tray menu (F15, D13).

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};

use crate::message::{Message, MessageArg, MessageKey};

/// The English resources, embedded in both binaries.
pub const ENGLISH: &str = include_str!("../locales/en.ftl");

/// Resolves [`Message`]s into text of one language.
pub struct Locale {
    bundle: FluentBundle<FluentResource>,
}

impl Locale {
    /// The English locale.
    #[expect(
        clippy::expect_used,
        reason = "en.ftl is embedded and parsed by a test"
    )]
    pub fn english() -> Self {
        let resource = FluentResource::try_new(ENGLISH.to_owned()).expect("en.ftl is valid Fluent");
        let mut bundle = FluentBundle::new_concurrent(vec![
            "en".parse().expect("`en` is a language identifier"),
        ]);
        // No Unicode isolation marks around arguments: the text goes into
        // HTML and the tray, both left-to-right.
        bundle.set_use_isolating(false);
        bundle
            .add_resource(resource)
            .expect("en.ftl has no duplicate messages");
        Self { bundle }
    }

    /// The text of `message`. A missing argument shows as `{$name}`.
    pub fn text(&self, message: &Message) -> String {
        let Some(pattern) = self
            .bundle
            .get_message(message.key.id())
            .and_then(|m| m.value())
        else {
            return message.key.id().to_owned();
        };
        let mut args = FluentArgs::with_capacity(message.args.len());
        for (name, value) in &message.args {
            let value = match value {
                MessageArg::Number(n) => FluentValue::from(*n),
                MessageArg::Text(s) => FluentValue::from(s.as_str()),
            };
            args.set(name.as_str(), value);
        }
        let mut errors = Vec::new();
        self.bundle
            .format_pattern(pattern, Some(&args), &mut errors)
            .into_owned()
    }

    /// The text of a message without arguments.
    pub fn key(&self, key: MessageKey) -> String {
        self.text(&Message::new(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent_syntax::ast::Entry;

    /// Every key has a message and every message is a key (D10).
    #[test]
    fn d10_locale_messages_match_message_keys() {
        let resource = FluentResource::try_new(ENGLISH.to_owned()).unwrap();
        let mut ids: Vec<&str> = resource
            .entries()
            .filter_map(|entry| match entry {
                Entry::Message(m) => Some(m.id.name),
                _ => None,
            })
            .collect();
        let mut keys: Vec<&str> = MessageKey::ALL.iter().map(|k| k.id()).collect();
        ids.sort_unstable();
        keys.sort_unstable();
        assert_eq!(ids, keys);
    }

    #[test]
    fn resolves_arguments_and_plurals() {
        let locale = Locale::english();
        let unread =
            |n: u32| locale.text(&Message::new(MessageKey::NotificationsUnread).arg("count", n));
        assert_eq!(unread(0), "No unread notifications");
        assert_eq!(unread(1), "One unread notification");
        assert_eq!(unread(7), "7 unread notifications");
        let internal = Message::new(MessageKey::ErrorInternal).arg("correlation_id", "abc-1");
        assert_eq!(
            locale.text(&internal),
            "Something went wrong (reference abc-1)."
        );
        assert_eq!(locale.key(MessageKey::AppTitle), "Exilence CE NG");
    }
}
