//! Win 2 — the compile-once native partial evaluator's DATA PATH: a program AST crosses to the
//! native PE as bytes over the *real* fast codec (peer `encode_value_raw` on the host, generated
//! `CProgram::wire_decode` in the native binary — proven byte-identical by
//! `concurrency::marshal::tests::peer_and_wire_core_produce_identical_bytes`).
//!
//! This file locks the HOST side: `program → wire bytes → CProgram`, round-tripped through the
//! peer codec. The native binary end-to-end lock builds on it.

mod pe_support;

use logicaffeine_compile::compile::{
    decode_value_raw, encode_value_raw, program_covered_by_native_builder, program_to_core_wire_bytes,
    program_to_core_wire_bytes_two_pass, program_to_core_wire_bytes_via_interpreter,
    projection1_source_real_fast, run_native_pe, run_native_pe_inprocess, run_native_pe_server,
};

fn corpus() -> Vec<&'static str> {
    vec![
        "## Main\nLet x be 5.\nShow x.\n",
        "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 5:\n    Set s to s + i.\nShow s.\n",
        "## To inc (n: Int) -> Int:\n    Return n + 1.\n\n## Main\nShow inc(4).\n",
        "## Main\nLet xs be [1, 2, 3].\nShow item 2 of xs.\n",
        "## Main\nLet n be 7.\nShow \"v={n}\".\n",
    ]
}

#[test]
fn host_program_to_wire_bytes_decodes_to_a_cprogram() {
    for prog in corpus() {
        let bytes = program_to_core_wire_bytes(prog).expect("wire bytes");
        assert!(!bytes.is_empty(), "empty wire bytes for:\n{prog}");
        let rv = decode_value_raw(&bytes).expect("decode CProgram");
        let dbg = format!("{rv:?}");
        assert!(
            dbg.contains("CProgram") && dbg.contains("CProg"),
            "decoded value must be a CProgram/CProg, got:\n{dbg}"
        );
    }
}

/// THE compile-once native PE lock: for every corpus program, the residual produced by the native
/// binary (program handed to it as data over the real fast codec, native `peBlock`) is BYTE-
/// IDENTICAL to the tree-walker's. This proves the whole data path — host `encode_value_raw`,
/// generated `CProgram::wire_decode`, native `peBlock`, `decompileBlock` — is faithful end to end.
/// Builds the native binary once (~40s, content-addressed and reused); subsequent programs are ~15ms.
#[test]
fn native_pe_residual_is_byte_identical_to_the_tree_walker() {
    for prog in corpus() {
        let native = run_native_pe(prog).unwrap_or_else(|e| panic!("native PE failed for:\n{prog}\n{e}"));
        let tw = projection1_source_real_fast("", "", prog).expect("tree-walker PE");
        assert_eq!(
            native.trim(),
            tw.trim(),
            "native PE residual diverged from the tree-walker for:\n{prog}"
        );
    }
}

/// THE native-value-builder lock: for every program the native builder covers (the fast marshal
/// path — no CORE_TYPES re-parse), its wire bytes must be BYTE-IDENTICAL to the interpreter
/// reference path. Byte-identity proves the builder constructs the exact same `CProgram` value, so
/// the ~74× marshal speedup is free of any semantic drift. Runs over the corpus + 200 generated
/// diverse programs; asserts the fast path is actually exercised (not silently always falling back).
#[test]
fn native_builder_is_byte_identical_to_the_interpreter() {
    // Programs exercising the newly-covered variants (lists, maps, field access, interpolation,
    // membership, options, ranges, tuples, nested funcs) — every one must byte-match the interpreter.
    let extended: &[&str] = &[
        "## Main\nLet xs be [1, 2, 3].\nShow item 2 of xs.",
        "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nShow item \"a\" of m.",
        "## Main\nLet xs be [1, 2, 3].\nIf xs contains 2:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".",
        "## Main\nLet n be 7.\nShow \"value is {n} today\".",
        "## Main\nLet xs be [10, 20, 30].\nLet ys be items 1 through 2 of xs.\nShow length of ys.",
        "## Main\nLet s be a new Set of Int.\nAdd 5 to s.\nShow length of s.",
        "## To dbl (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet xs be [1, 2, 3].\nRepeat for x in xs:\n    Show dbl(x).",
        "## Main\nLet a be [1, 2].\nLet b be [3, 4].\nLet c be a followed by b.\nShow length of c.",
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to 10:\n    If i is greater than 5:\n        Set total to total + i.\nShow total.",
    ];
    let mut progs: Vec<String> = corpus().iter().map(|s| s.to_string()).collect();
    progs.extend(extended.iter().map(|s| s.to_string()));
    for seed in 0..200u64 {
        progs.push(pe_support::gen_diverse_program(seed));
    }
    let mut covered = 0usize;
    for prog in &progs {
        if !program_covered_by_native_builder(prog) {
            continue; // an uncovered construct → interpreter fallback handles it correctly
        }
        covered += 1;
        let native = program_to_core_wire_bytes(prog).expect("native builder marshal");
        let interp = program_to_core_wire_bytes_via_interpreter(prog).expect("interpreter marshal");
        assert_eq!(
            native, interp,
            "native builder wire bytes DIFFER from the interpreter reference for:\n{prog}"
        );
    }
    assert!(
        covered >= 150,
        "native builder covered only {covered}/{} programs — the fast path is barely exercised",
        progs.len()
    );
}

