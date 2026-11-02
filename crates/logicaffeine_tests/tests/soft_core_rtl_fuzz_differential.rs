//! The soft-core RTL DIFFERENTIAL FUZZ — random programs run through the REAL Verilog, not just the model.
//!
//! `soft_core_fuzz_differential` proves `compile_to_full_words` → `run_soft_core_typed` (the cycle-accurate
//! Rust MODEL) equals the VM over ~4200 generated programs. `verify-soft-core-rtl.sh` proves the actual
//! `soft_core.v` Verilog equals that model — but only over a CURATED program list (~121 hand-picked idioms).
//! The residual those two leave open is the exact `$50M` class the whole campaign targets: a randomly-generated
//! op-COMBINATION where the SILICON (`soft_core.v`) diverges from the model, but which no curated testbench
//! happens to exercise. This harness closes it: a deterministic PRNG synthesizes integer-output programs, each
//! is baked into an iverilog testbench against the real `soft_core.v` (+ its RTL dependencies), and the
//! hardware's `Show` sequence is asserted BYTE-EXACT against the VM. A divergence is a real chip-vs-VM silicon
//! defect and panics with the reproducing seed + source + the iverilog transcript.
//!
//! WHY IT IS SOUND (a green run is a real guarantee, not a vacuous one):
//!   * The 32-bit `soft_core.v` `out` port carries a 32-bit value; the VM is 64-bit / arbitrary-precision. So
//!     the generator is bounded to keep EVERY intermediate and final value within ±2^30 — comfortably inside
//!     32 bits — and a belt-and-suspenders guard SKIPS (counted) any program whose model `Show` is a `BigInt`
//!     or exceeds the 30-bit window. Within that window the 32-bit hardware and the 64-bit VM MUST agree, so a
//!     mismatch is a genuine miscompile, never a width artifact.
//!   * Each program is surface-valid, TERMINATING (bounded loops, recursion depth ≤ 30), and error-free by
//!     construction (no div-by-zero, indices in range) — the VM runs it cleanly (asserted).
//!   * The model `Show` sequence is first asserted equal to the VM (`model == VM`), then the SAME values are the
//!     iverilog testbench's expected stream — so a hardware `PASS` transitively proves `RTL == VM`.
//!   * A BREADTH guard requires every integer datapath family (arithmetic, div/mod, branch, while / range /
//!     foreach loops, list build+index, functions, recursion, gcd, struct, tuple, globals, abs/min/max) to
//!     actually reach the hardware, and a floor on the total count keeps the corpus from silently decaying.
//!
//! Deterministic by seed (splitmix64): a failure reproduces exactly. iverilog-gated — if `iverilog` is not on
//! PATH the test SKIPS loudly (it is never silently vacuous on a box that has the toolchain).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::codegen_sva::soft_core_model::{run_soft_core_typed, ShowVal};
use logicaffeine_compile::compile::{
    vm_outcome, vm_outcome_concurrent, vm_outcome_net, RunOutcome,
};

