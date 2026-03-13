mod common;

use common::{compile_to_rust, assert_exact_output, run_logos};

// =============================================================================
// Sprint 2.1 — Specialization Mechanics (8 tests)
// =============================================================================

#[test]
fn pe_creates_specialized_function() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let n be parseInt("7").
Let y be multiply(3, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("multiply_s0"),
        "PE should create a specialized function named like multiply_s0_*.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_static_params_removed() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let n be parseInt("7").
Let y be multiply(3, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    let specialized = rust.lines()
        .find(|line| line.contains("multiply_s0") && line.contains("fn "));
    assert!(
        specialized.is_some(),
        "Specialized function definition should exist.\nGot:\n{}",
        rust
    );
    let sig = specialized.unwrap();
    let param_count = sig.matches(',').count() + if sig.contains('(') && !sig.contains("()") { 1 } else { 0 };
    assert_eq!(
        param_count, 1,
        "Specialized multiply_s0_* should have ONE parameter (b), not two.\nSignature: {}",
        sig
    );
}

#[test]
fn pe_body_substituted() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let n be parseInt("7").
Let y be multiply(3, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    let in_specialized = rust.lines()
        .skip_while(|line| !line.contains("multiply_s0"))
        .take(5)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        in_specialized.contains("3 *") || in_specialized.contains("3i64 *") || in_specialized.contains("(3)"),
        "Specialized body should contain literal 3 from static param a.\nSpecialized region:\n{}",
        in_specialized
    );
}

#[test]
fn pe_specialized_name_format() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To scale (factor: Int, x: Int) -> Int:
    Return factor * x.

## Main
Let n be parseInt("5").
Let y be scale(2, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("scale_s0"),
        "Specialized name should contain 'scale' and encode static arg position.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_call_site_rewritten() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let n be parseInt("7").
Let y be multiply(3, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    let main_section = rust.lines()
        .skip_while(|line| !line.contains("fn main"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        main_section.contains("multiply_s0"),
        "Call site in main should be rewritten to use specialized function.\nMain section:\n{}",
        main_section
    );
}

#[test]
fn pe_fold_runs_on_specialized_body() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int, b: Int) -> Int:
    Let c be a + 1.
    Return c * b.

## Main
Let n be parseInt("7").
Let y be f(4, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    let specialized = rust.lines()
        .skip_while(|line| !line.contains("f_s0"))
        .take(10)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        specialized.contains("5 *") || specialized.contains("5i64 *") || specialized.contains("(5)"),
        "After substitution a=4, fold should simplify c=4+1=5. Body should contain 5*b.\nSpecialized:\n{}",
        specialized
    );
}

#[test]
fn pe_dce_runs_on_specialized_body() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To select (flag: Bool, a: Int, b: Int) -> Int:
    If flag:
        Return a.
    Otherwise:
        Return b.

## Main
Let x be parseInt("10").
Let z be parseInt("20").
Let y be select(true, x, z).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    let specialized = rust.lines()
        .skip_while(|line| !line.contains("select_s0"))
        .take(10)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !specialized.is_empty(),
        "Specialized function select_s0_* should exist.\nGot:\n{}",
        rust
    );
    assert!(
        !specialized.contains("if ") && !specialized.contains("else"),
        "After substitution flag=true, DCE should eliminate if/else. Body should be just Return a.\nSpecialized:\n{}",
        specialized
    );
}

#[test]
fn pe_simplicity_check() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To dispatch (mode: Int, x: Int) -> Int:
    If mode equals 1:
        Return x + 10.
    If mode equals 2:
        Return x + 20.
    If mode equals 3:
        Return x + 30.
    Return x.

## Main
Let n be parseInt("5").
Let y be dispatch(1, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("dispatch_s0"),
        "Simplicity check should PASS — specialized is much smaller (1 branch vs 4).\nGot:\n{}",
        rust
    );
}

// =============================================================================
// Sprint 2.2 — Memoization (5 tests)
// =============================================================================

