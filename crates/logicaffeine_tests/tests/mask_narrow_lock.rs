//! ★ LOCK — an integer chain under a low-bit mask computes at the mask's width. ★
//!
//! The low `k` bits of `+`, `−`, `×`, `&`, `|`, `^` and a constant `<<` depend only on the low `k` bits of their
//! operands. So under `& (2^k − 1)` with `k ≤ 32`, a chain of those whose every node the range oracle proves stays
//! an i64 computes exactly in 32-bit wrapping arithmetic, and that is the form a C or Rust compiler narrows: a
//! masked LCG's unrolled steps become independent multiplies by `a`, `a²`, `a³`, `a⁴` instead of one serial 64-bit
//! multiply chain (`histogram` ran 1.5× slower than 0.10.1 on the 64-bit form). The `masknarrow` optimization does
//! it on the Rust and C emitters; this file holds the generated form, the gate (`## No masknarrow`), the refusal of a
//! chain the oracle cannot bound (an i64 overflow there means `BigInt`, whose `&` is not the low bits of a negative
//! value), and every edge — each width, negative operands, every operator, and unbounded chains computed exactly — to
//! the VM's output on the tree-walker, the generated Rust and both C backends.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test mask_narrow_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use logicaffeine_compile::compile::{compile_to_c, compile_to_rust, optimizations_fired};
use logicaffeine_compile::optimization::decorate_source;
use tier_parity::{Backend, Program};

/// The LCG sixteen benchmarks draw their inputs from, in a loop the oracle bounds.
const LCG: &str = "## Main\n\
Let mutable seed be 42.\n\
Let mutable total be 0.\n\
Let mutable i be 0.\n\
While i is less than 1000:\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Set total to (total + seed) % 1000000007.\n\
\x20   Set i to i + 1.\n\
Show seed.\n\
Show total.\n";

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

#[test]
fn the_masked_lcg_computes_in_32_bits_in_the_generated_rust() {
    let rust = compile_to_rust(LCG).expect("the LCG compiles to Rust");
    assert!(
        rust.contains("as u32).wrapping_mul(1103515245u32)"),
        "the masked LCG multiplies in u32:\n{rust}"
    );
    assert!(
        !rust.contains("seed * 1103515245"),
        "no 64-bit multiply is left in the masked chain:\n{rust}"
    );
}

#[test]
fn the_masked_lcg_computes_in_32_bits_in_the_generated_c() {
    let c = compile_to_c(LCG).expect("the LCG compiles to C");
    assert!(
        c.contains("((uint32_t)(seed)) * 1103515245u"),
        "the masked LCG multiplies in uint32_t:\n{}",
        c.lines().filter(|l| l.contains("seed")).collect::<Vec<_>>().join("\n")
    );
    assert!(
        !c.contains("seed * 1103515245LL"),
        "no 64-bit multiply is left in the masked chain"
    );
}

#[test]
fn masknarrow_fires_on_the_lcg_and_the_decorator_turns_it_off() {
    assert!(
        optimizations_fired(LCG).contains(&"masknarrow"),
        "masknarrow fires on the masked LCG: {:?}",
        optimizations_fired(LCG)
    );
    let off = decorate_source(LCG, &["masknarrow"]);
    assert!(
        !optimizations_fired(&off).contains(&"masknarrow"),
        "## No masknarrow turns it off"
    );
    let rust = compile_to_rust(&off).expect("compiles");
    assert!(
        !rust.contains("wrapping_mul(1103515245u32)"),
        "with masknarrow off the chain stays 64-bit"
    );
    let c = compile_to_c(&off).expect("compiles");
    assert!(
        !c.contains("(uint32_t)(seed)") && c.contains("seed * 1103515245LL"),
        "with masknarrow off the C chain stays 64-bit"
    );
}

