//! Gateway HTTP plumbing: origin validation, request logging, error mapping, cookie jar.
//! Credentials are attached only by `AppState::dashboard`/`AppState::api` in main.rs.

use reqwest::{header::HeaderMap, Client, RequestBuilder, Response, Url};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;

/// Keepalives arrive every 10 s, so 15 s without a single byte means the stream is dead.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum Error {
    Unreachable(String),
    Unauthorized(String),
    NotFound(String),
    Http(String),
    Invalid(String),
}

fn builder() -> reqwest::ClientBuilder {
    Client::builder()
        // Never route credentials through a proxy host or follow a redirect off the gateway.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
}

pub fn client() -> Client {
    builder().read_timeout(IDLE_TIMEOUT).build().expect("static client config")
}

/// For a reply the server works on silently (speech-to-text): no idle cutoff, 2 min in total.
pub fn slow_client() -> Client {
    builder().timeout(Duration::from_secs(120)).build().expect("static client config")
}

/// Accepts `http(s)://host[:port]` only: no credentials, path, query, or fragment.
pub fn parse_origin(input: &str) -> Result<Url, Error> {
    let bad = |why: &str| Error::Invalid(format!("{input:?}: {why}"));
    let url = Url::parse(input.trim()).map_err(|e| bad(&e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(bad("scheme must be http or https"));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(bad("missing host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad("must not contain a username or password"));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(bad("must be just scheme, host, and port"));
    }
    Ok(url)
}

/// Joins path segments onto an origin, percent-encoding each one (session ids included), plus query pairs.
pub fn endpoint(origin: &Url, segments: &[&str], query: &[(&str, &str)]) -> Url {
    let mut url = origin.clone();
    url.path_segments_mut().expect("http(s) origin").pop_if_empty().extend(segments);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query); // even an empty extend would leave a bare `?`
    }
    url
}

/// Sends a request, logging method, path, and status only. Non-2xx becomes an `Error`.
pub async fn send(client: &Client, request: RequestBuilder) -> Result<Response, Error> {
    let request = request.build().map_err(|e| Error::Invalid(describe(&e)))?;
    let (method, path) = (request.method().clone(), request.url().path().to_owned());
    let response = client.execute(request).await.map_err(|e| {
        eprintln!("[gateway] {method} {path} -> {}", describe(&e));
        Error::Unreachable(describe(&e))
    })?;
    let status = response.status();
    eprintln!("[gateway] {method} {path} -> {}", status.as_u16());
    if status.is_success() {
        return Ok(response);
    }
    let body: Value = response.json().await.unwrap_or_default();
    let message = body["detail"]
        .as_str()
        .or(body["error"]["message"].as_str())
        .unwrap_or(status.canonical_reason().unwrap_or("error"))
        .to_owned();
    Err(match status.as_u16() {
        401 => Error::Unauthorized(message),
        404 => Error::NotFound(message),
        code => Error::Http(format!("HTTP {code}: {message}")),
    })
}

/// reqwest's top-level message is vague ("error sending request"); the cause chain says why.
pub fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Applies `Set-Cookie` headers to the jar; returns whether anything changed.
pub fn absorb_cookies(jar: &mut BTreeMap<String, String>, headers: &HeaderMap) -> bool {
    let mut changed = false;
    for header in headers.get_all(reqwest::header::SET_COOKIE) {
        let Some((pair, attrs)) = header.to_str().ok().map(|h| h.split_once(';').unwrap_or((h, ""))) else {
            continue;
        };
        let Some((name, value)) = pair.trim().split_once('=') else { continue };
        let cleared = value.is_empty() || attrs.to_ascii_lowercase().replace(' ', "").contains("max-age=0");
        changed |= if cleared {
            jar.remove(name).is_some()
        } else {
            jar.insert(name.to_owned(), value.to_owned()).as_deref() != Some(value)
        };
    }
    changed
}

pub fn cookie_header(jar: &BTreeMap<String, String>) -> String {
    jar.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderValue, SET_COOKIE};

    #[test]
    fn origin_validation() {
        assert_eq!(parse_origin(" http://10.0.0.5:9119 ").unwrap().as_str(), "http://10.0.0.5:9119/");
        assert!(parse_origin("https://hermes.tailnet.ts.net/").is_ok());
        for bad in ["ftp://h", "http://u:p@h:1", "http://h/api", "http://h?x=1", "http://h#f", "h:9119", ""] {
            assert!(parse_origin(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn endpoint_encodes_segments() {
        let base = parse_origin("http://h:8642").unwrap();
        assert_eq!(endpoint(&base, &["api", "sessions", "../x"], &[]).as_str(), "http://h:8642/api/sessions/..%2Fx");
        assert_eq!(endpoint(&base, &["api"], &[("profile", "a b")]).as_str(), "http://h:8642/api?profile=a+b");
    }

    #[test]
    fn cookie_jar_rotates_and_clears() {
        let mut jar = BTreeMap::new();
        let mut headers = HeaderMap::new();
        headers.append(SET_COOKIE, HeaderValue::from_static("hermes_session_at=a1; HttpOnly; Path=/"));
        headers.append(SET_COOKIE, HeaderValue::from_static("hermes_session_rt=r1; HttpOnly"));
        assert!(absorb_cookies(&mut jar, &headers));
        assert_eq!(cookie_header(&jar), "hermes_session_at=a1; hermes_session_rt=r1");
        assert!(!absorb_cookies(&mut jar, &headers));

        let mut rotate = HeaderMap::new();
        rotate.append(SET_COOKIE, HeaderValue::from_static("hermes_session_at=a2; Path=/"));
        rotate.append(SET_COOKIE, HeaderValue::from_static("hermes_session_rt=; Max-Age=0"));
        assert!(absorb_cookies(&mut jar, &rotate));
        assert_eq!(cookie_header(&jar), "hermes_session_at=a2");
    }
}
