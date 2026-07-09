//! Codegen hot-path locks: the AOT shapes the benchmark suite's speed rests on.
//!
//! Exact-Int semantics (overflow ruling v2) and seq value semantics are both
//! CORRECT but each has a performance-recovery lowering that hot loops depend
//! on. These locks pin the recovered shapes so a semantics change can never
//! silently re-introduce the slow forms:
//!
//!   1. A root-NARROWED single exact op (both operands plain i64) fuses to the
//!      i64-native checked helper — no `LogosInt` round-trip in the hot path.
//!   2. The loopsplit-guarded fast branch carries its own overflow proof, so
//!      its body is RAW i64 arithmetic (this is what lets LLVM vectorize).
//!   3. A recursive take-seq/return-seq function whose argument is dead at
//!      every call site mutates IN PLACE — no per-recursion-level deep copy.
//!
//! Each lock also asserts runtime output, so the shape can never drift from
//! the meaning.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_exact_output, compile_to_rust};

// =============================================================================
// 1. Narrowed single exact op → fused i64 helper (no LogosInt round-trip)
// =============================================================================

/// `Set k to k + 1` on i64 locals is a single exact op narrowed straight back
/// to i64. The `logos_add_exact(..).expect_i64(..)` form materializes a
/// `LogosInt` enum per iteration; the fused `logos_add_i64` keeps the value in
/// a register with one overflow branch to a cold panic — byte-identical
/// semantics (same canonical overflow message), register-shaped hot path.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn narrowed_single_add_fuses_to_i64_helper() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable total be 0.
Let mutable i be 1.
While i is at most n:
    Let mutable k be i.
    While k is not 1:
        If k % 2 equals 0:
            Set k to k / 2.
        Otherwise:
            Set k to 3 * k + 1.
        Set total to total + 1.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(code).unwrap();
    // The HOT path must be LogosInt-free: any surviving `.expect_i64` may
    // only be the guarded dual's COLD fallback branch (`… if <leaf bounds> {
    // raw } else { exact.expect_i64 } …`), never an unguarded round-trip.
    for line in rust.lines() {
        if line.contains(".expect_i64") {
            assert!(
                line.contains("else {"),
                "unguarded LogosInt round-trip on a narrowed op — every \
                 expect_i64 must sit in a guarded dual's cold branch. Line:\n{line}\n\nFull:\n{rust}"
            );
        }
    }
    assert!(
        rust.contains("logos_add_i64(") || rust.contains("(total + 1)"),
        "`total + 1` should lower to the fused i64 helper (or oracle-proven raw), got:\n{rust}"
    );
}

/// Overflow through the fused helper keeps the interpreter's exact semantics:
/// the value promotes (tolerant sink) or panics with the canonical message
/// (narrowed sink) — `Show` is a tolerant sink, so the doubling chain must
/// print the exact 2^80, not wrap or trap.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn fused_helpers_keep_exact_promotion_semantics() {
    assert_exact_output(
        r#"## Main
Let mutable x be 1.
Let mutable i be 0.
While i is less than 80:
    Set x to x * 2.
    Set i to i + 1.
Show x.
"#,
        "1208925819614629174706176",
    );
}

// =============================================================================
// 2. Loopsplit's guarded fast branch is raw (its guard IS the overflow proof)
// =============================================================================

/// loop_sum's chunked fast path sits under a compiler-emitted value guard
/// chosen precisely so no add in the chunk can overflow. The body must
/// therefore be RAW i64 (`(sum + i)`) — checked helpers there both waste the
/// guard and block vectorization. The UNGUARDED big-n branch keeps its exact
/// helpers (overflow is genuinely reachable): the lock is raw-under-guard,
/// exact-elsewhere.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn loopsplit_guarded_branch_is_raw_i64() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable sum be 0.
Let mutable i be 1.
While i is at most n:
    Set sum to (sum + i) % 1000000007.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(code).unwrap();
    let Some(guard_pos) = rust.find("576460752240923487") else {
        panic!("loopsplit guard constant missing — chunked mod-hoist did not fire:\n{rust}");
    };
    let Some(defer_pos) = rust.find("__defer_stop") else {
        panic!("chunked inner loop (__defer_stop) missing:\n{rust}");
    };
    assert!(defer_pos > guard_pos, "chunk loop should sit inside the guard");
    // The chunk body: from the inner loop to the mod fold. Raw adds only.
    let chunk_end = rust[defer_pos..]
        .find("% 1000000007")
        .map(|p| defer_pos + p)
        .expect("mod fold after chunk loop");
    let chunk = &rust[defer_pos..chunk_end];
    assert!(
        !chunk.contains("logos_add_exact") && !chunk.contains("logos_add_i64"),
        "the guarded chunk body must be RAW i64 adds (the guard is the proof; \
         raw is what vectorizes). Got chunk:\n{chunk}"
    );
    assert!(
        chunk.contains("sum + i") || chunk.contains("(sum + i)"),
        "expected raw `sum + i` in the guarded chunk, got:\n{chunk}"
    );
}

