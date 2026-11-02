//! The soft-core DIFFERENTIAL FUZZ harness — the missing piece the hand-written corpus could not cover.
//!
//! `universal_completeness_lock` proves the chip (`compile_to_full_words` → `run_soft_core_typed`) equals the
//! VM over ~100 HAND-WRITTEN programs plus a few hundred hand-picked arithmetic constants. That is breadth by
//! enumeration. This harness is breadth by GENERATION: a deterministic PRNG synthesizes a large, diverse
//! corpus of surface-valid LOGOS programs — deep nested integer expressions (the BigInt-promotion / sign /
//! overflow stressor) plus 42 template families drawn from the proven idioms with every numeric and structural
//! parameter randomized (arithmetic, division, comparison, loops, lists/sets/tuples/maps, structs/enums,
//! functions, recursion, closures, globals, CoW-clone, concat, pop, contains, destructure, float scalar+seq,
//! Word32 and/or/xor/not/rotate, text, pow, CRDT bump+merge, gcd, numeric+string builtins [sqrt/abs/min/max/
//! floor/ceil/round/count_ones/chr/parseInt], concurrency [chan/spawn/handle/sleep/try/select], networking
//! [connect/listen/send/stream/await/sync]) — and runs the SAME differential on each. This reaches ~99 of the
//! chip-lowered opcodes — essentially the WHOLE lowered surface; the only ops it cannot cover are the honest
//! host/accelerator/nondeterministic BOUNDARIES the encoder rejects by design (`LoadNow`/`CheckPolicy`/
//! `parseFloat`/`format`/UUID/money/wire-codec/compression/crypto/FEC/SIMD/DOM), catalogued in
//! `soft_core_op_support_lock`.
//!
//! WHY THIS IS SOUND (and why a green run is a real guarantee, not a vacuous one):
//!   * Every generated program is surface-valid, TERMINATING (bounded loops, recursion depth ≤ 18), and
//!     ERROR-FREE by construction (no div-by-zero, indices in range) — so the VM runs it cleanly.
//!   * The full-core encoder ends in a LOUD catch-all `Err("… has no full-core chip lowering yet …")`. So the
//!     encoder itself is the honest boundary filter: a generated program that reaches an unlowered/boundary op
//!     returns `Err` and is SKIPPED (counted `boundary_skipped`). Every program that DOES lower must therefore
//!     print, on the cycle-accurate soft-core oracle, EXACTLY what the VM prints (the VM oracle is selected per
//!     family by `vm_oracle` — `vm_outcome_concurrent` for the concurrency family, `vm_outcome_net` for the
//!     networking one, `vm_outcome` otherwise). Any divergence is a real chip-vs-VM defect — the $50M
//!     silent-miscompile class — and the harness panics with the reproducing seed and source.
//!   * A BREADTH RATCHET locks the fuzz into the pit of success: EVERY one of the 42 datapath families must
//!     genuinely lower a healthy share of its programs, so the corpus can never silently decay into trivial
//!     `LoadConst;Show` programs and pass while testing nothing. (This is artifact-free per-family coverage,
//!     not a raw opcode count — instruction widths are context-dependent, so a word-walk cannot perfectly
//!     enumerate opcodes; the ~99 figure is a coarse diagnostic only.)
//!
//! Deterministic by seed (splitmix64, no `rand`/`Math.random` flake): a failure reproduces from its seed.

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::codegen_sva::soft_core_model::{run_soft_core_typed, ShowVal};
use logicaffeine_compile::compile::{
    vm_outcome, vm_outcome_concurrent, vm_outcome_net, RunOutcome,
};

/// splitmix64 — the same deterministic mixer `universal_completeness_lock` uses for its arithmetic fuzzes.
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
    /// uniform in `[0, n)` (n > 0).
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// a full-range signed operand — small, medium, or near-i64 extremes so sums/products cross the i64
    /// boundary and force the same unbounded-`Int`/BigInt promotion on both chip and VM.
    fn operand(&mut self) -> i64 {
        match self.below(4) {
            0 => (self.below(200) as i64) - 100, // small, both signs
            1 => self.next() as i64 / 2,         // medium
            2 => self.next() as i64,             // full i64, both signs
            _ => {
                // near an extreme, so a following `*` overflows into BigInt
                let base = if self.below(2) == 0 {
                    i64::MAX
                } else {
                    i64::MIN
                };
                base.wrapping_add((self.below(2000) as i64) - 1000)
            }
        }
    }
    /// a non-negative leaf for nested-expression positions (sign is introduced by binary `-`, keeping every
    /// sub-expression a syntactically unambiguous literal).
    fn leaf(&mut self) -> u64 {
        match self.below(3) {
            0 => self.below(100),
            1 => self.below(1_000_000),
            _ => self.next() / 2, // large, so products promote to BigInt
        }
    }
}