/// A chain over values the oracle cannot bound may leave i64; its exact value is then a `BigInt`, and `&` of a
/// negative `BigInt` is not the low bits two's complement would give. So it is never narrowed.
#[test]
fn a_chain_the_oracle_cannot_bound_is_not_narrowed() {
    let src = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## Main\n\
               Let a be parseInt(\"123456789\").\n\
               Let b be parseInt(\"987654321\").\n\
               Show (a * b * 1103515245 + 12345) & 65535.\n";
    let rust = compile_to_rust(src).expect("compiles");
    assert!(
        !rust.contains("as u32).wrapping_mul"),
        "an unbounded chain is not narrowed:\n{rust}"
    );
    let c = compile_to_c(src).expect("compiles");
    let user_c = c
        .split("// ==== LOGOS user code")
        .nth(1)
        .expect("generated user-code marker");
    assert!(
        !user_c.contains("(uint32_t)("),
        "an unbounded chain is not narrowed in C:\n{user_c}"
    );
}

/// Every width from 1 to 32, over positive and negative values in i64.
fn every_width() -> String {
    let mut src = String::from("## Main\nLet mutable x be 0 - 7.\nWhile x is less than 7:\n");
    for k in 1..=32u32 {
        let mask = (1u64 << k) - 1;
        src.push_str(&format!("    Show (x * 1103515245 + 12345) & {mask}.\n"));
    }
    src.push_str("    Set x to x + 3.\n");
    src
}

const EDGES: &[(&str, &str)] = &[
    (
        "lcg/the_benchmarks_generator",
        "## Main\nLet mutable seed be 42.\nLet mutable i be 0.\nWhile i is less than 20:\n    Set seed to (seed * 1103515245 + 12345) % 2147483648.\n    Show seed.\n    Set i to i + 1.\n",
    ),
    (
        "lcg/a_full_32_bit_modulus",
        "## Main\nLet mutable seed be 7.\nLet mutable i be 0.\nWhile i is less than 20:\n    Set seed to (seed * 1664525 + 1013904223) % 4294967296.\n    Show seed.\n    Set i to i + 1.\n",
    ),
    (
        "ops/negative_operands_keep_their_low_bits",
        "## Main\nLet mutable a be 0 - 1000.\nWhile a is less than 1000:\n    Show (a * 40503 - 77) & 65535.\n    Show (a - 9999) & 255.\n    Set a to a + 137.\n",
    ),
    (
        "ops/and_or_xor_and_a_constant_shift",
        "## Main\nLet mutable a be 0 - 50.\nWhile a is less than 50:\n    Show (((a shifted left by 7) ^ (a * 31)) | 5) & 4095.\n    Show ((a shifted left by 30) + a) & 4294967295.\n    Show ((a shifted left by 40) + a) & 65535.\n    Set a to a + 7.\n",
    ),
    (
        "ops/nested_masks",
        "## Main\nLet mutable a be 1.\nLet mutable i be 0.\nWhile i is less than 30:\n    Set a to (((a * 69069) & 1048575) * 3 + 1) & 16777215.\n    Show a.\n    Set i to i + 1.\n",
    ),
];

/// Chains no oracle bounds, so none narrows: their exact value is masked — a big integer that is not negative
/// leaves its low bits, and a negative one is refused in the reference's words.
const EXACT: &[(&str, &str)] = &[
    (
        "exact/an_unbounded_chain_keeps_its_low_bits",
        "## To native parseInt (s: Text) -> Int\n\n## Main\nLet a be parseInt(\"123456789\").\nLet b be parseInt(\"987654321\").\nShow (a * b * 1103515245 + 12345) & 65535.\nShow ((a shifted left by 30) + b) & 4294967295.\nShow (a + 1) & 255.\n",
    ),
    (
        "exact/a_negative_big_integer_under_a_mask_is_refused",
        "## To native parseInt (s: Text) -> Int\n\n## Main\nLet a be parseInt(\"123456789\").\nLet c be parseInt(\"-5\").\nShow (a + 1) & 255.\nShow (c * a * a * 1103515245 + 1) & 255.\n",
    ),
];

#[test]
fn every_masked_chain_means_the_same_on_every_backend() {
    let mut programs: Vec<Program> = EDGES
        .iter()
        .chain(EXACT)
        .map(|(id, source)| Program::new(format!("masknarrow/{id}"), *source))
        .collect();
    programs.push(Program::new("masknarrow/every_width".to_string(), every_width()));
    programs.push(Program::new("masknarrow/the_lcg".to_string(), LCG));
    let refused: Vec<&str> = programs
        .iter()
        .filter(|p| tier_parity::rejection(&p.source).is_some())
        .map(|p| p.id.as_str())
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which then pin nothing");
    let rows = tier_parity::sweep(
        &programs,
        &[
            Backend::TreeWalker,
            Backend::Vm,
            Backend::RustRelease,
            Backend::C,
            Backend::CBytecode,
        ],
        4,
    )
    .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the VM:\n{}", bad.len(), bad.join("\n"));
}

/// The edges must actually take the narrowed form, or the parity above pins nothing about it.
#[test]
fn the_edges_take_the_narrowed_form() {
    for (id, src) in EDGES {
        let rust = compile_to_rust(src).unwrap_or_else(|e| panic!("{id} compiles: {e:?}"));
        assert!(rust.contains("as u32)"), "{id} narrows in the generated Rust:\n{rust}");
    }
    let rust = compile_to_rust(&every_width()).expect("compiles");
    assert!(
        rust.matches("as u32)").count() >= 32,
        "every width narrows in the generated Rust"
    );
}
