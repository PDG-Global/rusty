// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use futures::StreamExt;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

const MAX_REDIRECTS: usize = 10;
const MAX_BODY_BYTES: u64 = 50 * 1024 * 1024; // 50 MB hard limit

pub struct WebFetchTool;

impl Default for WebFetchTool {
    fn default() -> Self {
        Self
    }
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }
}

// ── SSRF helpers ──────────────────────────────────────────────────────

/// Validate URL scheme and hostname against SSRF rules.
/// Returns `Ok(())` if the URL passes all checks, `Err(message)` if blocked.
fn validate_scheme_and_host(url: &reqwest::Url) -> Result<(), String> {
    // Scheme check: only allow http and https
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "Blocked URL scheme '{other}'. Only http and https are allowed."
            ));
        }
    }

    // Hostname check: block localhost, local domains, and RFC 2606 reserved TLDs
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    let host_lower = host.to_ascii_lowercase();
    if host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".local")
        || host_lower.ends_with(".localdomain")
        // RFC 2606 reserved TLDs: .test, .example, .invalid
        || host_lower.ends_with(".test")
        || host_lower.ends_with(".example")
        || host_lower.ends_with(".invalid")
    {
        return Err(format!("Blocked request to local host '{host}'."));
    }

    // Literal IP check: block private, loopback, link-local, metadata
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(format!("Blocked request to private/reserved IP {ip}."));
        }
    }

    Ok(())
}

/// Resolve DNS for the URL host, validate all resolved IPs, and build a
/// pinned client that overrides DNS with the validated address.
///
/// This is the core DNS-rebinding defence: the client's `resolve()` maps the
/// hostname to the exact IP we validated, so a subsequent DNS change by an
/// attacker cannot redirect the actual TCP connection.
///
/// Returns `Err(message)` if DNS fails or any resolved IP is blocked
/// (fail-closed).
async fn resolve_and_pin(url: &reqwest::Url) -> Result<reqwest::Client, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let port = url.port_or_known_default().unwrap_or(443);

    // Fail closed: if DNS resolution fails, block the request entirely.
    let addrs = tokio::net::lookup_host((host, port)).await.map_err(|e| {
        format!("DNS resolution failed for host '{host}': {e}. Request blocked for safety.")
    })?;

    let mut pinned_ip: Option<std::net::IpAddr> = None;
    for addr in addrs {
        if is_blocked_ip(addr.ip()) {
            return Err(format!(
                "Blocked: host '{host}' resolves to private/reserved IP {}.",
                addr.ip()
            ));
        }
        if pinned_ip.is_none() {
            pinned_ip = Some(addr.ip());
        }
    }

    let ip = pinned_ip.ok_or_else(|| {
        format!("DNS resolution for '{host}' returned no addresses.")
    })?;

    // Build a per-request client with pinned DNS to prevent DNS rebinding.
    let socket_addr = std::net::SocketAddr::new(ip, port);
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .resolve(host, socket_addr)
        .user_agent(rusty_core::rusty_user_agent())
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// Check if a status code is a redirect that should be followed.
fn is_redirect_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

