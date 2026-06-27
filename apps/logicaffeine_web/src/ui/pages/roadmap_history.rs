//! Release-history data for the roadmap page.
//!
//! Generated from `CHANGELOG.md` + git tags by `scripts/generate-roadmap.sh`
//! into `roadmap_history.json`, then baked at compile time — mirroring how
//! [`benchmarks`](super::benchmarks) bakes `latest.json`. The roadmap renders
//! one terse row per release; clicking a release opens its news article when
//! one exists ([`news_slug_for`]).

use std::sync::LazyLock;
use serde::Deserialize;

/// One released (or prepared) version. Newest-first in [`get_history`].
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub date: String,
    pub title: String,
    /// Whether a `v<version>` git tag exists. `false` = prepared, not yet cut.
    pub tagged: bool,
    /// Release-plumbing release (CI, benchmark infra, deploy, tooling, skipped).
    /// Collapsed by default on the roadmap.
    #[serde(default)]
    pub maintenance: bool,
}

static HISTORY: LazyLock<Vec<Release>> =
    LazyLock::new(|| serde_json::from_str(include_str!("roadmap_history.json")).unwrap());

/// All releases, newest-first.
pub fn get_history() -> &'static [Release] {
    &HISTORY
}

/// The news-article slug for a release, if one exists.
///
/// Release articles are titled `v<version> — …`, so the first whitespace token
/// is matched exactly — `v0.9.1` must not prefix-match `v0.9.16`.
pub fn news_slug_for(version: &str) -> Option<&'static str> {
    let token = format!("v{version}");
    crate::ui::pages::news::get_articles()
        .into_iter()
        .find(|a| a.title.split_whitespace().next() == Some(token.as_str()))
        .map(|a| a.slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_parses_and_is_nonempty() {
        assert!(!get_history().is_empty(), "roadmap_history.json deserialized to no releases");
    }

    #[test]
    fn every_release_is_well_formed() {
        for r in get_history() {
            assert!(!r.version.is_empty(), "release with empty version");
            assert!(!r.date.is_empty(), "release {} has empty date", r.version);
            assert!(!r.title.is_empty(), "release {} has empty title", r.version);
        }
    }

    #[test]
    fn newest_first_ordering() {
        let h = get_history();
        assert!(h.len() >= 2, "expected at least two releases");
        let parse = |v: &str| -> Vec<u32> { v.split('.').filter_map(|p| p.parse().ok()).collect() };
        assert!(
            parse(&h[0].version) >= parse(&h[1].version),
            "history not newest-first: {} before {}",
            h[0].version,
            h[1].version
        );
    }

    #[test]
    fn tagged_releases_with_articles_resolve_a_slug() {
        let any = get_history().iter().any(|r| news_slug_for(&r.version).is_some());
        assert!(any, "no release resolved a news-article slug — version→slug matcher is broken");
    }
}