#[test]
fn pe_same_key_reuses() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let x be parseInt("3").
Let y be parseInt("5").
Let r1 be f(3, x).
Let r2 be f(3, y).
Show r1 + r2.
"#;
    let rust = compile_to_rust(source).unwrap();
    let def_count = rust.lines()
        .filter(|line| line.contains("fn f_s0") && line.contains("("))
        .count();
    assert_eq!(
        def_count, 1,
        "Same key f(3, _) called twice should reuse ONE specialized function.\nGot:\n{}",
        rust
    );
    let call_count = rust.lines()
        .filter(|line| line.contains("f_s0") && !line.contains("fn f_s0"))
        .count();
    assert!(
        call_count >= 2,
        "Both call sites should use the specialized function.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_different_key_creates_new() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let x be parseInt("3").
Let r1 be f(3, x).
Let r2 be f(5, x).
Show r1 + r2.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("f_s0_3"),
        "Should have specialized variant for static a=3.\nGot:\n{}",
        rust
    );
    assert!(
        rust.contains("f_s0_5"),
        "Should have specialized variant for static a=5.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_variant_limit_8() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To scale (factor: Int, x: Int) -> Int:
    Return factor * x.

## Main
Let n be parseInt("1").
Let r1 be scale(1, n).
Let r2 be scale(2, n).
Let r3 be scale(3, n).
Let r4 be scale(4, n).
Let r5 be scale(5, n).
Let r6 be scale(6, n).
Let r7 be scale(7, n).
Let r8 be scale(8, n).
Let r9 be scale(9, n).
Let r10 be scale(10, n).
Show r1 + r2 + r3 + r4 + r5 + r6 + r7 + r8 + r9 + r10.
"#;
    let rust = compile_to_rust(source).unwrap();
    let variant_count = rust.lines()
        .filter(|line| line.contains("fn scale_s0") && line.contains("("))
        .count();
    assert!(
        variant_count <= 8,
        "At most 8 specialized variants allowed. Got {}.\nCode:\n{}",
        variant_count, rust
    );
    assert!(
        rust.contains("scale(") || rust.contains("scale ("),
        "Calls 9 and 10 should fall back to original unspecialized scale.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_embedding_terminates() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To deep (n: Int, x: Int) -> Int:
    If n equals 0:
        Return x.
    Return deep(n - 1, x + 1).

## Main
Let input be parseInt("5").
Let result be deep(20, input).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.is_empty(),
        "Compilation should terminate without infinite loop."
    );
    assert!(
        rust.contains("deep"),
        "Residual should contain a call to deep (recursive residual).\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_interner_used_for_names() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let n be parseInt("7").
Let y be multiply(3, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("multiply_s0"),
        "Specialized function name should be properly interned and appear in output.\nGot:\n{}",
        rust
    );
    let specialized_fn = rust.lines()
        .find(|line| line.contains("fn multiply_s0") && line.contains("("));
    assert!(
        specialized_fn.is_some(),
        "Interned name should be used as the function name in the definition.\nGot:\n{}",
        rust
    );
}

// =============================================================================
// Sprint 2.3 — Multiple Call Sites + Cascading (5 tests)
// =============================================================================

#[test]
fn pe_same_static_reuses() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let x be parseInt("3").
Let y be parseInt("5").
Let r1 be f(3, x).
Let r2 be f(3, y).
Show r1 + r2.
"#;
    let rust = compile_to_rust(source).unwrap();
    let def_count = rust.lines()
        .filter(|line| line.contains("fn f_s0") && line.contains("("))
        .count();
    assert_eq!(
        def_count, 1,
        "f(3, a) and f(3, b) should share ONE specialized variant.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_different_static_creates() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let x be parseInt("3").
Let r1 be f(3, x).
Let r2 be f(7, x).
Show r1 + r2.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("f_s0_3"),
        "Should have specialized variant for f(3, _).\nGot:\n{}",
        rust
    );
    assert!(
        rust.contains("f_s0_7"),
        "Should have specialized variant for f(7, _).\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_cascading_specialization() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To g (x: Int, y: Int) -> Int:
    Return x * y.

## To f (a: Int, b: Int) -> Int:
    Return g(a, b).

## Main
Let n be parseInt("5").
Let result be f(3, n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("f_s0"),
        "f should be specialized for a=3.\nGot:\n{}",
        rust
    );
    assert!(
        rust.contains("g_s0"),
        "g should also be specialized (cascading from f's specialized body).\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_nested_specialization() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To inner (x: Int, y: Int) -> Int:
    Return x * y.

## To outer (a: Int, b: Int) -> Int:
    Return inner(a, b) + inner(a, b + 1).

## Main
Let n be parseInt("5").
Let result be outer(5, n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("outer_s0"),
        "outer should be specialized for a=5.\nGot:\n{}",
        rust
    );
    assert!(
        rust.contains("inner_s0"),
        "inner should be specialized (cascading from outer's body).\nGot:\n{}",
        rust
    );
    let inner_def_count = rust.lines()
        .filter(|line| line.contains("fn inner_s0") && line.contains("("))
        .count();
    assert_eq!(
        inner_def_count, 1,
        "inner(5, n) and inner(5, n+1) should reuse same inner_s0 variant (same key).\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_multiple_static_params() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int, b: Int, c: Int) -> Int:
    Return a * b + c.

## Main
Let n be parseInt("5").
Let result be f(2, n, 7).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    let specialized = rust.lines()
        .find(|line| line.contains("fn f_s0") && line.contains("("));
    assert!(
        specialized.is_some(),
        "Specialized function for f(2, _, 7) should exist.\nGot:\n{}",
        rust
    );
    let sig = specialized.unwrap();
    let param_count = sig.matches(',').count() + if sig.contains('(') && !sig.contains("()") { 1 } else { 0 };
    assert_eq!(
        param_count, 1,
        "Specialized f should have ONE param (b), with a=2 and c=7 substituted.\nSignature: {}",
        sig
    );
    let body_region = rust.lines()
        .skip_while(|line| !line.contains("fn f_s0"))
        .take(5)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body_region.contains("2") && body_region.contains("7"),
        "Body should contain substituted values 2 and 7.\nBody:\n{}",
        body_region
    );
}

// =============================================================================
// Sprint 2.4 — Safety Guards (5 tests)
// =============================================================================

#[test]
fn pe_impure_skipped() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To sideEffect (n: Int) -> Int:
    Show n.
    Return n.

## Main
Let x be parseInt("5").
Let y be sideEffect(5).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("sideEffect_s0"),
        "Impure function (has Show) should NOT be specialized.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_io_preserved() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int, b: Int) -> Int:
    Show a.
    Return b.

## Main
Let n be parseInt("7").
Let y be f(3, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("f_s0"),
        "Function with IO (Show) should NOT be specialized even with static arg.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_collections_as_dynamic() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To sumFirst (items: Seq of Int, x: Int) -> Int:
    Return x + 1.

## Main
Let n be parseInt("5").
Let items be new Seq of Int.
Let y be sumFirst(items, 5).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    if rust.contains("sumFirst_s0") {
        let specialized = rust.lines()
            .find(|line| line.contains("fn sumFirst_s0") || line.contains("fn sum_first_s0"));
        if let Some(sig) = specialized {
            assert!(
                sig.contains("items") || sig.contains("Vec"),
                "Collection param should still be present in specialized function.\nSig: {}",
                sig
            );
        }
    }
}

#[test]
fn pe_self_referential_reuse() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int, b: Int) -> Int:
    If b equals 0:
        Return a.
    Return f(a, b - 1).

## Main
Let n be parseInt("5").
Let y be f(3, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    let variant_count = rust.lines()
        .filter(|line| line.contains("fn f_s0") && line.contains("("))
        .count();
    assert_eq!(
        variant_count, 1,
        "Self-referential f(3, b-1) should reuse same variant. Only ONE specialized function.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_no_specialize_all_dynamic() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To add (a: Int, b: Int) -> Int:
    Return a + b.

## Main
Let x be parseInt("3").
Let z be parseInt("5").
Let y be add(x, z).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("add_s0"),
        "All-dynamic args should NOT trigger specialization.\nGot:\n{}",
        rust
    );
}

// =============================================================================
// Sprint 2.5 — Pipeline Integration + E2E (12 tests)
// =============================================================================

#[test]
fn pe_after_propagate() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let n be parseInt("7").
Let a be 3.
Let y be multiply(a, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("multiply_s0"),
        "Propagation should resolve a=3 before PE sees multiply(a, n) → multiply(S(3), D).\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_before_ctfe() {
    let source = r#"## To add5 (x: Int) -> Int:
    Return x + 5.

## Main
Let y be add5(10).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("15"),
        "All-static call add5(10) should be fully evaluated to 15 by CTFE.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_fixpoint_terminates() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To g (x: Int, y: Int) -> Int:
    Return x * y.

## To f (a: Int, b: Int) -> Int:
    Let c be a + 1.
    Return g(c, b).

## Main
Let n be parseInt("5").
Let result be f(2, n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.is_empty(),
        "Fixpoint loop (fold → propagate → PE) should terminate."
    );
}

#[test]
fn pe_constant_arg_fully_evaluated() {
    let source = r#"## To add5 (x: Int) -> Int:
    Return x + 5.

## Main
Let y be add5(10).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("15"),
        "All-static add5(10) should be fully evaluated to 15.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_pipeline_fold_interaction() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To f (a: Int, b: Int) -> Int:
    Return (a + 1) * b.

## Main
Let n be parseInt("7").
Let y be f(4, n).
Show y.
"#;
    let rust = compile_to_rust(source).unwrap();
    let specialized = rust.lines()
        .skip_while(|line| !line.contains("f_s0"))
        .take(5)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        specialized.contains("5 *") || specialized.contains("5i64 *") || specialized.contains("(5)"),
        "After substitution a=4, fold should simplify (4+1)*b → 5*b.\nSpecialized:\n{}",
        specialized
    );
}

