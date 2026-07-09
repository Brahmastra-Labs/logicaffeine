//! Wave 5 stage-2: storable BigInt bindings. The exact-arithmetic ruling says
//! integer overflow promotes to BigInt EVERYWHERE — including when the promoted
//! value is stored in a binding, an accumulator, or passed to a function. The
//! tree-walker already holds unbounded integers; the compiled AOT must match
//! (today it types every Int binding `i64` and PANICS on overflow).

mod common;
use common::assert_compiled_equals_interpreted_eq as eq;

#[test]
fn constant_bigint_binding_then_show() {
    eq(
        "## Main\nLet big be 2 ** 100.\nShow big.\n",
        "1267650600228229401496703205376",
    );
}

#[test]
fn bigint_binding_used_in_arithmetic() {
    // 2^100 * 2^100 = 2^200, exact.
    eq(
        "## Main\nLet big be 2 ** 100.\nLet squared be big * big.\nShow squared.\n",
        "1606938044258990275541962092341162602522202993782792835301376",
    );
}

#[test]
fn factorial_accumulator_promotes() {
    // 25! overflows i64 partway through — the accumulator must promote.
    eq(
        "## Main\nLet p be 1.\nRepeat for i from 1 to 25:\n    Set p to p * i.\nShow p.\n",
        "15511210043330985984000000",
    );
}

#[test]
fn bigint_accumulator_via_addition() {
    // Sum grows past i64 by repeated doubling: start 1, double 70 times → 2^70.
    eq(
        "## Main\nLet x be 1.\nRepeat for i from 1 to 70:\n    Set x to x + x.\nShow x.\n",
        "1180591620717411303424",
    );
}

#[test]
fn small_int_binding_stays_exact_and_correct() {
    // A binding that never overflows must still show correctly (fast i64 path,
    // no behavior change).
    eq("## Main\nLet n be 6 * 7.\nShow n.\n", "42");
}

#[test]
fn bigint_binding_compared_stays_correct() {
    // A promoted binding compared against another value: 2^100 > 1000 is True.
    eq(
        "## Main\nLet big be 2 ** 100.\nIf big is greater than 1000:\n    Show \"huge\".\nOtherwise:\n    Show \"small\".\n",
        "huge",
    );
}

// ---- Function returns (the `-> LogosInt` half) ----

#[test]
fn factorial_function_returns_bigint() {
    // A function whose accumulator overflows i64 must return the exact BigInt.
    eq(
        "## To factorial (n: Int) -> Int:\n    Let mutable result be 1.\n    Let mutable i be 1.\n    While i is at most n:\n        Set result to result * i.\n        Set i to i + 1.\n    Return result.\n## Main\nShow factorial(25).\n",
        "15511210043330985984000000",
    );
}

#[test]
fn bigint_fn_result_stored_and_shown() {
    // The caller binds the bignum-returning call result and shows it.
    eq(
        "## To factorial (n: Int) -> Int:\n    Let mutable result be 1.\n    Let mutable i be 1.\n    While i is at most n:\n        Set result to result * i.\n        Set i to i + 1.\n    Return result.\n## Main\nLet f be factorial(30).\nShow f.\n",
        "265252859812191058636308480000000",
    );
}

#[test]
fn promoted_var_within_i64_passed_to_scalar_param() {
    // `p` is promoted (a running product) but stays within i64 (5! = 120); passing it to a
    // scalar `Int` param narrows to i64 — matching the tree-walker for the in-range value.
    eq(
        "## To double (n: Int) -> Int:\n    Return n * 2.\n## Main\nLet mutable p be 1.\nRepeat for i from 1 to 5:\n    Set p to p * i.\nShow double(p).\n",
        "240",
    );
}

#[test]
fn small_factorial_still_exact() {
    // A bignum-returning function called with a small argument still gives the exact value.
    eq(
        "## To factorial (n: Int) -> Int:\n    Let mutable result be 1.\n    Let mutable i be 1.\n    While i is at most n:\n        Set result to result * i.\n        Set i to i + 1.\n    Return result.\n## Main\nShow factorial(5).\n",
        "120",
    );
}
