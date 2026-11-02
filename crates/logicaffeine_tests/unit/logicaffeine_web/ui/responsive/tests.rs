//! The named breakpoints, media queries and touch sizes agree with each other and with the custom
//! properties the stylesheets read, so a band is one number wherever it is written.
use super::*;
use crate::ui::pages::studio_layout::DESKTOP_MIN_WIDTH;

/// The pixel lengths a media query names, in order.
fn lengths(query: &str) -> Vec<u32> {
    query.split("px").filter_map(|part| part.rsplit(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok())).collect()
}

#[test]
fn the_reader_reads_a_querys_lengths() {
    assert_eq!(lengths("(min-width: 769px) and (max-width: 1024px)"), [769, 1024]);
    assert_eq!(lengths("(pointer: coarse)"), Vec::<u32>::new());
}

#[test]
fn each_named_query_says_its_breakpoints() {
    use breakpoints::*;
    for (query, want) in [
        (media::PHONE, vec![PHONE_MAX]),
        (media::MOBILE, vec![MOBILE_MAX]),
        (media::DESKTOP, vec![MOBILE_MAX + 1]),
        (media::SMALL_PHONE, vec![SMALL_PHONE_MAX]),
        (media::TABLET, vec![TABLET_MAX]),
        (media::SMALL_LAPTOP, vec![MOBILE_MAX + 1, TABLET_MAX]),
        (media::TABLET_PORTRAIT, vec![PHONE_MAX + 1, MOBILE_MAX]),
        (media::UPRIGHT_TABLET, vec![MOBILE_MAX + 1, TABLET_MAX]),
        (media::TABLET_LANDSCAPE, vec![MOBILE_MAX + 1, SHORT_MAX + 1]),
        (media::PHONE_LANDSCAPE, vec![SHORT_MAX]),
        (media::FRAME_WIDE, vec![FRAME_WIDE_MIN]),
    ] {
        assert_eq!(lengths(query), want, "{query}");
    }
}

#[test]
fn every_named_query_is_listed_once() {
    let mut seen = std::collections::BTreeSet::new();
    for query in media::ALL {
        assert!(seen.insert(*query), "{query} is listed twice");
        assert!(query.starts_with('(') && query.ends_with(')'), "{query:?} is not a media condition");
    }
    for query in [media::PHONE, media::MOBILE, media::DESKTOP, media::COARSE, media::NO_HOVER, media::UPRIGHT_TABLET, media::TABLET_LANDSCAPE, media::PHONE_LANDSCAPE] {
        assert!(media::ALL.contains(&query), "{query} is not in media::ALL");
    }
}

#[test]
fn the_stylesheets_custom_properties_are_the_named_sizes() {
    assert!(MOBILE_BASE_STYLES.contains(&format!("--breakpoint-mobile: {}px;", breakpoints::MOBILE_MAX)), "--breakpoint-mobile is not MOBILE_MAX");
    assert!(MOBILE_BASE_STYLES.contains(&format!("--breakpoint-tablet: {}px;", breakpoints::TABLET_MAX)), "--breakpoint-tablet is not TABLET_MAX");
    assert!(MOBILE_BASE_STYLES.contains(&format!("--touch-min: {}px;", touch::MIN)), "--touch-min is not touch::MIN");
    assert!(MOBILE_BASE_STYLES.contains(&format!("--touch-comfortable: {}px;", touch::COMFORTABLE)), "--touch-comfortable is not touch::COMFORTABLE");
    assert_eq!(touch::MIN, 44, "WCAG 2.5.5's target is 44 by 44 CSS pixels");
}

/// The Studio's layout model lays out desktop columns from the first width the desktop query matches.
#[test]
fn the_layout_models_desktop_starts_where_the_desktop_query_does() {
    assert_eq!(DESKTOP_MIN_WIDTH, f64::from(breakpoints::MOBILE_MAX + 1));
}