#[test]
fn pe_factorial_output() {
    let source = r#"## To factorial (n: Int) -> Int:
    If n equals 0:
        Return 1.
    Return n * factorial(n - 1).

## Main
Let y be factorial(10).
Show y.
"#;
    assert_exact_output(source, "3628800");
}

#[test]
fn pe_branch_elimination_output() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To select (flag: Bool, a: Int, b: Int) -> Int:
    If flag:
        Return a.
    Otherwise:
        Return b.

## Main
Let n be parseInt("7").
Let y be select(true, n, 0).
Show y.
"#;
    assert_exact_output(source, "7");
}

#[test]
fn pe_partial_specialization_output() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

## Main
Let n be parseInt("7").
Let y be multiply(3, n).
Show y.
"#;
    assert_exact_output(source, "21");
}

#[test]
fn pe_recursive_memoization() {
    let source = r#"## To factorial (n: Int) -> Int:
    If n equals 0:
        Return 1.
    Return n * factorial(n - 1).

## Main
Let y be factorial(10).
Show y.
"#;
    assert_exact_output(source, "3628800");
}

#[test]
fn pe_code_bloat_limit() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To scale (factor: Int, x: Int) -> Int:
    Return factor * x.

## Main
Let n be parseInt("1").
Let r1 be scale(1, n).
Let r2 be scale(2, n).
Let r3 be scale(3, n).
Let r4 be scale(4, n).
Let r5 be scale(5, n).
Let r6 be scale(6, n).
Let r7 be scale(7, n).
Let r8 be scale(8, n).
Let r9 be scale(9, n).
Let r10 be scale(10, n).
Show r1 + r2 + r3 + r4 + r5 + r6 + r7 + r8 + r9 + r10.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run despite variant limit.\nstderr: {}\n\nGenerated Rust:\n{}",
        result.stderr, result.rust_code
    );
    assert_eq!(
        result.stdout.trim(),
        "55",
        "scale(1,1)+scale(2,1)+...+scale(10,1) = 55.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(), result.rust_code
    );
}

