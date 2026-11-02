//! wprog_all_benchmarks_lane — THE WHOLE BENCHMARK SUITE ON THE WHOLE-PROGRAM ENGINE. A census over all 35
//! benchmark programs (`benchmarks/programs/*/main.lg`): each is captured as a faithful function-form of its
//! core algorithm (parameterized by the size `n`, returning the primary numeric result), and either RUNS on the
//! sequenced whole-program engine (`eval_wprog_config` — register file + block walk + heap state plane + frame
//! stack + FLOAT LANE + float-in-Seq + reserved-capacity collections + map + text + itoa + WORD32, all proven
//! ≡ VM) bit-exactly vs the VM across many inputs. **ALL 35 RUN — the gap list is EMPTY, and every form is now
//! FAITHFUL to the real `main.lg` algorithm** (the 5-body nbody, the 32768-bucket analytics, the RNG+Map
//! two_sum, the mulAv/mulAtv recursion in spectral_norm, and the FULL SHA-256 compression). Every benchmark's
//! core — integer loops, recursion, collections (incl. multiple concurrently-growing + float/word32 elements),
//! floats (scalar + in-Seq + through recursion), maps, string building with int→string, and 32-bit-wrapping
//! Word32 crypto — runs on the sequenced whole-program engine ≡ the VM.
//! Lanes CLOSED this session:
//!   • `float-in-Seq` — a float pushed/stored into a Seq marks it float-typed (nbody, spectral_norm)
//!   • `Seq-in-recursion` — collection ops in `program_calls_fabric`, global heap across frames (quicksort, …)
//!   • `multi-grow-Seq` — an empty Seq reserves its own capacity block (`SEQ_RESERVE`) so several grow at once
//!                        (matrix_mult, knapsack, fannkuch, mergesort). A self-guarding BOUND: overflow → REFUSED.
//!   • `map` — a dictionary is a DIRECT-ADDRESS table (`heap[ptr+key]`); `contains k` = `heap[ptr+key] != 0`
//!   • `text` + `itoa` — string literals reserve growth capacity; `text + s` copies bytes; `text + <int>` writes
//!                       decimal digits in static cells; a 1-char literal in a compare reduces to its byte
//!   • `float-in-recursion` — the FULL float lane in `program_calls_fabric`, float-ness seeded from `param_types`
//!                            and propagated across calls via `return_type` (spectral_norm)
//!   • `Word32` — a ℤ/2³² wrapping-integer lane: word32/rotr/word32Shr/word_and/word_not/wrapping-arith +
//!                Seq-of-Word32, seeded/propagated like the float lane (the full SHA-256)
//! (plus `Bool` constants → sieve, bitwise-in-recursion → nqueens, and the `mark_int_reg` register-reuse fix.)
//!
//! This is a RATCHET (the `wprog_completeness_lock` pattern): the SUPPORTED floor only grows and the gap ceiling
//! only shrinks, and NOTHING is unclassified — so we can never be silently missing a benchmark. A SUPPORTED
//! benchmark that regresses turns the lock RED; a KNOWN_GAP that starts working turns it RED demanding promotion.
//! The full table (every benchmark, its lane, RUNS/REFUSED) prints every run.
//!
//! Run: `cargo test -p logicaffeine-tests --test wprog_all_benchmarks_lane`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;
#[path = "common/golden.rs"]
mod golden;

use std::panic::{catch_unwind, AssertUnwindSafe};

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_testbench, emit_wprog_engine_soc_verilog_backend, eval_wprog_config,
    eval_wprog_config_probed, eval_wprog_config_run, program_calls_fabric, serialize_program,
    wprog_config_fits_hw_engine, wprog_config_fits_stream_engine, wprog_config_image_unchecked,
    MemBackend, WProgConfig, WPE_HW_DEPTH, WPE_HW_HEAP, WPE_STREAM_DEPTH, WPE_STREAM_HEAP,
};
use logicaffeine_compile::compile::vm_outcome;

