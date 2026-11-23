//! The login screen (F1): the platform, remembered; **Authorize**, which opens
//! GGG's consent page from the core; and the copy-link control.

use ipc::MessageKey;
use ipc::command::{CopyLoginLink, NoArgs, StartLogin, StartLoginArgs};
use ipc::slices::{LoginState, Platform, Session};
use leptos::prelude::*;

use crate::LOCALE;
use crate::state::State;
use crate::tauri;

/// The login screen's route.
pub const LOGIN_PATH: &str = "/login";

const PLATFORMS: [(Platform, MessageKey); 3] = [
    (Platform::Pc, MessageKey::PlatformPc),
    (Platform::Xbox, MessageKey::PlatformXbox),
    (Platform::Playstation, MessageKey::PlatformPlaystation),
];

#[component]
pub fn Login(state: State) -> impl IntoView {
    // The remembered platform until the user picks one.
    let chosen = RwSignal::new(None::<Platform>);
    let platform = move || {
        chosen
            .get()
            .unwrap_or_else(|| state.settings.with(|s| s.platform))
    };
    let signing_in = move || state.session.with(|s| s.login == LoginState::SigningIn);
    let options = PLATFORMS.map(|(value, key)| {
        view! {
            <option value=platform_value(value) prop:selected=move || platform() == value>
                {LOCALE.key(key)}
            </option>
        }
    });
    let authorize = move |_| {
        tauri::send::<StartLogin>(StartLoginArgs {
            platform: platform(),
        })
    };
    let copy_link = |_| tauri::send::<CopyLoginLink>(NoArgs {});
    view! {
        <section class="login">
            <h1>{LOCALE.key(MessageKey::AppTitle)}</h1>
            <label for="platform">{LOCALE.key(MessageKey::LoginPlatform)}</label>
            <select
                id="platform"
                disabled=signing_in
                on:change=move |ev| chosen.set(Some(parse_platform(&event_target_value(&ev))))
            >
                {options}
            </select>
            <button class="primary" disabled=signing_in on:click=authorize>
                {move || {
                    LOCALE.key(if signing_in() {
                        MessageKey::LoginSigningIn
                    } else {
                        MessageKey::LoginAuthorize
                    })
                }}
            </button>
            <p class="hint">{LOCALE.key(MessageKey::LoginCopyLinkHint)}</p>
            <button
                class="link"
                disabled=signing_in
                on:click=copy_link
            >
                {LOCALE.key(MessageKey::LoginCopyLink)}
            </button>
        </section>
    }
}

/// Where `session` sends a window showing `path`: a signed-out user to the
/// login screen, a signed-in one from it to the Net worth view.
pub fn redirect(session: &Session, path: &str) -> Option<&'static str> {
    let signed_in = session.login == LoginState::SignedIn;
    match (signed_in, path == LOGIN_PATH) {
        (true, true) => Some("/"),
        (false, false) => Some(LOGIN_PATH),
        _ => None,
    }
}

/// The `<option>` value of a platform.
fn platform_value(platform: Platform) -> &'static str {
    match platform {
        Platform::Pc => "pc",
        Platform::Xbox => "xbox",
        Platform::Playstation => "playstation",
    }
}

fn parse_platform(value: &str) -> Platform {
    match value {
        "xbox" => Platform::Xbox,
        "playstation" => Platform::Playstation,
        _ => Platform::Pc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_option_values_round_trip() {
        for (platform, _) in PLATFORMS {
            assert_eq!(parse_platform(platform_value(platform)), platform);
        }
    }

    #[test]
    fn f1_signed_out_goes_to_login_and_signed_in_to_net_worth() {
        let session = |login| Session {
            login,
            ..Session::default()
        };
        let signed_out = session(LoginState::SignedOut);
        assert_eq!(redirect(&signed_out, "/"), Some(LOGIN_PATH));
        assert_eq!(redirect(&signed_out, "/settings"), Some(LOGIN_PATH));
        assert_eq!(redirect(&signed_out, LOGIN_PATH), None);
        assert_eq!(redirect(&session(LoginState::SigningIn), LOGIN_PATH), None);
        let signed_in = session(LoginState::SignedIn);
        assert_eq!(redirect(&signed_in, LOGIN_PATH), Some("/"));
        assert_eq!(redirect(&signed_in, "/settings"), None);
    }
}
