//! ════════════════════════════════════════════════════════════════════════════════════════════
//! WASM AOT — GUIDE-CORPUS COVERAGE RATCHET.
//!
//! `crates/logicaffeine_tests/tests/wasm_aot_lock.rs` locks the backend against a curated corpus
//! + an op-exhaustive catalog. THIS test locks it against the widest *real* corpus we ship: every
//! imperative example in the public Syntax Guide (`ui::pages::guide::content::SECTIONS`). It is the
//! guide-level analog of that file's bidirectional op lock — the thing that answers "does the AOT
//! backend support ALL the language the user actually sees?".
//!
//! For every `ExampleMode::Imperative` guide example we run `compile_to_wasm` and classify it:
//!
//!   • COMPILED  — the AOT backend lowers it to a `.wasm` module. Counted toward the floor.
//!   • REJECTED  — the backend returns `Unsupported(reason)`. Every rejection MUST appear in
//!                 [`KNOWN_DEFERRED`] (with the reason/phase it lands in). A rejection NOT on that
//!                 list is a silent coverage gap / regression → FAIL (fix the backend, or document
//!                 the deferral with its landing phase).
//!   • PANICKED  — `compile_to_wasm` unwound. That is a hard backend bug → FAIL (never a skip).
//!
//! Three guarantees, mirroring the op lock:
//!   1. `compiled >= SUPPORTED_FLOOR` — coverage is monotone (raise the floor as gaps close, never
//!      lower it to make a red pass).
//!   2. every REJECTED id ∈ KNOWN_DEFERRED — no new gap escapes unnoticed.
//!   3. every KNOWN_DEFERRED id STILL rejects — a deferral the backend has since grown to compile
//!      is a stale entry to REMOVE (a coverage win), so the deferred set only shrinks.
//!
//! ⚠️  The fix for a RED is in the BACKEND (`crates/logicaffeine_compile/src/vm/wasm/`), or — for a
//!     genuinely-deferred runtime (concurrency / CRDT / net / policy / memory-zones, which need the
//!     linker phase or an async host) — a documented KNOWN_DEFERRED entry. NEVER a wildcard skip.

#![cfg(feature = "web-tests")]

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_web::ui::pages::guide::content::{ExampleMode, Runnability, SECTIONS};
use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Imperative guide examples the direct WASM AOT backend does NOT yet lower, each with the reason /
/// phase it lands in. The shrinking gap toward full-language coverage. Adding a wildcard or moving a
/// compilable example here to dodge a red is forbidden (see the file header).
const KNOWN_DEFERRED: &[(&str, &str)] = &[
    // ── Concurrency ALL compiles self-contained now (channels = FIFO queues, `Launch` = synchronous
    //    task, `Stop`/`Sleep` = no-op, `select` = first-ready-recv-else-timeout), verified tw ==
    //    driven-VM == AOT. `select-timeout` compiles: the `Pipe of T` element type carried on `ChanNew`
    //    types the recv-arm variable even for a never-sent-to pipe. ──
    // ── CRDTs ALL compile self-contained single-replica now — counter (`±=`)/merge (Int sum),
    //    LWW/SharedMap (struct/map), Divergent register (`CrdtResolve` field write), RGA/sequence
    //    (`CrdtAppend` in-place list_push), AND the OR-SET field (`SharedSet of Text`, in-place
    //    mutation via the non-cow-clonable `CrdtSetText` kind). So none are here. ──
    // ── Networking + sync: the DETERMINISTIC LOCAL mode (offline single node, no relay) runs in all
    //    three engines — `Connect`/Listen/Send/Sync/PeerAgent are local no-ops, so the Shows + local
    //    CRDT values are the byte-identical output (verified tw == VM-net == AOT). `Connect` is a
    //    single-node no-op offline (nothing to dial → `net` stays None, the following ops run locally);
    //    a real deployment dials via the relay driver. So network-connect/-listen/-peer-agent/-send-
    //    message/-mdns/-file-transfer/-distributed + crdt-sync-* are NOT here. (`network-distributed`
    //    compiles: its `Mount` → `FailWith` traps exactly where tw errors without a VFS — error parity.) ──
    // ── Numeric tower (guide W3): Rational arithmetic and the `decimal`/`complex` builtins are
    //    exact heap-backed values from logicaffeine_base — the direct self-contained backend has no
    //    representation for them. They land with the P2 LINKER tier (rust-lld links the real base
    //    crate, the same path that gave wasm BigInt via `--emit wasm-linked`). Everything else in
    //    the W3 sections (BigInt promotion codegen, machine words, shifts, count_ones) lowers today. ──
    ("rational-exact", "Rational values need the linked base crate (wasm-linked/P2)"),
    ("decimal-vs-float", "the `decimal` builtin needs the linked base crate (wasm-linked/P2)"),
    ("complex-i", "the `complex` builtin needs the linked base crate (wasm-linked/P2)"),
    // ── Everyday types (guide W4): Money/Quantity/UUID are heap-backed exact values from
    //    logicaffeine_base — same wasm-linked/P2 landing as the numeric tower above. ──
    ("money-exact", "Money needs the linked base crate (wasm-linked/P2)"),
    ("money-split", "Money needs the linked base crate (wasm-linked/P2)"),
    ("quantity-dimensions", "Quantity needs the linked base crate (wasm-linked/P2)"),
    ("uuid-version", "the `uuid` builtins need the linked base crate (wasm-linked/P2)"),
    // ── Direct-backend growth items (guide W4), each a bounded lowering gap: ──
    ("map-destructure", "map iteration (`Repeat for (k, v) in m`) is not lowered yet"),
    ("program-wordcount", "map iteration (`Repeat for (k, v) in m`) is not lowered yet"),
    ("stdlib-file-read", "file.lg's `read` rides the shared FileRead builtin (tw+VM); the wasm backend refuses it — the browser has no filesystem (OPFS is the fs-module story)"),
    ("option-some-none", "the transparent-Option `When Some/None` matching (landed in tw+VM this wave) awaits the wasm inspect lowering"),
];

