use super::*;

#[test]
fn history_parses_and_is_nonempty() {
    assert!(
        !get_history().is_empty(),
        "roadmap_history.json deserialized to no releases"
    );
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
    let any = get_history()
        .iter()
        .any(|r| news_slug_for(&r.version).is_some());
    assert!(
        any,
        "no release resolved a news-article slug — version→slug matcher is broken"
    );
}
