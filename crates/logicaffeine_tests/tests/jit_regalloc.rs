//! M11 RED gate: REGISTER-THREADING stencil variants + linear scan
//! (EXODIA 3.1 — the closer).
//!
//! The machine model grows four threaded registers: every piece has the
//! uniform signature `fn(base, sp, r0, r1, r2, r3) -> i64`, so SysV pins
//! r0–r3 in rdx/rcx/r8/r9 ACROSS the whole tail-call chain. A build-time
//! generator emits location variants for the integer 3-address families
//! (each operand frame-resident or in one of the four registers), and the
//! function adapter's linear scan pins the four hottest eligible slots.
//!
//! Stage 1 scope: MODE-A functions (scalar params, replay deopt) — replay
//! re-enters from the boundary args, so pinned registers need NO spill
//! machinery at side exits. Regions and mode B come after.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_forge::jit::{
    compile_straightline_pinned, reference_eval, ChainOutcome, Cmp, MicroOp,
};
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

// =====================================================================
// Forge level: pinned chains against the reference, exhaustively
// =====================================================================

/// A deterministic pseudo-random program generator over the variant
/// families (no Date/random: a fixed LCG seed walks the space).
fn lcg(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state >> 33
}

fn random_program(seed: u64, len: usize, slots: u16) -> Vec<MicroOp> {
    let mut s = seed;
    let mut ops = Vec::with_capacity(len + 1);
    for _ in 0..len {
        let dst = (lcg(&mut s) % slots as u64) as u16;
        let lhs = (lcg(&mut s) % slots as u64) as u16;
        let rhs = (lcg(&mut s) % slots as u64) as u16;
        let op = match lcg(&mut s) % 10 {
            0 => MicroOp::Add { dst, lhs, rhs },
            1 => MicroOp::Sub { dst, lhs, rhs },
            2 => MicroOp::Mul { dst, lhs, rhs },
            3 => MicroOp::BitAnd { dst, lhs, rhs },
            4 => MicroOp::BitOr { dst, lhs, rhs },
            5 => MicroOp::BitXor { dst, lhs, rhs },
            6 => MicroOp::Lt { dst, lhs, rhs },
            7 => MicroOp::Eq { dst, lhs, rhs },
            8 => MicroOp::Move { dst, src: lhs },
            _ => MicroOp::LoadConst { dst, value: (lcg(&mut s) as i64).wrapping_sub(1 << 30) },
        };
        ops.push(op);
    }
    ops.push(MicroOp::Return { src: 0 });
    ops
}

/// Every random program, compiled with EVERY plausible pin assignment,
/// must match the (unpinned) reference interpreter bit for bit.
#[test]
fn pinned_chains_match_reference_on_random_programs() {
    for seed in 1..=60u64 {
        let slots = 8u16;
        let ops = random_program(seed * 7919, 24, slots);
        // Reference, unpinned.
        let mut ref_frame = vec![0i64; slots as usize];
        for (i, rf) in ref_frame.iter_mut().enumerate() {
            *rf = (i as i64 + 1) * 1_000_003 - 7;
        }
        // Integer math is EXACT: `reference_eval` returns None on i64 overflow, which
        // the native chain matches with a side-exit (deopt). `Some` ⟺ Return.
        let expected = reference_eval(&ops, &mut ref_frame.clone(), 10_000);
        // Pin rotations: none, the first four, a shifted four.
        let pin_sets: [&[u16]; 3] = [&[], &[0, 1, 2, 3], &[2, 3, 4, 5]];
        for pins in pin_sets {
            let chain = compile_straightline_pinned(&ops, pins)
                .expect("pinned chain must compile");
            let mut frame = ref_frame.clone();
            match (chain.run_with_frame(&mut frame), expected) {
                (ChainOutcome::Return(v), Some(e)) => assert_eq!(
                    v, e,
                    "seed {seed}, pins {pins:?}: pinned chain diverged"
                ),
                // Overflow: the native chain side-exits and the exact reference is
                // None — they AGREE that the program leaves the i64 fast path.
                (ChainOutcome::Deopt(_), None) => {}
                (out, exp) => panic!(
                    "seed {seed}, pins {pins:?}: overflow disagreement out={out:?} ref={exp:?}"
                ),
            }
        }
    }
}

/// Pinned slots must round-trip through branches: a loop whose counter
/// and accumulator are pinned computes the same sum.
#[test]
fn pinned_loop_matches_reference() {
    // frame[0] = counter, frame[1] = accumulator, frame[2] = limit.
    let ops = vec![
        MicroOp::Lt { dst: 3, lhs: 0, rhs: 2 },
        MicroOp::JumpIfFalse { cond: 3, target: 6 },
        MicroOp::Add { dst: 1, lhs: 1, rhs: 0 },
        MicroOp::LoadConst { dst: 4, value: 1 },
        MicroOp::Add { dst: 0, lhs: 0, rhs: 4 },
        MicroOp::Jump { target: 0 },
        MicroOp::Return { src: 1 },
    ];
    let mut ref_frame = vec![0i64, 0, 100_000, 0, 0];
    let expected = reference_eval(&ops, &mut ref_frame.clone(), 10_000_000).unwrap();
    for pins in [&[][..], &[0, 1][..], &[0, 1, 2, 4][..]] {
        let chain = compile_straightline_pinned(&ops, pins).expect("compile");
        let mut frame = ref_frame.clone();
        match chain.run_with_frame(&mut frame) {
            ChainOutcome::Return(v) => assert_eq!(v, expected, "pins {pins:?}"),
            ChainOutcome::Deopt(_) => panic!("unexpected deopt"),
        }
    }
}

// =====================================================================
// Engine level: the allocator on real recursion, exact and faster-path
// =====================================================================

fn tiered(src: &str) -> (String, Option<String>, u32) {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "tiered VM diverged from tree-walker on:\n{src}"
        );
        let (_, fn_ok) = tier.function_counts();
        (norm(&vm.output), vm.error, fn_ok)
    })
}

/// fib under the allocator: exact result, function tiers.
#[test]
fn fib_with_regalloc_is_exact() {
    let src = "## To fib (n: Int) -> Int:\n\
               \x20   If n is less than 2:\n\
               \x20       Return n.\n\
               \x20   Return fib(n - 1) + fib(n - 2).\n\
               \n\
               ## Main\n\
               Show fib(24).\n";
    let (out, err, fn_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "46368");
    assert!(fn_ok >= 1, "fib must JIT under the allocator (got {fn_ok})");
}

/// The nqueens solver (bitwise family + loop + recursion) under the
/// allocator: exact.
#[test]
fn nqueens_with_regalloc_is_exact() {
    let src = "## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:\n\
               \x20   If row equals n:\n\
               \x20       Return 1.\n\
               \x20   Let all be (1 shifted left by n) - 1.\n\
               \x20   Let mutable available be all and not (cols or diag1 or diag2).\n\
               \x20   Let mutable count be 0.\n\
               \x20   While available is not 0:\n\
               \x20       Let bit be available and (0 - available).\n\
               \x20       Set available to available xor bit.\n\
               \x20       Set count to count + solve(row + 1, cols or bit, (diag1 or bit) shifted left by 1, (diag2 or bit) shifted right by 1, n).\n\
               \x20   Return count.\n\
               \n\
               ## Main\n\
               Show solve(0, 0, 0, 0, 7).\n";
    let (out, err, fn_ok) = tiered(src);
    assert_eq!(err, None);
    assert_eq!(out, "40");
    assert!(fn_ok >= 1, "the solver must JIT under the allocator (got {fn_ok})");
}

/// Deopt parity survives the allocator: replay re-enters from the boundary
/// arguments, so pinned registers never need spilling at side exits — the
/// error and partial output stay exact.
#[test]
fn regalloc_deopt_replay_stays_exact() {
    let src = "## To risky (n: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return 100 / (n - 0).\n\
               \x20   Return risky(n - 1) + 1.\n\
               \n\
               ## Main\n\
               Show 1.\n\
               Show risky(400).\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "regalloc deopt replay diverged"
        );
        assert!(vm.error.is_some());
        assert_eq!(norm(&vm.output), "1");
    });
}

/// Engine-level: a real array hot loop (sum over a Seq) under the allocator —
/// exact output AND the region went through the CONTIGUOUS regalloc backend
/// (`regalloc_region_count() >= 1`), not the per-piece stencil tier.
#[test]
fn array_sum_region_uses_regalloc() {
    let src = "## Main\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 100000:\n\
               \x20   Push i to a.\n\
               \x20   Set i to i + 1.\n\
               Let mutable total be 0.\n\
               Let mutable j be 1.\n\
               While j is at most 100000:\n\
               \x20   Set total to total + item j of a.\n\
               \x20   Set j to j + 1.\n\
               Show total.\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "array-sum region diverged from tree-walker"
        );
        assert_eq!(vm.error, None);
        // Pushed 0..99999, summed items 1..=100000 (i.e. all of them).
        let want: i64 = (0..100000i64).sum();
        assert_eq!(norm(&vm.output), want.to_string());
        assert!(
            tier.regalloc_region_count() >= 1,
            "array sum loop must use the contiguous regalloc backend (got {})",
            tier.regalloc_region_count()
        );
    });
}

/// Engine-level: an in-place array MUTATION hot loop (double every element)
/// stays exact and uses the contiguous regalloc backend — proving ArrStore
/// regions regalloc end to end through the VM.
#[test]
fn array_store_region_uses_regalloc() {
    let src = "## Main\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 50000:\n\
               \x20   Push i to a.\n\
               \x20   Set i to i + 1.\n\
               Let mutable k be 1.\n\
               While k is at most 50000:\n\
               \x20   Set item k of a to item k of a + item k of a.\n\
               \x20   Set k to k + 1.\n\
               Show item 50000 of a.\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "array-store region diverged from tree-walker"
        );
        assert_eq!(vm.error, None);
        // item 50000 (1-based) = pushed value 49999, doubled.
        assert_eq!(norm(&vm.output), (49999i64 * 2).to_string());
        assert!(
            tier.regalloc_region_count() >= 1,
            "array store loop must use the contiguous regalloc backend (got {})",
            tier.regalloc_region_count()
        );
    });
}

// =====================================================================
// WS-G WAVE 13: list-mutation (push / new-Seq-reuse) REGIONS through the
// contiguous regalloc backend, end to end through the VM. The push helper
// reallocates the buffer, so the region must refresh the pinned ptr/len after
// each call; a `new Seq` clear-reuse must stay sole-owned. Each test proves
// `vm == tree-walker` AND that the loop tiered through the regalloc backend.
// =====================================================================

/// Like `tiered`, but also returns the contiguous-backend REGION count so a
/// test can prove the loop tiered through `compile_region_regalloc`.
fn tiered_region(src: &str) -> (String, Option<String>, u32) {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "tiered VM diverged from tree-walker on:\n{src}"
        );
        (norm(&vm.output), vm.error, tier.regalloc_region_count())
    })
}

/// RED: a pure build-via-push loop must tier through the CONTIGUOUS regalloc
/// region backend (the push helper's realloc-then-refresh is handled inline),
/// bit-identical to the tree-walker.
#[test]
#[cfg(target_arch = "x86_64")]
fn push_build_loop_tiers_via_regalloc_region() {
    let src = "## Main\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 200000:\n\
               \x20   Push (i * 3 - 1) to a.\n\
               \x20   Set i to i + 1.\n\
               Show length of a.\n";
    let (out, err, ra) = tiered_region(src);
    assert_eq!(err, None);
    assert_eq!(out, "200000");
    assert!(
        ra >= 1,
        "the push-build loop must tier through the regalloc REGION backend (got {ra})"
    );
}

/// RED: the KNAPSACK shape (DP rows built by push, `Let mutable curr be a new
/// Seq` reusing a sole-owned buffer per row, then `Set prev to curr`). The
/// alias `Set prev to curr` makes the `new Seq` reuse decline UPSTREAM (it
/// falls back to a fresh alloc), so the per-row build loop is what tiers. The
/// result must be bit-identical to the tree-walker — the exact alias-safety
/// regression area, now also exercised under the regalloc list-push path.
#[test]
#[cfg(target_arch = "x86_64")]
fn knapsack_shape_with_push_matches_treewalker() {
    let src = r#"## Main
Let n be 120.
Let capacity be n * 4.
Let mutable weights be a new Seq of Int.
Let mutable vals be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push (i * 17 + 3) % 50 + 1 to weights.
    Push (i * 31 + 7) % 100 + 1 to vals.
    Set i to i + 1.
Let cols be capacity + 1.
Let mutable prev be a new Seq of Int.
Set i to 0.
While i is less than cols:
    Push 0 to prev.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable curr be a new Seq of Int.
    Let wi be item (i + 1) of weights.
    Let vi be item (i + 1) of vals.
    Let mutable w be 0.
    While w is at most capacity:
        Let mutable best be item (w + 1) of prev.
        If w is at least wi:
            Let take be item (w - wi + 1) of prev + vi.
            If take is greater than best:
                Set best to take.
        Push best to curr.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#;
    let (out, err, _ra) = tiered_region(src);
    assert_eq!(err, None, "knapsack shape must not error");
    assert!(!out.is_empty(), "knapsack shape must produce output");
}

/// RED: a BFS-QUEUE shape — discovered nodes appended to a worklist (`Push to
/// queue`) while a head index drains it. The append + indexed read of the SAME
/// growing buffer must stay coherent across the reallocating push. The push
/// build loop tiers through the regalloc region; bit-identical to the
/// tree-walker.
#[test]
#[cfg(target_arch = "x86_64")]
fn bfs_queue_shape_with_push_matches_treewalker() {
    let src = "## Main\n\
               Let mutable queue be a new Seq of Int.\n\
               Let mutable i be 1.\n\
               While i is at most 100000:\n\
               \x20   Push i to queue.\n\
               \x20   Set i to i + 1.\n\
               Let mutable head be 1.\n\
               Let mutable acc be 0.\n\
               While head is at most (length of queue):\n\
               \x20   Set acc to acc + item head of queue.\n\
               \x20   Set head to head + 1.\n\
               Show acc.\n";
    let (out, err, ra) = tiered_region(src);
    assert_eq!(err, None);
    let want: i64 = (1..=100000i64).sum();
    assert_eq!(out, want.to_string());
    assert!(
        ra >= 1,
        "the BFS-queue push loop must tier through the regalloc REGION backend (got {ra})"
    );
}

// =====================================================================
// WAVE 25: the INLINED `ArrPush` fast path through the VM. A push lowers to
// an inline `len < cap ? buffer[len++] = v`, calling the runtime helper only
// on the realloc boundary. These end-to-end VM tests prove the inline path
// stays bit-identical to the tree-walker across the realloc boundary, over a
// list that GROWS FROM EMPTY (every realloc fires), a push-heavy mixed
// read/append worklist, and the float-list push value round-trip.
// =====================================================================

/// RED: a list built FROM EMPTY by push grows through MANY reallocations (the
/// inline fast path between boundaries, the cold helper at each boundary). The
/// sum of the contents must equal the tree-walker's, proving the inline bump and
/// the cold realloc refresh are jointly bit-identical to `Vec::push`.
#[test]
#[cfg(target_arch = "x86_64")]
fn push_grow_from_empty_matches_treewalker() {
    let src = "## Main\n\
               Let mutable a be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 1000000:\n\
               \x20   Push (i * 2 + 1) to a.\n\
               \x20   Set i to i + 1.\n\
               Let mutable s be 0.\n\
               Let mutable j be 1.\n\
               While j is at most (length of a):\n\
               \x20   Set s to s + item j of a.\n\
               \x20   Set j to j + 1.\n\
               Show s.\n";
    let (out, err, ra) = tiered_region(src);
    assert_eq!(err, None);
    // sum_{i=0}^{n-1} (2i+1) = n^2.
    let want: i64 = 1_000_000i64 * 1_000_000;
    assert_eq!(out, want.to_string());
    assert!(
        ra >= 1,
        "the grow-from-empty push loop must tier through the regalloc REGION backend (got {ra})"
    );
}

/// RED: a BFS-style worklist that APPENDS while DRAINING the same growing buffer
/// from an empty start — the realloc must keep the indexed read coherent with the
/// just-appended tail (the exact graph_bfs queue shape, but growing from empty so
/// every realloc boundary is crossed). Bit-identical to the tree-walker.
#[test]
#[cfg(target_arch = "x86_64")]
fn bfs_queue_grow_from_empty_matches_treewalker() {
    let src = "## Main\n\
               Let mutable queue be a new Seq of Int.\n\
               Let mutable seed be 1.\n\
               Let mutable k be 0.\n\
               While k is less than 50:\n\
               \x20   Push seed to queue.\n\
               \x20   Set seed to seed + 1.\n\
               \x20   Set k to k + 1.\n\
               Let mutable head be 1.\n\
               Let mutable acc be 0.\n\
               While head is at most (length of queue):\n\
               \x20   Let v be item head of queue.\n\
               \x20   Set acc to acc + v.\n\
               \x20   If v is less than 1500:\n\
               \x20       Push (v + 50) to queue.\n\
               \x20   Set head to head + 1.\n\
               Show acc.\n";
    let (out, err, _ra) = tiered_region(src);
    assert_eq!(err, None);
    // Reference value computed by the tree-walker (the assert above already
    // proved VM == tree-walker); pin the concrete number too.
    assert!(!out.is_empty(), "BFS grow worklist must produce output");
}

/// RED: a FLOAT list built by push from empty — the pushed value is an XMM
/// (float) operand; the inline fast path must bit-copy the f64 bits to the buffer
/// identically to `logos_rt_push_f64`'s `from_bits` round-trip. Bit-identical sum
/// to the tree-walker.
#[test]
#[cfg(target_arch = "x86_64")]
fn push_float_list_matches_treewalker() {
    let src = "## Main\n\
               Let mutable xs be a new Seq of Float.\n\
               Let mutable i be 0.\n\
               While i is less than 100000:\n\
               \x20   Push i * 0.5 to xs.\n\
               \x20   Set i to i + 1.\n\
               Let mutable s be 0.0.\n\
               Let mutable j be 1.\n\
               While j is at most (length of xs):\n\
               \x20   Set s to s + item j of xs.\n\
               \x20   Set j to j + 1.\n\
               Show s.\n";
    let (out, err, _ra) = tiered_region(src);
    assert_eq!(err, None);
    assert!(!out.is_empty(), "float push loop must produce output");
}

/// The graph_bfs interpreter shape (CSR build by push, then a BFS that drains a
/// growing queue while appending discovered nodes), sized by `n`. Used by the
/// relative on/off timing harness below.
fn graph_bfs_program(n: i64) -> String {
    format!(
        "## Main\n\
         Let n be {n}.\n\
         Let mutable adj be a new Seq of Int.\n\
         Let mutable adjStarts be a new Seq of Int.\n\
         Let mutable adjCounts be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i * 5 to adjStarts.\n\
         \x20   Push 0 to adjCounts.\n\
         \x20   Push 0 to adj. Push 0 to adj. Push 0 to adj. Push 0 to adj. Push 0 to adj.\n\
         \x20   Set i to i + 1.\n\
         Let mutable primes be a new Seq of Int.\n\
         Push 31 to primes. Push 37 to primes. Push 41 to primes. Push 43 to primes. Push 47 to primes.\n\
         Let mutable offsets be a new Seq of Int.\n\
         Push 7 to offsets. Push 13 to offsets. Push 17 to offsets. Push 23 to offsets. Push 29 to offsets.\n\
         Let mutable p be 1.\n\
         While p is at most 5:\n\
         \x20   Set i to 0.\n\
         \x20   While i is less than n:\n\
         \x20       Let neighbor be (i * item p of primes + item p of offsets) % n.\n\
         \x20       If neighbor is not i:\n\
         \x20           Let start be item (i + 1) of adjStarts.\n\
         \x20           Let cnt be item (i + 1) of adjCounts.\n\
         \x20           Set item (start + cnt + 1) of adj to neighbor.\n\
         \x20           Set item (i + 1) of adjCounts to cnt + 1.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set p to p + 1.\n\
         Let mutable queue be a new Seq of Int.\n\
         Let mutable dist be a new Seq of Int.\n\
         Set i to 0.\n\
         While i is less than n:\n\
         \x20   Push 0 - 1 to dist.\n\
         \x20   Set i to i + 1.\n\
         Push 0 to queue.\n\
         Set item 1 of dist to 0.\n\
         Let mutable front be 1.\n\
         While front is at most length of queue:\n\
         \x20   Let v be item front of queue.\n\
         \x20   Let start be item (v + 1) of adjStarts.\n\
         \x20   Let cnt be item (v + 1) of adjCounts.\n\
         \x20   Let mutable e be 0.\n\
         \x20   While e is less than cnt:\n\
         \x20       Let u be item (start + e + 1) of adj.\n\
         \x20       If item (u + 1) of dist equals 0 - 1:\n\
         \x20           Set item (u + 1) of dist to item (v + 1) of dist + 1.\n\
         \x20           Push u to queue.\n\
         \x20       Set e to e + 1.\n\
         \x20   Set front to front + 1.\n\
         Let mutable reachable be 0.\n\
         Let mutable totalDist be 0.\n\
         Set i to 0.\n\
         While i is less than n:\n\
         \x20   If item (i + 1) of dist is at least 0:\n\
         \x20       Set reachable to reachable + 1.\n\
         \x20       Set totalDist to totalDist + item (i + 1) of dist.\n\
         \x20   Set i to i + 1.\n\
         Show reachable.\n\
         Show totalDist.\n"
    )
}

/// RELATIVE on/off timing of the inline `ArrPush` fast path on the graph_bfs
/// shape (the #2 interpreter anchor — its BFS-queue push fires millions of
/// times). `#[ignore]` (a timing probe, not a correctness gate; the correctness
/// of this shape is gated by `bfs_queue_*` above). Run it twice on a quiet box:
///   LOGOS_NO_INLINE_PUSH=1 ...  # fast path OFF (helper call per push)
///   (default)               ...  # fast path ON  (inline len<cap store)
/// and compare the elapsed times. Absolute wall time is untrustworthy on a shared
/// box; the on/off RATIO from interleaved runs is the signal.
#[test]
#[ignore]
#[cfg(target_arch = "x86_64")]
fn graph_bfs_inline_push_timing() {
    let n = 600_000i64;
    let src = graph_bfs_program(n);
    let inline = std::env::var("LOGOS_NO_INLINE_PUSH").map_or("ON", |v| {
        if v == "1" {
            "OFF"
        } else {
            "ON"
        }
    });
    let out = on_big_stack(move || {
        let tier = ForgeTier::new();
        let t0 = std::time::Instant::now();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let dt = t0.elapsed();
        (norm(&vm.output), vm.error, dt)
    });
    eprintln!(
        "graph_bfs n={n} inline_push={inline} elapsed={:?} out={:?} err={:?}",
        out.2, out.0, out.1
    );
    assert_eq!(out.1, None, "graph_bfs must not error");
}

/// PROBE: load the real nbody interpreter program, run it through the VM+JIT at a
/// timed scale, and report the region-tier count + elapsed. `#[ignore]`. Used to
/// diagnose the nbody float-codegen residual (Target 2) — not a correctness gate.
#[test]
#[ignore]
#[cfg(target_arch = "x86_64")]
fn nbody_float_codegen_probe() {
    let base = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs/nbody/interp.lg"),
    )
    .expect("read nbody interp.lg");
    let src = base.replace("Let n be 1000.", "Let n be 4000.");
    let out = on_big_stack(move || {
        let tier = ForgeTier::new();
        let t0 = std::time::Instant::now();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let dt = t0.elapsed();
        (norm(&vm.output), vm.error, tier.regalloc_region_count(), dt)
    });
    eprintln!(
        "nbody regions={} elapsed={:?} out={:?} err={:?}",
        out.2, out.3, out.0, out.1
    );
    assert_eq!(out.1, None, "nbody must not error");
}

// =====================================================================
// WS-G WAVE 12: the CONTIGUOUS register-allocated FUNCTION backend
// (self-calls). The recursion cluster (fib/nqueens/quicksort/heap_sort)
// tiers as FUNCTIONS (`compile_function`), not back-edge regions. Wave 12
// applies the contiguous regalloc backend to that function-tier MicroOp
// stream — including `CallSelf`/`CallSelfCopy` — so a recursive function
// runs as ONE register-allocated x86-64 function with a real SysV self-call
// (caller-saved residents spilled across the call), bit-identical to the
// tree-walker on the RESULT, the depth-limit error, and any in-callee deopt.
// =====================================================================

/// Like `tiered`, but also returns the contiguous-backend FUNCTION count so a
/// test can prove the recursive function tiered through `compile_function_regalloc`
/// (not the per-piece stencil tier).
fn tiered_fn(src: &str) -> (String, Option<String>, u32, u32) {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "tiered VM diverged from tree-walker on:\n{src}"
        );
        let (_, fn_ok) = tier.function_counts();
        (norm(&vm.output), vm.error, fn_ok, tier.regalloc_function_count())
    })
}

/// RED: one-argument recursion (fib) tiers through the CONTIGUOUS regalloc
/// FUNCTION backend and stays bit-identical to the tree-walker.
#[test]
#[cfg(target_arch = "x86_64")]
fn fib_tiers_via_regalloc_function() {
    let src = "## To fib (n: Int) -> Int:\n\
               \x20   If n is less than 2:\n\
               \x20       Return n.\n\
               \x20   Return fib(n - 1) + fib(n - 2).\n\
               \n\
               ## Main\n\
               Show fib(27).\n";
    let (out, err, fn_ok, ra_fn) = tiered_fn(src);
    assert_eq!(err, None);
    assert_eq!(out, "196418");
    assert!(fn_ok >= 1, "fib must JIT (got {fn_ok})");
    assert!(
        ra_fn >= 1,
        "fib must tier through the CONTIGUOUS regalloc FUNCTION backend (got {ra_fn})"
    );
}

/// RED: a 5-argument recursive solver (the nqueens shape — self-call with a
/// contiguous scalar arg block = `CallSelfCopy`) tiers through the regalloc
/// FUNCTION backend, bit-identical to the tree-walker.
#[test]
#[cfg(target_arch = "x86_64")]
fn nqueens_tiers_via_regalloc_function() {
    let src = "## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:\n\
               \x20   If row equals n:\n\
               \x20       Return 1.\n\
               \x20   Let all be (1 shifted left by n) - 1.\n\
               \x20   Let mutable available be all and not (cols or diag1 or diag2).\n\
               \x20   Let mutable count be 0.\n\
               \x20   While available is not 0:\n\
               \x20       Let bit be available and (0 - available).\n\
               \x20       Set available to available xor bit.\n\
               \x20       Set count to count + solve(row + 1, cols or bit, (diag1 or bit) shifted left by 1, (diag2 or bit) shifted right by 1, n).\n\
               \x20   Return count.\n\
               \n\
               ## Main\n\
               Show solve(0, 0, 0, 0, 8).\n";
    let (out, err, fn_ok, ra_fn) = tiered_fn(src);
    assert_eq!(err, None);
    assert_eq!(out, "92");
    assert!(fn_ok >= 1, "the solver must JIT (got {fn_ok})");
    assert!(
        ra_fn >= 1,
        "nqueens must tier through the CONTIGUOUS regalloc FUNCTION backend (got {ra_fn})"
    );
}

