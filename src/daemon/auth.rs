//! The access token and the checks that keep other web pages and other machines out.
//!
//! A web page in the user's browser can send requests to `127.0.0.1` too, so listening on the loopback interface
//! is not protection by itself. Every request must therefore carry the secret token (which a malicious page does not
//! know) and, when it comes from a browser, an `Origin` that is the daemon's own address (or one the user allowed).

use axum::http::{header, HeaderMap};

/// A new random token: 256 bits, hexadecimal.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system has no random source");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compares two secrets in time that does not depend on where they differ.
pub fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

/// The token a request carries: `Authorization: Bearer <token>`, or a `token=` query parameter (a browser cannot
/// set headers on a WebSocket).
pub fn presented_token(headers: &HeaderMap, query: Option<&str>) -> Option<String> {
    if let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(t) = v.strip_prefix("Bearer ") {
            return Some(t.trim().to_owned());
        }
    }
    query?
        .split('&')
        .find_map(|kv| kv.strip_prefix("token="))
        .map(|t| t.to_owned())
}

/// Whether a browser request comes from an allowed page. Requests without an `Origin` (the command line, scripts)
/// are not browser requests and pass; a browser always sends one on cross-site requests and WebSockets.
pub fn origin_allowed(headers: &HeaderMap, extra_allowed: &[String]) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    if extra_allowed
        .iter()
        .any(|a| a.trim_end_matches('/') == origin.trim_end_matches('/'))
    {
        return true;
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    match (origin.split_once("://"), host) {
        (Some((_, origin_host)), Some(host)) => origin_host.eq_ignore_ascii_case(host),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn tokens_are_long_and_different() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn secret_comparison_is_exact() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "abcd"));
        assert!(!same_secret("", "x"));
        assert!(same_secret("", ""));
    }

    #[test]
    fn the_token_comes_from_the_header_or_the_query() {
        let h = headers(&[("authorization", "Bearer sekret")]);
        assert_eq!(presented_token(&h, None).as_deref(), Some("sekret"));
        assert_eq!(
            presented_token(&HeaderMap::new(), Some("a=1&token=xyz")).as_deref(),
            Some("xyz")
        );
        assert_eq!(presented_token(&HeaderMap::new(), Some("a=1")), None);
        assert_eq!(presented_token(&HeaderMap::new(), None), None);
    }

    #[test]
    fn origins() {
        let host = "127.0.0.1:7650";
        assert!(
            origin_allowed(&headers(&[("host", host)]), &[]),
            "no origin: not a browser"
        );
        assert!(origin_allowed(
            &headers(&[("host", host), ("origin", "http://127.0.0.1:7650")]),
            &[]
        ));
        assert!(!origin_allowed(
            &headers(&[("host", host), ("origin", "https://evil.example")]),
            &[]
        ));
        assert!(!origin_allowed(
            &headers(&[("host", host), ("origin", "http://127.0.0.1:9999")]),
            &[]
        ));
        assert!(origin_allowed(
            &headers(&[("host", host), ("origin", "http://localhost:5173")]),
            &["http://localhost:5173/".to_string()]
        ));
        assert!(
            !origin_allowed(&headers(&[("origin", "http://x")]), &[]),
            "origin without host"
        );
    }
}
