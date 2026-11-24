//! ★ LOCK — looking a key up in a map does not consume the key. ★
//!
//! A map lookup reads its key; it never takes it. `Repeat for (name, v) in base:` and then
//! `(item name of a) equals (item name of b)`, or two `Let`s reading `item name of` two maps, is how
//! two maps are compared entry by entry, and generated Rust that passed the key to the first lookup
//! by value moved it there, so the second failed to build ("use of moved value"). The tree-walker and
//! the VM run every program here, and `compile_to_rust` returns `Ok` for them, so the only measure
//! that catches a moved key is one that builds the Rust and runs it, which is what each row does.
//!
//! A map the emitter knows (a variable bound to one, an item of a `Seq of Map`, a loop over one, a
//! record's map field, a function's map result) is read through its own `get`, which borrows the key
//! (a key written in place as the text it is, with no `String` made for it); a key looked up in a
//! collection whose type the emitter does not know is copied in where the program reads it again.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test aot_map_key_reuse_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::run_logos;

/// `src` runs on AOT Rust and prints what the tree-walker prints, `expected`; the Rust it built from.
fn e2e(src: &str, expected: &str) -> String {
    let tw = logicaffeine_compile::compile::tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    assert_eq!(tw.output.trim_end(), expected, "the reference tier must agree with the test:\n{src}");
    let r = run_logos(src);
    assert!(
        r.success,
        "the generated Rust must BUILD and RUN:\n{}\n--- LOGOS ---\n{src}\n--- EMITTED ---\n{}",
        r.stderr, r.rust_code
    );
    assert_eq!(r.stdout.trim_end(), expected, "AOT must answer what the tree-walker answers:\n{src}");
    r.rust_code
}

/// The Rust of the program's function `name`.
fn function<'c>(rust: &'c str, name: &str) -> &'c str {
    let start = rust.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("no `fn {name}` in:\n{rust}"));
    let end = rust[start..].find("\n}\n").map_or(rust.len(), |e| start + e + 3);
    &rust[start..end]
}

/// Whether the arms of a `select` going on past it keep some variable of `base` in values apart: the lead
/// arm's end store against each later arm's, entry by entry, through a key read in two maps.
const ARMS_APART: &str = "\
## To apart (a: Int) and (b: Int) -> Bool:
    Return not (a equals b).

## To decide (ends: Seq of Map of Text to Int) and (base: Map of Text to Int) -> Bool:
    Let mutable lead be 0.
    Let mutable at be 0.
    Repeat for e in ends:
        Set at to at + 1.
        If lead equals 0:
            Set lead to at.
        Otherwise:
            Let leadEnd be item lead of ends.
            Repeat for (name, baseVal) in base:
                If (leadEnd contains name) and (e contains name):
                    Let lv be item name of leadEnd.
                    Let tv be item name of e.
                    If apart(lv, tv):
                        Return true.
    Return false.

## Main
Let mutable m1 be a new Map of Text to Int.
Set item \"x\" of m1 to 1.
Let mutable m2 be a new Map of Text to Int.
Set item \"x\" of m2 to 2.
Let mutable base be a new Map of Text to Int.
Set item \"x\" of base to 0.
Show decide([m1, m2], base).
";

/// ★ THE HEADLINE — two `Let`s read one key out of two maps: an item of a `Seq of Map` and the loop's own
/// element.
#[test]
fn a_key_read_out_of_two_maps_by_two_lets_builds() {
    e2e(ARMS_APART, "true");
}

/// One condition reads the key out of both maps.
#[test]
fn a_key_read_out_of_two_maps_in_one_condition_builds() {
    let src = "\
## To agree (ends: Seq of Map of Text to Text) and (base: Map of Text to Text) -> Int:
    Let mutable count be 0.
    Let lead be item 1 of ends.
    Repeat for e in ends:
        Repeat for (name, baseVal) in base:
            If (lead contains name) and (e contains name):
                If (item name of lead) equals (item name of e):
                    Set count to count + 1.
    Return count.

## Main
Let mutable m1 be a new Map of Text to Text.
Set item \"x\" of m1 to \"a\".
Let mutable m2 be a new Map of Text to Text.
Set item \"x\" of m2 to \"a\".
Let mutable base be a new Map of Text to Text.
Set item \"x\" of base to \"z\".
Show agree([m1, m2], base).
";
    e2e(src, "2");
}

/// The key is read out of two maps and then stored under in a third: three uses in one statement.
#[test]
fn a_key_stored_under_after_two_lookups_builds() {
    let src = "\
## To merged (ends: Seq of Map of Text to Int) -> Map of Text to Int:
    Let mutable out be a new Map of Text to Int.
    Let first be item 1 of ends.
    Repeat for e in ends:
        Repeat for (k, v) in e:
            If first contains k:
                Set item k of out to (item k of first) + (item k of e).
    Return out.

## Main
Let mutable m1 be a new Map of Text to Int.
Set item \"x\" of m1 to 1.
Let mutable m2 be a new Map of Text to Int.
Set item \"x\" of m2 to 2.
Set item \"y\" of m2 to 7.
Let r be merged([m1, m2]).
Show item \"x\" of r.
Show length of r.
";
    e2e(src, "3\n1");
}