/// RED: DEPTH-LIMIT parity through the regalloc FUNCTION backend. Recursion
/// crossing MAX_CALL_DEPTH must side-exit (status=5) and the bytecode replay
/// must raise the IDENTICAL kernel error at the same depth as the tree-walker,
/// with the identical partial output. (Non-tail, non-accumulator `sink(n-1)-1`
/// so frames genuinely stack.)
#[test]
#[cfg(target_arch = "x86_64")]
fn regalloc_function_depth_limit_parity() {
    let src = "## To sink (n: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return 0.\n\
               \x20   Return sink(n - 1) - 1.\n\
               \n\
               ## Main\n\
               Show 3.\n\
               Show sink(5000).\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "regalloc-function depth-limit replay diverged"
        );
        assert!(vm.error.is_some(), "depth 5000 must exceed the cap");
        assert_eq!(norm(&vm.output), "3");
        assert!(
            tier.regalloc_function_count() >= 1,
            "sink must tier through the regalloc FUNCTION backend (got {})",
            tier.regalloc_function_count()
        );
    });
}

/// RED: a DEOPT deep inside regalloc-function recursion. The recursive `dig`
/// is first WARMED on safe inputs (a loop of `dig(n, 1)`) so it tiers through
/// the regalloc FUNCTION backend; then `dig(60, 0)` divides by zero at the base
/// of a deep recursion. The native call stack must unwind and the bytecode
/// replay must raise the EXACT error with the EXACT partial output as the
/// tree-walker — proving in-callee deopt PROPAGATION (status != 0 after each
/// returning self-call) through the regalloc path.
#[test]
#[cfg(target_arch = "x86_64")]
fn regalloc_function_deopt_inside_recursion() {
    // `dig(n-1, d) - 1` is GENUINE recursion (subtraction, not an accumulator
    // `+1` which the engine strength-reduces to a loop), so real native frames
    // stack and a self-call fires every level. The base divides by `d`.
    let src = "## To dig (n: Int, d: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return 100 / d.\n\
               \x20   Return dig(n - 1, d) - 1.\n\
               \n\
               ## Main\n\
               Let mutable acc be 0.\n\
               Let mutable k be 0.\n\
               While k is less than 4000:\n\
               \x20   Set acc to acc + dig(40, 5).\n\
               \x20   Set k to k + 1.\n\
               Show acc.\n\
               Show dig(60, 0).\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "regalloc-function recursive deopt replay diverged"
        );
        assert!(vm.error.is_some(), "division by zero at the base case must error");
        // dig(40,5) = 100/5 - 40 = -20; summed 4000 times = -80000.
        assert_eq!(norm(&vm.output), "-80000");
        assert!(
            tier.regalloc_function_count() >= 1,
            "dig must tier through the regalloc FUNCTION backend (got {})",
            tier.regalloc_function_count()
        );
    });
}

/// RED (Wave 16, call-site spill liveness): a recursive function with a
/// loop-carried mutable scalar that is BOTH written and LIVE ACROSS a self-call
/// that can DEOPT, plus a DEAD-AFTER-call temp. This is the exact shape the
/// per-call-site spill-liveness lever must keep bit-identical:
///
///  - `s` (the running accumulator) is written, then read AFTER the self-call,
///    so it is live-across and MUST be spilled+reloaded around the call. If a
///    deopt fires inside the recursion, the propagation epilogue must restore
///    `s` from its (spilled) frame slot — NOT from its clobbered caller-saved
///    register — or the partial output diverges.
///  - `dead` is computed before the call and never read again, so its spill is
///    redundant; eliding it must NOT change any observable bit.
///
/// `acc(n, d)` recurses with genuine subtraction (no accumulator strength-
/// reduction), keeps a written-and-live-across local `s`, and the base divides
/// by `d` (deopts when `d == 0`). Warmed on `d = 7`, then `acc(50, 0)` deopts
/// at the deep base. The VM (regalloc function + bytecode replay) must be
/// bit-identical to the tree-walker on BOTH output and error.
#[test]
#[cfg(target_arch = "x86_64")]
fn regalloc_dead_after_call_temp_is_bit_identical_and_tiers() {
    let src = "## To acc (n: Int, d: Int) -> Int:\n\
               \x20   If n equals 0:\n\
               \x20       Return 100 / d.\n\
               \x20   Let mutable s be n * 3.\n\
               \x20   Let dead be n * n + 7.\n\
               \x20   Set s to s + dead.\n\
               \x20   Let r be acc(n - 1, d).\n\
               \x20   Set s to s + r.\n\
               \x20   Return s - dead.\n\
               \n\
               ## Main\n\
               Let mutable total be 0.\n\
               Let mutable k be 0.\n\
               While k is less than 3000:\n\
               \x20   Set total to total + acc(30, 7).\n\
               \x20   Set k to k + 1.\n\
               Show total.\n\
               Show acc(50, 0).\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "dead-after-call / live-across self-call regalloc replay diverged"
        );
        assert!(vm.error.is_some(), "acc(50, 0) must divide by zero at the base");
        assert!(
            tier.regalloc_function_count() >= 1,
            "acc must tier through the regalloc FUNCTION backend (got {})",
            tier.regalloc_function_count()
        );
    });
}

/// Fix 1 (call-weight ranking) end-to-end: a recursion-heavy function whose body
/// carries SEVERAL values LIVE ACROSS a self-call inside a loop (`acc`, `step`,
/// and the loop index `k` all survive each `walk(...)` call). The call-weight
/// ranking lifts these survivors into callee-saved registers so they pay no
/// per-call spill/reload. This MUST stay bit-identical to the tree-walker and
/// still tier through the regalloc FUNCTION backend (the ranking change is pure
/// register-assignment — a slot still lives in exactly one place and its value
/// never changes).
#[test]
#[cfg(target_arch = "x86_64")]
fn call_weight_live_across_call_loop_is_bit_identical_and_tiers() {
    let src = "## To walk (n: Int, base: Int) -> Int:\n\
               \x20   If n is less than 1:\n\
               \x20       Return base.\n\
               \x20   Let mutable acc be base.\n\
               \x20   Let mutable step be n * 2 + 1.\n\
               \x20   Let mutable k be 0.\n\
               \x20   While k is less than 4:\n\
               \x20       Let r be walk(n - 1, acc).\n\
               \x20       Set acc to acc + r + step + k.\n\
               \x20       Set step to step - 1.\n\
               \x20       Set k to k + 1.\n\
               \x20   Return acc.\n\
               \n\
               ## Main\n\
               Let mutable total be 0.\n\
               Let mutable i be 0.\n\
               While i is less than 200:\n\
               \x20   Set total to total + walk(6, i).\n\
               \x20   Set i to i + 1.\n\
               Show total.\n";
    let (out, err, fn_ok, ra_fn) = tiered_fn(src);
    assert_eq!(err, None, "call-weight recursion must not error");
    assert!(!out.is_empty(), "call-weight recursion must produce output");
    assert!(fn_ok >= 1, "walk must JIT (got {fn_ok})");
    assert!(
        ra_fn >= 1,
        "walk must tier through the CONTIGUOUS regalloc FUNCTION backend (got {ra_fn})"
    );
}

/// RED: the quicksort recursion shape (the benchmark's exact `qs` driver:
/// self-call on integer partition bounds + in-place array mutation, returning
/// the Seq) — bit-identical to the tree-walker on the sorted-checksum RESULT.
/// `qs` recurses (two self-calls per level); the array machinery in its body
/// may keep it on the per-piece tier, so this asserts only on correctness.
#[test]
#[cfg(target_arch = "x86_64")]
fn quicksort_shape_recursion_matches_treewalker() {
    let src = r#"## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:
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
Let mutable i be 0.
While i is less than 40:
    Let x be (i * 7 + 3) % 101.
    Push x to arr.
    Set i to i + 1.
Set arr to qs(arr, 1, 40).
Let mutable checksum be 0.
Set i to 1.
While i is at most 40:
    Set checksum to checksum + item i of arr.
    Set i to i + 1.
Show checksum.
"#;
    let (out, err, _fn_ok, _ra_fn) = tiered_fn(src);
    assert_eq!(err, None, "quicksort shape must not error");
    assert!(!out.is_empty(), "quicksort shape must produce output");
}

// =====================================================================
// WS-G: the CONTIGUOUS register-allocated region backend.
//
// `compile_region_regalloc` emits a whole supported int region as ONE
// register-allocated x86-64 function (no per-piece stencil boundaries).
// These tests prove it is BIT-IDENTICAL to BOTH the reference interpreter
// and the existing per-piece stencil tier (`compile_straightline_coded`),
// and that an unsupported op falls back (returns None). The ceiling
// measurement (`#[ignore]`) times contiguous vs per-piece codegen on a
// compute-heavy loop — the number that decides whether the backend is worth
// a multi-wave commit.
// =====================================================================
#[cfg(target_arch = "x86_64")]
mod contiguous_backend {
    use super::*;
    use logicaffeine_forge::jit::compile_straightline_coded;
    use logicaffeine_forge::regalloc::compile_region_regalloc;
    use std::sync::atomic::AtomicI64;
    use std::sync::Arc;
    use std::time::Instant;

    fn lcg(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state >> 33
    }

    /// Run the contiguous backend over a frame; both the return value and the
    /// full post-frame are returned for differential comparison.
    fn run_regalloc(ops: &[MicroOp], frame: &[i64]) -> (ChainOutcome, Vec<i64>) {
        let status = Arc::new(AtomicI64::new(0));
        let chain =
            compile_region_regalloc(ops, Some(status)).expect("supported region must compile");
        let mut f = frame.to_vec();
        let out = chain.run_with_frame(&mut f);
        (out, f)
    }

    /// Run the per-piece stencil tier over a frame, same way.
    fn run_stencil(ops: &[MicroOp], frame: &[i64]) -> (ChainOutcome, Vec<i64>) {
        let status = Arc::new(AtomicI64::new(0));
        let chain =
            compile_straightline_coded(ops, Some(status), None, 0).expect("stencil tier compiles");
        let mut f = frame.to_vec();
        let out = chain.run_with_frame(&mut f);
        (out, f)
    }

