---
title: Web Fetch
description: Fetch content from URLs with SSRF protection
---


## Overview

The `web_fetch` tool retrieves content from a URL and returns it as text. Useful for reading documentation, checking API responses, or fetching web content during a conversation.

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | Yes | The URL to fetch |
| `max_length` | integer | No | Maximum characters to return (default: 10,000) |

## Features

- Uses `reqwest` HTTP client with a 30-second timeout
- Returns the response body as plain text
- Supports HTTP and HTTPS
- Respects redirects (up to 10 hops, each re-validated)
- Truncates output to `max_length` to keep responses manageable

## Examples

Fetch a documentation page:

```json
{
  "url": "https://doc.rust-lang.org/std/vec/struct.Vec.html"
}
```

Fetch an API response with a limit:

```json
{
  "url": "https://api.github.com/repos/pdg-global/rusty/releases/latest",
  "max_length": 5000
}
```

## SSRF Protection

The `web_fetch` tool includes comprehensive protection against Server-Side Request Forgery (SSRF) attacks with multiple layers:

### Scheme Restriction

Only `http` and `https` schemes are allowed. Schemes like `file://`, `ftp://`, `data:`, and `gopher://` are rejected.

### Hostname Blocklist

Rejects `localhost`, `*.localhost`, `*.local`, and variants.

### IP Blocklist

Blocks the following address ranges:

- **Loopback**: `127.0.0.0/8`, `::1`
- **Private**: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `fc00::/7`
- **Link-local**: `169.254.0.0/16`, `fe80::/10`
- **CGNAT**: `100.64.0.0/10`
- **TEST-NET** ranges and multicast addresses

### DNS Rebinding Prevention

The tool resolves the hostname, pins the resolved IP, and rejects requests where DNS resolves to a private or blocklisted address. It builds a per-request client with a `resolve()` override to prevent DNS rebinding attacks.

### Redirect Validation

Every redirect target is re-checked against the full blocklist before following. This prevents attackers from redirecting to internal services after an initial valid request.

## Limitations

- Cannot fetch pages that require JavaScript rendering (single-page applications)
- Does not execute JavaScript; returns raw HTML only
- Some websites block automated requests; these will return an error
- Binary content (images, PDFs) is not supported; the tool returns raw bytes as text which may be garbled
- Large pages are truncated to `max_length` characters