/// splitmix64 — the deterministic mixer the model-level fuzz uses; no `rand`/time, so a failure reproduces.
struct Rng {
    state: u64,
}
impl Rng {
    fn new(seed: u64) -> Self {
        Rng {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// a signed operand in `[-mag, mag]`.
    fn signed(&mut self, mag: i64) -> i64 {
        (self.next() % (2 * mag as u64 + 1)) as i64 - mag
    }
}

/// A recursive `+ -` expression (NO `*`, so nesting can be arbitrarily deep and still stay 30-bit-safe with
/// bounded leaves — every node is a sum/difference of ≤ 2^20 magnitudes). Exercises the adder/subtractor and
/// deep evaluation-stack discipline on real silicon.
fn gen_addsub(rng: &mut Rng, depth: u32) -> String {
    if depth == 0 || rng.below(3) == 0 {
        return rng.below(1_000_000).to_string();
    }
    let l = gen_addsub(rng, depth - 1);
    let r = gen_addsub(rng, depth - 1);
    if rng.below(2) == 0 {
        format!("({l} + {r})")
    } else {
        format!("({l} - {r})")
    }
}

const CMPS: &[&str] = &[
    "is greater than",
    "is less than",
    "is at most",
    "is at least",
    "is equal to",
    "is not",
];

const FAMILIES: u64 = 29;
const FAMILY_NAMES: [&str; FAMILIES as usize] = [
    "addsub_expr",
    "let_chain",
    "small_mul",
    "div_mod",
    "cmp_branch",
    "while_acc",
    "range_sum",
    "list_build_index",
    "list_foreach",
    "two_arg_fn",
    "recursion",
    "gcd",
    "struct_field",
    "tuple_index",
    "global_read",
    "abs_min_max",
    "word32_bits",
    "float_arith",
    "text_concat",
    "empty_loop_edge",
    "set_ops",
    "map_ops",
    "copy_on_write",
    "tuple_destructure",
    "crdt_counter",
    "closure",
    "crdt_merge",
    "concurrency",
    "networking",
];

/// Synthesize one surface-valid, terminating, 30-bit-safe, INTEGER-OUTPUT LOGOS program for `family`.
fn gen(family: u64, rng: &mut Rng) -> String {
    match family {
        // 0 — deep +/- expression (adder/subtractor + deep nesting)
        0 => format!("## Main\n    Show {}.\n", gen_addsub(rng, 4)),

        // 1 — three-operand let-chain with +/- and small signed operands
        1 => {
            let (a, b, c) = (
                rng.signed(100_000),
                rng.signed(100_000),
                rng.signed(100_000),
            );
            let ops = ["+", "-"];
            let (o1, o2) = (ops[rng.below(2) as usize], ops[rng.below(2) as usize]);
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Let c be {c}.\n    Show (a {o1} b) {o2} c.\n")
        }

        // 2 — small multiply a * b (|product| ≤ 1e8 < 2^30) — the multiplier datapath
        2 => {
            let (a, b) = (rng.signed(10_000), rng.signed(10_000));
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Show a * b.\n")
        }

        // 3 — signed division and modulo (the shared divider), nonzero divisor
        3 => {
            let a = rng.signed(1_000_000);
            let b = ((rng.below(999) as i64) + 1) * if rng.below(2) == 0 { 1 } else { -1 };
            let op = if rng.below(2) == 0 { "/" } else { "%" };
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Show a {op} b.\n")
        }

        // 4 — comparison through if/else (the 0/1 branch datapath)
        4 => {
            let (a, b) = (rng.signed(100_000), rng.signed(100_000));
            let cmp = CMPS[rng.below(CMPS.len() as u64) as usize];
            let (x, y) = (rng.signed(100_000), rng.signed(100_000));
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    If a {cmp} b:\n        Show {x}.\n    Otherwise:\n        Show {y}.\n")
        }

        // 5 — while-loop accumulator (mutable Set, back-edge, bounded)
        5 => {
            let n = rng.below(50) + 1;
            let c = (rng.below(20) as i64) - 10;
            format!("## Main\n    Let mutable sum be 0.\n    Let mutable i be 1.\n    While i is at most {n}:\n        Set sum to sum + i * {c}.\n        Set i to i + 1.\n    Show sum.\n")
        }

        // 6 — counted range-sum (the range ITERATOR — IterPrepare/IterNext on silicon)
        6 => {
            let lo = (rng.below(20) as i64) - 10;
            let hi = lo + (rng.below(40) as i64);
            format!("## Main\n    Let mutable s be 0.\n    Repeat for x from {lo} to {hi}:\n        Set s to s + x.\n    Show s.\n")
        }

        // 7 — build a list in a loop, then index it (heap header + 1-based index)
        7 => {
            let n = rng.below(8) + 1;
            let k = rng.below(n) + 1;
            format!("## Main\n    Let mutable xs be [].\n    Let mutable i be 1.\n    While i is at most {n}:\n        Push i * i to xs.\n        Set i to i + 1.\n    Show item {k} of xs.\n")
        }

        // 8 — literal list + foreach-sum (the collection ITERATOR)
        8 => {
            let n = rng.below(6) + 1;
            let items: Vec<String> = (0..n).map(|_| rng.signed(100_000).to_string()).collect();
            format!("## Main\n    Let xs be [{}].\n    Let mutable s be 0.\n    Repeat for x in xs:\n        Set s to s + x.\n    Show s.\n", items.join(", "))
        }

        // 9 — user function with two args (call/return frame discipline); +/- keep it 30-bit-safe
        9 => {
            let (x, y) = (rng.signed(100_000), rng.signed(100_000));
            let o = ["+", "-"][rng.below(2) as usize];
            format!("## To f (a: Int, b: Int) -> Int:\n    Return a {o} b.\n## Main\n    Show f({x}, {y}).\n")
        }

        // 10 — recursion (sumdown / fib). Depth is bounded to the hardware CALL-FRAME budget: the testbench
        // instantiates `soft_core #(.FRAMES(64))`, so recursion deeper than that overflows the finite silicon
        // stack (an out-of-envelope input, like an over-width value — NOT a miscompile; the unbounded VM would
        // diverge by design). `sumdown` depth = n, so n < ~40 leaves margin under 64; `fib` depth = n and its
        // call count grows exponentially, so n < 20 keeps every program fast AND 30-bit-safe.
        10 => {
            if rng.below(2) == 0 {
                let n = rng.below(40);
                format!("## To sumdown (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Let rest be sumdown(n - 1).\n    Return n + rest.\n## Main\n    Show sumdown({n}).\n")
            } else {
                // fib(n) makes ~2·fib(n) calls; at REGS=256 the call FSM window-saves 256 regs/call, so fib(19)
                // (~8360 calls) exceeds the TB's 5e7-ns hang-detection budget. n ≤ 16 keeps the same recursion
                // BEHAVIOUR (depth + call/return machinery) well inside the budget — a throughput cap, not a value cap.
                let n = rng.below(17);
                format!("## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib({n}).\n")
            }
        }

        // 11 — Euclid's gcd end-to-end (while + mod + swap) over random inputs
        11 => {
            let a = rng.below(1_000_000) + 1;
            let b = rng.below(1_000_000) + 1;
            format!("## To gcd (a: Int, b: Int) -> Int:\n    Let mutable x be a.\n    Let mutable y be b.\n    While y is greater than 0:\n        Let t be y.\n        Set y to x % y.\n        Set x to t.\n    Return x.\n## Main\n    Show gcd({a}, {b}).\n")
        }

        // 12 — two-field struct: construct + read a random field (positional slot layout)
        12 => {
            let x = rng.signed(1_000_000);
            let y = rng.signed(1_000_000);
            let field = if rng.below(2) == 0 { "x" } else { "y" };
            format!("## Definition\nA Point has:\n    x: Int.\n    y: Int.\n## Main\n    Let p be a new Point with x {x} and y {y}.\n    Show p's {field}.\n")
        }

        // 13 — tuple: construct a 3-tuple, read a random 1-based position
        13 => {
            let (a, b, c) = (
                rng.signed(1_000_000),
                rng.signed(1_000_000),
                rng.signed(1_000_000),
            );
            let k = rng.below(3) + 1;
            format!("## Main\n    Let t be ({a}, {b}, {c}).\n    Show item {k} of t.\n")
        }

        // 14 — a function reading a Main-level GLOBAL (GlobalGet/GlobalSet)
        14 => {
            let (base, x) = (rng.signed(100_000), rng.signed(100_000));
            format!("## To addbase (n: Int) -> Int:\n    Return n + base.\n## Main\n    Let base be {base}.\n    Show addbase({x}).\n")
        }

        // 15 — integer builtins abs / min / max (the op-38 coprocessor), single small value out
        15 => {
            let (a, b) = (rng.signed(100_000_000), rng.signed(100_000_000));
            match rng.below(3) {
                0 => format!("## Main\n    Show abs({a}).\n"),
                1 => format!("## Main\n    Show min({a}, {b}).\n"),
                _ => format!("## Main\n    Show max({a}, {b}).\n"),
            }
        }

        // 16 — Word32 ring bit-ops (and/or/xor/not/rotl/rotr/shr) shown UNSIGNED off the `outw` port (kind 3).
        // FULL-RANGE 32-bit operands + edge shift/rotate counts (1..31) — the off-by-one-prone datapath where a
        // range-style silicon bug would hide; no 30-bit restriction (a Word32 is exactly 32 bits by definition).
        16 => {
            let a = rng.next() as u32;
            let b = rng.next() as u32;
            match rng.below(7) {
                0 => format!("## Main\n    Let a be word32({a}).\n    Let b be word32({b}).\n    Show intOfWord32(word_and(a, b)).\n"),
                1 => format!("## Main\n    Let a be word32({a}).\n    Let b be word32({b}).\n    Show intOfWord32(word_or(a, b)).\n"),
                2 => format!("## Main\n    Let a be word32({a}).\n    Let b be word32({b}).\n    Let c be a xor b.\n    Show intOfWord32(c).\n"),
                3 => format!("## Main\n    Show intOfWord32(word_not(word32({a}))).\n"),
                4 => format!("## Main\n    Show intOfWord32(rotl(word32({a}), {})).\n", rng.below(31) + 1),
                5 => format!("## Main\n    Show intOfWord32(rotr(word32({a}), {})).\n", rng.below(31) + 1),
                _ => format!("## Main\n    Show intOfWord32(word32Shr(word32({a}), {})).\n", rng.below(31) + 1),
            }
        }

        // 17 — IEEE-754 float arithmetic (the multi-cycle FPU: add/sub/mul/div/sqrt), shown off `outf` (kind 1)
        // and asserted BIT-FOR-BIT against the VM's f64. Operands are bound to VARIABLES first so the runtime
        // FPU is genuinely exercised (not constant-folded at compile time); the divisor carries a fractional part
        // ≥ 0.01 (LOGOS errors on 0.0/0.0 by design, so no NaN is generated).
        17 => {
            // magnitude ≥ 0.1 (never an exact zero divisor, and never -0.0 whose rendering is ambiguous)
            let mag = |rng: &mut Rng| format!("{}.{}", rng.below(1000), rng.below(99) + 1);
            let sgn = |rng: &mut Rng| if rng.below(2) == 0 { "-" } else { "" };
            // BOTH signs on a and b so the FPU sign path (add/sub/mul/div of negatives) is genuinely exercised.
            let a = format!("{}{}", sgn(rng), mag(rng));
            let b = format!("{}{}", sgn(rng), mag(rng));
            let head = format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n");
            match rng.below(5) {
                0 => format!("{head}    Show a + b.\n"),
                1 => format!("{head}    Show a - b.\n"),
                2 => format!("{head}    Show a * b.\n"),
                3 => format!("{head}    Show a / b.\n"),
                // sqrt requires a NON-NEGATIVE operand (LOGOS errors on sqrt of a negative) — use a fresh positive.
                _ => format!("## Main\n    Let b be {}.\n    Show sqrt(b).\n", mag(rng)),
            }
        }

        // 18 — Text: string concat + int interpolation, Shown as a STREAM over `outs` (the length word kind 2,
        // then one codepoint kind 4 per char) — the TextConst pool copy + SeqConcat + ShowText streaming FSM.
        // Random lowercase strings of random length exercise the codepoint copy and the length header.
        18 => {
            let letters = b"abcdefghijklmnopqrstuvwxyz";
            let mk = |rng: &mut Rng| -> String {
                let len = rng.below(6) + 1;
                (0..len)
                    .map(|_| letters[rng.below(26) as usize] as char)
                    .collect()
            };
            match rng.below(3) {
                0 => {
                    let s = mk(rng);
                    format!("## Main\n    Show \"{s}\".\n")
                }
                1 => {
                    let (s1, s2) = (mk(rng), mk(rng));
                    format!("## Main\n    Let s be \"{s1}\" + \"{s2}\".\n    Show s.\n")
                }
                _ => {
                    let s = mk(rng);
                    let n = rng.signed(100_000);
                    format!("## Main\n    Show \"{s} \" + {n}.\n")
                }
            }
        }

        // 19 — EMPTY / ZERO-ITERATION loops: the loop-exit + empty-collection edge the other families AVOID
        // (they force ≥ 1 iteration). The range bug WAS an empty-range bug (a non-empty range wrongly going empty),
        // so the genuinely-empty case — a range with hi < lo, a while that is false at entry, an empty-list
        // foreach — must ALSO run 0 iterations and Show the untouched initial value byte-exact on real silicon.
        19 => {
            match rng.below(3) {
                0 => {
                    let lo = rng.signed(1000);
                    let hi = lo - (rng.below(20) as i64) - 1; // hi < lo → an EMPTY range (0 iterations)
                    format!("## Main\n    Let mutable s be 7.\n    Repeat for x from {lo} to {hi}:\n        Set s to s + x.\n    Show s.\n")
                }
                1 => {
                    let init = rng.signed(100_000);
                    format!("## Main\n    Let mutable s be {init}.\n    Let mutable i be 1.\n    While i is at most 0:\n        Set s to s + 1.\n        Set i to i + 1.\n    Show s.\n")
                }
                _ => {
                    let init = rng.signed(100_000);
                    format!("## Main\n    Let xs be [].\n    Let mutable s be {init}.\n    Repeat for x in xs:\n        Set s to s + x.\n    Show s.\n")
                }
            }
        }

        // 20 — Set of Int: repeated Adds over a small range so duplicates are likely → the dedup that determines
        // `length` is genuinely exercised (the RTL set-membership scan must match the VM).
        20 => {
            let n = rng.below(6) + 2;
            let mut adds = String::new();
            for _ in 0..n {
                adds.push_str(&format!("    Add {} to s.\n", rng.below(10)));
            }
            format!("## Main\n    Let s be a new Set of Int.\n{adds}    Show length of s.\n")
        }

        // 21 — string-key Map: Set/Get by Text key + length (NewEmptyMap + map SetIndex/Index on the heap)
        21 => {
            let (a, b) = (rng.signed(100_000), rng.signed(100_000));
            let keys = ["a", "b", "c", "x", "y"];
            let (k1, k2) = (keys[rng.below(5) as usize], keys[rng.below(5) as usize]);
            format!("## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item \"{k1}\" of m to {a}.\n    Set item \"{k2}\" of m to {b}.\n    Show item \"{k1}\" of m.\n    Show length of m.\n")
        }

        // 22 — value semantics / copy-on-write (DeepClone, op 66): mutate the original after `copy of`, the copy
        // stays intact — the RTL must deep-copy, not alias.
        22 => {
            let (a, b) = (rng.signed(100_000), rng.signed(100_000));
            let newv = rng.signed(100_000);
            format!("## Main\n    Let a be [{a}, {b}].\n    Let c be copy of a.\n    Set item 1 of a to {newv}.\n    Show item 1 of c.\n")
        }

        // 23 — tuple destructuring in a for-each (DestructureTuple, op 71): spread each (p, q) and accumulate
        23 => {
            let (a, b, c, d) = (
                rng.signed(100_000),
                rng.signed(100_000),
                rng.signed(100_000),
                rng.signed(100_000),
            );
            format!("## Main\n    Let pairs be [({a}, {b}), ({c}, {d})].\n    Let mutable s be 0.\n    Repeat for (p, q) in pairs:\n        Set s to s + p - q.\n    Show s.\n")
        }

        // 24 — CRDT G-counter: a `Counter` field bumped twice via Increase (CrdtBump, ops 80-84). Monotonic, so
        // the increments are non-negative; the final count must match the VM's converged value.
        24 => {
            let (a, b) = (rng.below(10_000), rng.below(10_000));
            format!("## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable t be a new Tally.\n    Increase t's n by {a}.\n    Increase t's n by {b}.\n    Show t's n.\n")
        }

        // 25 — non-capturing closure passed to a higher-order function and invoked (MakeClosure / CallValue)
        25 => {
            let (k, x) = (rng.signed(100_000), rng.signed(100_000));
            format!("## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n + {k}, {x}).\n")
        }

        // 26 — CRDT field-wise merge (CrdtMerge): sum-join two counters, then read both fields
        26 => {
            let (a, b) = (rng.below(10_000) as i64, rng.below(10_000) as i64);
            format!("## A Counter has:\n    a: Int.\n    b: Int.\n## Main\n    Let mutable x be a new Counter with a {a}.\n    Let mutable y be a new Counter with b {b}.\n    Merge y into x.\n    Show x's a.\n    Show x's b.\n")
        }

        // 27 — deterministic CONCURRENCY (the soft-core's synchronous cooperative scheduler: Pipe/Launch/Receive,
        // Sleep, Try-send/recv, Launch+Stop handle, Select). Uses the `vm_outcome_concurrent` oracle (family 27).
        27 => {
            let v = rng.below(1_000_000) as i64;
            match rng.below(5) {
                0 => format!("## To produce (ch: Int):\n    Send {v} into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive first from jobs.\n    Show first.\n"),
                1 => format!("## Main\n    Show 1.\n    Sleep {}.\n    Show 2.\n", rng.below(200)),
                2 => format!("## Main\n    Let ch be a Pipe of Int.\n    Try to send {v} into ch.\n    Try to receive x from ch.\n    Show x.\n"),
                3 => format!("## To worker (ch: Int):\n    Send {v} into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n"),
                _ => format!("## To producer (ch: Int):\n    Send {v} into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 0.\n"),
            }
        }

        // 28 — offline-loopback NETWORKING (Listen/PeerAgent/Send/Await, Connect/Stream). Uses `vm_outcome_net`.
        _ => {
            let v = rng.below(1_000_000) as i64;
            if rng.below(2) == 0 {
                format!("## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send {v} to peer.\n    Await response from peer into got.\n    Show got.\n")
            } else {
                "## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Listen on \"me\".\n    Let items be [1, 2, 3].\n    Stream items to \"sink\".\n    Show 7.\n".to_string()
            }
        }
    }
}

fn silicon_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// A typed soft-core testbench: preload `prog` into `dut.imem`, run `soft_core.v` at the wide FPGA config, and
/// capture every `Show` into ONE 64-bit stream — a SIGNED int off `out` (kind 0, low 32), an IEEE-754 double off
/// `outf` (kind 1, full 64), and an UNSIGNED Word32 off `outw` (kind 3, low 32). Each entry is asserted against
/// `expect` by (kind, 64-bit-value): ints/word32 compare the low 32 bits (distinguished by kind = host sign),
/// floats compare bit-for-bit EXCEPT a NaN expectation accepts any quiet-NaN pattern (`f64::NAN` is not unique).
fn emit_typed_tb(prog: &[u32], expect: &[(u8, u64)], name: &str) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "`timescale 1ns/1ps");
    let _ = writeln!(s, "module tb;");
    let _ = writeln!(s, "  reg clk = 0, rst = 1; wire [31:0] out; wire [63:0] outf; wire [31:0] outw; wire [31:0] outs;");
    let _ = writeln!(s, "  wire ov, ofv, owv, osv, osf, halted;");
    let _ = writeln!(
        s,
        "  soft_core #(.FRAMES(64), .REGS(256), .IMEM_WORDS(1024), .HEAP(4096))"
    );
    let _ = writeln!(
        s,
        "    dut(.clk(clk), .rst(rst), .out(out), .out_valid(ov), .outf(outf), .outf_valid(ofv),"
    );
    let _ = writeln!(s, "        .outw(outw), .outw_valid(owv), .outs(outs), .outs_valid(osv), .outs_first(osf), .halted(halted));");
    // ONE ordered capture stream. A string `Show` streams over `outs`: the FIRST word (outs_first) is the char
    // COUNT (kind 2), then one codepoint per cycle (kind 4). Scalars are single words: int off `out` (kind 0),
    // float off `outf` (kind 1), word32 off `outw` (kind 3). The kind proves which port/host-rendering fired.
    let _ = writeln!(
        s,
        "  integer n = 0; reg [2:0] skind [0:255]; reg [63:0] sval [0:255];"
    );
    let _ = writeln!(s, "  always #5 clk = ~clk;");
    let _ = writeln!(s, "  always @(posedge clk) begin");
    let _ = writeln!(
        s,
        "    if (ov)  begin skind[n] = 3'd0; sval[n] = {{32'd0, out}};  n = n + 1; end"
    );
    let _ = writeln!(
        s,
        "    if (ofv) begin skind[n] = 3'd1; sval[n] = outf;            n = n + 1; end"
    );
    let _ = writeln!(
        s,
        "    if (owv) begin skind[n] = 3'd3; sval[n] = {{32'd0, outw}}; n = n + 1; end"
    );
    let _ = writeln!(s, "    if (osv) begin skind[n] = osf ? 3'd2 : 3'd4; sval[n] = {{32'd0, outs}}; n = n + 1; end");
    let _ = writeln!(s, "  end");
    let _ = writeln!(s, "  function is_nan(input [63:0] v); is_nan = (v[62:52] == 11'h7FF) && (v[51:0] != 52'd0); endfunction");
    let _ = writeln!(s, "  reg fail;");
    let _ = writeln!(s, "  initial begin");
    let _ = writeln!(s, "    #1;");
    for (i, word) in prog.iter().enumerate() {
        let _ = writeln!(s, "    dut.imem[{i}] = 32'h{word:08X};");
    }
    let _ = writeln!(s, "    #3 rst = 0;");
    let _ = writeln!(s, "    wait (halted);");
    let _ = writeln!(s, "    #10;");
    let _ = writeln!(s, "    fail = 0;");
    let _ = writeln!(s, "    if (n != {}) begin fail = 1; $display(\"FAIL: {name} — got %0d shows, want {}\", n); end", expect.len(), expect.len());
    for (i, (k, v)) in expect.iter().enumerate() {
        if *k == 1 && (*v >> 52) & 0x7ff == 0x7ff && (*v & 0xf_ffff_ffff_ffff) != 0 {
            // NaN expectation: accept any quiet-NaN payload (still assert the kind + that it IS a NaN).
            let _ = writeln!(s, "    if (!fail && (skind[{i}] !== 3'd1 || !is_nan(sval[{i}]))) begin fail = 1; $display(\"FAIL: {name} — show {i} kind=%0d val=%h, want a NaN\", skind[{i}], sval[{i}]); end");
        } else if *k == 1 {
            let _ = writeln!(s, "    if (!fail && (skind[{i}] !== 3'd1 || sval[{i}] !== 64'h{v:016X})) begin fail = 1; $display(\"FAIL: {name} — show {i} kind=%0d val=%h, want kind=1 {v:016X}\", skind[{i}], sval[{i}]); end");
        } else {
            let v32 = *v as u32;
            let _ = writeln!(
                s,
                "    if (!fail && (skind[{i}] !== 3'd{k} || sval[{i}][31:0] !== 32'h{v32:08X})) begin fail = 1; $display(\"FAIL: {name} — show {i} kind=%0d 0x%08X, want kind={k} 0x{v32:08X}\", skind[{i}], sval[{i}][31:0]); end"
            );
        }
    }
    let _ = writeln!(s, "    if (!fail) $display(\"PASS: {name}\");");
    let _ = writeln!(s, "    $finish;");
    let _ = writeln!(s, "  end");
    // Every in-envelope program (bounded loops, recursion ≤ FRAMES, fib(<20), a few multi-cycle FPU ops) halts
    // far under 5e7 ns; a genuine hang then FAILS LOUD in ~seconds of wall time instead of a multi-billion-ns wall.
    let _ = writeln!(
        s,
        "  initial begin #50000000 $display(\"FAIL: {name} timeout\"); $finish; end"
    );
    let _ = writeln!(s, "endmodule");
    s
}

