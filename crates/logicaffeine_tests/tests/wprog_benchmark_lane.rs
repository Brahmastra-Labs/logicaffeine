//! wprog_benchmark_lane — the whole-program engine RUNS THE BENCHMARKS. The bounded spatial fabric refuses
//! recursion, Main-only loops, and stateful/collection kernels (they aren't spatially unrollable); the
//! SEQUENCED whole-program engine (`eval_wprog_config` — register file + block walk + heap state plane + frame
//! stack + FLOAT LANE, all proven ≡ VM and lowered to RTL) runs them. This lane lowers real benchmark
//! algorithms — GCD, Fibonacci, Collatz, sum-of-squares, recursive factorial/sum, an array (collection) kernel,
//! AND the floating-point kernels (Leibniz π, a Mandelbrot escape-count, an `sqrt` reduction) — through the
//! whole-program lowering and proves each runs bit-exactly vs the VM across many inputs.
//!
//! The float benchmarks exercise the whole float lane end-to-end: `f64`-bit-pattern add/sub/mul/div/sqrt/floor,
//! the INT→FLOAT promotion that mirrors the VM's implicit `Int ∘ Float` coercion (`2.0 * k`), and genuine
//! IEEE-754 float comparisons (Mandelbrot's `zr² + zi² > 4.0` escape test) — not the integer bit-pattern
//! ordering a plain compare would use. Each returns an `Int` (a floored/scaled result) so the VM oracle is an
//! exact integer to diff against.
//!
//! Run: `cargo test -p logicaffeine-tests --test wprog_benchmark_lane`

#![cfg(not(target_arch = "wasm32"))]

use std::panic::{catch_unwind, AssertUnwindSafe};

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, program_calls_fabric, serialize_program,
};
use logicaffeine_compile::compile::vm_outcome;

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

/// Does the benchmark function run on the whole-program engine (block-walk or call-stack) ≡ the VM, for every
/// input? Loops/collections lower via `program_cfg_fabric`; recursion via the `program_calls_fabric` call stack.
fn runs_on_engine(src: &str, args_sets: &[&[i64]]) -> bool {
    let all = |f: &dyn Fn(&[i64]) -> i64| -> bool {
        catch_unwind(AssertUnwindSafe(|| {
            args_sets
                .iter()
                .all(|a| matches!(vm_f(src, a), Some(vm) if f(a) == vm))
        }))
        .unwrap_or(false)
    };
    all(&|a| eval_wprog_config(&serialize_program(&program_cfg_fabric(src, "f")), a))
        || all(&|a| eval_wprog_config(&program_calls_fabric(src, "f"), a))
}

struct Bench {
    name: &'static str,
    src: &'static str,
    args: &'static [&'static [i64]],
}

