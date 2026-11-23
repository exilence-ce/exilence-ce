//! What's New and Credits (F19, D11): the client repository's markdown,
//! fetched by the core and rendered here without raw HTML.

use ipc::MessageKey;
use ipc::command::{GetChangelog, GetCredits, LogLevel, NoArgs};
use leptos::prelude::*;
use leptos::task::spawn_local;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use wasm_bindgen::JsCast;

use crate::LOCALE;
use crate::shell::open_link;
use crate::tauri;

/// `markdown` as HTML that carries nothing but the markdown's own markup:
/// raw HTML shows as text, images as their alt text, and only HTTPS links
/// stay links; any other link keeps just its text.
pub fn to_html(markdown: &str) -> String {
    // Links never nest, but the stack keeps each end with its start anyway.
    let mut kept_links = Vec::new();
    let events = Parser::new_ext(markdown, Options::empty()).filter_map(|event| match event {
        Event::Html(html) | Event::InlineHtml(html) => Some(Event::Text(html)),
        Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => None,
        Event::Start(Tag::Link { ref dest_url, .. }) => {
            let kept = is_https(dest_url);
            kept_links.push(kept);
            kept.then_some(event)
        }
        Event::End(TagEnd::Link) => kept_links.pop().unwrap_or(false).then_some(event),
        event => Some(event),
    });
    let mut html = String::with_capacity(markdown.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut html, events);
    html
}

fn is_https(url: &str) -> bool {
    url.get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
}

/// The two markdown dialogs of the support panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoPage {
    /// The changelog; also opens by itself once after the first tour.
    WhatsNew,
    Credits,
}

impl InfoPage {
    fn title(self) -> MessageKey {
        match self {
            Self::WhatsNew => MessageKey::WhatsNewTitle,
            Self::Credits => MessageKey::SupportCredits,
        }
    }

    async fn load(self) -> Result<String, tauri::InvokeError> {
        match self {
            Self::WhatsNew => tauri::invoke::<GetChangelog>(&NoArgs {}).await,
            Self::Credits => tauri::invoke::<GetCredits>(&NoArgs {}).await,
        }
    }
}

/// `page` while `open`: its markdown is fetched each time it opens. Links
/// open in the system browser, never in the window.
#[component]
pub fn InfoDialog(page: InfoPage, open: RwSignal<bool>) -> impl IntoView {
    view! {
        <Show when=move || open.get()>
            <InfoDialogContent page open />
        </Show>
    }
}

#[component]
fn InfoDialogContent(page: InfoPage, open: RwSignal<bool>) -> impl IntoView {
    // `None` while loading; `Some(None)` when it could not be fetched.
    let content = RwSignal::new(None::<Option<String>>);
    spawn_local(async move {
        let html = match page.load().await {
            Ok(markdown) => Some(to_html(&markdown)),
            Err(error) => {
                tauri::log(LogLevel::Warn, format!("{page:?}: {error}"));
                None
            }
        };
        // Closed meanwhile: the signal is gone with the dialog.
        content.try_set(Some(html));
    });
    let body = move || match content.get() {
        None => {
            view! { <p class="loading">{LOCALE.key(MessageKey::InfoDialogLoading)}</p> }.into_any()
        }
        Some(None) => {
            view! { <p class="label">{LOCALE.key(MessageKey::InfoDialogUnavailable)}</p> }
                .into_any()
        }
        Some(Some(html)) => {
            view! { <div class="markdown" inner_html=html on:click=follow_link></div> }.into_any()
        }
    };
    view! {
        <div class="modal-backdrop">
            <section class="dialog info-dialog" role="dialog" aria-modal="true">
                <h2>{LOCALE.key(page.title())}</h2>
                {body}
                <div class="dialog-actions">
                    <button class="primary" on:click=move |_| open.set(false)>
                        {LOCALE.key(MessageKey::InfoDialogClose)}
                    </button>
                </div>
            </section>
        </div>
    }
}

/// A click on a rendered link opens it through the core, which opens only
/// allowlisted hosts; the window itself never navigates.
fn follow_link(ev: web_sys::MouseEvent) {
    let anchor = ev
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .and_then(|element| element.closest("a").ok().flatten());
    if let Some(anchor) = anchor {
        ev.prevent_default();
        if let Some(href) = anchor.get_attribute("href") {
            open_link(&href);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f19_markdown_renders_the_changelog_markup() {
        let html = to_html("# Changelog\n\n## 2.0.0\n\n- **New** `thing`\n- *other*\n");
        assert_eq!(
            html,
            "<h1>Changelog</h1>\n<h2>2.0.0</h2>\n<ul>\n<li><strong>New</strong> \
             <code>thing</code></li>\n<li><em>other</em></li>\n</ul>\n"
        );
    }

    #[test]
    fn f19_markdown_shows_raw_html_as_text() {
        let html = to_html(
            "<script>alert(1)</script>\n\nText <img src=x onerror=alert(1)> and \
             <a href=\"javascript:alert(1)\">x</a>\n\n<div onclick=\"alert(1)\">block</div>\n",
        );
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(!html.contains("<a "), "{html}");
        assert!(!html.contains("<div"), "{html}");
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{html}"
        );
        assert!(
            html.contains("&lt;img src=x onerror=alert(1)&gt;"),
            "{html}"
        );
    }

    #[test]
    fn f19_markdown_keeps_only_https_links() {
        assert_eq!(
            to_html("[issues](https://github.com/exilence-ce/exilence-ce/issues)"),
            "<p><a href=\"https://github.com/exilence-ce/exilence-ce/issues\">issues</a></p>\n"
        );
        for markdown in [
            "[x](javascript:alert(1))",
            "[x](JAVASCRIPT:alert(1))",
            "[x](data:text/html,<script>alert(1)</script>)",
            "[x](http://example.com)",
            "[x](/settings)",
            "[x][r]\n\n[r]: vbscript:msgbox",
        ] {
            assert_eq!(to_html(markdown), "<p>x</p>\n", "{markdown}");
        }
        let html = to_html("<javascript:alert(1)>");
        assert!(!html.contains("href"), "{html}");
        assert_eq!(
            to_html("<https://poe.ninja>"),
            "<p><a href=\"https://poe.ninja\">https://poe.ninja</a></p>\n"
        );
    }

    #[test]
    fn f19_markdown_shows_images_as_their_alt_text() {
        assert_eq!(
            to_html("![a *logo*](https://example.com/track.png)"),
            "<p>a <em>logo</em></p>\n"
        );
        assert_eq!(
            to_html("[![badge](https://img.example/b.svg)](https://github.com/x)"),
            "<p><a href=\"https://github.com/x\">badge</a></p>\n"
        );
    }
}