// ── Tool implementation ───────────────────────────────────────────────

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL. Returns the response body as text. Use for reading documentation, API responses, or web pages."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch"
                },
                "max_length": {
                    "type": "integer",
                    "description": "Maximum characters to return (default: 10000)"
                }
            },
            "required": ["url"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let url_str = input["url"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'url' parameter".into()))?;
        let max_length = input["max_length"].as_u64().unwrap_or(10000) as usize;

        // ── Parse and validate initial URL ────────────────────────────
        let url = reqwest::Url::parse(url_str)
            .map_err(|e| RustyError::Tool(format!("Invalid URL: {e}")))?;

        if let Err(msg) = validate_scheme_and_host(&url) {
            return Ok(ToolResult::error(msg));
        }

        // ── Manual redirect loop with full SSRF checks per hop ───────
        // We disable reqwest's automatic redirects because the sync
        // redirect policy cannot perform async DNS resolution, leaving
        // redirect targets unchecked against DNS-rebinding attacks.
        let mut current_url = url;
        let mut redirects: usize = 0;

        loop {
            // DNS resolution + IP pinning (fail-closed on DNS failure)
            let client = match resolve_and_pin(&current_url).await {
                Ok(c) => c,
                Err(msg) => return Ok(ToolResult::error(msg)),
            };

            debug!("Fetching URL: {current_url}");

            let resp = client
                .get(current_url.as_str())
                .send()
                .await
                .map_err(|e| RustyError::Tool(format!("Failed to fetch URL: {e}")))?;

            // ── Handle redirects manually ────────────────────────────
            if is_redirect_status(resp.status()) {
                if redirects >= MAX_REDIRECTS {
                    return Err(RustyError::Tool("Too many redirects".into()));
                }

                let location = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        RustyError::Tool("Redirect without Location header".into())
                    })?;

                current_url = current_url.join(location).map_err(|e| {
                    RustyError::Tool(format!("Invalid redirect URL: {e}"))
                })?;

                // Validate redirect target (scheme, hostname, literal IP).
                // DNS resolution happens at the top of the next iteration.
                if let Err(msg) = validate_scheme_and_host(&current_url) {
                    return Ok(ToolResult::error(msg));
                }

                redirects += 1;
                continue;
            }

            // ── Non-redirect response: process it ────────────────────
            let status = resp.status();
            if !status.is_success() {
                return Ok(ToolResult::error(format!("HTTP {status}: {current_url}")));
            }

            // Content-Length pre-check: reject obviously oversized bodies
            // before reading any data.
            if let Some(len) = resp.content_length() {
                if len > MAX_BODY_BYTES {
                    return Ok(ToolResult::error(format!(
                        "Response body too large ({len} bytes, limit is {MAX_BODY_BYTES})"
                    )));
                }
            }

            // Streaming body read with hard size cap.
            // We read up to max_length * 2 bytes (to allow clean truncation)
            // but never exceed MAX_BODY_BYTES total.
            let mut stream = resp.bytes_stream();
            let mut body = Vec::with_capacity(max_length.min(1_000_000));
            let mut total_bytes: usize = 0;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| {
                    RustyError::Tool(format!("Failed reading response: {e}"))
                })?;
                total_bytes += chunk.len();
                if total_bytes > MAX_BODY_BYTES as usize {
                    return Ok(ToolResult::error(
                        "Response body exceeds maximum size limit",
                    ));
                }
                body.extend_from_slice(&chunk);
                // Once we have enough for truncation, stop reading.
                if body.len() >= max_length * 2 {
                    break;
                }
            }

            let body = String::from_utf8_lossy(&body).to_string();

            let truncated = if body.len() > max_length {
                // Floor to a valid UTF-8 char boundary to avoid panicking on
                // multi-byte characters (CJK, emoji, accented, etc.).
                let safe = body.floor_char_boundary(max_length);
                format!(
                    "{}...\n\n[Truncated: showing {} of {} chars]",
                    &body[..safe],
                    safe,
                    body.len()
                )
            } else {
                body
            };

            return Ok(ToolResult::success(format!(
                "URL: {current_url}\nStatus: {status}\n\n{truncated}"
            )));
        }
    }
}

// ── IP blocklist ──────────────────────────────────────────────────────