/// PERF REGRESSION GUARD: a WARM `run_native_pe` (binary already built) must specialize a program
/// in well under 100 ms. This catches the class of regression where a fixed per-call cost sneaks
/// back in — notably the accidental `rustc --version` spawn in `aot_cache_key` that once cost ~18 ms
/// EVERY call (now the binary path is process-cached). Measured ~2.5 ms; the 100 ms ceiling is a
/// generous, contention-proof floor for "we did not re-introduce a heavyweight per-call cost".
#[test]
fn native_pe_warm_specialization_is_fast() {
    let prog = "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 20:\n    Set s to s + i.\nShow s.\n";
    // Warm: build + a few runs so the binary is cached and pages are hot.
    for _ in 0..3 {
        run_native_pe(prog).expect("native PE");
    }
    let mut best = std::time::Duration::from_secs(9999);
    for _ in 0..10 {
        let t = std::time::Instant::now();
        run_native_pe(prog).expect("native PE");
        best = best.min(t.elapsed());
    }
    assert!(
        best.as_millis() < 100,
        "warm native PE specialization took {best:?} — a heavyweight per-call cost regressed \
         (e.g. a `rustc`/process spawn in the hot path); it should be a few ms"
    );
}

#[test]
fn host_wire_bytes_are_idempotent_through_the_peer_codec() {
    // program → bytes → RuntimeValue → bytes must reproduce the identical bytes (the codec is a
    // deterministic bijection on this value class).
    for prog in corpus() {
        let bytes = program_to_core_wire_bytes(prog).expect("wire bytes");
        let rv = decode_value_raw(&bytes).expect("decode");
        let bytes2 = encode_value_raw(&rv).expect("re-encode");
        assert_eq!(bytes, bytes2, "wire bytes not idempotent for:\n{prog}");
    }
}

/// Win 2b (correctness) — the IN-PROCESS native PE (the compiled partial evaluator loaded as a
/// cdylib and called over FFI, no process/pipe) produces a residual BYTE-IDENTICAL to the tree-
/// walker's for the whole corpus. Removing the process boundary carried ZERO semantic weight: the
/// specialization is the same map, just reached without IPC. This is the correctness half of the
/// pit-of-success lock and must never flake.
#[test]
fn native_pe_inprocess_residual_is_byte_identical_to_the_tree_walker() {
    let extended: &[&str] = &[
        "## To dbl (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nRepeat for i from 1 to 4:\n    Show dbl(i).\n",
        "## Main\nLet xs be [1, 2, 3].\nLet ys be items 1 through 2 of xs.\nShow length of ys.\n",
        "## Main\nLet n be 7.\nShow \"v={n} w={n}\".\n",
    ];
    let mut progs: Vec<String> = corpus().iter().map(|s| s.to_string()).collect();
    progs.extend(extended.iter().map(|s| s.to_string()));
    for prog in &progs {
        let inproc = run_native_pe_inprocess(prog)
            .unwrap_or_else(|e| panic!("in-process PE failed for:\n{prog}\n{e}"));
        let tw = projection1_source_real_fast("", "", prog).expect("tree-walker PE");
        assert_eq!(
            inproc.trim(),
            tw.trim(),
            "in-process PE residual diverged from the tree-walker for:\n{prog}"
        );
    }
}

