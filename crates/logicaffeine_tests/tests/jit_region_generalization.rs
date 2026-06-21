//! GENERALIZATION fuzzer for region register allocation.
//!
//! The 30 benchmarks must not be the only programs the region tier is
//! correct on. This generates a wide family of DISTINCT region-shaped
//! programs from a deterministic LCG seed — varied loop bounds, operation
//! mixes (counter arithmetic, array reads/writes, accumulators, modulo,
//! conditionals, nested loops) — and asserts the tiered VM (region pinning
//! ACTIVE) produces byte-identical output to the independent tree-walker
//! oracle on every one. A pin-selection heuristic that "worked on the
//! benchmarks" but miscompiled general loops would fail here loudly.
//!
//! Parity against the tree-walker catches any miscompile directly (a wrong
//! pin would produce a wrong answer); the dedicated `LOGOS_JIT_CANARY`
//! sweep covers out-of-bounds writes process-wide.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

/// Assemble one random region-shaped program. Every emitted statement is
/// valid LOGOS; the body mixes the op families that drive pin selection
/// (variant arithmetic, array index reads/writes, accumulators, modulo,
/// conditionals, an optional inner loop) so different seeds pin different
/// slots.
fn random_region_program(seed: u64) -> String {
    let mut s = seed;
    let n = 300 + (lcg(&mut s) % 700) as i64; // array length 300..1000
    let kmod = 1 + (lcg(&mut s) % 9999) as i64;
    let k1 = 1 + (lcg(&mut s) % 97) as i64;
    let k2 = 1 + (lcg(&mut s) % 31) as i64;
    let thr = (lcg(&mut s) % 5000) as i64;

    let mut p = String::new();
    p.push_str("## Main\n");
    p.push_str("Let mutable arr be a new Seq of Int.\n");
    p.push_str("Let mutable seed be 12345.\n");
    p.push_str("Let mutable g be 0.\n");
    p.push_str(&format!("While g is less than {n}:\n"));
    p.push_str("\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n");
    p.push_str("\x20   Push (seed / 65536) % 32768 to arr.\n");
    p.push_str("\x20   Set g to g + 1.\n");
    p.push_str("Let mutable acc be 0.\n");
    p.push_str("Let mutable cnt be 0.\n");
    p.push_str("Let mutable i be 1.\n");
    p.push_str(&format!("While i is at most {n}:\n"));

    // 3–5 body statements drawn from valid templates.
    let body_len = 3 + (lcg(&mut s) % 3) as usize;
    for _ in 0..body_len {
        match lcg(&mut s) % 7 {
            0 => p.push_str(&format!(
                "\x20   Set acc to (acc + item i of arr * {k1}) % 1000000007.\n"
            )),
            1 => p.push_str(&format!(
                "\x20   Set acc to (acc + item i of arr + {k2}) % {kmod}.\n"
            )),
            2 => p.push_str(&format!(
                "\x20   If item i of arr is greater than {thr}:\n\x20       Set cnt to cnt + 1.\n"
            )),
            3 => p.push_str("\x20   Set item i of arr to (item i of arr + 1) % 32768.\n"),
            4 => p.push_str(&format!(
                "\x20   Set acc to acc + (item i of arr % {k1}) - {k2}.\n"
            )),
            5 => {
                // a tiny inner loop touching the array (depth-2 region)
                p.push_str("\x20   Let mutable j be 1.\n");
                p.push_str("\x20   While j is at most 3:\n");
                p.push_str(&format!(
                    "\x20       Set acc to (acc + item i of arr * j) % {kmod}.\n"
                ));
                p.push_str("\x20       Set j to j + 1.\n");
            }
            _ => p.push_str(&format!("\x20   Set acc to acc + {k1} - {k2}.\n")),
        }
    }
    p.push_str("\x20   Set i to i + 1.\n");
    p.push_str("Show acc.\n");
    p.push_str("Show cnt.\n");
    // A checksum of the (possibly mutated) array, so array writes are observed.
    p.push_str("Let mutable chk be 0.\n");
    p.push_str("Set i to 1.\n");
    p.push_str(&format!("While i is at most {n}:\n"));
    p.push_str("\x20   Set chk to (chk + item i of arr * i) % 1000000007.\n");
    p.push_str("\x20   Set i to i + 1.\n");
    p.push_str("Show chk.\n");
    p
}

/// Every generated region program: tiered VM (region pinning active by
/// default) == tree-walker, byte for byte.
#[test]
fn region_pinning_generalizes_over_random_loop_programs() {
    let mut tiered_count = 0u32;
    for seed in 1..=120u64 {
        let src = random_region_program(seed.wrapping_mul(2654435761));
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "region-pinned VM diverged from tree-walker on generated seed {seed}:\n{src}"
        );
        assert_eq!(vm.error, None, "generated program {seed} should not error");
        let (_, region_ok) = tier.region_counts();
        if region_ok >= 1 {
            tiered_count += 1;
        }
    }
    // The whole point is that pinning ACTUALLY FIRES on these — otherwise
    // the parity proves nothing about the optimization.
    assert!(
        tiered_count >= 100,
        "region pinning must engage on the bulk of generated programs (only {tiered_count}/120 tiered)"
    );
}
