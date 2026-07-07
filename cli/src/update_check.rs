//! Background check for newer hmanlab versions on the npm registry.
//!
//! Fires once at startup, caches the result for 24 h, never blocks the
//! UI. On debug builds the check is skipped so `cargo run` doesn't ping
//! the registry on every launch. The check fails silently — a flaky
//! network shouldn't produce visible noise — the user only sees a
//! header notice when a newer version definitely exists.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const REGISTRY_URL: &str = "https://registry.npmjs.org/hmanlab";
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Serialize, Deserialize)]
struct Cache {
    latest: String,
    checked_at: u64,
}

#[derive(Deserialize)]
struct RegistryResponse {
    #[serde(rename = "dist-tags")]
    dist_tags: DistTags,
}

#[derive(Deserialize)]
struct DistTags {
    latest: String,
}

/// Returns `Some(latest)` if npm advertises a version newer than the
/// compiled-in one. Returns `None` for "up to date", "registry down",
/// "network off", or "first launch with no cache yet".
pub async fn check() -> Option<String> {
    if cfg!(debug_assertions) {
        return None;
    }

    let current = env!("CARGO_PKG_VERSION");

    if let Some(cache) = read_cache() {
        let age = now_secs().saturating_sub(cache.checked_at);
        if age < CACHE_TTL_SECS {
            return version_if_newer(current, &cache.latest);
        }
    }

    let latest = fetch_latest().await.ok()?;
    write_cache(&latest);
    version_if_newer(current, &latest)
}

async fn fetch_latest() -> Result<String> {
    fetch_latest_npm().await
}

/// Same registry round-trip as the background check, but exposed so
/// the `/update` and `/settings` commands can run their own fresh
/// look-up without going through the 24 h cache.
pub async fn fetch_latest_npm() -> Result<String> {
    let client = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?;
    let resp: RegistryResponse = client
        .get(REGISTRY_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(resp.dist_tags.latest)
}

/// Public re-export of the internal newer-than check so `/update` can
/// reuse the same semver-tolerant comparison as the background notice.
pub fn newer(current: &str, latest: &str) -> bool {
    cmp_semver(current, latest).is_lt()
}

fn cache_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/hmanlab/update.json"))
}

fn read_cache() -> Option<Cache> {
    let p = cache_path()?;
    let raw = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(latest: &str) {
    let Some(p) = cache_path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let cache = Cache {
        latest: latest.to_string(),
        checked_at: now_secs(),
    };
    if let Ok(s) = serde_json::to_string(&cache) {
        let _ = std::fs::write(p, s);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn version_if_newer(current: &str, latest: &str) -> Option<String> {
    if cmp_semver(current, latest).is_lt() {
        Some(latest.to_string())
    } else {
        None
    }
}

/// Best-effort semver compare. Pre-release suffixes (`-rc.1`) are
/// ignored — a stable release with the same numeric triple is
/// considered equal. Good enough for "is npm newer than me", which is
/// the only question we're asking.
fn cmp_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> (u32, u32, u32) {
        let core = s.split('-').next().unwrap_or(s);
        let mut parts = core.split('.');
        let major = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    };
    parse(a).cmp(&parse(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_minor() {
        assert_eq!(version_if_newer("0.2.0", "0.3.0"), Some("0.3.0".into()));
    }

    #[test]
    fn same_version() {
        assert_eq!(version_if_newer("0.2.0", "0.2.0"), None);
    }

    #[test]
    fn older_npm() {
        assert_eq!(version_if_newer("0.3.0", "0.2.0"), None);
    }

    #[test]
    fn prerelease_treated_as_stable_for_compare() {
        // 0.2.0-rc.1 and 0.2.0 are both "0.2.0" core — equal, no upgrade.
        assert_eq!(version_if_newer("0.2.0-rc.1", "0.2.0"), None);
    }

    #[test]
    fn major_bump() {
        assert_eq!(version_if_newer("0.2.0", "1.0.0"), Some("1.0.0".into()));
    }
}