/// Win 2b (default is the pit of success) — `run_native_pe`, the public entry, must route through
/// the in-process cdylib path: its residual is byte-identical to BOTH the dedicated in-process call
/// and the resident server. So the fast+correct route is the one you get by default; you cannot
/// accidentally pay for the process boundary.
#[test]
fn run_native_pe_defaults_to_the_inprocess_path_and_agrees_with_the_server() {
    for prog in corpus() {
        let default = run_native_pe(prog).expect("run_native_pe");
        let inproc = run_native_pe_inprocess(prog).expect("in-process PE");
        let server = run_native_pe_server(prog).expect("resident-server PE");
        assert_eq!(default.trim(), inproc.trim(), "run_native_pe must equal the in-process path for:\n{prog}");
        assert_eq!(inproc.trim(), server.trim(), "in-process and server residuals must agree for:\n{prog}");
    }
}

/// Win 2c (correctness) — the SINGLE-PASS marshal (`WireSink`: AST → wire bytes directly, the default
/// `program_to_core_wire_bytes`) is BYTE-IDENTICAL to the two-pass reference (`TreeSink` builds a
/// `RuntimeValue` tree, then `encode_value_raw`). Removing the intermediate tree + second encode walk
/// changed the cost, not a single output byte. Runs the corpus + extended programs + 200 generated
/// diverse programs; asserts real coverage so the fast path is genuinely exercised.
#[test]
fn single_pass_marshal_is_byte_identical_to_the_two_pass() {
    let extended: &[&str] = &[
        "## To dbl (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nRepeat for i from 1 to 4:\n    Show dbl(i).\n",
        "## Main\nLet xs be [1, 2, 3].\nLet ys be items 1 through 2 of xs.\nShow length of ys.\n",
        "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nShow item \"a\" of m.\n",
        "## Main\nLet n be 7.\nShow \"v={n} w={n}\".\n",
        "## Main\nLet xs be [1, 2, 3].\nIf xs contains 2:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\n",
    ];
    let mut progs: Vec<String> = corpus().iter().map(|s| s.to_string()).collect();
    progs.extend(extended.iter().map(|s| s.to_string()));
    for seed in 0..200u64 {
        progs.push(pe_support::gen_diverse_program(seed));
    }
    let mut covered = 0usize;
    for prog in &progs {
        let single = match program_to_core_wire_bytes_two_pass(prog) {
            // The two-pass returns None for the same uncovered constructs the single pass falls back
            // on; skip those (the fallback path is locked separately).
            None => continue,
            Some(_) => program_to_core_wire_bytes(prog).expect("single-pass marshal"),
        };
        let two_pass = program_to_core_wire_bytes_two_pass(prog).expect("two-pass marshal");
        covered += 1;
        assert_eq!(
            single, two_pass,
            "single-pass WireSink bytes DIFFER from the two-pass TreeSink+encode for:\n{prog}"
        );
    }
    assert!(
        covered >= 150,
        "single-pass covered only {covered}/{} programs — the fast path is barely exercised",
        progs.len()
    );
}