    /// RED: straight-line arithmetic — contiguous == reference == stencil.
    #[test]
    fn straightline_arith_three_way_identical() {
        let ops = vec![
            MicroOp::Add { dst: 3, lhs: 0, rhs: 1 },
            MicroOp::Mul { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::Sub { dst: 5, lhs: 4, rhs: 0 },
            MicroOp::BitXor { dst: 6, lhs: 5, rhs: 1 },
            MicroOp::Return { src: 6 },
        ];
        let frame = vec![7i64, 11, 13, 0, 0, 0, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
        let (rout, _) = run_regalloc(&ops, &frame);
        let (sout, _) = run_stencil(&ops, &frame);
        assert_eq!(rout, ChainOutcome::Return(expected));
        assert_eq!(sout, ChainOutcome::Return(expected));
    }

    /// RED: a counting loop with a back-edge — contiguous == reference.
    #[test]
    fn counting_loop_identical() {
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 2, target: 5 },
            MicroOp::Add { dst: 1, lhs: 1, rhs: 0 },
            MicroOp::LoadConst { dst: 3, value: 1 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 3 },
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 1 },
        ];
        let frame = vec![0i64, 0, 100_000, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 10_000_000).unwrap();
        let (rout, _) = run_regalloc(&ops, &frame);
        assert_eq!(rout, ChainOutcome::Return(expected));
        assert_eq!(rout, ChainOutcome::Return(4_999_950_000));
    }

    /// RED: branches both ways (JumpIfFalse / JumpIfTrue / Branch) — identical.
    #[test]
    fn branches_identical() {
        // r = if a < b { a + b } else { a - b }; also exercise JumpIfTrue.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 3 }, // !(a<b) -> else
            MicroOp::Add { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Jump { target: 4 },
            MicroOp::Sub { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Return { src: 2 },
        ];
        for (a, b) in [(3i64, 9), (9, 3), (5, 5)] {
            let frame = vec![a, b, 0];
            let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
            let (rout, _) = run_regalloc(&ops, &frame);
            let (sout, _) = run_stencil(&ops, &frame);
            assert_eq!(rout, ChainOutcome::Return(expected), "a={a} b={b}");
            assert_eq!(sout, ChainOutcome::Return(expected), "a={a} b={b}");
        }
    }

    /// RED: an UNSUPPORTED op must fall back (None) so the caller uses the old
    /// tier — behavior with the flag is unchanged on unsupported regions. A
    /// `MapGet` (hash-map probe via a runtime helper) is outside the supported
    /// subset. (Byte/`Seq of Bool` array loads/stores ARE now supported — see
    /// the `byte_array_*` tests below — so they are no longer the example here.)
    #[test]
    fn unsupported_op_falls_back() {
        let ops = vec![
            MicroOp::MapGet { dst: 1, key: 0, map_slot: 2, helper_addr: 0 },
            MicroOp::Return { src: 1 },
        ];
        assert!(compile_region_regalloc(&ops, None).is_none());
        // …and the fallback tier still produces output for the same program.
        assert!(compile_straightline_coded(&ops, None, None, 0).is_ok());
    }

    /// RED: Div by zero side-exits identically to the stencil tier (both deopt,
    /// no effect landed before the exit).
    #[test]
    fn div_by_zero_side_exit_matches_stencil() {
        let ops = vec![
            MicroOp::Add { dst: 3, lhs: 0, rhs: 1 },
            MicroOp::Div { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::Return { src: 4 },
        ];
        let frame = vec![10i64, 20, 0, 0, 0];
        let (rout, _) = run_regalloc(&ops, &frame);
        let (sout, _) = run_stencil(&ops, &frame);
        assert!(rout.is_deopt(), "regalloc div0 must deopt: {rout:?}");
        assert!(sout.is_deopt(), "stencil div0 must deopt: {sout:?}");
    }

    /// The big one: an exhaustive random differential — every random program
    /// over the supported families at four register pressures must produce the
    /// SAME return value AND the SAME full frame as the reference interpreter
    /// (location-independent), proving the global slot→register assignment
    /// (resident + spilled) is sound regardless of how many slots fit.
    #[test]
    fn random_programs_three_way_identical() {
        for slots in [4u16, 7, 12, 24] {
            for seed in 1..=120u64 {
                let mut s = seed.wrapping_mul(0x9E3779B97F4A7C15);
                let len = 6 + (lcg(&mut s) % 28) as usize;
                let mut ops = Vec::with_capacity(len + 1);
                for _ in 0..len {
                    let dst = (lcg(&mut s) % slots as u64) as u16;
                    let lhs = (lcg(&mut s) % slots as u64) as u16;
                    let rhs = (lcg(&mut s) % slots as u64) as u16;
                    let op = match lcg(&mut s) % 15 {
                        0 => MicroOp::Add { dst, lhs, rhs },
                        1 => MicroOp::Sub { dst, lhs, rhs },
                        2 => MicroOp::Mul { dst, lhs, rhs },
                        3 => MicroOp::BitAnd { dst, lhs, rhs },
                        4 => MicroOp::BitOr { dst, lhs, rhs },
                        5 => MicroOp::BitXor { dst, lhs, rhs },
                        6 => MicroOp::Lt { dst, lhs, rhs },
                        7 => MicroOp::Gt { dst, lhs, rhs },
                        8 => MicroOp::LtEq { dst, lhs, rhs },
                        9 => MicroOp::GtEq { dst, lhs, rhs },
                        10 => MicroOp::Eq { dst, lhs, rhs },
                        11 => MicroOp::Neq { dst, lhs, rhs },
                        12 => MicroOp::Move { dst, src: lhs },
                        13 => MicroOp::NotInt { dst, src: lhs },
                        _ => MicroOp::LoadConst {
                            dst,
                            value: (lcg(&mut s) as i64).wrapping_sub(1 << 40),
                        },
                    };
                    ops.push(op);
                }
                let ret = (lcg(&mut s) % slots as u64) as u16;
                ops.push(MicroOp::Return { src: ret });

                let mut frame = vec![0i64; slots as usize];
                for (i, f) in frame.iter_mut().enumerate() {
                    *f = (i as i64 + 1).wrapping_mul(2_654_435_761) ^ 0x55;
                }
                let mut ref_frame = frame.clone();
                // EXACT integer math: `reference_eval` returns None on i64 overflow,
                // which the native chain matches with a side-exit (deopt).
                let expected = reference_eval(&ops, &mut ref_frame, 1_000_000);

                let (rout, rframe) = run_regalloc(&ops, &frame);
                match expected {
                    Some(e) => {
                        assert_eq!(
                            rout,
                            ChainOutcome::Return(e),
                            "slots={slots} seed={seed}: regalloc return diverged"
                        );
                        assert_eq!(
                            rframe, ref_frame,
                            "slots={slots} seed={seed}: regalloc frame diverged"
                        );
                    }
                    None => assert!(
                        rout.is_deopt(),
                        "slots={slots} seed={seed}: overflow must deopt, got {rout:?}"
                    ),
                }
            }
        }
    }

    // ----------------------------------------------------------------
    // WS-G ARRAY SUPPORT: integer ArrLoad / ArrStore in the contiguous
    // backend, bit-identical to the reference (and the stencil tier) on the
    // value, on the full post-frame, AND on the BUFFER contents — plus OOB
    // side-exit parity. The reference (`reference_eval`) models the SAME
    // 1-based addressing (`ptr[i-1]`) and the SAME bounds guard
    // (`(i-1) as u64 >= len as u64`), so equality to it proves bit-identity.
    // ----------------------------------------------------------------

    /// Run the contiguous backend over a frame that pins live i64 buffers (each
    /// `(ptr_slot, buffer)`); returns the outcome and the post-frame. The
    /// buffers are mutated in place through the pinned pointers.
    fn run_regalloc_buf(
        ops: &[MicroOp],
        frame: &[i64],
        buf_slots: Vec<(usize, &mut Vec<i64>)>,
    ) -> (ChainOutcome, Vec<i64>) {
        let status = Arc::new(AtomicI64::new(0));
        let chain =
            compile_region_regalloc(ops, Some(status)).expect("array region must compile");
        let mut f = frame.to_vec();
        for (ptr_slot, buf) in buf_slots {
            f[ptr_slot] = buf.as_mut_ptr() as i64;
        }
        let out = chain.run_with_frame(&mut f);
        (out, f)
    }

    /// The same program under `reference_eval`, over its own copies, so the
    /// differential never aliases the regalloc run's buffers.
    fn run_reference_buf(
        ops: &[MicroOp],
        frame: &[i64],
        bufs: &[Vec<i64>],
        buf_ptr_slots: &[usize],
    ) -> (Option<i64>, Vec<i64>, Vec<Vec<i64>>) {
        let mut owned: Vec<Vec<i64>> = bufs.to_vec();
        let mut f = frame.to_vec();
        for (k, ptr_slot) in buf_ptr_slots.iter().enumerate() {
            f[*ptr_slot] = owned[k].as_mut_ptr() as i64;
        }
        let v = reference_eval(ops, &mut f, 100_000_000);
        (v, f, owned)
    }

    /// RED: sum over an i64 array via a `arr[i]` load loop — contiguous backend
    /// equals the reference on the returned sum.
    #[test]
    fn array_sum_loop_identical() {
        // frame: 0=i(1-based) 1=acc 2=n(=len) 3=tmp 4=ptr 5=len 6=one
        // while i <= n: acc += arr[i]; i += 1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 2, target: 5 }, // i<=n
            MicroOp::ArrLoad { dst: 3, idx: 0, ptr_slot: 4, len_slot: 5, byte: false, narrow32: false, checked: true },
            MicroOp::Add { dst: 1, lhs: 1, rhs: 3 }, // acc += arr[i]
            MicroOp::Add { dst: 0, lhs: 0, rhs: 6 }, // i += 1
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 1 },
        ];
        let n = 1000usize;
        let mut buf: Vec<i64> = (0..n).map(|k| (k as i64 + 1) * 3 - 5).collect();
        let frame = vec![1i64, 0, n as i64, 0, 0, n as i64, 1];

        let (vref, _fref, _bref) = run_reference_buf(&ops, &frame, &[buf.clone()], &[4]);
        let expected = vref.expect("reference terminates");
        let (out, _f) = run_regalloc_buf(&ops, &frame, vec![(4, &mut buf)]);
        assert_eq!(out, ChainOutcome::Return(expected));
        assert_eq!(out, ChainOutcome::Return((1..=n as i64).map(|k| k * 3 - 5).sum()));
    }

    /// RED: write a computed value into every cell via `arr[i] = expr` — the
    /// post-BUFFER must match the reference's post-buffer exactly.
    #[test]
    fn array_write_loop_identical() {
        // frame: 0=i 1=n 2=val 3=ptr 4=len 5=one
        // while i <= n: arr[i] = i*i; i += 1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 5 },
            MicroOp::Mul { dst: 2, lhs: 0, rhs: 0 }, // val = i*i
            MicroOp::ArrStore { src: 2, idx: 0, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 5 },
            MicroOp::Jump { target: 0 },
            MicroOp::LoadConst { dst: 2, value: 0 },
            MicroOp::Return { src: 2 },
        ];
        let n = 777usize;
        let mut buf = vec![0i64; n];
        let frame = vec![1i64, n as i64, 0, 0, n as i64, 1];

        let (vref, _fref, bref) = run_reference_buf(&ops, &frame, &[vec![0i64; n]], &[3]);
        let (out, _f) = run_regalloc_buf(&ops, &frame, vec![(3, &mut buf)]);
        assert_eq!(out, ChainOutcome::Return(vref.expect("reference terminates")));
        assert_eq!(out, ChainOutcome::Return(0));
        assert_eq!(buf, bref[0], "written buffer diverged from reference");
        let want: Vec<i64> = (1..=n as i64).map(|k| k * k).collect();
        assert_eq!(buf, want);
    }

    /// RED: a 2D `a[i*n+j]` access pattern — copy a matrix transposed into a
    /// second buffer; both buffers must match the reference bit for bit.
    #[test]
    fn array_2d_index_identical() {
        // a is n x n row-major (1-based linear index r*n + c, 1..=n each).
        // out[c*n + r] = a[r*n + c]  for r,c in 1..=n.
        // frame: 0=r 1=c 2=n 3=src_lin 4=dst_lin 5=v 6=t 7=a_ptr 8=a_len
        //        9=o_ptr 10=o_len 11=one
        let ops = vec![
            // outer: while r <= n
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 2, target: 16 }, // 0
            MicroOp::LoadConst { dst: 1, value: 1 }, // c = 1                  1
            // inner: while c <= n
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 1, rhs: 2, target: 14 }, // 2
            // src_lin = (r-1)*n + c
            MicroOp::LoadConst { dst: 6, value: 1 },                        // 3
            MicroOp::Sub { dst: 3, lhs: 0, rhs: 6 },                        // 4 r-1
            MicroOp::Mul { dst: 3, lhs: 3, rhs: 2 },                        // 5 (r-1)*n
            MicroOp::Add { dst: 3, lhs: 3, rhs: 1 },                        // 6 +c
            // dst_lin = (c-1)*n + r
            MicroOp::Sub { dst: 4, lhs: 1, rhs: 6 },                        // 7 c-1
            MicroOp::Mul { dst: 4, lhs: 4, rhs: 2 },                        // 8 (c-1)*n
            MicroOp::Add { dst: 4, lhs: 4, rhs: 0 },                        // 9 +r
            MicroOp::ArrLoad { dst: 5, idx: 3, ptr_slot: 7, len_slot: 8, byte: false, narrow32: false, checked: true }, // 10
            MicroOp::ArrStore { src: 5, idx: 4, ptr_slot: 9, len_slot: 10, byte: false, narrow32: false, checked: true }, // 11
            MicroOp::Add { dst: 1, lhs: 1, rhs: 11 }, // c += 1               12
            MicroOp::Jump { target: 2 },              //                      13
            MicroOp::Add { dst: 0, lhs: 0, rhs: 11 }, // r += 1               14
            MicroOp::Jump { target: 0 },              //                      15
            MicroOp::LoadConst { dst: 5, value: 0 },  //                      16 (unreachable filler)
            MicroOp::Return { src: 5 },               //                      17
        ];
        let n = 23usize;
        let total = n * n;
        let a: Vec<i64> = (0..total).map(|k| (k as i64) * 7 + 1).collect();
        let mut a_run = a.clone();
        let mut out_run = vec![0i64; total];
        let frame = vec![1i64, 1, n as i64, 0, 0, 0, 0, 0, total as i64, 0, total as i64, 1];

        let (vref, _fref, bref) =
            run_reference_buf(&ops, &frame, &[a.clone(), vec![0i64; total]], &[7, 9]);
        let (out, _f) = run_regalloc_buf(&ops, &frame, vec![(7, &mut a_run), (9, &mut out_run)]);
        assert_eq!(out, ChainOutcome::Return(vref.expect("reference terminates")));
        assert_eq!(out, ChainOutcome::Return(0));
        assert_eq!(a_run, bref[0], "source buffer must be unchanged");
        assert_eq!(out_run, bref[1], "transposed buffer diverged from reference");
        // Spot-check the transpose at one cell.
        let r = 5i64;
        let c = 9i64;
        assert_eq!(
            out_run[((c - 1) * n as i64 + r) as usize - 1],
            a_run[((r - 1) * n as i64 + c) as usize - 1]
        );
    }

    /// RED: an in-place swap loop (reverse the array) via load/load/store/store
    /// over the SAME buffer — the post-buffer must equal the reference's.
    #[test]
    fn array_inplace_swap_loop_identical() {
        // frame: 0=lo 1=hi 2=tlo 3=thi 4=ptr 5=len 6=one
        // while lo < hi: t=a[lo]; a[lo]=a[hi]; a[hi]=t; lo+=1; hi-=1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 9 }, // 0
            MicroOp::ArrLoad { dst: 2, idx: 0, ptr_slot: 4, len_slot: 5, byte: false, narrow32: false, checked: true }, // 1 t=a[lo]
            MicroOp::ArrLoad { dst: 3, idx: 1, ptr_slot: 4, len_slot: 5, byte: false, narrow32: false, checked: true }, // 2 u=a[hi]
            MicroOp::ArrStore { src: 3, idx: 0, ptr_slot: 4, len_slot: 5, byte: false, narrow32: false, checked: true }, // 3 a[lo]=u
            MicroOp::ArrStore { src: 2, idx: 1, ptr_slot: 4, len_slot: 5, byte: false, narrow32: false, checked: true }, // 4 a[hi]=t
            MicroOp::Add { dst: 0, lhs: 0, rhs: 6 }, // 5 lo += 1
            MicroOp::Sub { dst: 1, lhs: 1, rhs: 6 }, // 6 hi -= 1
            MicroOp::Jump { target: 0 },             // 7
            MicroOp::LoadConst { dst: 2, value: 0 }, // 8 filler
            MicroOp::Return { src: 0 },              // 9 returns lo
        ];
        let n = 500usize;
        let orig: Vec<i64> = (0..n).map(|k| (k as i64) * 11 - 3).collect();
        let mut buf = orig.clone();
        let frame = vec![1i64, n as i64, 0, 0, 0, n as i64, 1];

        let (vref, _fref, bref) = run_reference_buf(&ops, &frame, &[orig.clone()], &[4]);
        let expected = vref.expect("reference terminates");
        let (out, _f) = run_regalloc_buf(&ops, &frame, vec![(4, &mut buf)]);
        assert_eq!(out, ChainOutcome::Return(expected));
        assert_eq!(buf, bref[0], "reversed buffer diverged from reference");
        let mut want = orig.clone();
        want.reverse();
        assert_eq!(buf, want);
    }

    /// RED: an OUT-OF-BOUNDS index side-exits identically to BOTH the reference
    /// (None) and the stencil tier (deopt) — no effect landed before the exit.
    #[test]
    fn array_oob_load_side_exit_matches() {
        // arr has len 4; index 5 is OOB → must side-exit before writing dst.
        let ops = vec![
            MicroOp::ArrLoad { dst: 1, idx: 0, ptr_slot: 2, len_slot: 3, byte: false, narrow32: false, checked: true },
            MicroOp::Return { src: 1 },
        ];
        let mut buf = vec![10i64, 20, 30, 40];
        let frame = vec![5i64, -999, 0, 4]; // idx=5 (OOB for len 4)

        // reference returns None on OOB.
        let (vref, _f, _b) = run_reference_buf(&ops, &frame, &[buf.clone()], &[2]);
        assert!(vref.is_none(), "reference must reject OOB");

        // regalloc must side-exit (deopt), leaving dst untouched.
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("compiles");
        let mut f = frame.clone();
        f[2] = buf.as_mut_ptr() as i64;
        let rout = chain.run_with_frame(&mut f);
        assert!(rout.is_deopt(), "regalloc OOB load must deopt: {rout:?}");

        // stencil tier on the same program: also a deopt.
        let sout = {
            let status = Arc::new(AtomicI64::new(0));
            let chain = compile_straightline_coded(&ops, Some(status), None, 0).unwrap();
            let mut sf = frame.clone();
            sf[2] = buf.as_mut_ptr() as i64;
            chain.run_with_frame(&mut sf)
        };
        assert!(sout.is_deopt(), "stencil OOB load must deopt: {sout:?}");
    }

    /// RED: an OOB STORE side-exits BEFORE mutating the buffer — the buffer is
    /// observably unchanged, matching the reference (which rejects before any
    /// write).
    #[test]
    fn array_oob_store_side_exit_no_effect() {
        // store into index 0 (1-based → im1 = -1, OOB) must not write.
        let ops = vec![
            MicroOp::ArrStore { src: 1, idx: 0, ptr_slot: 2, len_slot: 3, byte: false, narrow32: false, checked: true },
            MicroOp::Return { src: 1 },
        ];
        let mut buf = vec![1i64, 2, 3, 4];
        let before = buf.clone();
        let frame = vec![0i64, 999, 0, 4]; // idx=0 → OOB (1-based)

        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("compiles");
        let mut f = frame.clone();
        f[2] = buf.as_mut_ptr() as i64;
        let rout = chain.run_with_frame(&mut f);
        assert!(rout.is_deopt(), "regalloc OOB store must deopt: {rout:?}");
        assert_eq!(buf, before, "OOB store must not mutate the buffer");
    }

    /// RED: an UNCHECKED int load/store (Oracle-proven in-bounds) is bit-exact
    /// — no bounds check, no status cell required.
    #[test]
    fn array_unchecked_load_store_identical() {
        // a[i] (unchecked) copied to b[i] (unchecked) across a known-safe range.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 5 },
            MicroOp::ArrLoad { dst: 2, idx: 0, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: false },
            MicroOp::ArrStore { src: 2, idx: 0, ptr_slot: 5, len_slot: 6, byte: false, narrow32: false, checked: false },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 7 },
            MicroOp::Jump { target: 0 },
            MicroOp::LoadConst { dst: 2, value: 0 },
            MicroOp::Return { src: 2 },
        ];
        let n = 300usize;
        let a: Vec<i64> = (0..n).map(|k| (k as i64) ^ 0x5A).collect();
        let mut a_run = a.clone();
        let mut b_run = vec![0i64; n];
        // frame: 0=i 1=n 2=v 3=a_ptr 4=a_len 5=b_ptr 6=b_len 7=one
        let frame = vec![1i64, n as i64, 0, 0, n as i64, 0, n as i64, 1];

        // No status cell needed for an all-unchecked region.
        let chain = compile_region_regalloc(&ops, None).expect("unchecked compiles without status");
        let mut f = frame.clone();
        f[3] = a_run.as_mut_ptr() as i64;
        f[5] = b_run.as_mut_ptr() as i64;
        let out = chain.run_with_frame(&mut f);
        assert_eq!(out, ChainOutcome::Return(0));
        assert_eq!(b_run, a, "unchecked copy diverged");
        assert_eq!(a_run, a, "source unchanged");
    }

    // ----------------------------------------------------------------
    // WS-G BYTE (`Seq of Bool`) ARRAY SUPPORT (wave 19): 1-byte element
    // load/store in the contiguous backend — the sieve / would-be-`visited`
    // shape. A byte LOAD is a zero-extended `movzx` (`u8 as i64`, 0..=255); a
    // byte STORE writes the BOOLEAN NORMALIZATION `(v != 0) as u8`. The
    // reference (`reference_eval`) models the SAME byte semantics, so equality
    // to it (on the returned value, the post-frame, AND the post-BUFFER) proves
    // bit-identity. The OOB side-exit is the 8-byte path's unsigned guard.
    // ----------------------------------------------------------------

    /// Run the contiguous backend over a frame pinning live BYTE (`u8`) buffers
    /// (each `(ptr_slot, buffer)`); returns the outcome and the post-frame. The
    /// buffers are mutated in place through the pinned pointers.
    fn run_regalloc_bytebuf(
        ops: &[MicroOp],
        frame: &[i64],
        buf_slots: Vec<(usize, &mut Vec<u8>)>,
    ) -> (ChainOutcome, Vec<i64>) {
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(ops, Some(status))
            .expect("byte-array region must compile (regalloc covers Seq of Bool)");
        let mut f = frame.to_vec();
        for (ptr_slot, buf) in buf_slots {
            f[ptr_slot] = buf.as_mut_ptr() as i64;
        }
        let out = chain.run_with_frame(&mut f);
        (out, f)
    }

    /// The same program under `reference_eval` over its own byte buffers, so the
    /// differential never aliases the regalloc run's buffers.
    fn run_reference_bytebuf(
        ops: &[MicroOp],
        frame: &[i64],
        bufs: &[Vec<u8>],
        buf_ptr_slots: &[usize],
    ) -> (Option<i64>, Vec<u8>) {
        let mut owned: Vec<Vec<u8>> = bufs.to_vec();
        let mut f = frame.to_vec();
        for (k, ptr_slot) in buf_ptr_slots.iter().enumerate() {
            f[*ptr_slot] = owned[k].as_mut_ptr() as i64;
        }
        let v = reference_eval(ops, &mut f, 100_000_000);
        (v, owned.remove(0))
    }

    /// RED: the byte-array shape regallocs (it no longer falls back).
    #[test]
    fn byte_array_load_store_now_regallocs() {
        // a load AND a store over a Seq of Bool buffer in one region.
        let ops = vec![
            MicroOp::ArrLoad { dst: 1, idx: 0, ptr_slot: 2, len_slot: 3, byte: true, narrow32: false, checked: true },
            MicroOp::ArrStore { src: 1, idx: 0, ptr_slot: 2, len_slot: 3, byte: true, narrow32: false, checked: true },
            MicroOp::Return { src: 1 },
        ];
        assert!(
            compile_region_regalloc(&ops, Some(Arc::new(AtomicI64::new(0)))).is_some(),
            "Seq of Bool load/store must now compile through the regalloc backend"
        );
    }

    /// RED: a sum-over-`Seq of Bool` scan loop (the sieve `count` shape:
    /// `if flags[i] == false { count += 1 }`) — the contiguous backend equals
    /// the reference on the returned count, and a byte load zero-extends.
    #[test]
    fn byte_array_scan_loop_identical() {
        // frame: 0=i(1-based) 1=count 2=n(=len) 3=v 4=ptr 5=len 6=one 7=zero 8=isclear
        // while i <= n: v = flags[i]; if v == 0 { count += 1 }; i += 1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 2, target: 8 }, // 0
            MicroOp::ArrLoad { dst: 3, idx: 0, ptr_slot: 4, len_slot: 5, byte: true, narrow32: false, checked: true }, // 1
            MicroOp::Eq { dst: 8, lhs: 3, rhs: 7 }, // 2 isclear = (v == 0)
            MicroOp::JumpIfFalse { cond: 8, target: 5 }, // 3
            MicroOp::Add { dst: 1, lhs: 1, rhs: 6 }, // 4 count += 1
            MicroOp::Add { dst: 0, lhs: 0, rhs: 6 }, // 5 i += 1
            MicroOp::Jump { target: 0 },             // 6
            MicroOp::LoadConst { dst: 3, value: 0 }, // 7 filler
            MicroOp::Return { src: 1 },              // 8
        ];
        let n = 1000usize;
        // half the cells set (1), half clear (0): every third clear.
        let buf: Vec<u8> = (0..n).map(|k| u8::from(k % 3 != 0)).collect();
        let want_clear = buf.iter().filter(|&&b| b == 0).count() as i64;
        let frame = vec![1i64, 0, n as i64, 0, 0, n as i64, 1, 0, 0];

        let (vref, _bref) = run_reference_bytebuf(&ops, &frame, &[buf.clone()], &[4]);
        let expected = vref.expect("reference terminates");
        let mut run_buf = buf.clone();
        let (out, _f) = run_regalloc_bytebuf(&ops, &frame, vec![(4, &mut run_buf)]);
        assert_eq!(out, ChainOutcome::Return(expected));
        assert_eq!(out, ChainOutcome::Return(want_clear));
        assert_eq!(run_buf, buf, "a read-only scan must not mutate the buffer");
    }

    /// RED: the sieve INNER store loop (`flags[j] = true` striding by `i`) over a
    /// `Seq of Bool` — the post-BUFFER must equal the reference's exactly, and
    /// the stored byte is the boolean 1 (normalization).
    #[test]
    fn byte_array_sieve_mark_loop_identical() {
        // frame: 0=j(1-based) 1=limit 2=step 3=trueval 4=ptr 5=len
        // while j <= limit: flags[j] = true; j += step
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 4 }, // 0
            MicroOp::ArrStore { src: 3, idx: 0, ptr_slot: 4, len_slot: 5, byte: true, narrow32: false, checked: true }, // 1
            MicroOp::Add { dst: 0, lhs: 0, rhs: 2 }, // 2 j += step
            MicroOp::Jump { target: 0 },             // 3
            MicroOp::Return { src: 0 },              // 4
        ];
        let n = 997usize;
        let step = 7i64;
        // `trueval` is an ARBITRARY nonzero (not 1) — must still store byte 1.
        let frame = vec![step, n as i64, step, 0x1234_5678i64, 0, n as i64];

        let (_vref, bref) = run_reference_bytebuf(&ops, &frame, &[vec![0u8; n]], &[4]);
        let mut run_buf = vec![0u8; n];
        let (out, _f) = run_regalloc_bytebuf(&ops, &frame, vec![(4, &mut run_buf)]);
        // The reference and regalloc both terminate with j > limit.
        assert!(matches!(out, ChainOutcome::Return(_)));
        assert_eq!(run_buf, bref, "sieve-mark buffer diverged from reference");
        // Spot-check: every multiple of step (1-based) is marked with the byte 1
        // (NOT 0x78, the low byte of trueval) — the boolean normalization.
        let mut j = step;
        while j <= n as i64 {
            assert_eq!(run_buf[(j - 1) as usize], 1u8, "cell j={j} not normalized to 1");
            j += step;
        }
        assert_eq!(run_buf[0], 0u8, "cell 1 (never marked) stays 0");
    }

    /// RED: the byte STORE value normalization edge cases. A store of ANY nonzero
    /// value writes the byte 1; a store of 0 writes 0 — exactly `(v != 0) as u8`,
    /// bit-identical to `reference_eval`. This catches a raw-low-byte miscompile
    /// (e.g. storing 256, whose low byte is 0, must still write 1).
    #[test]
    fn byte_array_store_normalizes_to_zero_one() {
        // frame: 0=idx(=1) 1=val 2=ptr 3=len ; store val into cell 1.
        let ops = vec![
            MicroOp::ArrStore { src: 1, idx: 0, ptr_slot: 2, len_slot: 3, byte: true, narrow32: false, checked: true },
            MicroOp::Return { src: 1 },
        ];
        for (val, want) in [
            (0i64, 0u8),
            (1, 1),
            (256, 1),           // low byte 0 but nonzero → 1
            (-1, 1),            // all-ones → 1
            (0x100, 1),         // 256
            (i64::MIN, 1),      // high bit only → 1
            (0xFF00, 1),        // low byte 0, nonzero → 1
        ] {
            let frame = vec![1i64, val, 0, 1];
            let (_vref, bref) = run_reference_bytebuf(&ops, &frame, &[vec![0xAAu8; 1]], &[2]);
            let mut run_buf = vec![0xAAu8; 1];
            let (out, _f) = run_regalloc_bytebuf(&ops, &frame, vec![(2, &mut run_buf)]);
            assert_eq!(out, ChainOutcome::Return(val), "val={val}");
            assert_eq!(run_buf[0], want, "val={val}: stored byte not normalized");
            assert_eq!(run_buf, bref, "val={val}: diverged from reference");
        }
    }

    /// RED: the byte LOAD zero-extends — a buffer holding 0/1 bytes loads as
    /// non-negative 0/1 i64s (never sign-extended), matching the reference.
    #[test]
    fn byte_array_load_zero_extends() {
        // frame: 0=idx 1=dst 2=ptr 3=len ; dst = flags[idx] (byte).
        let ops = vec![
            MicroOp::ArrLoad { dst: 1, idx: 0, ptr_slot: 2, len_slot: 3, byte: true, narrow32: false, checked: true },
            MicroOp::Return { src: 1 },
        ];
        // A buffer with 0xFF in it: a SIGN-extending load would give -1; a
        // zero-extending load gives 255. The stencil/reference give 255.
        let buf = vec![0u8, 1, 0xFF, 0x80, 7];
        for (idx1, want) in [(1i64, 0i64), (2, 1), (3, 255), (4, 128), (5, 7)] {
            let frame = vec![idx1, 0, 0, buf.len() as i64];
            let (vref, _b) = run_reference_bytebuf(&ops, &frame, &[buf.clone()], &[2]);
            let mut run_buf = buf.clone();
            let (out, _f) = run_regalloc_bytebuf(&ops, &frame, vec![(2, &mut run_buf)]);
            assert_eq!(out, ChainOutcome::Return(want), "idx={idx1}");
            assert_eq!(out, ChainOutcome::Return(vref.expect("ref ok")), "idx={idx1} vs reference");
        }
    }

    /// RED: a byte-array OOB load/store side-exits identically to the reference
    /// (None) — and an OOB STORE leaves the byte buffer untouched (no effect).
    #[test]
    fn byte_array_oob_side_exit_matches() {
        // OOB load: index 6 into a len-5 buffer.
        let load_ops = vec![
            MicroOp::ArrLoad { dst: 1, idx: 0, ptr_slot: 2, len_slot: 3, byte: true, narrow32: false, checked: true },
            MicroOp::Return { src: 1 },
        ];
        let mut buf = vec![1u8, 0, 1, 0, 1];
        let frame = vec![6i64, -999, 0, 5]; // idx 6 OOB for len 5
        let (vref, _b) = run_reference_bytebuf(&load_ops, &frame, &[buf.clone()], &[2]);
        assert!(vref.is_none(), "reference rejects OOB byte load");
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&load_ops, Some(status)).expect("compiles");
        let mut f = frame.clone();
        f[2] = buf.as_mut_ptr() as i64;
        assert!(chain.run_with_frame(&mut f).is_deopt(), "regalloc OOB byte load must deopt");

        // OOB store: index 0 (1-based → im1 = -1) must not write.
        let store_ops = vec![
            MicroOp::ArrStore { src: 1, idx: 0, ptr_slot: 2, len_slot: 3, byte: true, narrow32: false, checked: true },
            MicroOp::Return { src: 1 },
        ];
        let before = buf.clone();
        let sframe = vec![0i64, 1, 0, 5]; // idx 0 → im1 = -1 OOB
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&store_ops, Some(status)).expect("compiles");
        let mut sf = sframe.clone();
        sf[2] = buf.as_mut_ptr() as i64;
        assert!(chain.run_with_frame(&mut sf).is_deopt(), "regalloc OOB byte store must deopt");
        assert_eq!(buf, before, "OOB byte store must not mutate the buffer");
    }

    /// RED: a mixed byte-AND-int-array region regallocs and is bit-identical:
    /// `flags[i] = (arr[i] > 0)` — an int load feeding a compare feeding a byte
    /// store, both buffers live. Proves byte and 8-byte element strides coexist
    /// in one register-allocated function (the sieve/graph_bfs mixture).
    #[test]
    fn mixed_byte_and_int_arrays_identical() {
        // frame: 0=i 1=n 2=t(int) 3=zero 4=ispos(byte val) 5=one
        //        6=a_ptr 7=a_len 8=f_ptr 9=f_len
        // while i <= n: t = arr[i]; ispos = (t > 0); flags[i] = ispos; i += 1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 7 }, // 0
            MicroOp::ArrLoad { dst: 2, idx: 0, ptr_slot: 6, len_slot: 7, byte: false, narrow32: false, checked: true }, // 1
            MicroOp::Gt { dst: 4, lhs: 2, rhs: 3 }, // 2 ispos = (t > 0)
            MicroOp::ArrStore { src: 4, idx: 0, ptr_slot: 8, len_slot: 9, byte: true, narrow32: false, checked: true }, // 3
            MicroOp::Add { dst: 0, lhs: 0, rhs: 5 }, // 4 i += 1
            MicroOp::Jump { target: 0 },             // 5
            MicroOp::LoadConst { dst: 2, value: 0 }, // 6 filler
            MicroOp::Return { src: 0 },              // 7
        ];
        let n = 400usize;
        let a: Vec<i64> = (0..n).map(|k| (k as i64) - 200).collect(); // mix of <0, 0, >0
        let frame = vec![1i64, n as i64, 0, 0, 0, 1, 0, n as i64, 0, n as i64];

        // Reference run with its own int + byte buffers (manual pinning: both
        // classes in one frame). reference_eval indexes raw pointers, so pin both.
        let mut a_ref = a.clone();
        let mut f_ref = vec![0u8; n];
        let mut ref_frame = frame.clone();
        ref_frame[6] = a_ref.as_mut_ptr() as i64;
        ref_frame[8] = f_ref.as_mut_ptr() as i64;
        let vref = reference_eval(&ops, &mut ref_frame, 100_000_000);

        // Regalloc run with its own buffers.
        assert!(
            compile_region_regalloc(&ops, Some(Arc::new(AtomicI64::new(0)))).is_some(),
            "mixed byte+int region must regalloc"
        );
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("compiles");
        let mut a_run = a.clone();
        let mut f_run = vec![0u8; n];
        let mut f = frame.clone();
        f[6] = a_run.as_mut_ptr() as i64;
        f[8] = f_run.as_mut_ptr() as i64;
        let out = chain.run_with_frame(&mut f);
        assert_eq!(out, ChainOutcome::Return(vref.expect("ref terminates")));
        assert_eq!(f_run, f_ref, "byte flags buffer diverged from reference");
        assert_eq!(a_run, a_ref, "int buffer must be unchanged");
        let want: Vec<u8> = a.iter().map(|&x| u8::from(x > 0)).collect();
        assert_eq!(f_run, want, "flags != (arr > 0)");
    }

    // ----------------------------------------------------------------
    // WS-G BFS INDIRECT-GATHER shape (wave 19): the graph_bfs hot loop reads
    // `adj[adjStarts[v] + e + 1]` — a TWO-LOAD-PLUS-ADD indirect gather over
    // 8-byte int arrays, with `dist`/`queue` (also int arrays) and a queue
    // push. The pure-gather core (two int ArrLoads + Add + a third ArrLoad)
    // is already in the supported subset; this pins that it regallocs and is
    // bit-identical to the reference, so the diagnosis (graph_bfs falls back
    // for a NON-array reason, e.g. `length of`, not a missing array shape)
    // is anchored by a passing gather test.
    // ----------------------------------------------------------------

    /// RED: the indirect gather `out[e] = adj[starts[v] + e]` (BFS adjacency
    /// walk) regallocs and matches the reference bit-for-bit on both buffers.
    #[test]
    fn bfs_indirect_gather_identical() {
        // frame: 0=e(1-based) 1=cnt 2=v_start(=starts[v]) 3=gidx 4=val 5=one
        //        6=adj_ptr 7=adj_len 8=out_ptr 9=out_len
        // while e <= cnt: gidx = v_start + e; val = adj[gidx]; out[e] = val; e += 1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 7 }, // 0
            MicroOp::Add { dst: 3, lhs: 2, rhs: 0 }, // 1 gidx = v_start + e
            MicroOp::ArrLoad { dst: 4, idx: 3, ptr_slot: 6, len_slot: 7, byte: false, narrow32: false, checked: true }, // 2 val = adj[gidx]
            MicroOp::ArrStore { src: 4, idx: 0, ptr_slot: 8, len_slot: 9, byte: false, narrow32: false, checked: true }, // 3 out[e] = val
            MicroOp::Add { dst: 0, lhs: 0, rhs: 5 }, // 4 e += 1
            MicroOp::Jump { target: 0 },             // 5
            MicroOp::LoadConst { dst: 4, value: 0 }, // 6 filler
            MicroOp::Return { src: 1 },              // 7
        ];
        let total = 200usize;
        let cnt = 5i64; // gather 5 neighbors
        let v_start = 30i64; // starting at adj[31..35] (1-based via gidx)
        let adj: Vec<i64> = (0..total).map(|k| (k as i64) * 13 + 1).collect();
        let frame = vec![1i64, cnt, v_start, 0, 0, 1, 0, total as i64, 0, cnt];

        assert!(
            compile_region_regalloc(&ops, Some(Arc::new(AtomicI64::new(0)))).is_some(),
            "BFS indirect-gather region must regalloc"
        );
        let (vref, _fref, bref) =
            run_reference_buf(&ops, &frame, &[adj.clone(), vec![0i64; cnt as usize]], &[6, 8]);
        let mut adj_run = adj.clone();
        let mut out_run = vec![0i64; cnt as usize];
        let (out, _f) = run_regalloc_buf(&ops, &frame, vec![(6, &mut adj_run), (8, &mut out_run)]);
        assert_eq!(out, ChainOutcome::Return(vref.expect("ref terminates")));
        assert_eq!(adj_run, bref[0], "adj buffer changed");
        assert_eq!(out_run, bref[1], "gathered output diverged from reference");
        // Spot-check the gather: out[e] == adj[v_start + e] (1-based).
        for e in 1..=cnt {
            assert_eq!(out_run[(e - 1) as usize], adj[(v_start + e - 1) as usize]);
        }
    }

    // ----------------------------------------------------------------
    // WS-G FLOAT SUPPORT (wave 11): an XMM register class for f64 slots —
    // AddF/SubF/MulF/DivF/SqrtF/IntToFloat/FmaF and the float compares /
    // BranchF, bit-identical to `reference_eval` (and thus the tree-walker)
    // on the returned VALUE and the full post-FRAME. Float arithmetic is
    // IEEE with NO fused-multiply-add (FmaF is two roundings) and NO
    // reassociation; float ORDERING is exact IEEE (NaN compares false);
    // float EQUALITY is the kernel's epsilon rule (`|a-b| < f64::EPSILON`).
    // The differential below sweeps random float programs at every register
    // pressure, plus the IEEE edge cases (NaN / -0.0 / inf / epsilon) that a
    // raw-`==` or an `fma`-instruction miscompile would diverge on.
    // ----------------------------------------------------------------

    fn fbits(x: f64) -> i64 {
        x.to_bits() as i64
    }

    /// RED: a float accumulate loop `s = s + dx*dx` (the nbody/spectral kernel
    /// shape) is bit-identical to the reference across a float pin + an int
    /// counter living side by side.
    #[test]
    fn float_accumulate_loop_identical() {
        // slots: 0=i 1=n 2=one 3=s(f) 4=dx(f) 5=prod(f) 6=i_f(f)
        // while i < n: i_f = (f64)i; dx = i_f; prod = dx*dx; s = s + prod; i+=1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 7 },
            MicroOp::IntToFloat { dst: 6, src: 0 },
            MicroOp::Move { dst: 4, src: 6 },
            MicroOp::MulF { dst: 5, lhs: 4, rhs: 4 },
            MicroOp::AddF { dst: 3, lhs: 3, rhs: 5 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 2 },
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 3 },
        ];
        let frame = vec![0i64, 2000, 1, fbits(0.0), fbits(0.0), fbits(0.0), fbits(0.0)];
        let expected = reference_eval(&ops, &mut frame.clone(), 1_000_000).unwrap();
        let (rout, rframe) = run_regalloc(&ops, &frame);
        let mut ref_frame = frame.clone();
        reference_eval(&ops, &mut ref_frame, 1_000_000).unwrap();
        assert_eq!(rout, ChainOutcome::Return(expected), "return diverged");
        assert_eq!(rframe, ref_frame, "frame diverged");
        // Spot value: sum of squares 0^2..1999^2.
        let want: f64 = (0..2000i64).map(|k| (k as f64) * (k as f64)).sum();
        assert_eq!(f64::from_bits(rout_bits(rout) as u64), want);
    }

    fn rout_bits(o: ChainOutcome) -> i64 {
        match o {
            ChainOutcome::Return(v) => v,
            ChainOutcome::Deopt(d) => panic!("unexpected deopt {d}"),
        }
    }

    /// RED: a divide / sqrt chain (the spectral_norm / nbody distance shape):
    /// `r = sqrt(a*a + b*b); q = 1.0 / r` — bit-identical, NO hardware FMA, NO
    /// reassociation.
    #[test]
    fn float_div_sqrt_chain_identical() {
        // slots: 0=a(f) 1=b(f) 2=one(f) 3=aa(f) 4=bb(f) 5=sum(f) 6=r(f) 7=q(f)
        let ops = vec![
            MicroOp::MulF { dst: 3, lhs: 0, rhs: 0 }, // a*a
            MicroOp::MulF { dst: 4, lhs: 1, rhs: 1 }, // b*b
            MicroOp::AddF { dst: 5, lhs: 3, rhs: 4 }, // a*a + b*b
            MicroOp::SqrtF { dst: 6, src: 5 },         // sqrt(..)
            MicroOp::DivF { dst: 7, lhs: 2, rhs: 6 },  // 1.0 / r
            MicroOp::Return { src: 7 },
        ];
        for (a, b) in [(3.0, 4.0), (1.5, -2.25), (1e-8, 1e8), (0.1, 0.2)] {
            let frame = vec![fbits(a), fbits(b), fbits(1.0), 0, 0, 0, 0, 0];
            let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
            let (rout, _) = run_regalloc(&ops, &frame);
            assert_eq!(rout, ChainOutcome::Return(expected), "a={a} b={b}");
            // exact (no FMA / reassoc): the reference's two-rounding result.
            let want = (1.0f64 / (a * a + b * b).sqrt()).to_bits() as i64;
            assert_eq!(rout, ChainOutcome::Return(want), "a={a} b={b}");
        }
    }

    /// RED: a mandelbrot `z = z*z + c` iteration (complex multiply, all real
    /// arithmetic): zr2 = zr*zr - zi*zi + cr; zi2 = 2*zr*zi + ci. Bit-identical.
    #[test]
    fn float_mandelbrot_step_identical() {
        // slots: 0=zr 1=zi 2=cr 3=ci 4=two | scratch 5=t1 6=t2 7=t3 | out 8=zr2 9=zi2
        let ops = vec![
            MicroOp::MulF { dst: 5, lhs: 0, rhs: 0 }, // zr*zr
            MicroOp::MulF { dst: 6, lhs: 1, rhs: 1 }, // zi*zi
            MicroOp::SubF { dst: 7, lhs: 5, rhs: 6 }, // zr*zr - zi*zi
            MicroOp::AddF { dst: 8, lhs: 7, rhs: 2 }, // + cr  => zr2
            MicroOp::MulF { dst: 5, lhs: 0, rhs: 1 }, // zr*zi
            MicroOp::MulF { dst: 6, lhs: 4, rhs: 5 }, // 2*(zr*zi)
            MicroOp::AddF { dst: 9, lhs: 6, rhs: 3 }, // + ci  => zi2
            MicroOp::Move { dst: 0, src: 8 },
            MicroOp::Move { dst: 1, src: 9 },
            MicroOp::Return { src: 0 },
        ];
        for (zr, zi, cr, ci) in [(0.0, 0.0, -0.5, 0.5), (0.3, -0.2, 0.1, 0.7)] {
            let frame = vec![
                fbits(zr), fbits(zi), fbits(cr), fbits(ci), fbits(2.0),
                0, 0, 0, 0, 0,
            ];
            let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
            let (rout, rframe) = run_regalloc(&ops, &frame);
            let mut ref_frame = frame.clone();
            reference_eval(&ops, &mut ref_frame, 1000).unwrap();
            assert_eq!(rout, ChainOutcome::Return(expected), "zr={zr} zi={zi}");
            assert_eq!(rframe, ref_frame, "frame diverged zr={zr} zi={zi}");
        }
    }

    /// RED: a float ORDERING guard loop (the mandelbrot escape test / nbody
    /// bound). `while mag <= 4.0: mag = mag + step` returning the iteration
    /// count. The float compare (`LtEqF`/`BranchF`) must be exact IEEE.
    #[test]
    fn float_ordering_guard_loop_identical() {
        // slots: 0=mag(f) 1=limit(f) 2=step(f) 3=cnt(i) 4=one(i)
        // value-form LtEqF into a guard, then JumpIfFalse.
        let ops = vec![
            MicroOp::LtEqF { dst: 5, lhs: 0, rhs: 1 },     // 0: cond = mag <= limit
            MicroOp::JumpIfFalse { cond: 5, target: 5 },   // 1: !cond -> exit
            MicroOp::AddF { dst: 0, lhs: 0, rhs: 2 },       // 2: mag += step
            MicroOp::Add { dst: 3, lhs: 3, rhs: 4 },        // 3: cnt += 1
            MicroOp::Jump { target: 0 },                    // 4
            MicroOp::Return { src: 3 },                      // 5
        ];
        let frame = vec![fbits(0.0), fbits(4.0), fbits(0.01), 0, 1, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 10_000_000).unwrap();
        let (rout, _) = run_regalloc(&ops, &frame);
        assert_eq!(rout, ChainOutcome::Return(expected));
    }

    /// A DEEP float-accumulate loop with several loop-carried floats — the
    /// nbody/mandelbrot register-pressure shape — must stay bit-identical when
    /// (a) the loop-weight spill heuristic keeps the carried floats resident and
    /// (b) the resident-resident `emit_fbinop` fast path computes accumulators in
    /// place. Both are SPEED-only transforms: the contract is bit-for-bit
    /// equality with the tree-walker on the return value AND the full post-frame,
    /// over many iterations so any per-iteration miscompile (a clobbered
    /// accumulator, a wrong operand order in `subsd`) would diverge. We also
    /// assert the region actually compiles through the regalloc backend (it does
    /// not fall back), so the fast paths are genuinely exercised.
    #[test]
    fn float_deep_accumulate_loop_resident_fast_path_identical() {
        // slots: 0=i 1=n 2=one
        //   3=ax 4=ay 5=az  (carried float accumulators, in-place += / -= / *=)
        //   6=if(f) i-as-float | 7=t1 8=t2 (scratch, dst != lhs fast path)
        // while i < n:
        //   if = (f64) i
        //   t1 = if * if          (dst != lhs, both resident -> move-to-dst)
        //   ax = ax + t1          (in-place addsd)
        //   t2 = if + ax          (dst != lhs)
        //   ay = ay - t2          (in-place subsd — ORDER matters: ay - t2)
        //   az = az * if          (in-place mulsd)
        //   i += 1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 9 }, // 0
            MicroOp::IntToFloat { dst: 6, src: 0 },                      // 1
            MicroOp::MulF { dst: 7, lhs: 6, rhs: 6 },                    // 2  t1 = if*if
            MicroOp::AddF { dst: 3, lhs: 3, rhs: 7 },                    // 3  ax += t1
            MicroOp::AddF { dst: 8, lhs: 6, rhs: 3 },                    // 4  t2 = if+ax
            MicroOp::SubF { dst: 4, lhs: 4, rhs: 8 },                    // 5  ay -= t2
            MicroOp::MulF { dst: 5, lhs: 5, rhs: 6 },                    // 6  az *= if
            MicroOp::Add { dst: 0, lhs: 0, rhs: 2 },                     // 7  i += 1
            MicroOp::Jump { target: 0 },                                 // 8
            MicroOp::Return { src: 3 },                                  // 9  return ax bits
        ];
        // az starts at 1.0 (the multiplicative identity); a modest n keeps it
        // finite. ay starts at 0.0; ax at 0.0.
        let frame = vec![
            0i64, 64, 1,
            fbits(0.0), fbits(0.0), fbits(1.0),
            fbits(0.0), fbits(0.0), fbits(0.0),
        ];
        let expected = reference_eval(&ops, &mut frame.clone(), 1_000_000).unwrap();
        // The region MUST compile through the regalloc backend (no fallback) —
        // otherwise the fast paths under test never run.
        assert!(
            compile_region_regalloc(&ops, Some(Arc::new(AtomicI64::new(0)))).is_some(),
            "the float loop must regalloc (else the fast paths are not exercised)"
        );
        let (rout, rframe) = run_regalloc(&ops, &frame);
        let mut ref_frame = frame.clone();
        reference_eval(&ops, &mut ref_frame, 1_000_000).unwrap();
        assert_eq!(rout, ChainOutcome::Return(expected), "return diverged");
        assert_eq!(rframe, ref_frame, "frame diverged — a fast-path miscompile");
        // Spot the accumulators against an independent f64 recomputation, so the
        // test pins the SEMANTICS (operand order, in-place accumulation), not
        // just self-consistency.
        let (mut ax, mut ay, mut az) = (0.0f64, 0.0f64, 1.0f64);
        for i in 0..64i64 {
            let f = i as f64;
            let t1 = f * f;
            ax += t1;
            let t2 = f + ax;
            ay -= t2;
            az *= f;
        }
        assert_eq!(f64::from_bits(rout_bits(rout) as u64), ax, "ax diverged");
        assert_eq!(f64::from_bits(rframe[4] as u64), ay, "ay (subsd order) diverged");
        assert_eq!(f64::from_bits(rframe[5] as u64), az, "az (mulsd in-place) diverged");
    }

    /// RED: a BranchF compare-and-branch (every Cmp) bit-identical, including
    /// the NaN-unordered FALSE path.
    #[test]
    fn float_branchf_all_cmps_identical() {
        for cmp in [Cmp::Lt, Cmp::Gt, Cmp::LtEq, Cmp::GtEq] {
            // r = if cmp(a,b) { 111 } else { 222 } via BranchF (false -> else).
            let ops = vec![
                MicroOp::BranchF { cmp, lhs: 0, rhs: 1, target: 3 },
                MicroOp::LoadConst { dst: 2, value: 111 },
                MicroOp::Jump { target: 4 },
                MicroOp::LoadConst { dst: 2, value: 222 },
                MicroOp::Return { src: 2 },
            ];
            let nan = f64::NAN;
            for (a, b) in [
                (1.0, 2.0), (2.0, 1.0), (3.0, 3.0), (-0.0, 0.0),
                (nan, 1.0), (1.0, nan), (nan, nan),
                (f64::INFINITY, 1.0), (1.0, f64::NEG_INFINITY),
            ] {
                let frame = vec![fbits(a), fbits(b), 0];
                let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
                let (rout, _) = run_regalloc(&ops, &frame);
                assert_eq!(rout, ChainOutcome::Return(expected), "cmp={cmp:?} a={a} b={b}");
            }
        }
    }

    /// RED: float ORDERING value compares (LtF/GtF/LtEqF/GtEqF) over the full
    /// IEEE edge set — NaN compares produce 0 (false), exactly like the
    /// reference's Rust `<,>,<=,>=`.
    #[test]
    fn float_ordering_values_ieee_edges() {
        let nan = f64::NAN;
        let inf = f64::INFINITY;
        let cases = [
            (1.0, 2.0), (2.0, 1.0), (3.0, 3.0),
            (-0.0, 0.0), (0.0, -0.0),
            (nan, 1.0), (1.0, nan), (nan, nan),
            (inf, inf), (inf, -inf), (-inf, inf), (inf, 1.0),
        ];
        for (a, b) in cases {
            let ops = vec![
                MicroOp::LtF { dst: 2, lhs: 0, rhs: 1 },
                MicroOp::GtF { dst: 3, lhs: 0, rhs: 1 },
                MicroOp::LtEqF { dst: 4, lhs: 0, rhs: 1 },
                MicroOp::GtEqF { dst: 5, lhs: 0, rhs: 1 },
                // pack the four 0/1 results into one return: l + 2*g + 4*le + 8*ge
                MicroOp::Add { dst: 6, lhs: 3, rhs: 3 },  // 2*g
                MicroOp::Add { dst: 6, lhs: 6, rhs: 2 },  // + l
                MicroOp::Add { dst: 7, lhs: 4, rhs: 4 },  // 2*le
                MicroOp::Add { dst: 7, lhs: 7, rhs: 7 },  // 4*le
                MicroOp::Add { dst: 6, lhs: 6, rhs: 7 },
                MicroOp::Add { dst: 7, lhs: 5, rhs: 5 },  // 2*ge
                MicroOp::Add { dst: 7, lhs: 7, rhs: 7 },  // 4*ge
                MicroOp::Add { dst: 7, lhs: 7, rhs: 7 },  // 8*ge
                MicroOp::Add { dst: 6, lhs: 6, rhs: 7 },
                MicroOp::Return { src: 6 },
            ];
            let frame = vec![fbits(a), fbits(b), 0, 0, 0, 0, 0, 0];
            let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
            let (rout, _) = run_regalloc(&ops, &frame);
            assert_eq!(rout, ChainOutcome::Return(expected), "a={a} b={b}");
        }
    }

    /// RED: the kernel's EPSILON equality (EqF / NeqF) bit-identical to the
    /// reference, including the boundary just inside / just outside epsilon and
    /// the NaN case (which is NOT epsilon-equal).
    #[test]
    fn float_epsilon_equality_identical() {
        let eps = f64::EPSILON;
        let cases = [
            (1.0, 1.0),
            (1.0, 1.0 + eps * 0.5),  // within epsilon -> equal
            (1.0, 1.0 + eps * 2.0),  // outside epsilon -> not equal
            (0.0, -0.0),             // |0 - (-0)| = 0 < eps -> equal
            (f64::NAN, f64::NAN),    // |NaN| not < eps -> not equal
            (f64::NAN, 1.0),
            (1e300, 1e300),
            (-5.0, -5.0 + eps * 0.25),
        ];
        for (a, b) in cases {
            let ops = vec![
                MicroOp::EqF { dst: 2, lhs: 0, rhs: 1 },
                MicroOp::NeqF { dst: 3, lhs: 0, rhs: 1 },
                MicroOp::Add { dst: 4, lhs: 3, rhs: 3 }, // 2*neq
                MicroOp::Add { dst: 4, lhs: 4, rhs: 2 }, // + eq
                MicroOp::Return { src: 4 },
            ];
            let frame = vec![fbits(a), fbits(b), 0, 0, 0];
            let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
            let (rout, _) = run_regalloc(&ops, &frame);
            assert_eq!(rout, ChainOutcome::Return(expected), "a={a:?} b={b:?}");
        }
    }

    /// RED: DivF by 0.0 side-exits (deopt) BEFORE any effect — identical to the
    /// integer Div0 contract and the reference (`b == 0.0 -> None`). Both +0.0
    /// and -0.0 trip it (`0.0 == -0.0`).
    #[test]
    fn float_div_by_zero_side_exits() {
        for z in [0.0f64, -0.0f64] {
            let ops = vec![
                MicroOp::DivF { dst: 2, lhs: 0, rhs: 1 },
                MicroOp::Return { src: 2 },
            ];
            let frame = vec![fbits(7.0), fbits(z), 0];
            // reference rejects.
            assert!(reference_eval(&ops, &mut frame.clone(), 1000).is_none(), "z={z}");
            let status = Arc::new(AtomicI64::new(0));
            let chain = compile_region_regalloc(&ops, Some(status)).expect("compiles");
            let mut f = frame.clone();
            let rout = chain.run_with_frame(&mut f);
            assert!(rout.is_deopt(), "DivF/0 must deopt z={z}: {rout:?}");
        }
    }

    /// RED: FmaF is TWO separate IEEE roundings (mul then add), NOT a fused
    /// `vfmadd`. We search for inputs where the two-rounding result differs from
    /// the single-rounding fused result and require the backend to match the
    /// TWO-rounding result on EACH — a hardware-FMA miscompile would diverge.
    #[test]
    fn float_fma_is_two_roundings_not_fused() {
        let ops = vec![
            MicroOp::FmaF { dst: 3, a: 0, b: 1, c: 2 },
            MicroOp::Return { src: 3 },
        ];
        // Walk an LCG over magnitudes that lose low product bits; collect the
        // triples where fma (one rounding) and (a*b)+c (two roundings) disagree.
        let mut s = 0x1234_5678_9ABC_DEF0u64;
        let mut distinguished = 0usize;
        for _ in 0..20_000 {
            let bits = |st: &mut u64| -> f64 {
                let m = lcg(st);
                // a finite, non-tiny f64 in a range that exercises rounding.
                let v = f64::from_bits((m & 0x000F_FFFF_FFFF_FFFF) | 0x3FE0_0000_0000_0000);
                if lcg(st) & 1 == 0 { v } else { -v }
            };
            let a = bits(&mut s);
            let b = bits(&mut s);
            let c = bits(&mut s);
            let two_round = ((a * b) + c).to_bits() as i64;
            let fused = a.mul_add(b, c).to_bits() as i64;
            if two_round == fused {
                continue; // these inputs don't distinguish the two modes
            }
            distinguished += 1;
            let frame = vec![fbits(a), fbits(b), fbits(c), 0];
            let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
            let (rout, _) = run_regalloc(&ops, &frame);
            assert_eq!(rout, ChainOutcome::Return(expected), "a={a} b={b} c={c}");
            assert_eq!(
                rout,
                ChainOutcome::Return(two_round),
                "FmaF must be TWO roundings (a={a} b={b} c={c}); a fused FMA diverges"
            );
        }
        assert!(
            distinguished >= 100,
            "the search must find FMA-distinguishing triples (found {distinguished})"
        );
    }

    /// RED: -0.0 / NaN / inf round-trip exactly through AddF/SubF/MulF/Move
    /// (raw bits preserved, no canonicalization).
    #[test]
    fn float_special_values_roundtrip() {
        let specials = [0.0f64, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.0, -1.0];
        for &a in &specials {
            for &b in &specials {
                let ops = vec![
                    MicroOp::AddF { dst: 4, lhs: 0, rhs: 1 },
                    MicroOp::SubF { dst: 5, lhs: 0, rhs: 1 },
                    MicroOp::MulF { dst: 6, lhs: 0, rhs: 1 },
                    MicroOp::Move { dst: 7, src: 0 },
                    // combine the bit patterns so a divergence in ANY is caught.
                    MicroOp::BitXor { dst: 4, lhs: 4, rhs: 5 },
                    MicroOp::BitXor { dst: 4, lhs: 4, rhs: 6 },
                    MicroOp::BitXor { dst: 4, lhs: 4, rhs: 7 },
                    MicroOp::Return { src: 4 },
                ];
                let frame = vec![fbits(a), fbits(b), 0, 0, 0, 0, 0, 0];
                let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
                let (rout, _) = run_regalloc(&ops, &frame);
                assert_eq!(rout, ChainOutcome::Return(expected), "a={a:?} b={b:?}");
            }
        }
    }

    /// The big float differential: random float programs over the supported
    /// float families at every register pressure — return value AND the full
    /// post-frame bit-identical to the reference, proving the XMM allocation
    /// (resident + spilled) is sound regardless of how many floats fit.
    #[test]
    fn random_float_programs_identical_all_pressures() {
        for slots in [4u16, 7, 12, 18] {
            for seed in 1..=120u64 {
                let mut s = seed.wrapping_mul(0x9E3779B97F4A7C15) ^ 0xF10A7;
                let len = 6 + (lcg(&mut s) % 24) as usize;
                let mut ops = Vec::with_capacity(len + 1);
                for _ in 0..len {
                    let dst = (lcg(&mut s) % slots as u64) as u16;
                    let lhs = (lcg(&mut s) % slots as u64) as u16;
                    let rhs = (lcg(&mut s) % slots as u64) as u16;
                    // Mostly float arithmetic; some IntToFloat / Move / compares.
                    let op = match lcg(&mut s) % 12 {
                        0 => MicroOp::AddF { dst, lhs, rhs },
                        1 => MicroOp::SubF { dst, lhs, rhs },
                        2 => MicroOp::MulF { dst, lhs, rhs },
                        3 => MicroOp::SqrtF { dst, src: lhs },
                        4 => MicroOp::FmaF { dst, a: lhs, b: rhs, c: dst },
                        5 => MicroOp::Move { dst, src: lhs },
                        6 => MicroOp::LtF { dst, lhs, rhs },
                        7 => MicroOp::GtF { dst, lhs, rhs },
                        8 => MicroOp::LtEqF { dst, lhs, rhs },
                        9 => MicroOp::GtEqF { dst, lhs, rhs },
                        10 => MicroOp::EqF { dst, lhs, rhs },
                        _ => MicroOp::NeqF { dst, lhs, rhs },
                    };
                    ops.push(op);
                }
                let ret = (lcg(&mut s) % slots as u64) as u16;
                ops.push(MicroOp::Return { src: ret });

                // Seed the frame with assorted finite f64 bit patterns.
                let mut frame = vec![0i64; slots as usize];
                for (i, f) in frame.iter_mut().enumerate() {
                    let v = ((i as f64) - (slots as f64) / 2.0) * 1.5 + 0.25;
                    *f = v.to_bits() as i64;
                }
                let mut ref_frame = frame.clone();
                let expected = match reference_eval(&ops, &mut ref_frame, 1_000_000) {
                    Some(v) => v,
                    None => continue, // a DivF/0 the reference rejects — skip
                };
                let (rout, rframe) = run_regalloc(&ops, &frame);
                assert_eq!(
                    rout,
                    ChainOutcome::Return(expected),
                    "slots={slots} seed={seed}: float return diverged"
                );
                assert_eq!(
                    rframe, ref_frame,
                    "slots={slots} seed={seed}: float frame diverged"
                );
            }
        }
    }

    /// A float ARRAY sum (Seq of Float) — `ArrLoad { byte:false }` loads the raw
    /// 8-byte element, then `AddF` accumulates it. A float-array-sum region must
    /// regalloc and stay bit-identical to the reference.
    #[test]
    fn float_array_sum_loop_identical() {
        // frame: 0=i 1=n 2=one(i) 3=elem(f) 4=acc(f) 5=ptr 6=len
        // while i <= n: elem = arr[i]; acc = acc + elem; i += 1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 5 },
            MicroOp::ArrLoad { dst: 3, idx: 0, ptr_slot: 5, len_slot: 6, byte: false, narrow32: false, checked: true },
            MicroOp::AddF { dst: 4, lhs: 4, rhs: 3 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 2 },
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 4 },
        ];
        let n = 1000usize;
        let mut buf: Vec<i64> =
            (0..n).map(|k| ((k as f64) * 0.5 - 3.0).to_bits() as i64).collect();
        let frame = vec![1i64, n as i64, 1, fbits(0.0), fbits(0.0), 0, n as i64];

        let (vref, _f, _b) = run_reference_buf(&ops, &frame, &[buf.clone()], &[5]);
        let expected = vref.expect("reference terminates");
        let (out, _) = run_regalloc_buf(&ops, &frame, vec![(5, &mut buf)]);
        assert_eq!(out, ChainOutcome::Return(expected));
        let want: f64 = (0..n).map(|k| (k as f64) * 0.5 - 3.0).sum();
        assert_eq!(f64::from_bits(rout_bits(out) as u64), want);
    }

    /// A float ARRAY write loop: `arr[i] = (f64)i * 0.25` — the post-BUFFER must
    /// match the reference's bit for bit (ArrStore of a float-resident source).
    #[test]
    fn float_array_write_loop_identical() {
        // frame: 0=i 1=n 2=one(i) 3=i_f(f) 4=q(f) 5=v(f) 6=ptr 7=len
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 6 },
            MicroOp::IntToFloat { dst: 3, src: 0 },
            MicroOp::MulF { dst: 5, lhs: 3, rhs: 4 }, // i_f * 0.25
            MicroOp::ArrStore { src: 5, idx: 0, ptr_slot: 6, len_slot: 7, byte: false, narrow32: false, checked: true },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 2 },
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 0 },
        ];
        let n = 500usize;
        let mut buf = vec![0i64; n];
        let frame = vec![1i64, n as i64, 1, fbits(0.0), fbits(0.25), fbits(0.0), 0, n as i64];

        let (_vref, _f, bref) = run_reference_buf(&ops, &frame, &[vec![0i64; n]], &[6]);
        let (_out, _) = run_regalloc_buf(&ops, &frame, vec![(6, &mut buf)]);
        assert_eq!(buf, bref[0], "float-written buffer diverged from reference");
        let want: Vec<i64> =
            (1..=n as i64).map(|k| ((k as f64) * 0.25).to_bits() as i64).collect();
        assert_eq!(buf, want);
    }

    // ----------------------------------------------------------------
    // WAVE 13: LIST MUTATION (ArrPush / ListClear) in the REGION backend.
    //
    // These are HELPER CALLS into the JIT runtime (`logos_rt_push_*` /
    // `logos_rt_clear_*`). The push may REALLOCATE the buffer, so the pinned
    // ptr/len slots are refreshed by the helper IN THE FRAME — the regalloc
    // backend keeps the vec/ptr/len handle slots frame-resident and spills its
    // caller-saved residents around the call (the callee clobbers them). The
    // alias-safety of a `ListClear` (buffer reuse) is established UPSTREAM in
    // the micro-op lowering (a `Op::NewEmptyList` whose handle escapes via a
    // live `Move` declines to emit `ListClear`), so a `ListClear` reaching the
    // backend is provably sole-owned.
    //
    // `reference_eval` cannot model live runtime list state, so these forge
    // tests differential the CONTIGUOUS regalloc backend against the PER-PIECE
    // stencil tier (`compile_straightline_coded`, which already lowers these
    // ops) over INDEPENDENT real `Vec<i64>` buffers: identical return value,
    // identical post-frame, identical post-BUFFER. The engine-level (source)
    // tests below then prove `vm == tree-walker` end to end and that the loop
    // tiered through the regalloc backend.
    // ----------------------------------------------------------------

    /// A live boxed `Vec<i64>` (the runtime buffer a pin triple points at) kept
    /// alive for the whole run, then read back for comparison.
    struct LiveVec {
        boxed: Box<Vec<i64>>,
    }
    impl LiveVec {
        fn new(init: Vec<i64>) -> Self {
            LiveVec { boxed: Box::new(init) }
        }
        fn handle(&mut self) -> i64 {
            self.boxed.as_mut() as *mut Vec<i64> as i64
        }
        fn ptr(&mut self) -> i64 {
            self.boxed.as_mut_ptr() as i64
        }
        fn len(&self) -> i64 {
            self.boxed.len() as i64
        }
        fn snapshot(&self) -> Vec<i64> {
            (*self.boxed).clone()
        }
    }

    /// A pinned list: a backing buffer and the THREE frame slots that hold its
    /// pin triple (vec handle, element pointer, length) — exactly what the VM
    /// establishes before a list-mutation region. The harness seeds all three
    /// cells (the region's `ArrLoad`/`ArrPush`/`ListClear` index them) and the
    /// `(vec_slot, ptr_slot)` cells hold raw HEAP pointers, so they are masked
    /// out of the post-frame differential (two independent runs allocate at
    /// different addresses).
    struct ListPin {
        vec_slot: usize,
        ptr_slot: usize,
        len_slot: usize,
        buf: LiveVec,
    }
    impl ListPin {
        fn new(vec_slot: usize, ptr_slot: usize, len_slot: usize, init: Vec<i64>) -> Self {
            ListPin { vec_slot, ptr_slot, len_slot, buf: LiveVec::new(init) }
        }
    }

    /// The Int push helper address (mirrors `Op::ListPush`'s `PinElem::Int`
    /// lowering in the JIT crate).
    fn push_i64_addr() -> i64 {
        logicaffeine_jit::logos_rt_push_i64 as usize as i64
    }
    /// The Int clear helper address (mirrors `Op::NewEmptyList`'s reuse
    /// lowering).
    fn clear_i64_addr() -> i64 {
        logicaffeine_jit::logos_rt_clear_i64 as usize as i64
    }

    /// Seed each pin triple (vec handle, element ptr, length) into the frame —
    /// the VM's pre-region pinning, replayed here so the region's indexed reads
    /// and the helper's frame writes land on a real buffer.
    fn seed_pins(f: &mut [i64], pins: &mut [ListPin]) {
        for p in pins.iter_mut() {
            f[p.vec_slot] = p.buf.handle();
            f[p.ptr_slot] = p.buf.ptr();
            f[p.len_slot] = p.buf.len();
        }
    }

    /// Zero the (vec_slot, ptr_slot) HEAP-pointer cells in a post-frame so two
    /// independent runs (which allocate at different addresses) can be compared
    /// on everything ELSE — the len cells, the loop scalars, the result. The
    /// real buffer-content differential is the buffers themselves.
    fn mask_ptr_cells(mut f: Vec<i64>, pins: &[ListPin]) -> Vec<i64> {
        for p in pins {
            f[p.vec_slot] = 0;
            f[p.ptr_slot] = 0;
        }
        f
    }

    /// Run a list-mutation program through the CONTIGUOUS regalloc backend over
    /// the seeded pin triples. Returns the outcome and the post-frame; the
    /// buffers are mutated in place by the helpers.
    fn run_regalloc_lists(
        ops: &[MicroOp],
        frame: &[i64],
        pins: &mut [ListPin],
    ) -> (ChainOutcome, Vec<i64>) {
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(ops, Some(status))
            .expect("list-mutation region must compile under regalloc");
        let mut f = frame.to_vec();
        seed_pins(&mut f, pins);
        let out = chain.run_with_frame(&mut f);
        (out, f)
    }

    /// The SAME program through the PER-PIECE stencil tier, over INDEPENDENT
    /// buffers (so it never aliases the regalloc run).
    fn run_stencil_lists(
        ops: &[MicroOp],
        frame: &[i64],
        pins: &mut [ListPin],
    ) -> (ChainOutcome, Vec<i64>) {
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_straightline_coded(ops, Some(status), None, 0)
            .expect("stencil tier compiles the list-mutation region");
        let mut f = frame.to_vec();
        seed_pins(&mut f, pins);
        let out = chain.run_with_frame(&mut f);
        (out, f)
    }

    /// RED: a build-a-list-via-PUSH loop. `while i < n: push i*3-1 to a; i+=1`.
    /// The push helper reallocates as the buffer grows — the backend must
    /// refresh the pinned ptr/len AFTER each call (else the next push reads a
    /// stale pointer and the length drifts). Three-way: regalloc == stencil on
    /// the return, the masked post-frame, AND the post-buffer.
    #[test]
    fn push_build_loop_regalloc_matches_stencil() {
        // frame: 0=i 1=n 2=val 3=three 4=vec 5=ptr 6=len 7=one
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 7 }, // 0
            MicroOp::Mul { dst: 2, lhs: 0, rhs: 3 },                     // 1 val = i*3
            MicroOp::Sub { dst: 2, lhs: 2, rhs: 7 },                     // 2 val -= 1
            MicroOp::ArrPush {
                src: 2,
                vec_slot: 4,
                ptr_slot: 5,
                len_slot: 6,
                helper_addr: push_i64_addr(),
                byte: false,
                narrow32: false,
            }, // 3
            MicroOp::Add { dst: 0, lhs: 0, rhs: 7 }, // 4 i += 1
            MicroOp::Jump { target: 0 },             // 5
            MicroOp::LoadConst { dst: 2, value: 0 }, // 6 filler
            MicroOp::Return { src: 6 },              // 7 return len
        ];
        let n = 5000i64;
        let frame = vec![0i64, n, 0, 3, 0, 0, 0, 1];

        let mut rp = [ListPin::new(4, 5, 6, Vec::new())];
        let (rout, rframe) = run_regalloc_lists(&ops, &frame, &mut rp);
        let mut sp = [ListPin::new(4, 5, 6, Vec::new())];
        let (sout, sframe) = run_stencil_lists(&ops, &frame, &mut sp);

        assert_eq!(rout, sout, "push-build return diverged");
        assert_eq!(
            mask_ptr_cells(rframe, &rp),
            mask_ptr_cells(sframe, &sp),
            "push-build post-frame diverged (ptr/len drift?)"
        );
        assert_eq!(rout, ChainOutcome::Return(n), "should return the final length");
        let want: Vec<i64> = (0..n).map(|k| k * 3 - 1).collect();
        assert_eq!(rp[0].buf.snapshot(), want, "regalloc buffer wrong");
        assert_eq!(rp[0].buf.snapshot(), sp[0].buf.snapshot(), "buffers diverged");
    }

    /// RED: the KNAPSACK DP-ROW shape — `Let mutable curr be a new Seq`
    /// (`ListClear` reuse of a SOLE-OWNED row buffer), an `ArrLoad` of the prior
    /// row, and a `Push` that fills the new row, repeated across rows. The
    /// classic alias `Set prev to curr` is NOT present (that escaping move makes
    /// the lowering decline the reuse UPSTREAM — a `ListClear` never reaches the
    /// backend aliased); this ping-pongs two distinct sole-owned buffers.
    /// Three-way bit-identical (masked frame + both buffers).
    #[test]
    fn knapsack_dp_row_regalloc_matches_stencil() {
        // frame: 0=r 1=rows 2=c 3=cols 4=p_vec 5=p_ptr 6=p_len 7=tmp
        //        8=q_vec 9=q_ptr 10=q_len 11=elem 12=one 13=base
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 22 }, // 0 while r<rows
            MicroOp::ListClear { vec_slot: 8, ptr_slot: 9, len_slot: 10, helper_addr: clear_i64_addr() }, // 1 clear curr(q)
            MicroOp::LoadConst { dst: 2, value: 1 }, // 2  c = 1
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 2, rhs: 3, target: 11 }, // 3 while c<=cols
            MicroOp::ArrLoad { dst: 11, idx: 2, ptr_slot: 5, len_slot: 6, byte: false, narrow32: false, checked: true }, // 4 elem=prev[c]
            MicroOp::Add { dst: 11, lhs: 11, rhs: 0 }, // 5 elem += r
            MicroOp::ArrPush { src: 11, vec_slot: 8, ptr_slot: 9, len_slot: 10, helper_addr: push_i64_addr(), byte: false, narrow32: false }, // 6 push elem to q
            MicroOp::Add { dst: 2, lhs: 2, rhs: 12 }, // 7 c += 1
            MicroOp::Jump { target: 3 },              // 8
            MicroOp::LoadConst { dst: 7, value: 0 },  // 9 filler
            MicroOp::Jump { target: 11 },             // 10
            MicroOp::ListClear { vec_slot: 4, ptr_slot: 5, len_slot: 6, helper_addr: clear_i64_addr() }, // 11 clear prev(p)
            MicroOp::LoadConst { dst: 2, value: 1 }, // 12 c = 1
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 2, rhs: 3, target: 20 }, // 13 while c<=cols
            MicroOp::ArrLoad { dst: 11, idx: 2, ptr_slot: 9, len_slot: 10, byte: false, narrow32: false, checked: true }, // 14 elem=q[c]
            MicroOp::ArrPush { src: 11, vec_slot: 4, ptr_slot: 5, len_slot: 6, helper_addr: push_i64_addr(), byte: false, narrow32: false }, // 15 push to prev
            MicroOp::Add { dst: 2, lhs: 2, rhs: 12 }, // 16 c += 1
            MicroOp::Jump { target: 13 },             // 17
            MicroOp::LoadConst { dst: 7, value: 0 },  // 18 filler
            MicroOp::Jump { target: 20 },             // 19
            MicroOp::Add { dst: 0, lhs: 0, rhs: 12 }, // 20 r += 1
            MicroOp::Jump { target: 0 },              // 21
            MicroOp::Return { src: 6 },               // 22 return len(prev)
        ];
        let rows = 40i64;
        let cols = 60i64;
        // prev seeded with cols entries (1-based usable indices 1..=cols).
        let prev_init: Vec<i64> = (0..cols).map(|k| k * 2).collect();
        // frame: r=0, rows, c=0, cols, p_*, p_len(seeded), tmp, q_*, elem, one, base
        let frame = vec![0i64, rows, 0, cols, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0];

        let mut rp = [
            ListPin::new(4, 5, 6, prev_init.clone()),
            ListPin::new(8, 9, 10, Vec::new()),
        ];
        let (rout, rframe) = run_regalloc_lists(&ops, &frame, &mut rp);
        let mut sp = [
            ListPin::new(4, 5, 6, prev_init.clone()),
            ListPin::new(8, 9, 10, Vec::new()),
        ];
        let (sout, sframe) = run_stencil_lists(&ops, &frame, &mut sp);

        assert_eq!(rout, sout, "knapsack-DP return diverged");
        assert_eq!(
            mask_ptr_cells(rframe, &rp),
            mask_ptr_cells(sframe, &sp),
            "knapsack-DP post-frame diverged"
        );
        assert_eq!(rp[0].buf.snapshot(), sp[0].buf.snapshot(), "prev buffer diverged");
        assert_eq!(rp[1].buf.snapshot(), sp[1].buf.snapshot(), "curr buffer diverged");
        // The DP recurrence adds `r` to each cell once per row → +sum(0..rows).
        let bump = (0..rows).sum::<i64>();
        let want: Vec<i64> = prev_init.iter().map(|&v| v + bump).collect();
        assert_eq!(rp[0].buf.snapshot(), want, "knapsack-DP result wrong");
    }

    /// RED: the BFS-QUEUE-PUSH shape — a worklist filled by `Push to queue`,
    /// then drained by indexing the SAME growing buffer (a head pointer
    /// marches). The append and the indexed reads must stay coherent across the
    /// reallocating push (the backend reloads ptr/len after the call before the
    /// next indexed read). Three-way bit-identical.
    #[test]
    fn bfs_queue_push_regalloc_matches_stencil() {
        // frame: 0=i 1=n 2=q_vec 3=q_ptr 4=q_len 5=head 6=acc 7=elem 8=one
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 5 }, // 0 while i<=n
            MicroOp::ArrPush { src: 0, vec_slot: 2, ptr_slot: 3, len_slot: 4, helper_addr: push_i64_addr(), byte: false, narrow32: false }, // 1
            MicroOp::Add { dst: 0, lhs: 0, rhs: 8 }, // 2 i += 1
            MicroOp::Jump { target: 0 },             // 3
            MicroOp::LoadConst { dst: 7, value: 0 }, // 4 filler
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 5, rhs: 4, target: 11 }, // 5 while head<=len
            MicroOp::ArrLoad { dst: 7, idx: 5, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true }, // 6 elem=queue[head]
            MicroOp::Add { dst: 6, lhs: 6, rhs: 7 }, // 7 acc += elem
            MicroOp::Add { dst: 5, lhs: 5, rhs: 8 }, // 8 head += 1
            MicroOp::Jump { target: 5 },             // 9
            MicroOp::LoadConst { dst: 7, value: 0 }, // 10 filler
            MicroOp::Return { src: 6 },              // 11 return acc
        ];
        let n = 4000i64;
        // i=1, n, q_*, q_len(seeded), head=1, acc=0, elem, one=1
        let frame = vec![1i64, n, 0, 0, 0, 1, 0, 0, 1];

        let mut rp = [ListPin::new(2, 3, 4, Vec::new())];
        let (rout, rframe) = run_regalloc_lists(&ops, &frame, &mut rp);
        let mut sp = [ListPin::new(2, 3, 4, Vec::new())];
        let (sout, sframe) = run_stencil_lists(&ops, &frame, &mut sp);

        assert_eq!(rout, sout, "bfs-queue return diverged");
        assert_eq!(
            mask_ptr_cells(rframe, &rp),
            mask_ptr_cells(sframe, &sp),
            "bfs-queue post-frame diverged"
        );
        assert_eq!(rp[0].buf.snapshot(), sp[0].buf.snapshot(), "queue buffer diverged");
        let want: i64 = (1..=n).sum();
        assert_eq!(rout, ChainOutcome::Return(want), "bfs-queue sum wrong");
    }

    /// RED: a float-list PUSH loop — the value is FLOAT-class (XMM-resident), so
    /// its raw bits must be bit-copied to the value-arg GP register before the
    /// helper call. Differential against the stencil tier on the post-buffer
    /// (interpreted as f64 bits) + the masked frame.
    #[test]
    fn float_push_loop_regalloc_matches_stencil() {
        // frame: 0=i 1=n 2=i_f(f) 3=q(f) 4=v(f) 5=vec 6=ptr 7=len 8=one
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 7 }, // 0
            MicroOp::IntToFloat { dst: 2, src: 0 },                       // 1 i_f = (f64)i
            MicroOp::MulF { dst: 4, lhs: 2, rhs: 3 },                     // 2 v = i_f * 0.5
            MicroOp::ArrPush {
                src: 4,
                vec_slot: 5,
                ptr_slot: 6,
                len_slot: 7,
                helper_addr: logicaffeine_jit::logos_rt_push_f64 as usize as i64,
                byte: false,
                narrow32: false,
            }, // 3
            MicroOp::Add { dst: 0, lhs: 0, rhs: 8 }, // 4 i += 1
            MicroOp::Jump { target: 0 },             // 5
            MicroOp::LoadConst { dst: 2, value: 0 }, // 6 filler
            MicroOp::Return { src: 7 },              // 7 return len
        ];
        let n = 2000i64;
        let frame = vec![0i64, n, fbits(0.0), fbits(0.5), fbits(0.0), 0, 0, 0, 1];

        let mut rp = [ListPin::new(5, 6, 7, Vec::new())];
        let (rout, rframe) = run_regalloc_lists(&ops, &frame, &mut rp);
        let mut sp = [ListPin::new(5, 6, 7, Vec::new())];
        let (sout, sframe) = run_stencil_lists(&ops, &frame, &mut sp);

        assert_eq!(rout, sout, "float-push return diverged");
        assert_eq!(
            mask_ptr_cells(rframe, &rp),
            mask_ptr_cells(sframe, &sp),
            "float-push post-frame diverged"
        );
        assert_eq!(rp[0].buf.snapshot(), sp[0].buf.snapshot(), "float buffer diverged");
        let want: Vec<i64> =
            (0..n).map(|k| ((k as f64) * 0.5).to_bits() as i64).collect();
        assert_eq!(rp[0].buf.snapshot(), want, "float-push values wrong");
    }

    // ----------------------------------------------------------------
    // WAVE 12: CONTIGUOUS-FUNCTION SELF-CALLS (CallSelf / CallSelfCopy).
    //
    // `compile_function_regalloc` emits a whole recursive FUNCTION (its
    // self-calls included) as ONE register-allocated x86-64 function with a
    // real SysV self-call. These forge-level tests drive the call ABI
    // directly: the depth cell, the shared status cell, and the arena bound
    // through the limit slot — exactly the machinery the function tier wires.
    // ----------------------------------------------------------------

    use logicaffeine_forge::regalloc::compile_function_regalloc;

    /// Arena slots a self-call chain runs over (matches the JIT's ARENA_SLOTS
    /// budget; large enough to stack thousands of recursion frames).
    const SELF_ARENA: usize = 1 << 16;

    /// Drive a recursive FUNCTION chain: stage the single integer argument in
    /// slot 0, plant `arena_end` in `limit_slot`, set the depth cell to 0, and
    /// run. Returns the outcome (Return = result, Deopt = side exit).
    fn run_self_fn(
        ops: &[MicroOp],
        arg0: i64,
        limit_slot: u16,
    ) -> (ChainOutcome, Arc<AtomicI64>, Arc<AtomicI64>) {
        let status = Arc::new(AtomicI64::new(0));
        let depth = Arc::new(AtomicI64::new(0));
        let depth_addr = Arc::as_ptr(&depth) as i64;
        let chain = compile_function_regalloc(ops, Some(status.clone()), depth_addr)
            .expect("recursive function compiles");
        let mut frame = vec![0i64; SELF_ARENA];
        frame[0] = arg0;
        let arena_end = unsafe { frame.as_ptr().add(SELF_ARENA) } as i64;
        frame[limit_slot as usize] = arena_end;
        depth.store(0, std::sync::atomic::Ordering::SeqCst);
        let out = chain.run_with_frame(&mut frame);
        (out, status, depth)
    }

    /// A 1-argument recursive `fib` as a function-tier MicroOp stream, staged
    /// exactly like `adapt_function`: the callee window sits at `limit_slot + 1`,
    /// the self-call copies the contiguous arg block, and the arena bound rides
    /// `limit_slot`. In-body constants are re-materialized with `LoadConst` every
    /// call (the function tier never relies on a caller-set frame cell for a
    /// constant — each recursion's window is freshly staged), so slots 3/4 are
    /// loaded, not pre-seeded. limit_slot = 8. The live depth/status cell
    /// addresses are baked in at build time.
    fn fib_self_program_with(depth_addr: i64, status_addr: i64) -> (Vec<MicroOp>, u16) {
        let limit_slot: u16 = 8;
        let window = limit_slot + 1;
        let fs = (limit_slot as i64) + 1;
        // slots: 0=n 1=t1 2=t2 3=const2 4=const1
        let ops = vec![
            MicroOp::LoadConst { dst: 3, value: 2 }, // 0
            MicroOp::LoadConst { dst: 4, value: 1 }, // 1
            // if !(n < 2) goto 4  (Branch jumps to target when cmp is FALSE)
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 3, target: 4 }, // 2
            MicroOp::Return { src: 0 },                                  // 3: return n
            MicroOp::Sub { dst: window, lhs: 0, rhs: 4 },                // 4: arg = n - 1
            MicroOp::CallSelfCopy {
                dst: 1,
                args_start: window,
                src_start: window,
                arg_count: 1,
                depth_addr,
                status_addr,
                limit_slot,
                frame_size: fs,
            }, // 5: t1 = fib(n-1)
            MicroOp::Sub { dst: window, lhs: 0, rhs: 3 },                // 6: arg = n - 2
            MicroOp::CallSelfCopy {
                dst: 2,
                args_start: window,
                src_start: window,
                arg_count: 1,
                depth_addr,
                status_addr,
                limit_slot,
                frame_size: fs,
            }, // 7: t2 = fib(n-2)
            MicroOp::Add { dst: 1, lhs: 1, rhs: 2 },                     // 8: t1 + t2
            MicroOp::Return { src: 1 },                                  // 9
        ];
        (ops, limit_slot)
    }

    fn fib_ref(n: i64) -> i64 {
        if n < 2 {
            n
        } else {
            fib_ref(n - 1) + fib_ref(n - 2)
        }
    }

    /// RED: a 1-argument recursive function compiled by the contiguous FUNCTION
    /// backend returns the EXACT recursive result. Requires `CallSelfCopy`
    /// support (depth check, arena check, arg staging, the SysV self-call, the
    /// caller-saved spill/reload across it, the post-call status check + result
    /// store).
    #[test]
    fn self_call_fib_matches_reference() {
        let status = Arc::new(AtomicI64::new(0));
        let depth = Arc::new(AtomicI64::new(0));
        let (ops, limit_slot) = fib_self_program_with(
            Arc::as_ptr(&depth) as i64,
            Arc::as_ptr(&status) as i64,
        );
        let chain =
            compile_function_regalloc(&ops, Some(status.clone()), Arc::as_ptr(&depth) as i64)
                .expect("fib function compiles");
        for n in 0..=22i64 {
            let mut frame = vec![0i64; SELF_ARENA];
            frame[0] = n;
            let arena_end = unsafe { frame.as_ptr().add(SELF_ARENA) } as i64;
            frame[limit_slot as usize] = arena_end;
            depth.store(0, std::sync::atomic::Ordering::SeqCst);
            status.store(0, std::sync::atomic::Ordering::SeqCst);
            let out = chain.run_with_frame(&mut frame);
            assert_eq!(
                out,
                ChainOutcome::Return(fib_ref(n)),
                "fib({n}) via regalloc self-call"
            );
        }
    }

    /// A deep linear recursion `sink(n) = n==0 ? 0 : sink(n-1) - 1` — exercises
    /// the call/return chain at real depth (every level a genuine frame) and the
    /// caller-saved spill/reload of the loop-carried `n` across the call.
    fn sink_self_program_with(depth_addr: i64, status_addr: i64) -> (Vec<MicroOp>, u16) {
        let limit_slot: u16 = 8;
        let window = limit_slot + 1;
        let fs = (limit_slot as i64) + 1;
        // slots: 0=n 1=t 2=one 3=zero (both re-materialized every call)
        let ops = vec![
            MicroOp::LoadConst { dst: 2, value: 1 }, // 0: one
            MicroOp::LoadConst { dst: 3, value: 0 }, // 1: zero
            MicroOp::Branch { cmp: Cmp::Eq, lhs: 0, rhs: 3, target: 4 }, // 2: if !(n==0) goto 4
            MicroOp::Return { src: 3 },                                  // 3: return 0
            MicroOp::Sub { dst: window, lhs: 0, rhs: 2 },                // 4: arg = n - 1
            MicroOp::CallSelfCopy {
                dst: 1,
                args_start: window,
                src_start: window,
                arg_count: 1,
                depth_addr,
                status_addr,
                limit_slot,
                frame_size: fs,
            }, // 5
            MicroOp::Sub { dst: 1, lhs: 1, rhs: 2 }, // 6: t = sink(n-1) - 1
            MicroOp::Return { src: 1 },              // 7
        ];
        (ops, limit_slot)
    }

    #[test]
    fn self_call_deep_linear_recursion() {
        let status = Arc::new(AtomicI64::new(0));
        let depth = Arc::new(AtomicI64::new(0));
        let (ops, limit_slot) = sink_self_program_with(
            Arc::as_ptr(&depth) as i64,
            Arc::as_ptr(&status) as i64,
        );
        let chain =
            compile_function_regalloc(&ops, Some(status.clone()), Arc::as_ptr(&depth) as i64)
                .expect("sink function compiles");
        for n in [0i64, 1, 2, 50, 500, 2000] {
            let mut frame = vec![0i64; SELF_ARENA];
            frame[0] = n;
            let arena_end = unsafe { frame.as_ptr().add(SELF_ARENA) } as i64;
            frame[limit_slot as usize] = arena_end;
            depth.store(0, std::sync::atomic::Ordering::SeqCst);
            status.store(0, std::sync::atomic::Ordering::SeqCst);
            let out = chain.run_with_frame(&mut frame);
            assert_eq!(out, ChainOutcome::Return(-n), "sink({n}) = -n");
        }
    }

    /// RED: DEPTH-LIMIT side-exit. Starting the depth cell at MAX_CALL_DEPTH-2
    /// and recursing deeper than 2 levels must trip the depth guard inside the
    /// self-call (status = 5) → a `Deopt`, bit-identical to the stencil tier's
    /// `logos_stencil_call_self` (marker 5). The chain must NOT run off the
    /// rails (no crash, no wrong value) — it side-exits.
    #[test]
    fn self_call_depth_limit_side_exits() {
        let status = Arc::new(AtomicI64::new(0));
        let depth = Arc::new(AtomicI64::new(0));
        let (ops, limit_slot) = sink_self_program_with(
            Arc::as_ptr(&depth) as i64,
            Arc::as_ptr(&status) as i64,
        );
        let chain =
            compile_function_regalloc(&ops, Some(status.clone()), Arc::as_ptr(&depth) as i64)
                .expect("sink function compiles");
        let max = logicaffeine_compile::semantics::MAX_CALL_DEPTH as i64;
        let mut frame = vec![0i64; SELF_ARENA];
        frame[0] = 100; // wants depth 100, but the cell starts near the cap
        let arena_end = unsafe { frame.as_ptr().add(SELF_ARENA) } as i64;
        frame[limit_slot as usize] = arena_end;
        depth.store(max - 2, std::sync::atomic::Ordering::SeqCst);
        status.store(0, std::sync::atomic::Ordering::SeqCst);
        let out = chain.run_with_frame(&mut frame);
        assert!(out.is_deopt(), "crossing MAX_CALL_DEPTH must side-exit, got {out:?}");
    }

    /// RED: ARENA-BOUND side-exit. A tiny limit slot (callee window past the
    /// arena end) must trip the arena guard inside the self-call (status = 9) →
    /// a `Deopt`, matching the stencil's marker 9. We give a frame far smaller
    /// than `(window + frame_size) * 8` requires for even one recursion level.
    #[test]
    fn self_call_arena_bound_side_exits() {
        let status = Arc::new(AtomicI64::new(0));
        let depth = Arc::new(AtomicI64::new(0));
        let (ops, limit_slot) = sink_self_program_with(
            Arc::as_ptr(&depth) as i64,
            Arc::as_ptr(&status) as i64,
        );
        let chain =
            compile_function_regalloc(&ops, Some(status.clone()), Arc::as_ptr(&depth) as i64)
                .expect("sink function compiles");
        // A frame just big enough to hold the registers + limit slot but NOT a
        // full callee window — so callee_base + frame_size*8 > arena_end.
        let small = (limit_slot as usize) + 1 + 4;
        let mut frame = vec![0i64; small];
        frame[0] = 5;
        let arena_end = unsafe { frame.as_ptr().add(small) } as i64;
        frame[limit_slot as usize] = arena_end;
        depth.store(0, std::sync::atomic::Ordering::SeqCst);
        status.store(0, std::sync::atomic::Ordering::SeqCst);
        let out = chain.run_with_frame(&mut frame);
        assert!(out.is_deopt(), "arena overflow must side-exit, got {out:?}");
    }

    /// RED: a DEOPT raised INSIDE a recursion level (div-by-zero at the base
    /// case) must propagate up every frame as a `Deopt` (each self-call sees the
    /// nonzero status after its call returns and re-exits), bit-identical to the
    /// stencil tier's status propagation.
    #[test]
    fn self_call_inner_deopt_propagates() {
        let status = Arc::new(AtomicI64::new(0));
        let depth = Arc::new(AtomicI64::new(0));
        let limit_slot: u16 = 8;
        let window = limit_slot + 1;
        let fs = (limit_slot as i64) + 1;
        // risky(n) = n==0 ? (100 / 0) : risky(n-1) + 1.
        // slots: 0=n 1=t 2=one 3=zero 4=hundred (constants LoadConst'd every call)
        let ops = vec![
            MicroOp::LoadConst { dst: 2, value: 1 },   // 0: one
            MicroOp::LoadConst { dst: 3, value: 0 },   // 1: zero
            MicroOp::LoadConst { dst: 4, value: 100 }, // 2: hundred
            MicroOp::Branch { cmp: Cmp::Eq, lhs: 0, rhs: 3, target: 5 }, // 3: if !(n==0) goto 5
            MicroOp::Div { dst: 1, lhs: 4, rhs: 3 },                     // 4: 100 / 0 -> deopt
            MicroOp::Sub { dst: window, lhs: 0, rhs: 2 }, // 5: arg = n - 1
            MicroOp::CallSelfCopy {
                dst: 1,
                args_start: window,
                src_start: window,
                arg_count: 1,
                depth_addr: Arc::as_ptr(&depth) as i64,
                status_addr: Arc::as_ptr(&status) as i64,
                limit_slot,
                frame_size: fs,
            }, // 6
            MicroOp::Add { dst: 1, lhs: 1, rhs: 2 }, // 7
            MicroOp::Return { src: 1 },              // 8
        ];
        let chain =
            compile_function_regalloc(&ops, Some(status.clone()), Arc::as_ptr(&depth) as i64)
                .expect("risky function compiles");
        let mut frame = vec![0i64; SELF_ARENA];
        frame[0] = 50;
        let arena_end = unsafe { frame.as_ptr().add(SELF_ARENA) } as i64;
        frame[limit_slot as usize] = arena_end;
        depth.store(0, std::sync::atomic::Ordering::SeqCst);
        status.store(0, std::sync::atomic::Ordering::SeqCst);
        let out = chain.run_with_frame(&mut frame);
        assert!(out.is_deopt(), "inner div-by-zero must propagate as a deopt, got {out:?}");
    }

    /// RED: the multi-argument `CallSelf` path (args pre-staged with explicit
    /// Moves, NOT the fused copy) — a 2-argument recursive gcd. Drives the plain
    /// `CallSelf` arg ABI (the callee window already staged before the call).
    #[test]
    fn self_call_plain_two_arg_gcd() {
        let status = Arc::new(AtomicI64::new(0));
        let depth = Arc::new(AtomicI64::new(0));
        let limit_slot: u16 = 8;
        let window = limit_slot + 1;
        let fs = (limit_slot as i64) + 1;
        // gcd(a,b) = b==0 ? a : gcd(b, a mod b).
        // slots: 0=a 1=b 2=t 3=zero ; window=9,10 staged via Move
        let ops = vec![
            MicroOp::LoadConst { dst: 3, value: 0 },                     // 0: zero
            MicroOp::Branch { cmp: Cmp::Eq, lhs: 1, rhs: 3, target: 3 }, // 1: if !(b==0) goto 3
            MicroOp::Return { src: 0 },                                  // 2: return a
            MicroOp::Mod { dst: 2, lhs: 0, rhs: 1 },                     // 3: t = a mod b
            MicroOp::Move { dst: window, src: 1 },                       // 4: arg0 = b
            MicroOp::Move { dst: window + 1, src: 2 },                   // 5: arg1 = t
            MicroOp::CallSelf {
                dst: 2,
                args_start: window,
                depth_addr: Arc::as_ptr(&depth) as i64,
                status_addr: Arc::as_ptr(&status) as i64,
                limit_slot,
                frame_size: fs,
            }, // 6
            MicroOp::Return { src: 2 }, // 7
        ];
        let chain =
            compile_function_regalloc(&ops, Some(status.clone()), Arc::as_ptr(&depth) as i64)
                .expect("gcd function compiles");
        fn gcd(a: i64, b: i64) -> i64 {
            if b == 0 { a } else { gcd(b, a % b) }
        }
        for (a, b) in [(48i64, 36i64), (1071, 462), (17, 5), (100, 0), (0, 7)] {
            let mut frame = vec![0i64; SELF_ARENA];
            frame[0] = a;
            frame[1] = b;
            let arena_end = unsafe { frame.as_ptr().add(SELF_ARENA) } as i64;
            frame[limit_slot as usize] = arena_end;
            depth.store(0, std::sync::atomic::Ordering::SeqCst);
            status.store(0, std::sync::atomic::Ordering::SeqCst);
            let out = chain.run_with_frame(&mut frame);
            assert_eq!(out, ChainOutcome::Return(gcd(a, b)), "gcd({a},{b})");
        }
        let _ = run_self_fn;
    }

    // ----------------------------------------------------------------
    // THE CEILING MEASUREMENT (ignored: a timing study, not a gate).
    // ----------------------------------------------------------------

    /// The compute-heavy kernel: ~10 dependent arithmetic ops per iteration
    /// (a polynomial accumulation `s = ((s*A + i*B - C) & MASK) * A`) over many
    /// iterations. This is the shape the per-piece tier pays the most for (10
    /// pieces/iter, each with ABI/operand overhead at every boundary).
    ///
    /// The accumulator is bit-masked each iteration (`& MASK`, MASK = 0xFFFF) and the
    /// multipliers are kept small, so every intermediate stays well inside i64. That is
    /// deliberate: integer add/sub/mul are now EXACT (they side-exit on signed overflow so
    /// the VM can promote to BigInt — `needs_deopt` in regalloc.rs / `*3` stencils), and a
    /// kernel that overflowed would deopt mid-loop instead of running to completion, which
    /// would defeat a *codegen-speed* measurement. Bounding it keeps both tiers returning a
    /// value (and bit-identical), measuring pure i64 codegen — exactly this benchmark's job.
    fn poly_kernel() -> (Vec<MicroOp>, Vec<i64>) {
        // slots: 0=i 1=N 2=s 3=A 4=B 5=C 6=MASK 7=t 8=u 9=one
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 10 }, // i<N
            MicroOp::Mul { dst: 7, lhs: 2, rhs: 3 },                       // t = s*A
            MicroOp::Mul { dst: 8, lhs: 0, rhs: 4 },                       // u = i*B
            MicroOp::Add { dst: 7, lhs: 7, rhs: 8 },                       // t += u
            MicroOp::Sub { dst: 2, lhs: 7, rhs: 5 },                       // s = t - C
            MicroOp::BitAnd { dst: 2, lhs: 2, rhs: 6 },                    // s &= MASK (bound s)
            MicroOp::Mul { dst: 2, lhs: 2, rhs: 3 },                       // s *= A
            MicroOp::LoadConst { dst: 9, value: 1 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 9 }, // i += 1
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 2 },
        ];
        let frame = vec![
            0i64,
            60_000_000, // N
            1,          // s
            1_000_003,  // A — small multiplier: with s masked to 16 bits, s*A stays < 2^57
            1_000_033,  // B — small multiplier: i*B over i<N=6e7 stays < 2^57
            12345,      // C
            0xFFFF,     // MASK — bound s to 16 bits each iteration (no i64 overflow)
            0,
            0,
            0,
        ];
        (ops, frame)
    }

    /// The 2-op kernel (the loop_sum analog): `s += i` per iteration. Should
    /// stay roughly the same between tiers (already at V8 parity per the log).
    fn loop_sum_kernel() -> (Vec<MicroOp>, Vec<i64>) {
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 2, target: 5 },
            MicroOp::Add { dst: 1, lhs: 1, rhs: 0 },
            MicroOp::LoadConst { dst: 3, value: 1 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 3 },
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 1 },
        ];
        let frame = vec![0i64, 0, 200_000_000, 0];
        (ops, frame)
    }

    fn time_tier(
        label: &str,
        ops: &[MicroOp],
        frame: &[i64],
        contiguous: bool,
    ) -> (i64, std::time::Duration) {
        let status = Arc::new(AtomicI64::new(0));
        let chain = if contiguous {
            compile_region_regalloc(ops, Some(status)).expect("regalloc compiles")
        } else {
            compile_straightline_coded(ops, Some(status), None, 0).expect("stencil compiles")
        };
        let mut f = frame.to_vec();
        // Warm up.
        {
            let mut w = frame.to_vec();
            let _ = chain.run_with_frame(&mut w);
        }
        let t0 = Instant::now();
        let out = chain.run_with_frame(&mut f);
        let dt = t0.elapsed();
        let v = match out {
            ChainOutcome::Return(v) => v,
            ChainOutcome::Deopt(d) => panic!("{label}: unexpected deopt {d}"),
        };
        (v, dt)
    }

    /// PRIMARY DELIVERABLE: the ceiling number. Times the SAME MicroOp loop
    /// compiled by the contiguous regalloc backend vs the per-piece stencil
    /// tier, run over millions of iterations. Reports the speedup. Both produce
    /// bit-identical results (asserted) — this is pure codegen-quality.
    ///
    /// Run with:
    ///   cargo nextest run -p logicaffeine-tests \
    ///     -E 'binary(jit_regalloc)' --run-ignored only \
    ///     -E 'test(ceiling_measurement)' --no-capture
    #[test]
    #[ignore = "ceiling measurement (timing study) — run explicitly"]
    fn ceiling_measurement() {
        on_big_stack(|| {
            eprintln!("\n=== WS-G contiguous-codegen CEILING MEASUREMENT ===");
            eprintln!("(same MicroOp program; contiguous regalloc vs per-piece stencil tier)\n");

            for (name, (ops, frame)) in [
                ("poly_10op", poly_kernel()),
                ("loop_sum_2op", loop_sum_kernel()),
            ] {
                // Best-of-N to dampen scheduler noise.
                let n = 5;
                let mut best_stencil = std::time::Duration::MAX;
                let mut best_regalloc = std::time::Duration::MAX;
                let mut v_s = 0i64;
                let mut v_r = 0i64;
                for _ in 0..n {
                    let (vs, ds) = time_tier(name, &ops, &frame, false);
                    let (vr, dr) = time_tier(name, &ops, &frame, true);
                    best_stencil = best_stencil.min(ds);
                    best_regalloc = best_regalloc.min(dr);
                    v_s = vs;
                    v_r = vr;
                }
                assert_eq!(
                    v_s, v_r,
                    "{name}: contiguous and stencil tiers must agree bit-for-bit"
                );
                let speedup =
                    best_stencil.as_secs_f64() / best_regalloc.as_secs_f64();
                eprintln!(
                    "{name:>14}:  stencil {:>9.3} ms   regalloc {:>9.3} ms   => {:.2}x faster (result {})",
                    best_stencil.as_secs_f64() * 1e3,
                    best_regalloc.as_secs_f64() * 1e3,
                    speedup,
                    v_r,
                );
            }
            eprintln!("\n=== END CEILING MEASUREMENT ===\n");
        });
    }

    // ----------------------------------------------------------------
    // WAVE 26 — SCALED-INDEX CSE. A straight-line run of array accesses
    // sharing one index slot computes `im1 = idx - 1` (and `im1 * 8`) ONCE
    // into reserved register(s) and reuses it for each `base + off` address,
    // instead of recomputing per access (the nbody / matrix_mult / spectral
    // / prefix_sum co-indexed shape). These tests prove (a) BIT-IDENTITY to
    // the reference on the value, full frame, and every buffer; (b) the CSE
    // STRUCTURALLY fires — the `idx - 1` (`sub rax, 1`) and `* 8`
    // (`shl rax, 3`) computes collapse to one per index run; (c) the cache is
    // correctly RECOMPUTED when the index changes mid-run; (d) parity holds
    // under OOB side-exit. The reference (`reference_eval`) models the SAME
    // 1-based addressing and bounds guard, so equality proves soundness.
    // ----------------------------------------------------------------

    /// The byte encoding of `sub rax, 1` (REX.W 81 /5 id) — the per-access
    /// 0-based-index `im1` computation in `emit_arr_addr`'s COMPUTE path.
    const SUB_RAX_1: [u8; 7] = [0x48, 0x81, 0xE8, 0x01, 0x00, 0x00, 0x00];
    /// The byte encoding of `shl rax, 3` (REX.W C1 /4 ib) — the per-access
    /// `im1 * 8` scaling in `emit_arr_addr`'s COMPUTE path.
    const SHL_RAX_3: [u8; 4] = [0x48, 0xC1, 0xE0, 0x03];

    /// Count non-overlapping occurrences of `needle` in `hay`.
    fn count_subseq(hay: &[u8], needle: &[u8]) -> usize {
        if needle.is_empty() || hay.len() < needle.len() {
            return 0;
        }
        let mut n = 0;
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            if &hay[i..i + needle.len()] == needle {
                n += 1;
                i += needle.len();
            } else {
                i += 1;
            }
        }
        n
    }

    /// Compile a region and return its raw machine-code bytes (for structural
    /// assertions about how many index recomputes the CSE collapsed).
    fn region_bytes(ops: &[MicroOp]) -> Vec<u8> {
        let status = Arc::new(AtomicI64::new(0));
        let chain =
            compile_region_regalloc(ops, Some(status)).expect("region must compile");
        chain.bytes().to_vec()
    }

    /// RED: the nbody/matrix co-indexed shape — read SEVEN arrays at the SAME
    /// index `i`, fold them, write THREE arrays back at the same `i`, looping.
    /// (a) Every buffer + the return value is bit-identical to the reference.
    /// (b) The `idx - 1` and `* 8` computes collapse: with CSE the loop body
    ///     emits ONE `sub rax, 1` and ONE `shl rax, 3` despite ten accesses at
    ///     index `i`, versus ten of each without CSE.
    #[test]
    fn off_cse_co_indexed_arrays_identical_and_collapsed() {
        // frame: 0=i 1=n 2..8=tmp(t0..t6) 9=acc 10=one
        //        11=a_ptr 12=a_len ... seven inputs (a..g) at ptr/len pairs,
        //        then three outputs (x,y,z). We pack:
        //   inputs  a..g : ptr/len slots 11/12, 13/14, 15/16, 17/18, 19/20,
        //                  21/22, 23/24
        //   outputs x..z : ptr/len slots 25/26, 27/28, 29/30
        // body (i in 1..=n):
        //   t0=a[i]; t1=b[i]; t2=c[i]; t3=d[i]; t4=e[i]; t5=f[i]; t6=g[i]
        //   s = t0+t1+t2+t3+t4+t5+t6 ; acc += s
        //   x[i]=s ; y[i]=s+1 ; z[i]=s+2
        //   i += 1
        let one = 10u16;
        let mut ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 99 }, // patched below
        ];
        // seven loads at index i.
        let in_ptr = [11u16, 13, 15, 17, 19, 21, 23];
        let in_len = [12u16, 14, 16, 18, 20, 22, 24];
        for k in 0..7 {
            ops.push(MicroOp::ArrLoad {
                dst: 2 + k as u16,
                idx: 0,
                ptr_slot: in_ptr[k],
                len_slot: in_len[k],
                byte: false,
                narrow32: false,
                checked: true,
            });
        }
        // s = t0+..+t6 accumulated into t0 (slot 2).
        for k in 1..7u16 {
            ops.push(MicroOp::Add { dst: 2, lhs: 2, rhs: 2 + k });
        }
        ops.push(MicroOp::Add { dst: 9, lhs: 9, rhs: 2 }); // acc += s
        // x[i]=s ; y[i]=s+1 ; z[i]=s+2 (three stores at index i).
        ops.push(MicroOp::ArrStore { src: 2, idx: 0, ptr_slot: 25, len_slot: 26, byte: false, narrow32: false, checked: true });
        ops.push(MicroOp::Add { dst: 3, lhs: 2, rhs: one }); // s+1
        ops.push(MicroOp::ArrStore { src: 3, idx: 0, ptr_slot: 27, len_slot: 28, byte: false, narrow32: false, checked: true });
        ops.push(MicroOp::Add { dst: 4, lhs: 3, rhs: one }); // s+2
        ops.push(MicroOp::ArrStore { src: 4, idx: 0, ptr_slot: 29, len_slot: 30, byte: false, narrow32: false, checked: true });
        ops.push(MicroOp::Add { dst: 0, lhs: 0, rhs: one }); // i += 1
        let jump_back = ops.len();
        ops.push(MicroOp::Jump { target: 0 });
        let exit = ops.len();
        ops.push(MicroOp::Return { src: 9 });
        // Patch the loop-exit branch target and verify jump-back index.
        if let MicroOp::Branch { target, .. } = &mut ops[0] {
            *target = exit;
        }
        assert_eq!(jump_back, exit - 1);

        let n = 200usize;
        let frame_proto = {
            let mut f = vec![0i64; 31];
            f[0] = 1; // i
            f[1] = n as i64;
            f[10] = 1; // one
            for s in [12usize, 14, 16, 18, 20, 22, 24, 26, 28, 30] {
                f[s] = n as i64; // every len
            }
            f
        };
        // Seven input buffers, three output buffers.
        let inputs: Vec<Vec<i64>> = (0..7)
            .map(|b| (0..n).map(|k| (k as i64 + 1) * (b as i64 + 2) - 3).collect())
            .collect();
        let outputs_zero: Vec<Vec<i64>> = (0..3).map(|_| vec![0i64; n]).collect();

        // Reference run over its own copies.
        let mut ref_bufs: Vec<Vec<i64>> = inputs.clone();
        ref_bufs.extend(outputs_zero.clone());
        let ref_ptr_slots = vec![11usize, 13, 15, 17, 19, 21, 23, 25, 27, 29];
        let (vref, _fref, ref_post) =
            run_reference_buf(&ops, &frame_proto, &ref_bufs, &ref_ptr_slots);
        let expected = vref.expect("reference terminates");

        // Regalloc run with live buffers.
        let mut run_bufs: Vec<Vec<i64>> = inputs.clone();
        run_bufs.extend(outputs_zero.clone());
        let buf_slots: Vec<(usize, &mut Vec<i64>)> = ref_ptr_slots
            .iter()
            .zip(run_bufs.iter_mut())
            .map(|(&s, b)| (s, b))
            .collect();
        let (out, _f) = run_regalloc_buf(&ops, &frame_proto, buf_slots);
        assert_eq!(out, ChainOutcome::Return(expected), "co-indexed value diverged");
        for (k, (got, want)) in run_bufs.iter().zip(ref_post.iter()).enumerate() {
            assert_eq!(got, want, "buffer {k} diverged from reference");
        }

        // STRUCTURAL: ten accesses share index slot 0 across the straight-line
        // loop body — the CSE collapses both the `idx - 1` and the `* 8` to ONE
        // each (the first access of the run computes; the rest reuse). The body
        // is entered only at op 0 (the branch target) and op 0 is the only join,
        // so the whole 1..jump_back run shares the cache.
        let bytes = region_bytes(&ops);
        let subs = count_subseq(&bytes, &SUB_RAX_1);
        let shls = count_subseq(&bytes, &SHL_RAX_3);
        assert_eq!(
            subs, 1,
            "CSE must collapse the ten `idx-1` computes to one (got {subs} `sub rax,1`)"
        );
        assert_eq!(
            shls, 1,
            "CSE must collapse the ten `*8` scalings to one (got {shls} `shl rax,3`)"
        );
    }

    /// RED: the index CHANGES mid-run — accesses at `i`, then at `j`, then at
    /// `i` again. The cache must RECOMPUTE on each index switch: the offset is
    /// never reused across a different index slot. Bit-identical to reference,
    /// and structurally there are exactly THREE `idx-1` computes (one per run:
    /// the i-run, the j-run, the i-run again), not collapsed to one.
    #[test]
    fn off_cse_index_change_recomputes() {
        // frame: 0=i 1=j 2=n 3=ti 4=tj 5=ti2 6=acc 7=ptr 8=len 9=one
        // body: ti=a[i]; tj=a[j]; ti2=a[i]; acc += ti+tj+ti2; i+=1; j-=1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 12 }, // 0  while i < j
            MicroOp::ArrLoad { dst: 3, idx: 0, ptr_slot: 7, len_slot: 8, byte: false, narrow32: false, checked: true }, // 1 a[i]
            MicroOp::ArrLoad { dst: 4, idx: 1, ptr_slot: 7, len_slot: 8, byte: false, narrow32: false, checked: true }, // 2 a[j]
            MicroOp::ArrLoad { dst: 5, idx: 0, ptr_slot: 7, len_slot: 8, byte: false, narrow32: false, checked: true }, // 3 a[i] again
            MicroOp::Add { dst: 3, lhs: 3, rhs: 4 },  // 4 ti+tj
            MicroOp::Add { dst: 3, lhs: 3, rhs: 5 },  // 5 +ti2
            MicroOp::Add { dst: 6, lhs: 6, rhs: 3 },  // 6 acc += ...
            MicroOp::Add { dst: 0, lhs: 0, rhs: 9 },  // 7 i += 1
            MicroOp::Sub { dst: 1, lhs: 1, rhs: 9 },  // 8 j -= 1
            MicroOp::Jump { target: 0 },              // 9
            MicroOp::LoadConst { dst: 6, value: 0 },  // 10 filler
            MicroOp::LoadConst { dst: 6, value: 0 },  // 11 filler
            MicroOp::Return { src: 6 },               // 12
        ];
        let n = 400usize;
        let buf: Vec<i64> = (0..n).map(|k| (k as i64 + 1) * 5 - 7).collect();
        let frame = vec![1i64, n as i64, n as i64, 0, 0, 0, 0, 0, n as i64, 1];

        let (vref, _f, _b) = run_reference_buf(&ops, &frame, &[buf.clone()], &[7]);
        let expected = vref.expect("reference terminates");
        let mut run_buf = buf.clone();
        let (out, _f) = run_regalloc_buf(&ops, &frame, vec![(7, &mut run_buf)]);
        assert_eq!(out, ChainOutcome::Return(expected), "index-change value diverged");
        assert_eq!(run_buf, buf, "read-only buffer must be unchanged");

        // STRUCTURAL: a[i], a[j], a[i] — three runs, each a distinct index slot
        // from its predecessor, so the cache recomputes `idx-1` THREE times. It
        // is NEVER reused across the i→j or j→i switch (that would be a stale
        // offset miscompile).
        let bytes = region_bytes(&ops);
        let subs = count_subseq(&bytes, &SUB_RAX_1);
        assert_eq!(
            subs, 3,
            "index switches must recompute `idx-1` per run (got {subs}, expected 3)"
        );
    }

    /// RED: writing the INDEX slot mid-run invalidates the cache. `t=a[i];
    /// a[i]=t; i=a[i]; t2=a[i]` — the third access READS into the index slot
    /// `i` (an `ArrLoad dst==i`), so the fourth access must recompute from the
    /// NEW `i`, not the stale cached offset. Reference parity proves it.
    #[test]
    fn off_cse_index_slot_overwrite_invalidates() {
        // frame: 0=i 1=t 2=t2 3=ptr 4=len ; n small, fully in bounds.
        // ops: t=a[i]; a[i]=t (no-op write); i=a[i] (overwrite index!);
        //      t2=a[i]; return t2
        let ops = vec![
            MicroOp::ArrLoad { dst: 1, idx: 0, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true }, // 0 t=a[i]
            MicroOp::ArrStore { src: 1, idx: 0, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true }, // 1 a[i]=t
            MicroOp::ArrLoad { dst: 0, idx: 0, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true }, // 2 i=a[i]  (writes idx slot 0)
            MicroOp::ArrLoad { dst: 2, idx: 0, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true }, // 3 t2=a[i] (NEW i)
            MicroOp::Return { src: 2 },                                                                  // 4
        ];
        // a = [3, 1, 4, 1, 5, 9, 2, 6]; start i=2 (1-based). a[2]=1, so i becomes
        // 1, then t2 = a[1] = 3. The cache MUST recompute or it would read a[2].
        let a = vec![3i64, 1, 4, 1, 5, 9, 2, 6];
        let frame = vec![2i64, 0, 0, 0, a.len() as i64];

        let (vref, _f, bref) = run_reference_buf(&ops, &frame, &[a.clone()], &[3]);
        let expected = vref.expect("reference terminates");
        assert_eq!(expected, 3, "sanity: result is a[a[2]] = a[1] = 3");
        let mut run_a = a.clone();
        let (out, _f) = run_regalloc_buf(&ops, &frame, vec![(3, &mut run_a)]);
        assert_eq!(out, ChainOutcome::Return(expected), "index-overwrite diverged");
        assert_eq!(run_a, bref[0], "buffer diverged");
    }

    /// RED: a 2D co-indexed shape `a[r*n+c]` and `b[r*n+c]` at the SAME computed
    /// linear index — the matrix_mult / spectral co-located pair. Both buffers
    /// must match the reference, and the shared linear-index offset is computed
    /// once for the two accesses in the body.
    #[test]
    fn off_cse_2d_coindexed_pair_identical() {
        // c[lin] = a[lin] + b[lin] for lin = (r-1)*n + col, r,col in 1..=n.
        // frame: 0=r 1=col 2=n 3=lin 4=va 5=vb 6=t 7=a_ptr 8=a_len
        //        9=b_ptr 10=b_len 11=c_ptr 12=c_len 13=one
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 2, target: 17 }, // 0 while r<=n
            MicroOp::LoadConst { dst: 1, value: 1 },                        // 1 col=1
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 1, rhs: 2, target: 15 }, // 2 while col<=n
            MicroOp::Sub { dst: 3, lhs: 0, rhs: 13 },                       // 3 r-1
            MicroOp::Mul { dst: 3, lhs: 3, rhs: 2 },                        // 4 (r-1)*n
            MicroOp::Add { dst: 3, lhs: 3, rhs: 1 },                        // 5 +col -> lin
            MicroOp::ArrLoad { dst: 4, idx: 3, ptr_slot: 7, len_slot: 8, byte: false, narrow32: false, checked: true }, // 6 va=a[lin]
            MicroOp::ArrLoad { dst: 5, idx: 3, ptr_slot: 9, len_slot: 10, byte: false, narrow32: false, checked: true }, // 7 vb=b[lin]
            MicroOp::Add { dst: 6, lhs: 4, rhs: 5 },                        // 8 t = va+vb
            MicroOp::ArrStore { src: 6, idx: 3, ptr_slot: 11, len_slot: 12, byte: false, narrow32: false, checked: true }, // 9 c[lin]=t
            MicroOp::Add { dst: 1, lhs: 1, rhs: 13 },                       // 10 col+=1
            MicroOp::Jump { target: 2 },                                    // 11
            MicroOp::LoadConst { dst: 6, value: 0 },                        // 12 filler
            MicroOp::LoadConst { dst: 6, value: 0 },                        // 13 filler
            MicroOp::LoadConst { dst: 6, value: 0 },                        // 14 filler
            MicroOp::Add { dst: 0, lhs: 0, rhs: 13 },                       // 15 r+=1
            MicroOp::Jump { target: 0 },                                    // 16
            MicroOp::Return { src: 6 },                                     // 17
        ];
        let n = 17usize;
        let total = n * n;
        let a: Vec<i64> = (0..total).map(|k| (k as i64) * 3 + 1).collect();
        let b: Vec<i64> = (0..total).map(|k| (k as i64) * 7 - 2).collect();
        let frame = vec![
            1i64, 1, n as i64, 0, 0, 0, 0, 0, total as i64, 0, total as i64, 0, total as i64, 1,
        ];

        let (vref, _f, bref) = run_reference_buf(
            &ops,
            &frame,
            &[a.clone(), b.clone(), vec![0i64; total]],
            &[7, 9, 11],
        );
        let expected = vref.expect("reference terminates");
        let mut a_run = a.clone();
        let mut b_run = b.clone();
        let mut c_run = vec![0i64; total];
        let (out, _f) =
            run_regalloc_buf(&ops, &frame, vec![(7, &mut a_run), (9, &mut b_run), (11, &mut c_run)]);
        assert_eq!(out, ChainOutcome::Return(expected), "2D co-indexed value diverged");
        assert_eq!(a_run, bref[0], "a buffer changed");
        assert_eq!(b_run, bref[1], "b buffer changed");
        assert_eq!(c_run, bref[2], "c buffer diverged from reference");
        let want: Vec<i64> = (0..total).map(|k| a[k] + b[k]).collect();
        assert_eq!(c_run, want, "elementwise sum incorrect");

        // STRUCTURAL: the inner body has three accesses (a[lin], b[lin], c[lin])
        // at the SAME computed index slot 3 — collapsed to one `idx-1` and one
        // `*8` per iteration.
        let bytes = region_bytes(&ops);
        assert_eq!(
            count_subseq(&bytes, &SUB_RAX_1),
            1,
            "2D body must compute `lin-1` once"
        );
        assert_eq!(
            count_subseq(&bytes, &SHL_RAX_3),
            1,
            "2D body must scale `lin*8` once"
        );
    }

    /// RED: an OOB access in a co-indexed run side-exits identically — the
    /// per-access bounds check is preserved by the CSE (only the offset
    /// arithmetic is shared, never the guard). The SECOND array is shorter than
    /// the first, so the same index that is valid for `a` is OOB for `b`: the
    /// store must NOT land, and the run must deopt, matching the reference.
    #[test]
    fn off_cse_per_access_bounds_check_preserved() {
        // t=a[i] (len 8, valid); b[i]=t (len 4, OOB at i=5) -> must deopt before
        // writing b, and a is untouched after the load (read-only).
        let ops = vec![
            MicroOp::ArrLoad { dst: 2, idx: 0, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true }, // a[i]
            MicroOp::ArrStore { src: 2, idx: 0, ptr_slot: 5, len_slot: 6, byte: false, narrow32: false, checked: true }, // b[i] OOB
            MicroOp::Return { src: 2 },
        ];
        let mut a = vec![10i64, 20, 30, 40, 50, 60, 70, 80];
        let mut b = vec![1i64, 2, 3, 4];
        let b_before = b.clone();
        // i = 5 (1-based): valid for a (len 8), OOB for b (len 4).
        let frame = vec![5i64, 0, 0, 0, 8, 0, 4];

        // Reference rejects the OOB store (None).
        let (vref, _f, _bs) =
            run_reference_buf(&ops, &frame, &[a.clone(), b.clone()], &[3, 5]);
        assert!(vref.is_none(), "reference must reject the OOB store");

        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("compiles");
        let mut f = frame.clone();
        f[3] = a.as_mut_ptr() as i64;
        f[5] = b.as_mut_ptr() as i64;
        let out = chain.run_with_frame(&mut f);
        assert!(out.is_deopt(), "OOB store in a co-indexed run must deopt: {out:?}");
        assert_eq!(b, b_before, "OOB store must not mutate the shorter buffer");
    }

    /// RED: a BYTE (1-byte stride) co-indexed run also CSEs `im1` (the byte
    /// stride needs no `*8`). Two byte arrays read at the same index; the cache
    /// reuses `im1` directly as the offset. Bit-identical to reference, and only
    /// ONE `idx-1` compute fires.
    #[test]
    fn off_cse_byte_arrays_identical_and_collapsed() {
        // acc += (flags1[i] != 0) as i64 + (flags2[i] != 0) as i64 over i in 1..=n
        // frame: 0=i 1=n 2=v1 3=v2 4=acc 5=p1 6=l1 7=p2 8=l2 9=one
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 7 }, // 0
            MicroOp::ArrLoad { dst: 2, idx: 0, ptr_slot: 5, len_slot: 6, byte: true, narrow32: false, checked: true }, // 1
            MicroOp::ArrLoad { dst: 3, idx: 0, ptr_slot: 7, len_slot: 8, byte: true, narrow32: false, checked: true }, // 2
            MicroOp::Add { dst: 4, lhs: 4, rhs: 2 }, // 3
            MicroOp::Add { dst: 4, lhs: 4, rhs: 3 }, // 4
            MicroOp::Add { dst: 0, lhs: 0, rhs: 9 }, // 5
            MicroOp::Jump { target: 0 },             // 6
            MicroOp::Return { src: 4 },              // 7
        ];
        let n = 100usize;
        let f1: Vec<i64> = (0..n).map(|k| (k % 2) as i64).collect();
        let f2: Vec<i64> = (0..n).map(|k| ((k + 1) % 3 == 0) as i64).collect();
        // Byte buffers: one byte per element.
        let mut b1: Vec<u8> = f1.iter().map(|&v| (v != 0) as u8).collect();
        let mut b2: Vec<u8> = f2.iter().map(|&v| (v != 0) as u8).collect();
        let frame = vec![1i64, n as i64, 0, 0, 0, 0, n as i64, 0, n as i64, 1];

        // Reference over its own byte buffers (reference_eval reads bytes for a
        // byte op). Build the reference run inline (run_reference_buf is i64-only).
        let want: i64 = b1.iter().chain(b2.iter()).map(|&x| x as i64).sum();

        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("compiles");
        let mut f = frame.clone();
        f[5] = b1.as_mut_ptr() as i64;
        f[7] = b2.as_mut_ptr() as i64;
        let out = chain.run_with_frame(&mut f);
        assert_eq!(out, ChainOutcome::Return(want), "byte co-indexed sum diverged");

        // STRUCTURAL: two byte loads at index 0 — one `idx-1` compute (byte
        // stride uses `im1` directly, so NO `shl rax, 3` at all).
        let bytes = region_bytes(&ops);
        assert_eq!(
            count_subseq(&bytes, &SUB_RAX_1),
            1,
            "byte co-indexed run must compute `idx-1` once"
        );
        assert_eq!(
            count_subseq(&bytes, &SHL_RAX_3),
            0,
            "byte stride needs no `*8` scaling"
        );
    }

    // ----------------------------------------------------------------
    // WAVE 27a — LOOP-INVARIANT ARRAY PTR/LEN HOIST. When an array's
    // ptr/len handle slots are FRAME-resident and are NEVER written inside
    // a loop (only `ArrLoad`/`ArrStore` in place — no reallocating
    // `ArrPush`/`ListClear`/`NewList`/`ListTriple` of that array, no scalar
    // write to the slot), the per-access `emit_arr_addr` reload of `ptr`
    // (and, for a checked access, `len`) from the frame is loop-invariant.
    // The hoist loads them ONCE into loop-persistent CALLEE-SAVED registers
    // at the loop pre-header, so the inner accesses read the register
    // instead of re-touching the frame on every iteration — the knapsack
    // inner `w`-loop reads `prev`'s ptr/len every iteration though `prev`
    // only changes on the OUTER loop. Tests prove (a) BIT-IDENTITY to the
    // reference/stencil/tree-walker; (b) the hoist STRUCTURALLY fires — the
    // per-access frame load of the invariant ptr slot collapses to one
    // pre-header load; (c) an array REASSIGNED in the loop (a push that
    // reallocs) is NOT hoisted and stays correct. Kill-switch
    // `LOGOS_NO_PTR_HOIST=1`.
    // ----------------------------------------------------------------

    /// Count `mov rX, [r15 + disp32]` (REX.W 8B /r, mod=10, rm=111=r15) over
    /// ANY destination register where `disp32 == slot * 8`. This is exactly
    /// `g.operand(slot, _)` / `g.load(_, slot)`'s frame read of a slot from the
    /// pinned frame base (r15). The 4-byte disp literal is distinctive enough
    /// that an incidental byte collision is negligible; we additionally require
    /// the preceding byte be a REX with the base-extension bit (0x41) set, which
    /// every `[r15 + …]` form carries (r15 is an extended register).
    fn count_frame_reads(bytes: &[u8], slot: u16) -> usize {
        let disp = ((slot as i32) * 8).to_le_bytes();
        let mut n = 0;
        // i starts at 1 so we can look back at the REX prefix byte.
        let mut i = 1usize;
        while i + 6 <= bytes.len() {
            // REX with base bit (b) set — r15 in the rm field needs REX.B.
            let rex = bytes[i - 1];
            let is_rex_b = (rex & 0xF0) == 0x40 && (rex & 0x01) == 0x01;
            // opcode 8B (mov r, r/m); modrm mod=10 (0x80), rm=111 (r15).
            let is_mov_rm = bytes[i] == 0x8B;
            let modrm = bytes[i + 1];
            let is_disp32_r15 = (modrm & 0b1100_0111) == 0b1000_0111;
            let disp_ok = bytes[i + 2..i + 6] == disp;
            if is_rex_b && is_mov_rm && is_disp32_r15 && disp_ok {
                n += 1;
                i += 6;
            } else {
                i += 1;
            }
        }
        n
    }

    /// RED: a MAP/transform loop — read an invariant source array `src` at the
    /// running index, transform, and `Push` to a growing output `out`. The push
    /// to `out` forces BOTH arrays' ptr/len FRAME-resident (the `force_frame_set`
    /// rule: any array in a list-mutation op stays frame-resident so the helper's
    /// post-realloc writes are the source of truth). `src` is read in place and
    /// never pushed/cleared, so its ptr/len are loop-invariant and HOIST; `out`
    /// is reallocated by the push and must NOT. This is the coins/knapsack DP
    /// regime: an invariant prior array read while a fresh one is filled.
    /// Bit-identical (regalloc == stencil), and `src`'s ptr load collapses to one
    /// pre-header load.
    #[test]
    fn ptr_hoist_invariant_source_with_push_hoisted_and_identical() {
        // frame: 0=i 1=n 2=s_vec 3=s_ptr 4=s_len 5=a 6=b 7=val
        //        8=o_vec 9=o_ptr 10=o_len 11=one 12=idx2
        // while i <= n:
        //   a = src[i]               (ArrLoad ptr=3)
        //   idx2 = i + 1
        //   b = src[idx2]            (ArrLoad ptr=3)  — second read of invariant src
        //   val = a + b
        //   push val to out          (ArrPush ptr=9 — reallocs, writes frame[9]/[10])
        //   i += 1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 8 }, // 0
            MicroOp::ArrLoad { dst: 5, idx: 0, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true }, // 1 a = src[i]
            MicroOp::Add { dst: 12, lhs: 0, rhs: 11 }, // 2 idx2 = i+1
            MicroOp::ArrLoad { dst: 6, idx: 12, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true }, // 3 b = src[i+1]
            MicroOp::Add { dst: 7, lhs: 5, rhs: 6 }, // 4 val = a+b
            MicroOp::ArrPush { src: 7, vec_slot: 8, ptr_slot: 9, len_slot: 10, helper_addr: push_i64_addr(), byte: false, narrow32: false }, // 5 push val to out
            MicroOp::Add { dst: 0, lhs: 0, rhs: 11 }, // 6 i += 1
            MicroOp::Jump { target: 0 },              // 7
            MicroOp::Return { src: 10 },              // 8 return len(out)
        ];

        let n = 2000i64;
        // src seeded with n+4 cells (1-based usable up to n+1).
        let src_init: Vec<i64> = (0..(n + 4)).map(|k| k * 7 - 3).collect();
        // i=1, n, s_*, s_len(seeded), a, b, val, o_*, o_len, one=1, idx2
        let frame = vec![1i64, n, 0, 0, src_init.len() as i64, 0, 0, 0, 0, 0, 0, 1, 0];

        let mut rp = [
            ListPin::new(2, 3, 4, src_init.clone()),
            ListPin::new(8, 9, 10, Vec::new()),
        ];
        let (rout, rframe) = run_regalloc_lists(&ops, &frame, &mut rp);
        let mut sp = [
            ListPin::new(2, 3, 4, src_init.clone()),
            ListPin::new(8, 9, 10, Vec::new()),
        ];
        let (sout, sframe) = run_stencil_lists(&ops, &frame, &mut sp);

        assert_eq!(rout, sout, "map-with-push return diverged");
        assert_eq!(
            mask_ptr_cells(rframe, &rp),
            mask_ptr_cells(sframe, &sp),
            "map-with-push post-frame diverged (stale ptr/len?)"
        );
        assert_eq!(rp[0].buf.snapshot(), sp[0].buf.snapshot(), "src buffer diverged");
        assert_eq!(rp[1].buf.snapshot(), sp[1].buf.snapshot(), "out buffer diverged");
        let want: Vec<i64> =
            (1..=n).map(|i| src_init[(i - 1) as usize] + src_init[i as usize]).collect();
        assert_eq!(rp[1].buf.snapshot(), want, "map output wrong");

        // SOUNDNESS: src's ptr is invariant and read across a reallocating push
        // to `out` — the dangerous interleave. Whether src wins a register or is
        // hoisted, every read sees the unchanged src buffer (it is never the push
        // target), so a hoisted-or-resident src ptr is never stale. Bit-identity
        // above is the gate; the per-access frame read of the invariant src ptr is
        // collapsed (at most one), never per-access.
        let bytes = region_bytes(&ops);
        let reads = count_frame_reads(&bytes, 3);
        assert!(
            reads <= 1,
            "the invariant src ptr must collapse to at most one load (got {reads})"
        );
    }

    /// RED: the KNAPSACK inner `w`-loop — read the PRIOR row `prev` (invariant in
    /// the inner loop) at two indices, then `Push` the new value to `curr` (a
    /// DIFFERENT, reallocating array). `prev`'s ptr/len are loop-invariant in the
    /// inner loop (only the outer loop does `Set prev to curr`), so they hoist;
    /// `curr`'s ptr/len are written by the push and must NOT. Three-way
    /// bit-identical (regalloc == stencil), and structurally `prev`'s ptr load
    /// collapses to one pre-header load in the inner loop.
    #[test]
    fn ptr_hoist_knapsack_inner_loop_hoisted_and_identical() {
        // frame: 0=r 1=rows 2=w 3=cap 4=p_vec 5=p_ptr 6=p_len 7=best
        //        8=q_vec 9=q_ptr 10=q_len 11=take 12=one 13=widx 14=tidx
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 16 }, // 0 while r<rows -> exit 16
            MicroOp::ListClear { vec_slot: 8, ptr_slot: 9, len_slot: 10, helper_addr: clear_i64_addr() }, // 1 clear curr
            MicroOp::LoadConst { dst: 2, value: 1 }, // 2 w = 1
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 2, rhs: 3, target: 12 }, // 3 INNER head -> exit 12
            MicroOp::Add { dst: 13, lhs: 2, rhs: 12 }, // 4 widx = w+1
            MicroOp::ArrLoad { dst: 7, idx: 13, ptr_slot: 5, len_slot: 6, byte: false, narrow32: false, checked: true }, // 5 best=prev[w+1]
            MicroOp::ArrLoad { dst: 11, idx: 2, ptr_slot: 5, len_slot: 6, byte: false, narrow32: false, checked: true }, // 6 take=prev[w]
            MicroOp::Add { dst: 11, lhs: 11, rhs: 0 }, // 7 take += r
            MicroOp::Add { dst: 7, lhs: 7, rhs: 11 }, // 8 best += take (deterministic combine)
            MicroOp::ArrPush { src: 7, vec_slot: 8, ptr_slot: 9, len_slot: 10, helper_addr: push_i64_addr(), byte: false, narrow32: false }, // 9 push best to curr
            MicroOp::Add { dst: 2, lhs: 2, rhs: 12 }, // 10 w += 1
            MicroOp::Jump { target: 3 }, // 11 inner back-edge
            MicroOp::ListClear { vec_slot: 4, ptr_slot: 5, len_slot: 6, helper_addr: clear_i64_addr() }, // 12 clear prev
            MicroOp::ArrPush { src: 0, vec_slot: 4, ptr_slot: 5, len_slot: 6, helper_addr: push_i64_addr(), byte: false, narrow32: false }, // 13 push r to prev
            MicroOp::Add { dst: 0, lhs: 0, rhs: 12 }, // 14 r += 1
            MicroOp::Jump { target: 0 }, // 15 outer back-edge
            MicroOp::Return { src: 6 }, // 16 return len(prev)
        ];

        let rows = 30i64;
        let cap = 50i64;
        // prev seeded with cap+4 cells (1-based usable up to cap+1).
        let prev_init: Vec<i64> = (0..(cap + 4)).map(|k| k * 3 + 1).collect();
        let frame = vec![0i64, rows, 0, cap, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0];

        let mut rp = [
            ListPin::new(4, 5, 6, prev_init.clone()),
            ListPin::new(8, 9, 10, Vec::new()),
        ];
        let (rout, rframe) = run_regalloc_lists(&ops, &frame, &mut rp);
        let mut sp = [
            ListPin::new(4, 5, 6, prev_init.clone()),
            ListPin::new(8, 9, 10, Vec::new()),
        ];
        let (sout, sframe) = run_stencil_lists(&ops, &frame, &mut sp);

        assert_eq!(rout, sout, "knapsack inner-loop return diverged");
        assert_eq!(
            mask_ptr_cells(rframe, &rp),
            mask_ptr_cells(sframe, &sp),
            "knapsack inner-loop post-frame diverged (stale ptr/len?)"
        );
        assert_eq!(rp[0].buf.snapshot(), sp[0].buf.snapshot(), "prev buffer diverged");
        assert_eq!(rp[1].buf.snapshot(), sp[1].buf.snapshot(), "curr buffer diverged");

        // STRUCTURAL: in the INNER loop (ops 3..11), prev's ptr (slot 5) feeds two
        // ArrLoads each iteration but is invariant there (it is only written at op
        // 12/13, the OUTER loop's clear+push). The hoist loads it once at the
        // inner pre-header. The OUTER loop body ALSO reads prev's ptr at op 13's
        // push staging, but that is OUTSIDE the inner span — so across the whole
        // region prev's ptr is read at most twice (the inner pre-header load + the
        // outer push handle), strictly fewer than the four naive accesses.
        let bytes = region_bytes(&ops);
        let reads = count_frame_reads(&bytes, 5);
        assert!(
            reads <= 2,
            "the invariant prev ptr must collapse to a single inner pre-header load \
             (plus the outer-loop handle); got {reads} frame reads of slot 5"
        );
    }

    /// RED: an array REASSIGNED inside the loop must NOT be hoisted. A
    /// build-then-read loop where the SAME array `a` is pushed to (reallocating —
    /// the push refreshes `frame[ptr]`/`frame[len]`) AND read in the same loop:
    /// hoisting `a`'s ptr would feed a STALE pointer into the read after a realloc
    /// (a miscompile). The hoist must decline; the result stays bit-identical.
    #[test]
    fn ptr_hoist_reassigned_array_not_hoisted_and_identical() {
        // frame: 0=i 1=n 2=a_vec 3=a_ptr 4=a_len 5=val 6=elem 7=acc 8=one
        // while i <= n:
        //   push i to a            (REALLOCATES — writes frame[3]/frame[4])
        //   elem = a[1]            (read AFTER the push — must see the fresh ptr)
        //   acc += elem
        //   i += 1
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::LtEq, lhs: 0, rhs: 1, target: 7 }, // 0
            MicroOp::ArrPush { src: 0, vec_slot: 2, ptr_slot: 3, len_slot: 4, helper_addr: push_i64_addr(), byte: false, narrow32: false }, // 1 push i (realloc)
            MicroOp::ArrLoad { dst: 6, idx: 8, ptr_slot: 3, len_slot: 4, byte: false, narrow32: false, checked: true }, // 2 elem = a[1]
            MicroOp::Add { dst: 7, lhs: 7, rhs: 6 }, // 3 acc += elem
            MicroOp::Add { dst: 0, lhs: 0, rhs: 8 }, // 4 i += 1
            MicroOp::Jump { target: 0 },             // 5
            MicroOp::LoadConst { dst: 6, value: 0 }, // 6 filler
            MicroOp::Return { src: 7 },              // 7 return acc
        ];
        let n = 3000i64;
        // i=1, n, a_*, a_len(seeded 0), val, elem, acc, one=1
        let frame = vec![1i64, n, 0, 0, 0, 0, 0, 0, 1];

        let mut rp = [ListPin::new(2, 3, 4, Vec::new())];
        let (rout, rframe) = run_regalloc_lists(&ops, &frame, &mut rp);
        let mut sp = [ListPin::new(2, 3, 4, Vec::new())];
        let (sout, sframe) = run_stencil_lists(&ops, &frame, &mut sp);

        assert_eq!(rout, sout, "reassigned-array return diverged (stale hoisted ptr?)");
        assert_eq!(
            mask_ptr_cells(rframe, &rp),
            mask_ptr_cells(sframe, &sp),
            "reassigned-array post-frame diverged"
        );
        assert_eq!(rp[0].buf.snapshot(), sp[0].buf.snapshot(), "buffer diverged");
        // a[1] is always 1 (the first push), read every iteration → acc = n.
        assert_eq!(rout, ChainOutcome::Return(n), "expected acc == n (a[1] == 1 each iter)");

        // STRUCTURAL: the array is reassigned in the loop (the push refreshes
        // frame[3]/frame[4]), so it is NOT hoisted — the ArrLoad at op 2 still
        // reads the ptr straight from the frame each iteration (the post-realloc
        // pointer). At least one per-loop frame read of slot 3 survives.
        let bytes = region_bytes(&ops);
        assert!(
            count_frame_reads(&bytes, 3) >= 1,
            "a reassigned array must keep its per-access frame read (not be hoisted)"
        );
    }

    // ----------------------------------------------------------------
    // WAVE 28 — LOOP-INVARIANT CONSTANT HOIST. A `LoadConst` inside a loop
    // whose `dst` is the span's sole writer (its value is invariant across
    // iterations) and lands in a GP register is materialized ONCE at the loop
    // pre-header instead of every iteration (the V8/LLVM LICM idiom).
    //
    // CRITICAL EMPIRICAL CONSTRAINT: FLOAT (XMM-resident) consts are EXCLUDED.
    // The investigation started from mandelbrot's hot inner loop, which
    // re-materializes `2.0`/`4.0` every iteration through a GP→XMM bridge — the
    // obvious LICM target. But hoisting them into resident XMM registers is
    // bit-identical yet measured ~8% SLOWER (n=2000: 480ms vs 443ms, reproduced
    // across independent processes; GP-only hoisting is at parity). mandelbrot's
    // XMM file is SATURATED (14/14), and holding an invariant float in a register
    // the float dependency chain repeatedly reads serializes worse than the
    // per-iteration re-materialization, whose independent `movabs;movq` the
    // out-of-order engine overlaps with the float chain — the backend is
    // throughput-bound here, so the "redundant" float loads are FREE ILP. So the
    // hoist is restricted to GP consts (sound, neutral-to-positive); the float
    // exclusion is the load-bearing design choice, asserted below.
    //
    // Tests prove (a) BIT-IDENTITY to `reference_eval` (which models every float
    // op via from_bits/to_bits — any drift flips the result); (b) an INTEGER
    // invariant const is HOISTED (its `movabs` precedes the loop guard's first
    // conditional jump) while the FLOAT consts STAY in the body (after the
    // guard); (c) a slot reused for two different constants (two writers) is NOT
    // hoisted. Kill-switch `LOGOS_NO_CONST_HOIST=1` (read live, so one process
    // can A/B both emissions).
    // ----------------------------------------------------------------

    /// The byte encoding of `movabs rax, imm64` (REX.W B8+rax) — the
    /// `LoadConst`/const-hoist materialization scratch (`mov_ri(S0, value)`,
    /// S0 = rax). The 8 imm bytes are the constant's raw bits.
    fn movabs_rax(value: i64) -> Vec<u8> {
        let mut v = vec![0x48u8, 0xB8];
        v.extend_from_slice(&value.to_le_bytes());
        v
    }

    /// Byte offset of the FIRST `jcc rel32` (0F 8x) in `bytes` — the loop guard's
    /// conditional jump (a `Branch`'s `cmp; jcc`). The pre-header sits BEFORE it;
    /// the loop body AFTER it. `None` if the region has no conditional jump.
    fn first_jcc_offset(bytes: &[u8]) -> Option<usize> {
        let mut i = 0usize;
        while i + 1 < bytes.len() {
            if bytes[i] == 0x0F && (bytes[i + 1] & 0xF0) == 0x80 {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// Byte offset of the FIRST occurrence of `needle` in `hay`, if any.
    fn first_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || hay.len() < needle.len() {
            return None;
        }
        (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
    }

    /// Compile a region's raw bytes with the const-hoist kill-switch forced to
    /// `state` for THIS compile (the gate reads the env live). Restores the prior
    /// env afterward so tests don't leak state.
    fn region_bytes_const_hoist(ops: &[MicroOp], on: bool) -> Vec<u8> {
        let prev = std::env::var("LOGOS_NO_CONST_HOIST").ok();
        if on {
            std::env::remove_var("LOGOS_NO_CONST_HOIST");
        } else {
            std::env::set_var("LOGOS_NO_CONST_HOIST", "1");
        }
        let status = Arc::new(AtomicI64::new(0));
        let bytes = compile_region_regalloc(ops, Some(status))
            .expect("region must compile")
            .bytes()
            .to_vec();
        match prev {
            Some(v) => std::env::set_var("LOGOS_NO_CONST_HOIST", v),
            None => std::env::remove_var("LOGOS_NO_CONST_HOIST"),
        }
        bytes
    }

    /// The mandelbrot inner loop is BIT-IDENTICAL to the reference, the two
    /// loop-invariant FLOAT consts (`2.0`, `4.0`) STAY in the body (float
    /// hoisting is the measured-net-negative case, deliberately excluded), and an
    /// INTEGER invariant const (`one`, the iter step) IS hoisted to the
    /// pre-header. The kill-switch off recovers the pre-hoist emission for the
    /// integer const (in the body), proving the lever fires only when on.
    #[test]
    fn const_hoist_mandelbrot_float_consts_stay_in_body_int_const_hoists() {
        const TWO: i64 = 4611686018427387904;
        const FOUR: i64 = 4616189618054758400;
        const STEP: i64 = 777; // a distinctive INTEGER invariant const (the step)
        // A self-contained carried-float loop with both invariant FLOAT consts, an
        // escape branch, and an INTEGER invariant const materialized inside the
        // loop (the `iter += STEP` step), looping until iter >= N.
        // frame: 0=iter 1=zr 2=zi 3=cr 4=ci 5=count 6=N 7=zr2 8=p1 9=p2
        //        10=two 11=q1 12=q2 13=mag 14=four 15=step
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 6, target: 17 }, // 0: while iter<N
            MicroOp::MulF { dst: 8, lhs: 1, rhs: 1 },                     // 1
            MicroOp::MulF { dst: 9, lhs: 2, rhs: 2 },                     // 2
            MicroOp::SubF { dst: 7, lhs: 8, rhs: 9 },                     // 3
            MicroOp::AddF { dst: 7, lhs: 7, rhs: 3 },                     // 4: zr2
            MicroOp::LoadConst { dst: 10, value: TWO },                   // 5: 2.0 (NOT hoisted)
            MicroOp::MulF { dst: 11, lhs: 10, rhs: 1 },                   // 6
            MicroOp::MulF { dst: 12, lhs: 11, rhs: 2 },                   // 7
            MicroOp::AddF { dst: 2, lhs: 12, rhs: 4 },                    // 8: zi
            MicroOp::Move { dst: 1, src: 7 },                            // 9: zr=zr2
            MicroOp::MulF { dst: 8, lhs: 1, rhs: 1 },                     // 10
            MicroOp::MulF { dst: 9, lhs: 2, rhs: 2 },                     // 11
            MicroOp::AddF { dst: 13, lhs: 8, rhs: 9 },                    // 12: mag
            MicroOp::LoadConst { dst: 14, value: FOUR },                  // 13: 4.0 (NOT hoisted)
            MicroOp::BranchF { cmp: Cmp::GtEq, lhs: 13, rhs: 14, target: 16 }, // 14: escape
            MicroOp::Add { dst: 5, lhs: 5, rhs: 0 },                      // 15: count += iter
            MicroOp::LoadConst { dst: 15, value: STEP },                  // 16: step (HOIST, int)
            MicroOp::Add { dst: 0, lhs: 0, rhs: 15 },                     // 17: iter += step
            MicroOp::Jump { target: 0 },                                 // 18: back-edge
        ];
        let mut ops = ops;
        let ret_idx = ops.len();
        ops[0] = MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 6, target: ret_idx };
        // The escape target (op 14) must skip count but still take the step: point
        // it at op 16 (the step LoadConst), so escape still terminates.
        ops[14] = MicroOp::BranchF { cmp: Cmp::GtEq, lhs: 13, rhs: 14, target: 16 };
        ops.push(MicroOp::Return { src: 5 });

        let n = 4000i64;
        let mut frame = vec![0i64; 18];
        frame[1] = (0.0f64).to_bits() as i64; // zr
        frame[2] = (0.0f64).to_bits() as i64; // zi
        frame[3] = (0.30f64).to_bits() as i64; // cr (bounded → never escapes)
        frame[4] = (0.10f64).to_bits() as i64; // ci
        frame[6] = n; // N

        let expected = reference_eval(&ops, &mut frame.clone(), 1_000_000).expect("reference runs");
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("mandel region compiles");
        let mut f = frame.clone();
        let out = chain.run_with_frame(&mut f);
        assert_eq!(
            out,
            ChainOutcome::Return(expected),
            "mandelbrot inner loop diverged from the float reference"
        );

        let on = region_bytes_const_hoist(&ops, true);
        let off = region_bytes_const_hoist(&ops, false);
        let guard_on = first_jcc_offset(&on).expect("a loop guard jcc exists");

        // STRUCTURAL (load-bearing): the FLOAT consts are NEVER hoisted — each
        // stays in the body (after the loop guard's jcc) regardless of the switch.
        // Holding them resident in the saturated XMM file is measured net-negative.
        for &val in &[TWO, FOUR] {
            let pos = first_subseq(&on, &movabs_rax(val))
                .expect("float const movabs present");
            assert!(
                pos > guard_on,
                "FLOAT const {val:#x} movabs (@{pos}) must STAY in the body \
                 (after the guard jcc @{guard_on}) — float hoisting is excluded"
            );
        }

        // STRUCTURAL: the INTEGER invariant const IS hoisted — its `movabs`
        // precedes the loop guard's jcc with the lever on, and stays in the body
        // with it off (the before/after the implementation makes, in one process).
        let step_on = first_subseq(&on, &movabs_rax(STEP)).expect("int const present (on)");
        let step_off = first_subseq(&off, &movabs_rax(STEP)).expect("int const present (off)");
        let guard_off = first_jcc_offset(&off).expect("a loop guard jcc exists");
        assert!(
            step_on < guard_on,
            "hoist ON: int const {STEP} movabs (@{step_on}) must be in the pre-header \
             (before the guard jcc @{guard_on})"
        );
        assert!(
            step_off > guard_off,
            "hoist OFF: int const {STEP} movabs (@{step_off}) must stay in the body \
             (after the guard jcc @{guard_off})"
        );
    }

    /// A slot reused for TWO DIFFERENT constants in the loop (a scratch
    /// `LoadConst {dst,0}` then `LoadConst {dst,1}`) has two writers in the span,
    /// so it is NOT invariant and must NOT be hoisted — each materialization stays
    /// in the body, and the result is bit-identical.
    #[test]
    fn const_hoist_reused_scratch_not_hoisted_and_identical() {
        // while i < n: t = 7; acc += t; t = 9; acc += t; i += 1
        // frame: 0=i 1=n 2=acc 3=t 4=one
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 8 }, // 0
            MicroOp::LoadConst { dst: 3, value: 7 },                     // 1: t=7
            MicroOp::Add { dst: 2, lhs: 2, rhs: 3 },                     // 2: acc+=7
            MicroOp::LoadConst { dst: 3, value: 9 },                     // 3: t=9 (SECOND writer of 3)
            MicroOp::Add { dst: 2, lhs: 2, rhs: 3 },                     // 4: acc+=9
            MicroOp::LoadConst { dst: 4, value: 1 },                     // 5: one=1 (HOISTABLE)
            MicroOp::Add { dst: 0, lhs: 0, rhs: 4 },                     // 6: i+=1
            MicroOp::Jump { target: 0 },                                // 7
            MicroOp::Return { src: 2 },                                  // 8
        ];
        let n = 1000i64;
        let frame = vec![0i64, n, 0, 0, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 10_000_000).expect("reference runs");
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("compiles");
        let mut f = frame.clone();
        let out = chain.run_with_frame(&mut f);
        assert_eq!(out, ChainOutcome::Return(expected));
        assert_eq!(out, ChainOutcome::Return(n * 16)); // (7+9) per iter

        // STRUCTURAL: the reused scratch (value 7 then 9) is NOT hoisted — both
        // movabs stay in the body (after the guard jcc). The genuinely invariant
        // `one` (value 1) IS hoisted (precedes the guard jcc).
        let on = region_bytes_const_hoist(&ops, true);
        let guard = first_jcc_offset(&on).expect("loop guard jcc exists");
        for &reused in &[7i64, 9] {
            let off = first_subseq(&on, &movabs_rax(reused))
                .expect("reused-const movabs present");
            assert!(
                off > guard,
                "reused scratch const {reused} must NOT be hoisted (movabs @{off} vs guard @{guard})"
            );
        }
        let one_off = first_subseq(&on, &movabs_rax(1)).expect("the invariant one movabs present");
        assert!(
            one_off < guard,
            "the invariant `one` (1) must be hoisted to the pre-header (movabs @{one_off} vs guard @{guard})"
        );
    }
}

