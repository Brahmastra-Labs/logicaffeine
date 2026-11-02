//! Swap classes, pinned at the unit level: which names one class holds, where a swap is found, and what closing and
//! excluding a claim set leave behind.
use super::*;
use crate::ui_bridge::with_parsed_program;

/// Run `f` over `src`'s statements and interner.
fn with_program<R>(src: &str, f: impl for<'a> FnOnce(&'a [Stmt<'a>], &Interner) -> R) -> R {
    with_parsed_program(src, |parsed, interner| match parsed {
        Ok((stmts, _, _)) => f(stmts, interner),
        Err(advice) => panic!("test program must parse: {advice}"),
    })
}

fn sorted_class(classes: &SwapClasses, name: &str) -> Vec<String> {
    let mut c = classes.class_of(name).to_vec();
    c.sort();
    c
}

#[test]
fn chained_swaps_share_one_class_and_separate_swaps_do_not() {
    let src = "## Main\nLet mutable a be 1.\nLet mutable b be 2.\nLet mutable c be 3.\nLet mutable d be 4.\n\
               Let mutable e be 5.\nSwap a and b.\nSwap b and c.\nSwap d and e.\n";
    with_program(src, |stmts, interner| {
        let classes = SwapClasses::of(stmts, interner);
        assert_eq!(sorted_class(&classes, "a"), ["a", "b", "c"]);
        assert_eq!(sorted_class(&classes, "c"), ["a", "b", "c"]);
        assert_eq!(sorted_class(&classes, "d"), ["d", "e"]);
        assert!(classes.class_of("z").is_empty());
        assert!(!classes.is_swapped("z"));
    });
}

#[test]
fn a_swap_in_a_branch_a_loop_or_a_closure_is_found() {
    let src = "## Main\nLet mutable a be 1.\nLet mutable b be 2.\nLet mutable c be 3.\nLet mutable d be 4.\n\
               Let mutable e be 5.\nLet mutable f be 6.\nIf a equals 1:\n    Swap a and b.\n\
               While c is less than 0:\n    Swap c and d.\nLet g be (n: Int) ->:\n    Swap e and f.\n    Return n.\n";
    with_program(src, |stmts, interner| {
        let classes = SwapClasses::of(stmts, interner);
        assert_eq!(sorted_class(&classes, "a"), ["a", "b"]);
        assert_eq!(sorted_class(&classes, "c"), ["c", "d"]);
        assert_eq!(sorted_class(&classes, "e"), ["e", "f"]);
    });
}

#[test]
fn a_swap_inside_a_function_is_not_the_enclosing_scopes() {
    let src = "## To f (n: Int) -> Int:\n    Let mutable a be n.\n    Let mutable b be 1.\n    Swap a and b.\n    Return a.\n\
               ## Main\nLet mutable a be 1.\nLet mutable b be 2.\nShow f(a + b).\n";
    with_program(src, |stmts, interner| {
        assert!(
            SwapClasses::of(stmts, interner).is_empty(),
            "the function body is its own scope"
        );
        let body = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { body, .. } => Some(*body),
                _ => None,
            })
            .expect("a function definition");
        assert_eq!(
            sorted_class(&SwapClasses::of(body, interner), "a"),
            ["a", "b"]
        );
    });
}

#[test]
fn a_name_swapped_with_itself_is_swapped_alone() {
    let src = "## Main\nLet mutable a be 1.\nSwap a and a.\n";
    with_program(src, |stmts, interner| {
        let classes = SwapClasses::of(stmts, interner);
        assert!(classes.is_swapped("a"));
        assert_eq!(classes.class_of("a"), ["a"]);
    });
}

/// `(a, b, c, d)` as interned by a program that chains `a`–`b`–`c` and leaves `d` alone.
fn chain_program<R>(f: impl FnOnce(&SwapClasses, [Symbol; 4], &Interner) -> R) -> R {
    let src = "## Main\nLet mutable a be 1.\nLet mutable b be 2.\nLet mutable c be 3.\nLet mutable d be 4.\n\
               Swap a and b.\nSwap b and c.\n";
    with_program(src, |stmts, interner| {
        let classes = SwapClasses::of(stmts, interner);
        let sym = |n: &str| interner.lookup(n).expect("interned");
        f(&classes, [sym("a"), sym("b"), sym("c"), sym("d")], interner)
    })
}

#[test]
fn a_class_claimed_whole_and_alike_keeps_its_claims() {
    chain_program(|classes, [a, b, c, d], interner| {
        let mut claims: HashMap<Symbol, usize> =
            [(a, 3), (b, 3), (c, 3), (d, 9)].into_iter().collect();
        classes.close(&mut claims, interner);
        assert_eq!(claims.len(), 4);
    });
}

#[test]
fn one_member_claimed_differently_drops_the_whole_class() {
    chain_program(|classes, [a, b, c, d], interner| {
        let mut claims: HashMap<Symbol, usize> =
            [(a, 3), (b, 3), (c, 1), (d, 9)].into_iter().collect();
        classes.close(&mut claims, interner);
        assert_eq!(
            claims.into_iter().collect::<Vec<_>>(),
            [(d, 9)],
            "only the unswapped claim survives"
        );
    });
}

#[test]
fn one_member_left_unclaimed_drops_the_whole_class() {
    chain_program(|classes, [a, b, _c, d], interner| {
        let mut claims: HashSet<Symbol> = [a, b, d].into_iter().collect();
        classes.close_set(&mut claims, interner);
        assert_eq!(claims.into_iter().collect::<Vec<_>>(), [d]);
    });
}

#[test]
fn close_by_compares_only_what_it_is_told_to() {
    chain_program(|classes, [a, b, c, _d], interner| {
        let mut claims: HashMap<Symbol, (usize, &str)> =
            [(a, (3, "first")), (b, (3, "second")), (c, (3, "third"))]
                .into_iter()
                .collect();
        classes.close_by(&mut claims, interner, |x, y| x.0 == y.0);
        assert_eq!(
            claims.len(),
            3,
            "the lengths agree, and the length is all that is compared"
        );
    });
}

#[test]
fn exclusion_drops_every_swapped_name_and_nothing_else() {
    chain_program(|classes, [a, b, c, d], interner| {
        let mut claims: HashMap<Symbol, ()> =
            [(a, ()), (b, ()), (c, ()), (d, ())].into_iter().collect();
        classes.exclude(&mut claims, interner);
        assert_eq!(claims.into_keys().collect::<Vec<_>>(), [d]);
        let mut set: HashSet<Symbol> = [a, d].into_iter().collect();
        classes.exclude_set(&mut set, interner);
        assert_eq!(set.into_iter().collect::<Vec<_>>(), [d]);
    });
}

#[test]
fn a_scope_without_swaps_leaves_every_claim_alone() {
    let src = "## Main\nLet mutable a be 1.\nLet mutable b be 2.\nSet a to b.\n";
    with_program(src, |stmts, interner| {
        let classes = SwapClasses::of(stmts, interner);
        assert!(classes.is_empty());
        let a = interner.lookup("a").expect("interned");
        let mut claims: HashMap<Symbol, usize> = [(a, 1)].into_iter().collect();
        classes.close(&mut claims, interner);
        classes.exclude(&mut claims, interner);
        assert_eq!(claims.len(), 1);
    });
}
