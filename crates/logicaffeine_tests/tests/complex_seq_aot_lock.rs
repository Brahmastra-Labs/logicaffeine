//! `Seq of ComplexF` is a first-class typed sequence across EVERY tier — including the Rust-AOT
//! backend. This locks the AOT wiring that makes it so: a `[complex(f, f), ...]` literal infers
//! `LogosSeq<LogosComplexF>`; `item i of xs` over a ComplexF seq (a local, a literal, or a borrow
//! `&[LogosComplexF]` / scalar `ComplexF` PARAMETER) routes `+ − × ÷` / `conjugate` through the
//! `LogosComplexF` method path instead of the (type-mismatched) float operators; a ComplexF seq
//! can be BUILT (`Set item i of result to …`) and RETURNED. Without this the emitted Rust fails to
//! compile (`cannot add/multiply LogosComplexF`), so `assert_compiled_equals_interpreted` is the gate.
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted};

#[test]
fn complexf_seq_literal_index_and_arith() {
    let src = "\
## Main
Let xs be [complex(1.0, 2.0), complex(3.0, 4.0), complex(0.0, -1.0)].
Show item 1 of xs.
Show (item 1 of xs) + (item 2 of xs).
Show (item 1 of xs) * conjugate(item 1 of xs).
";
    assert_all_tiers(src, "1+2i\n4+6i\n5");
    assert_compiled_equals_interpreted(src);
}

#[test]
fn complexf_dot_over_seq_params() {
    // Params `x, y : Seq of ComplexF` become `&[LogosComplexF]`; `(item i of x)*(item i of y)`
    // must take the ComplexF method path across the borrow. (1+2i)(5+6i)+(3+4i)(7+8i) = -18+68i.
    let src = "\
## To cdot (n: Int, x: Seq of ComplexF, y: Seq of ComplexF) -> ComplexF:
    Let mutable sum be complex(0.0, 0.0).
    Repeat for i from 1 to n:
        Set sum to sum + (item i of x) * (item i of y).
    Return sum.

## Main
Let xs be [complex(1.0, 2.0), complex(3.0, 4.0)].
Let ys be [complex(5.0, 6.0), complex(7.0, 8.0)].
Show cdot(2, xs, ys).
";
    assert_all_tiers(src, "-18+68i");
    assert_compiled_equals_interpreted(src);
}

#[test]
fn complexf_seq_is_built_and_returned() {
    // A ComplexF scalar param `alpha`, a built-and-returned `Seq of ComplexF` result. 2·[1+2i,3+4i].
    let src = "\
## To cscal (n: Int, alpha: ComplexF, x: Seq of ComplexF) -> Seq of ComplexF:
    Let mutable result be x.
    Repeat for i from 1 to n:
        Set item i of result to alpha * (item i of result).
    Return result.

## Main
Let ys be cscal(2, complex(2.0, 0.0), [complex(1.0, 2.0), complex(3.0, 4.0)]).
Show ys.
";
    assert_all_tiers(src, "[2+4i, 6+8i]");
    assert_compiled_equals_interpreted(src);
}
