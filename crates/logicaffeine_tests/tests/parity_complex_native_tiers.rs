//! Full-audit + regression lock for the FLOATING-POINT complex tier on the native backends.
//! The `c*` transcendentals build `complex(realExpr, realExpr)` (a `ComplexF`); before the C
//! backend grew `cplx_reals`/`rat_from_f64` + the AOT-Rust `COMPLEXF_RETURNING_MATH_FNS` recognizer,
//! this whole family silently failed to lower to C (and mis-lowered on AOT-Rust through `as i64`/
//! `as f64`). These were only ever tested tw/VM. This pins them on AOT-Rust + C so the fix can't rot.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted};

const ALL: &str = "## Main
Show cExp(0.0, 0.0).
Show cLog(1.0, 0.0).
Show cSqrt(4.0, 0.0).
Show cSin(0.0, 0.0).
Show cCos(0.0, 0.0).
Show cSinh(0.0, 0.0).
Show cCosh(0.0, 0.0).
Show cTan(0.0, 0.0).
Show cTanh(0.0, 0.0).
Show cRect(2.0, 0.0).
Show cConj(3, 4).
Show floor(re(cSqrt(2.0, 0.0)) * 1000.0).
Show floor(cPhase(1.0, 1.0) * 1000.0).
";

#[test]
fn complex_transcendentals_on_aot_rust() {
    assert_compiled_equals_interpreted(ALL);
}

#[test]
fn complex_transcendentals_on_c() {
    // sqrt(2)=1.41421… → floor(re·1000)=1414. cConj(3,4)=3-4i. The rest reduce to exact 0/1/2.
    assert_c_output(ALL, "1\n0\n2\n0\n1\n0\n1\n0\n0\n2\n3-4i\n1414\n785");
}
