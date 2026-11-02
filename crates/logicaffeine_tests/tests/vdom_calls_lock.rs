//! ★ LOCK — `VDOM_CALLS`, the `vdom.lg` functions a lowered page calls, is exactly the set the
//! lowering writes, and every one of them is a function `vdom.lg` defines. ★
//!
//! A page's program splices `vdom.lg`, and the pruner keeps only what something reaches. The
//! author's source names no `vd` function, so the calls the lowering writes are kept by this list,
//! handed to the pruner as roots (`loader.rs`). A call the lowering writes and the list misses is
//! pruned out from under the page, which then fails with an unknown function on whichever tier
//! first reaches it; a name the list keeps and nothing calls is dead code in every page's module,
//! bytes a browser downloads for nothing. Both directions are held here, against the lowering's
//! own source, so a new tape call cannot land half-registered.

use logicaffeine_language::parser::lgx_lower::{VDOM_CALLS, VDOM_FEATURES};
use std::collections::BTreeSet;
use std::path::Path;

#[path = "support/lgx_lower_source.rs"]
mod lgx_lower_source;

/// `source` with the declaration opening `head` cut out, through its closing `];`.
fn without(source: &str, head: &str) -> String {
    let start = source
        .find(head)
        .unwrap_or_else(|| panic!("the lowering declares {head}"));
    let end = start + source[start..].find("];").expect("the declaration closes") + 2;
    format!("{}{}", &source[..start], &source[end..])
}

/// The lowering's source with the `VDOM_CALLS` and `VDOM_FEATURES` declarations cut out, so the
/// census counts the calls the lowering WRITES and never the lists themselves.
fn source_without_the_list() -> String {
    let source = lgx_lower_source::source();
    without(
        &without(&source, "pub const VDOM_CALLS"),
        "pub const VDOM_FEATURES",
    )
}

fn vdom_source() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/assets/std/vdom.lg");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every `"vdName"` string literal in `text`: a `vd` then an uppercase letter, then letters.
fn vd_literals(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut found = BTreeSet::new();
    let mut i = 0;
    while let Some(at) = text[i..].find("\"vd") {
        let start = at + i + 1;
        let mut end = start + 2;
        while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
            end += 1;
        }
        let shaped = end > start + 2 && bytes[start + 2].is_ascii_uppercase();
        if shaped && end < bytes.len() && bytes[end] == b'"' {
            found.insert(text[start..end].to_string());
        }
        i = start;
    }
    found
}

fn written() -> BTreeSet<String> {
    let calls = vd_literals(&source_without_the_list());
    assert!(
        calls.contains("vdCommit") && calls.contains("vdOpen"),
        "the census found the lowering's calls — a census that reads nothing passes for the wrong reason: {calls:?}"
    );
    calls
}

fn listed() -> BTreeSet<String> {
    VDOM_CALLS.iter().map(|s| s.to_string()).collect()
}

#[test]
fn every_call_the_lowering_writes_is_listed() {
    let missing: Vec<_> = written().difference(&listed()).cloned().collect();
    assert!(
        missing.is_empty(),
        "the lowering writes calls VDOM_CALLS does not keep: {missing:?}"
    );
}

#[test]
fn every_listed_call_is_one_the_lowering_writes() {
    let unused: Vec<_> = listed().difference(&written()).cloned().collect();
    assert!(
        unused.is_empty(),
        "VDOM_CALLS keeps functions no lowering calls: {unused:?}"
    );
}

#[test]
fn every_listed_call_is_a_function_vdom_defines() {
    let vdom = vdom_source();
    let undefined: Vec<_> = VDOM_CALLS
        .iter()
        .filter(|name| !vdom.contains(&format!("\n## To {name}(")))
        .collect();
    assert!(
        undefined.is_empty(),
        "VDOM_CALLS names functions vdom.lg does not define: {undefined:?}"
    );
}

#[test]
fn the_list_is_sorted_and_names_each_call_once() {
    let mut sorted = VDOM_CALLS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        VDOM_CALLS,
        sorted.as_slice(),
        "VDOM_CALLS in order, each name once"
    );
}

#[test]
fn the_census_reads_a_quoted_call_and_nothing_else() {
    let text = r#"vd_call("vdOpen", x); "vdOpenSoft" `vdClose` "vd" "vdx" "vdFrag Open" "vdSite"."#;
    let found: Vec<_> = vd_literals(text).into_iter().collect();
    assert_eq!(found, ["vdOpen", "vdOpenSoft", "vdSite"]);
}

// ---- `VDOM_FEATURES`: a feature's code is entered only through the calls that write its records. ----

#[test]
fn every_feature_is_written_by_calls_the_lowering_makes() {
    for f in VDOM_FEATURES {
        assert!(
            !f.writers.is_empty(),
            "feature `{}` names no writer, so nothing could ever reach it",
            f.prefix
        );
        for w in f.writers {
            assert!(
                VDOM_CALLS.contains(w),
                "feature `{}`'s writer {w} is not a call the lowering makes",
                f.prefix
            );
            assert!(
                !w.starts_with(f.prefix),
                "feature `{}`'s writer {w} is named as one of its own functions",
                f.prefix
            );
        }
    }
}

#[test]
fn every_feature_names_functions_vdom_defines() {
    let vdom = vdom_source();
    for f in VDOM_FEATURES {
        let defined = vdom.matches(&format!("\n## To {}", f.prefix)).count();
        assert!(
            defined > 0,
            "feature `{}` names no function vdom.lg defines",
            f.prefix
        );
    }
}

#[test]
fn the_lowering_enters_no_feature_but_through_its_writers() {
    let direct: Vec<_> = VDOM_CALLS
        .iter()
        .filter(|call| VDOM_FEATURES.iter().any(|f| call.starts_with(f.prefix)))
        .collect();
    assert!(
        direct.is_empty(),
        "the lowering calls a feature's function directly, which its page would then carry whether or not \
         it writes the feature's records: {direct:?}"
    );
}
