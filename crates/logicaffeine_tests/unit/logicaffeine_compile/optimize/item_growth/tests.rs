use super::*;
use crate::ui_bridge::with_parsed_program;

/// The plan for the counted loop whose counter is `counter`, at the top level of `src`'s `## Main`.
fn plan_of(src: &str, counter: &str) -> Option<Plan> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.unwrap_or_else(|advice| panic!("the program parses: {advice}"));
        let oracle = crate::optimize::oracle_analyze(stmts);
        let at = (0..stmts.len().saturating_sub(1))
            .rev()
            .find(|&i| {
                matches!(&stmts[i], Stmt::Let { var, .. } | Stmt::Set { target: var, .. }
                    if interner.resolve(*var) == counter)
                    && matches!(stmts[i + 1], Stmt::While { .. })
            })
            .unwrap_or_else(|| panic!("a counted loop over `{counter}`"));
        plan(&stmts[..at], &stmts[at], &stmts[at + 1], &oracle)
    })
}

const COUNTS: &str = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("1000").
Let mutable counts be a new Seq of Int.
Let mutable k be 0.
While k is less than 1000:
    Push 0 to counts.
    Set k to k + 1.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Let v be (seed / 65536) % 1000.
    Set item (v + 1) of counts to (item (v + 1) of counts) + 1.
    Set i to i + 1.
Show item 1 of counts.
"#;

const ROLLED: &str = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("50").
Let capacity be 200.
Let mutable vals be a new Seq of Int.
Let mutable weights be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 50 + 1 to weights.
    Push (seed / 65536) % 100 + 1 to vals.
    Set i to i + 1.
Let mutable prev be a new Seq of Int.
Set i to 0.
While i is at most capacity:
    Push 0 to prev.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable curr be a new Seq of Int.
    Let wi be item (i + 1) of weights.
    Let vi be item (i + 1) of vals.
    Let mutable w be 0.
    While w is at most capacity:
        Let mutable best be item (w + 1) of prev.
        If w is at least wi:
            Let take be item (w - wi + 1) of prev + vi.
            If take is greater than best:
                Set best to take.
        Push best to curr.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#;

#[test]
fn a_count_kept_in_a_sequence_grows_one_per_pass() {
    let plan = plan_of(COUNTS, "i").expect("the count's increment is planned");
    assert_eq!(
        plan.limit_at_most,
        i64::MAX,
        "an item that grows by one per pass from zero stays a word for every pass a word limit allows"
    );
    assert!(!plan.raw.is_empty(), "the increment goes raw: {plan:?}");
}

#[test]
fn a_capacity_hint_plans_what_its_sequence_would_without_one() {
    let hinted = |src: &str, seq: &str, capacity: &str| {
        let from = format!("Let mutable {seq} be a new Seq of Int.");
        assert!(src.contains(&from), "the fixture creates `{seq}`");
        src.replace(&from, &format!("Let mutable {seq} be a new Seq of Int with capacity {capacity}."))
    };
    let shape = |plan: Option<Plan>| plan.map(|p| (p.limit_at_most, p.raw.len()));
    assert_eq!(
        shape(plan_of(&hinted(COUNTS, "counts", "1000"), "i")),
        shape(plan_of(COUNTS, "i")),
        "a hinted count table grows as the plain one does"
    );
    let rolled = hinted(&hinted(ROLLED, "prev", "capacity + 1"), "curr", "capacity + 1");
    assert_eq!(
        shape(plan_of(&rolled, "i")),
        shape(plan_of(ROLLED, "i")),
        "hinted rolled rows grow as the plain ones do"
    );
    assert!(shape(plan_of(&rolled, "i")).is_some(), "the hinted row is planned");
}

#[test]
fn a_rolled_row_grows_by_its_largest_increment_per_pass() {
    let plan = plan_of(ROLLED, "i").expect("the row's sum is planned");
    // Items start at 0 and grow by at most 100 (the largest value) per pass; `item + vi` on pass t is at most
    // 100·t + 100, a word while t <= (i64::MAX - 100) / 100.
    let passes = (i64::MAX as i128 - 100) / 100 + 1;
    assert_eq!(plan.limit_at_most as i128, passes);
    assert!(!plan.raw.is_empty());
}

