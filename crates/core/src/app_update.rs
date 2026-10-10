//! Checks whether a newer CPMX2077 release is out on GitHub. Notify only:
//! nothing is downloaded or replaced, the user gets the release page.

use std::cmp::Ordering;

use serde::Serialize;

use crate::sources::github::{Client, Release};
use crate::{APP_VERSION, Error, Result};

/// Where CPMX2077 releases are published. GitHub redirects the API to a
/// repository's new name after a rename or transfer.
pub const REPO_OWNER: &str = "Higher-dimentionalSemanticsFanatic";
pub const REPO_NAME: &str = "LinuxCPMX2077";

/// Setting: "0" turns off the check on startup (on by default).
pub const STARTUP_SETTING: &str = "app_update_on_startup";
/// Setting: the release tag whose startup notice was dismissed.
pub const DISMISSED_SETTING: &str = "app_update_dismissed";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Latest {
    pub tag: String,
    pub name: Option<String>,
    pub published: Option<i64>,
    pub prerelease: bool,
    /// Release notes as plain text (never render as HTML).
    pub notes: String,
    /// The release page on github.com.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// Version of the running app.
    pub current: String,
    /// Highest-versioned published release, pre-releases included.
    pub latest: Option<Latest>,
    /// `latest` is newer than the running app.
    pub newer: bool,
}

/// `v0.6.0` / `0.6.0-rc1` → `[0, 6, 0]`; `None` for tags that aren't a version.
pub fn parse_version(tag: &str) -> Option<Vec<u64>> {
    let t = tag.trim();
    let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
    let core = t.split(['-', '+']).next()?;
    let parts: Vec<u64> = core.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    (!parts.is_empty() && parts.len() <= 4).then_some(parts)
}

fn cmp(a: &[u64], b: &[u64]) -> Ordering {
    let n = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..n).map(|i| at(a, i).cmp(&at(b, i))).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
}

/// The highest-versioned release in `releases` compared with `current`.
pub fn evaluate(current: &str, releases: &[Release]) -> Status {
    let best = releases
        .iter()
        .filter_map(|r| parse_version(&r.tag).map(|v| (v, r)))
        .max_by(|(a, _), (b, _)| cmp(a, b));
    let newer = match (&best, parse_version(current)) {
        (Some((v, _)), Some(cur)) => cmp(v, &cur).is_gt(),
        _ => false,
    };
    Status {
        current: current.to_string(),
        newer,
        latest: best.map(|(_, r)| Latest {
            tag: r.tag.clone(),
            name: r.name.clone(),
            published: r.published,
            prerelease: r.prerelease,
            notes: r.notes.clone(),
            url: r.url.clone(),
        }),
    }
}

/// Ask GitHub for CPMX2077's releases and compare them with this build.
pub fn check(client: &Client) -> Result<Status> {
    let releases = client
        .releases(REPO_OWNER, REPO_NAME)
        .map_err(|e| Error::Other(format!("release list of {REPO_OWNER}/{REPO_NAME} not reachable: {e}")))?;
    Ok(evaluate(APP_VERSION, &releases))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::serve;

    #[test]
    fn parses_tags() {
        assert_eq!(parse_version("v0.6.0"), Some(vec![0, 6, 0]));
        assert_eq!(parse_version("0.7.0-rc1"), Some(vec![0, 7, 0]));
        assert_eq!(parse_version("V1.2"), Some(vec![1, 2]));
        assert_eq!(parse_version("nightly"), None);
        assert_eq!(parse_version("v1..2"), None);
        assert_eq!(parse_version(""), None);
    }

    fn rel(tag: &str, prerelease: bool) -> Release {
        Release {
            id: 1,
            tag: tag.into(),
            name: Some(tag.into()),
            published: None,
            prerelease,
            notes: String::new(),
            url: format!("https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/tag/{tag}"),
            assets: vec![],
        }
    }

    #[test]
    fn picks_the_highest_version_including_prereleases() {
        let rs = [rel("v0.6.0", true), rel("v0.10.0", true), rel("v0.9.1", false), rel("nightly", true)];
        let s = evaluate("0.6.0", &rs);
        assert!(s.newer);
        assert_eq!(s.latest.unwrap().tag, "v0.10.0", "0.10 > 0.9, not compared as text");

        let s = evaluate("0.10.0", &rs);
        assert!(!s.newer, "same version is not an update");
        let s = evaluate("0.11.0", &rs);
        assert!(!s.newer, "a local build ahead of the releases");
        let s = evaluate("0.6.0", &[]);
        assert!(!s.newer && s.latest.is_none());
    }

    #[test]
    fn checks_the_release_list() {
        let (addr, seen) = serve(vec![(
            "/releases",
            200,
            vec![],
            r#"[{"id":7,"tag_name":"v99.0.0","name":"v99.0.0","draft":false,"prerelease":true,
                 "published_at":"2026-10-12T10:00:00Z","body":"- New\r\n- Fixed",
                 "html_url":"https://github.com/o/r/releases/tag/v99.0.0","assets":[]},
                {"id":8,"tag_name":"v100.0.0","draft":true,"assets":[]}]"#
                .into(),
        )]);
        let s = check(&Client::with_base(&addr).unwrap()).unwrap();
        assert!(seen.lock().unwrap()[0].line.contains(&format!("/repos/{REPO_OWNER}/{REPO_NAME}/releases")));
        assert!(s.newer);
        let l = s.latest.unwrap();
        assert_eq!(l.tag, "v99.0.0", "drafts are left out");
        assert!(l.prerelease);
        assert_eq!(l.notes, "- New\n- Fixed");
        assert_eq!(s.current, APP_VERSION);
    }

    #[test]
    fn unreachable_repository_is_an_error_naming_it() {
        let (addr, _) = serve(vec![("/releases", 404, vec![], r#"{"message":"Not Found"}"#.into())]);
        let e = check(&Client::with_base(&addr).unwrap()).unwrap_err().to_string();
        assert!(e.contains("not reachable") && e.contains("404"), "{e}");
    }
}