/// Run the VM on `f(args)` and return its first integer token, or None on error / non-integer output.
fn vm_f(src: &str, args: &[i64]) -> Option<i64> {
    let al = args
        .iter()
        .map(|a| {
            if *a < 0 {
                format!("(0 - {})", -a)
            } else {
                a.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let prog = format!("{src}## Main\n    Show f({al}).\n");
    let out = vm_outcome(&prog);
    if out.error.is_some() {
        return None;
    }
    out.output.split_whitespace().next()?.parse::<i64>().ok()
}

/// Does `f` run on the whole-program engine ≡ the VM for every input? Loops/collections/floats lower via
/// `program_cfg_fabric` (single self-contained function); recursion + multi-function via `program_calls_fabric`.
fn runs_on_engine(src: &str, args_sets: &[&[i64]]) -> bool {
    let all = |lower: &dyn Fn(&[i64]) -> i64| -> bool {
        catch_unwind(AssertUnwindSafe(|| {
            args_sets
                .iter()
                .all(|a| matches!(vm_f(src, a), Some(vm) if lower(a) == vm))
        }))
        .unwrap_or(false)
    };
    all(&|a| eval_wprog_config(&serialize_program(&program_cfg_fabric(src, "f")), a))
        || all(&|a| eval_wprog_config(&program_calls_fabric(src, "f"), a))
}

/// A benchmark's classification: does its lane run on the whole-program engine today?
#[derive(Clone, Copy, PartialEq)]
enum Status {
    /// Runs ≡ VM on the engine — a regression turns the lock RED.
    Supported,
    /// A documented gap: the named lane isn't wired yet. If it starts working, the lock RED-demands promotion.
    KnownGap(&'static str),
}
use Status::{KnownGap, Supported};

struct Bench {
    name: &'static str,
    status: Status,
    src: &'static str,
    args: &'static [&'static [i64]],
}

// Every one of the 35 benchmark programs, as a faithful function-form of its core algorithm. Each RUNS ≡ VM on
// the whole-program engine (SUPPORTED) or names the lane it still needs (KnownGap). See the module doc.
const BENCHMARKS: &[Bench] = &[
    // ── Integer loops (block-walk) ──────────────────────────────────────────────────────────────────────────
    Bench {
        name: "loop_sum",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let mutable sum be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set sum to (sum + i) % 1000000007.\n        Set i to i + 1.\n    Return sum.\n",
        args: &[&[0], &[1], &[10], &[1000], &[5000]],
    },
    Bench {
        name: "fib_iterative",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let mutable a be 0.\n    Let mutable b be 1.\n    Let mutable i be 0.\n    While i is less than n:\n        Let temp be b.\n        Set b to (a + b) % 1000000007.\n        Set a to temp.\n        Set i to i + 1.\n    Return a.\n",
        args: &[&[0], &[1], &[10], &[50], &[90]],
    },
    Bench {
        name: "collatz",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Let mutable k be i.\n        While k is not 1:\n            If k % 2 equals 0:\n                Set k to k / 2.\n            Otherwise:\n                Set k to 3 * k + 1.\n            Set total to total + 1.\n        Set i to i + 1.\n    Return total.\n",
        args: &[&[1], &[7], &[27], &[100]],
    },
    Bench {
        name: "primes",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let mutable count be 0.\n    Let mutable i be 2.\n    While i is at most n:\n        Let mutable isPrime be 1.\n        Let mutable d be 2.\n        While d * d is at most i:\n            If i % d equals 0:\n                Set isPrime to 0.\n                Break.\n            Set d to d + 1.\n        If isPrime equals 1:\n            Set count to count + 1.\n        Set i to i + 1.\n    Return count.\n",
        args: &[&[2], &[10], &[100], &[1000]],
    },
    Bench {
        name: "vm",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let regs be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than 8:\n        Push i + 1 to regs.\n        Set i to i + 1.\n    Let mutable seed be 42.\n    Let mutable k be 0.\n    While k is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Let instr be (seed / 256) % 8388608.\n        Let op be instr % 8.\n        Let dst be (instr / 8) % 8.\n        Let a be (instr / 64) % 8.\n        Let imm be (instr / 512) % 251.\n        If op equals 0:\n            Set item (dst + 1) of regs to imm.\n        If op equals 1:\n            Set item (dst + 1) of regs to (item (dst + 1) of regs + item (a + 1) of regs) % 1000000007.\n        If op equals 6:\n            Set item (dst + 1) of regs to item (a + 1) of regs.\n        If op equals 7:\n            Set item (dst + 1) of regs to (item (dst + 1) of regs + item (a + 1) of regs + imm) % 1000000007.\n        Set k to k + 1.\n    Let mutable checksum be 0.\n    Set i to 0.\n    While i is less than 8:\n        Set checksum to (checksum + (i + 1) * (item (i + 1) of regs)) % 1000000007.\n        Set i to i + 1.\n    Return checksum.\n",
        args: &[&[0], &[10], &[100], &[500]],
    },
    // ── Recursion (frame stack) ─────────────────────────────────────────────────────────────────────────────
    Bench {
        name: "fib",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return f(n - 1) + f(n - 2).\n",
        args: &[&[0], &[1], &[10], &[20], &[27]],
    },
    Bench {
        name: "gcd",
        status: Supported,
        src: "## To gcd (a: Int, b: Int) -> Int:\n    Let mutable x be a.\n    Let mutable y be b.\n    While y is greater than 0:\n        Let temp be y.\n        Set y to x % y.\n        Set x to temp.\n    Return x.\n## To f (n: Int) -> Int:\n    Let mutable sum be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Let mutable j be i.\n        While j is at most n:\n            Set sum to sum + gcd(i, j).\n            Set j to j + 1.\n        Set i to i + 1.\n    Return sum.\n",
        args: &[&[1], &[5], &[20], &[60]],
    },
    Bench {
        name: "ackermann",
        status: Supported,
        src: "## To ack (m: Int) and (n: Int) -> Int:\n    If m equals 0:\n        Return n + 1.\n    If n equals 0:\n        Return ack(m - 1, 1).\n    Return ack(m - 1, ack(m, n - 1)).\n## To f (n: Int) -> Int:\n    Return ack(2, n).\n",
        args: &[&[0], &[1], &[3], &[5]],
    },
    Bench {
        name: "binary_trees",
        status: Supported,
        src: "## To makeCheck (depth: Int) -> Int:\n    If depth equals 0:\n        Return 1.\n    Return 1 + makeCheck(depth - 1) + makeCheck(depth - 1).\n## To f (n: Int) -> Int:\n    Return makeCheck(n).\n",
        args: &[&[0], &[1], &[5], &[10]],
    },
    Bench {
        name: "nqueens",
        status: Supported,
        src: "## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:\n    If row equals n:\n        Return 1.\n    Let all be (1 shifted left by n) - 1.\n    Let available be all & ~(cols | diag1 | diag2).\n    Let mutable count be 0.\n    While available is not 0:\n        Let bit be available & (0 - available).\n        Set available to available xor bit.\n        Set count to count + solve(row + 1, cols | bit, (diag1 | bit) shifted left by 1, (diag2 | bit) shifted right by 1, n).\n    Return count.\n## To f (n: Int) -> Int:\n    Return solve(0, 0, 0, 0, n).\n",
        args: &[&[1], &[4], &[6], &[8]],
    },
    // ── Seq collections (heap state plane) ──────────────────────────────────────────────────────────────────
    Bench {
        name: "array_fill",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let arr be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than n:\n        Push (i * 7 + 3) % 1000000 to arr.\n        Set i to i + 1.\n    Let mutable sum be 0.\n    Set i to 1.\n    While i is at most n:\n        Set sum to (sum + item i of arr) % 1000000007.\n        Set i to i + 1.\n    Return sum.\n",
        args: &[&[1], &[10], &[100], &[500]],
    },
    Bench {
        name: "array_reverse",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let arr be a new Seq of Int.\n    Let mutable seed be 42.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Push (seed / 65536) % 32768 to arr.\n        Set i to i + 1.\n    Let mutable lo be 1.\n    Let mutable hi be n.\n    While lo is less than hi:\n        Let tmp be item lo of arr.\n        Set item lo of arr to item hi of arr.\n        Set item hi of arr to tmp.\n        Set lo to lo + 1.\n        Set hi to hi - 1.\n    Return item 1 of arr.\n",
        args: &[&[1], &[10], &[100], &[300]],
    },
    Bench {
        name: "prefix_sum",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let arr be a new Seq of Int.\n    Let mutable seed be 42.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Push ((seed / 65536) % 32768) % 1000 to arr.\n        Set i to i + 1.\n    Set i to 2.\n    While i is at most n:\n        Set item i of arr to (item i of arr + item (i - 1) of arr) % 1000000007.\n        Set i to i + 1.\n    Return item n of arr.\n",
        args: &[&[1], &[10], &[100], &[500]],
    },
    Bench {
        name: "bubble_sort",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let arr be a new Seq of Int.\n    Let mutable seed be 42.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 4294967296.\n        Push (seed / 65536) % 32768 to arr.\n        Set i to i + 1.\n    Set i to 0.\n    While i is less than n - 1:\n        Let mutable j be 1.\n        While j is at most n - 1 - i:\n            Let a be item j of arr.\n            Let b be item (j + 1) of arr.\n            If a is greater than b:\n                Set item j of arr to b.\n                Set item (j + 1) of arr to a.\n            Set j to j + 1.\n        Set i to i + 1.\n    Return item 1 of arr.\n",
        args: &[&[1], &[10], &[50], &[120]],
    },
    Bench {
        name: "counting_sort",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let arr be a new Seq of Int.\n    Let mutable seed be 42.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Push (seed / 65536) % 1000 to arr.\n        Set i to i + 1.\n    Let counts be a new Seq of Int.\n    Set i to 0.\n    While i is less than 1000:\n        Push 0 to counts.\n        Set i to i + 1.\n    Set i to 1.\n    While i is at most n:\n        Let v be item i of arr.\n        Set item (v + 1) of counts to (item (v + 1) of counts) + 1.\n        Set i to i + 1.\n    Let sorted be a new Seq of Int.\n    Let mutable v be 0.\n    While v is less than 1000:\n        Let mutable c be 0.\n        While c is less than item (v + 1) of counts:\n            Push v to sorted.\n            Set c to c + 1.\n        Set v to v + 1.\n    Let mutable checksum be 0.\n    Set i to 1.\n    While i is at most n:\n        Set checksum to (checksum + item i of sorted) % 1000000007.\n        Set i to i + 1.\n    Return checksum.\n",
        args: &[&[1], &[10], &[100], &[300]],
    },
    Bench {
        name: "histogram",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let counts be a new Seq of Int.\n    Let mutable idx be 0.\n    While idx is less than 1000:\n        Push 0 to counts.\n        Set idx to idx + 1.\n    Let mutable seed be 42.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Let v be ((seed / 65536) % 32768) % 1000.\n        Set item (v + 1) of counts to (item (v + 1) of counts) + 1.\n        Set i to i + 1.\n    Let mutable maxFreq be 0.\n    Set i to 0.\n    While i is less than 1000:\n        If item (i + 1) of counts is greater than maxFreq:\n            Set maxFreq to item (i + 1) of counts.\n        Set i to i + 1.\n    Return maxFreq.\n",
        args: &[&[1], &[10], &[100], &[500]],
    },
    Bench {
        name: "analytics",
        status: Supported,
        // The REAL 32768-bucket table (a single collection, so it grows past its reserve at the heap frontier).
        src: "## To f (n: Int) -> Int:\n    Let mutable groups be a new Seq of Int.\n    Let mutable gi be 0.\n    While gi is less than 32768:\n        Push 0 to groups.\n        Set gi to gi + 1.\n    Let mutable seed be 42.\n    Let mutable kept be 0.\n    Let mutable r be 0.\n    While r is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Let key be (seed / 65536) % 32768.\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Let value be (seed / 65536) % 32768.\n        If value % 3 is greater than 0:\n            Let contrib be (value * key + 7) % 1000000007.\n            Let g be key % 32768.\n            Set item (g + 1) of groups to (item (g + 1) of groups + contrib) % 1000000007.\n            Set kept to kept + 1.\n        Set r to r + 1.\n    Let mutable checksum be 0.\n    Let mutable g be 0.\n    While g is less than 32768:\n        Set checksum to (checksum + (g + 1) * (item (g + 1) of groups)) % 1000000007.\n        Set g to g + 1.\n    Return checksum.\n",
        args: &[&[1], &[10], &[50], &[100]],
    },
    Bench {
        name: "matrix_mult",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let a be a new Seq of Int.\n    Let b be a new Seq of Int.\n    Let c be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than n:\n        Let mutable j be 0.\n        While j is less than n:\n            Push (i * n + j) % 100 to a.\n            Push (j * n + i) % 100 to b.\n            Push 0 to c.\n            Set j to j + 1.\n        Set i to i + 1.\n    Set i to 0.\n    While i is less than n:\n        Let mutable k be 0.\n        While k is less than n:\n            Let mutable j be 0.\n            While j is less than n:\n                Let idx be i * n + j + 1.\n                Set item idx of c to (item idx of c + item (i * n + k + 1) of a * item (k * n + j + 1) of b) % 1000000007.\n                Set j to j + 1.\n            Set k to k + 1.\n        Set i to i + 1.\n    Let mutable checksum be 0.\n    Set i to 1.\n    While i is at most n * n:\n        Set checksum to (checksum + item i of c) % 1000000007.\n        Set i to i + 1.\n    Return checksum.\n",
        args: &[&[1], &[3], &[8], &[16]],
    },
    Bench {
        name: "sieve",
        status: Supported,
        src: "## To f (limit: Int) -> Int:\n    Let flags be a new Seq of Bool.\n    Let mutable i be 0.\n    While i is at most limit:\n        Push false to flags.\n        Set i to i + 1.\n    Let mutable count be 0.\n    Set i to 2.\n    While i is at most limit:\n        If item (i + 1) of flags equals false:\n            Set count to count + 1.\n            Let mutable j be i * i.\n            While j is at most limit:\n                Set item (j + 1) of flags to true.\n                Set j to j + i.\n        Set i to i + 1.\n    Return count.\n",
        args: &[&[2], &[10], &[100], &[1000]],
    },
    Bench {
        name: "coins",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let coinList be a new Seq of Int.\n    Push 1 to coinList.\n    Push 5 to coinList.\n    Push 10 to coinList.\n    Push 25 to coinList.\n    Push 50 to coinList.\n    Push 100 to coinList.\n    Let dp be a new Seq of Int.\n    Push 1 to dp.\n    Let mutable i be 1.\n    While i is at most n:\n        Push 0 to dp.\n        Set i to i + 1.\n    Let mutable c be 1.\n    While c is at most 6:\n        Let coin be item c of coinList.\n        Let mutable j be coin.\n        While j is at most n:\n            Set item (j + 1) of dp to (item (j + 1) of dp + item (j - coin + 1) of dp) % 1000000007.\n            Set j to j + 1.\n        Set c to c + 1.\n    Return item (n + 1) of dp.\n",
        args: &[&[1], &[10], &[100], &[500]],
    },
    Bench {
        name: "knapsack",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let capacity be n * 5.\n    Let weights be a new Seq of Int.\n    Let vals be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than n:\n        Push (i * 17 + 3) % 50 + 1 to weights.\n        Push (i * 31 + 7) % 100 + 1 to vals.\n        Set i to i + 1.\n    Let cols be capacity + 1.\n    Let mutable prev be a new Seq of Int.\n    Set i to 0.\n    While i is less than cols:\n        Push 0 to prev.\n        Set i to i + 1.\n    Set i to 0.\n    While i is less than n:\n        Let curr be a new Seq of Int.\n        Let wi be item (i + 1) of weights.\n        Let vi be item (i + 1) of vals.\n        Let mutable w be 0.\n        While w is at most capacity:\n            Let mutable best be item (w + 1) of prev.\n            If w is at least wi:\n                Let take be item (w - wi + 1) of prev + vi.\n                If take is greater than best:\n                    Set best to take.\n            Push best to curr.\n            Set w to w + 1.\n        Set prev to curr.\n        Set i to i + 1.\n    Return item (capacity + 1) of prev.\n",
        args: &[&[1], &[5], &[15], &[30]],
    },
    Bench {
        name: "fannkuch",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let perm1 be a new Seq of Int.\n    Let count be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than n:\n        Push i to perm1.\n        Push 0 to count.\n        Set i to i + 1.\n    Let mutable maxFlips be 0.\n    Let mutable checksum be 0.\n    Let mutable permCount be 0.\n    Let mutable r be n.\n    Let mutable done be 0.\n    While done equals 0:\n        While r is greater than 1:\n            Set item r of count to r.\n            Set r to r - 1.\n        Let perm be a new Seq of Int.\n        Set i to 1.\n        While i is at most n:\n            Push item i of perm1 to perm.\n            Set i to i + 1.\n        Let mutable flips be 0.\n        While item 1 of perm is not 0:\n            Let k be item 1 of perm + 1.\n            Let mutable lo be 1.\n            Let mutable hi be k.\n            While lo is less than hi:\n                Let tmp be item lo of perm.\n                Set item lo of perm to item hi of perm.\n                Set item hi of perm to tmp.\n                Set lo to lo + 1.\n                Set hi to hi - 1.\n            Set flips to flips + 1.\n        If flips is greater than maxFlips:\n            Set maxFlips to flips.\n        If permCount % 2 equals 0:\n            Set checksum to checksum + flips.\n        Otherwise:\n            Set checksum to checksum - flips.\n        Set permCount to permCount + 1.\n        Set done to 1.\n        While done equals 1:\n            If r equals n:\n                Set done to 2.\n            Otherwise:\n                Let perm0 be item 1 of perm1.\n                Set i to 1.\n                While i is at most r:\n                    Set item i of perm1 to item (i + 1) of perm1.\n                    Set i to i + 1.\n                Set item (r + 1) of perm1 to perm0.\n                Set item (r + 1) of count to (item (r + 1) of count) - 1.\n                If item (r + 1) of count is greater than 0:\n                    Set done to 0.\n                Otherwise:\n                    Set r to r + 1.\n    Return checksum.\n",
        args: &[&[1], &[3], &[5], &[7]],
    },
    Bench {
        name: "graph_bfs",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let adj be a new Seq of Int.\n    Let adjStarts be a new Seq of Int.\n    Let adjCounts be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than n:\n        Push i * 5 to adjStarts.\n        Push 0 to adjCounts.\n        Push 0 to adj. Push 0 to adj. Push 0 to adj. Push 0 to adj. Push 0 to adj.\n        Set i to i + 1.\n    Let primes be a new Seq of Int.\n    Push 31 to primes. Push 37 to primes. Push 41 to primes. Push 43 to primes. Push 47 to primes.\n    Let offsets be a new Seq of Int.\n    Push 7 to offsets. Push 13 to offsets. Push 17 to offsets. Push 23 to offsets. Push 29 to offsets.\n    Let mutable p be 1.\n    While p is at most 5:\n        Set i to 0.\n        While i is less than n:\n            Let neighbor be (i * item p of primes + item p of offsets) % n.\n            If neighbor is not i:\n                Let start be item (i + 1) of adjStarts.\n                Let cnt be item (i + 1) of adjCounts.\n                Set item (start + cnt + 1) of adj to neighbor.\n                Set item (i + 1) of adjCounts to cnt + 1.\n            Set i to i + 1.\n        Set p to p + 1.\n    Let queue be a new Seq of Int.\n    Let dist be a new Seq of Int.\n    Set i to 0.\n    While i is less than n:\n        Push 0 - 1 to dist.\n        Set i to i + 1.\n    Push 0 to queue.\n    Set item 1 of dist to 0.\n    Let mutable front be 1.\n    While front is at most length of queue:\n        Let v be item front of queue.\n        Let start be item (v + 1) of adjStarts.\n        Let cnt be item (v + 1) of adjCounts.\n        Let mutable e be 0.\n        While e is less than cnt:\n            Let u be item (start + e + 1) of adj.\n            If item (u + 1) of dist equals 0 - 1:\n                Set item (u + 1) of dist to item (v + 1) of dist + 1.\n                Push u to queue.\n            Set e to e + 1.\n        Set front to front + 1.\n    Let mutable reachable be 0.\n    Set i to 0.\n    While i is less than n:\n        If item (i + 1) of dist is at least 0:\n            Set reachable to reachable + 1.\n        Set i to i + 1.\n    Return reachable.\n",
        args: &[&[2], &[5], &[16], &[32]],
    },
    // ── Recursion + a SINGLE Seq threaded in-place (collection ops in program_calls_fabric; the heap is global
    // across frames, so the array built in `f` and mutated through the recursion runs). mergesort is BELOW —
    // it allocates fresh Seqs inside the recursion (multi-grow-Seq).
    Bench {
        name: "quicksort",
        status: Supported,
        src: "## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:\n    If lo is at least hi:\n        Return arr.\n    Let pivot be item hi of arr.\n    Let mutable result be arr.\n    Let mutable i be lo.\n    Let mutable j be lo.\n    While j is less than hi:\n        If item j of result is at most pivot:\n            Let tmp be item i of result.\n            Set item i of result to item j of result.\n            Set item j of result to tmp.\n            Set i to i + 1.\n        Set j to j + 1.\n    Let tmp be item i of result.\n    Set item i of result to item hi of result.\n    Set item hi of result to tmp.\n    Set result to qs(result, lo, i - 1).\n    Set result to qs(result, i + 1, hi).\n    Return result.\n## To f (n: Int) -> Int:\n    Let arr be a new Seq of Int.\n    Let mutable seed be 42.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Push (seed / 65536) % 32768 to arr.\n        Set i to i + 1.\n    Let sorted be qs(arr, 1, n).\n    Let mutable checksum be 0.\n    Set i to 1.\n    While i is at most n:\n        Set checksum to (checksum + item i of sorted) % 1000000007.\n        Set i to i + 1.\n    Return checksum.\n",
        args: &[&[1], &[5], &[20], &[60]],
    },
    Bench {
        name: "mergesort",
        status: Supported,
        src: "## To mergeSort (arr: Seq of Int) -> Seq of Int:\n    Let n be length of arr.\n    If n is less than 2:\n        Return arr.\n    Let mid be n / 2.\n    Let left be a new Seq of Int.\n    Let right be a new Seq of Int.\n    Let mutable i be 1.\n    While i is at most mid:\n        Push item i of arr to left.\n        Set i to i + 1.\n    While i is at most n:\n        Push item i of arr to right.\n        Set i to i + 1.\n    Let sl be mergeSort(left).\n    Let sr be mergeSort(right).\n    Let result be a new Seq of Int.\n    Let mutable li be 1.\n    Let mutable ri be 1.\n    While li is at most length of sl:\n        If ri is greater than length of sr:\n            Push item li of sl to result.\n            Set li to li + 1.\n        Otherwise:\n            If item li of sl is at most item ri of sr:\n                Push item li of sl to result.\n                Set li to li + 1.\n            Otherwise:\n                Push item ri of sr to result.\n                Set ri to ri + 1.\n    While ri is at most length of sr:\n        Push item ri of sr to result.\n        Set ri to ri + 1.\n    Return result.\n## To f (n: Int) -> Int:\n    Let arr be a new Seq of Int.\n    Let mutable seed be 42.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Push (seed / 65536) % 32768 to arr.\n        Set i to i + 1.\n    Let sorted be mergeSort(arr).\n    Let mutable checksum be 0.\n    Set i to 1.\n    While i is at most n:\n        Set checksum to (checksum + item i of sorted) % 1000000007.\n        Set i to i + 1.\n    Return checksum.\n",
        args: &[&[1], &[5], &[20], &[60]],
    },
    Bench {
        name: "heap_sort",
        status: Supported,
        src: "## To siftDown (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:\n    Let mutable result be arr.\n    Let mutable root be start.\n    While 2 * root + 1 is at most end:\n        Let child be 2 * root + 1.\n        Let mutable swapIdx be root.\n        If item (swapIdx + 1) of result is less than item (child + 1) of result:\n            Set swapIdx to child.\n        If child + 1 is at most end:\n            If item (swapIdx + 1) of result is less than item (child + 2) of result:\n                Set swapIdx to child + 1.\n        If swapIdx equals root:\n            Return result.\n        Let tmp be item (root + 1) of result.\n        Set item (root + 1) of result to item (swapIdx + 1) of result.\n        Set item (swapIdx + 1) of result to tmp.\n        Set root to swapIdx.\n    Return result.\n## To f (n: Int) -> Int:\n    Let arr be a new Seq of Int.\n    Let mutable seed be 42.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Push (seed / 65536) % 32768 to arr.\n        Set i to i + 1.\n    Let mutable start be (n - 2) / 2.\n    While start is at least 0:\n        Set arr to siftDown(arr, start, n - 1).\n        Set start to start - 1.\n    Let mutable end be n - 1.\n    While end is greater than 0:\n        Let tmp be item 1 of arr.\n        Set item 1 of arr to item (end + 1) of arr.\n        Set item (end + 1) of arr to tmp.\n        Set end to end - 1.\n        Set arr to siftDown(arr, 0, end).\n    Let mutable checksum be 0.\n    Set i to 1.\n    While i is at most n:\n        Set checksum to (checksum + item i of arr) % 1000000007.\n        Set i to i + 1.\n    Return checksum.\n",
        args: &[&[1], &[5], &[20], &[60]],
    },
    // ── Float scalar (float lane) ───────────────────────────────────────────────────────────────────────────
    Bench {
        name: "pi_leibniz",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let mutable sum be 0.0.\n    Let mutable sign be 1.0.\n    Let mutable k be 0.\n    While k is less than n:\n        Set sum to sum + sign / (2.0 * k + 1.0).\n        Set sign to 0.0 - sign.\n        Set k to k + 1.\n    Return floor(sum * 4.0 * 1000000.0).\n",
        args: &[&[0], &[1], &[10], &[100], &[1000]],
    },
    Bench {
        name: "mandelbrot",
        status: Supported,
        src: "## To f (px: Int) -> Int:\n    Let cr be 0.01 * px - 2.0.\n    Let ci be 0.5.\n    Let mutable zr be 0.0.\n    Let mutable zi be 0.0.\n    Let mutable i be 0.\n    While i is less than 100:\n        Let zr2 be zr * zr - zi * zi + cr.\n        Let zi2 be 2.0 * zr * zi + ci.\n        Set zr to zr2.\n        Set zi to zi2.\n        If zr * zr + zi * zi is greater than 4.0:\n            Return i.\n        Set i to i + 1.\n    Return i.\n",
        args: &[&[0], &[50], &[150], &[250], &[399]],
    },
    // ── float-in-Seq (float VALUES on the heap) ─────────────────────────────────────────────────────────────
    // The REAL 5-body simulation: 7 concurrently-grown `Seq of Float` (positions, velocities, masses), the
    // pairwise-force integrator with `sqrt`, and momentum correction. Pushing/storing a float marks the Seq
    // float-typed (`FLOAT_COLLS`), so `item i of v` reads flow into the float lane; `mark_int_reg` keeps loop
    // counters/indices integer despite reusing float temp slots. Returns the floored final energy · 1e9.
    Bench {
        name: "nbody",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let pi be 3.141592653589793.\n    Let solarMass be 4.0 * pi * pi.\n    Let daysPerYear be 365.24.\n    Let mutable bx be a new Seq of Float.\n    Let mutable posY be a new Seq of Float.\n    Let mutable bz be a new Seq of Float.\n    Let mutable bvx be a new Seq of Float.\n    Let mutable bvy be a new Seq of Float.\n    Let mutable bvz be a new Seq of Float.\n    Let mutable bm be a new Seq of Float.\n    Push 0.0 to bx. Push 0.0 to posY. Push 0.0 to bz.\n    Push 0.0 to bvx. Push 0.0 to bvy. Push 0.0 to bvz.\n    Push solarMass to bm.\n    Push 4.841431442464721 to bx. Push 0.0 - 1.1603200440274284 to posY. Push 0.0 - 0.10362204447112311 to bz.\n    Push 0.001660076642744037 * daysPerYear to bvx. Push 0.007699011184197404 * daysPerYear to bvy. Push 0.0 - 0.0000690460016972063 * daysPerYear to bvz.\n    Push 0.0009547919384243266 * solarMass to bm.\n    Push 8.34336671824458 to bx. Push 4.124798564124305 to posY. Push 0.0 - 0.4035234171143214 to bz.\n    Push 0.0 - 0.002767425107268624 * daysPerYear to bvx. Push 0.004998528012349172 * daysPerYear to bvy. Push 0.00002304172975737639 * daysPerYear to bvz.\n    Push 0.0002858859806661308 * solarMass to bm.\n    Push 12.894369562139131 to bx. Push 0.0 - 15.111151401698631 to posY. Push 0.0 - 0.22330757889265573 to bz.\n    Push 0.002964601375647616 * daysPerYear to bvx. Push 0.0023784717395948095 * daysPerYear to bvy. Push 0.0 - 0.000029658956854023756 * daysPerYear to bvz.\n    Push 0.00004366244043351563 * solarMass to bm.\n    Push 15.379697114850917 to bx. Push 0.0 - 25.919314609987964 to posY. Push 0.17925877295037118 to bz.\n    Push 0.0026806777249038932 * daysPerYear to bvx. Push 0.001628241700382423 * daysPerYear to bvy. Push 0.0 - 0.00009515922545197159 * daysPerYear to bvz.\n    Push 0.00005151389020466114 * solarMass to bm.\n    Let mutable px be 0.0.\n    Let mutable py be 0.0.\n    Let mutable pz be 0.0.\n    Let mutable i be 1.\n    While i is at most 5:\n        Set px to px + item i of bvx * item i of bm.\n        Set py to py + item i of bvy * item i of bm.\n        Set pz to pz + item i of bvz * item i of bm.\n        Set i to i + 1.\n    Set item 1 of bvx to 0.0 - px / solarMass.\n    Set item 1 of bvy to 0.0 - py / solarMass.\n    Set item 1 of bvz to 0.0 - pz / solarMass.\n    Let dt be 0.01.\n    Let mutable step be 0.\n    While step is less than n:\n        Set i to 1.\n        While i is at most 5:\n            Let mutable j be i + 1.\n            While j is at most 5:\n                Let dx be item i of bx - item j of bx.\n                Let dy be item i of posY - item j of posY.\n                Let dz be item i of bz - item j of bz.\n                Let dist be sqrt(dx * dx + dy * dy + dz * dz).\n                Let mag be dt / (dist * dist * dist).\n                Set item i of bvx to item i of bvx - dx * item j of bm * mag.\n                Set item i of bvy to item i of bvy - dy * item j of bm * mag.\n                Set item i of bvz to item i of bvz - dz * item j of bm * mag.\n                Set item j of bvx to item j of bvx + dx * item i of bm * mag.\n                Set item j of bvy to item j of bvy + dy * item i of bm * mag.\n                Set item j of bvz to item j of bvz + dz * item i of bm * mag.\n                Set j to j + 1.\n            Set i to i + 1.\n        Set i to 1.\n        While i is at most 5:\n            Set item i of bx to item i of bx + dt * item i of bvx.\n            Set item i of posY to item i of posY + dt * item i of bvy.\n            Set item i of bz to item i of bz + dt * item i of bvz.\n            Set i to i + 1.\n        Set step to step + 1.\n    Let mutable e be 0.0.\n    Set i to 1.\n    While i is at most 5:\n        Set e to e + 0.5 * item i of bm * (item i of bvx * item i of bvx + item i of bvy * item i of bvy + item i of bvz * item i of bvz).\n        Let mutable j be i + 1.\n        While j is at most 5:\n            Let dx be item i of bx - item j of bx.\n            Let dy be item i of posY - item j of posY.\n            Let dz be item i of bz - item j of bz.\n            Set e to e - item i of bm * item j of bm / sqrt(dx * dx + dy * dy + dz * dz).\n            Set j to j + 1.\n        Set i to i + 1.\n    Return floor(e * 1000000000.0).\n",
        args: &[&[0], &[1], &[10], &[50]],
    },
    // spectral_norm — the REAL power-iteration: u/v/tmp float Seqs through the mulAv/mulAtv/mulAtav recursion
    // (float lane in `program_calls_fabric`, with float-ness seeded from `param_types` and Call return types).
    Bench {
        name: "spectral_norm",
        status: Supported,
        src: "## To aVal (i: Int, j: Int) -> Float:\n    Return 1.0 / ((i + j) * (i + j + 1) / 2 + i + 1).\n## To mulAv (n: Int, v: Seq of Float, out: Seq of Float) -> Seq of Float:\n    Let mutable result be out.\n    Let mutable i be 0.\n    While i is less than n:\n        Let mutable sum be 0.0.\n        Let mutable j be 0.\n        While j is less than n:\n            Set sum to sum + aVal(i, j) * item (j + 1) of v.\n            Set j to j + 1.\n        Set item (i + 1) of result to sum.\n        Set i to i + 1.\n    Return result.\n## To mulAtv (n: Int, v: Seq of Float, out: Seq of Float) -> Seq of Float:\n    Let mutable result be out.\n    Let mutable i be 0.\n    While i is less than n:\n        Let mutable sum be 0.0.\n        Let mutable j be 0.\n        While j is less than n:\n            Set sum to sum + aVal(j, i) * item (j + 1) of v.\n            Set j to j + 1.\n        Set item (i + 1) of result to sum.\n        Set i to i + 1.\n    Return result.\n## To mulAtav (n: Int, v: Seq of Float, result: Seq of Float, tmp: Seq of Float) -> Seq of Float:\n    Let t be mulAv(n, v, tmp).\n    Return mulAtv(n, t, result).\n## To f (n: Int) -> Int:\n    Let mutable u be a new Seq of Float.\n    Let mutable v be a new Seq of Float.\n    Let mutable tmp be a new Seq of Float.\n    Let mutable i be 0.\n    While i is less than n:\n        Push 1.0 to u.\n        Push 0.0 to v.\n        Push 0.0 to tmp.\n        Set i to i + 1.\n    Set i to 0.\n    While i is less than 10:\n        Set v to mulAtav(n, u, v, tmp).\n        Set u to mulAtav(n, v, u, tmp).\n        Set i to i + 1.\n    Let mutable vbv be 0.0.\n    Let mutable vv be 0.0.\n    Set i to 1.\n    While i is at most n:\n        Set vbv to vbv + item i of u * item i of v.\n        Set vv to vv + item i of v * item i of v.\n        Set i to i + 1.\n    Return floor(sqrt(vbv / vv) * 1000000000.0).\n",
        args: &[&[1], &[3], &[5], &[10]],
    },
    // Map / dictionary: `a new Map of Int to Int`, `m contains k`, `Set m at k to v`. The map is a DIRECT-ADDRESS
    // table on the heap (`heap[ptr + key]`, same model `collect` uses for get/set); `contains` is `heap[ptr+key]
    // != 0`. Keys repeat (`i % 5 + 1`) so `contains` genuinely returns true — the count grows once each key is seen.
    Bench {
        name: "two_sum",
        status: Supported,
        // The REAL two_sum: a seeded RNG array, then a Map complement lookup (`seen contains target - x`).
        src: "## To f (n: Int) -> Int:\n    Let target be n.\n    Let arr be a new Seq of Int.\n    Let mutable seed be 42.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Push ((seed / 65536) % 32768) % n to arr.\n        Set i to i + 1.\n    Let seen be a new Map of Int to Int.\n    Let mutable count be 0.\n    Set i to 1.\n    While i is at most n:\n        Let x be item i of arr.\n        Let complement be target - x.\n        If complement is at least 0:\n            If seen contains complement:\n                Set count to count + 1.\n        Set seen at x to 1.\n        Set i to i + 1.\n    Return count.\n",
        args: &[&[1], &[5], &[20], &[50]],
    },
    Bench {
        name: "collect",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let m be a new Map of Int to Int.\n    Let mutable i be 1.\n    While i is less than n + 1:\n        Set item i of m to i * 2.\n        Set i to i + 1.\n    Let mutable found be 0.\n    Set i to 1.\n    While i is less than n + 1:\n        If item i of m equals i * 2:\n            Set found to found + 1.\n        Set i to i + 1.\n    Return found.\n",
        args: &[&[1], &[10], &[100]],
    },
    // Text building with int→string (ITOA): `result + i` appends `i`'s decimal digits, emitted as static cells
    // (digit-count comparison chain + MSB-first writes; invalid positions read-and-write-back). The `equals " "`
    // compares an indexed char to a 1-char string literal — the literal is reduced to its byte (`heap[ptr+1]`).
    Bench {
        name: "strings",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let mutable result be \"\".\n    Let mutable i be 0.\n    While i is less than n:\n        Set result to result + i + \" \".\n        Set i to i + 1.\n    Let mutable count be 0.\n    Set i to 1.\n    While i is at most length of result:\n        If item i of result equals \" \":\n            Set count to count + 1.\n        Set i to i + 1.\n    Return count.\n",
        args: &[&[1], &[3], &[10], &[20]],
    },
    Bench {
        name: "string_search",
        status: Supported,
        src: "## To f (n: Int) -> Int:\n    Let mutable text be \"\".\n    Let mutable pos be 0.\n    While pos is less than n:\n        Set text to text + \"a\".\n        Set pos to pos + 1.\n    Let needle be \"a\".\n    Let mutable count be 0.\n    Let mutable i be 1.\n    While i is at most length of text:\n        If item i of text equals item 1 of needle:\n            Set count to count + 1.\n        Set i to i + 1.\n    Return count.\n",
        args: &[&[1], &[10], &[100]],
    },
    Bench {
        name: "sha256",
        status: Supported,
        // The REAL SHA-256 compression (message schedule + 64 rounds + ch/maj/σ + the 64-entry K
        // table + IVs), through the WORD32 lane (word32/rotr/word32Shr/word_and/word_not/wrapping-add
        // + Seq of Word32) in `program_calls_fabric`. `Repeat for` range/for-each loops are written as
        // While loops (identical computation) since the engine lowers While, not IterNext.
        src: "## To sha256K -> Seq of Word32:
    Let mutable kk be a new Seq of Word32.
    Push word32(1116352408) to kk.
    Push word32(1899447441) to kk.
    Push word32(3049323471) to kk.
    Push word32(3921009573) to kk.
    Push word32(961987163) to kk.
    Push word32(1508970993) to kk.
    Push word32(2453635748) to kk.
    Push word32(2870763221) to kk.
    Push word32(3624381080) to kk.
    Push word32(310598401) to kk.
    Push word32(607225278) to kk.
    Push word32(1426881987) to kk.
    Push word32(1925078388) to kk.
    Push word32(2162078206) to kk.
    Push word32(2614888103) to kk.
    Push word32(3248222580) to kk.
    Push word32(3835390401) to kk.
    Push word32(4022224774) to kk.
    Push word32(264347078) to kk.
    Push word32(604807628) to kk.
    Push word32(770255983) to kk.
    Push word32(1249150122) to kk.
    Push word32(1555081692) to kk.
    Push word32(1996064986) to kk.
    Push word32(2554220882) to kk.
    Push word32(2821834349) to kk.
    Push word32(2952996808) to kk.
    Push word32(3210313671) to kk.
    Push word32(3336571891) to kk.
    Push word32(3584528711) to kk.
    Push word32(113926993) to kk.
    Push word32(338241895) to kk.
    Push word32(666307205) to kk.
    Push word32(773529912) to kk.
    Push word32(1294757372) to kk.
    Push word32(1396182291) to kk.
    Push word32(1695183700) to kk.
    Push word32(1986661051) to kk.
    Push word32(2177026350) to kk.
    Push word32(2456956037) to kk.
    Push word32(2730485921) to kk.
    Push word32(2820302411) to kk.
    Push word32(3259730800) to kk.
    Push word32(3345764771) to kk.
    Push word32(3516065817) to kk.
    Push word32(3600352804) to kk.
    Push word32(4094571909) to kk.
    Push word32(275423344) to kk.
    Push word32(430227734) to kk.
    Push word32(506948616) to kk.
    Push word32(659060556) to kk.
    Push word32(883997877) to kk.
    Push word32(958139571) to kk.
    Push word32(1322822218) to kk.
    Push word32(1537002063) to kk.
    Push word32(1747873779) to kk.
    Push word32(1955562222) to kk.
    Push word32(2024104815) to kk.
    Push word32(2227730452) to kk.
    Push word32(2361852424) to kk.
    Push word32(2428436474) to kk.
    Push word32(2756734187) to kk.
    Push word32(3204031479) to kk.
    Push word32(3329325298) to kk.
    Return kk.
## To ssig0 (x: Word32) -> Word32:
    Return rotr(x, 7) xor rotr(x, 18) xor word32Shr(x, 3).
## To ssig1 (x: Word32) -> Word32:
    Return rotr(x, 17) xor rotr(x, 19) xor word32Shr(x, 10).
## To bsig0 (x: Word32) -> Word32:
    Return rotr(x, 2) xor rotr(x, 13) xor rotr(x, 22).
## To bsig1 (x: Word32) -> Word32:
    Return rotr(x, 6) xor rotr(x, 11) xor rotr(x, 25).
## To ch (x: Word32) and (y: Word32) and (z: Word32) -> Word32:
    Return word_and(x, y) xor word_and(word_not(x), z).
## To maj (x: Word32) and (y: Word32) and (z: Word32) -> Word32:
    Return word_and(x, y) xor word_and(x, z) xor word_and(y, z).
## To sha256Compress (state: Seq of Word32) and (m: Seq of Word32) and (kk: Seq of Word32) -> Seq of Word32:
    Let mutable sched be a new Seq of Word32.
    Let mutable mi be 1.
    While mi is at most 16:
        Push item mi of m to sched.
        Set mi to mi + 1.
    Let mutable t be 16.
    While t is at most 63:
        Let s0 be ssig0(item (t - 14) of sched).
        Let s1 be ssig1(item (t - 1) of sched).
        Let nw be s1 + (item (t - 6) of sched) + s0 + (item (t - 15) of sched).
        Push nw to sched.
        Set t to t + 1.
    Let mutable a be item 1 of state.
    Let mutable b be item 2 of state.
    Let mutable c be item 3 of state.
    Let mutable d be item 4 of state.
    Let mutable e be item 5 of state.
    Let mutable f be item 6 of state.
    Let mutable g be item 7 of state.
    Let mutable h be item 8 of state.
    Set t to 0.
    While t is at most 63:
        Let t1 be h + bsig1(e) + ch(e, f, g) + (item (t + 1) of kk) + (item (t + 1) of sched).
        Let t2 be bsig0(a) + maj(a, b, c).
        Set h to g.
        Set g to f.
        Set f to e.
        Set e to d + t1.
        Set d to c.
        Set c to b.
        Set b to a.
        Set a to t1 + t2.
        Set t to t + 1.
    Let mutable out be a new Seq of Word32.
    Push (item 1 of state) + a to out.
    Push (item 2 of state) + b to out.
    Push (item 3 of state) + c to out.
    Push (item 4 of state) + d to out.
    Push (item 5 of state) + e to out.
    Push (item 6 of state) + f to out.
    Push (item 7 of state) + g to out.
    Push (item 8 of state) + h to out.
    Return out.
## To f (n: Int) -> Int:
    Let kk be sha256K().
    Let mutable state be a new Seq of Word32.
    Push word32(0) to state.
    Push word32(0) to state.
    Push word32(0) to state.
    Push word32(0) to state.
    Push word32(0) to state.
    Push word32(0) to state.
    Push word32(0) to state.
    Push word32(0) to state.
    Let mutable k be 0.
    While k is less than n:
        Let mutable m be a new Seq of Word32.
        Push (item 1 of state) to m.
        Push (item 2 of state) to m.
        Push (item 3 of state) to m.
        Push (item 4 of state) to m.
        Push (item 5 of state) to m.
        Push (item 6 of state) to m.
        Push (item 7 of state) to m.
        Push (item 8 of state) to m.
        Push word32(2147483648) to m.
        Push word32(0) to m.
        Push word32(0) to m.
        Push word32(0) to m.
        Push word32(0) to m.
        Push word32(0) to m.
        Push word32(0) to m.
        Push word32(256) to m.
        Let mutable h0 be a new Seq of Word32.
        Push word32(1779033703) to h0.
        Push word32(3144134277) to h0.
        Push word32(1013904242) to h0.
        Push word32(2773480762) to h0.
        Push word32(1359893119) to h0.
        Push word32(2600822924) to h0.
        Push word32(528734635) to h0.
        Push word32(1541459225) to h0.
        Set state to sha256Compress(h0, m, kk).
        Set k to k + 1.
    Return intOfWord32(item 1 of state).
",
        args: &[&[0], &[1], &[2], &[4]],
    },
];

#[test]
fn the_whole_program_engine_runs_the_whole_benchmark_suite() {
    let mut supported_ok = Vec::new();
    let mut supported_broken = Vec::new();
    let mut gap_still = Vec::new();
    let mut gap_now_works = Vec::new();

    for b in BENCHMARKS {
        // Anchor: the VM must run every input (the oracle exists) before we claim anything about the engine.
        for a in b.args {
            assert!(
                vm_f(b.src, a).is_some(),
                "benchmark `{}` VM error at {a:?}",
                b.name
            );
        }
        let runs = runs_on_engine(b.src, b.args);
        match b.status {
            Supported if runs => supported_ok.push(b.name),
            Supported => supported_broken.push(b.name),
            KnownGap(lane) if runs => gap_now_works.push((b.name, lane)),
            KnownGap(lane) => gap_still.push((b.name, lane)),
        }
    }

    println!(
        "\nWHOLE-PROGRAM ENGINE — ALL {} BENCHMARKS:\n  RUNS ≡VM ({}/{}): {supported_ok:?}\n  KNOWN GAPS ({}): {gap_still:?}",
        BENCHMARKS.len(),
        supported_ok.len(),
        supported_ok.len() + supported_broken.len(),
        gap_still.len(),
    );

    // RATCHET 1: every SUPPORTED benchmark must still run ≡VM (a regression is RED).
    assert!(
        supported_broken.is_empty(),
        "these benchmarks are marked SUPPORTED but did NOT run on the whole-program engine ≡VM (regression): \
         {supported_broken:?}"
    );
    // RATCHET 2: a KNOWN_GAP that starts working must be PROMOTED to Supported (so the floor grows, never silently).
    assert!(
        gap_now_works.is_empty(),
        "these benchmarks are marked KNOWN_GAP but now RUN on the engine — promote them to Supported: \
         {gap_now_works:?}"
    );
}

/// Build each benchmark's engine config the way `runs_on_engine` proves ≡VM (prefer the single-function
/// `program_cfg_fabric`; fall back to `program_calls_fabric` for recursion), returning `None` if neither lowers.
fn config_for(b: &Bench) -> Option<WProgConfig> {
    let cfg = catch_unwind(AssertUnwindSafe(|| {
        serialize_program(&program_cfg_fabric(b.src, "f"))
    }))
    .ok();
    if let Some(c) = &cfg {
        if b.args
            .iter()
            .all(|a| matches!(vm_f(b.src, a), Some(vm) if eval_wprog_config(c, a) == vm))
        {
            return cfg;
        }
    }
    let calls = catch_unwind(AssertUnwindSafe(|| program_calls_fabric(b.src, "f"))).ok();
    if let Some(c) = &calls {
        if b.args
            .iter()
            .all(|a| matches!(vm_f(b.src, a), Some(vm) if eval_wprog_config(c, a) == vm))
        {
            return calls;
        }
    }
    None
}

/// Generator: for EVERY benchmark, build its config, fit-check it against the DEPLOYED big engine (`WPE_HW_*` =
/// HEAP=64/DEPTH=8), and for the ones that FIT, write `bench_<name>.wire` + a `bench_manifest.txt` line
/// (`name arg oracle`), asserting each wire's engine oracle equals the VM. Prints an honest FIT / NOFIT table
/// with the exact cap each over-budget benchmark exceeds — so "run all 36" has a truthful boundary, not a silent
/// drop. A run writes into a scratch directory and asserts the manifest is the committed
/// `benchmarks/silicon/programs/bench_manifest.txt`, whose oracles are the goldens the board suite checks. The
/// committed wires are what Board B ran (`fabric_silicon_runs.json`); `scripts/test/gen-silicon-goldens.sh wires`
/// writes new ones, and a new wire is proven only by running the board suite again.
#[test]
#[ignore]
fn generate_benchmark_wires_for_the_fpga() {
    let mut manifest = String::new();
    let (mut fit, mut nofit, mut nolow) = (0u32, 0u32, 0u32);
    // WPE_PROFILE selects which deployed engine to fit against: `hw` = the XC7A100T flip-flop engine
    // (bench_*.wire), `stream` = the XC7A200T block-RAM engine (s200_*.wire). One code path, so the two
    // boards can never drift apart in how their programs are built.
    let stream = std::env::var("WPE_PROFILE").as_deref() == Ok("stream");
    let prefix = if stream { "s200" } else { "bench" };
    let dir = golden::output_dir(
        "benchmarks/silicon/programs",
        &format!("logos_{prefix}_wires"),
    );
    let regen = format!(
        "{}scripts/test/gen-silicon-goldens.sh wires",
        if stream { "WPE_PROFILE=stream " } else { "" }
    );
    for b in BENCHMARKS {
        let cfg = match config_for(b) {
            Some(c) => c,
            None => {
                nolow += 1;
                eprintln!("NOLOW {:<16} (no ≡VM lowering)", b.name);
                continue;
            }
        };
        // Pick the LARGEST declared argument this engine profile can actually run, MEASURED rather than
        // guessed: run each candidate and keep it only if its peak frame depth fits DEPTH and its heap
        // footprint fits HEAP. A hardcoded per-benchmark table (what this used to be) silently goes stale the
        // moment the caps change — and an arg that overruns DEPTH does not fail, it returns a wrong number.
        // Sorting by the oracle's step count keeps the biggest real workload that still fits.
        let (heap_cap, depth_cap) = if stream {
            (WPE_STREAM_HEAP, WPE_STREAM_DEPTH)
        } else {
            (WPE_HW_HEAP, WPE_HW_DEPTH)
        };
        let mut runnable: Vec<(u64, &[i64])> = b
            .args
            .iter()
            .filter_map(|a| {
                catch_unwind(AssertUnwindSafe(|| eval_wprog_config_probed(&cfg, a, &[])))
                    .ok()
                    .filter(|(_, _, heap, depth)| heap.len() <= heap_cap && *depth <= depth_cap)
                    .map(|(_, steps, _, _)| (steps, *a))
            })
            .collect();
        runnable.sort_by_key(|(steps, _)| *steps);
        let Some((_, arg)) = runnable.last().copied() else {
            nofit += 1;
            // Say WHICH bound each argument broke — "does not fit" without the dimension is a dead end.
            let probes: Vec<String> = b
                .args
                .iter()
                .filter_map(|a| {
                    catch_unwind(AssertUnwindSafe(|| eval_wprog_config_probed(&cfg, a, &[])))
                        .ok()
                        .map(|(_, _, heap, depth)| {
                            format!("{a:?}: heap {} depth {depth}", heap.len())
                        })
                })
                .collect();
            eprintln!(
                "NOFIT {:<16} no argument fits HEAP={heap_cap}/DEPTH={depth_cap} — {}",
                b.name,
                if probes.is_empty() {
                    "every argument panicked".to_string()
                } else {
                    probes.join("; ")
                }
            );
            continue;
        };
        let fits = if stream {
            wprog_config_fits_stream_engine(&cfg)
        } else {
            wprog_config_fits_hw_engine(&cfg)
        };
        match fits {
            Ok(()) => {
                let wire = wprog_config_image_unchecked(&cfg).wire_bytes(arg);
                std::fs::write(dir.join(format!("{prefix}_{}.wire", b.name)), &wire)
                    .expect("write wire");
                // Record BOTH the engine oracle and the VM's own answer. The on-chip result is compared
                // against the VM — the engine model agreeing with itself proves nothing about the language.
                // They must be equal here (the census ratchet already proves engine ≡ VM); writing both makes
                // any future divergence visible in the manifest instead of hidden behind one number.
                let oracle = eval_wprog_config(&cfg, arg);
                let vm = vm_f(b.src, arg);
                assert_eq!(
                    Some(oracle), vm,
                    "{}: engine oracle {oracle} != VM {vm:?} at arg {arg:?} — the wire image would be \
                     validated on silicon against a golden that does not match the language",
                    b.name
                );
                let args = arg
                    .iter()
                    .map(|x| x.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                manifest.push_str(&format!("{} {} {}\n", b.name, args, oracle));
                fit += 1;
                eprintln!(
                    "FIT   {:<16} arg={arg:?} oracle={oracle} ({} bytes)",
                    b.name,
                    wire.len()
                );
            }
            Err(why) => {
                nofit += 1;
                eprintln!("NOFIT {:<16} exceeds a cap: {why}", b.name);
            }
        }
    }
    std::fs::write(dir.join(format!("{prefix}_manifest.txt")), &manifest).expect("write manifest");
    golden::committed_bytes(
        &format!("benchmarks/silicon/programs/{prefix}_manifest.txt"),
        manifest.as_bytes(),
        &regen,
    );
    eprintln!(
        "\n=== {}/{} benchmarks FIT the {} engine profile; {nofit} exceed a cap or have no runnable argument, \
         {nolow} don't lower ===",
        fit,
        BENCHMARKS.len(),
        if std::env::var("WPE_PROFILE").as_deref() == Ok("stream") { "XC7A200T stream" } else { "XC7A100T hw" }
    );
}

/// The XC7A200T engine (`WPE_STREAM`, config image in block RAM) is the profile the whole suite is meant to
/// run on, streamed over UART. That intent is only real if the caps actually COVER every benchmark — so this
/// asserts it dimension by dimension instead of leaving it to a `.wire` that loads fine and returns a wrong
/// number. Each shortfall names the benchmark that drives it, so raising a cap is a measured change.
///
/// This is the lock that makes "all 35 on the big chip" a fact rather than an aspiration. DEPTH is included
/// precisely because it is the dimension the hardware cannot self-report: the engine advances `sp` past
/// `DEPTH` unchecked, so an over-deep program is answered incorrectly rather than refused.
#[test]
fn the_stream_engine_profile_covers_every_benchmark_in_the_suite() {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
        WPE_STREAM_DEPTH, WPE_STREAM_MAXARG, WPE_STREAM_MAXBLK, WPE_STREAM_MAXCELL,
        WPE_STREAM_MAXK, WPE_STREAM_MAXLIV, WPE_STREAM_MAXV, WPE_STREAM_MAXWR, WPE_STREAM_NREG,
    };

    let mut short: Vec<String> = Vec::new();
    let mut note = |dim: &str, need: usize, cap: usize, who: &str| {
        if need > cap {
            short.push(format!(
                "{dim}: needs {need} ({who}) but WPE_STREAM_{dim}={cap}"
            ));
        }
    };

    let (mut blk, mut reg, mut cell, mut v, mut wr, mut liv, mut arg, mut k, mut dep) = (
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
    );
    let mx = |acc: &mut (usize, &'static str), val: usize, name: &'static str| {
        if val > acc.0 {
            *acc = (val, name);
        }
    };
    for b in BENCHMARKS {
        let Some(cfg) = config_for(b) else { continue };
        mx(&mut blk, cfg.blocks.len(), b.name);
        mx(&mut reg, nregs(&cfg), b.name);
        mx(
            &mut cell,
            cfg.blocks.iter().map(|x| x.cells.len()).sum(),
            b.name,
        );
        mx(
            &mut v,
            cfg.blocks.iter().map(|x| x.cells.len()).max().unwrap_or(0),
            b.name,
        );
        mx(
            &mut wr,
            cfg.blocks.iter().map(|x| x.writes.len()).sum(),
            b.name,
        );
        mx(
            &mut liv,
            cfg.blocks.iter().map(|x| x.livein.len()).sum(),
            b.name,
        );
        mx(&mut arg, total_args(&cfg), b.name);
        mx(&mut k, konst_count(&cfg), b.name);
        mx(&mut dep, peak_depth(&cfg, b.args), b.name);
    }

    note("MAXBLK", blk.0, WPE_STREAM_MAXBLK, blk.1);
    note("NREG", reg.0, WPE_STREAM_NREG, reg.1);
    note("MAXCELL", cell.0, WPE_STREAM_MAXCELL, cell.1);
    note("MAXV", v.0, WPE_STREAM_MAXV, v.1);
    note("MAXWR", wr.0, WPE_STREAM_MAXWR, wr.1);
    note("MAXLIV", liv.0, WPE_STREAM_MAXLIV, liv.1);
    note("MAXARG", arg.0, WPE_STREAM_MAXARG, arg.1);
    note("MAXK", k.0, WPE_STREAM_MAXK, k.1);
    note("DEPTH", dep.0, WPE_STREAM_DEPTH, dep.1);

    assert!(
        short.is_empty(),
        "the XC7A200T engine profile does NOT cover the benchmark suite in {} dimension(s):\n  {}\n\
         Every one of these is a benchmark that will be accepted by the loader and produce a WRONG answer \
         rather than an error, because nothing between the wire image and the engine re-checks the profile. \
         Raise the cap (and re-route the bitstream) or the suite cannot honestly be claimed to run on it.",
        short.len(),
        short.join("\n  ")
    );
}

/// Distinct konsts in a config, walked by hand — `wprog_config_image()` panics on configs that exceed the
/// GENERIC engine caps, and both the sizing oracle and the coverage lock need to measure configs bigger than
/// any single engine profile.
fn konst_count(cfg: &WProgConfig) -> usize {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::WTerm;
    let mut set = std::collections::BTreeSet::new();
    let mut note = |s: &logicaffeine_compile::codegen_sva::wprog_engine_rtl::WSrc| {
        if s.mode == 1 {
            set.insert(s.konst);
        }
    };
    for b in &cfg.blocks {
        for c in &b.cells {
            for s in &c.ops {
                note(s);
            }
        }
        for (_, s) in &b.writes {
            note(s);
        }
        match &b.term {
            WTerm::Branch { cond, .. } => note(cond),
            WTerm::Return(s) => note(s),
            WTerm::Call { args, .. } => args.iter().for_each(&mut note),
            WTerm::Jump(_) => {}
        }
    }
    set.len()
}

/// The register-file size a config demands (highest register touched, at least the parameter count).
fn nregs(cfg: &WProgConfig) -> usize {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::WTerm;
    let mut m = 0u16;
    for b in &cfg.blocks {
        for r in &b.livein {
            m = m.max(*r);
        }
        for (r, _) in &b.writes {
            m = m.max(*r);
        }
        if let WTerm::Call { dst_reg, .. } = &b.term {
            m = m.max(*dst_reg);
        }
    }
    (m as usize + 1).max(cfg.n_params).max(1)
}

/// Total call arguments a config passes — the `MAXARG` demand.
fn total_args(cfg: &WProgConfig) -> usize {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::WTerm;
    cfg.blocks
        .iter()
        .map(|x| {
            if let WTerm::Call { args, .. } = &x.term {
                args.len()
            } else {
                0
            }
        })
        .sum()
}

/// The PEAK call-frame depth a benchmark reaches over every argument it declares. Unlike every other
/// dimension this is not readable off the config — it depends on the arguments, so it must be run.
fn peak_depth(cfg: &WProgConfig, args: &[&[i64]]) -> usize {
    args.iter()
        .filter_map(|a| {
            catch_unwind(AssertUnwindSafe(|| eval_wprog_config_probed(cfg, a, &[]).3)).ok()
        })
        .max()
        .unwrap_or(0)
}

/// Sizing oracle for the "runs-everything" engine: the MAX config dimension each benchmark demands, so the
/// BRAM-backed universal engine can be sized to the exact ceiling that covers all 35 (no waste, no shortfall).
/// Prints the per-dimension max + which benchmark drives it. `#[ignore]` — a diagnostic, run on demand.
#[test]
#[ignore]
fn print_max_dims_across_all_benchmarks() {
    let mx = |acc: &mut (usize, &'static str), v: usize, name: &'static str| {
        if v > acc.0 {
            *acc = (v, name);
        }
    };
    let (
        mut blocks,
        mut regs,
        mut tcells,
        mut cpb,
        mut twr,
        mut wpb,
        mut tliv,
        mut lpb,
        mut targs,
        mut konsts,
    ) = (
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
        (0, ""),
    );
    let mut depth = (0usize, "");
    for b in BENCHMARKS {
        let Some(cfg) = config_for(b) else {
            continue;
        };
        mx(&mut blocks, cfg.blocks.len(), b.name);
        mx(&mut regs, nregs(&cfg), b.name);
        mx(
            &mut tcells,
            cfg.blocks.iter().map(|x| x.cells.len()).sum(),
            b.name,
        );
        mx(
            &mut cpb,
            cfg.blocks.iter().map(|x| x.cells.len()).max().unwrap_or(0),
            b.name,
        );
        mx(
            &mut twr,
            cfg.blocks.iter().map(|x| x.writes.len()).sum(),
            b.name,
        );
        mx(
            &mut wpb,
            cfg.blocks.iter().map(|x| x.writes.len()).max().unwrap_or(0),
            b.name,
        );
        mx(
            &mut tliv,
            cfg.blocks.iter().map(|x| x.livein.len()).sum(),
            b.name,
        );
        mx(
            &mut lpb,
            cfg.blocks.iter().map(|x| x.livein.len()).max().unwrap_or(0),
            b.name,
        );
        mx(&mut targs, total_args(&cfg), b.name);
        mx(&mut konsts, konst_count(&cfg), b.name);
        // DEPTH is the ONE dimension that is not a static property of the config — it depends on the
        // arguments, so it can only be MEASURED by running the program. It is also the one the hardware gets
        // silently wrong (the engine advances `sp` past DEPTH with no bound check), so leaving it out of the
        // ceiling is what let a DEPTH=4 engine ship against DEPTH=8 programs and return plausible wrong
        // numbers on silicon.
        mx(&mut depth, peak_depth(&cfg, b.args), b.name);
    }
    eprintln!("\n=== MAX config dimensions across all lowering benchmarks (the runs-everything ceiling) ===");
    eprintln!(
        "MAXBLK  (blocks)          = {:>4}  ({})",
        blocks.0, blocks.1
    );
    eprintln!("NREG    (registers)       = {:>4}  ({})", regs.0, regs.1);
    eprintln!(
        "MAXCELL (total cells)     = {:>4}  ({})",
        tcells.0, tcells.1
    );
    eprintln!("MAXVAL  (cells/block, v[])= {:>4}  ({})", cpb.0, cpb.1);
    eprintln!("MAXWR   (total writes)    = {:>4}  ({})", twr.0, twr.1);
    eprintln!("        (writes/block)    = {:>4}  ({})", wpb.0, wpb.1);
    eprintln!("MAXLIV  (total live-ins)  = {:>4}  ({})", tliv.0, tliv.1);
    eprintln!("        (live-ins/block)  = {:>4}  ({})", lpb.0, lpb.1);
    eprintln!("MAXARG  (total call args) = {:>4}  ({})", targs.0, targs.1);
    eprintln!(
        "MAXK    (distinct konsts) = {:>4}  ({})",
        konsts.0, konsts.1
    );
    eprintln!(
        "DEPTH   (peak call frames)= {:>4}  ({})  [MEASURED, not static]",
        depth.0, depth.1
    );
}

/// Stream `cfg`(`arg`) through the engine emitted with `backend` at tight per-benchmark caps (`caps_for`) under
/// iverilog. Returns `Ok(result)`
/// when the returned 64-bit result == `eval_wprog_config` (≡VM), else `Err(reason)`. Proves the interpret CORE runs
/// a FULL-SIZE program end to end on the actual RTL — closing the RTL-vs-model gap for the big benchmarks. Once
/// green on the FF (`Registers`) backend, the FF→BRAM substrate swap is a fit optimization on proven-correct logic.
/// The largest heap (64-bit words) the survey will size an engine to under iverilog — keeps sims tractable on a
/// loaded box. A benchmark whose eval heap footprint exceeds this at every arg is skipped (its demo input is too
/// big to simulate here — a real silicon-capacity note, not an engine bug).
const SURVEY_HEAP_CAP: usize = 131072;

/// Tight per-benchmark engine caps = the program's own dimensions + a little headroom, with HEAP sized to the
/// program's ACTUAL footprint at `arg` (from the eval model) so the survey never starves the heap and reports a
/// false divergence. Sizing each engine to its benchmark keeps the small programs' iverilog sims SMALL and fast —
/// essential on a shared, heavily-loaded box where a 35× big-caps sweep chokes.
fn caps_for(cfg: &WProgConfig, arg: &[i64]) -> [usize; 10] {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
        eval_wprog_config_counted_heap, WTerm,
    };
    let mut nreg = cfg.n_params as u16;
    let (mut cells, mut writes, mut liveins, mut args) = (0usize, 0usize, 0usize, 0usize);
    let mut konsts = std::collections::BTreeSet::new();
    for b in &cfg.blocks {
        for r in &b.livein {
            nreg = nreg.max(*r);
        }
        for (r, s) in &b.writes {
            nreg = nreg.max(*r);
            if s.mode == 1 {
                konsts.insert(s.konst);
            }
        }
        for c in &b.cells {
            for s in &c.ops {
                if s.mode == 1 {
                    konsts.insert(s.konst);
                }
            }
        }
        match &b.term {
            WTerm::Branch { cond, .. } => {
                if cond.mode == 1 {
                    konsts.insert(cond.konst);
                }
            }
            WTerm::Return(s) => {
                if s.mode == 1 {
                    konsts.insert(s.konst);
                }
            }
            WTerm::Call {
                args: a, dst_reg, ..
            } => {
                nreg = nreg.max(*dst_reg);
                args += a.len();
                for s in a {
                    if s.mode == 1 {
                        konsts.insert(s.konst);
                    }
                }
            }
            WTerm::Jump(_) => {}
        }
        cells += b.cells.len();
        writes += b.writes.len();
        liveins += b.livein.len();
    }
    let heap_used = eval_wprog_config_counted_heap(cfg, arg).2.len();
    [
        (nreg as usize + 3),      // NREG   (+ headroom)
        cfg.blocks.len() + 1,     // MAXBLK
        cells.max(8),             // MAXCELL (whole-program flat cell total)
        writes.max(4),            // MAXWR
        args.max(2),              // MAXARG
        liveins.max(2),           // MAXLIV
        konsts.len().max(2),      // MAXK
        (heap_used + 64).max(64), // HEAP  (the program's ACTUAL footprint + margin — never a false starve)
        32,           // DEPTH (recursion depth — matches the ackermann/quicksort passes)
        cells.max(8), // MAXV  (per-block v[]; the whole-cell total is a safe upper bound here)
    ]
}

/// UART bit period of the lane's host, in engine cycles, and its engine clock period in ns (`#5` half-period).
const LANE_CLKS_PER_BIT: usize = 8;
const LANE_CLOCK_NS: u64 = 10;

/// The simulated time a healthy run can need: the host streaming the image and reading the reply (a byte is
/// ten bits and one more cycle), the parameter seed and heap clear, twice the model's bound on the engine's
/// run cycles, and the Show records the run cycles leave out. A run still going past this has diverged —
/// judged in simulated cycles, so a loaded box can slow the simulation but never fail it.
fn sim_budget_ns(
    wire_bytes: usize,
    caps: [usize; 10],
    run_cycle_bound: u64,
    shown_bytes: u64,
) -> u64 {
    let byte = 10 * LANE_CLKS_PER_BIT as u64 + 1;
    let reply = 14;
    let cycles = wire_bytes as u64 * byte
        + (caps[0] + caps[7]) as u64
        + 2 * run_cycle_bound
        + 2 * (shown_bytes + reply) * byte
        + 10_000;
    cycles * LANE_CLOCK_NS
}

fn big_engine_run(
    tag: &str,
    backend: MemBackend,
    cfg: &WProgConfig,
    arg: &[i64],
) -> Result<i64, String> {
    let oracle = eval_wprog_config(cfg, arg);
    let caps = caps_for(cfg, arg);
    let work = eval_wprog_config_run(cfg, arg, &[]).work;
    let run_cycle_bound = work.rtl_cycle_bound(caps[0], caps[4]);
    let soc = emit_wprog_engine_soc_verilog_backend(tag, caps, backend);
    if !common::require_tool("iverilog") {
        return Ok(oracle); // harness-absent: RTL emitted, model ≡VM already checked by the caller
    }
    let dir = std::env::temp_dir().join("logos_wpe_bigscale");
    std::fs::create_dir_all(&dir).unwrap();
    let im = wprog_config_image_unchecked(cfg);
    let wire = im.wire_bytes(arg);
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>()
    };
    std::fs::write(dir.join(format!("wire_{tag}.hex")), hex(&wire)).unwrap();
    std::fs::write(
        dir.join(format!("lens_{tag}.hex")),
        format!("{:08x}\n", wire.len() as u32),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("oracle_{tag}.hex")),
        format!("{:016x}\n", oracle as u64),
    )
    .unwrap();
    let tb = emit_wprog_engine_soc_testbench(tag, 1, LANE_CLKS_PER_BIT, wire.len());
    let default_timeout = "#20000000000 $display(\"FAIL: timeout\")";
    assert_eq!(
        tb.matches(default_timeout).count(),
        1,
        "the testbench carries one simulated-time timeout"
    );
    let budget = sim_budget_ns(wire.len(), caps, run_cycle_bound, work.shown_bytes);
    let tb = tb.replace(
        default_timeout,
        &format!("#{budget} $display(\"FAIL: timeout\")"),
    );
    let srcv = dir.join(format!("wpe_soc_{tag}.v"));
    let tbv = dir.join(format!("tb_wpe_soc_{tag}.v"));
    let sim = dir.join(format!("sim_{tag}.vvp"));
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    if !build.status.success() {
        return Err(format!(
            "iverilog build failed: {}",
            String::from_utf8_lossy(&build.stderr)
        ));
    }
    // The testbench's own timeout, set to the program's simulated-time budget, ends a run that never answers.
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    if stdout.lines().any(|l| l.trim() == "FAIL: timeout") {
        return Err(format!(
            "no answer within the {budget} ns budget (twice the model's bound of {run_cycle_bound} run cycles): a \
             non-terminating divergence; oracle={oracle}"
        ));
    }
    if !stdout.contains("PASS:") {
        return Err(format!(
            "diverged (oracle={oracle}): {}",
            stdout
                .lines()
                .find(|l| l.contains("FAIL"))
                .unwrap_or("")
                .trim()
        ));
    }
    let cycles: u64 = stdout
        .lines()
        .find_map(|l| {
            l.split_once("cycles=")
                .and_then(|(_, r)| r.split_whitespace().next()?.parse().ok())
        })
        .ok_or_else(|| format!("no FRAME line reports the run cycles:\n{stdout}"))?;
    if cycles > run_cycle_bound {
        return Err(format!("ran {cycles} cycles, past the model's bound of {run_cycle_bound}: the bound is unsound"));
    }
    Ok(oracle)
}

/// The model's bound on the engine's run cycles holds where the engine waits on its multiplier: a power walks the
/// multiplier once per step, and a multiply waits out the multiplier's stages before its write. A model that priced
/// either as one cycle is passed by these programs on the RTL, on both the register and the config-BRAM backend.
#[test]
fn the_run_cycle_bound_holds_where_the_engine_waits_on_its_multiplier() {
    let programs: &[(&str, &str, &[i64])] = &[
        ("pow_long", "## To f (x: Int) -> Int:\n    Return 1 to the power of x.\n", &[1000]),
        ("pow_wide", "## To f (x: Int) -> Int:\n    Return 2 to the power of x.\n", &[62]),
        (
            "mul_loop",
            "## To f (n: Int) -> Int:\n    Let mutable p be 1.\n    Let mutable i be 0.\n    While i is less than n:\n        \
             Set p to (p * 7 + i) % 1024.\n        Set i to i + 1.\n    Return p.\n",
            &[300],
        ),
    ];
    for &(tag, src, arg) in programs {
        let cfg = serialize_program(&program_cfg_fabric(src, "f"));
        assert_eq!(Some(eval_wprog_config(&cfg, arg)), vm_f(src, arg), "{tag}: the engine model is the VM");
        for (name, backend) in [("reg", MemBackend::Registers), ("cfg", MemBackend::ConfigBram)] {
            big_engine_run(&format!("bound_{tag}_{name}"), backend, &cfg, arg)
                .unwrap_or_else(|e| panic!("{tag} on the {name} backend: {e}"));
        }
    }
}

/// The largest test arg ≤ 24 whose eval HEAP footprint stays sim-tractable (`≤ SURVEY_HEAP_CAP`), so the run is
/// non-trivial yet the iverilog engine's heap array stays a reasonable size. `None` = every arg overflows the sim
/// heap cap (the benchmark's smallest demo input is too big to simulate here — a silicon-capacity note).
fn survey_arg(b: &Bench, cfg: &WProgConfig) -> Option<&'static [i64]> {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::eval_wprog_config_counted_heap;
    b.args
        .iter()
        .copied()
        .filter(|a| a.iter().all(|&x| (0..=24).contains(&x)))
        .filter(|a| eval_wprog_config_counted_heap(cfg, a).2.len() <= SURVEY_HEAP_CAP)
        .max_by_key(|a| a[0])
}

/// SCOPE SURVEY: run EVERY benchmark through the interpret core (Registers backend, FF in sim — no hold constraint)
/// with the heap sized to the program's ACTUAL footprint, and report which are RTL≡VM and which DIVERGE. With the
/// heap no longer starved, a divergence here is a REAL engine bug (not a survey artifact); SKIP = the demo input is
/// too big to simulate (silicon-capacity note). Asserts all runnable pass. `#[ignore]` — iverilog, minutes.
#[test]
#[ignore]
fn all_benchmarks_run_through_the_full_caps_rtl_core() {
    let mut pass = Vec::new();
    let mut fail = Vec::new();
    let mut skip = Vec::new();
    for b in BENCHMARKS {
        // Each benchmark is isolated in catch_unwind so ONE panicking harness path (a build/spawn error, an image
        // cap assert) cannot abort the whole survey — the point is the COMPLETE map, not the first failure.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let cfg = config_for(b)?;
            match survey_arg(b, &cfg) {
                Some(arg) => Some(Ok((
                    arg,
                    big_engine_run(b.name, MemBackend::Registers, &cfg, arg),
                ))),
                None => Some(Err(())), // lowers, but every demo arg overflows the sim heap cap
            }
        }));
        match outcome {
            Ok(Some(Ok((arg, Ok(r))))) => {
                eprintln!("RTL≡VM {:<14} arg={arg:?} -> {r}", b.name);
                pass.push(b.name);
            }
            Ok(Some(Ok((arg, Err(why))))) => {
                eprintln!("DIVERGE {:<13} arg={arg:?}: {why}", b.name);
                fail.push(b.name);
            }
            Ok(Some(Err(()))) => {
                eprintln!("SKIP   {:<14} (demo heap > {SURVEY_HEAP_CAP} words at every arg — too big to sim here)", b.name);
                skip.push(b.name);
            }
            Ok(None) => eprintln!("NOLOW  {:<14} (no ≡VM lowering)", b.name),
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| p.downcast_ref::<&str>().copied())
                    .unwrap_or("<panic>");
                eprintln!("PANIC  {:<14} {}", b.name, msg.lines().next().unwrap_or(""));
                fail.push(b.name);
            }
        }
    }
    eprintln!(
        "\n=== RTL≡VM (heap sized to footprint): {}/{} runnable pass; {} skipped (heap too big to sim); diverge: {:?} ===",
        pass.len(),
        pass.len() + fail.len(),
        skip.len(),
        fail
    );
    assert!(
        fail.is_empty(),
        "RTL diverges from eval_wprog_config for: {fail:?}"
    );
}