#[test]
fn pe_depth_limit_preserves_correctness() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To deep (n: Int, x: Int) -> Int:
    If n equals 0:
        Return x.
    Return deep(n - 1, x + 1).

## Main
Let n be parseInt("5").
Let result be deep(20, n).
Show result.
"#;
    assert_exact_output(source, "25");
}

#[test]
fn pe_simplicity_check_passes() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To complex (mode: Int, x: Int) -> Int:
    If mode equals 1:
        Return x + 1.
    If mode equals 2:
        Return x + 2.
    If mode equals 3:
        Return x + 3.
    Return x.

## Main
Let n be parseInt("10").
Let result be complex(1, n).
Show result.
"#;
    assert_exact_output(source, "11");
}

// =============================================================================
// Sprint A — Fix PE Foundation (Step A1, A2, A3)
// =============================================================================

#[test]
fn pe_spec_key_is_structured() {
    // SpecKey should be (Symbol, Vec<Option<Literal>>), not (Symbol, String)
    // Verify two calls with same static values produce the same key
    let source = r#"
## To scale (factor: Int) and (x: Int) -> Int:
    Return factor * x.

## Main
    Let a be scale(3, 10).
    Let b be scale(3, 20).
    Show a.
    Show b.
"#;
    assert_exact_output(source, "30\n60");
}

#[test]
fn pe_spec_key_no_string_collision() {
    // Two different static patterns that would collide with string keys:
    // scale(10, D) and scale(1, 0, D) if serialized as "S10,D" vs "S1,S0,D"
    // With structured keys, these are different Vecs with different lengths.
    let source = r#"
## To process2 (a: Int) and (b: Int) -> Int:
    Return a + b.

## To process3 (a: Int) and (b: Int) and (c: Int) -> Int:
    Return a + b + c.

## Main
    Let x be process2(10, 5).
    Let y be process3(1, 0, 5).
    Show x.
    Show y.
"#;
    assert_exact_output(source, "15\n6");
}

