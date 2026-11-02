//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! HOW MANY KERNELS DO WE HAVE? — the registry, counted, with a proof behind every cell.
//!
//! The competitive claim to answer is "1,000+ optimized kernels in registry, each reverse-engineered
//! and re-targeted per chip". Ours are not reverse-engineered one at a time: a kernel here is a
//! (LOGOS ISA op × target) LOWERING, generated from one IR, and every cell is differentially verified
//! against the VM oracle rather than gated on a downstream benchmark score. The registry therefore
//! already exists — what it has never had is a NUMBER, and a number nobody states is a number nobody
//! can hold you to.
//!
//! ★ THIS COUNTS NOTHING ITSELF. Every figure below is READ OUT of the lock that proves it, by parsing
//! that lock's own floor constant. A census that carried its own copies would drift the first time a
//! floor moved and would then be quoting itself. If a floor rises, this sees it; if a lock is deleted
//! or renamed, this fails rather than silently dropping a target from the total.
//!
//! ★ AND IT IS A ROLL-UP, WHICH IS A WEAKER THING THAN A PROOF. Each floor is proven by its own lock
//! running its own differential. This file asserts only that the floors exist, that they sum to at
//! least what they summed to before, and that the total is reported honestly.
//!
//! ★ TWO UNIVERSES, NEVER MIXED. `REGISTRY` counts (ISA op × target) lowerings against the ISA; `TENSOR_REGISTRY`
//! counts tensor-op kernels (attention, gated delta, rmsnorm, rope, …) against the probe table of the lock that
//! proves them — each row names its floor AND its universe, both read from that lock, so neither table's
//! headroom is computed against the other's denominator.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test kernel_registry_census_lock`
//!
//! ★ MUTATION PROBE: lower any floor in its own lock — `the_registry_only_grows` reds with the target
//! named. Point a row at a lock file that does not define that constant — `every_row_is_read_from_the_
//! lock_that_proves_it` reds, which is the pair that keeps this file from inventing numbers.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

/// `(target, the lock that proves it, that lock's floor constant)`.
const REGISTRY: &[(&str, &str, &str)] = &[
    (
        "C",
        "crates/logicaffeine_tests/tests/op_backend_matrix_lock.rs",
        "FLOOR_C",
    ),
    (
        "Rust",
        "crates/logicaffeine_tests/tests/op_backend_matrix_lock.rs",
        "FLOOR_RUST",
    ),
    (
        "RISC-V",
        "crates/logicaffeine_tests/tests/op_backend_matrix_lock.rs",
        "FLOOR_RISCV",
    ),
    (
        "WASM",
        "crates/logicaffeine_tests/tests/op_backend_matrix_lock.rs",
        "FLOOR_WASM",
    ),
    (
        "CUDA",
        "crates/logicaffeine_tests/tests/op_backend_matrix_lock.rs",
        "FLOOR_CUDA",
    ),
    (
        "Tenstorrent",
        "crates/logicaffeine_tests/tests/op_backend_matrix_lock.rs",
        "FLOOR_TENSTORRENT",
    ),
    (
        "Fabric (CGRA)",
        "crates/logicaffeine_tests/tests/op_backend_matrix_lock.rs",
        "FLOOR_FABRIC",
    ),
    (
        "SambaNova RDU",
        "crates/logicaffeine_tests/tests/rdu_op_coverage_lock.rs",
        "RDU_REALIZED_FLOOR",
    ),
];

/// The ISA the targets are counted against.
const TOTAL_OPS_LOCK: (&str, &str) = (
    "crates/logicaffeine_tests/tests/fabric_coverage_lock.rs",
    "TOTAL_OPS",
);

/// TENSOR-OP kernels — a different universe from the ISA: `(target, the lock that proves it, its floor
/// constant, its universe constant)`. A cell is a tensor sugar (matmul, rmsnorm, attention, gated delta, …)
/// lowered through the whole-program walk, composed, certified, shipped as the target's artifact, and
/// evaluated ≡ the VM (an Fp32 twin to 2⁻¹⁶ for wiring, the chip's own precision within its rounding bound).
const TENSOR_REGISTRY: &[(&str, &str, &str, &str)] = &[(
    "SambaNova SN50 (tensor ops, program path)",
    "crates/logicaffeine_tests/tests/spatial_program_path_ledger_lock.rs",
    "PROGRAM_PATH_FLOOR",
    "PROGRAM_PATH_UNIVERSE",
)];