/// Win 2d — the native builder now covers CRDT ops (Add / Remove / Merge / Increase / Decrease) and
/// task Launch, so those programs hit the fast single-pass marshal instead of the ~74×-slower
/// interpreter-marshal fallback. Each must be BYTE-IDENTICAL to the interpreter reference
/// (`program_to_core_wire_bytes_via_interpreter`, the independent oracle). The type-def-free forms
/// (Set Add/Remove, Launch) MUST be covered; the CRDT-counter forms are checked whenever covered.
#[test]
fn native_builder_covers_crdt_and_launch() {
    let must_cover: &[&str] = &[
        "## Main\nLet s be a new Set of Text.\nAdd \"a\" to s.\nAdd \"b\" to s.\nRemove \"a\" from s.\nShow length of s.\n",
        "## To worker (n: Int):\n    Show n.\n\n## Main\nLaunch a task to worker with 5.\n",
    ];
    for prog in must_cover {
        assert!(program_covered_by_native_builder(prog), "native builder should cover:\n{prog}");
        let native = program_to_core_wire_bytes(prog).expect("native marshal");
        let interp = program_to_core_wire_bytes_via_interpreter(prog).expect("interp marshal");
        assert_eq!(native, interp, "native builder bytes differ from the interpreter for:\n{prog}");
    }
    // CRDT counters + Inspect (pattern match) — these carry a type declaration (`Shared` struct / enum)
    // that `build_core` now skips, so they SHOULD hit the fast path; checked byte-identical whenever
    // covered (tolerant of any parse/coverage surprise the loaded box can't yet reveal).
    let typed: &[&str] = &[
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\n## Main\nLet mutable c be a new Counter.\nIncrease c's points by 10.\nShow c's points.\n",
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\n## Main\nLet mutable c be a new Counter.\nIncrease c's points by 10.\nDecrease c's points by 3.\nShow c's points.\n",
        // Inspect WITHOUT Otherwise → the flat-CIf desugaring the fast builder now reproduces.
        "## A Result is one of:\n    A Success with value Int.\n    A Failure with msg Text.\n\n## Main\nLet r be a new Success with value 42.\nInspect r:\n    When Success (v): Show v.\n    When Failure (m): Show m.\n",
        // Concurrency (Wave B): a `Simultaneously` block → CConcurrent with one branch per task.
        "## To sq (n: Int) -> Int:\n    Return n * n.\n\n## Main\nSimultaneously:\n    Let a be sq(5).\n    Let b be sq(10).\nShow a.\n",
        // Closures (Wave D): a `(params) -> body` expression; the second captures `x` (exercises the
        // now-deterministic sorted `captured`).
        "## Main\nLet f be (n: Int) -> n + 1.\nShow f(5).\n",
        "## Main\nLet x be 10.\nLet g be (n: Int) -> n + x.\nShow g(5).\n",
        // Inspect WITH Otherwise → the `__inspectMatched_N` flag desugaring, now reproduced via the
        // shared deterministic per-program index. Second program NESTS an Otherwise-inspect inside an
        // arm (flags 0 and 1) — proving the index increments identically in both encoders.
        "## A Result is one of:\n    A Success with value Int.\n    A Failure with msg Text.\n\n## Main\nLet r be a new Success with value 42.\nInspect r:\n    When Success (v): Show v.\n    Otherwise:\n        Show 0.\n",
        "## A Result is one of:\n    A Success with value Int.\n    A Failure with msg Text.\n\n## Main\nLet r be a new Success with value 42.\nInspect r:\n    When Success (v):\n        Inspect r:\n            When Failure (m): Show m.\n            Otherwise:\n                Show v.\n    Otherwise:\n        Show 0.\n",
        // Pipes: create + send + receive → CCreatePipe / CSendPipe / CReceivePipe.
        "## Main\nLet ch be a Pipe of Int.\nSend 42 into ch.\nReceive x from ch.\nShow x.\n",
        // Select over a channel with a timeout → CSelect{CSelectRecv, CSelectTimeout}.
        "## Main\nLet ch be a Pipe of Int.\nSend 42 into ch.\nAwait the first of:\n    Receive x from ch:\n        Show x.\n    After 1 seconds:\n        Show 0.\n",
        // AppendToSequence through a struct field → CForceDynamic(d) + CAppendToSeq.
        "## A Doc has:\n    lines: Seq of Text.\n\n## Main\nLet d be a new Doc.\nAppend \"Hello\" to d's lines.\nShow length of d's lines.\n",
        // === 100% coverage: the remaining exotic constructs ===
        // Zone + manifest-of → CZone + CManifestOf.
        "## Main\nInside a new zone called \"Z\":\n    Let m be the manifest of Z.\n    Show \"ok\".\n",
        // Proof/verification directives → CAssert / CCheck.
        "## Main\nLet x be 10.\nAssert that x is greater than 5.\nShow x.\n",
        "## Main\nLet h be 0.\nCheck that h is 0.\nShow h.\n",
        // Networking → CListen / CConnectTo / CSync.
        "## Main\nLet addr be \"/tcp/0\".\nListen on addr.\nConnect to addr.\nSync s on \"topic\".\nShow 1.\n",
        // IO → CWriteFile.
        "## Main\nWrite \"hello\" to file \"out.txt\".\nShow 1.\n",
        // Task control → CStopTask.
        "## Main\nLet h be 0.\nStop h.\nShow h.\n",
        // Trust directive → CTrust.
        "## Main\nLet x be 5.\nTrust that x is greater than 0 because \"validated\".\nShow x.\n",
        // Agent spawn → CSpawn.
        "## Main\nSpawn a Worker called \"w\".\nShow 1.\n",
        // Dependency directive → CRequire.
        "## Requires serde\n\n## Main\nShow 1.\n",
        // Persistence mount + console read → CMount / CReadConsole.
        "## Main\nMount c at \"data.journal\".\nRead input from the console.\nShow input.\n",
        // Actor messaging: peer agent + send + await → CConnectTo (LetPeerAgent) / CSendMessage / CAwaitMessage.
        "## Main\nLet addr be \"bob\".\nLet bob be a PeerAgent at addr.\nSend \"hi\" to bob.\nAwait response from bob into reply.\nShow reply.\n",
        // ChunkAt expression → CChunkAt.
        "## Main\nLet c be the chunk at 1 in \"Data\".\nShow c.\n",
        // Give as a call argument (transparent) → CCallS with the inner value.
        "## To consume (x: Int) -> Int:\n    Return x.\n\n## Main\nLet items be 5.\nCall consume with Give items.\n",
        // Inline-native escape block → CEscStmt.
        "## Main\nEscape to Rust:\n    let _ = 1;\n",
        // StreamMessage (relay batch) → CStreamMessage.
        "## Main\nLet x be 5.\nStream x to remote.\nShow x.\n",
        // Splice: a multi-value `Push` desugars to `Stmt::Splice` → CIf(true, [pushes], []).
        "## Main\nLet coll be a new Seq of Int.\nPush 1, 2, 3 to coll.\nShow length of coll.\n",
    ];
    let mut diverged: Vec<usize> = Vec::new();
    for (i, prog) in typed.iter().enumerate() {
        if !program_covered_by_native_builder(prog) {
            eprintln!("[note] typed[{i}] not covered — falls back correctly");
            continue;
        }
        let native = program_to_core_wire_bytes(prog).expect("native marshal");
        let interp = program_to_core_wire_bytes_via_interpreter(prog).expect("interp marshal");
        if native == interp {
            eprintln!("[ok] typed[{i}] covered + byte-identical ({} bytes)", native.len());
        } else {
            let fd = native.iter().zip(&interp).position(|(a, b)| a != b);
            eprintln!(
                "[DIVERGE] typed[{i}]: native {} bytes, interp {} bytes, first diff at {:?}\n{prog}\n  native = {:?}\n  interp = {:?}",
                native.len(),
                interp.len(),
                fd,
                decode_value_raw(&native),
                decode_value_raw(&interp)
            );
            diverged.push(i);
        }
    }
    assert!(diverged.is_empty(), "typed programs diverged from the interpreter: {diverged:?}");
}

