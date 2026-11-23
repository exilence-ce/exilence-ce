//! Opening external links in the system browser (F15). Only the core opens
//! links, and only to allowlisted hosts; the webview has no shell or opener
//! plugin.

use tauri::Url;

/// Hosts `open_link` opens, over HTTPS only: ko-fi and Discord (F15),
/// poe.ninja's item pages (F11, D3), and the client repository's issue
/// templates and the links in What's New and Credits (F19, D11).
pub const ALLOWED_HOSTS: &[&str] = &["ko-fi.com", "discord.gg", "poe.ninja", "github.com"];

/// `target` as a URL if it is an HTTPS link to an allowlisted host, without
/// credentials or a port.
pub fn allowed(target: &str) -> Option<Url> {
    let url = Url::parse(target).ok()?;
    let allowed = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url
            .host_str()
            .is_some_and(|host| ALLOWED_HOSTS.contains(&host));
    allowed.then_some(url)
}

/// Opens `target` in the system browser if it is [`allowed`].
pub fn open(target: &str) -> Result<(), String> {
    let url = allowed(target).ok_or_else(|| format!("link not allowlisted: {target}"))?;
    open::that_detached(url.as_str()).map_err(|e| format!("cannot open {url}: {e}"))
}