/// Real benchmark algorithms, as parameterized functions the whole-program engine runs. Each is a kernel the
/// SPATIAL fabric refuses (unbounded loop / recursion / heap state) but the sequenced engine handles.
const BENCHMARKS: &[Bench] = &[
    // GCD (Euclid) — the `gcd` benchmark's while-loop, refused by the spatial fabric (unbounded).
    Bench {
        name: "gcd",
        src: "## To f (a: Int, b: Int) -> Int:\n    Let mutable x be a.\n    Let mutable y be b.\n    While y is greater than 0:\n        Let t be y.\n        Set y to x % y.\n        Set x to t.\n    Return x.\n",
        args: &[&[1071, 462], &[48, 18], &[17, 5], &[100, 100], &[13, 1]],
    },
    // Fibonacci (iterative) — the `fib` benchmark's accumulate loop.
    Bench {
        name: "fib",
        src: "## To f (n: Int) -> Int:\n    Let mutable a be 0.\n    Let mutable b be 1.\n    Let mutable i be n.\n    While i is greater than 0:\n        Let next be a + b.\n        Set a to b.\n        Set b to next.\n        Set i to i - 1.\n    Return a.\n",
        args: &[&[0], &[1], &[10], &[20], &[30]],
    },
    // Collatz step-count — the `collatz` benchmark's if/else loop with division and multiply.
    Bench {
        name: "collatz",
        src: "## To f (n0: Int) -> Int:\n    Let mutable n be n0.\n    Let mutable steps be 0.\n    While n is greater than 1:\n        If n % 2 is equal to 0:\n            Set n to n / 2.\n        Otherwise:\n            Set n to 3 * n + 1.\n        Set steps to steps + 1.\n    Return steps.\n",
        args: &[&[1], &[6], &[27], &[97], &[871]],
    },
    // Sum of squares — the `sumsq` benchmark's multiply-accumulate loop.
    Bench {
        name: "sumsq",
        src: "## To f (n: Int) -> Int:\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set total to total + i * i.\n        Set i to i + 1.\n    Return total.\n",
        args: &[&[1], &[4], &[10], &[25], &[100]],
    },
    // Recursive factorial — a recursion benchmark (call stack), refused by the spatial fabric.
    Bench {
        name: "factorial-recursive",
        src: "## To f (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * f(n - 1).\n",
        args: &[&[1], &[3], &[5], &[8], &[10]],
    },
    // Recursive sum-down — self-recursion accumulating over the frame stack.
    Bench {
        name: "sumdown-recursive",
        src: "## To f (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + f(n - 1).\n",
        args: &[&[0], &[5], &[10], &[50], &[100]],
    },
    // Array (collection) kernel — build a list, sum it: the heap state plane, refused by the spatial fabric.
    Bench {
        name: "array-sum",
        src: "## To f (x: Int) -> Int:\n    Let xs be [x, 10, 20, 30].\n    Return item 1 of xs + item 2 of xs + item 3 of xs + item 4 of xs.\n",
        args: &[&[0], &[5], &[40], &[7], &[100]],
    },
    // Prime counting — the `primes` benchmark's NESTED loop (trial division), doubly-unbounded.
    Bench {
        name: "primes-count",
        src: "## To f (n: Int) -> Int:\n    Let mutable count be 0.\n    Let mutable i be 2.\n    While i is at most n:\n        Let mutable isprime be 1.\n        Let mutable j be 2.\n        While j is less than i:\n            If i % j is equal to 0:\n                Set isprime to 0.\n            Set j to j + 1.\n        Set count to count + isprime.\n        Set i to i + 1.\n    Return count.\n",
        args: &[&[2], &[10], &[30], &[50]],
    },
    // Array max — a loop over a list with an indexed compare (collection + control flow).
    Bench {
        name: "array-max",
        src: "## To f (x: Int) -> Int:\n    Let xs be [x, 10, 20, 5].\n    Let mutable m be item 1 of xs.\n    Let mutable i be 2.\n    While i is at most 4:\n        If item i of xs is greater than m:\n            Set m to item i of xs.\n        Set i to i + 1.\n    Return m.\n",
        args: &[&[0], &[25], &[100], &[15]],
    },
    // Exponentiation by iterated multiply — the multiply-accumulate loop kernel.
    Bench {
        name: "power",
        src: "## To f (a: Int, b: Int) -> Int:\n    Let mutable r be 1.\n    Let mutable i be 0.\n    While i is less than b:\n        Set r to r * a.\n        Set i to i + 1.\n    Return r.\n",
        args: &[&[2, 5], &[3, 3], &[5, 2], &[7, 0]],
    },
    // ── FLOAT LANE ────────────────────────────────────────────────────────────────────────────────────────
    // Leibniz π — the `pi_leibniz` benchmark's alternating float series (`sum += sign / (2k+1)`), scaled and
    // floored to an Int. Exercises float add/sub/mul/div + the INT→FLOAT promotion of the loop counter `k`.
    Bench {
        name: "pi_leibniz",
        src: "## To f (n: Int) -> Int:\n    Let mutable sum be 0.0.\n    Let mutable sign be 1.0.\n    Let mutable k be 0.\n    While k is less than n:\n        Set sum to sum + sign / (2.0 * k + 1.0).\n        Set sign to 0.0 - sign.\n        Set k to k + 1.\n    Let scaled be sum * 4.0 * 1000000.0.\n    Return floor(scaled).\n",
        args: &[&[0], &[1], &[2], &[5], &[10], &[50], &[100]],
    },
    // Mandelbrot escape-count — the `mandelbrot` benchmark's core iteration (`z ← z² + c`) with the genuine
    // IEEE-754 escape test `zr² + zi² > 4.0` (a FLOAT comparison, not integer bit-pattern ordering). `px`
    // scans the real axis; the result is the escape iteration (capped at 100).
    Bench {
        name: "mandelbrot",
        src: "## To f (px: Int) -> Int:\n    Let cr be 0.01 * px - 2.0.\n    Let ci be 0.5.\n    Let mutable zr be 0.0.\n    Let mutable zi be 0.0.\n    Let mutable i be 0.\n    While i is less than 100:\n        Let zr2 be zr * zr - zi * zi + cr.\n        Let zi2 be 2.0 * zr * zi + ci.\n        Set zr to zr2.\n        Set zi to zi2.\n        If zr * zr + zi * zi is greater than 4.0:\n            Return i.\n        Set i to i + 1.\n    Return i.\n",
        args: &[&[0], &[50], &[100], &[150], &[200], &[250], &[300], &[399]],
    },
    // `sqrt` reduction — the float square-root functional unit (`spectral_norm`/`nbody` both end in `sqrt`).
    // `floor(sqrt(x) · 1000)` gives an exact integer oracle over the perfect and non-perfect squares.
    Bench {
        name: "sqrt-reduce",
        src: "## To f (x: Int) -> Int:\n    Let r be sqrt(x * 1.0).\n    Return floor(r * 1000.0).\n",
        args: &[&[0], &[1], &[2], &[4], &[9], &[50], &[100]],
    },
];

#[test]
fn the_whole_program_engine_runs_the_benchmarks() {
    let mut ran = Vec::new();
    let mut refused = Vec::new();
    for b in BENCHMARKS {
        // Anchor: the VM must run every input (the oracle exists) before we claim the engine matches it.
        for a in b.args {
            assert!(
                vm_f(b.src, a).is_some(),
                "benchmark `{}` VM error at {a:?}",
                b.name
            );
        }
        if runs_on_engine(b.src, b.args) {
            ran.push(b.name);
        } else {
            refused.push(b.name);
        }
    }
    println!(
        "whole-program engine BENCHMARK lane: {}/{} benchmark kernels run ≡VM (loops + recursion + \
         collections — the categories the spatial fabric refuses):\n  RAN: {ran:?}",
        ran.len(),
        BENCHMARKS.len()
    );
    assert!(
        refused.is_empty(),
        "these benchmark kernels did NOT run on the whole-program engine ≡VM: {refused:?}"
    );
}