/// DISAMBIGUATE the RTL survey's divergences WITHOUT iverilog, using the eval model's actual heap footprint + step
/// count per benchmark. A `result=x` divergence whose `heap > survey HEAP cap` is a SURVEY ARTIFACT (heap overflow),
/// not an engine bug. A run's simulated-time budget scales with the model's own work, so a long program gets a long
/// budget and a timeout is a run that never answers. What remains — small heap yet still diverging — is a REAL
/// RTL bug to fix. `#[ignore]` — fast (eval only).
#[test]
#[ignore]
fn heap_and_steps_per_benchmark() {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::eval_wprog_config_counted_heap;
    eprintln!(
        "\n{:<14} {:>6} {:>10} {:>7}  result",
        "benchmark", "heap", "steps", "nreg"
    );
    for b in BENCHMARKS {
        let Some(cfg) = config_for(b) else {
            eprintln!("{:<14} (no ≡VM lowering)", b.name);
            continue;
        };
        let arg = survey_arg(b, &cfg).unwrap_or(b.args[0]);
        let (res, steps, heap) = eval_wprog_config_counted_heap(&cfg, arg);
        let flag = if heap.len() > SURVEY_HEAP_CAP {
            " >SIM-CAP"
        } else if steps > 5_000_000 {
            " SLOW"
        } else {
            ""
        };
        eprintln!(
            "{:<14} {:>8} {:>10} {:>5}  {res}{flag}  (arg={arg:?})",
            b.name,
            heap.len(),
            steps,
            cfg.blocks.len()
        );
    }
}

