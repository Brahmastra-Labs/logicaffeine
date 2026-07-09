//! ════════════════════════════════════════════════════════════════════════════════════════════
//! FUTAMURA TIER LOCK (Tooth 4) — a Jones-optimal residual is a first-class Logos program, so it
//! must run byte-IDENTICALLY on every execution tier. Optimality that made the residual
//! tier-divergent would be a miscompile the count_dispatch oracle can't see.
//!
//! This lock covers the P1 residual across the tree-walker and the bytecode VM (the two portable
//! tiers, sharing one frontend). It asserts, for every corpus program:
//!   • residual on tree-walker == residual on VM  (output AND error), and
//!   • that == the ORIGINAL source's output       (optimality preserved semantics).
//!
//! Phase 5 extends this with the VM-driven PE (`residual_tw == residual_vm`), the P2/P3 round-trip
//! residuals, and an `#[ignore]`d AOT slice. Strictly monotone: add tiers, never remove.
//! ════════════════════════════════════════════════════════════════════════════════════════════

mod pe_support;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use pe_support::*;

fn norm(s: &str) -> String {
    s.trim().to_string()
}

/// Representative programs spanning the folded surface (straight-line, interp, struct, list,
/// loop, recursion, map, set, closure).
fn tier_corpus() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("int_arith", "## Main\nShow 2 + 3 * 4.", "14"),
        ("text_interp", "## Main\nLet n be 7.\nShow \"v={n}\".", "v=7"),
        ("list_index", "## Main\nLet xs be [10, 20, 30].\nShow item 2 of xs.", "20"),
        (
            "while_loop",
            "## Main\nLet mutable i be 3.\nLet mutable s be 0.\nWhile i is greater than 0:\n    Set s to s + i.\n    Set i to i - 1.\nShow s.",
            "6",
        ),
        (
            "recursion",
            "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(5).",
            "120",
        ),
        (
            "map_get",
            "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nSet item \"b\" of m to 2.\nShow item \"b\" of m.",
            "2",
        ),
    ]
}

/// The PE ENGINE itself — not just its residual — is tier-invariant: running the fixed PE
/// program on the register VM must produce the byte-identical residual it produces on the
/// tree-walker, for every corpus program. This is what licenses driving the corpus on the VM
/// for the measured ~2× (the residual then still satisfies the run-identity lock below).
#[test]
fn pe_engine_residual_identical_on_vm() {
    let mut failures = Vec::new();
    for (name, prog, _expected) in tier_corpus() {
        let res_tw = decompile(prog);
        let res_vm = decompile_on_vm(prog);
        match (res_tw, res_vm) {
            (Ok(a), Ok(b)) if a == b => {}
            (Ok(a), Ok(b)) => failures.push(format!(
                "[{name}] PE residual differs by tier:\n  tree-walker:\n{a}\n  VM:\n{b}"
            )),
            (a, b) => failures.push(format!(
                "[{name}] PE engine tier error: tw={a:?} vm={b:?}"
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "PE-engine tier divergence ({}):\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

/// Every P1 residual runs identically on the tree-walker and the bytecode VM, and both match the
/// original program's output.
#[test]
fn p1_residuals_run_identically_across_tree_walker_and_vm() {
    let mut failures = Vec::new();
    for (name, prog, expected) in tier_corpus() {
        let residual = match decompile(prog) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("[{name}] P1 projection failed: {e}"));
                continue;
            }
        };
        let src = tw_outcome(prog);
        let res_tw = tw_outcome(&residual);
        let res_vm = vm_outcome(&residual);

        if res_tw.error != res_vm.error {
            failures.push(format!(
                "[{name}] residual tier ERROR diverged: tw={:?} vm={:?}",
                res_tw.error, res_vm.error
            ));
        }
        if norm(&res_tw.output) != norm(&res_vm.output) {
            failures.push(format!(
                "[{name}] residual tier OUTPUT diverged:\n  tw: {:?}\n  vm: {:?}",
                res_tw.output, res_vm.output
            ));
        }
        if norm(&res_tw.output) != norm(&src.output) {
            failures.push(format!(
                "[{name}] residual output != source output:\n  residual: {:?}\n  source:   {:?}",
                res_tw.output, src.output
            ));
        }
        if norm(&res_tw.output) != expected {
            failures.push(format!(
                "[{name}] wrong output: got {:?}, expected {expected:?}",
                res_tw.output
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "P1 residual tier-parity violations ({}):\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}