/// Check if an IP address is in a blocked range (private, loopback,
/// link-local, metadata, benchmarking, 6to4 relay, multicast).
fn is_blocked_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()                              // 127.0.0.0/8
            || v4.is_private()                            // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            || v4.is_link_local()                         // 169.254.0.0/16
            || v4.is_unspecified()                        // 0.0.0.0
            || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)  // 100.64.0.0/10 (CGNAT)
            || (v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 2)   // 192.0.2.0/24 (TEST-NET-1)
            || (v4.octets()[0] == 198 && v4.octets()[1] == 51 && v4.octets()[2] == 100) // 198.51.100.0/24 (TEST-NET-2)
            || (v4.octets()[0] == 203 && v4.octets()[1] == 0 && v4.octets()[2] == 113)  // 203.0.113.0/24 (TEST-NET-3)
            || (v4.octets()[0] == 198 && (v4.octets()[1] & 0xFE) == 18)  // 198.18.0.0/15 (benchmarking)
            || (v4.octets()[0] == 192 && v4.octets()[1] == 88 && v4.octets()[2] == 99)  // 192.88.99.0/24 (6to4 relay)
            || v4.octets()[0] >= 224                                               // 224.0.0.0/4 (multicast)
        }
        std::net::IpAddr::V6(v6) => {
            // IPv4-mapped IPv6 addresses (::ffff:0:0/96) must be checked
            // against the IPv4 rules.  Without this, ::ffff:127.0.0.1 and
            // ::ffff:10.0.0.1 bypass all IPv4 blocklist checks.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip(std::net::IpAddr::V4(v4));
            }
            // IPv4-compatible IPv6 addresses (::a.b.c.d, now deprecated) are
            // also decoded and checked against IPv4 rules so that ::127.0.0.1
            // cannot bypass the blocklist.
            let segments = v6.segments();
            // IPv4-compatible addresses (::a.b.c.d) are deprecated but still parsed
            // by some stacks. Decode them and run the IPv4 blocklist, but skip ::1
            // which is the canonical IPv6 loopback and is handled below.
            if segments[0..6].iter().all(|&s| s == 0)
                && segments[6] != 0xFFFF
                && segments[6] != 0
            {
                let v4 = std::net::Ipv4Addr::new(
                    (segments[6] >> 8) as u8,
                    (segments[6] & 0xFF) as u8,
                    (segments[7] >> 8) as u8,
                    (segments[7] & 0xFF) as u8,
                );
                return is_blocked_ip(std::net::IpAddr::V4(v4));
            }
            v6.is_loopback()       // ::1
            || v6.is_unspecified() // ::
            || {
                (segments[0] & 0xFFC0) == 0xFE80  // fe80::/10 (link-local)
                || (segments[0] & 0xFE00) == 0xFC00 // fc00::/7 (unique local)
                || (segments[0] & 0xFF00) == 0xFF00 // ff00::/8 (multicast)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── IPv4 tests ───────────────────────────────────────────────────

    #[test]
    fn test_blocked_ipv4_loopback() {
        assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("127.0.0.2".parse().unwrap()));
        assert!(is_blocked_ip("127.255.255.255".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_private() {
        assert!(is_blocked_ip("10.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("172.16.0.1".parse().unwrap()));
        assert!(is_blocked_ip("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_link_local() {
        assert!(is_blocked_ip("169.254.1.1".parse().unwrap()));
        assert!(is_blocked_ip("169.254.169.254".parse().unwrap())); // cloud metadata
    }

    #[test]
    fn test_blocked_ipv4_unspecified() {
        assert!(is_blocked_ip("0.0.0.0".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_cgnat() {
        assert!(is_blocked_ip("100.64.0.1".parse().unwrap()));
        assert!(is_blocked_ip("100.127.255.255".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_test_nets() {
        assert!(is_blocked_ip("192.0.2.1".parse().unwrap())); // TEST-NET-1
        assert!(is_blocked_ip("198.51.100.1".parse().unwrap())); // TEST-NET-2
        assert!(is_blocked_ip("203.0.113.1".parse().unwrap())); // TEST-NET-3
    }

    #[test]
    fn test_blocked_ipv4_benchmarking() {
        assert!(is_blocked_ip("198.18.0.1".parse().unwrap())); // 198.18.0.0/15
        assert!(is_blocked_ip("198.18.255.255".parse().unwrap()));
        assert!(is_blocked_ip("198.19.255.255".parse().unwrap()));
        // Neighbours outside the range should be allowed
        assert!(!is_blocked_ip("198.17.255.255".parse().unwrap()));
        assert!(!is_blocked_ip("198.20.0.1".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_6to4_relay() {
        assert!(is_blocked_ip("192.88.99.1".parse().unwrap())); // 192.88.99.0/24
        assert!(is_blocked_ip("192.88.99.254".parse().unwrap()));
        // Neighbours outside /24 should be allowed
        assert!(!is_blocked_ip("192.88.98.1".parse().unwrap()));
        assert!(!is_blocked_ip("192.88.100.1".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_multicast() {
        assert!(is_blocked_ip("224.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("239.255.255.255".parse().unwrap()));
    }

    #[test]
    fn test_allowed_ipv4_public() {
        assert!(!is_blocked_ip("8.8.8.8".parse().unwrap())); // Google DNS
        assert!(!is_blocked_ip("1.1.1.1".parse().unwrap())); // Cloudflare DNS
        assert!(!is_blocked_ip("93.184.216.34".parse().unwrap())); // example.com
    }

    // ── IPv6 tests ───────────────────────────────────────────────────

    #[test]
    fn test_blocked_ipv6_loopback() {
        assert!(is_blocked_ip("::1".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv6_unspecified() {
        assert!(is_blocked_ip("::".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv6_link_local() {
        assert!(is_blocked_ip("fe80::1".parse().unwrap()));
        assert!(is_blocked_ip("fe80::abcd:1234".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv6_unique_local() {
        assert!(is_blocked_ip("fc00::1".parse().unwrap()));
        assert!(is_blocked_ip("fd00::1".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_mapped_ipv6_loopback() {
        assert!(is_blocked_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("::ffff:127.255.255.255".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_mapped_ipv6_private() {
        assert!(is_blocked_ip("::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("::ffff:172.16.0.1".parse().unwrap()));
        assert!(is_blocked_ip("::ffff:192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_mapped_ipv6_link_local() {
        assert!(is_blocked_ip("::ffff:169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_mapped_ipv6_unspecified() {
        assert!(is_blocked_ip("::ffff:0.0.0.0".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_mapped_ipv6_multicast() {
        assert!(is_blocked_ip("::ffff:224.0.0.1".parse().unwrap()));
    }

    #[test]
    fn test_allowed_ipv4_mapped_ipv6_public() {
        assert!(!is_blocked_ip("::ffff:8.8.8.8".parse().unwrap())); // Google DNS
        assert!(!is_blocked_ip("::ffff:1.1.1.1".parse().unwrap())); // Cloudflare
        assert!(!is_blocked_ip("::ffff:93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn test_blocked_ipv4_compatible_ipv6() {
        // Deprecated IPv4-compatible notation (::a.b.c.d/96) must still be
        // checked against IPv4 blocklist rules.
        assert!(is_blocked_ip("::127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("::10.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("::192.168.1.1".parse().unwrap()));
        assert!(is_blocked_ip("::169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn test_allowed_ipv4_compatible_ipv6_public() {
        assert!(!is_blocked_ip("::8.8.8.8".parse().unwrap()));
        assert!(!is_blocked_ip("::1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn test_allowed_ipv6_public() {
        assert!(!is_blocked_ip("2606:4700:4700::1111".parse().unwrap())); // Cloudflare
    }

    // ── Scheme and hostname tests ────────────────────────────────────

    #[test]
    fn test_blocked_scheme() {
        let url = reqwest::Url::parse("file:///etc/passwd").unwrap();
        assert!(validate_scheme_and_host(&url).is_err());
    }

    #[test]
    fn test_localhost_hostnames() {
        let cases = vec![
            "localhost",
            "sub.localhost",
            "myhost.local",
            "myhost.localdomain",
            "sub.myhost.localdomain",
            // RFC 2606 reserved TLDs
            "myhost.test",
            "sub.myhost.test",
            "myhost.example",
            "myhost.invalid",
        ];
        for host in cases {
            let url = reqwest::Url::parse(&format!("https://{host}/")).unwrap();
            assert!(
                validate_scheme_and_host(&url).is_err(),
                "Expected '{host}' to be blocked"
            );
        }
    }

    #[test]
    fn test_allowed_hostnames() {
        let cases = vec![
            "example.com",
            "docs.rust-lang.org",
            "api.github.com",
            "myserver.localnet",  // not .local
            "localhost.com",      // not localhost itself
            "testing.example.com", // .example but this is a real domain
        ];
        for host in cases {
            let url = reqwest::Url::parse(&format!("https://{host}/")).unwrap();
            assert!(
                validate_scheme_and_host(&url).is_ok(),
                "Expected '{host}' to be allowed"
            );
        }
    }
}
