---
title: Web Fetch
description: Fetch content from URLs
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
- Respects redirects (up to 10 hops)
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

Fetch raw file content:

```json
{
  "url": "https://raw.githubusercontent.com/pdg-global/rusty/main/Cargo.toml"
}
```

## SSRF Protection

The `web_fetch` tool includes built-in protection against Server-Side Request Forgery (SSRF) attacks:

- **Scheme validation**: Only `http://` and `https://` URLs are accepted. Schemes like `file://`, `ftp://`, `data:`, and `gopher://` are rejected.
- **Hostname blocking**: Requests to `localhost`, `127.0.0.1`, `[::1]`, `metadata.google.internal`, and AWS/GCP/Azure metadata endpoints are blocked.
- **IP range blocking**: Private IP ranges (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, link-local `169.254.0.0/16`) are rejected via DNS resolution.
- **Redirect validation**: Redirects are followed up to 10 hops, with each redirect target validated against the same rules.

## Limitations

- Cannot fetch pages that require JavaScript rendering (single-page applications)
- Does not execute JavaScript; returns raw HTML only
- Some websites block automated requests; these will return an error
- Binary content (images, PDFs) is not supported; the tool returns raw bytes as text which may be garbled
- Large pages are truncated to `max_length` characters

## Use Cases

- Reading official documentation for libraries and frameworks
- Checking the contents of a GitHub repository or file
- Fetching API responses to understand data formats
- Looking up error messages or solutions online
- Verifying URLs and endpoints
