use super::*;

/// Run `f` on the band whose first counter `counter` is set to 0 last in `## Main`, with the checked program's
/// kinds. The parse's place-write splices are fused first, as the optimizer does before anything reads a nest.
fn with_nest<R>(
    src: &str,
    counter: &str,
    f: impl FnOnce(&Nest, &super::super::kinds::Kinds, &crate::analysis::types::TypeEnv) -> R,
) -> R {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("the program parses");
        let (ea, sa) = (crate::arena::Arena::new(), crate::arena::Arena::new());
        let stmts =
            super::super::splice_fuse::fuse_place_splices(stmts.to_vec(), &ea, &sa, interner);
        let stmts = &stmts[..];
        let env = crate::analysis::check_program_located(stmts, interner, registry)
            .expect("the program checks");
        let kinds = super::super::kinds::of_program(stmts, &env, interner);
        let at = stmts
            .iter()
            .rposition(|s| match s {
                Stmt::Set {
                    target,
                    value: Expr::Literal(Literal::Number(0)),
                } => interner.resolve(*target) == counter,
                _ => false,
            })
            .expect("the nest's first counter is initialized");
        let nest = nest_at(stmts, at).expect("a band of counted loops");
        f(&nest, &kinds, &env)
    })
}

/// The tile verdict, rows unaliased by value semantics (`reference` false) or only where proven (`reference` true).
fn verdict_in(src: &str, reference: bool) -> String {
    with_nest(src, "i", |nest, kinds, env| {
        match tile(
            nest,
            &|e| kinds.int_valued(e, env),
            &|s| kinds.seq(s),
            &|s| !reference || kinds.rows_unaliased(s),
        ) {
            Legality::Always => "always".to_string(),
            Legality::When(pre) => format!("when {}", pre.len()),
            Legality::Never(why) => format!("never: {why}"),
        }
    })
}

fn verdict_of(src: &str) -> String {
    verdict_in(src, false)
}

/// The bounds verdict on the band whose counter is `counter`: the number of ranges and whether an index reads a
/// name other than a counter.
fn bounds_of(src: &str, counter: &str) -> String {
    with_nest(src, counter, |nest, kinds, env| {
        match bounds(
            nest,
            &|e| kinds.int_valued(e, env),
            &|s| kinds.seq(s),
            &|_, _| false,
        ) {
            Ok((pre, symbolic)) => format!("{} ranges, symbolic {symbolic}", pre.len()),
            Err(why) => format!("refused: {why}"),
        }
    })
}

const HEAD: &str =
    "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"40\").\n";

fn nest(body: &str) -> String {
    format!(
        "{HEAD}Let mutable a be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable h be 0.\nLet mutable t be 0.\nLet mutable i be 0.\nWhile i is less than n * n:\n    Push i % 7 to a.\n    Push 0 to c.\n    Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Let mutable k be 0.\n    While k is less than n:\n        Let mutable j be 0.\n        While j is less than n:\n{body}            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nShow h.\nShow t.\n"
    )
}

#[test]
fn a_matrix_product_may_run_in_tile_order_once_its_indices_are_in_range() {
    let body = "            Let idx be i * n + j + 1.\n            Set item idx of c to (item idx of c) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of a).\n";
    assert_eq!(
        verdict_of(&nest(body)),
        "when 4",
        "c read, c stored, a twice: one range each"
    );
}

#[test]
fn a_nest_folding_into_a_hash_may_not() {
    let body = "            Set h to (h * 31 + i * 1089 + k * 33 + j) % 1000000007.\n";
    assert!(
        verdict_of(&nest(body)).starts_with("never"),
        "the hash reads its own earlier value in order"
    );
}

#[test]
fn a_nest_storing_a_running_count_may_not() {
    let body = "            Let idx be i * n + j + 1.\n            Set t to t + 1.\n            Set item idx of c to t.\n";
    assert!(
        verdict_of(&nest(body)).starts_with("never"),
        "the stored count depends on the order"
    );
}

#[test]
fn an_int_sum_is_a_dependence_until_every_tier_is_exact() {
    let body = "            Set h to h + (item (i * n + k + 1) of a).\n";
    assert_eq!(
        verdict_of(&nest(body)),
        "never: a variable carried from one iteration to the next"
    );
}

#[test]
fn a_real_sum_never_reorders() {
    let body = "            Set h to h + 0.5.\n";
    assert!(
        verdict_of(&nest(body)).starts_with("never"),
        "a Real sum rounds differently in another order"
    );
}

#[test]
fn two_iterations_that_swap_cells_are_a_dependence() {
    let body = "            Set item (i * n + j + 1) of c to item (j * n + i + 1) of c.\n";
    assert!(
        verdict_of(&nest(body)).starts_with("never"),
        "a transpose in place reads what another iteration wrote"
    );
}

