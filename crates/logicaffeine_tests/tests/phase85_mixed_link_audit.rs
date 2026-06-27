//! Adversarial audit of the mixed-document linking feature — edge cases that try to
//! BREAK the partitioner, the constructor-wrapper boxing, the `Ensures` rewriter, and
//! the `hard` (Require) flag's survival through optimization.

use logicaffeine_compile::{
    compile_to_rust, extract_math_module_from_source, extract_math_rust_from_source, partition_mixed,
};

// --- partition_mixed: multi-line Coq Definition ------------------------------

#[test]
fn partition_grabs_multiline_coq_definition() {
    // The Definition spans two lines (first ends `:=`, second ends `.`). The whole
    // block must go to the math stream and be BLANKED out of the imperative stream.
    let src = "Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.\n\
               Definition my_double : MyNat -> MyNat :=\n\
               \x20 fun n : MyNat => MSucc (MSucc n).\n\
               \n\
               ## Main\n\
               Let x be my_double(MZero).\n\
               Show x.";
    let (imp, math) = partition_mixed(src);
    let math = math.expect("math present");
    assert!(math.contains("fun n : MyNat"), "second line of the def is in the math stream:\n{math}");
    assert!(!imp.contains("fun n : MyNat"), "the def body must be blanked from the imperative stream:\n{imp}");
    assert!(!imp.contains("my_double : MyNat"), "the def header must be blanked too:\n{imp}");
    assert!(imp.contains("my_double(MZero)"), "imperative call is preserved:\n{imp}");
}

#[test]
fn partition_leaves_pure_imperative_byte_identical() {
    // A pure imperative program — including a STRING that contains a math keyword —
    // must be returned unchanged, with no math stream.
    let src = "## Main\nLet msg be \"Definition of done\".\nShow msg.";
    let (imp, math) = partition_mixed(src);
    assert_eq!(imp, src, "pure imperative source returned verbatim");
    assert!(math.is_none(), "no math stream");
}

// --- constructor wrappers: box ONLY recursive fields -------------------------

#[test]
fn constructor_wrapper_boxes_only_recursive_fields() {
    // `Cons : Int -> Lst -> Lst` — the Int field is NOT boxed, the recursive Lst field IS.
    let m = extract_math_module_from_source("Inductive Lst := Nil : Lst | Cons : Int -> Lst -> Lst.");
    assert!(m.contains("pub const Nil: Lst = Lst::Nil;"), "nullary ctor → const:\n{m}");
    assert!(
        m.contains("pub fn Cons(a0: i64, a1: Lst) -> Lst { Lst::Cons(a0, Box::new(a1)) }"),
        "non-recursive field unboxed, recursive field boxed:\n{m}"
    );
}

// --- Require survives optimization (stays a HARD assert) ----------------------

#[test]
fn require_stays_hard_after_optimization() {
    // Compiled WITH the default optimizer (constant propagation/folding etc. run). A
    // `Require` must remain a hard `assert!` — never silently downgraded to debug_assert!.
    let src = "## Main\nLet a be 5.\nLet b be 7.\nSet a to b.\nRequire that a is greater than 0.\nShow a.";
    let rust = compile_to_rust(src).expect("compiles");
    assert!(rust.contains("assert!("), "Require lowered to a hard assert after opts:\n{rust}");
    assert!(!rust.contains("debug_assert!("), "must NOT be downgraded to debug_assert:\n{rust}");
}

#[test]
fn proven_enum_extraction_is_deterministic() {
    // Constructor wrappers + Display must be byte-identical across recompiles.
    const SRC: &str = "Inductive Tree := Tip : Tree | Branch : Tree -> Tree -> Tree.";
    let first = extract_math_module_from_source(SRC);
    assert!(first.contains("pub fn Branch("), "sanity: ctor api present:\n{first}");
    for k in 1..16 {
        assert_eq!(first, extract_math_module_from_source(SRC), "non-deterministic (run {k})");
    }
}

// --- bugs found by adversarial review (RED→GREEN) ----------------------------

#[test]
fn partition_coq_block_stops_at_block_header() {
    // An unterminated Coq block (missing `.`) must NOT swallow a following `## Main`.
    let src = "Definition d : MyNat :=\n  MZero\n## Main\nShow 1.";
    let (imp, _math) = partition_mixed(src);
    assert!(imp.contains("## Main"), "## Main must survive an unterminated math block:\n{imp}");
    assert!(imp.contains("Show 1"), "imperative body preserved:\n{imp}");
}

#[test]
fn div_mod_function_not_exercised_with_zero_sample() {
    // `div n n` extracts to `n / n`; the self-verifying demo must NOT call it on the
    // `0` sample (which would be a compile-time `0 / 0` divide-by-zero panic).
    let rust = extract_math_rust_from_source("Definition q : Int -> Int := fun n : Int => div n n.");
    assert!(rust.contains("pub fn q"), "the function is still extracted:\n{rust}");
    assert!(!rust.contains("0i64 / 0i64"), "demo must not divide the zero sample:\n{rust}");
    assert!(!rust.contains("0i64 % 0i64"), "demo must not mod the zero sample:\n{rust}");
}

