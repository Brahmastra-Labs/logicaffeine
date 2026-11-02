//! FORGE runs a REAL HASH native — FNV-1a. The whole point of the Word lanes (loop-carried `Word32` state,
//! generic `xor`, modular `*`) is that they compose on real hashing code, not just isolated micro-cases.
//! FNV-1a operates on a BYTE stream (`Seq of Int` → pins as an IntList, so NO Word-list is needed): its hot
//! loop is `h = (h xor byte) * 16777619` in the Word32 ring, indexing the pinned bytes. This lock proves:
//!   • CORRECTNESS (every arch) — the forge's FNV digest == the tree-walker's, on a large input.
//!   • TIERING (x86_64) — the hash loop actually went native (`region >= 1`), not declined to the VM.
//!   • SPEED (x86_64) — native FNV clears a loose 3× floor over the bytecode VM (measured far higher).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome_with_args;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;
use std::time::{Duration, Instant};

/// FNV-1a over an `n`-byte stream `(j mod 256)`, digest folded to an Int for the `Show`.
fn fnv_src(n: usize) -> String {
    format!(
        "## To fnv1a (bytes: Seq of Int) -> Word32:\n\
         \x20   Let mutable h be word32(2166136261).\n\
         \x20   Let mutable i be 1.\n\
         \x20   Let len be length of bytes.\n\
         \x20   While i is at most len:\n\
         \x20       Set h to (h xor word32(item i of bytes)) * word32(16777619).\n\
         \x20       Set i to i + 1.\n\
         \x20   Return h.\n\
         ## Main\n\
         Let mutable bytes be a new Seq of Int.\n\
         Let mutable j be 0.\n\
         While j is less than {n}:\n\
         \x20   Push (j mod 256) to bytes.\n\
         \x20   Set j to j + 1.\n\
         Let d be fnv1a(bytes).\n\
         Show intOfWord32(d).\n"
    )
}

fn best_of<F: Fn() -> String>(reps: usize, run: F) -> (Duration, String) {
    let mut best = Duration::MAX;
    let mut out = String::new();
    for _ in 0..reps {
        let t = Instant::now();
        out = run();
        best = best.min(t.elapsed());
    }
    (best, out)
}

#[test]
fn forge_runs_fnv1a_native_and_matches_vm() {
    // CORRECTNESS + TIERING on a moderate input (fast; the hash loop still crosses enough to tier).
    let src = fnv_src(2_000_000);
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
    let vm = vm_outcome_with_args(&src, &[], None);
    assert!(
        forge.error.is_none() && vm.error.is_none(),
        "fnv1a errored: {:?} {:?}",
        forge.error,
        vm.error
    );
    assert_eq!(
        forge.output.trim(),
        vm.output.trim(),
        "forge FNV-1a disagrees with the VM (forge={:?} vm={:?})",
        forge.output.trim(),
        vm.output.trim()
    );
    #[cfg(target_arch = "x86_64")]
    {
        let (_, region_ok) = tier.region_counts();
        assert!(
            region_ok >= 1,
            "FNV-1a's hash loop did NOT tier (region={region_ok}) — a Word-lane op declined; the real hash \
             is running on the bytecode VM"
        );
    }
}

#[test]
fn forge_fnv1a_beats_the_vm() {
    // SPEED — building the byte list is unavoidable in-language, so both runs pay it equally; the RATIO
    // isolates the hash loop. Loose 3× floor with best-of-3 can't flake under contention.
    let src = fnv_src(4_000_000);
    let (forge_t, forge_out) = best_of(3, || {
        let tier = ForgeTier::new();
        vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier)).output
    });
    let (vm_t, vm_out) = best_of(3, || vm_outcome_with_args(&src, &[], None).output);
    assert_eq!(
        forge_out.trim(),
        vm_out.trim(),
        "forge FNV-1a disagrees with the VM under timing"
    );
    eprintln!(
        "forge FNV-1a: forge={:.1}ms vm={:.1}ms speedup={:.1}x",
        forge_t.as_secs_f64() * 1e3,
        vm_t.as_secs_f64() * 1e3,
        vm_t.as_secs_f64() / forge_t.as_secs_f64()
    );
    #[cfg(target_arch = "x86_64")]
    assert!(
        forge_t.as_secs_f64() * 3.0 < vm_t.as_secs_f64(),
        "forge FNV-1a was not >=3x faster than the VM (forge={:.1}ms vm={:.1}ms) — the hash loop stopped tiering",
        forge_t.as_secs_f64() * 1e3,
        vm_t.as_secs_f64() * 1e3
    );
}