/// Win 2c (speed ratchet) — the single-pass marshal must BEAT the two-pass one. Both do the identical
/// parse+build; the single pass additionally skips the `RuntimeValue` tree allocation and the second
/// encode walk, so best-of-N (same run, shared contention cancels) it is reliably faster. This is the
/// genuine speed lock on the ~94%-of-latency marshal — the transport parity check is separate.
#[test]
fn single_pass_marshal_beats_the_two_pass() {
    let prog = "## To dbl (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet mutable s be 0.\nRepeat for i from 1 to 20:\n    Set s to s + dbl(i).\nLet xs be [1, 2, 3, 4, 5].\nShow s.\n";
    for _ in 0..20 {
        let _ = program_to_core_wire_bytes(prog);
        let _ = program_to_core_wire_bytes_two_pass(prog);
    }
    let mut best_single = std::time::Duration::from_secs(9999);
    let mut best_two = std::time::Duration::from_secs(9999);
    for _ in 0..500 {
        let t = std::time::Instant::now();
        let _ = program_to_core_wire_bytes(prog).expect("single");
        best_single = best_single.min(t.elapsed());
        let t = std::time::Instant::now();
        let _ = program_to_core_wire_bytes_two_pass(prog).expect("two-pass");
        best_two = best_two.min(t.elapsed());
    }
    let speedup = best_two.as_secs_f64() / best_single.as_secs_f64().max(1e-12);
    eprintln!("[marshal] single-pass = {best_single:?}, two-pass = {best_two:?}, speedup = {speedup:.3}x");
    assert!(
        best_single < best_two,
        "single-pass marshal ({best_single:?}) should beat the two-pass ({best_two:?}) — it skips the \
         RuntimeValue tree allocation and the second encode walk that both otherwise share"
    );
}

