//! Native-tier lock for the IEEE float-classification predicates (`isNaN`/`isInf`/`isFinite`).
//!
//! These are pure-LOGOS (`isNaN(x) = not(x==x)`, `isFinite(x) = not isNaN(x-x)`), so their
//! correctness on the compiled tiers hinges on the backend NOT folding `x-x`/`x==x` and on float
//! overflow producing IEEE `inf`/`nan`. It also guards the C-backend `NAN`/`INFINITY` literal emit
//! (a folded non-finite float constant used to emit the invalid C token `NaN`). `assert_all_tiers`
//! only checks tree-walker≡VM, so this pins AOT-Rust and C byte-for-byte.
#![allow(unused_imports, dead_code)]
mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted};

// inf via overflow (1e308 * 1e308), nan via inf - inf — LOGOS errors on 0.0/0.0 by design.
const SRC: &str = "## Main\nLet hugev be 1.0e308.\nLet infv be hugev * hugev.\nLet nanv be infv - infv.\nShow isNaN(nanv).\nShow isInf(infv).\nShow isFinite(3.5).\nShow isFinite(infv).\nShow isFinite(nanv).\n";

#[test]
fn float_predicates_on_aot_rust() {
    assert_compiled_equals_interpreted(SRC);
}

#[test]
fn float_predicates_on_c() {
    assert_c_output(SRC, "true\ntrue\ntrue\nfalse\nfalse");
}