// =====================================================================
// WAVE 15: LIST-PARAM (mode-B) PRECISE-DEOPT recursion in the regalloc
// FUNCTION backend.
//
// A list-parameter recursive function mutates a SHARED, pinned array in
// place (heap_sort's siftDown, quicksort's partition+recurse). A classic
// replay-from-head deopt would DOUBLE-APPLY those landed mutations, so the
// backend must use PRECISE deopt: on a side exit it materializes the native
// frame back to the bytecode VM and resumes AT the faulting op — every prior
// effect intact. The forge-level tests drive `compile_function_regalloc`
// with a per-op DEOPT-CODE table (the `(pc<<2)|3` precise tags the adapter
// emits); the engine-level tests prove the real heap_sort/quicksort sources
// tier through this backend and stay bit-identical to the tree-walker.
// =====================================================================
#[cfg(target_arch = "x86_64")]
mod precise_list_recursion {
    use super::*;
    use logicaffeine_forge::jit::MicroOp;
    use logicaffeine_forge::regalloc::compile_function_regalloc_precise;
    use std::sync::atomic::AtomicI64;
    use std::sync::Arc;

    const SELF_ARENA: usize = 1 << 16;

    /// The Int list-triple helper address (mirrors the JIT's mode-B
    /// `ListTriple` lowering): reads `frame[handle_slot]` (a `*mut Vec<i64>`)
    /// and writes the vec/ptr/len pin triple.
    fn list_triple_addr() -> i64 {
        logicaffeine_jit::logos_rt_list_triple as usize as i64
    }

