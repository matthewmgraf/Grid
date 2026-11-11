//! Update checks against GitHub Releases: reading the "latest release" answer and deciding
//! whether it is newer than this build. Downloading is up to the platform layer.

use serde::Deserialize;

/// GitHub's API address for the newest published (non-draft, non-prerelease) release.
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/matthewmgraf/Grid/releases/latest";

/// File name of the program in a release's downloads.
const ASSET_NAME: &str = "grid.exe";

/// A release that can be installed.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// e.g. "0.2.0".
    pub version: String,
    /// Direct download of `grid.exe`.
    pub download_url: String,
    /// Size of `grid.exe` in bytes, to check the download is complete.
    pub size: u64,
    /// The release's page on GitHub, for release notes.
    pub page_url: String,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Reads GitHub's answer for the latest release. Errors if it isn't a release with a `grid.exe`.
pub fn parse_latest(json: &str) -> Result<Release, String> {
    let api: ApiRelease = serde_json::from_str(json).map_err(|e| format!("Unexpected answer from GitHub: {e}"))?;
    let asset = api
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(ASSET_NAME))
        .ok_or_else(|| format!("The latest release ({}) has no {ASSET_NAME} to download.", api.tag_name))?;
    Ok(Release {
        version: api.tag_name.trim_start_matches(['v', 'V']).to_string(),
        download_url: asset.browser_download_url.clone(),
        size: asset.size,
        page_url: api.html_url,
    })
}

/// Whether version `candidate` (e.g. "0.10.0") is newer than `current` (e.g. "0.9.2").
/// Compares the dot-separated numbers in order; anything after a `-` (e.g. "-beta") is ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let numbers = |v: &str| -> Vec<u64> {
        v.trim_start_matches(['v', 'V'])
            .split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|n| n.trim().parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (numbers(candidate), numbers(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.2.0-beta", "0.2.0"));
    }

    #[test]
    fn reads_a_github_release() {
        let json = r#"{
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/matthewmgraf/Grid/releases/tag/v0.2.0",
            "assets": [
                { "name": "notes.txt", "browser_download_url": "https://example.invalid/notes.txt", "size": 10 },
                { "name": "grid.exe", "browser_download_url": "https://example.invalid/grid.exe", "size": 9500000 }
            ]
        }"#;
        let r = parse_latest(json).unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.download_url, "https://example.invalid/grid.exe");
        assert_eq!(r.size, 9_500_000);
    }

    #[test]
    fn release_without_exe_is_an_error() {
        let json = r#"{ "tag_name": "v0.2.0", "html_url": "x", "assets": [] }"#;
        assert!(parse_latest(json).unwrap_err().contains("grid.exe"));
    }
}
