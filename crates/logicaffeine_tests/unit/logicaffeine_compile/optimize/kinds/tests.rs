use super::*;

/// `(name, int?, seq?)` for each name in `names`, by the kinds of the checked program `src`.
fn kinds_of(src: &str, names: &[&str]) -> Vec<(String, bool, bool)> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("the program parses");
        let env = crate::analysis::check_program_located(stmts, interner, registry)
            .expect("the program checks");
        let kinds = of_program(stmts, &env, interner);
        names
            .iter()
            .map(|n| {
                let s = interner
                    .lookup(n)
                    .unwrap_or_else(|| panic!("`{n}` is named"));
                (n.to_string(), kinds.int(s), kinds.seq(s))
            })
            .collect()
    })
}

fn row(name: &str, int: bool, seq: bool) -> (String, bool, bool) {
    (name.to_string(), int, seq)
}

#[test]
fn a_name_bound_only_to_ints_is_an_int() {
    let src = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"4\").\nLet mutable i be 0.\nWhile i is less than n:\n    Set i to i + 1.\nLet m be n * 2 - 1 + length of [1, 2].\nShow m.\n";
    assert_eq!(
        kinds_of(src, &["n", "i", "m"]),
        [
            row("n", true, false),
            row("i", true, false),
            row("m", true, false)
        ]
    );
}

#[test]
fn one_real_binding_makes_a_name_no_int() {
    let src = "## Main\nLet mutable x be 3.\nSet x to 2.5.\nShow x.\n";
    assert_eq!(kinds_of(src, &["x"]), [row("x", false, false)]);
}

#[test]
fn a_division_is_not_known_to_be_an_int() {
    let src = "## Main\nLet q be 7 / 2.\nShow q.\n";
    assert_eq!(kinds_of(src, &["q"]), [row("q", false, false)]);
}

#[test]
fn scopes_are_merged_so_a_name_typed_two_ways_is_neither() {
    let src = "## To half (x: Real) -> Real:\n    Return x / 2.0.\n\n## Main\nLet x be 4.\nShow half(2.0).\nShow x.\n";
    assert_eq!(kinds_of(src, &["x"]), [row("x", false, false)]);
}

#[test]
fn a_counter_over_a_range_is_an_int_and_over_a_sequence_is_not_known() {
    let src = "## Main\nLet xs be [4, 5].\nRepeat for i from 1 to 3:\n    Show i.\nRepeat for v in xs:\n    Show v.\n";
    assert_eq!(
        kinds_of(src, &["i", "v", "xs"]),
        [
            row("i", true, false),
            row("v", false, false),
            row("xs", false, true)
        ]
    );
}

#[test]
fn a_binding_this_module_does_not_read_makes_the_name_unknown() {
    let src =
        "## Main\nLet mutable xs be [1, 2].\nLet mutable y be 0.\nPop from xs into y.\nShow y.\n";
    assert_eq!(
        kinds_of(src, &["y"]),
        [row("y", false, false)],
        "`Pop … into` binds y, and this module does not read what"
    );
}

#[test]
fn sequences_are_new_seqs_list_literals_and_declared_parameters() {
    let src = "## To first (xs: Seq of Int) -> Int:\n    Return item 1 of xs.\n\n## Main\nLet mutable c be a new Seq of Int.\nLet d be [1, 2, 3].\nLet m be a new Map of Text to Int.\nShow first(d).\nShow length of c.\nShow length of m.\n";
    assert_eq!(
        kinds_of(src, &["xs", "c", "d", "m"]),
        [
            row("xs", false, true),
            row("c", false, true),
            row("d", false, true),
            row("m", false, false)
        ]
    );
}

/// Whether the rows of `c` are unaliased, by the kinds of the checked, splice-fused program `src`.
fn rows_of(src: &str) -> bool {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("the program parses");
        let (ea, sa) = (crate::arena::Arena::new(), crate::arena::Arena::new());
        let stmts =
            super::super::splice_fuse::fuse_place_splices(stmts.to_vec(), &ea, &sa, interner);
        let env = crate::analysis::check_program_located(&stmts, interner, registry)
            .expect("the program checks");
        of_program(&stmts, &env, interner).rows_unaliased(interner.lookup("c").expect("c is named"))
    })
}

const FILL: &str = "## Main\nLet mutable c be a new Seq of (Seq of Int).\nLet mutable i be 0.\nWhile i is less than 4:\n    Let mutable crow be a new Seq of Int.\n    Push i to crow.\n    Push crow to c.\nTAIL    Set i to i + 1.\nSet item 1 of (item 2 of c) to 9.\nShow item 1 of (item 2 of c).\nShow length of c.\n";

#[test]
fn rows_pushed_fresh_once_are_unaliased() {
    assert!(rows_of(&FILL.replace("TAIL", "")));
}

#[test]
fn a_row_pushed_twice_may_be_one_object() {
    assert!(!rows_of(&FILL.replace("TAIL", "    Push crow to c.\n")));
}

#[test]
fn a_row_read_into_a_binding_may_be_held_twice() {
    assert!(!rows_of(&FILL.replace("TAIL", "").replace(
        "Show length of c.\n",
        "Let r be item 1 of c.\nShow r.\n"
    )));
}

#[test]
fn a_row_used_after_its_push_is_still_held_by_its_binding() {
    assert!(!rows_of(&FILL.replace("TAIL", "    Push 7 to crow.\n")));
}

#[test]
fn a_row_stored_whole_is_not_known_to_be_fresh() {
    assert!(!rows_of(&FILL.replace("TAIL", "").replace(
        "Show length of c.\n",
        "Let mutable z be a new Seq of Int.\nSet item 1 of c to z.\nShow length of c.\n"
    )));
}