#[test]
fn a_doubling_item_has_no_linear_bound() {
    for update in [
        "(item (v + 1) of counts) * 2",
        "(item (v + 1) of counts) + (item (v + 1) of counts)",
    ] {
        let src = COUNTS
            .replace("Push 0 to counts", "Push 1 to counts")
            .replace("(item (v + 1) of counts) + 1", update);
        assert!(plan_of(&src, "i").is_none(), "{update} doubles an item per pass");
    }
}

#[test]
fn a_sequence_any_call_receives_is_not_followed() {
    let src = COUNTS
        .replace("## Main\n", "## To touch (xs: Seq of Int) -> Int:\n    Return 0.\n\n## Main\n")
        .replace("    Set i to i + 1.\nShow", "    Let z be touch(counts).\n    Set i to i + 1.\nShow");
    assert!(plan_of(&src, "i").is_none());
}

#[test]
fn an_increment_a_nested_loop_repeats_is_refused() {
    let src = COUNTS.replace(
        "    Set item (v + 1) of counts to (item (v + 1) of counts) + 1.\n",
        "    Let mutable r be 0.\n    While r is less than 3:\n        Set item (v + 1) of counts to (item (v + 1) of counts) + 1.\n        Set r to r + 1.\n",
    );
    assert!(plan_of(&src, "i").is_none(), "each pass could add three");
}

#[test]
fn a_negative_increment_is_refused() {
    let src = COUNTS.replace("(item (v + 1) of counts) + 1", "(item (v + 1) of counts) - 1");
    assert!(plan_of(&src, "i").is_none(), "the items fall without a bound below");
}

#[test]
fn an_inclusive_limit_runs_one_pass_more() {
    let exclusive = plan_of(ROLLED, "i").unwrap().limit_at_most;
    let src = ROLLED.replace(
        "Set i to 0.\nWhile i is less than n:\n    Let mutable curr",
        "Set i to 0.\nWhile i is at most n:\n    Let mutable curr",
    );
    assert_eq!(plan_of(&src, "i").unwrap().limit_at_most, exclusive - 1);
}

#[test]
fn a_line_and_a_span_keep_their_arithmetic() {
    let s = Span {
        lo: Line { c: 1, s: 0 },
        hi: Line { c: 100, s: 100 },
    };
    assert_eq!(s.passes_within_word(), Some((i64::MAX as i128 - 100) / 100 + 1));
    assert_eq!(Span::level(-5, 5).passes_within_word(), Some(i128::MAX));
    assert_eq!(Span::level(0, i64::MAX as i128 + 1).passes_within_word(), None);
    let falling = Span {
        lo: Line { c: 0, s: -2 },
        hi: Line { c: 0, s: 0 },
    };
    assert_eq!(falling.passes_within_word(), Some((-(i64::MIN as i128)) / 2 + 1));
    let a = Span::level(1, 3);
    let b = Span {
        lo: Line { c: 0, s: 1 },
        hi: Line { c: 10, s: 2 },
    };
    assert_eq!(
        a.add(b).unwrap(),
        Span {
            lo: Line { c: 1, s: 1 },
            hi: Line { c: 13, s: 2 }
        }
    );
    assert_eq!(
        a.sub(b).unwrap(),
        Span {
            lo: Line { c: -9, s: -2 },
            hi: Line { c: 3, s: -1 }
        }
    );
    assert_eq!(a.mul(b), None, "a moving factor makes a curve");
    assert_eq!(a.mul(Span::level(-2, 4)).unwrap(), Span::level(-6, 12));
    assert_eq!(
        a.join(b),
        Span {
            lo: Line { c: 0, s: 0 },
            hi: Line { c: 10, s: 2 }
        }
    );
}
