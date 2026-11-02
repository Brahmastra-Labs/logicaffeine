//! soft_core_width_parity — the chip differs NOWHERE from the VM on integers.
//!
//! The soft-core's integer datapath was fixed-width and WRAPPED where the VM's unbounded `Int` PROMOTES to
//! BigInt — so a value exceeding the datapath printed differently. That divergence is now CLOSED at the
//! source: `run_soft_core_typed_w`'s value registers are auto-promoting (a fast `Small(i64)` path + an
//! exact `Big(BigInt)` fallback on overflow, mirroring `RuntimeValue::Int`/`BigInt`), so an `Int` is
//! UNBOUNDED exactly like the VM's. The benchmark PRNGs (`(seed * 1103515245 + 12345) % 2147483648` in
//! `analytics`, `% 4294967296` in `bubble_sort`) whose products overflow 32 bits and whose moduli exceed
//! `i32::MAX` now compute EXACTLY, at ANY width.
//!
//! This is a REFRAME of the old width toggle: the width no longer causes a correctness divergence — it is a
//! perf/area knob for the fast path (and, in the encoder, chooses the constant-encoding form). Both
//! `run_soft_core_typed_w(_, _, 32)` and `(_, _, 64)` now print EXACTLY what `vm_outcome` prints, compared
//! by the EXACT decimal (`ShowVal::fmt`), with NO `mod 2^64` and NO range dodge anywhere. The width-64
//! encoding still emits the 3-word `LoadConstWide64` (chip op 222) for an over-i32 constant where the
//! 32-bit encoding uses the 2-word form (an encoder distinction the analytics/bubble_sort lock still pins).

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words_w;
use logicaffeine_compile::codegen_sva::soft_core_model::{run_soft_core_typed_w, ShowVal};
use logicaffeine_compile::compile::vm_outcome;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

/// Does one soft-core `Show` equal the VM's corresponding output line — EXACTLY? The chip's integer datapath
/// is now UNBOUNDED (an `Int` auto-promotes to `BigInt` on overflow, exactly like the VM), so the chip differs
/// NOWHERE from the VM on integers: the honest comparison is the EXACT rendered decimal (`ShowVal::fmt`), NOT
/// `mod 2^64`. Every `Show` type — integer (Small or BigInt), TEXT (string interpolation), Word32 (SHA hash
/// words, unsigned), Float — compares by its exact `fmt` form, byte-identical to the VM's `Show`. This is the
/// TRUE differential over the whole output tape, with no range dodge anywhere.
fn show_matches(sv: &ShowVal, vm_line: &str) -> bool {
    sv.fmt() == vm_line
}

/// A large cycle budget — the baked benchmarks run their full PRNG + bubble-sort loops on the model.
const CYCLES: usize = 500_000_000;

fn programs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

/// Read `benchmarks/programs/<name>/main.lg` and BAKE a fixed input in place of the CLI preamble: drop the
/// two `## To native args`/`## To native parseInt` declaration lines, delete the `Let arguments be args().`
/// line, and rewrite `Let n be parseInt(item 2 of arguments).` to `Let n be <n>.` — so the program becomes
/// a self-contained, argument-free `.lg` the encoder and the VM run identically. Every other line is kept
/// verbatim (the arithmetic under test is untouched).
fn bake(name: &str, n: u64) -> String {
    let path = programs_dir().join(name).join("main.lg");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    const PARSE_SUFFIX: &str = " be parseInt(item 2 of arguments).";
    let mut out: Vec<String> = Vec::new();
    let mut spliced = false;
    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("## To native args") || t.starts_with("## To native parseInt") {
            continue; // drop the native-decl lines (no chip/VM meaning once the input is baked)
        }
        if t == "Let arguments be args()." {
            continue; // no argv — the input is baked below
        }
        // `Let <var> be parseInt(item 2 of arguments).` for ANY variable name (n, limit, …) → `Let <var> be N.`
        if let Some(var) = t
            .strip_prefix("Let ")
            .and_then(|r| r.strip_suffix(PARSE_SUFFIX))
        {
            out.push(format!("Let {var} be {n}."));
            spliced = true;
            continue;
        }
        out.push(line.to_string());
    }
    let baked = out.join("\n");
    // The bake must leave no trace of the CLI preamble — a stray `args()`/`parseInt`/`arguments` would make
    // the program non-self-contained and silently change what we compare.
    assert!(
        !baked.contains("args()") && !baked.contains("parseInt") && !baked.contains("arguments"),
        "{name}: bake left a CLI-preamble reference behind:\n{baked}"
    );
    assert!(
        spliced,
        "{name}: no `Let <var> be parseInt(item 2 of arguments).` line to bake the input into"
    );
    baked
}

