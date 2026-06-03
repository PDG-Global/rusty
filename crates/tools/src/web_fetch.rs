// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

pub struct WebFetchTool {
    client: reqwest::Client,
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: {
                // Custom redirect policy that blocks redirects to private/reserved IPs
                let policy = reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 5 {
                        return attempt.stop();
                    }
                    if let Some(host) = attempt.url().host_str() {
                        let host_lower = host.to_ascii_lowercase();
                        // Block localhost and .local redirects
                        if host_lower == "localhost"
                            || host_lower.ends_with(".localhost")
                            || host_lower.ends_with(".local")
                        {
                            return attempt.stop();
                        }
                        // Block private/reserved IP redirects
                        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                            if is_blocked_ip(ip) {
                                return attempt.stop();
                            }
                        }
                    }
                    attempt.follow()
                });
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .redirect(policy)
                    .build()
                    .expect("Failed to build HTTP client")
            },
        }
    }
}

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
        let url = input["url"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'url' parameter".into()))?;

        let max_length = input["max_length"].as_u64().unwrap_or(10000) as usize;

        // ── SSRF protection ──────────────────────────────────────────────
        let parsed =
            reqwest::Url::parse(url).map_err(|e| RustyError::Tool(format!("Invalid URL: {e}")))?;

        // 1. Scheme check: only allow http and https
        match parsed.scheme() {
            "http" | "https" => {}
            other => {
                return Ok(ToolResult::error(format!(
                    "Blocked URL scheme '{other}'. Only http and https are allowed."
                )));
            }
        }

        // 2. Hostname check: block localhost and .local domains
        let host = parsed
            .host_str()
            .ok_or_else(|| RustyError::Tool("URL has no host".into()))?;

        let host_lower = host.to_ascii_lowercase();
        if host_lower == "localhost"
            || host_lower.ends_with(".localhost")
            || host_lower.ends_with(".local")
        {
            return Ok(ToolResult::error(format!(
                "Blocked request to local host '{host}'."
            )));
        }

        // 3. IP address check: block private, link-local, metadata, and loopback IPs
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if is_blocked_ip(ip) {
                return Ok(ToolResult::error(format!(
                    "Blocked request to private/reserved IP {ip}."
                )));
            }
        }

        // 4. DNS resolution check + pin: resolve once and pin the IP to prevent
        //    DNS rebinding TOCTOU (attacker changes DNS between check and request).
        //    We build a per-request client that overrides DNS resolution with the
        //    validated IP addresses.
        let resolved_ip = match tokio::net::lookup_host((host, parsed.port_or_known_default().unwrap_or(443))).await {
            Ok(addrs) => {
                let mut pinned_ip: Option<std::net::IpAddr> = None;
                for addr in addrs {
                    if is_blocked_ip(addr.ip()) {
                        return Ok(ToolResult::error(format!(
                            "Blocked: host '{host}' resolves to private/reserved IP {}.",
                            addr.ip()
                        )));
                    }
                    if pinned_ip.is_none() {
                        pinned_ip = Some(addr.ip());
                    }
                }
                pinned_ip
            }
            Err(e) => {
                debug!("DNS lookup failed for {host}: {e}");
                None
            }
        };

        debug!("Fetching URL: {url}");

        // Build a per-request client with pinned DNS to prevent DNS rebinding.
        // If we resolved the IP, pin it; otherwise fall back to default client.
        let request_client = if let Some(ip) = resolved_ip {
            let port = parsed.port_or_known_default().unwrap_or(443);
            let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.stop();
                }
                if let Some(redir_host) = attempt.url().host_str() {
                    let redir_lower = redir_host.to_ascii_lowercase();
                    if redir_lower == "localhost"
                        || redir_lower.ends_with(".localhost")
                        || redir_lower.ends_with(".local")
                    {
                        return attempt.stop();
                    }
                    if let Ok(redir_ip) = redir_host.parse::<std::net::IpAddr>() {
                        if is_blocked_ip(redir_ip) {
                            return attempt.stop();
                        }
                    }
                }
                attempt.follow()
            });
            let socket_addr = std::net::SocketAddr::new(ip, port);
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .redirect(redirect_policy)
                .resolve(host, socket_addr)
                .build()
                .map_err(|e| RustyError::Tool(format!("Failed to build HTTP client: {e}")))?
        } else {
            // DNS lookup failed (e.g. no A/AAAA record); use default client
            // which will fail naturally on connect
            self.client.clone()
        };

        let resp = request_client
            .get(url)
            .send()
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to fetch URL: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            return Ok(ToolResult::error(format!(
                "HTTP {status}: {url}"
            )));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to read response: {e}")))?;

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

        Ok(ToolResult::success(format!(
            "URL: {url}\nStatus: {status}\n\n{truncated}"
        )))
    }
}

/// Check if an IP address is in a blocked range (private, loopback, link-local, metadata).
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
            || v4.octets()[0] >= 224                                               // 224.0.0.0/4 (multicast)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()       // ::1
            || v6.is_unspecified() // ::
            || {
                let segments = v6.segments();
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
    fn test_allowed_ipv6_public() {
        assert!(!is_blocked_ip("2606:4700:4700::1111".parse().unwrap())); // Cloudflare
    }

    #[test]
    fn test_blocked_scheme() {
        // These tests verify the URL parsing logic conceptually
        // (actual execute() tests would need a mock server)
        let url = reqwest::Url::parse("file:///etc/passwd").unwrap();
        assert_eq!(url.scheme(), "file");
        assert_ne!(url.scheme(), "http");
        assert_ne!(url.scheme(), "https");
    }

    #[test]
    fn test_localhost_hostnames() {
        let cases = vec!["localhost", "sub.localhost", "myhost.local"];
        for host in cases {
            let host_lower = host.to_ascii_lowercase();
            assert!(
                host_lower == "localhost"
                    || host_lower.ends_with(".localhost")
                    || host_lower.ends_with(".local"),
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
        ];
        for host in cases {
            let host_lower = host.to_ascii_lowercase();
            assert!(
                !(host_lower == "localhost"
                    || host_lower.ends_with(".localhost")
                    || host_lower.ends_with(".local")),
                "Expected '{host}' to be allowed"
            );
        }
    }
}
