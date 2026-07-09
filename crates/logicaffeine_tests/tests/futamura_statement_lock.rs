//! ════════════════════════════════════════════════════════════════════════════════════════════
//! FUTAMURA STATEMENT LOCK — every statement the tree-walker executes MUST also survive the
//! projection-1 self-interpreter encoding (`encode_program_source`), or it is SILENTLY DROPPED from
//! the residual program. `encode_stmt_src`'s catch-all does `return String::new()`, so any statement
//! it does not explicitly handle vanishes from the Futamura projection — the program would specialize
//! to something that no longer does the dropped operation. This already bit CRDT statements once.
//!
//!  ⚠️  YOU DO NOT GET TO CHANGE THIS TEST TO LET THINGS REGRESS.  ⚠️
//!  - NEVER delete a row from `LOCKED` or weaken an assertion to make a RED case pass. A RED row
//!    means a statement is being dropped from the Futamura projections — the fix is to HANDLE it in
//!    `compile::encode_stmt_src` AND the PE dialects (`optimize/pe_source.logos`,
//!    `pe_mini_source.logos`, `pe_bti_source.logos`, `decompile_source.logos`), never to relax here.
//!  - You may ONLY edit this test to ADD coverage (a new statement, a stronger assertion). Strictly
//!    monotone: it grows, it never shrinks.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use logicaffeine_compile::compile::encode_program_source;

/// One program exercising the full networking + streaming + concurrency cluster, so they are LOCKED
/// TOGETHER: a single encoding must carry every one. The right column is the self-interpreter
/// constructor that proves the statement was encoded (not dropped).
const NETWORKING_AND_STREAMING_PROGRAM: &str = "## Main\n\
    \x20   Let msg be 1.\n\
    \x20   Let items be [1, 2, 3].\n\
    \x20   Listen on \"me\".\n\
    \x20   Connect to \"relay\".\n\
    \x20   Send msg to \"agent\".\n\
    \x20   Stream items to \"agent\".\n\
    \x20   Await response from \"agent\" into reply.\n";

/// Constructors that MUST appear in the encoding. ADD here when the language gains a statement;
/// never remove.
const LOCKED: &[&str] = &["CListen", "CConnectTo", "CSendMessage", "CStreamMessage", "CAwaitMessage"];

#[test]
fn every_networking_and_streaming_statement_survives_futamura_projection() {
    let encoded = encode_program_source(NETWORKING_AND_STREAMING_PROGRAM)
        .expect("the networking/streaming program must encode for projection-1");
    for ctor in LOCKED {
        assert!(
            encoded.contains(ctor),
            "FUTAMURA LOCK REGRESSION: a statement was DROPPED from the projection-1 encoding \
             (missing `{ctor}`). `encode_stmt_src`'s catch-all silently drops any statement it does \
             not handle — the fix is to HANDLE the statement in `encode_stmt_src` + the PE dialects, \
             NEVER to weaken or delete this lock.\n\nEncoded form:\n{encoded}"
        );
    }
}
