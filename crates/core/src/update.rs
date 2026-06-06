use serde::Deserialize;

const GITHUB_API_URL: &str = "https://api.github.com/repos/PDG-Global/rusty/releases/latest";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

/// Result of checking for updates against the latest GitHub release.
#[derive(Debug, Clone)]
pub struct UpdateCheckResult {
    /// The newer version string (without `v` prefix) if an update is available.
    pub latest_version: String,
}

/// The current version of rusty, from the Cargo.toml manifest.
pub fn current_version() -> &'static str {
    CURRENT_VERSION
}

/// Check whether a newer release exists on GitHub.
///
/// Returns `Ok(Some(result))` if an update is available, `Ok(None)` if the
/// current version is up-to-date, or `Err` if the check could not be
/// completed (network error, timeout, parse failure). All errors are
/// non-fatal: callers should treat them as "unknown" rather than failures.
pub async fn check_for_update() -> Result<Option<UpdateCheckResult>, crate::RustyError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .user_agent(super::rusty_user_agent())
        .build()
        .map_err(|e| crate::RustyError::Other(format!("Failed to build HTTP client: {e}")))?;

    let resp = client
        .get(GITHUB_API_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| crate::RustyError::Other(format!("Update check request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(crate::RustyError::Other(format!(
            "Update check returned status {}",
            resp.status()
        )));
    }

    let release: GitHubRelease = resp
        .json()
        .await
        .map_err(|e| crate::RustyError::Other(format!("Failed to parse release info: {e}")))?;

    let latest = release.tag_name.trim_start_matches('v').to_string();

    if version_is_newer(&latest, CURRENT_VERSION) {
        Ok(Some(UpdateCheckResult {
            latest_version: latest,
        }))
    } else {
        Ok(None)
    }
}

/// Compare two semver-like version strings.
///
/// Returns `true` when `latest` is strictly newer than `current`.
/// Only compares numeric components (`major.minor.patch`); pre-release
/// suffixes are ignored for simplicity.
fn version_is_newer(latest: &str, current: &str) -> bool {
    let latest_parts = parse_version(latest);
    let current_parts = parse_version(current);

    for (l, c) in latest_parts.iter().zip(current_parts.iter()) {
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }
    // If all compared parts are equal, the version with more components wins
    // (e.g. 0.1.6 > 0.1).
    latest_parts.len() > current_parts.len()
}

/// Parse a version string like `0.1.6` into a vector of numeric components.
fn parse_version(s: &str) -> Vec<u64> {
    s.split('.')
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_is_newer() {
        assert!(version_is_newer("0.2.0", "0.1.0"));
        assert!(version_is_newer("0.1.6", "0.1.5"));
        assert!(version_is_newer("1.0.0", "0.9.9"));
        assert!(!version_is_newer("0.1.0", "0.1.0"));
        assert!(!version_is_newer("0.1.0", "0.1.6"));
        assert!(!version_is_newer("0.1.0", "0.2.0"));
        // Extra components
        assert!(version_is_newer("0.1.0.1", "0.1.0"));
        assert!(!version_is_newer("0.1.0", "0.1.0.1"));
    }

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("0.1.6"), vec![0, 1, 6]);
        assert_eq!(parse_version("1.2.3.4"), vec![1, 2, 3, 4]);
        assert_eq!(parse_version("10"), vec![10]);
    }
}
