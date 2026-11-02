//! The cross-block probe programs — collections grown past one block so the bump allocator reaches high heap
//! addresses, and a chain of blocks — checked on the VM, with the heap-address reach they need, before any
//! gate-level run of the same programs.
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_counted_heap_preloaded, serialize_program,
};

fn vm_preloaded(src: &str, fn_name: &str, input: i64, heap_init: &[i64]) -> i64 {
    let cfg = serialize_program(&program_cfg_fabric(src, fn_name));
    eval_wprog_config_counted_heap_preloaded(&cfg, &[input], heap_init).0
}

// PROGRAM 1 — HIGH-ADDRESS CROSS-BLOCK HEAP. Grow 8 collections (each reserves 1+2048=2049 words) so the bump
// allocator marches past 16384; the 8th collection's Push lands at a heap address > 16384. Write distinctive
// (incl negative) values across that boundary in one block, then in a LATER block read them back and combine.
const HIGH_ADDR_SRC: &str = "\
## To acc (n: Int) -> Int:
    Let a be a new Seq of Int.
    Let b be a new Seq of Int.
    Let c be a new Seq of Int.
    Let d be a new Seq of Int.
    Let e be a new Seq of Int.
    Let f be a new Seq of Int.
    Let g be a new Seq of Int.
    Let h be a new Seq of Int.
    Push 111 to h.
    Push -222 to h.
    Push 333 to h.
    Push -444 to h.
    Let mutable s be 0.
    Set s to s + (item 1 of h).
    Set s to s + (item 2 of h).
    Set s to s + (item 3 of h).
    Set s to s + (item 4 of h).
    Return s.
";

#[test]
fn scratch_high_addr_vm() {
    let empty: Vec<i64> = Vec::new();
    let vm = vm_preloaded(HIGH_ADDR_SRC, "acc", 0, &empty);
    eprintln!("HIGH_ADDR vm = {vm} (expect 111-222+333-444 = -222)");
    assert_eq!(vm, 111 - 222 + 333 - 444);
}

// PROGRAM 2 — BLOCK-TO-BLOCK STATE CHAIN. Block N's computed result feeds block N+1's branch, several times.
const CHAIN_SRC: &str = "\
## To g (x: Int) -> Int:
    Let mutable a be x + 7.
    Let mutable b be 0.
    If a is greater than 10:
        Set b to a * 3.
    Otherwise:
        Set b to a + 100.
    Let mutable c be 0.
    If b is greater than 40:
        Set c to b - 5.
    Otherwise:
        Set c to b + 5.
    Let mutable d be 0.
    If (c % 2) is equal to 0:
        Set d to c * 2.
    Otherwise:
        Set d to c - 1.
    Return d.
";

#[test]
fn scratch_chain_vm() {
    let empty: Vec<i64> = Vec::new();
    for x in [1i64, 6, 20] {
        let vm = vm_preloaded(CHAIN_SRC, "g", x, &empty);
        eprintln!("CHAIN g({x}) = {vm}");
    }
}

// PROGRAM 3 — ARGMAX-OVER-MANY spanning the 16384 boundary. Grow to a high base, then store 16 distinctive
// values (incl negatives, high bit-planes) straddling 16384 via a runtime-grown collection, then argmax and
// return best*100 + idx.
const ARGMAX_SRC: &str = "\
## To acc (n: Int) -> Int:
    Let a be a new Seq of Int.
    Let b be a new Seq of Int.
    Let c be a new Seq of Int.
    Let d be a new Seq of Int.
    Let e be a new Seq of Int.
    Let f be a new Seq of Int.
    Let g be a new Seq of Int.
    Let h be a new Seq of Int.
    Push 5 to h.
    Push -80 to h.
    Push 1300 to h.
    Push 12 to h.
    Push -4000 to h.
    Push 770 to h.
    Push 25600 to h.
    Push -1 to h.
    Push 9000 to h.
    Push 43 to h.
    Push -9000 to h.
    Push 640 to h.
    Push 100000 to h.
    Push 7 to h.
    Push -50 to h.
    Push 5110 to h.
    Let mutable best be item 1 of h.
    Let mutable bi be 0.
    Let mutable i be 1.
    While i is less than 16:
        Let w be item (i + 1) of h.
        If w is greater than best:
            Set best to w.
            Set bi to i.
        Set i to i + 1.
    Return (best * 100) + bi.
";

#[test]
fn scratch_argmax_vm() {
    let empty: Vec<i64> = Vec::new();
    let vm = vm_preloaded(ARGMAX_SRC, "acc", 0, &empty);
    // vals: idx12 = 100000 is max → 100000*100 + 12 = 10000012
    eprintln!("ARGMAX vm = {vm} (expect 100000*100+12 = 10000012)");
    assert_eq!(vm, 100000 * 100 + 12);
}
