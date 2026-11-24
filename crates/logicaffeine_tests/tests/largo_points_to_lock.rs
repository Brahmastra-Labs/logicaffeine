//! ★ LOCK — `largo points-to`: what each variable of a program may point to, from the command line. ★
//!
//! `largo points-to FILE` prints each function's variables with the allocation sites each may hold, the heap's field
//! edges and what each site allocates; `--contexts` analyzes every call path as its own context; `--json` prints the
//! same as data. The program is the identity called on two lists.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};

const IDENTITY: &str = "## To id (p: Seq of Int) -> Seq of Int:
    Return p.

## Main
Let a be a new Seq of Int.
Let b be a new Seq of Int.
Let x be id(a).
Let y be id(b).
Show length of x.
Show length of y.
";

fn points_to(args: &[&str]) -> (bool, String, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("identity.lg"), IDENTITY).expect("write source");
    let mut argv = vec!["points-to", "identity.lg"];
    argv.extend_from_slice(args);
    let out = largo_in(dir.path(), &argv);
    (out.status.success(), stdout(&out), stderr(&out))
}

fn line<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.lines()
        .find(|l| l.trim_start().starts_with(prefix))
        .unwrap_or_else(|| panic!("a line starting {prefix:?}:\n{text}"))
}

#[test]
fn without_contexts_both_results_may_hold_both_lists() {
    let (ok, out, err) = points_to(&[]);
    assert!(ok, "`largo points-to` succeeds:\n{err}");
    let x = line(&out, "x →");
    assert_eq!(x.matches('H').count(), 2, "x may be either list: {x}");
    assert!(out.contains("sites:") && out.contains("a new Seq"), "the sites are named:\n{out}");
    assert!(!out.contains(".%"), "the analysis's temporaries are not shown:\n{out}");
}

#[test]
fn with_contexts_each_result_holds_its_own_list() {
    let (ok, out, err) = points_to(&["--contexts"]);
    assert!(ok, "`largo points-to --contexts` succeeds:\n{err}");
    assert!(out.contains("3 contexts"), "Main's one and id's two:\n{out}");
    let (x, a) = (line(&out, "x →"), line(&out, "a →"));
    assert_eq!(x.split('→').nth(1), a.split('→').nth(1), "x holds exactly what a holds:\n{out}");
    let (y, b) = (line(&out, "y →"), line(&out, "b →"));
    assert_eq!(y.split('→').nth(1), b.split('→').nth(1), "y holds exactly what b holds:\n{out}");
}

#[test]
fn json_gives_the_analysis_as_data() {
    let (ok, out, err) = points_to(&["--contexts", "--json"]);
    assert!(ok, "`largo points-to --contexts --json` succeeds:\n{err}");
    let report: serde_json::Value = serde_json::from_str(out.trim()).expect("JSON");
    assert_eq!(report["contexts"], 3);
    assert_eq!(report["variables"]["Main.x"], report["variables"]["Main.a"], "{report}");
    assert_ne!(report["variables"]["Main.x"], report["variables"]["Main.y"], "{report}");
    assert!(report["sites"].as_array().is_some_and(|s| s.len() >= 2), "{report}");
}