// =============================================================================
// 3. Affine-recursion tower solves to guarded closed forms (O8b)
// =============================================================================

/// The specialized Ackermann tower is a chain of `f(0)=C; f(n)=g(f(n−1))`
/// recursions whose closed forms LLVM once derived from raw i64 ops — checked
/// exact arithmetic blocks that reasoning, so O8b derives them in OUR
/// optimizer with an explicit `0 <= n <= N_SAFE` version guard (proven-raw
/// fast branch) over the original recursion (exact fallback). This lock pins
/// both the VALUE (ackermann(3,5) = 2^8 − 3) and the SHAPE (a closed-form
/// shift, not a recursive chain, on the guarded path).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn affine_recursion_tower_solves_to_guarded_closed_form() {
    // `n` comes from argv so CTFE cannot fold the whole call away — the
    // solver's guarded closed form is the only route to a collapse.
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To ackermann (m: Int, n: Int) -> Int:
    If m is 0:
        Return n + 1.
    If n is 0:
        Return ackermann(m - 1, 1).
    Return ackermann(m - 1, ackermann(m, n - 1)).

## Main
Let arguments be args().
Show ackermann(3, parseInt(item 2 of arguments)).
"#;
    let run = common::run_logos_with_args(code, &["5"]);
    assert!(run.success, "ackermann tower program must run: {}", run.stderr);
    assert_eq!(run.stdout.trim(), "253", "ackermann(3,5) = 2^8 - 3");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("<<"),
        "the m=3 tower member should solve to a guarded `scale << n` closed \
         form (O8b), got:\n{rust}"
    );
}

// =============================================================================
// 4. Bound-versioned loop nests (O9): the spectral_norm shape
// =============================================================================

/// A counted nest evaluating a tiny pure Int-chain helper of its IVs must be
/// VERSIONED on the invariant bound — fast clone with the helper inlined and
/// the chain raw (what LLVM vectorizes), original loop as the exact fallback.
/// A per-iteration guard or `LogosInt` round-trip in the hot body is exactly
/// what this lock forbids: the fast branch must carry the RAW chain text.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn counted_nest_with_chain_helper_is_bound_versioned() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To aVal (i: Int, j: Int) -> Float:
    Return 1.0 / ((i + j) * (i + j + 1) / 2 + i + 1).

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable total be 0.0.
Let mutable i be 0.
While i is less than n:
    Let mutable j be 0.
    While j is less than n:
        Set total to total + aVal(i, j).
        Set j to j + 1.
    Set i to i + 1.
