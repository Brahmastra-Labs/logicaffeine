//! ★ LOCK — a LOGOS program's points-to sets, from its own statements, by Whaley and Lam's rules. ★
//!
//! `analysis::points_to::points_to_facts` reads a program's statements as Datalog facts (allocations, copies, field and
//! element writes and reads, calls, parameters, returns), and `points_to` evaluates PLDI 2004's rules over them: each
//! variable's set of allocation sites, without contexts or with every call path its own context. The cases are
//! programs whose answer is known by reading them:
//!
//! - the identity called on two lists: without contexts each result may be either list, in contexts each is its own;
//! - a record field written and read back holds what was written, and two records keep their fields apart;
//! - a list holds what was pushed into it and gives it back by `item`, by `Repeat` and by `Pop`;
//! - a recursive function's parameter holds what its callers and its own recursive call pass;
//! - a list a library function returns is an object, and two names copied from one share it.

use logicaffeine_compile::analysis::points_to::{points_to, points_to_facts, PointsTo};
use std::collections::BTreeSet;

fn sites(p: &PointsTo, var: &str) -> BTreeSet<String> {
    p.variables.get(var).cloned().unwrap_or_default()
}

fn analysed(source: &str, contexts: bool) -> PointsTo {
    points_to(source, contexts).unwrap_or_else(|e| panic!("the analysis runs: {e}\n{source}"))
}

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

#[test]
fn the_identity_called_twice_merges_without_contexts_and_keeps_apart_with_them() {
    let merged = analysed(IDENTITY, false);
    let (a, b) = (sites(&merged, "Main.a"), sites(&merged, "Main.b"));
    assert_eq!(a.len(), 1, "a holds the one list it was given: {merged:#?}");
    assert_eq!(b.len(), 1, "{merged:#?}");
    assert_ne!(a, b, "two allocations are two sites");
    let both: BTreeSet<String> = a.union(&b).cloned().collect();
    assert_eq!(sites(&merged, "Main.x"), both, "without contexts x may be either list");
    assert_eq!(sites(&merged, "Main.y"), both);
    assert_eq!(sites(&merged, "id.p"), both, "the parameter sees both calls");

    let apart = analysed(IDENTITY, true);
    assert_eq!(sites(&apart, "Main.x"), a, "in contexts x is a's list alone: {apart:#?}");
    assert_eq!(sites(&apart, "Main.y"), b, "and y is b's");
    assert_eq!(sites(&apart, "id.p"), both, "merged over its contexts the parameter still sees both");
    assert_eq!(apart.contexts, Some(3), "Main's one context and id's two");
}

const BOXES: &str = "## A Box has:
    A contents: Seq of Int.

## Main
Let xs be a new Seq of Int.
Let ys be a new Seq of Int.
Let b be a new Box with contents xs.
Let c be a new Box.
Set c's contents to ys.
Let got be b's contents.
Let other be c's contents.
Show length of got.
Show length of other.
";

#[test]
fn a_record_field_holds_what_was_written_and_two_records_keep_theirs_apart() {
    let p = analysed(BOXES, false);
    assert_eq!(sites(&p, "Main.got"), sites(&p, "Main.xs"), "b's contents is xs: {p:#?}");
    assert_eq!(sites(&p, "Main.other"), sites(&p, "Main.ys"), "c's contents is ys");
    assert!(sites(&p, "Main.got").is_disjoint(&sites(&p, "Main.ys")), "and never ys");
    let (b, xs) = (sites(&p, "Main.b"), sites(&p, "Main.xs"));
    let field = (b.iter().next().cloned().unwrap(), "contents".to_string(), xs.iter().next().cloned().unwrap());
    assert!(p.heap.contains(&field), "the heap holds b.contents → xs: {:?}", p.heap);
}

const LISTS: &str = "## Main
Let first be a new Seq of Int.
Let second be a new Seq of Int.
Let mutable outer be a new Seq of Seq of Int.
Push first to outer.
Push second to outer.
Let got be item 1 of outer.
Repeat for inner in outer:
    Show length of inner.
Pop from outer into last.
Show length of got.
Show length of last.
";

#[test]
fn a_list_gives_back_what_was_pushed_by_item_by_repeat_and_by_pop() {
    let p = analysed(LISTS, false);
    let both: BTreeSet<String> = sites(&p, "Main.first").union(&sites(&p, "Main.second")).cloned().collect();
    assert_eq!(both.len(), 2, "{p:#?}");
    for var in ["Main.got", "Main.inner", "Main.last"] {
        assert_eq!(sites(&p, var), both, "{var} may be either list pushed: {p:#?}");
    }
}

const RECURSION: &str = "## To walk (xs: Seq of Int) and (n: Int) -> Seq of Int:
    If n is at most 0:
        Return xs.
    Return walk(xs, n - 1).

## Main
Let a be a new Seq of Int.
Let r be walk(a, 3).
Show length of r.
";

#[test]
fn a_recursive_function_passes_its_argument_through_its_own_calls() {
    for contexts in [false, true] {
        let p = analysed(RECURSION, contexts);
        assert!(!sites(&p, "Main.a").is_empty(), "contexts {contexts}: a holds the list it was given: {p:#?}");
        assert_eq!(sites(&p, "walk.xs"), sites(&p, "Main.a"), "contexts {contexts}: {p:#?}");
        assert_eq!(sites(&p, "Main.r"), sites(&p, "Main.a"), "contexts {contexts}: the result is a's list");
    }
}

const LIBRARY: &str = "## Main
Let words be split(\"a b c\", \" \").
Let same be words.
Show length of same.
";

#[test]
fn a_list_a_library_call_returns_is_an_object_two_names_copied_from_it_share() {
    for contexts in [false, true] {
        let p = analysed(LIBRARY, contexts);
        assert!(!sites(&p, "Main.words").is_empty(), "contexts {contexts}: split's list is an object: {p:#?}");
        assert_eq!(sites(&p, "Main.same"), sites(&p, "Main.words"), "contexts {contexts}: same shares words' list");
    }
}

#[test]
fn every_parameter_is_a_formal_and_every_call_of_a_defined_function_is_a_call() {
    let facts = points_to_facts(RECURSION).expect("the facts").facts;
    let of = |predicate: &str| -> Vec<Vec<String>> {
        facts.iter().filter(|f| f.predicate == predicate).map(|f| f.ground().expect("a fact is ground")).collect()
    };
    assert_eq!(
        of("Formal"),
        vec![vec!["walk".to_string(), "Z0".into(), "walk.xs".into()], vec!["walk".into(), "Z1".into(), "walk.n".into()]]
    );
    let calls = of("Calls");
    assert_eq!(calls.len(), 2, "walk's own call and Main's: {calls:?}");
    assert!(calls.iter().any(|c| c[0] == "walk" && c[2] == "walk"));
    assert!(calls.iter().any(|c| c[0] == "Main" && c[2] == "walk"));
    assert!(of("Returns").iter().all(|r| r[0] == "walk"), "only walk returns");
}