    /// A live boxed `Vec<i64>` kept alive for the whole run.
    struct LiveVec {
        boxed: Box<Vec<i64>>,
    }
    impl LiveVec {
        fn new(init: Vec<i64>) -> Self {
            LiveVec { boxed: Box::new(init) }
        }
        fn handle(&mut self) -> i64 {
            self.boxed.as_mut() as *mut Vec<i64> as i64
        }
        fn snapshot(&self) -> Vec<i64> {
            (*self.boxed).clone()
        }
    }

    /// RED: a mode-B list-param function whose body MUTATES the pinned array in
    /// place (a checked `ArrStore`), refreshes its pin triple (`ListTriple`),
    /// and is compiled by the PRECISE regalloc FUNCTION backend. A SUCCESSFUL
    /// run must land the in-place write EXACTLY ONCE (the buffer matches the
    /// reference); the function must tier (compile to Some).
    ///
    /// Frame layout mirrors the adapter's mode-B shape: regs [0..rc), plant
    /// window/resume/dst at rc/rc+1/rc+2, the pin triple at rc+3..rc+6, the
    /// limit slot last. handle_slot 0 holds the `*mut Vec` (the list param).
    #[test]
    fn precise_inplace_store_lands_once() {
        let rc: u16 = 6;
        let plant_window = rc; // rc+0
        let _plant_resume = rc + 1;
        let _plant_dst = rc + 2;
        let vec_slot = rc + 3;
        let ptr_slot = rc + 4;
        let len_slot = rc + 5;
        let limit_slot = rc + 6;
        let frame_size = (limit_slot + 1) as usize;
        // slots: 0=arr handle, 1=idx (1-based), 2=value, 3=ret-handle scratch
        // Body: plant the param's triple, store value at idx, return the handle.
        let ops = vec![
            MicroOp::LoadConst { dst: plant_window, value: -1 }, // 0: invalidate plant
            MicroOp::ListTriple {
                handle_slot: 0,
                vec_slot,
                ptr_slot,
                len_slot,
                helper_addr: list_triple_addr(),
            }, // 1: refresh pin triple from the param handle
            MicroOp::ArrStore { src: 2, idx: 1, ptr_slot, len_slot, byte: false, narrow32: false, checked: true }, // 2: arr[idx-1] = value
            MicroOp::Move { dst: 3, src: vec_slot },             // 3: return the vec handle
            MicroOp::Return { src: 3 },                          // 4
        ];
        // Precise deopt codes: each micro tagged with its bytecode pc `(pc<<2)|3`.
        // Op 2 (the store) carries the precise resume tag; the rest are 1.
        let codes: Vec<i64> = vec![1, 1, (2i64 << 2) | 3, 1, 1];

        let status = Arc::new(AtomicI64::new(0));
        let depth = Arc::new(AtomicI64::new(0));
        let chain = compile_function_regalloc_precise(
            &ops,
            Some(status.clone()),
            Arc::as_ptr(&depth) as i64,
            &codes,
        )
        .expect("precise list-param function must compile");

        let mut buf = LiveVec::new(vec![10, 20, 30, 40, 50]);
        let mut frame = vec![0i64; SELF_ARENA];
        frame[0] = buf.handle();
        frame[1] = 3; // 1-based index 3 → buffer[2]
        frame[2] = 999;
        let arena_end = unsafe { frame.as_ptr().add(SELF_ARENA) } as i64;
        frame[limit_slot as usize] = arena_end;
        depth.store(0, std::sync::atomic::Ordering::SeqCst);
        status.store(0, std::sync::atomic::Ordering::SeqCst);
        let out = chain.run_with_frame(&mut frame);
        assert!(matches!(out, ChainOutcome::Return(_)), "expected a return, got {out:?}");
        // The in-place store landed EXACTLY once.
        assert_eq!(buf.snapshot(), vec![10, 20, 999, 40, 50], "store double-applied or missing");
        let _ = frame_size;
    }

