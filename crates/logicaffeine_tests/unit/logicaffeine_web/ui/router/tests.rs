//! The boot-normalization invariant: on startup the router parses the browser
//! URL into [`Route`], serializes it back, and *replaces the browser URL* with
//! the serialization whenever the two differ. Any URL state the route type
//! does not model is silently destroyed before page components can read it —
//! these tests lock every query-carrying route to a lossless round-trip.
use super::*;
use std::str::FromStr;

#[track_caller]
fn roundtrip(url: &str) -> String {
    let route = Route::from_str(url).unwrap_or_else(|e| panic!("{url} failed to parse: {e}"));
    route.to_string()
}

#[test]
fn studio_deep_link_survives_roundtrip() {
    let serialized = roundtrip("/studio?file=examples/logic/prover-demo.logic");
    assert!(
        serialized.contains("file=examples/logic/prover-demo.logic"),
        "studio file param must survive boot normalization, got {serialized}"
    );

    let route = Route::from_str("/studio?file=examples/logic/prover-demo.logic").unwrap();
    assert_eq!(
        route,
        Route::Studio {
            file: Some("examples/logic/prover-demo.logic".to_string())
        }
    );
    assert_eq!(
        Route::from_str("/studio").unwrap(),
        Route::Studio { file: None },
        "plain /studio must still parse"
    );
}

#[test]
fn learn_module_fragment_survives_roundtrip() {
    // `/learn#<module-id>` opens the Learn page at that module; the fragment must survive
    // boot normalization or every deep link lands at the top of the page.
    assert_eq!(roundtrip("/learn#modal"), "/learn#modal");
    assert_eq!(roundtrip("/learn#further_modal"), "/learn#further_modal");
    assert_eq!(
        roundtrip("/learn"),
        "/learn",
        "a bare /learn must stay bare"
    );
}

#[test]
fn studio_file_url_is_canonical() {
    let url = studio_file_url("/examples/logic/prover-demo.logic");
    assert_eq!(url, "/studio?file=examples/logic/prover-demo.logic");
    assert_eq!(url, studio_file_url("examples/logic/prover-demo.logic"));

    let route = Route::from_str(&url).expect("canonical studio URL parses");
    assert_eq!(
        route,
        Route::Studio {
            file: Some("examples/logic/prover-demo.logic".to_string())
        }
    );
    assert_eq!(
        route.to_string(),
        url,
        "writer and router agree byte-for-byte"
    );
}