/// The width-64 parity check for one baked benchmark: encode at width 64, run the fabric at width 64, and
/// assert each fabric `Show` equals the corresponding VM `Show` EXACTLY (`ShowVal::fmt` == the VM line — no
/// `mod 2^64`, the chip's `Int` is unbounded like the VM's). Returns the VM output lines so the caller can
/// also probe the width-32 behaviour.
#[track_caller]
fn assert_width64_matches_vm(name: &str, baked: &str) -> Vec<String> {
    let words = compile_to_full_words_w(baked, 64)
        .unwrap_or_else(|e| panic!("{name}: width-64 encode failed: {e}"));
    let fabric = run_soft_core_typed_w(&words, CYCLES, 64);

    let vm = vm_outcome(baked);
    assert!(vm.error.is_none(), "{name}: VM errored: {:?}", vm.error);
    let vm_lines: Vec<String> = vm.output.trim().lines().map(str::to_string).collect();

    assert_eq!(
        fabric.len(),
        vm_lines.len(),
        "{name}: fabric emitted {} Shows, VM emitted {} — {:?} vs {vm_lines:?}",
        fabric.len(),
        vm_lines.len(),
        fabric.iter().map(ShowVal::fmt).collect::<Vec<_>>(),
    );
    for (idx, (fab, vmline)) in fabric.iter().zip(vm_lines.iter()).enumerate() {
        assert!(
            show_matches(fab, vmline),
            "{name}: Show #{idx} diverges at width 64 — fabric {:?} != VM {vmline:?}",
            fab.fmt(),
        );
    }
    vm_lines
}

/// The benchmarks the width-64 soft-core runs EXACTLY like the VM (`== VM mod 2^64`, NO range dodge). Every
/// benchmark that LOWERS on the full core matches — there are ZERO correctness divergences; the width toggle
/// closed the last one (the LCG overflow). Grow this set as the encoder lowers more ISA (a `FRONTIER` benchmark
/// that starts running must move HERE or the lock goes RED — the pit-of-success ratchet).
const MATCH_AT_64: &[&str] = &[
    "ackermann",
    "analytics",
    "array_fill",
    "array_reverse",
    "binary_trees",
    "bubble_sort",
    "coins",
    "collatz",
    "collect",
    "counting_sort",
    "fannkuch",
    "fib",
    "fib_iterative",
    "gcd",
    "graph_bfs",
    "heap_sort",
    "histogram",
    "knapsack",
    "loop_sum",
    "mandelbrot",
    "matrix_mult",
    "mergesort",
    "nbody",
    "nqueens",
    "pi_leibniz",
    "prefix_sum",
    "primes",
    "quicksort",
    "sha256",
    "sieve",
    "spectral_norm",
    "string_search",
    "strings",
    "two_sum",
    "vm",
];

/// The ISA-LOWERING FRONTIER — now CLOSED (empty). The last three benchmarks (nbody, pi_leibniz,
/// spectral_norm) formatted a Float result with a `.N` precision spec (`Show "{result:.15}"`), the one op
/// (`FormatValue` on a float) the full core did not lower; they now emit the chip-synthetic
/// `FormatValueFloat` (op 225 — `fregs[src]` rendered to N decimals EXACTLY as the VM's `apply_format_spec`,
/// then a `ShowText`), so the width-64 soft-core matches the VM on the WHOLE benchmark suite: 35/35. A new
/// benchmark using an unlowered op would land HERE (the set-equality guard forces every dir into exactly one
/// set); until then the frontier stays empty.
const FRONTIER: &[&str] = &[];