#[test]
fn partial_application_of_builtin_is_not_extracted() {
    // `fun n => add n` (partial) cannot lower to an operator — the def must be filtered
    // out (no broken `add(n)` call to an undefined function).
    let rust = extract_math_rust_from_source(
        "Definition adder : Int -> (Int -> Int) := fun n : Int => add n.",
    );
    assert!(!rust.contains("fn adder"), "partially-applied-builtin def must be filtered:\n{rust}");
}

#[test]
fn constructor_colliding_with_prelude_gets_no_wrapper() {
    // A ctor named `None`/`Some` must NOT emit a free `const None`/`fn Some` wrapper
    // (it would silently shadow the std prelude under `use proven::*;`).
    let rust = extract_math_module_from_source("Inductive Opt := None : Opt | Some : Int -> Opt.");
    assert!(rust.contains("pub enum Opt"), "enum still emitted:\n{rust}");
    assert!(!rust.contains("pub const None"), "no wrapper for prelude name None:\n{rust}");
    assert!(!rust.contains("pub fn Some("), "no wrapper for prelude name Some:\n{rust}");
}

#[test]
fn ensures_guards_returns_inside_select_arms() {
    // A `Return` in a `select!` arm exits the function — the postcondition must guard
    // it. Without the fix the rewriter skips Select, so the in-arm returns are unguarded.
    let src = "## To pick (n: Int) -> Int:\n    \
               Ensures n is at least 0.\n    \
               Let ch be a Pipe of Int.\n    \
               Send n into ch.\n    \
               Await the first of:\n        \
               Receive x from ch:\n            Return n.\n        \
               After 1 seconds:\n            Return n.\n\
               ## Main\nShow 1.";
    let rust = compile_to_rust(src).expect("compiles");
    // Two select-arm returns, each preceded by the postcondition assert.
    let n = rust.matches("assert!(").count();
    assert!(n >= 2, "postcondition must guard BOTH select-arm returns (>=2 assert!); found {n}:\n{rust}");
}

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_div_function_compiles_standalone() {
    // The standalone math compile of a div function must rustc-compile (the demo no
    // longer divides by the zero sample).
    use logicaffeine_compile::extract_math_rust_from_source;
    let rust = extract_math_rust_from_source("Definition half : Int -> Int := fun n : Int => div n n.");
    // Reuse the e2e harness via a tiny standalone project.
    let dir = std::env::temp_dir().join("logos_div_standalone");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname=\"divtest\"\nversion=\"0.1.0\"\nedition=\"2021\"\n").unwrap();
    std::fs::write(dir.join("src/main.rs"), &rust).unwrap();
    let out = std::process::Command::new("cargo")
        .args(["build", "--quiet"])
        .current_dir(&dir)
        .output()
        .expect("cargo build");
    assert!(out.status.success(), "div standalone must compile:\n{}", String::from_utf8_lossy(&out.stderr));
}

// --- mutually-recursive inductives: SCC-aware boxing (RED→GREEN) --------------

#[test]
fn mutually_recursive_inductives_box_cross_fields() {
    // Tree↔Forest form a cycle; the cross-references must be boxed (or the enums are
    // infinite-size and rustc rejects them). Self-only `term_references` missed this.
    let m = extract_math_module_from_source(
        "Inductive Tree := Leaf : Tree | Node : Forest -> Tree.\n\
         Inductive Forest := Nil2 : Forest | Grow : Tree -> Forest -> Forest.",
    );
    assert!(m.contains("Node(Box<Forest>)"), "Tree::Node cross-field boxed:\n{m}");
    assert!(m.contains("Grow(Box<Tree>, Box<Forest>)"), "Forest::Grow cross+self boxed:\n{m}");
}