/// The curated guide corpus must keep at least this many imperative examples lowering to WASM. Raise
/// it as gaps close; never lower it to make a red pass.
// 139 → 137 on the W7 corpus REDEFINITION (ExpectedError examples left the corpus by class
// rule — two of them previously compiled), then 137 → 136 when theorem-socrates moved to
// the new Theorem mode (proved live by the kernel — it is no longer imperative corpus).
// The backend lost nothing in either step; the denominator changed. 136 → 141 when the
// `{v=}` debug-interpolation lowering landed (interp-debug left KNOWN_DEFERRED).
// 141 → 142 when `stdlib-result` was PROMOTED: `BoundaryType` learned to model a `Result`, so a
// generic enum's payload now resolves at a region boundary and the guide's Result example lowers.
const SUPPORTED_FLOOR: usize = 142;

#[test]
fn wasm_aot_covers_the_imperative_guide_corpus() {
    let mut compiled: Vec<&str> = Vec::new();
    let mut rejected: Vec<(&str, String)> = Vec::new();
    let mut panicked: Vec<&str> = Vec::new();

    for section in SECTIONS.iter() {
        for ex in section.examples {
            if ex.mode != ExampleMode::Imperative {
                continue;
            }
            // ExpectedError examples exist to DISPLAY a diagnostic — several
            // fail at parse time by design, so they are error exhibits, not
            // backend-coverage corpus. (Principled class rule, not a skip of
            // any compilable example.)
            if matches!(ex.runnability, Runnability::ExpectedError { .. }) {
                continue;
            }
            let code = ex.code;
            match catch_unwind(AssertUnwindSafe(|| compile_to_wasm(code))) {
                Ok(Ok(_)) => compiled.push(ex.id),
                Ok(Err(e)) => {
                    let msg = format!("{e:?}");
                    // Distill the `unsupported …` reason for a readable census.
                    let reason = msg.split("wasm AOT backend: ").nth(1).unwrap_or(&msg);
                    let reason = reason
                        .split("\")")
                        .next()
                        .unwrap_or(reason)
                        .trim_matches('"')
                        .to_string();
                    rejected.push((ex.id, reason));
                }
                Err(_) => panicked.push(ex.id),
            }
        }
    }

    let total = compiled.len() + rejected.len() + panicked.len();
    println!(
        "=== WASM AOT guide coverage: {total} imperative examples → {} compiled, {} deferred, {} panicked ===",
        compiled.len(),
        rejected.len(),
        panicked.len()
    );
    for (id, reason) in &rejected {
        println!("  defer {id}: {reason}");
    }

    // (3-hard-bug) No compile may unwind.
    assert!(panicked.is_empty(), "compile_to_wasm PANICKED on guide example(s): {panicked:?} — a hard backend bug, fix vm/wasm/");

    let deferred_ids: BTreeSet<&str> = KNOWN_DEFERRED.iter().map(|(id, _)| *id).collect();
    let rejected_ids: BTreeSet<&str> = rejected.iter().map(|(id, _)| *id).collect();

    // (2-no-new-gap) Every rejection is a documented deferral.
    let undocumented: Vec<&(&str, String)> = rejected
        .iter()
        .filter(|(id, _)| !deferred_ids.contains(id))
        .collect();
    assert!(
        undocumented.is_empty(),
        "WASM AOT REJECTED guide example(s) NOT on the KNOWN_DEFERRED allowlist — a new coverage gap \
         to FIX in the backend (crates/logicaffeine_compile/src/vm/wasm/) or document with its landing \
         phase: {undocumented:?}"
    );

    // (shrink-only) Every documented deferral still rejects — a now-compiling one must be PROMOTED.
    let stale: Vec<&str> = KNOWN_DEFERRED
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| !rejected_ids.contains(id))
        .collect();
    assert!(
        stale.is_empty(),
        "KNOWN_DEFERRED lists guide example(s) the WASM backend now COMPILES — REMOVE them (a coverage \
         win) and raise SUPPORTED_FLOOR: {stale:?}"
    );

    // (1-monotone-floor) Coverage never regresses.
    assert!(
        compiled.len() >= SUPPORTED_FLOOR,
        "WASM AOT guide coverage REGRESSED: only {} imperative examples lowered (floor {SUPPORTED_FLOOR}). \
         Fix the backend; do not lower the floor. Rejects: {rejected:?}",
        compiled.len()
    );
}