#[test]
fn a_divisor_fixed_before_the_nest_is_a_precondition() {
    let body = "            Let idx be i * n + j + 1.\n            Set item idx of c to (item idx of c) + k / n.\n";
    assert_eq!(verdict_of(&nest(body)), "when 3", "c twice, and n nonzero");
}

#[test]
fn a_shown_value_is_never_reordered() {
    let body = "            Show i.\n";
    assert!(verdict_of(&nest(body)).starts_with("never"));
}

#[test]
fn an_index_reading_a_value_the_band_changes_is_not_placed() {
    let body = "            Let mutable idx be i * n + j + 1.\n            Set idx to idx + 0.\n            Set item idx of c to 1.\n";
    assert_eq!(
        verdict_of(&nest(body)),
        "never: an index that reads a value the band changes"
    );
}

#[test]
fn a_bound_that_may_be_real_keeps_the_written_order() {
    let src = format!(
        "{HEAD}Let mutable c be a new Seq of Int.\nLet mutable x be 3.\nSet x to 2.5.\nLet mutable i be 0.\nSet i to 0.\nWhile i is less than x:\n    Let mutable j be 0.\n    While j is less than n:\n        Set item (j + 1) of c to i.\n        Set j to j + 1.\n    Set i to i + 1.\nShow c.\n"
    );
    assert_eq!(verdict_of(&src), "never: a bound that is not an Int");
}

#[test]
fn a_map_is_not_a_sequence() {
    let src = format!(
        "{HEAD}Let mutable c be a new Map of Int to Int.\nLet mutable i be 0.\nSet i to 0.\nWhile i is less than n:\n    Let mutable j be 0.\n    While j is less than n:\n        Set item (i * n + j + 1) of c to 1.\n        Set j to j + 1.\n    Set i to i + 1.\nShow length of c.\n"
    );
    assert_eq!(
        verdict_of(&src),
        "never: an access or a divisor of a kind this test does not place"
    );
}

const ROWS: &str = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"8\").\nLet mutable a be a new Seq of (Seq of Int).\nLet mutable c be a new Seq of (Seq of Int).\nLet mutable i be 0.\nWhile i is less than n * n:\n    Let mutable arow be a new Seq of Int.\n    Push i % 7 to arow.\n    Push arow to a.\n    Let mutable crow be a new Seq of Int.\n    Push 0 to crow.\n    Push crow to c.\n    Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Let mutable k be 0.\n    While k is less than n:\n        Let mutable j be 0.\n        While j is less than n:\n            Let idx be i * n + j + 1.\n            Set item 1 of (item idx of c) to (item 1 of (item idx of c)) + (item 1 of (item (i * n + k + 1) of a)) * (item 1 of (item (k * n + j + 1) of a)).\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nShow item 1 of (item 1 of c).\n";

#[test]
fn a_product_through_rows_runs_in_tile_order_when_no_two_rows_are_one_object() {
    assert_eq!(
        verdict_in(ROWS, false),
        "when 4",
        "value semantics: each row index ranged, each item 1 left checked"
    );
    assert_eq!(
        verdict_in(ROWS, true),
        "when 4",
        "reference semantics: every row is pushed fresh, once"
    );
}

#[test]
fn a_row_pushed_twice_may_be_one_object_under_reference_semantics() {
    let shared = ROWS.replace(
        "    Push crow to c.\n",
        "    Push crow to c.\n    Push arow to c.\n",
    );
    assert!(
        verdict_in(&shared, true).starts_with("never: rows of a sequence that may be one object"),
        "{}",
        verdict_in(&shared, true)
    );
}

const ONE_LOOP: &str = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"8\").\nLet m be parseInt(\"5\").\nLet i be parseInt(\"3\").\nLet mutable a be a new Seq of Int.\nLet mutable h be 0.\nLet mutable q be 0.\nWhile q is less than n * n:\n    Push q to a.\n    Set q to q + 1.\nLet mutable j be 0.\nSet j to 0.\nWhile j is less than m:\n    BODY\n    Set j to j + 1.\nShow h.\n";

#[test]
fn a_loop_reading_through_an_offset_is_bounded_by_its_corners() {
    let src = ONE_LOOP.replace("BODY", "Set h to h + item (i * n + j + 1) of a.");
    assert_eq!(bounds_of(&src, "j"), "1 ranges, symbolic true");
}

#[test]
fn a_loop_indexing_by_its_counter_alone_is_left_to_the_oracle() {
    let src = ONE_LOOP.replace("BODY", "Set h to h + item (j + 1) of a.");
    assert_eq!(bounds_of(&src, "j"), "1 ranges, symbolic false");
}

#[test]
fn a_loop_that_grows_the_sequence_it_reads_is_not_bounded() {
    let src = ONE_LOOP.replace(
        "BODY",
        "Set h to h + item (i * n + j + 1) of a.\n    Push h to a.",
    );
    assert_eq!(
        bounds_of(&src, "j"),
        "refused: a sequence the loop resizes or rebinds"
    );
}