/// The pit-of-success locks over the whole web app source. Read side: no
/// page may scrape the query string from the browser location — the
/// router destroys unmodeled query params at boot, so scraping is always
/// a latent deep-link bug; query state must be declared on the route
/// (`?:name`) and received as props. Write side: nobody may hand-build a
/// query URL — links come from the `Route` type ([`studio_file_url`],
/// `Link { to: Route::… }`) so writers can never drift from what the
/// router parses.
#[test]
fn query_scraping_is_forbidden() {
    // Needles assembled at runtime so this test's own source never matches.
    // (needle, files exempt from it)
    let readers_forbidden_everywhere: &[String] = &[
        format!("location(){}", ".search()"),
        format!("location{}", ".search()"),
        format!("UrlSearch{}", "Params"),
    ];
    let writers_allowed_only_in_router: &[String] = &[
        format!("studio?{}", "file="),
        format!("news?{}", "tag="),
        format!("registry?{}", "q="),
        format!("success?{}", "session_id="),
    ];

    let src_root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut offenders = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(src_root)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable src dir") {
            let path = entry.expect("readable dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !path.extension().is_some_and(|e| e == "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("readable source file");
            for needle in readers_forbidden_everywhere {
                if source.contains(needle.as_str()) {
                    offenders.push(format!(
                        "{} scrapes the query string (`{needle}`) — declare the \
                             parameter on the route (`?:name`) and take it as a prop",
                        path.display()
                    ));
                }
            }
            // router.rs is the one place allowed to spell query-URL shapes
            // (route definitions, canonical builders, these tests); seo.rs
            // holds the JSON-LD SearchAction template, whose `{search_term}`
            // placeholder cannot be built from the Route type.
            if path.ends_with("router.rs") || path.ends_with("seo.rs") {
                continue;
            }
            for needle in writers_allowed_only_in_router {
                if source.contains(needle.as_str()) {
                    offenders.push(format!(
                        "{} hand-builds a query URL (`{needle}`) — build it from \
                             the Route type (studio_file_url / Link {{ to: Route::… }})",
                        path.display()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "query-string handling outside the route type:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn news_tag_survives_roundtrip() {
    let serialized = roundtrip("/news?tag=releases");
    assert!(
        serialized.contains("tag=releases"),
        "news tag param must survive boot normalization, got {serialized}"
    );
}

#[test]
fn registry_oauth_params_survive_roundtrip() {
    let serialized = roundtrip("/registry?token=t0ken&login=octocat");
    assert!(
        serialized.contains("token=t0ken") && serialized.contains("login=octocat"),
        "registry OAuth params must survive boot normalization, got {serialized}"
    );

    let serialized = roundtrip("/registry?error=denied");
    assert!(
        serialized.contains("error=denied"),
        "registry OAuth error must survive boot normalization, got {serialized}"
    );
}

#[test]
fn registry_search_action_url_works() {
    // The JSON-LD SearchAction advertises /registry?q={search_term}.
    let serialized = roundtrip("/registry?q=parser");
    assert!(
        serialized.contains("q=parser"),
        "registry search query must survive boot normalization, got {serialized}"
    );
    let route = Route::from_str("/registry?q=parser").unwrap();
    assert_eq!(
        route,
        Route::Registry {
            token: None,
            login: None,
            error: None,
            q: Some("parser".to_string())
        }
    );
}

#[test]
fn percent_encoded_file_roundtrips() {
    let serialized = roundtrip("/studio?file=my%20notes.logos");
    assert!(
        serialized.contains("file=my%20notes.logos"),
        "encoded file param must survive boot normalization, got {serialized}"
    );
    let route = Route::from_str(&serialized).expect("re-serialized URL parses");
    assert_eq!(serialized, route.to_string(), "serialization is stable");
}

#[test]
fn boot_normalization_is_fixpoint() {
    for url in [
        "/",
        "/studio",
        "/studio?file=examples/logic/prover-demo.logic",
        "/studio?file=examples/code/basics/hello.logos",
        "/news",
        "/news?tag=releases",
        "/success",
        "/success?session_id=cs_test_123",
        "/registry",
        "/registry?token=t0ken&login=octocat",
        "/registry?error=denied",
        "/registry?q=parser",
        "/registry/package/logicaffeine-base",
        "/news/some-article",
        "/workspace/logic",
    ] {
        let first = Route::from_str(url).unwrap_or_else(|e| panic!("{url} failed to parse: {e}"));
        let s1 = first.to_string();
        let second = Route::from_str(&s1)
            .unwrap_or_else(|e| panic!("{s1} (from {url}) failed to re-parse: {e}"));
        assert_eq!(
            first, second,
            "{url}: parse → serialize → parse must be identity"
        );
        assert_eq!(
            s1,
            second.to_string(),
            "{url}: serialization must be a fixpoint"
        );
    }
}

#[test]
fn contact_is_the_page_and_pricing_its_old_address() {
    // `/contact` is where people look for the team. `/pricing`, the page's old address, opens the
    // same page, and boot normalization rewrites the address bar to it.
    for url in ["/contact", "/contact/", "/pricing", "/pricing/"] {
        let route = Route::from_str(url).unwrap_or_else(|e| panic!("{url} failed to parse: {e}"));
        assert!(
            matches!(route, Route::Contact { .. }),
            "{url} parses to {route:?}, not the contact page"
        );
        assert_eq!(route.to_string(), "/contact", "{url} must normalize to the contact page");
    }
    let form = Route::from_str("/contact#contact").expect("the form's address parses");
    assert_eq!(form.to_string(), "/contact#contact", "the form's address is kept as is");
}

#[test]
fn profile_lands_on_learn() {
    // The profile page is gone; its address opens the Learn roadmap, which shows the learner's
    // progress, and boot normalization rewrites the address bar to /learn.
    for url in ["/profile", "/profile/"] {
        let route = Route::from_str(url).unwrap_or_else(|e| panic!("{url} failed to parse: {e}"));
        assert!(
            matches!(route, Route::Learn { .. }),
            "{url} parses to {route:?}, not the Learn page"
        );
        assert_eq!(
            route.to_string(),
            "/learn",
            "{url} must normalize to /learn"
        );
    }
}

#[test]
fn contact_section_fragment_survives_roundtrip() {
    // `/contact#contact` is where every call to the form points; the fragment must survive boot
    // normalization or the link lands at the top of the page.
    assert_eq!(roundtrip("/contact#contact"), "/contact#contact");
    assert_eq!(roundtrip("/contact#faq"), "/contact#faq");
    assert_eq!(roundtrip("/contact"), "/contact", "a bare /contact must stay bare, with no trailing #");
    assert_eq!(roundtrip("/contact/"), "/contact");
}

#[test]
fn privacy_section_fragment_survives_roundtrip() {
    // The contact form's privacy note links `/privacy#pricing-requests` (the section keeps its name).
    assert_eq!(
        roundtrip("/privacy#pricing-requests"),
        "/privacy#pricing-requests"
    );
    assert_eq!(
        roundtrip("/privacy"),
        "/privacy",
        "a bare /privacy must stay bare, with no trailing #"
    );
}