/// Which cell KINDS each benchmark uses — the RTL engine must implement every kind the eval model does (0 arith,
/// 1 select, 2 func, 3 heap-read, 4 heap-write, 5 alloc, 6 DEEP-COPY, 7 text-concat, 8 char-at, 9 text-eq). A
/// benchmark using a kind the RTL falls through on (currently 6/7/8/9 → `: hp`) diverges. `#[ignore]` — eval only.
#[test]
#[ignore]
fn cell_kinds_per_benchmark() {
    for b in BENCHMARKS {
        let Some(cfg) = config_for(b) else {
            continue;
        };
        let mut kinds = std::collections::BTreeSet::new();
        for blk in &cfg.blocks {
            for c in &blk.cells {
                kinds.insert(c.kind);
            }
        }
        let unimpl: Vec<u8> = kinds
            .iter()
            .copied()
            .filter(|k| (6..=9).contains(k))
            .collect();
        eprintln!(
            "{:<14} kinds={:?}{}",
            b.name,
            kinds,
            if unimpl.is_empty() {
                String::new()
            } else {
                format!("  <- RTL-UNIMPL {unimpl:?}")
            }
        );
    }
}

/// BISECT the sort divergence: run each sorting benchmark through the RTL engine (heap sized to footprint) at
/// increasing n and report the first n where the RTL diverges from eval. Localizes the scale-dependent bug — a
/// break at a power-of-two / specific-width n points at a truncated field; a break tied to heap size points at
/// address width; a break tied to recursion depth points at the frame stack. `#[ignore]` — iverilog.
#[test]
#[ignore]
fn bisect_the_sort_divergence() {
    for name in ["quicksort", "heap_sort", "mergesort"] {
        let b = BENCHMARKS.iter().find(|x| x.name == name).expect("bench");
        let Some(cfg) = config_for(b) else {
            eprintln!("{name}: no ≡VM lowering");
            continue;
        };
        for n in [1i64, 2, 3, 4, 5, 8, 12, 15, 16, 17, 18, 20] {
            let arg = [n];
            let heap = {
                use logicaffeine_compile::codegen_sva::wprog_engine_rtl::eval_wprog_config_counted_heap;
                eval_wprog_config_counted_heap(&cfg, &arg).2.len()
            };
            let tag = format!("{name}_n{n}");
            match big_engine_run(&tag, MemBackend::Registers, &cfg, &arg) {
                Ok(r) => eprintln!("{name:<10} n={n:<3} heap={heap:<7} RTL≡VM -> {r}"),
                Err(why) => eprintln!("{name:<10} n={n:<3} heap={heap:<7} DIVERGE: {why}"),
            }
        }
    }
}

