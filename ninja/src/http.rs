//! The app's HTTP client for GGG and poe.ninja, and Debug mode's request
//! lines (F20), logged through `log` under this module's target.

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::HeaderMap;

/// How much of a failed response's body Debug mode logs.
pub const BODY_LIMIT: usize = 4 * 1024;

/// An HTTP client with `user_agent` that trusts the webpki roots, as the
/// backend channel does: poe.ninja's, and the app's GGG client.
pub fn client(user_agent: String) -> Result<reqwest::Client, Box<dyn std::error::Error>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(Duration::from_secs(30))
        .tls_backend_preconfigured(tls)
        .build()?)
}

/// Sends `request` to `service` ("GGG", "poe.ninja"). In Debug mode logs its
/// method, URL, status, duration and rate-limit headers, and the body of a
/// failed response up to [`BODY_LIMIT`] (F20).
pub async fn send(
    service: &str,
    request: reqwest::RequestBuilder,
) -> reqwest::Result<reqwest::Response> {
    if !log::log_enabled!(log::Level::Debug) {
        return request.send().await;
    }
    let (client, request) = request.build_split();
    let request = request?;
    let line = format!("{service} {} {}", request.method(), request.url());
    let started = Instant::now();
    let answered = client.execute(request).await;
    let ms = started.elapsed().as_millis();
    let response = match answered {
        Ok(response) => response,
        Err(error) => {
            log::debug!("{line}: no answer after {ms} ms: {error}");
            return Err(error);
        }
    };
    let status = response.status();
    let limits = rate_limit_headers(response.headers());
    log::debug!("{line}: {status} after {ms} ms{limits}");
    if !status.is_client_error() && !status.is_server_error() {
        return Ok(response);
    }
    // Read here to log it; the caller gets it back with status and headers.
    let (version, headers) = (response.version(), response.headers().clone());
    let body = response.bytes().await?;
    log::debug!("{line}: {status} body: {}", excerpt(&body));
    let mut rebuilt = ::http::Response::new(body);
    *rebuilt.status_mut() = status;
    *rebuilt.version_mut() = version;
    *rebuilt.headers_mut() = headers;
    Ok(rebuilt.into())
}

/// The rate-limit headers among `headers` as ` [name: value, …]`; empty
/// without any.
pub fn rate_limit_headers(headers: &HeaderMap) -> String {
    let found: Vec<String> = headers
        .iter()
        .filter(|(name, _)| {
            let name = name.as_str();
            name.starts_with("x-rate-limit-")
                || name.starts_with("ratelimit")
                || name == "retry-after"
        })
        .map(|(name, value)| format!("{name}: {}", String::from_utf8_lossy(value.as_bytes())))
        .collect();
    if found.is_empty() {
        String::new()
    } else {
        format!(" [{}]", found.join(", "))
    }
}

/// At most [`BODY_LIMIT`] bytes of `body` as text, noting a cut.
pub fn excerpt(body: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(&body[..body.len().min(BODY_LIMIT)]).into_owned();
    if body.len() > BODY_LIMIT {
        text.push_str(&format!(" … (cut at {BODY_LIMIT} of {} bytes)", body.len()));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f20_failed_bodies_are_cut_at_4_kb() {
        assert_eq!(excerpt(b"refused"), "refused");
        let body = "x".repeat(BODY_LIMIT + 10);
        let cut = excerpt(body.as_bytes());
        assert!(cut.starts_with(&"x".repeat(BODY_LIMIT)));
        assert!(!cut.starts_with(&"x".repeat(BODY_LIMIT + 1)));
        assert!(cut.ends_with("(cut at 4096 of 4106 bytes)"), "{cut}");
    }

    #[test]
    fn f20_rate_limit_headers_are_picked_out() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-rate-limit-policy",
            "stash-request-limit".parse().unwrap(),
        );
        headers.insert("retry-after", "60".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());
        let found = rate_limit_headers(&headers);
        assert!(
            found.contains("x-rate-limit-policy: stash-request-limit"),
            "{found}"
        );
        assert!(found.contains("retry-after: 60"), "{found}");
        assert!(!found.contains("content-type"), "{found}");
        assert_eq!(rate_limit_headers(&HeaderMap::new()), "");
    }
}