#[test]
fn non_cyclic_inductive_field_is_not_boxed() {
    // A → B with no cycle back: B is NOT boxed (no over-boxing regression).
    let m = extract_math_module_from_source(
        "Inductive Leafy := MkLeafy : Bare -> Leafy.\nInductive Bare := MkBare : Bare.",
    );
    assert!(m.contains("MkLeafy(Bare)"), "acyclic field stays unboxed:\n{m}");
    assert!(!m.contains("MkLeafy(Box<Bare>)"), "must not over-box acyclic field:\n{m}");
}

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_mutually_recursive_value_builds_and_shows() {
    use logicaffeine_compile::compile::compile_and_run;
    let dir = std::env::temp_dir().join("logos_mutual_rec_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let src = "Inductive Tree := Leaf : Tree | Node : Forest -> Tree.\n\
               Inductive Forest := Nil2 : Forest | Grow : Tree -> Forest -> Forest.\n\n\
               ## Main\nLet t be Node(Grow(Leaf, Nil2)).\nShow t.";
    let out = compile_and_run(src, &dir).expect("mutually-recursive value compiles and runs");
    assert_eq!(out.trim(), "Node(Grow(Leaf, Nil2))", "got: {out:?}");
}

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_match_derefs_only_boxed_fields() {
    // `ICons g r` binds g (non-recursive Tag, unboxed) + r (recursive Item, boxed).
    // Returning g must NOT deref it (the coarse all-bindings deref would emit `(*g)`
    // on a non-Box and fail to compile).
    use logicaffeine_compile::compile::compile_and_run;
    let dir = std::env::temp_dir().join("logos_match_deref_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let src = "Inductive Tag := TA : Tag | TB : Tag.\n\
               Inductive Item := INone : Item | ICons : Tag -> Item -> Item.\n\
               Definition firstTag : Item -> Tag := fun i : Item => match i return (fun _ : Item => Tag) with | INone => TA | ICons g r => g.\n\n\
               ## Main\nLet it be ICons(TB, INone).\nShow firstTag(it).";
    let out = compile_and_run(src, &dir).expect("mixed-field match compiles and runs");
    assert_eq!(out.trim(), "TB", "got: {out:?}");
}

// --- Futamura decompiler preserves `hard` (Require) --------------------------

#[test]
fn projection1_decompile_preserves_require() {
    // The 1st-projection decompiler (Rust `decompile_stmt`) must round-trip a hard
    // `Require` as `Require that`, not silently downgrade it to `Assert that`.
    use logicaffeine_compile::compile::projection1_source;
    let prog = "## Main\nLet x be 5.\nRequire that x is greater than 0.\nShow x.";
    let residual = projection1_source("", "", prog).expect("projection1");
    assert!(residual.contains("Require that"), "Require preserved in decompiled residual:\n{residual}");
    assert!(!residual.contains("Assert that"), "must not downgrade Require to Assert:\n{residual}");
}

#[test]
fn self_encoding_distinguishes_require_from_assert() {
    // The Futamura self-encoding must model the hard/dev distinction: `Require` encodes
    // as the dedicated `CRequire` variant; `Assert` stays `CRuntimeAssert`.
    use logicaffeine_compile::compile::encode_program_source;
    let req = encode_program_source("## Main\nLet x be 5.\nRequire that x is greater than 0.").expect("encode");
    assert!(req.contains("CHardAssert"), "hard Require encodes as CHardAssert:\n{req}");
    let asrt = encode_program_source("## Main\nLet x be 5.\nAssert that x is greater than 0.").expect("encode");
    assert!(asrt.contains("CRuntimeAssert") && !asrt.contains("CHardAssert"), "soft Assert stays CRuntimeAssert:\n{asrt}");
}

// --- compile-AND-run edge cases ---------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_ensures_guards_nested_early_return() {
    // Postcondition `x is at most 99` must be checked on the EARLY return inside the If,
    // not only the fallthrough. clamp(150) takes the early path and violates it → panic.
    use logicaffeine_compile::compile::{compile_and_run, CompileError};
    let dir = std::env::temp_dir().join("logos_ensures_nested_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let src = "## To clamp (x: Int) -> Int:\n    Ensures x is at most 99.\n    If x is greater than 100:\n        Return x.\n    Return x.\n## Main\nShow clamp(150).";
    match compile_and_run(src, &dir) {
        Err(CompileError::Runtime(_)) => {} // the nested-return postcondition fired
        other => panic!("Ensures must guard the nested early return; got: {other:?}"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_constructor_with_mixed_fields_builds_and_shows() {
    // A constructor with a non-recursive (Int) AND a recursive (Lst) field, built from
    // imperative code and Shown — exercises selective boxing end-to-end.
    use logicaffeine_compile::compile::compile_and_run;
    let dir = std::env::temp_dir().join("logos_ctor_mixed_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let src = "Inductive Lst := Nil : Lst | Cons : Int -> Lst -> Lst.\n\n## Main\nLet xs be Cons(5, Cons(7, Nil)).\nShow xs.";
    let out = compile_and_run(src, &dir).expect("compiles and runs");
    assert_eq!(out.trim(), "Cons(5, Cons(7, Nil))", "got: {out:?}");
}

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_multiline_definition_mixed_runs() {
    use logicaffeine_compile::compile::compile_and_run;
    let dir = std::env::temp_dir().join("logos_multiline_def_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let src = "Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.\n\
               Definition my_double : MyNat -> MyNat :=\n\
               \x20 fun n : MyNat => MSucc (MSucc n).\n\
               \n\
               ## Main\n\
               Let x be my_double(MZero).\n\
               Show x.";
    let out = compile_and_run(src, &dir).expect("compiles and runs");
    assert_eq!(out.trim(), "MSucc(MSucc(MZero))", "got: {out:?}");
}