/// The maps are fields of two records.
#[test]
fn a_key_read_out_of_two_record_fields_builds() {
    let src = "\
## A Board has:
    a scores, which is Map of Text to Int.

## To same (a: Board) and (b: Board) and (names: Seq of Text) -> Int:
    Let mutable count be 0.
    Repeat for name in names:
        If (item name of a's scores) equals (item name of b's scores):
            Set count to count + 1.
    Return count.

## Main
Let mutable s1 be a new Map of Text to Int.
Set item \"x\" of s1 to 1.
Set item \"y\" of s1 to 2.
Let mutable s2 be a new Map of Text to Int.
Set item \"x\" of s2 to 1.
Set item \"y\" of s2 to 3.
Let a be a new Board with scores s1.
Let b be a new Board with scores s2.
Show same(a, b, [\"x\", \"y\"]).
";
    e2e(src, "1");
}

/// The maps are what two calls return.
#[test]
fn a_key_read_out_of_two_calls_builds() {
    let src = "\
## To table (n: Int) -> Map of Text to Int:
    Let mutable m be a new Map of Text to Int.
    Set item \"x\" of m to n.
    Set item \"y\" of m to n + 1.
    Return m.

## To spread (names: Seq of Text) -> Int:
    Let mutable total be 0.
    Repeat for name in names:
        Set total to total + (item name of table(1)) + (item name of table(2)).
    Return total.

## Main
Show spread([\"x\", \"y\"]).
";
    e2e(src, "8");
}

/// The maps are items a list holds and a map holds, read where they stand: the key is copied into each lookup.
#[test]
fn a_key_read_out_of_maps_held_by_a_list_and_a_map_builds() {
    let src = "\
## To pick (ends: Seq of Map of Text to Int) and (grid: Map of Text to Map of Text to Int) and (names: Seq of Text) -> Int:
    Let mutable total be 0.
    Repeat for name in names:
        Set total to total + (item name of (item 1 of ends)) + (item name of (item 2 of ends)).
        Set total to total + (item name of (item \"a\" of grid)) + (item name of (item \"b\" of grid)).
    Return total.

## Main
Let mutable m1 be a new Map of Text to Int.
Set item \"x\" of m1 to 1.
Let mutable m2 be a new Map of Text to Int.
Set item \"x\" of m2 to 2.
Let mutable g be a new Map of Text to Map of Text to Int.
Set item \"a\" of g to m1.
Set item \"b\" of g to m2.
Show pick([m1, m2], g, [\"x\"]).
";
    e2e(src, "6");
}

/// The key is a record's field, read twice: a lookup leaves the record whole.
#[test]
fn a_field_used_as_a_key_twice_builds() {
    let src = "\
## A Person has:
    a name, which is Text.

## To twice (p: Person) and (ends: Seq of Map of Text to Int) -> Int:
    Let mutable total be 0.
    Repeat for e in ends:
        If e contains p's name:
            Set total to total + (item (p's name) of e) + (item (p's name) of e).
    Return total.

## Main
Let mutable m1 be a new Map of Text to Int.
Set item \"ann\" of m1 to 4.
Let p be a new Person with name \"ann\".
Show twice(p, [m1]).
";
    e2e(src, "8");
}

/// An item of a `Seq of Map` and a loop's element of one are the map they hold: each lookup goes through the
/// map's own `get`, and what it reads is typed.
#[test]
fn an_item_and_an_element_of_a_seq_of_maps_are_read_as_maps() {
    let rust = e2e(ARMS_APART, "true");
    let decide = function(&rust, "decide");
    for map in ["leadEnd", "e"] {
        assert!(
            !decide.contains(&format!("LogosIndex::logos_get(&{map},")),
            "`{map}` is read through the untyped index:\n{decide}"
        );
        assert!(decide.contains(&format!("{map}.get(__key)")), "`{map}` is not read through its own `get`:\n{decide}");
    }
    for read in ["lv", "tv"] {
        assert!(decide.contains(&format!("let {read}: i64 =")), "`{read}` is not typed:\n{decide}");
    }
}

/// An `Int` key is copied by the lookup itself: nothing clones it.
#[test]
fn an_int_key_is_never_cloned() {
    let src = "\
## To pairs (ends: Seq of Map of Int to Int) and (keys: Seq of Int) -> Int:
    Let mutable total be 0.
    Repeat for e in ends:
        Repeat for k in keys:
            If e contains k:
                Set total to total + (item k of e) + (item k of e).
    Return total.

## Main
Let mutable m1 be a new Map of Int to Int.
Set item 1 of m1 to 10.
Let mutable m2 be a new Map of Int to Int.
Set item 1 of m2 to 20.
Set item 2 of m2 to 5.
Show pairs([m1, m2], [1, 2]).
";
    let rust = e2e(src, "70");
    let pairs = function(&rust, "pairs");
    assert!(!pairs.contains("k.clone()"), "an `Int` key is cloned:\n{pairs}");
}

/// A key written in place is looked up as the text it is, borrowed: the lookup makes no `String` for it.
#[test]
fn a_key_written_in_place_is_looked_up_without_a_copy() {
    let src = "\
## To tally (words: Seq of Text) -> Int:
    Let mutable counts be a new Map of Text to Int.
    Set item \"the\" of counts to 0.
    Repeat for w in words:
        If w equals \"the\":
            Set item \"the\" of counts to (item \"the\" of counts) + 1.
    Return item \"the\" of counts.

## Main
Show tally([\"the\", \"cat\", \"the\"]).
";
    let rust = e2e(src, "2");
    let tally = function(&rust, "tally");
    assert!(tally.contains("let __key: &str = \"the\";"), "the key is not looked up as itself:\n{tally}");
    assert!(
        !tally.contains("let __key: &String = &::std::string::String::from(\"the\")"),
        "a lookup makes a `String` for its key:\n{tally}"
    );
}
