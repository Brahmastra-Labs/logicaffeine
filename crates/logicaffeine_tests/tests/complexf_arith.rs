mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted};

#[test]
fn native_float_complex_arithmetic_and_promotion() {
    // Native ComplexF: float args to `complex(...)` yield the floating-point complex, with +−×÷,
    // and an exact Complex meeting a float PROMOTES to ComplexF (the "wrap" up the tower).
    let src = "\
## Main
Let z be complex(3.0, 4.0).
Let w be complex(1.0, 2.0).
Show z + w.
Show z * w.
Show z - w.
Show z / w.
Let e be complex(2, 0).
Show e + complex(0.0, 1.0).
";
    // z/w is exactly 2.2 − 0.4i, but ComplexF is FLOATING-POINT and carries the f64 rounding
    // (…0.40000000000000013) — the inexactness the exact `Complex` avoids. The last line is the
    // promotion: an exact `Complex` meeting a float becomes `ComplexF` (2+i).
    assert_all_tiers(src, "4+6i\n-5+10i\n2+2i\n2.2-0.40000000000000013i\n2+i");
}

#[test]
fn native_float_complex_compiles_to_rust_and_matches_the_interpreter() {
    // The AOT tier: `complex(3.0, 4.0)` lowers to `LogosComplexF`, and `+ − × ÷` emit its methods.
    // The compiled Rust must produce byte-identical output to the interpreter — closing "every tier".
    let src = "\
## Main
Let z be complex(3.0, 4.0).
Let w be complex(1.0, 2.0).
Show z + w.
Show z * w.
Show z - w.
Show z / w.
";
    assert_compiled_equals_interpreted(src);
}

#[test]
fn complex_sqrt_and_abs_on_float_complex() {
    // The operations that unblock numerical complex decompositions: √(3+4i)=2+i (complex sqrt via
    // real sqrt), |3+4i|=5, and √(-1)=i — none of which the exact Gaussian complex can do.
    let src = "\
## Main
Let z be complex(3.0, 4.0).
Show sqrt(z).
Show abs(z).
Show sqrt(complex(-1.0, 0.0)).
";
    common::assert_all_tiers(src, "2+i\n5\ni");
}

#[test]
fn complex_sqrt_and_abs_compile_to_rust_and_match() {
    // AOT: sqrt/abs on a float-complex lower to LogosComplexF::sqrt / .abs(), and sqrt(z) chains into
    // ComplexF arithmetic. Compiled Rust must equal the interpreter.
    let src = "\
## Main
Let z be complex(3.0, 4.0).
Show sqrt(z).
Show abs(z).
Show sqrt(z) + complex(0.0, 0.0).
";
    assert_compiled_equals_interpreted(src);
}

#[test]
fn bound_complex_sqrt_reused_compiles_and_matches() {
    // Complex-Cholesky shape: `Let d be sqrt(..)` then reuse `d` in ComplexF arithmetic. The binding
    // must be AOT-typed `LogosComplexF` so `d + ..` and `d * d` take the ComplexF codegen path.
    let src = "\
## Main
Let d be sqrt(complex(3.0, 4.0)).
Show d.
Show d + complex(1.0, 0.0).
Show d * d.
";
    assert_compiled_equals_interpreted(src);
}

#[test]
fn conjugate_over_the_complex_tower_all_tiers() {
    // conjugate negates the imaginary part: exact `Complex` stays exact, `ComplexF` stays float, a
    // real is its own conjugate, and z·conj(z) = |z|² is real (25, shown without an imaginary part).
    let src = "\
## Main
Show conjugate(complex(3, 4)).
Show conjugate(complex(3.0, 4.0)).
Show conjugate(5).
Show complex(3.0, 4.0) * conjugate(complex(3.0, 4.0)).
";
    common::assert_all_tiers(src, "3-4i\n3-4i\n5\n25");
}

#[test]
fn conjugate_compiles_to_rust_and_matches_the_interpreter() {
    // AOT: conjugate lowers to `.conjugate()` on LogosComplex / LogosComplexF (real passes through),
    // and a BOUND conjugate result reused in ComplexF arithmetic keeps the ComplexF codegen path.
    let src = "\
## Main
Show conjugate(complex(3, 4)).
Show conjugate(complex(3.0, 4.0)).
Show complex(3.0, 4.0) * conjugate(complex(3.0, 4.0)).
Let c be conjugate(complex(2.0, -5.0)).
Show c + complex(0.0, 0.0).
";
    assert_compiled_equals_interpreted(src);
}