/// A recursive integer expression over `+ - *` and division by a guaranteed-nonzero literal. Total and
/// always parseable; nesting explores deep sign/overflow/promotion paths a flat template never reaches.
fn gen_expr(rng: &mut Rng, depth: u32) -> String {
    if depth == 0 || rng.below(3) == 0 {
        return rng.leaf().to_string();
    }
    let l = gen_expr(rng, depth - 1);
    let r = gen_expr(rng, depth - 1);
    match rng.below(5) {
        0 => format!("({l} + {r})"),
        1 => format!("({l} - {r})"),
        2 => format!("({l} * {r})"),
        3 => format!("({l} / {})", rng.below(999) + 1), // nonzero literal divisor
        _ => format!("({l} % {})", rng.below(999) + 1), // nonzero literal modulus
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

/// Synthesize one surface-valid, terminating, error-free LOGOS program. `category` selects a template family
/// so the corpus is broad by construction (each category is exercised ~`SEEDS / CATEGORIES` times).
fn gen_program(category: u64, rng: &mut Rng) -> String {
    match category {
        // 0 — deep nested integer expression (the BigInt/overflow/sign workhorse)
        0 => format!("## Main\n    Show {}.\n", gen_expr(rng, 4)),

        // 1 — three-operand let-chain with random ops and full-range (signed) operands
        1 => {
            let (a, b, c) = (rng.operand(), rng.operand(), rng.operand());
            let ops = ["+", "-", "*"];
            let o1 = ops[rng.below(3) as usize];
            let o2 = ops[rng.below(3) as usize];
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Let c be {c}.\n    Show (a {o1} b) {o2} c.\n")
        }

        // 2 — signed division and modulo (the shared-divider sign path), nonzero divisor
        2 => {
            let a = rng.operand();
            let b = ((rng.below(999) as i64) + 1) * if rng.below(2) == 0 { 1 } else { -1 };
            let optxt = if rng.below(2) == 0 { "/" } else { "%" };
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Show a {optxt} b.\n")
        }

        // 3 — comparison through an if/else (the 0/1 branch datapath)
        3 => {
            let (a, b) = (rng.operand(), rng.operand());
            let cmp = CMPS[rng.below(CMPS.len() as u64) as usize];
            let (x, y) = (rng.operand(), rng.operand());
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    If a {cmp} b:\n        Show {x}.\n    Otherwise:\n        Show {y}.\n")
        }

        // 4 — while-loop accumulator (mutable Set, back-edge, bounded)
        4 => {
            let n = rng.below(50) + 1;
            let c = (rng.below(20) as i64) - 10;
            format!("## Main\n    Let mutable sum be 0.\n    Let mutable i be 1.\n    While i is at most {n}:\n        Set sum to sum + i * {c}.\n        Set i to i + 1.\n    Show sum.\n")
        }

        // 5 — counted range-sum (iterator cursor/exit)
        5 => {
            let lo = (rng.below(20) as i64) - 10;
            let hi = lo + (rng.below(40) as i64); // lo ≤ hi
            format!("## Main\n    Let mutable s be 0.\n    Repeat for x from {lo} to {hi}:\n        Set s to s + x.\n    Show s.\n")
        }

        // 6 — build a list in a loop then index it (heap header + 1-based index)
        6 => {
            let n = rng.below(8) + 1;
            let k = rng.below(n) + 1; // 1-based, in range
            format!("## Main\n    Let mutable xs be [].\n    Let mutable i be 1.\n    While i is at most {n}:\n        Push i * i to xs.\n        Set i to i + 1.\n    Show item {k} of xs.\n")
        }

        // 7 — literal list + foreach-sum
        7 => {
            let n = rng.below(6) + 1;
            let items: Vec<String> = (0..n)
                .map(|_| ((rng.below(200) as i64) - 100).to_string())
                .collect();
            format!("## Main\n    Let xs be [{}].\n    Let mutable s be 0.\n    Repeat for x in xs:\n        Set s to s + x.\n    Show s.\n", items.join(", "))
        }

        // 8 — user function with two args (call/return frame discipline)
        8 => {
            let (x, y) = (rng.operand(), rng.operand());
            let ops = ["+", "-", "*"];
            let o = ops[rng.below(3) as usize];
            format!("## To f (a: Int, b: Int) -> Int:\n    Return a {o} b.\n## Main\n    Show f({x}, {y}).\n")
        }

        // 9 — recursion (sumdown / fib), depth bounded so the model finishes fast
        9 => {
            let n = rng.below(19); // fib(18) heaviest, still instant on the model
            if rng.below(2) == 0 {
                format!("## To sumdown (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Let rest be sumdown(n - 1).\n    Return n + rest.\n## Main\n    Show sumdown({n}).\n")
            } else {
                format!("## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib({n}).\n")
            }
        }

        // 10 — float datapath (FPU add/sub/mul; both chip and VM are f64 so the rendered decimal must agree)
        10 => {
            let f1 = format!("{}.{}", rng.below(1000), rng.below(100));
            let f2 = format!("{}.{}", rng.below(1000), rng.below(100) + 1);
            let ops = ["+", "-", "*"];
            let o = ops[rng.below(3) as usize];
            format!("## Main\n    Show {f1} {o} {f2}.\n")
        }

        // 11 — Word32 ring arithmetic + unsigned show
        11 => {
            let a = rng.next() as u32;
            let b = rng.next() as u32;
            match rng.below(3) {
                0 => format!("## Main\n    Let a be word32({a}).\n    Let b be word32({b}).\n    Show intOfWord32(word_and(a, b)).\n"),
                1 => format!("## Main\n    Let a be word32({a}).\n    Let b be word32({b}).\n    Show intOfWord32(word_or(a, b)).\n"),
                _ => {
                    let k = rng.below(31) + 1;
                    format!("## Main\n    Show intOfWord32(rotr(word32({a}), {k})).\n")
                }
            }
        }

        // 12 — Text concat and interpolation (the text plane)
        12 => {
            let letters = b"abcdefghijklmnopqrstuvwxyz";
            let mk = |rng: &mut Rng| -> String {
                let len = rng.below(5) + 1;
                (0..len)
                    .map(|_| letters[rng.below(26) as usize] as char)
                    .collect()
            };
            if rng.below(2) == 0 {
                let (s1, s2) = (mk(rng), mk(rng));
                format!("## Main\n    Let s be \"{s1}\" + \"{s2}\".\n    Show s.\n")
            } else {
                let s = mk(rng);
                let n = rng.operand();
                format!("## Main\n    Show \"{s} \" + {n}.\n")
            }
        }

        // 13 — two-field struct (record): construct + read a random field (positional slot layout)
        13 => {
            let x = (rng.below(1_000_000)) as i64;
            let y = (rng.below(1_000_000)) as i64;
            let field = if rng.below(2) == 0 { "x" } else { "y" };
            format!("## Definition\nA Point has:\n    x: Int.\n    y: Int.\n## Main\n    Let p be a new Point with x {x} and y {y}.\n    Show p's {field}.\n")
        }

        // 14 — three-field struct: read a random field (off-by-one slot bug diverges)
        14 => {
            let a = (rng.below(1_000_000)) as i64;
            let b = (rng.below(1_000_000)) as i64;
            let c = (rng.below(1_000_000)) as i64;
            let field = ["a", "b", "c"][rng.below(3) as usize];
            format!("## Definition\nA Vec3 has:\n    a: Int.\n    b: Int.\n    c: Int.\n## Main\n    Let v be a new Vec3 with a {a} and b {b} and c {c}.\n    Show v's {field}.\n")
        }

        // 15 — inductive (enum) + Inspect/match: tag test + positional payload bind on both arms
        15 => {
            let hdr = "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n";
            let a = (rng.below(100_000)) as i64;
            let b = (rng.below(100_000)) as i64;
            if rng.below(2) == 0 {
                format!("{hdr}## Main\nLet s be a new Circle with radius {a}.\nInspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show w.\n")
            } else {
                format!("{hdr}## Main\nLet s be a new Rectangle with width {a} and height {b}.\nInspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show h.\n")
            }
        }

        // 16 — Set of Int: repeated Adds over a small range so duplicates are likely → the dedup that
        // determines `length` is genuinely exercised (chip must match the VM's set membership)
        16 => {
            let n = rng.below(6) + 2;
            let mut adds = String::new();
            for _ in 0..n {
                let v = (rng.below(10)) as i64;
                adds.push_str(&format!("    Add {v} to s.\n"));
            }
            format!("## Main\n    Let s be a new Set of Int.\n{adds}    Show length of s.\n")
        }

        // 17 — tuple: construct a 3-tuple, read a random 1-based position
        17 => {
            let (a, b, c) = (rng.operand(), rng.operand(), rng.operand());
            let k = rng.below(3) + 1;
            format!("## Main\n    Let t be ({a}, {b}, {c}).\n    Show item {k} of t.\n")
        }

        // 18 — value semantics / copy-on-write: mutate the original after `copy of`, the copy stays intact
        18 => {
            let (a, b) = ((rng.below(1000)) as i64, (rng.below(1000)) as i64);
            let newv = (rng.below(1000)) as i64;
            format!("## Main\n    Let a be [{a}, {b}].\n    Let c be copy of a.\n    Set item 1 of a to {newv}.\n    Show item 1 of c.\n")
        }

        // 19 — list concatenation (`a followed by b` → a fresh SeqConcat), read a random element of the join
        19 => {
            let (a1, a2, b1, b2) = (rng.operand(), rng.operand(), rng.operand(), rng.operand());
            let k = rng.below(4) + 1;
            format!("## Main\n    Let a be [{a1}, {a2}].\n    Let b be [{b1}, {b2}].\n    Let c be a followed by b.\n    Show item {k} of c.\n")
        }

        // 20 — a real algorithm end-to-end (Euclid's gcd): while-loop + mod + swap over random inputs
        20 => {
            let a = rng.below(100_000) + 1;
            let b = rng.below(100_000) + 1;
            format!("## To gcd (a: Int, b: Int) -> Int:\n    Let mutable x be a.\n    Let mutable y be b.\n    While y is greater than 0:\n        Let t be y.\n        Set y to x % y.\n        Set x to t.\n    Return x.\n## Main\n    Show gcd({a}, {b}).\n")
        }

        // 21 — integer power (the sequential-multiply Pow unit, op 18); exponent bounded, base up to BigInt
        21 => {
            let a = rng.below(20);
            let b = rng.below(11); // 0..10 multiplies; a**b can exceed i64 → the chip must promote like the VM
            format!("## Main\n    Show {a} ** {b}.\n")
        }

        // 22 — CRDT G-counter bump (a `Counter` field, Increase/CrdtBump, op 80-84)
        22 => {
            let a = (rng.below(10_000)) as i64;
            let b = (rng.below(10_000)) as i64;
            format!("## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable t be a new Tally.\n    Increase t's n by {a}.\n    Increase t's n by {b}.\n    Show t's n.\n")
        }

        // 23 — non-capturing closure passed to a higher-order function and invoked (MakeClosure/CallValue)
        23 => {
            let k = (rng.below(1000)) as i64;
            let x = (rng.below(1000)) as i64;
            format!("## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n + {k}, {x}).\n")
        }

        // 24 — float-seq literal: a `[f, f, f]` built through the float PAYLOAD plane, then indexed
        24 => {
            let f =
                |rng: &mut Rng| format!("{}.{}", (rng.below(2000) as i64) - 1000, rng.below(1000));
            let (a, b, c) = (f(rng), f(rng), f(rng));
            let k = rng.below(3) + 1;
            format!("## Main\n    Let xs be [{a}, {b}, {c}].\n    Show item {k} of xs.\n")
        }

        // 25 — deterministic concurrency: a Pipe + Launch a task that Sends, then Receive (the CONCURRENT
        // oracle — see `vm_oracle`). Exercises Spawn/Chan* (op 92-104) and the windowed spawn arg-copy.
        25 => {
            let v = (rng.below(1_000_000)) as i64;
            format!("## To produce (ch: Int):\n    Send {v} into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    Receive first from jobs.\n    Show first.\n")
        }

        // 26 — integer builtins on the op-38 coprocessor: abs / min / max / count_ones (BigInt-exact)
        26 => {
            let (a, b) = (rng.operand(), rng.operand());
            match rng.below(4) {
                0 => format!("## Main\n    Show abs({a}).\n"),
                1 => format!("## Main\n    Show min({a}, {b}).\n"),
                2 => format!("## Main\n    Show max({a}, {b}).\n"),
                _ => format!(
                    "## Main\n    Show count_ones({}).\n",
                    rng.below(1_000_000_000)
                ),
            }
        }

        // 27 — float→int builtins (floor / ceil / round) and a float min, over the float plane
        27 => {
            let ff =
                |rng: &mut Rng| format!("{}.{}", (rng.below(200) as i64) - 100, rng.below(1000));
            match rng.below(4) {
                0 => format!("## Main\n    Let x be {}.\n    Show floor(x).\n", ff(rng)),
                1 => format!("## Main\n    Let x be {}.\n    Show ceil(x).\n", ff(rng)),
                2 => format!("## Main\n    Let x be {}.\n    Show round(x).\n", ff(rng)),
                _ => format!(
                    "## Main\n    Let a be {}.\n    Let b be {}.\n    Show min(a, b).\n",
                    ff(rng),
                    ff(rng)
                ),
            }
        }

        // 28 — offline-loopback networking: Listen + PeerAgent + Send + Await (NetListen/Send/Await/MakePeer)
        28 => {
            let v = (rng.below(1_000_000)) as i64;
            format!("## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send {v} to peer.\n    Await response from peer into got.\n    Show got.\n")
        }

        // 29 — string-key map: Set/Get by Text key + length (NewEmptyMap + map SetIndex/Index)
        29 => {
            let (a, b) = (rng.operand(), rng.operand());
            let keys = ["a", "b", "c", "x", "y"];
            let (k1, k2) = (keys[rng.below(5) as usize], keys[rng.below(5) as usize]);
            format!("## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item \"{k1}\" of m to {a}.\n    Set item \"{k2}\" of m to {b}.\n    Show item \"{k1}\" of m.\n    Show length of m.\n")
        }

        // 30 — tuple destructuring in a for-each (DestructureTuple, op 71)
        30 => {
            let (a, b, c, d) = (rng.operand(), rng.operand(), rng.operand(), rng.operand());
            format!("## Main\n    Let pairs be [({a}, {b}), ({c}, {d})].\n    Let mutable s be 0.\n    Repeat for (p, q) in pairs:\n        Set s to s + p - q.\n    Show s.\n")
        }

        // 31 — CRDT field-wise merge (CrdtMerge): sum-join two counters
        31 => {
            let (a, b) = ((rng.below(10_000)) as i64, (rng.below(10_000)) as i64);
            format!("## A Counter has:\n    a: Int.\n    b: Int.\n## Main\n    Let mutable x be a new Counter with a {a}.\n    Let mutable y be a new Counter with b {b}.\n    Merge y into x.\n    Show x's a.\n    Show x's b.\n")
        }

        // 32 — list Pop (into a binding) or list Contains (Op::ListPop / Op::Contains op 63)
        32 => {
            let n = rng.below(5) + 2;
            let items: Vec<String> = (0..n)
                .map(|_| ((rng.below(100)) as i64).to_string())
                .collect();
            if rng.below(2) == 0 {
                format!(
                    "## Main\n    Let mutable xs be [{}].\n    Pop from xs into y.\n    Show y.\n",
                    items.join(", ")
                )
            } else {
                let needle = (rng.below(120)) as i64;
                format!("## Main\n    Let xs be [{}].\n    If xs contains {needle}:\n        Show 1.\n    Otherwise:\n        Show 0.\n", items.join(", "))
            }
        }

        // 33 — a function reading a Main-level GLOBAL binding (GlobalGet/GlobalSet)
        33 => {
            let (base, x) = (rng.operand(), rng.operand());
            format!("## To addbase (n: Int) -> Int:\n    Return n + base.\n## Main\n    Let base be {base}.\n    Show addbase({x}).\n")
        }

        // 34 — Word32 xor / not / rotl / logical-shr (BitXor / Not / Rotl / Word32Shr coprocessor)
        34 => {
            let (a, b) = (rng.next() as u32, rng.next() as u32);
            match rng.below(4) {
                0 => format!("## Main\n    Let a be word32({a}).\n    Let b be word32({b}).\n    Let c be a xor b.\n    Show intOfWord32(c).\n"),
                1 => format!("## Main\n    Show intOfWord32(word_not(word32({a}))).\n"),
                2 => {
                    let k = rng.below(31) + 1;
                    format!("## Main\n    Show intOfWord32(rotl(word32({a}), {k})).\n")
                }
                _ => {
                    let k = rng.below(31) + 1;
                    format!("## Main\n    Show intOfWord32(word32Shr(word32({a}), {k})).\n")
                }
            }
        }

        // 35 — deterministic scalar builtins: sqrt (float plane) / chr (op 233) / parseInt (op 236) /
        //      text_bytes (op 237) over a codepoint of a random UTF-8 width
        35 => match rng.below(4) {
            0 => {
                let f = format!("{}.{}", rng.below(10_000), rng.below(1000));
                format!("## Main\n    Show floor(sqrt({f})).\n")
            }
            1 => {
                let code = rng.below(1000) + 33; // valid, printable-ish codepoints (below the surrogate range)
                format!("## Main\n    Show chr({code}).\n")
            }
            2 => {
                let n = (rng.next() as i64) / 2;
                format!("## Main\n    Show parseInt(\"{n}\").\n")
            }
            _ => {
                let code = match rng.below(4) {
                    0 => rng.below(0x80),
                    1 => 0x80 + rng.below(0x800 - 0x80),
                    2 => 0xE000 + rng.below(0x1_0000 - 0xE000),
                    _ => 0x1_0000 + rng.below(0x11_0000 - 0x1_0000),
                };
                format!("## Main\n    Let b be text_bytes(\"x\" + chr({code})).\n    Show length of b.\n    Repeat for v in b:\n        Show v.\n")
            }
        },

        // 36 — Sleep: a deterministic scheduler no-op between Shows (op 88; the CONCURRENT oracle)
        36 => {
            let n = rng.below(200);
            format!("## Main\n    Show 1.\n    Sleep {n}.\n    Show 2.\n")
        }

        // 37 — non-blocking channel Try-send / Try-receive (ChanTrySend/ChanTryRecv; CONCURRENT oracle)
        37 => {
            let v = (rng.below(1_000_000)) as i64;
            format!("## Main\n    Let ch be a Pipe of Int.\n    Try to send {v} into ch.\n    Try to receive x from ch.\n    Show x.\n")
        }

        // 38 — handle-bound Launch + Stop (SpawnHandle/TaskAbort; CONCURRENT oracle)
        38 => {
            let v = (rng.below(1_000_000)) as i64;
            format!("## To worker (ch: Int):\n    Send {v} into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Let h be Launch a task to worker with jobs.\n    Receive first from jobs.\n    Stop h.\n    Show first.\n")
        }

        // 39 — Select (`Await the first of` a Receive arm + a timeout arm; SelectArmRecv/Timeout/Wait)
        39 => {
            let v = (rng.below(1_000_000)) as i64;
            format!("## To producer (ch: Int):\n    Send {v} into ch.\n## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to producer with jobs.\n    Await the first of:\n        Receive x from jobs:\n            Show x.\n        After 10 seconds:\n            Show 0.\n")
        }

        // 40 — networking: Connect + Stream, and the Shared-CRDT Sync (NetConnect/NetStream/NetSync; NET oracle)
        40 => {
            if rng.below(2) == 0 {
                "## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Listen on \"me\".\n    Let items be [1, 2, 3].\n    Stream items to \"sink\".\n    Show 7.\n".to_string()
            } else {
                let (a, b) = ((rng.below(1000)) as i64, (rng.below(1000)) as i64);
                format!("## A Counter is Shared and has:\n    A total: Int.\n\n## Main\n    Connect to \"ws://127.0.0.1:7777\".\n    Listen on \"me\".\n    Let mutable c be a new Counter with total {a}.\n    Increase c's total by {b}.\n    Sync c on \"room\".\n    Show c's total.\n")
            }
        }

        // 41 — short-circuit boolean connectives in a condition: `or` (JumpIfTrue) / `and` (JumpIfFalse).
        // (The Shared-CRDT `Append`/`Resolve` surface forms are DELIBERATELY absent: they compile to a
        // host-CRDT `CallBuiltin` the encoder rejects as a Boundary — NOT to `Op::CrdtAppend`/`CrdtResolve`,
        // which are reachable only via the hand-assembled path, like `CrdtJoin` — so there is no single-node
        // source form to fuzz them with. The catalog lock covers those opcodes.)
        _ => {
            let (a, b, c, d) = (rng.operand(), rng.operand(), rng.operand(), rng.operand());
            let conn = if rng.below(2) == 0 { "or" } else { "and" };
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Let c be {c}.\n    Let d be {d}.\n    If a is greater than b {conn} c is less than d:\n        Show 1.\n    Otherwise:\n        Show 0.\n")
        }
    }
}

/// Multi-word instruction widths (the immediate-carrying ops); every other opcode is a single word. This is
/// the SAME table `fifty_million_dollar_test::op_width` uses, matching the soft-core model's PC advancement,
/// so the opcode walk below never misreads an embedded immediate's high byte as an opcode.
fn op_width(oc: u8) -> usize {
    match oc {
        37 => 2,
        200 => 2,
        201 => 3,
        216 => 2,
        222 => 3,
        _ => 1,
    }
}

/// Walk one compiled chip image, collecting the opcode byte of every REAL instruction (advancing the pc by
/// `op_width`), so the breadth ratchet counts genuine opcodes rather than data high-bytes.
fn opcodes_of_image(words: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pc = 0usize;
    while pc < words.len() {
        let oc = (words[pc] >> 24) as u8;
        out.push(oc);
        pc += op_width(oc);
    }
    out
}

const CATEGORIES: u64 = 42;

/// The differential, returning an outcome so the fuzz loop can tally coverage instead of only panicking. A
/// real chip-vs-VM divergence panics here (loud, with the reproducing source) — that is the $50M find.
enum Outcome {
    /// lowered and chip == VM; the opcodes the image used (for the breadth ratchet).
    Lowered(Vec<u8>),
    /// the encoder honestly declined this program (boundary op) — skipped.
    Boundary,
}

/// The VM oracle for a category. Deterministic-concurrency programs (`Pipe`/`Launch`/`Receive`) are DECLINED
/// by the plain `vm_outcome` (a scheduler op "requires the scheduler driver"); the chip's synchronous
/// Spawn/Chan* semantics mirror `vm_outcome_concurrent` (the cooperative VM, seed 0) — the SAME oracle the AOT
/// concurrency lock and `universal_completeness_lock` compare against — so that family uses it instead.
fn vm_oracle(category: u64, src: &str) -> RunOutcome {
    match category {
        25 | 36 | 37 | 38 | 39 => vm_outcome_concurrent(src), // scheduler ops: chan/spawn/sleep/select
        28 | 40 => vm_outcome_net(src), // offline networking — plain `vm_outcome` errors on a bare Connect
        _ => vm_outcome(src),
    }
}

fn differential(name: &str, category: u64, src: &str) -> Outcome {
    let words = match compile_to_full_words(src) {
        Ok(w) => w,
        Err(_) => return Outcome::Boundary, // honest boundary reject — not a defect
    };
    let vm = vm_oracle(category, src);
    // A generated program is total by construction; a VM error means the GENERATOR emitted something
    // unexpected. Surface it loudly rather than silently skipping — the generator must stay clean.
    assert!(
        vm.error.is_none(),
        "{name}: the fuzzer produced a program the VM errored on ({:?}) — generator bug\n  src = {src:?}",
        vm.error
    );
    let shows = run_soft_core_typed(&words, 50_000_000);
    let got = shows
        .iter()
        .map(ShowVal::fmt)
        .collect::<Vec<_>>()
        .join("\n");
    // The VM prints "<value>\n" per Show; the chip `got` joins the SAME rendered values with "\n" and carries
    // no trailing terminator, so `vm.output == got + "\n"` exactly. Strip that ONE trailing newline — NOT
    // `.trim()`, which would also strip a value that legitimately IS whitespace (`chr(160)` = U+00A0 nbsp,
    // `chr(9)` = tab, `chr(32)` = space): trimming only the VM side turned an EXACT agreement into a false
    // $50M divergence. `strip_suffix` keeps the comparison byte-exact on both sides (verified: `vm.raw ==
    // got + "\n"` for every whitespace codepoint), so a real trailing/leading-whitespace bug still diverges.
    let want = vm.output.strip_suffix('\n').unwrap_or(vm.output.as_str());
    assert_eq!(
        got.as_str(),
        want,
        "{name}: SOFT-CORE CHIP != VM — a real silent-miscompile ($50M class)\n  src = {src:?}"
    );
    Outcome::Lowered(opcodes_of_image(&words))
}

/// The main fuzz: many deterministic seeds, every template family, chip ≡ VM on every program that lowers,
/// and a broad opcode footprint. A divergence panics inside `differential`; otherwise the tallies below lock
/// the run against vacuity (enough lowered programs, enough distinct opcodes).
#[test]
fn soft_core_chip_equals_vm_over_a_generated_corpus() {
    const SEEDS: u64 = CATEGORIES * 100; // ~100 programs per template family
    let per_cat = (SEEDS / CATEGORIES) as u64;
    let mut lowered = 0u64;
    let mut boundary = 0u64;
    let mut lowered_per_category = vec![0u64; CATEGORIES as usize];
    let mut opcode_union: BTreeSet<u8> = BTreeSet::new();

    for seed in 0..SEEDS {
        let category = seed % CATEGORIES;
        let mut rng = Rng::new(seed);
        let src = gen_program(category, &mut rng);
        match differential("fuzz", category, &src) {
            Outcome::Lowered(ops) => {
                lowered += 1;
                lowered_per_category[category as usize] += 1;
                opcode_union.extend(ops); // diagnostic only — see the eprintln + the note on the breadth guard
            }
            Outcome::Boundary => boundary += 1,
        }
    }

    // The `distinct opcodes` figure is a coarse LOWER-BOUND diagnostic, not an assertion: instruction widths
    // are context-dependent (float/LoadConst/text lower to a variable number of words), so a raw-word walk
    // cannot perfectly separate opcodes from immediates. The breadth GUARANTEE below is the artifact-free
    // per-category coverage instead.
    eprintln!(
        "soft-core fuzz: {lowered}/{SEEDS} lowered, {boundary} boundary-skipped; per-category = {:?}; \
         ~{} distinct opcodes (approx): {:?}",
        lowered_per_category,
        opcode_union.len(),
        opcode_union
    );

    // Non-vacuity 1: the overwhelming majority of generated programs must actually LOWER and be differenced —
    // otherwise the templates drifted off the lowered surface and the fuzz proves little.
    assert!(
        lowered >= SEEDS * 9 / 10,
        "only {lowered}/{SEEDS} generated programs lowered ({boundary} boundary-skipped) — the templates \
         drifted off the lowered ISA surface; the differential is nearly vacuous"
    );

    // Non-vacuity 2 (the BREADTH RATCHET / pit-of-success lock, artifact-free): EVERY one of the 13 datapath
    // families — deep integer expressions, signed div/mod, comparison branches, while/range loops, list
    // build+index, foreach, user functions, recursion, the float lane, the Word32 ring, and the text plane —
    // must genuinely lower a healthy share of its programs. This proves broad ISA exercise directly, with no
    // dependence on the approximate opcode walk. A family that stops lowering (a lowering regression narrowed
    // to one datapath) drops its count and goes RED here. Never weaken a family's floor to make a shrinking
    // corpus pass — fix the lowering.
    for (cat, &count) in lowered_per_category.iter().enumerate() {
        assert!(
            count >= per_cat * 9 / 10,
            "template family {cat} lowered only {count}/{per_cat} programs — that datapath fell off the \
             lowered ISA surface; the fuzz's breadth collapsed there"
        );
    }
}

#[test]
#[ignore]
fn dump_category_opcodes() {
    for category in 0..CATEGORIES {
        let mut seen: BTreeSet<u8> = BTreeSet::new();
        for seed in 0..40u64 {
            let src = gen_program(category, &mut Rng::new(seed * CATEGORIES + category));
            if let Ok(words) = compile_to_full_words(&src) {
                seen.extend(opcodes_of_image(&words));
            }
        }
        eprintln!("category {category}: {:?}", seen);
    }
}

/// The generator is a pure function of its seed: a failure reproduces exactly. (Guards against an accidental
/// dependence on `Math.random`/time/global state that would make a fuzz failure un-reproducible.)
#[test]
fn the_generator_is_deterministic() {
    for seed in [0u64, 1, 7, 42, 100, 999, 1299] {
        for category in 0..CATEGORIES {
            let a = gen_program(category, &mut Rng::new(seed));
            let b = gen_program(category, &mut Rng::new(seed));
            assert_eq!(
                a, b,
                "gen_program(seed={seed}, category={category}) is non-deterministic"
            );
        }
    }
}