/// The 64-bit heap words a program's RUNTIME may use on the A7 config-BRAM engine: the chip's ~77K BRAM words
/// (135 RAMB36 × 512) minus the config image + register file + `v` scratch + frame stack (~20 RAMB36 at all-35
/// caps). Conservative — refine once the CFG_BRAM engine's non-heap BRAM is measured by the fit-check synth.
const SILICON_HEAP_BUDGET: usize = 49152;

/// SILICON DEMO MANIFEST: for each benchmark, the LARGEST test arg whose eval heap footprint fits the on-chip BRAM
/// heap budget — the representative demo input to stream over UART on the A7 (the whole point of "all 35 on
/// silicon"). A benchmark whose smallest arg still overflows is flagged (needs heap reuse in the lowering, or a
/// smaller sub-test input). The derived manifest is the committed `benchmarks/silicon/programs/
/// bench_silicon_manifest.txt` (`scripts/test/gen-silicon-goldens.sh demo-manifest` writes it). Eval-only, fast.
#[test]
#[ignore]
fn silicon_demo_manifest() {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::eval_wprog_config_counted_heap;
    let mut manifest = String::from("# benchmark  args  oracle  heap_words   (demo inputs sized to the A7 on-chip heap budget)\n");
    let (mut fit, mut nofit) = (0u32, 0u32);
    for b in BENCHMARKS {
        let Some(cfg) = config_for(b) else {
            eprintln!("NOLOW  {:<14}", b.name);
            continue;
        };
        let choice = b
            .args
            .iter()
            .copied()
            .map(|a| (a, eval_wprog_config_counted_heap(&cfg, a).2.len()))
            .filter(|(_, h)| *h <= SILICON_HEAP_BUDGET)
            .max_by_key(|(a, _)| a[0]);
        match choice {
            Some((arg, heap)) => {
                let oracle = eval_wprog_config(&cfg, arg);
                let args = arg
                    .iter()
                    .map(|x| x.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                manifest.push_str(&format!("{} {} {} {}\n", b.name, args, oracle, heap));
                eprintln!(
                    "FIT   {:<14} arg={arg:?} heap={heap:<6} -> {oracle}",
                    b.name
                );
                fit += 1;
            }
            None => {
                let minh = b
                    .args
                    .iter()
                    .map(|a| eval_wprog_config_counted_heap(&cfg, a).2.len())
                    .min()
                    .unwrap_or(0);
                eprintln!("NOFIT {:<14} smallest-arg heap {minh} > budget {SILICON_HEAP_BUDGET} (needs heap reuse)", b.name);
                nofit += 1;
            }
        }
    }
    golden::committed_bytes(
        "benchmarks/silicon/programs/bench_silicon_manifest.txt",
        manifest.as_bytes(),
        "scripts/test/gen-silicon-goldens.sh demo-manifest",
    );
    eprintln!(
        "\n=== {fit}/{} benchmarks fit the ~{}K-word on-chip heap at a demo arg; {nofit} need heap reuse ===",
        fit + nofit,
        SILICON_HEAP_BUDGET / 1024
    );
}

/// ISOLATES the one RTL≡VM divergence the full-caps survey reports: `analytics`.
///
/// The survey runs each benchmark at a single arg, so it says "analytics diverges" without saying
/// what the divergence depends on. This runs the SAME program at every demo arg, which is what turns
/// a one-line survey row into a characterised bug:
///
///   • It diverges at EVERY arg — 1, 10, 50 and 100 — so it is not write-count dependent. `arg` is
///     only the loop bound `n`; the 32768-bucket table is built and summed at every arg.
///   • The error is a CONSTANT, independent of the data: the engine's checksum is exactly
///     73,741,817 too high at all four args. That number is `32768 * 32768 mod 1000000007`.
///   • The heap is 32769 words at every arg, laid out as the length at `heap[0]` and the elements at
///     `heap[1..=32768]`. Index 0 is the ONLY word holding the value 32768 — so the arithmetic is
///     consistent with element 32768 reading back the length word instead of its own value.
///
/// The model is NOT implicated: `config_for` refuses a config unless `eval_wprog_config == vm` at
/// EVERY arg, so model ≡ VM is established before this runs. Only the emitted RTL disagrees.
///
/// RULED OUT so far, each by a program that does NOT reproduce: a plain push-then-read of a
/// 32768-element Seq (heap 32769) is correct on the engine, and so is push, indexed-write through
/// `Set item … of`, then read. So neither the heap crossing 2^15 words nor the indexed-write path is
/// sufficient on its own — the trigger needs more of analytics' structure (its read-modify-write
/// accumulation into a bucket, or its wider arithmetic). A bisect that rewrites analytics' `32768`
/// literals does NOT isolate this, because those literals are also the program's moduli.
///
/// `#[ignore]` — iverilog, one build+sim per arg.
#[test]
#[ignore]
fn analytics_rtl_equals_vm_at_every_arg() {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::eval_wprog_config_counted_heap;
    let b = BENCHMARKS
        .iter()
        .find(|b| b.name == "analytics")
        .expect("the analytics benchmark");
    let cfg =
        config_for(b).expect("analytics lowers, and its model already matches the VM at every arg");
    let mut bad = Vec::new();
    for arg in b.args {
        let heap = eval_wprog_config_counted_heap(&cfg, arg).2.len();
        match big_engine_run(
            &format!("analytics_a{}", arg[0]),
            MemBackend::Registers,
            &cfg,
            arg,
        ) {
            Ok(r) => eprintln!("RTL≡VM analytics arg={arg:?} heap={heap} -> {r}"),
            Err(why) => {
                eprintln!("DIVERGE analytics arg={arg:?} heap={heap}: {why}");
                bad.push(format!("  arg={arg:?} heap={heap}: {why}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "analytics diverges from the VM on the RTL engine at {} of {} args. The model matches the VM at \
         every one of them, so this is an ENGINE bug, and the constant offset says where to look:\n{}",
        bad.len(),
        b.args.len(),
        bad.join("\n")
    );
}

/// SCRATCH: bisect the boundary with data that cannot mask the bug (all zeros => expected 0).
#[test]
#[ignore]
fn zz_bisect_zero() {
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::eval_wprog_config_counted_heap;
    for n in [4096usize, 8192, 16384, 16385, 32766, 32767, 32768] {
        let src = format!(
            "## To f (q: Int) -> Int:\n    Let mutable xs be a new Seq of Int.\n    Let mutable i be 0.\n    \
             While i is less than {n}:\n        Push 0 to xs.\n        Set i to i + 1.\n    \
             Let mutable acc be 0.\n    Let mutable g be 0.\n    While g is less than {n}:\n        \
             Set acc to (acc + (g + 1) * (item (g + 1) of xs)) % 1000000007.\n        Set g to g + 1.\n    Return acc.\n"
        );
        let cfg = serialize_program(&program_cfg_fabric(&src, "f"));
        let arg: &[i64] = &[1];
        let heap = eval_wprog_config_counted_heap(&cfg, arg).2.len();
        match big_engine_run(&format!("bz_{n}"), MemBackend::Registers, &cfg, arg) {
            Ok(r) => eprintln!("BZ n={n:<6} heap={heap:<6} RTL≡VM -> {r}"),
            Err(w) => eprintln!("BZ n={n:<6} heap={heap:<6} DIVERGE {w}"),
        }
    }
}

/// The benchmarks `the_caravel_engine_runs_exactly_the_benchmarks_its_profile_covers` measures as fitting the shipped
/// Caravel profile, in suite order.
const CARAVEL_RUNS: &[&str] = &[
    "loop_sum",
    "fib_iterative",
    "collatz",
    "primes",
    "vm",
    "fib",
    "gcd",
    "ackermann",
    "binary_trees",
    "nqueens",
    "array_fill",
    "array_reverse",
    "prefix_sum",
    "bubble_sort",
    "counting_sort",
    "histogram",
    "matrix_mult",
    "sieve",
    "coins",
    "graph_bfs",
    "pi_leibniz",
    "mandelbrot",
    "two_sum",
    "collect",
];

/// The benchmarks the fabricated whole-program engine runs: every program whose demands — each table it loads,
/// and the frame depth and heap words its declared inputs reach — fit the shipped Caravel profile
/// (`wpe_caravel::WPE_CARAVEL_CAPS`), with the dimension that stops every other one. A program outside the profile
/// is refused at load by the host's fit-check or trapped on die (`0xE1` depth, `0xE2` heap), never answered wrong,
/// so this list is exactly what the chip computes. The WPE submission's README states it, and this holds the two
/// to each other.
#[test]
fn the_caravel_engine_runs_exactly_the_benchmarks_its_profile_covers() {
    use logicaffeine_compile::codegen_sva::caravel_project::WPE_PROJECT;
    use logicaffeine_compile::codegen_sva::wpe_caravel::WPE_CARAVEL_CAPS;
    use logicaffeine_compile::codegen_sva::wpe_floorplan::{WpeDemands, WpeProfile};

    let profile = WpeProfile::from_caps(WPE_CARAVEL_CAPS);
    let mut runs = Vec::new();
    let mut table = Vec::new();
    for b in BENCHMARKS {
        let Some(cfg) = config_for(b) else {
            table.push(format!("  {:<18} does not lower", b.name));
            continue;
        };
        let d = catch_unwind(AssertUnwindSafe(|| WpeDemands::of(&cfg, b.args)))
            .unwrap_or_else(|_| panic!("{}: the model could not run its declared inputs", b.name));
        let short = profile.shortfalls(&d);
        if short.is_empty() {
            runs.push(b.name);
            table.push(format!("  {:<18} RUNS", b.name));
        } else {
            let why: Vec<String> = short
                .iter()
                .map(|(dim, need, cap)| format!("{dim} {need} > {cap}"))
                .collect();
            table.push(format!("  {:<18} refused: {}", b.name, why.join(", ")));
        }
    }
    println!(
        "the Caravel engine's profile against the suite:\n{}",
        table.join("\n")
    );
    assert_eq!(
        runs,
        CARAVEL_RUNS,
        "the programs the Caravel profile covers changed:\n{}",
        table.join("\n")
    );
    let readme = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(WPE_PROJECT.overlay)
            .join("README.md"),
    )
    .expect("the WPE submission README");
    let claim = format!("runs {} of the {} programs", runs.len(), BENCHMARKS.len());
    assert!(
        readme.contains(&claim),
        "the WPE README must say the chip `{claim}`"
    );
    for name in &runs {
        assert!(
            readme.contains(&format!("`{name}`")),
            "the WPE README must name `{name}` among the programs it runs"
        );
    }
}
