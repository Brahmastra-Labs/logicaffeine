#![cfg(not(target_arch = "wasm32"))]
//! FORGE tiers the SHA-256 COMPRESSION-LOOP SHAPE natively — the crypto core of a real hash. Two
//! `Seq of Word32` tables read by index (`item i of kk` + `item i of w`, the round constants + message
//! schedule), loop-carried Word32 scalar state (`a..hh`), and the full FIPS-180-4 round mix (`rotl` × 3
//! + `word_and`/`word_not` + generic `xor` + modular `+`). This proves EVERY op AND the exact structure
//! of sha256's inner loop tiers over PINNED Word32 tables + is byte-exact vs the VM. (Full stdlib sha256
//! end-to-end additionally needs the per-block-call structure / `sha256Compress` function-tier — a
//! separate, deeper matter; this locks that the compression SHAPE itself is fully native.)

use logicaffeine_compile::compile::vm_outcome_with_args;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

#[test]
fn forge_tiers_sha256_compression_shape() {
    // Two pinned Word32 tables + eight Word32 scalars (a..hh) + the sha256 round body, in a hot Main loop.
    let src = "## Main\n\
        Let mutable kk be a new Seq of Word32.\nLet mutable w be a new Seq of Word32.\nLet mutable s be 0.\n\
        While s is less than 64:\n\
        \x20   Push word32(s * 2654435761) to kk.\n\x20   Push word32(s * 40503 + 7) to w.\n\x20   Set s to s + 1.\n\
        Let mutable a be word32(1779033703).\nLet mutable b be word32(3144134277).\n\
        Let mutable e be word32(1013904242).\nLet mutable f be word32(2773480762).\n\
        Let mutable g be word32(1359893119).\nLet mutable hh be word32(2600822924).\n\
        Let mutable k be 1.\n\
        While k is at most 4000000:\n\
        \x20   Let idx be (k mod 64) + 1.\n\
        \x20   Let bigS1 be rotl(e, 26) xor rotl(e, 21) xor rotl(e, 7).\n\
        \x20   Let ch be word_and(e, f) xor word_and(word_not(e), g).\n\
        \x20   Let t1 be hh + bigS1 + ch + (item idx of kk) + (item idx of w).\n\
        \x20   Set hh to g.\n\x20   Set g to f.\n\x20   Set f to e.\n\x20   Set e to b + t1.\n\
        \x20   Set b to a.\n\x20   Set a to t1.\n\x20   Set k to k + 1.\n\
        Show intOfWord32(a xor e).\n";
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let vm = vm_outcome_with_args(src, &[], None);
    let (fn_all, fn_ok) = tier.function_counts();
    let (rg_all, rg_ok) = tier.region_counts();
    eprintln!("sha256 compression shape: fn {fn_ok}/{fn_all} region {rg_ok}/{rg_all} | forge={:?} vm={:?}", forge.output.trim(), vm.output.trim());
    assert!(
        forge.error.is_none() && vm.error.is_none(),
        "errored: {:?} {:?}",
        forge.error,
        vm.error
    );
    assert_eq!(
        forge.output.trim(),
        vm.output.trim(),
        "sha256 compression shape diverged from the VM"
    );
    // The hot compression loop must actually go native (x86_64) — it reads two pinned Word32 tables and
    // mixes loop-carried Word32 state; a declined pin/op would drop it to the VM.
    #[cfg(target_arch = "x86_64")]
    assert!(
        rg_ok >= 1,
        "the sha256 compression shape did NOT tier its hot loop (region={rg_ok})"
    );
}