/// Compile `prog`'s testbench against the REAL `soft_core.v` (+ deps) with iverilog and run it; `Ok` iff the
/// hardware printed `PASS`, else `Err(transcript)`.
fn run_on_rtl(
    prog: &[u32],
    expect: &[(u8, u64)],
    name: &str,
    workdir: &Path,
    sil: &Path,
) -> Result<(), String> {
    let tb_path = workdir.join(format!("{name}.v"));
    let vvp_path = workdir.join(format!("{name}.vvp"));
    std::fs::write(&tb_path, emit_typed_tb(prog, expect, name))
        .map_err(|e| format!("write tb: {e}"))?;
    let deps = [
        "alu.v",
        "shared_divider.v",
        "crdt_merge.v",
        "bigint_alu.v",
        "bigint_div.v",
        "fpu.v",
        "soft_core.v",
    ];
    let mut compile = Command::new("iverilog");
    compile.arg("-g2012").arg("-o").arg(&vvp_path);
    for d in deps {
        compile.arg(sil.join(d));
    }
    compile.arg(&tb_path);
    let c = compile
        .output()
        .map_err(|e| format!("spawn iverilog: {e}"))?;
    if !c.status.success() {
        return Err(format!(
            "iverilog compile failed:\n{}",
            String::from_utf8_lossy(&c.stderr)
        ));
    }
    let r = Command::new("vvp")
        .arg(&vvp_path)
        .output()
        .map_err(|e| format!("spawn vvp: {e}"))?;
    let out = String::from_utf8_lossy(&r.stdout);
    // Drop iverilog's benign `WARNING` lines; the meaningful line is the single PASS/FAIL the TB prints.
    let meaningful: String = out
        .lines()
        .filter(|l| !l.trim_start().starts_with("WARNING"))
        .collect::<Vec<_>>()
        .join("\n");
    if meaningful.contains(&format!("PASS: {name}")) {
        Ok(())
    } else {
        Err(format!("hardware did not PASS:\n{meaningful}"))
    }
}