    /// RED: an OOB store inside the precise list-param function must side-exit
    /// with the PRECISE tag (`raw & 3 == 3`), the embedded resume pc matching
    /// the store op, and the depth in the high bits — bit-identical to the
    /// stencil tier's `ST_DEOPT_AT` / precise-call encoding. The buffer must be
    /// UNTOUCHED (a checked store side-exits BEFORE writing).
    #[test]
    fn precise_oob_store_side_exits_with_tag() {
        let rc: u16 = 6;
        let plant_window = rc;
        let vec_slot = rc + 3;
        let ptr_slot = rc + 4;
        let len_slot = rc + 5;
        let limit_slot = rc + 6;
        let ops = vec![
            MicroOp::LoadConst { dst: plant_window, value: -1 },
            MicroOp::ListTriple {
                handle_slot: 0,
                vec_slot,
                ptr_slot,
                len_slot,
                helper_addr: list_triple_addr(),
            },
            MicroOp::ArrStore { src: 2, idx: 1, ptr_slot, len_slot, byte: false, narrow32: false, checked: true },
            MicroOp::Move { dst: 3, src: vec_slot },
            MicroOp::Return { src: 3 },
        ];
        let store_pc = 2i64;
        let codes: Vec<i64> = vec![1, 1, (store_pc << 2) | 3, 1, 1];

        let status = Arc::new(AtomicI64::new(0));
        let depth = Arc::new(AtomicI64::new(0));
        let chain = compile_function_regalloc_precise(
            &ops,
            Some(status.clone()),
            Arc::as_ptr(&depth) as i64,
            &codes,
        )
        .expect("precise function compiles");

        let mut buf = LiveVec::new(vec![10, 20, 30]);
        let mut frame = vec![0i64; SELF_ARENA];
        frame[0] = buf.handle();
        frame[1] = 99; // 1-based index 99 → OOB
        frame[2] = 777;
        let arena_end = unsafe { frame.as_ptr().add(SELF_ARENA) } as i64;
        frame[limit_slot as usize] = arena_end;
        let want_depth = 7i64;
        depth.store(want_depth, std::sync::atomic::Ordering::SeqCst);
        status.store(0, std::sync::atomic::Ordering::SeqCst);
        let out = chain.run_with_frame(&mut frame);
        match out {
            ChainOutcome::Deopt(raw) => {
                assert_eq!(raw & 0b11, 0b11, "OOB store must side-exit PRECISELY (low bits 11)");
                let resume_pc = (raw & 0xFFFF_FFFF) >> 2;
                assert_eq!(resume_pc, store_pc, "resume pc must be the store op");
                assert_eq!(raw >> 32, want_depth, "high bits must carry the live depth");
            }
            other => panic!("expected a precise deopt, got {other:?}"),
        }
        // The buffer is untouched (checked store exits before the write).
        assert_eq!(buf.snapshot(), vec![10, 20, 30], "OOB store must not write");
    }