/// FRONTIER-op SURVEY (temporary): for each FRONTIER benchmark, report the FIRST blocking op — the encoder
/// error (which names the unlowered `Op`) or the run-loop panic. Guides the systematic lowering to 35/35.
#[test]
#[ignore = "survey — run with --ignored to list each frontier benchmark's blocking op"]
fn survey_frontier_blocking_ops() {
    for name in FRONTIER {
        let reason = catch_unwind(AssertUnwindSafe(|| {
            let baked = bake(name, 6);
            match compile_to_full_words_w(&baked, 64) {
                Err(e) => format!("ENCODE-ERR: {}", e.chars().take(90).collect::<String>()),
                Ok(words) => {
                    let vm = vm_outcome(&baked);
                    if let Some(e) = vm.error {
                        return format!(
                            "VM-ERR: {}",
                            e.to_string().chars().take(50).collect::<String>()
                        );
                    }
                    let fabric = run_soft_core_typed_w(&words, CYCLES, 64);
                    let vm_lines: Vec<String> =
                        vm.output.trim().lines().map(str::to_string).collect();
                    if fabric.len() == vm_lines.len()
                        && fabric
                            .iter()
                            .zip(&vm_lines)
                            .all(|(f, v)| show_matches(f, v))
                    {
                        "NOW-MATCHES (promote!)".to_string()
                    } else {
                        let fab: Vec<String> = fabric.iter().take(3).map(ShowVal::fmt).collect();
                        format!(
                            "MISMATCH fab={fab:?} vm={:?}",
                            &vm_lines[..vm_lines.len().min(3)]
                        )
                    }
                }
            }
        }))
        .unwrap_or_else(|e| {
            let m = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            format!(
                "PANIC: {}",
                m.lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect::<String>()
            )
        });
        println!("{name:16} {reason}");
    }
}