const SEEDS_PER_FAMILY: u64 = 10;
/// Each family must reach the HARDWARE on ≥ 90% of its seeds (mirrors the model fuzz's per-category floor) — a
/// family that silently starts skipping (a lowering regression narrowed to one datapath) then goes RED.
const PER_FAMILY_FLOOR: u64 = SEEDS_PER_FAMILY * 9 / 10;
/// Total floor = the sum of the per-family floors, so the whole corpus can't decay either.
const RAN_FLOOR: u64 = FAMILIES * PER_FAMILY_FLOOR;

/// The VM oracle for a family. The scheduler ops (chan/spawn/select/sleep, family 27) are DECLINED by plain
/// `vm_outcome` ("requires the scheduler driver") — the soft-core's synchronous cooperative Spawn/Chan mirrors
/// `vm_outcome_concurrent` (seed 0, the same oracle the model fuzz + AOT concurrency lock use). Offline
/// networking (family 28) needs `vm_outcome_net` (plain `vm_outcome` errors on a bare Connect/Listen).
fn vm_oracle(family: u64, src: &str) -> RunOutcome {
    match family {
        27 => vm_outcome_concurrent(src),
        28 => vm_outcome_net(src),
        _ => vm_outcome(src),
    }
}

/// The RTL fuzz: many deterministic seeds × every integer datapath family, each program run through the REAL
/// `soft_core.v` under iverilog, hardware `Show` sequence asserted byte-exact against the VM. A divergence panics
/// with the reproducing seed + source — that is the `$50M` silicon miscompile find. The tallies below lock the
/// run against vacuity (enough programs actually reached the hardware; every family did).
#[test]
fn soft_core_rtl_equals_vm_over_a_generated_corpus() {
    if !common::require_tool("iverilog") {
        return;
    }
    let sil = silicon_dir();
    for f in [
        "alu.v",
        "shared_divider.v",
        "crdt_merge.v",
        "bigint_alu.v",
        "bigint_div.v",
        "fpu.v",
        "soft_core.v",
    ] {
        assert!(
            sil.join(f).is_file(),
            "missing RTL source {f} in {}",
            sil.display()
        );
    }
    let workdir = std::env::temp_dir().join("logos_soft_core_rtl_fuzz");
    std::fs::create_dir_all(&workdir).expect("create workdir");

    let mut ran = 0u64;
    let mut boundary = 0u64;
    let mut over_width = 0u64;
    let mut ran_per_family = vec![0u64; FAMILIES as usize];
    let mut opcode_union: BTreeSet<u8> = BTreeSet::new();

    for family in 0..FAMILIES {
        for s in 0..SEEDS_PER_FAMILY {
            let seed = family * 1_000 + s;
            let mut rng = Rng::new(seed);
            let src = gen(family, &mut rng);

            let words = match compile_to_full_words(&src) {
                Ok(w) => w,
                Err(_) => {
                    boundary += 1;
                    continue;
                }
            };
            let vm = vm_oracle(family, &src);
            assert!(
                vm.error.is_none(),
                "the generator produced a VM-erroring program (generator bug)\n  seed={seed}\n  src={src:?}\n  err={:?}",
                vm.error
            );

            let shows = run_soft_core_typed(&words, 50_000_000);
            // Build the (kind, 32-bit-value) expected stream. A signed `Int` must sit in the 30-bit-safe window
            // where the 32-bit hardware and the 64-bit VM MUST agree (kind 0, `out` port); a `Word32` is an
            // exact 32-bit ring value with no width ambiguity (kind 3, `outw` port). Anything else (BigInt that
            // overflows 32 bits, Float, Text) is out of this differential's representable set — skipped, counted.
            let mut expect: Vec<(u8, u64)> = Vec::new();
            let mut representable = true;
            for sh in &shows {
                match sh {
                    ShowVal::Int(v) if v.abs() < (1 << 30) => expect.push((0, (*v as u32) as u64)),
                    ShowVal::Word32(w) => expect.push((3, *w as u64)),
                    ShowVal::Float(f) => expect.push((1, f.to_bits())),
                    ShowVal::Text(t) => {
                        // A string streams as a length word (kind 2) then one codepoint per cycle (kind 4).
                        let cps: Vec<u32> = t.chars().map(|c| c as u32).collect();
                        expect.push((2, cps.len() as u64));
                        for cp in cps {
                            expect.push((4, cp as u64));
                        }
                    }
                    // A signed Int that overflows the 30-bit window, or ANY BigInt, is out of this
                    // differential's representable set (the 32-bit `out` port would truncate) — skip, counted.
                    // The match is EXHAUSTIVE (no `_`): a new `ShowVal` kind breaks the build until handled here.
                    ShowVal::Int(_) | ShowVal::BigInt(_) => {
                        representable = false;
                        over_width += 1;
                        break;
                    }
                }
            }
            if !representable {
                continue;
            }

            // Sanity: the model already equals the VM (the model-level guarantee); if not, the bug is in the
            // generator/model, NOT the hardware — surface it distinctly so an RTL failure is never misattributed.
            let model_str = shows
                .iter()
                .map(ShowVal::fmt)
                .collect::<Vec<_>>()
                .join("\n");
            let want = vm.output.strip_suffix('\n').unwrap_or(vm.output.as_str());
            assert_eq!(
                model_str, want,
                "MODEL != VM (generator/model bug, not the RTL)\n  seed={seed}\n  src={src:?}"
            );

            let name = format!("rtlfuzz_{}_s{s}", FAMILY_NAMES[family as usize]);
            match run_on_rtl(&words, &expect, &name, &workdir, &sil) {
                Ok(()) => {
                    ran += 1;
                    ran_per_family[family as usize] += 1;
                    for w in &words {
                        opcode_union.insert((w >> 24) as u8);
                    }
                }
                Err(transcript) => panic!(
                    "SOFT-CORE RTL != VM — a real silicon miscompile on the actual Verilog ($50M class)\n  \
                     seed={seed}\n  family={}\n  src={src:?}\n  expect(kind,u32)={expect:X?}\n  {transcript}",
                    FAMILY_NAMES[family as usize]
                ),
            }
        }
    }

    eprintln!(
        "soft-core RTL fuzz: {ran} ran on real soft_core.v via iverilog, {boundary} boundary, {over_width} \
         over-width; per-family={ran_per_family:?}; ~{} distinct opcode-bytes",
        opcode_union.len()
    );

    // Non-vacuity 1: a healthy majority of generated programs must actually reach the HARDWARE.
    assert!(
        ran >= RAN_FLOOR,
        "only {ran} programs ran on the real RTL (floor {RAN_FLOOR}) — the harness drifted off the lowered \
         integer surface; the silicon differential is near-vacuous"
    );
    // Non-vacuity 2 (breadth): EVERY integer datapath family must reach the hardware, or its silicon path is
    // unexercised. A family that stops reaching the RTL (a lowering regression narrowed to one datapath) goes
    // RED here. Never lower a family's floor to make a shrinking corpus pass — fix the lowering.
    for (f, &c) in ran_per_family.iter().enumerate() {
        assert!(
            c >= PER_FAMILY_FLOOR,
            "family {} ({}) reached the real RTL only {c}/{SEEDS_PER_FAMILY} times (floor {PER_FAMILY_FLOOR}) — \
             its silicon datapath fell off the lowered surface; the fuzz's breadth collapsed there",
            f, FAMILY_NAMES[f]
        );
    }
}