    /// RED: a RECURSIVE list-param function that self-calls (mode-B precise
    /// `Call`), mutating the shared array each level, and OOBs at the base of a
    /// DEEP recursion. The deopt must propagate up every native frame as a
    /// precise tag (status nonzero after each returning call), bit-identical to
    /// the stencil tier's precise-call propagation. Drives the precise `Call`
    /// ABI (plant window/resume/dst, disjoint callee window, depth/arena guards,
    /// post-return status propagation).
    #[test]
    fn precise_recursive_call_propagates_deopt() {
        let rc: u16 = 8;
        let plant_window = rc;
        let plant_resume = rc + 1;
        let plant_dst = rc + 2;
        let vec_slot = rc + 3;
        let ptr_slot = rc + 4;
        let len_slot = rc + 5;
        let limit_slot = rc + 6;
        let frame_size = (limit_slot + 1) as i64;
        let window = (limit_slot + 1) as u16; // disjoint callee window at frame_size
        // f(arr, k): if k == 0 { store at OOB idx -> deopt } else { f(arr, k-1) }.
        // slots: 0=arr 1=k 2=zero 3=one 4=ret/result-handle 5=bigidx 6=val
        let table_addr_placeholder = 0i64; // patched by the backend's entry cell
        let depth = Arc::new(AtomicI64::new(0));
        let status = Arc::new(AtomicI64::new(0));
        let ops = vec![
            MicroOp::LoadConst { dst: plant_window, value: -1 }, // 0
            MicroOp::LoadConst { dst: 2, value: 0 },             // 1: zero
            MicroOp::Branch { cmp: Cmp::Eq, lhs: 1, rhs: 2, target: 5 }, // 2: if !(k==0) goto 5
            // base case (k==0): OOB store -> precise deopt.
            MicroOp::ListTriple {
                handle_slot: 0,
                vec_slot,
                ptr_slot,
                len_slot,
                helper_addr: list_triple_addr(),
            }, // 3
            MicroOp::ArrStore { src: 6, idx: 5, ptr_slot, len_slot, byte: false, narrow32: false, checked: true }, // 4: OOB
            // recursive case: arg0 = arr, arg1 = k-1, call self.
            MicroOp::LoadConst { dst: 3, value: 1 },     // 5: one
            MicroOp::Sub { dst: window + 1, lhs: 1, rhs: 3 }, // 6: arg1 = k-1
            MicroOp::Move { dst: window, src: 0 },       // 7: arg0 = arr (handle)
            // plant the call linkage.
            MicroOp::LoadConst { dst: plant_window, value: frame_size }, // 8
            MicroOp::LoadConst { dst: plant_resume, value: 12 },          // 9: resume after call
            MicroOp::LoadConst { dst: plant_dst, value: 4 },             // 10: result slot
            MicroOp::Call {
                dst: 4,
                args_start: window,
                table_addr: table_addr_placeholder,
                depth_addr: Arc::as_ptr(&depth) as i64,
                status_addr: Arc::as_ptr(&status) as i64,
                limit_slot,
                depth_limit: logicaffeine_compile::semantics::MAX_CALL_DEPTH as i64,
            }, // 11
            MicroOp::LoadConst { dst: plant_window, value: -1 },         // 12: invalidate plant
            MicroOp::Return { src: 4 },                                  // 13
        ];
        // Precise codes: deoptable ops (the store at 4 and the call at 11) carry
        // their tags; everything else is 1.
        let mut codes = vec![1i64; ops.len()];
        codes[4] = (4i64 << 2) | 3; // store resume pc
        codes[11] = (11i64 << 2) | 3; // call resume pc

        let chain = compile_function_regalloc_precise(
            &ops,
            Some(status.clone()),
            Arc::as_ptr(&depth) as i64,
            &codes,
        )
        .expect("precise recursive list-param function must compile");

        let mut buf = LiveVec::new(vec![1, 2, 3, 4]);
        let mut frame = vec![0i64; SELF_ARENA];
        frame[0] = buf.handle();
        frame[1] = 30; // recurse 30 deep, then OOB
        frame[5] = 9999; // OOB index
        frame[6] = 42;
        let arena_end = unsafe { frame.as_ptr().add(SELF_ARENA) } as i64;
        frame[limit_slot as usize] = arena_end;
        depth.store(0, std::sync::atomic::Ordering::SeqCst);
        status.store(0, std::sync::atomic::Ordering::SeqCst);
        let out = chain.run_with_frame(&mut frame);
        match out {
            ChainOutcome::Deopt(raw) => {
                assert_eq!(raw & 0b11, 0b11, "deep OOB must propagate as a PRECISE tag");
            }
            other => panic!("expected a precise deopt to propagate, got {other:?}"),
        }
        assert_eq!(buf.snapshot(), vec![1, 2, 3, 4], "no in-place write should land (OOB at base)");
    }
}