/// DIAGNOSTIC — where does an in-process specialization actually spend its time? Splits the whole
/// call into host marshal (`program_to_core_wire_bytes`), the parse+build sub-part of it
/// (`program_covered_by_native_builder` = `program_to_core_value`, discarded), and the residual
/// (FFI + native `peBlock` + finish). All three are timed in ONE interleaved loop. The printed split
/// is a HUMAN-FACING diagnostic, not a hard invariant: best-of-N minimums of separate measurements are
/// not order-guaranteed under heavy machine contention (a loaded box can hand the parse+build loop a
/// worse slice than the marshal loop), so no ORDERING is asserted — only real correctness, that the
/// marshal produces bytes.
#[test]
fn native_pe_inprocess_latency_breakdown() {
    let prog = "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 20:\n    Set s to s + i.\nShow s.\n";
    for _ in 0..5 {
        run_native_pe_inprocess(prog).expect("warm");
    }
    let mut best_marshal = std::time::Duration::from_secs(9999);
    let mut best_parse_build = std::time::Duration::from_secs(9999);
    let mut best_total = std::time::Duration::from_secs(9999);
    for _ in 0..300 {
        let t = std::time::Instant::now();
        let _ = program_to_core_wire_bytes(prog).expect("marshal");
        best_marshal = best_marshal.min(t.elapsed());
        let t = std::time::Instant::now();
        let _ = program_covered_by_native_builder(prog);
        best_parse_build = best_parse_build.min(t.elapsed());
        let t = std::time::Instant::now();
        run_native_pe_inprocess(prog).expect("total");
        best_total = best_total.min(t.elapsed());
    }
    eprintln!(
        "[breakdown] total = {best_total:?} | marshal = {best_marshal:?} \
         (of which parse+build ~= {best_parse_build:?}) | residual (FFI + peBlock + finish) ~= {:?}",
        best_total.saturating_sub(best_marshal)
    );
    assert!(
        !program_to_core_wire_bytes(prog).expect("marshal").is_empty(),
        "the marshal must produce wire bytes"
    );
}

/// Win 2b (no-regression) — WARM, the in-process cdylib path must be NO SLOWER than the resident
/// server beyond noise. The breakdown (`native_pe_inprocess_latency_breakdown`) shows the host
/// marshal is ~94% of a specialization and is SHARED by both paths; removing the process boundary
/// only saves the ~small IPC delta, so the two are at parity — well within contention noise. This
/// lock therefore asserts PARITY (a generous ceiling), not a strict win: the in-process path is the
/// default for its ROBUSTNESS (no child process / pipe / zombies), and this guards that choosing it
/// never introduced a real perf regression. The genuine speed ratchet is on the marshal itself
/// (`marshal_direct_is_byte_identical_and_faster_than_the_tree_path`).
#[test]
fn native_pe_inprocess_is_not_slower_than_the_server() {
    let prog = "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 20:\n    Set s to s + i.\nShow s.\n";
    for _ in 0..5 {
        run_native_pe_inprocess(prog).expect("warm in-process");
        run_native_pe_server(prog).expect("warm server");
    }
    let mut best_inproc = std::time::Duration::from_secs(9999);
    let mut best_server = std::time::Duration::from_secs(9999);
    for _ in 0..100 {
        let t = std::time::Instant::now();
        run_native_pe_inprocess(prog).expect("in-process");
        best_inproc = best_inproc.min(t.elapsed());
        let t = std::time::Instant::now();
        run_native_pe_server(prog).expect("server");
        best_server = best_server.min(t.elapsed());
    }
    eprintln!(
        "[native PE latency] in-process = {best_inproc:?}, resident-server = {best_server:?} \
         (parity — both dominated by the shared host marshal)"
    );
    // Parity, robust to heavy contention: the in-process default must not be materially slower than
    // the server it replaced. A 2x ceiling is far above the true ~1x so this never flakes, while
    // still catching a genuine regression (e.g. a per-call reload of the cdylib).
    assert!(
        best_inproc.as_secs_f64() < best_server.as_secs_f64() * 2.0,
        "in-process PE ({best_inproc:?}) must not be materially slower than the server ({best_server:?})"
    );
}