/// THE ALL-BENCHMARK DIFFERENTIAL LOCK — the chip and the VM locked together over the WHOLE benchmark suite.
/// Three guards make it a ratchet that can't be gamed: (1) SET-EQUALITY — every benchmark directory is in
/// exactly one of `MATCH_AT_64`/`FRONTIER` (a new benchmark, or a mis-filed one, goes RED); (2) every
/// `MATCH_AT_64` benchmark's width-64 soft-core run equals the VM `mod 2^64` with NO range restriction (a
/// regression that makes the chip disagree with the VM goes RED); (3) every `FRONTIER` benchmark genuinely does
/// NOT run to a VM-matching result (so a benchmark can't be parked on the frontier once it actually works — it
/// must be claimed as a MATCH). Together: the width-64 chip is proven interchangeable with the VM on every
/// program it can execute, and the exact op-coverage frontier is pinned.
#[test]
fn every_benchmark_is_locked_to_the_vm_at_width_64() {
    const N: u64 = 6; // small so the factorial/permutation benchmarks (fannkuch, nqueens) finish in the budget

    // (1) SET-EQUALITY: MATCH ∪ FRONTIER == every benchmark dir, disjoint.
    let mut all: Vec<String> = std::fs::read_dir(programs_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    all.sort();
    let mut declared: Vec<String> = MATCH_AT_64
        .iter()
        .chain(FRONTIER)
        .map(|s| s.to_string())
        .collect();
    declared.sort();
    assert_eq!(
        all, declared,
        "benchmark set drift: every benchmark must be classified in exactly one of MATCH_AT_64 / FRONTIER"
    );
    for name in MATCH_AT_64 {
        assert!(
            !FRONTIER.contains(name),
            "{name} is in BOTH MATCH_AT_64 and FRONTIER"
        );
    }

    // (2) every MATCH benchmark: the width-64 soft-core equals the VM, mod 2^64, no dodge.
    for name in MATCH_AT_64 {
        let baked = bake(name, N);
        assert_width64_matches_vm(name, &baked);
    }

    // (3) every FRONTIER benchmark: does NOT run to a VM-matching result. The WHOLE check (bake included) is
    //     caught — a bake that can't strip the CLI preamble, an encode error, a run panic (an unlowered op), a
    //     text/float Show the integer path can't read, or a value mismatch ALL mean "doesn't run to a matching
    //     result", i.e. genuinely on the ISA-lowering frontier. The frontier benchmarks are EXPECTED to panic,
    //     so silence the panic hook while probing them (restored after) — the caught panics are the signal.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let unexpectedly_matched: Vec<&str> = FRONTIER
        .iter()
        .filter(|name| {
            catch_unwind(AssertUnwindSafe(|| {
                let baked = bake(name, N);
                let words = match compile_to_full_words_w(&baked, 64) {
                    Ok(w) => w,
                    Err(_) => return false, // doesn't encode (an ISA op the full core can't lower)
                };
                let vm = vm_outcome(&baked);
                if vm.error.is_some() {
                    return false;
                }
                let vm_lines: Vec<String> = vm.output.trim().lines().map(str::to_string).collect();
                let fabric = run_soft_core_typed_w(&words, CYCLES, 64);
                fabric.len() == vm_lines.len()
                    && fabric
                        .iter()
                        .zip(vm_lines.iter())
                        .all(|(f, v)| show_matches(f, v))
            }))
            .unwrap_or(false) // any panic → doesn't run to a matching result
        })
        .copied()
        .collect();
    std::panic::set_hook(prev_hook);
    assert!(
        unexpectedly_matched.is_empty(),
        "these FRONTIER benchmarks now MATCH the VM at width 64 — promote them to MATCH_AT_64: {unexpectedly_matched:?}"
    );

    println!(
        "All-benchmark parity: {}/{} run on the width-64 soft-core == VM mod 2^64 (ZERO divergences) — the \
         ISA-lowering frontier is CLOSED ({} remaining). Float `Show \"{{f:.N}}\"` now lowers via op 225 \
         (FormatValueFloat), so nbody/pi_leibniz/spectral_norm reach parity too.",
        MATCH_AT_64.len(),
        all.len(),
        FRONTIER.len()
    );
}

/// THE LOCK: `analytics` (`% 2147483648`, i.e. `% 2³¹`) and `bubble_sort` (`% 4294967296`, i.e. `% 2³²`) —
/// the two benchmarks whose PRNGs overflow the datapath and use an over-i32 modulus — differ NOWHERE from the
/// VM at EVERY width. This is the concrete proof that the width is a fast-path/area knob, not a correctness
/// switch: BigInt handles exactness.
///
/// Two invariants close the old 32-bit divergence completely:
///   • The value datapath auto-promotes to BigInt on overflow, so `seed * 1103515245 ≈ 2⁶¹` and the modulo
///     are computed unbounded like the VM's `Int` — the arithmetic wraps NOWHERE, at width 32 or 64.
///   • Over-i32 constants (the `2³¹` / `2³²` moduli) are carried in FULL by op 222 at EVERY width, so the
///     modulus is never truncated (`2³²` no longer collapses to `% 0`). The encoded image is therefore
///     width-independent, and the run reproduces the VM byte-for-byte at both widths.
#[test]
fn analytics_and_bubble_sort_differ_nowhere_at_every_width() {
    const N: u64 = 12;

    for name in ["analytics", "bubble_sort"] {
        let baked = bake(name, N);

        // (1) Width 64 = EXACT PARITY: the fabric prints what the VM prints, byte-for-byte.
        let vm_lines = assert_width64_matches_vm(name, &baked);

        let w32 = compile_to_full_words_w(&baked, 32)
            .unwrap_or_else(|e| panic!("{name}: width-32 encode failed: {e}"));
        let w64 = compile_to_full_words_w(&baked, 64)
            .unwrap_or_else(|e| panic!("{name}: width-64 encode failed: {e}"));

        // (2) The two encodings are now IDENTICAL: the over-i32 modulus is carried by the same 3-word
        //     LoadConstWide64 (op 222) at both widths, so the width no longer changes the image. BigInt
        //     subsumed the old constant-encoding toggle — the full-core image is width-independent.
        assert_eq!(
            w32, w64,
            "{name}: width-32 and width-64 encodings differ — the over-i32 constant must be carried by op \
             222 at BOTH widths (the full-core image is width-independent now that BigInt handles exactness)"
        );
        assert!(
            w64.iter().any(|word| (word >> 24) as u8 == 222),
            "{name}: encoding contains no LoadConstWide64 (op 222) — the over-i32 modulus was not emitted \
             as a full 64-bit constant"
        );

        // (3) The width-32 fabric run ALSO differs NOWHERE: with the auto-promoting datapath AND the full
        //     constant, width 32 reproduces the VM byte-for-byte, exactly as width 64 does. There is no
        //     residual 32-bit divergence — neither analytics' `% 2³¹` nor bubble_sort's `% 2³²`.
        let fab32 = run_soft_core_typed_w(&w32, CYCLES, 32);
        assert!(
            fab32.len() == vm_lines.len()
                && fab32
                    .iter()
                    .zip(vm_lines.iter())
                    .all(|(f, v)| show_matches(f, v)),
            "{name}: width-32 fabric diverges from the VM — differ-nowhere broken at width 32 \
             (fabric32 = {:?}, vm = {vm_lines:?})",
            fab32.iter().map(ShowVal::fmt).collect::<Vec<_>>(),
        );
    }
}

/// A tiny splitmix64 so the BigInt fuzz is deterministic (no `rand` dep, no flake). Same generator the
/// universal_completeness_lock fuzzes use.
fn mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// THE BIGINT EXACTNESS FUZZ — the proof the chip differs NOWHERE from the VM on integers, INCLUDING the
/// over-i64 range. It fuzzes programs that deliberately produce BIGINT results — a full-range `i64` product
/// (~2^126, a 38-digit integer), a factorial-to-N loop (100! is a 158-digit integer), and repeated squaring
/// — and asserts the soft-core's output is BYTE-IDENTICAL to `vm_outcome` (`ShowVal::fmt` == the VM's `Show`
/// line), with NO `mod`, NO range dodge. It runs at BOTH width 32 and width 64: the value datapath is now
/// unbounded (auto-promoting to `BigInt` on overflow, exactly like the VM's `Int`), so the WIDTH is a
/// perf/area knob, not a correctness switch — both widths equal the VM exactly. This test was RED before the
/// BigInt-fallback change (the chip wrapped where the VM promoted) and is GREEN after.
#[test]
fn chip_equals_vm_exactly_over_bigint_fuzz() {
    // Each fuzzed program is self-contained (no CLI preamble) — the encoder and VM run it identically — and
    // deliberately overflows i64 so the result is a genuine BigInt whose exact decimal both must print.
    fn build(seed: u64) -> String {
        let mut st = seed;
        match mix(&mut st) % 5 {
            // (a) full-range i64 product — ~2^126, a 38-digit BigInt.
            0 => {
                let a = mix(&mut st) as i64;
                let b = mix(&mut st) as i64;
                format!("## Main\nLet a be {a}.\nLet b be {b}.\nShow a * b.\n")
            }
            // (b) a three-factor product + an offset — a longer overflow chain (`(a*b)*c + d`).
            1 => {
                let a = (mix(&mut st) % 100_000_000_000) as i64;
                let b = (mix(&mut st) % 100_000_000_000) as i64;
                let c = (mix(&mut st) % 100_000_000_000) as i64;
                let d = mix(&mut st) as i64;
                format!("## Main\nLet a be {a}.\nLet b be {b}.\nLet c be {c}.\nLet d be {d}.\nShow a * b * c + d.\n")
            }
            // (c) factorial to N (N up to 100 → 100! is 158 digits), the classic runaway BigInt.
            2 => {
                let n = 20 + (mix(&mut st) % 81) as i64; // 20..=100
                format!(
                    "## Main\nLet mutable f be 1.\nLet mutable i be 1.\nWhile i is less than or equal to {n}:\n    Set f to f * i.\n    Set i to i + 1.\nShow f.\n"
                )
            }
            // (d) repeated squaring — `x := x*x`, K times, from a small base (exponential digit growth).
            3 => {
                let base = 2 + (mix(&mut st) % 30) as i64;
                let k = 5 + (mix(&mut st) % 6) as i64; // 5..=10 squarings → up to base^1024
                format!(
                    "## Main\nLet mutable x be {base}.\nLet mutable i be 0.\nWhile i is less than {k}:\n    Set x to x * x.\n    Set i to i + 1.\nShow x.\n"
                )
            }
            // (e) a running accumulation of a large product, promoting mid-loop, then the final total.
            _ => {
                let a = mix(&mut st) as i64;
                let b = mix(&mut st) as i64;
                format!(
                    "## Main\nLet mutable acc be 0.\nLet mutable i be 0.\nWhile i is less than 50:\n    Set acc to acc + {a} * {b}.\n    Set i to i + 1.\nShow acc.\n"
                )
            }
        }
    }

    let mut state = 0x50F7_0A75_1CE4_D0DEu64;
    for iter in 0..300 {
        let src = build(mix(&mut state));

        let vm = vm_outcome(&src);
        assert!(
            vm.error.is_none(),
            "iter {iter}: VM errored on {src:?}: {:?}",
            vm.error
        );
        let vm_out = vm.output.trim().to_string();

        for width in [32u32, 64u32] {
            let words = compile_to_full_words_w(&src, width)
                .unwrap_or_else(|e| panic!("iter {iter} w{width}: encode failed on {src:?}: {e}"));
            let fabric = run_soft_core_typed_w(&words, CYCLES, width);
            let got: String = fabric
                .iter()
                .map(ShowVal::fmt)
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(
                got, vm_out,
                "iter {iter} w{width}: soft-core != VM EXACTLY over the BigInt path\n  src = {src:?}"
            );
        }
    }
}