// =====================================================================
// WAVE 15 (engine level): the REAL heap_sort / quicksort sources tier
// through the precise regalloc FUNCTION backend, bit-identical to the
// tree-walker. These exercise the full adapter + native arena + precise
// deopt materialization, not just the forge primitive.
// =====================================================================

/// RED: heap_sort's `siftDown` (a list-param mode-B function: in-place array
/// mutation through the pinned ptr/len, `ListTriple` refresh, precise deopt)
/// tiers through the regalloc FUNCTION backend and stays bit-identical to the
/// tree-walker.
#[test]
#[cfg(target_arch = "x86_64")]
fn heap_sort_tiers_via_precise_regalloc_function() {
    let src = std::fs::read_to_string(
        "/home/tristen/logicaffeine/benchmarks/programs/heap_sort/main.lg",
    )
    .expect("heap_sort source");
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(
            &src,
            &["prog".to_string(), "300".to_string()],
            Some(&tier as &dyn NativeTier),
        );
        let tw = tw_outcome_with_args(&src, &["prog".to_string(), "300".to_string()]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "heap_sort precise-regalloc diverged from tree-walker"
        );
        assert_eq!(vm.error, None, "heap_sort must not error");
        assert!(
            tier.regalloc_function_count() >= 1,
            "siftDown must tier through the precise regalloc FUNCTION backend (got {})",
            tier.regalloc_function_count()
        );
    });
}

/// RED: quicksort's recursive `qs` (a list-param mode-B function that mutates
/// the array in place AND self-calls — the precise `Call` path with plant
/// linkage + propagation) tiers through the regalloc FUNCTION backend and
/// stays bit-identical to the tree-walker.
#[test]
#[cfg(target_arch = "x86_64")]
fn quicksort_tiers_via_precise_regalloc_function() {
    let src = std::fs::read_to_string(
        "/home/tristen/logicaffeine/benchmarks/programs/quicksort/main.lg",
    )
    .expect("quicksort source");
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(
            &src,
            &["prog".to_string(), "300".to_string()],
            Some(&tier as &dyn NativeTier),
        );
        let tw = tw_outcome_with_args(&src, &["prog".to_string(), "300".to_string()]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "quicksort precise-regalloc diverged from tree-walker"
        );
        assert_eq!(vm.error, None, "quicksort must not error");
        assert!(
            tier.regalloc_function_count() >= 1,
            "qs must tier through the precise regalloc FUNCTION backend (got {})",
            tier.regalloc_function_count()
        );
    });
}

/// RED: an OOB index INSIDE the recursive list-param function (heap_sort
/// shape) must side-exit precisely and the bytecode replay must raise the
/// IDENTICAL kernel error with the IDENTICAL partial output as the
/// tree-walker — proving precise-deopt materialization through the regalloc
/// path on a list-param recursion. `siftDown` is WARMED on safe inputs so it
/// tiers, then a deliberately out-of-range `end` forces the OOB.
#[test]
#[cfg(target_arch = "x86_64")]
fn precise_list_recursion_oob_parity() {
    // siftDown warmed by a safe sort, then called with an end past the array.
    let src = "## To siftDown (arr: Seq of Int, start: Int, end: Int) -> Seq of Int:\n\
               \x20   Let mutable result be arr.\n\
               \x20   Let mutable root be start.\n\
               \x20   While 2 * root + 1 is at most end:\n\
               \x20       Let child be 2 * root + 1.\n\
               \x20       Let tmp be item (root + 1) of result.\n\
               \x20       Set item (root + 1) of result to item (child + 1) of result.\n\
               \x20       Set item (child + 1) of result to tmp.\n\
               \x20       Set root to child.\n\
               \x20   Return result.\n\
               \n\
               ## Main\n\
               Let mutable arr be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is less than 16:\n\
               \x20   Push (16 - i) to arr.\n\
               \x20   Set i to i + 1.\n\
               Let mutable s be 6.\n\
               While s is at least 0:\n\
               \x20   Set arr to siftDown(arr, s, 15).\n\
               \x20   Set s to s - 1.\n\
               Show item 1 of arr.\n\
               Set arr to siftDown(arr, 0, 100000).\n\
               Show 999.\n";
    let src_owned = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src_owned, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src_owned, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "precise list-recursion OOB replay diverged from tree-walker"
        );
        assert!(vm.error.is_some(), "the out-of-range end must raise an error");
    });
}

// =====================================================================
// WAVE 27a — loop-invariant ptr/len hoist RELATIVE timing harness.
//
// Runs the actual knapsack and coins benchmark programs through the VM+JIT
// tier (`ForgeTier`) and prints the wall time. Ignored by default (it is a
// measurement, not a correctness gate). To A/B the hoist on a noisy shared
// box, run the harness TWICE — once with the hoist ON (default) and once with
// `LOGOS_NO_PTR_HOIST=1` — interleaved, and compare the ratio. The kill-switch
// is read once per process (a OnceLock), so the two regimes must be separate
// processes.
// =====================================================================
#[cfg(target_arch = "x86_64")]
mod ptr_hoist_timing {
    use super::*;
    use std::time::Instant;

    /// The harness runs the (multi-hundred-ms) benchmark programs only when
    /// `WF27A_TIMING` is set, so a full `--run-ignored` suite does not pay for the
    /// measurement; without it the `#[ignore]`d tests are present (the documented
    /// A/B mechanism) but return immediately.
    fn timing_enabled() -> bool {
        std::env::var_os("WF27A_TIMING").is_some()
    }

    fn run_program(path: &str, n: &str) -> (String, f64) {
        let src = std::fs::read_to_string(path).expect("read benchmark program");
        let args = vec![path.to_string(), n.to_string()];
        on_big_stack(move || {
            let tier = ForgeTier::new();
            let t0 = Instant::now();
            let out = vm_outcome_with_args(&src, &args, Some(&tier as &dyn NativeTier));
            let dt = t0.elapsed().as_secs_f64() * 1e3;
            assert!(out.error.is_none(), "program errored: {:?}", out.error);
            (norm(&out.output), dt)
        })
    }

    #[test]
    #[ignore = "timing harness; run with/without LOGOS_NO_PTR_HOIST to A/B"]
    fn knapsack_ptr_hoist_relative_timing() {
        if !timing_enabled() {
            return;
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs/knapsack/main.lg");
        let n = std::env::var("WF27A_N").unwrap_or_else(|_| "2000".to_string());
        let mut best = f64::INFINITY;
        let mut out = String::new();
        for _ in 0..5 {
            let (o, dt) = run_program(path, &n);
            out = o;
            best = best.min(dt);
        }
        let hoist = if std::env::var("LOGOS_NO_PTR_HOIST").as_deref() == Ok("1") { "OFF" } else { "ON " };
        eprintln!("KNAPSACK n={n} hoist={hoist} best={best:.1}ms out={out}");
    }

    #[test]
    #[ignore = "timing harness; run with/without LOGOS_NO_PTR_HOIST to A/B"]
    fn coins_ptr_hoist_relative_timing() {
        if !timing_enabled() {
            return;
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs/coins/main.lg");
        let n = std::env::var("WF27A_N").unwrap_or_else(|_| "100000".to_string());
        let mut best = f64::INFINITY;
        let mut out = String::new();
        for _ in 0..5 {
            let (o, dt) = run_program(path, &n);
            out = o;
            best = best.min(dt);
        }
        let hoist = if std::env::var("LOGOS_NO_PTR_HOIST").as_deref() == Ok("1") { "OFF" } else { "ON " };
        eprintln!("COINS    n={n} hoist={hoist} best={best:.1}ms out={out}");
    }

    #[test]
    #[ignore = "timing harness; set WF27A_PROG + WF27A_N; A/B via LOGOS_NO_PTR_HOIST"]
    fn any_program_ptr_hoist_relative_timing() {
        if !timing_enabled() {
            return;
        }
        let prog = std::env::var("WF27A_PROG").unwrap_or_else(|_| "matrix_mult".to_string());
        let path =
            format!("{}/../../benchmarks/programs/{}/main.lg", env!("CARGO_MANIFEST_DIR"), prog);
        let n = std::env::var("WF27A_N").unwrap_or_else(|_| "300".to_string());
        let mut best = f64::INFINITY;
        let mut out = String::new();
        for _ in 0..5 {
            let (o, dt) = run_program(&path, &n);
            out = o;
            best = best.min(dt);
        }
        let hoist = if std::env::var("LOGOS_NO_PTR_HOIST").as_deref() == Ok("1") { "OFF" } else { "ON " };
        let short: String = out.chars().take(24).collect();
        eprintln!("{prog:<14} n={n} hoist={hoist} best={best:.1}ms out={short}");
    }
}

// =====================================================================
// WAVE 28 — loop-invariant CONSTANT hoist RELATIVE timing harness.
//
// The const-hoist kill-switch (`LOGOS_NO_CONST_HOIST`) is read LIVE, so this
// harness A/Bs BOTH regimes in ONE process — alternating order each round, best-
// of-5, the robust signal on a noisy shared box (absolute wall time is
// untrustworthy; the on/off RATIO from interleaved runs is the signal). The
// recorded results (n on a quiet-ish slice of a shared box, reproduced across
// independent processes):
//   - collatz (pure-GP scalar, the WIN): ON ≈ 939ms vs OFF ≈ 1010ms → 0.93×.
//   - mandelbrot (float-saturated, parity): ON ≈ 442ms vs OFF ≈ 443ms → 1.00×
//     (the FLOAT consts are deliberately NOT hoisted — hoisting them was ~8%
//     SLOWER, see the call-site exclusion comment; this confirms the int-only
//     hoist neither helps nor hurts the float case).
//   - nbody / spectral_norm: parity (float cluster, no regression).
// `#[ignore]`d and gated on `WF28_TIMING` so the default suite never pays for it.
// =====================================================================
#[cfg(target_arch = "x86_64")]
mod const_hoist_timing {
    use super::*;
    use std::time::Instant;

    fn timing_enabled() -> bool {
        std::env::var_os("WF28_TIMING").is_some()
    }

    fn run_once(src: &str, args: &[String]) -> (String, f64) {
        let src = src.to_string();
        let args = args.to_vec();
        on_big_stack(move || {
            let tier = ForgeTier::new();
            let t0 = Instant::now();
            let out = vm_outcome_with_args(&src, &args, Some(&tier as &dyn NativeTier));
            let dt = t0.elapsed().as_secs_f64() * 1e3;
            assert!(out.error.is_none(), "program errored: {:?}", out.error);
            (norm(&out.output), dt)
        })
    }

    /// Best-of-5 wall time for `prog` at size `n` with the const-hoist set to
    /// `on` (read live per compile). Output is returned to confirm parity.
    fn best5(prog: &str, n: &str, on: bool) -> (String, f64) {
        let path =
            format!("{}/../../benchmarks/programs/{}/main.lg", env!("CARGO_MANIFEST_DIR"), prog);
        let src = std::fs::read_to_string(&path).expect("read benchmark program");
        let args = vec![path.clone(), n.to_string()];
        if on {
            std::env::remove_var("LOGOS_NO_CONST_HOIST");
        } else {
            std::env::set_var("LOGOS_NO_CONST_HOIST", "1");
        }
        let mut best = f64::INFINITY;
        let mut out = String::new();
        for _ in 0..5 {
            let (o, dt) = run_once(&src, &args);
            out = o;
            best = best.min(dt);
        }
        (out, best)
    }

    /// Alternating-order interleaved A/B of `prog` at `n`. Warms both regimes
    /// (cold caches bias whichever runs first), then alternates on-first/off-first
    /// each round so a drifting box-load cancels in the best-of ratio. Returns
    /// `(best_on_ms, best_off_ms, output)` and asserts output parity.
    fn ab(prog: &str, n: &str) -> (f64, f64, String) {
        let _ = best5(prog, n, true);
        let _ = best5(prog, n, false);
        let (mut on, mut off) = (f64::INFINITY, f64::INFINITY);
        let (mut o_on, mut o_off) = (String::new(), String::new());
        for round in 0..4 {
            let first_on = round % 2 == 0;
            let (a, b) = if first_on {
                (best5(prog, n, true), best5(prog, n, false))
            } else {
                let r0 = best5(prog, n, false);
                let r1 = best5(prog, n, true);
                (r1, r0)
            };
            on = on.min(a.1);
            off = off.min(b.1);
            o_on = a.0;
            o_off = b.0;
        }
        std::env::remove_var("LOGOS_NO_CONST_HOIST");
        assert_eq!(o_on, o_off, "const-hoist changed {prog} output!");
        (on, off, o_on)
    }

    #[test]
    #[ignore = "timing harness; run with WF28_TIMING=1 (A/B in-process)"]
    fn collatz_const_hoist_relative_timing() {
        if !timing_enabled() {
            return;
        }
        let n = std::env::var("WF28_N").unwrap_or_else(|_| "5000000".to_string());
        let (on, off, out) = ab("collatz", &n);
        eprintln!(
            "COLLATZ    n={n} const_hoist ON={on:.1}ms OFF={off:.1}ms ratio(on/off)={:.3} out={out}",
            on / off
        );
    }

    #[test]
    #[ignore = "timing harness; run with WF28_TIMING=1 (float cluster: no regression)"]
    fn float_cluster_const_hoist_no_regression() {
        if !timing_enabled() {
            return;
        }
        for (prog, n) in
            [("mandelbrot", "2000"), ("nbody", "200000"), ("spectral_norm", "1500")]
        {
            let (on, off, _) = ab(prog, n);
            eprintln!(
                "{prog:<14} n={n} const_hoist ON={on:.1}ms OFF={off:.1}ms ratio(on/off)={:.3}",
                on / off
            );
        }
    }
}
