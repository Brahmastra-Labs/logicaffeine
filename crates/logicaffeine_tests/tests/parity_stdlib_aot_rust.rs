//! AOT-Rust tier lock for the pure-LOGOS stdlib prelude.
//!
//! The ordinary parity tests (`parity_*.rs`) run through `assert_all_tiers`, which proves
//! tree-walker ≡ VM only. That left a whole tier UNTESTED: the AOT-Rust backend splices an
//! ENTIRE referenced stdlib module into the emitted program (the C backend tree-shakes; the
//! Rust backend does not). So a SINGLE stdlib function that fails to compile as Rust poisons
//! EVERY program that pulls in its module — `Show gcd(48, 18)` stops compiling because some
//! unrelated function three definitions down emits ill-typed Rust. That failure mode is
//! invisible to `assert_all_tiers`, and it is exactly how the complex-transcendental wave
//! silently broke `math.lg` on the AOT-Rust tier (three distinct codegen faults: `parseFloat`
//! resolving to a bare unqualified call, `complex()`-returning functions declared `-> Complex`
//! but emitting `LogosComplexF`, and `produces_float` not recognizing the `-> Real`
//! transcendentals so `complex(sin(x), cos(y))` mis-selected the exact representation and
//! truncated the float parts through `as i64`).
//!
//! Each test below references one function from a compute module, which splices that whole
//! module, and asserts the emitted Rust COMPILES and agrees with the interpreter
//! (`assert_compiled_equals_interpreted` = the AOT-Rust path: LOGOS → Rust → rustc → run). One
//! trivial call per module is sufficient because the whole module is spliced — any
//! non-compiling function in it fails the module's test. This is the invariant that makes the
//! "runs byte-identically on every tier" promise honest for the stdlib on the Rust backend, and
//! it ratchets: adding a stdlib function that does not lower to valid Rust goes RED here.

#![allow(unused_imports, dead_code)]
mod common;
use common::assert_compiled_equals_interpreted;

/// One representative call per compute stdlib module. Referencing the function demand-loads and
/// splices the entire module; `assert_compiled_equals_interpreted` then rustc-compiles the
/// emitted program and checks it against the interpreter, so any ill-typed function anywhere in
/// the module fails this test.
macro_rules! aot_module_compiles {
    ($name:ident, $src:expr) => {
        #[test]
        fn $name() {
            assert_compiled_equals_interpreted($src);
        }
    };
}

aot_module_compiles!(math_lg_compiles_on_aot_rust, "## Main\nShow gcd(48, 18).\n");
aot_module_compiles!(
    strings_lg_compiles_on_aot_rust,
    "## Main\nShow startsWith(\"hello\", \"he\").\n"
);
aot_module_compiles!(
    iter_lg_compiles_on_aot_rust,
    "## Main\nShow sumSeq([1, 2, 3]).\n"
);
aot_module_compiles!(
    convert_lg_compiles_on_aot_rust,
    "## Main\nShow numText(42).\n"
);
aot_module_compiles!(bits_lg_compiles_on_aot_rust, "## Main\nShow popcount(7).\n");
aot_module_compiles!(
    operator_lg_compiles_on_aot_rust,
    "## Main\nShow opAdd(2, 3).\n"
);
aot_module_compiles!(
    bytes_lg_compiles_on_aot_rust,
    "## Main\nShow byteIsLowerC(97).\n"
);
aot_module_compiles!(
    chars_lg_compiles_on_aot_rust,
    "## Main\nShow charIsDigit(53).\n"
);
aot_module_compiles!(
    statistics_lg_compiles_on_aot_rust,
    "## Main\nShow floor(meanReal([2.0, 4.0])).\n"
);
aot_module_compiles!(
    sort_lg_compiles_on_aot_rust,
    "## Main\nShow isSortedSeq([1, 2, 3]).\n"
);
aot_module_compiles!(
    heap_lg_compiles_on_aot_rust,
    "## Main\nShow heapLen([1, 2, 3]).\n"
);
aot_module_compiles!(
    encoding_lg_compiles_on_aot_rust,
    "## Main\nShow toHex(\"A\").\n"
);
aot_module_compiles!(
    calendar_lg_compiles_on_aot_rust,
    "## Main\nShow isLeapYear(2024).\n"
);
aot_module_compiles!(
    builder_lg_compiles_on_aot_rust,
    "## Main\nShow builderString(builderWriteString(newBuilder(), \"hi\")).\n"
);

// ── Int->Real conversion functions. The "one call per module" tests above only force the
// FEW functions transitively reached from that call to compile — the stdlib is demand-loaded,
// so an UNCALLED stdlib function is never type-checked on ANY tier. That hole shipped a broken
// `toReal` (`n * 1.0` fails to type-check on a declared Int param; `n + 0.0` is the coercing
// form) and 15 sibling conversions, all undetected. This test explicitly references every one
// of them so the demand-compiler is forced to type-check + lower each on the AOT-Rust tier.
aot_module_compiles!(
    convert_to_real_functions_compile_on_aot_rust,
    "## Main
Show trunc(toReal(7)).
Show floor(modfInt(3.9)).
Show trunc(modfFrac(3.9) * 100.0).
Show trunc(logb(8.0)).
Show trunc(fmodReal(17.0, 5.0)).
Show trunc(remainderReal(17.0, 5.0)).
Show roundHalfEven(2.5).
Show trunc(roundEvenReal(2.5)).
Show trunc(divEuclid(17.0, 5.0)).
Show trunc(remEuclid(17.0, 5.0)).
Show lrint(2.5).
Show trunc(rint(2.5)).
Show trunc(meanInt([2, 4, 6])).
Show trunc(quantile([1.0, 2.0, 3.0, 4.0], 1, 2)).
Show trunc(ulp(1.0) * 1.0e16).
"
);