Show total.
"#;
    // ---- SHAPE (perf pin): O9 must version the nest on the solved leaf bound,
    //      carry the RAW inlined chain in the fast branch, and keep the hot path
    //      free of LogosInt round-trips — any surviving `.expect_i64` may only be
    //      the exact fallback's cold `else` branch (same rule as lock 1). ----
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("1073741824"),
        "the nest should be versioned on the solved leaf bound (2^30 for the \
         triangular chain), got:\n{rust}"
    );
    assert!(
        rust.contains("(i + j)"),
        "the fast branch should carry the RAW inlined chain, got:\n{rust}"
    );
    for line in rust.lines() {
        if line.contains(".expect_i64") {
            assert!(
                line.contains("else {"),
                "LogosInt round-trip on the versioned hot path — every expect_i64 \
                 must sit in the exact fallback's cold branch. Line:\n{line}\n\nFull:\n{rust}"
            );
        }
    }

    // ---- CORRECTNESS (behavior pin): the versioned fast branch must equal an
    //      INDEPENDENT host oracle — same loop order, integer denominator, IEEE-754
    //      accumulation — BIT-for-bit, across a domain of trip counts. The
    //      interpreter reference can't be fed argv, and a literal `n` would let CTFE
    //      fold the nest away (destroying the shape pinned above), so this
    //      host-computed reference is the reference engine here. Shortest-repr float
    //      printing round-trips, so parsing the output recovers the exact f64. ----
    fn reference_sum(n: i64) -> f64 {
        let mut total = 0.0_f64;
        let mut i = 0_i64;
        while i < n {
            let mut j = 0_i64;
            while j < n {
                let denom = (i + j) * (i + j + 1) / 2 + i + 1;
                total += 1.0_f64 / denom as f64;
                j += 1;
            }
            i += 1;
        }
        total
    }

    for n in [1_i64, 9, 16] {
        let run = common::run_logos_with_args(code, &[&n.to_string()]);
        assert!(run.success, "bound-versioned nest must run at n={n}: {}", run.stderr);
        let got: f64 = run
            .stdout
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("n={n}: output is not a float ({e}): {:?}", run.stdout));
        assert_eq!(
            got.to_bits(),
            reference_sum(n).to_bits(),
            "n={n}: the versioned fast path diverged from the IEEE-754 host oracle \
             (got {got}, want {})",
            reference_sum(n)
        );
    }

    // ---- GOLDEN + DETERMINISM: n=4 is the canonical spectral sum; the versioned
    //      output must match it exactly, agree bit-for-bit with the host oracle, and
    //      two independent compile+run cycles must print byte-identically. ----
    let a = common::run_logos_with_args(code, &["4"]);
    let b = common::run_logos_with_args(code, &["4"]);
    assert!(
        a.success && b.success,
        "bound-versioned nest must run at n=4: {} {}",
        a.stderr,
        b.stderr
    );
    assert_eq!(
        a.stdout.trim(),
        "3.3088403701561604",
        "canonical spectral sum 1/T(i,j) over [0,4)² (golden)"
    );
    let got4: f64 = a.stdout.trim().parse().expect("n=4 output is a float");
    assert_eq!(
        got4.to_bits(),
        reference_sum(4).to_bits(),
        "n=4 diverged from the IEEE-754 host oracle"
    );
    assert_eq!(
        a.stdout, b.stdout,
        "two independent compiles must be byte-identical (determinism)"
    );
}

// =============================================================================
// 5. Recursive seq-through function: dead-at-call-site args mutate in place
// =============================================================================

/// The quicksort shape: `qs` takes a Seq, element-writes it, recurses twice,
/// returns it — and every call site passes a handle that is DEAD after the
/// call (`Set result to qs(result, ...)`, `Set arr to qs(arr, ...)`). Value
/// semantics is unobservable here, so the callee must run IN PLACE — the
/// `&mut [T]` lowering (or an equivalent unique-handle move chain with no
/// per-level copy). A `.clone()` of the seq per recursion level is the
/// O(n · depth) catastrophe this lock exists to prevent.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn recursive_seq_through_fn_runs_in_place() {
    let code = r#"## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:
    If lo is at least hi:
        Return arr.
    Let pivot be item hi of arr.
    Let mutable result be arr.
    Let mutable i be lo.
    Let mutable j be lo.
    While j is less than hi:
        If item j of result is at most pivot:
            Let tmp be item i of result.
            Set item i of result to item j of result.
            Set item j of result to tmp.
            Set i to i + 1.
        Set j to j + 1.
    Let tmp be item i of result.
    Set item i of result to item hi of result.
    Set item hi of result to tmp.
    Set result to qs(result, lo, i - 1).
    Set result to qs(result, i + 1, hi).
    Return result.

## Main
Let mutable arr be a new Seq of Int.
Push 5 to arr.
Push 3 to arr.
Push 9 to arr.
Push 1 to arr.
Push 7 to arr.
Set arr to qs(arr, 1, 5).
Show "" + item 1 of arr + " " + item 3 of arr + " " + item 5 of arr.
"#;
    assert_exact_output(code, "1 5 9");
    let rust = compile_to_rust(code).unwrap();
    let qs_start = rust.find("fn qs").expect("qs function in generated code");
    let qs_body = &rust[qs_start..rust[qs_start..]
        .find("\nfn ")
        .map(|p| qs_start + p)
        .unwrap_or(rust.len())];
    assert!(
        !qs_body.contains(".clone()"),
        "qs must not deep-copy (or refcount-share into a later cow deep-copy) \
         the seq per recursion level — its argument is dead at every call \
         site, so in-place mutation is unobservable and required. Got:\n{qs_body}"
    );
    assert!(
        !qs_body.contains(".cow()"),
        "in-place qs needs no cow barriers — a unique handle (or &mut slice) \
         proves them away. Got:\n{qs_body}"
    );
}