/// The registry may only grow. Raising this is the point; lowering it means a lowering was lost.
const FROZEN_TOTAL: usize = 887;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Read `const <name>: usize = <n>;` out of a lock file. The census never carries its own copy.
fn floor_of(file: &str, name: &str) -> usize {
    let path = root().join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let needle = format!("const {name}: usize = ");
    let at = text.find(&needle).unwrap_or_else(|| {
        panic!("{file} no longer defines `{name}` — the census is quoting a proof that moved")
    });
    let rest = &text[at + needle.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("{file}:{name} is not a plain integer"))
}

/// ── EVERY ROW IS READ FROM THE LOCK THAT PROVES IT.
#[test]
fn every_row_is_read_from_the_lock_that_proves_it() {
    for (target, file, konst) in REGISTRY {
        let n = floor_of(file, konst);
        assert!(n > 0, "{target}: a floor of zero is not a covered target");
        assert!(
            n <= 200,
            "{target}: {konst} = {n} is larger than any plausible ISA — the census is reading the wrong constant"
        );
    }
    let total_ops = floor_of(TOTAL_OPS_LOCK.0, TOTAL_OPS_LOCK.1);
    assert!(
        total_ops >= 100,
        "the ISA is {total_ops} ops, which is not the ISA this census is about"
    );
    for (target, file, floor, universe) in TENSOR_REGISTRY {
        let (n, u) = (floor_of(file, floor), floor_of(file, universe));
        assert!(
            n > 0 && n <= u,
            "{target}: {floor} = {n} out of {universe} = {u} is not a coverage count"
        );
    }
}

/// ── THE REGISTRY, COUNTED. One number, and the arithmetic that gets to it.
#[test]
fn the_registry_only_grows() {
    let total_ops = floor_of(TOTAL_OPS_LOCK.0, TOTAL_OPS_LOCK.1);
    let mut total = 0usize;
    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    for (target, file, konst) in REGISTRY {
        let n = floor_of(file, konst);
        total += n;
        rows.push(((*target).to_string(), n, total_ops.saturating_sub(n)));
    }
    println!("\n  LOGOS kernel registry — (ISA op x target) lowerings, each verified against the VM oracle\n");
    println!("    {:<16} {:>8} {:>10}", "target", "lowered", "headroom");
    for (t, n, gap) in &rows {
        println!("    {t:<16} {n:>8} {gap:>10}");
    }
    let headroom: usize = rows.iter().map(|(_, _, g)| g).sum();
    println!("    {:<16} {:>8} {:>10}", "ISA TOTAL", total, headroom);
    println!(
        "\n  {total} lowerings over {} targets against a {total_ops}-op ISA.",
        REGISTRY.len()
    );
    println!(
        "  Raising every target to full ISA coverage adds {headroom}, for {}.\n",
        total + headroom
    );
    println!("  Tensor-op kernels (each composed, certified, and evaluated ≡ the VM):\n");
    for (target, file, floor, universe) in TENSOR_REGISTRY {
        let (n, u) = (floor_of(file, floor), floor_of(file, universe));
        total += n;
        println!("    {target:<44} {n:>4} of {u}");
    }
    println!("\n  REGISTRY TOTAL: {total}\n");

    assert!(
        total >= FROZEN_TOTAL,
        "the registry SHRANK: {total} lowerings against a frozen {FROZEN_TOTAL}. A target lost coverage — \
         find which floor fell and why before re-freezing."
    );
}

/// ── THE CHIP THAT MATTERS RIGHT NOW IS IN IT, AND IS NOT THE WEAKEST ROW.
///
/// A registry that covered everything except the part we are about to be handed would be the wrong
/// registry. The RDU column is asserted separately so it cannot be carried by the others.
#[test]
fn the_sambanova_column_is_real_and_leading() {
    let rdu = floor_of(
        "crates/logicaffeine_tests/tests/rdu_op_coverage_lock.rs",
        "RDU_REALIZED_FLOOR",
    );
    let total_ops = floor_of(TOTAL_OPS_LOCK.0, TOTAL_OPS_LOCK.1);
    assert!(
        rdu >= 115,
        "the SN50 column is {rdu} of {total_ops} ops — it was 115 and must not fall"
    );
    let others: Vec<usize> = REGISTRY
        .iter()
        .filter(|(t, _, _)| *t != "SambaNova RDU")
        .map(|(_, f, k)| floor_of(f, k))
        .collect();
    let best_other = others.iter().copied().max().unwrap_or(0);
    assert!(
        rdu >= best_other,
        "the RDU column ({rdu}) has fallen behind the best other target ({best_other}) — the chip we are \
         preparing for should not be the one we cover least"
    );
    println!("SambaNova RDU: {rdu} of {total_ops} ISA ops lowered and VM-verified (best other target: {best_other})");
}