// ── Direct regression guards for the three codegen faults the complex wave exposed. Each is a
// standalone reproduction so a failure names the specific broken mechanism, not just "math.lg".

/// `parseFloat` must lower to the fully-qualified `logicaffeine_system::text::parseFloat` (it
/// lives in the `text` submodule, not the crate-root glob) — a bare call fails to resolve (E0425)
/// and poisons every module that composes `Int → Real` through it (`fmodReal`, `modfInt`, …).
#[test]
fn parsefloat_resolves_on_aot_rust() {
    assert_compiled_equals_interpreted("## Main\nShow floor(parseFloat(\"3.14\") * 100.0).\n");
    assert_compiled_equals_interpreted("## Main\nShow floor(fmodReal(7.5, 2.0) * 100.0).\n");
}

/// Every `complex()`-returning `cmath` function must lower to a type-consistent `LogosComplexF`
/// (its float parts are computed from the `-> Real` transcendentals) — not the exact
/// `LogosComplex`, which both mismatches the return type and truncates the float parts through
/// `as i64`. Called with `Real` arguments (the int-literal → `Real`-parameter call-site coercion
/// is a separate, pre-existing AOT gap tracked independently). Covers the WHOLE cmath surface so
/// the `aot_native` tier claim on the `complex.*` capabilities is proven per-function, not
/// extrapolated.
/// Cross-tier byte-identity for a `Real`-parameter function called with INTEGER-literal arguments.
/// The interpreter and VM coerce int→Real at the call boundary; the AOT-Rust backend must too. Before
/// the call-site `as f64` coercion, `sin(0)` and `cConj(3, 4)` failed to compile on AOT (i64 argument
/// to an `f64` parameter) while passing on tree-walker/VM — a silent tier divergence. Covers a stdlib
/// transcendental, a complex constructor, and a user-defined Real-parameter function.
#[test]
fn int_literal_to_real_param_coerces_on_aot_rust() {
    assert_compiled_equals_interpreted("## Main\nShow floor(sin(0) * 1000.0).\n");
    assert_compiled_equals_interpreted("## Main\nShow cConj(3, 4).\n");
    assert_compiled_equals_interpreted(
        "## To scaleHalf (x: Real) -> Real:\n    Return x / 2.0.\n## Main\nShow floor(scaleHalf(5) * 100.0).\n",
    );
    // Mixed Int/Real parameters: ONLY the Real parameter coerces — the Int parameter must stay i64
    // (coercing it would be `(2) as f64` where an i64 is wanted, a type error). Locks that the
    // coercion is keyed on each parameter's own type, not the call as a whole.
    assert_compiled_equals_interpreted(
        "## To mix (a: Int) and (b: Real) -> Real:\n    Return b.\n## Main\nShow floor(mix(1, 2) * 100.0).\n",
    );
}

#[test]
fn complex_returning_functions_lower_on_aot_rust() {
    for src in [
        "## Main\nShow cExp(0.0, 0.0).\n",
        "## Main\nShow cLog(1.0, 0.0).\n",
        "## Main\nShow cSqrt(4.0, 0.0).\n",
        "## Main\nShow cSin(0.0, 0.0).\n",
        "## Main\nShow cCos(0.0, 0.0).\n",
        "## Main\nShow cSinh(0.0, 0.0).\n",
        "## Main\nShow cCosh(0.0, 0.0).\n",
        "## Main\nShow cTan(1.0, 0.0).\n",
        "## Main\nShow cTanh(0.0, 0.0).\n",
        "## Main\nShow floor(cPhase(0.0, 1.0) * 1000.0).\n",
        "## Main\nShow cRect(2.0, 0.0).\n",
        "## Main\nShow cConj(3.0, 4.0).\n",
    ] {
        assert_compiled_equals_interpreted(src);
    }
}

/// Greedy word-wrap (`fillText`/`wrapText`) on the AOT-Rust tier. (The C backend aliases a mutable
/// Text pushed into a `Seq of Text`, so these caps claim tree-walker/vm/aot_native only — see the
/// `na_rationale` in the catalog; this locks the AOT-Rust tier they DO claim.)
#[test]
fn textwrap_wrap_fill_on_aot_rust() {
    assert_compiled_equals_interpreted("## Main\nShow fillText(\"one two three four\", 8).\n");
    assert_compiled_equals_interpreted(
        "## Main\nShow item 2 of wrapText(\"aa bb cc dd ee\", 5).\n",
    );
    assert_compiled_equals_interpreted(
        "## Main\nShow length of wrapText(\"The quick brown fox jumps\", 10).\n",
    );
}