/// Reproduction probe — dump the exact source + model shows + compiled opcodes for a family's seeds, so a
/// hardware divergence can be diagnosed without guessing the RNG by hand.
#[test]
#[ignore]
fn dump_family_range_sum() {
    let family = std::env::var("DUMP_FAMILY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6u64);
    for s in 0..SEEDS_PER_FAMILY {
        let seed = family * 1_000 + s;
        let src = gen(family, &mut Rng::new(seed));
        let words = compile_to_full_words(&src).expect("compile");
        let shows = run_soft_core_typed(&words, 50_000_000);
        let vm = vm_outcome(&src);
        let ops: Vec<u8> = words.iter().map(|w| (w >> 24) as u8).collect();
        eprintln!(
            "seed={seed}\n  src={src:?}\n  model={:?}\n  vm={:?}\n  opcodes={ops:?}",
            shows.iter().map(ShowVal::fmt).collect::<Vec<_>>(),
            vm.output
        );
    }
}

/// Probe the bigint-output question: does a value that overflows 32 bits compile to the plain 32-bit Mul+Show
/// path (⇒ the RTL would truncate while the model promotes to BigInt), or is it rejected / routed to a wide
/// path? Prints the opcodes + the model's Show kind so the exposure can be judged. Run:
/// `cargo nextest run -E 'test(probe_bigint_lowering)' --run-ignored all --no-capture`
#[test]
#[ignore]
fn probe_bigint_lowering() {
    for src in [
        "## Main\n    Let a be 2000000000.\n    Let b be 2000000000.\n    Show a * b.\n",
        "## Main\n    Show 3000000000.\n",
        "## Main\n    Let a be 1000000000.\n    Let b be 1000000000.\n    Let c be a * b.\n    Show c + c.\n",
    ] {
        match compile_to_full_words(src) {
            Ok(words) => {
                let ops: Vec<u8> = words.iter().map(|w| (w >> 24) as u8).collect();
                let shows = run_soft_core_typed(&words, 50_000_000);
                let kinds: Vec<&str> = shows.iter().map(|s| match s {
                    ShowVal::Int(_) => "Int", ShowVal::BigInt(_) => "BigInt",
                    ShowVal::Float(_) => "Float", ShowVal::Text(_) => "Text", ShowVal::Word32(_) => "Word32",
                }).collect();
                eprintln!("COMPILED  src={src:?}\n  opcodes={ops:?}\n  model shows={:?} kinds={kinds:?}",
                    shows.iter().map(ShowVal::fmt).collect::<Vec<_>>());
            }
            Err(e) => eprintln!("REJECTED (boundary) src={src:?}\n  err={}", e.lines().next().unwrap_or("")),
        }
    }
}

/// The generator is a pure function of its seed: a failure reproduces exactly (no `rand`/time/global state).
#[test]
fn the_rtl_generator_is_deterministic() {
    for seed in [0u64, 1, 7, 42, 1000, 15005] {
        for family in 0..FAMILIES {
            let a = gen(family, &mut Rng::new(seed));
            let b = gen(family, &mut Rng::new(seed));
            assert_eq!(
                a, b,
                "gen(seed={seed}, family={family}) is non-deterministic"
            );
        }
    }
}