#[test]
fn pe_structured_embedding_detects_growth() {
    // A function called with increasingly large static arguments should be
    // terminated by the embedding check before hitting the variant limit.
    // With string containment, "S5" is contained in "S50" which is wrong.
    // With structured embedding, Number(5) embeds in Number(50) correctly.
    let source = r#"
## To grow (n: Int) and (x: Int) -> Int:
    If n is at most 0:
        Return x.
    Return grow(n - 1, x + 1).

## Main
    Let result be grow(5, 0).
    Show result.
"#;
    assert_exact_output(source, "5");
}

#[test]
fn pe_structured_embedding_no_false_positive() {
    // "S5" is a substring of "S50" — string containment gives a false positive.
    // Structured embedding: Number(5) embeds in Number(50) (|5| ≤ |50|) — correct.
    // But Number(50) does NOT embed in Number(5) — no false negative.
    let source = r#"
## To compute (base: Int) and (x: Int) -> Int:
    Return base * x.

## Main
    Let a be compute(5, 10).
    Let b be compute(50, 10).
    Show a.
    Show b.
"#;
    assert_exact_output(source, "50\n500");
}

#[test]
fn pe_effect_env_allows_pure_specialization() {
    // A function that uses patterns the ad-hoc body_has_io might not recognize
    // as pure (e.g., complex control flow) should still be specializable if
    // EffectEnv confirms it's pure.
    let source = r#"
## To classify (n: Int) -> Int:
    If n is greater than 100:
        Return 3.
    If n is greater than 10:
        Return 2.
    If n is greater than 0:
        Return 1.
    Return 0.

## Main
    Let a be classify(50).
    Let b be classify(5).
    Show a.
    Show b.
"#;
    assert_exact_output(source, "2\n1");
}

#[test]
fn pe_effect_env_blocks_impure_specialization() {
    // A function with IO should not be specialized, confirmed by EffectEnv
    let source = r#"
## To logAndCompute (n: Int) and (x: Int) -> Int:
    Show n.
    Return n * x.

## Main
    Let result be logAndCompute(5, 10).
    Show result.
"#;
    assert_exact_output(source, "5\n50");
}

// =============================================================================
// Phase 2.1 — EffectEnv Wiring (3 tests)
// =============================================================================

#[test]
fn pe_effect_env_check_is_not_io() {
    // Check is classified as security_check (not IO) by EffectEnv,
    // but body_has_io() incorrectly treats it as IO and blocks specialization.
    // With EffectEnv wired in using !function_has_io(), this function
    // should be specializable since Check is security_check, not IO.
    let source = r#"## Record User:
    role: Text.

## Policy
A User is admin if the user's role equals "admin".

## To native parseInt (s: Text) -> Int

## To validate (threshold: Int, u: User) -> Int:
    Check that the u is admin.
    If threshold is greater than 10:
        Return threshold.
    Return 0.

## Main
Let u be a new User with role "admin".
Let result be validate(100, u).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("validate_s0"),
        "Function with Check (security_check, not IO) should be specializable \
         when EffectEnv is wired in with !function_has_io() override.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_effect_env_check_plus_show_still_blocked() {
    // A function with both Check AND Show should NOT be specialized,
    // because Show IS io even though Check is only security_check.
    let source = r#"## Record User:
    role: Text.

## Policy
A User is admin if the user's role equals "admin".

## To native parseInt (s: Text) -> Int

## To audit (threshold: Int, u: User) -> Int:
    Check that the u is admin.
    Show threshold.
    If threshold is greater than 10:
        Return threshold.
    Return 0.

## Main
Let u be a new User with role "admin".
Let result be audit(100, u).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("audit_s0"),
        "Function with Check AND Show should NOT be specialized — Show is IO.\nGot:\n{}",
        rust
    );
}

#[test]
fn pe_effect_env_transitive_io_blocked() {
    // EffectEnv propagates effects through the call graph.
    // If a function calls another function that has IO, the caller
    // should also be marked as having IO and blocked from specialization.
    // body_has_io() doesn't look through function calls, so without EffectEnv
    // `compute` would be incorrectly specialized. With EffectEnv, the transitive
    // IO through `printVal` is detected and specialization is blocked.
    let source = r#"## To native parseInt (s: Text) -> Int

## To printVal (x: Int) -> Int:
    Show x.
    Return x.

## To compute (factor: Int, n: Int) -> Int:
    Let shown be printVal(n).
    Return factor * shown.

## Main
Let n be parseInt("7").
Let result be compute(3, n).
Show result.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("compute_s0"),
        "Function that transitively calls IO (Show via printVal) should NOT be \
         specialized when EffectEnv propagates IO through call graph.\nGot:\n{}",
        rust
    );
}
