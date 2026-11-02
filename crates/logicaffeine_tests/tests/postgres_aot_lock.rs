//! ════════════════════════════════════════════════════════════════════════════════════════════
//! POSTGRES AOT-RUST LOCK — `assets/std/postgres.lg` must COMPILE and RUN through the AOT-Rust
//! backend (`codegen/`), not only the interpreter and register-VM tiers.
//!
//! `postgres.lg` (the pure-LOGOS PostgreSQL v3 wire client) slices `Seq of Int` byte buffers,
//! returns `(Seq of Text, Seq of Bool)` tuples from its frame parsers, threads message text through
//! value-copied `String` bindings, and — via its SCRAM/MD5 auth path — drags the whole `crypto.lg`
//! module into the compile. For a long time none of that survived AOT-Rust codegen: a `largo build`
//! of any program importing it emitted Rust that failed to compile with ~two dozen latent bugs
//! (LogosSeq range-indexing, a dropped slice-end binding, tuple-of-Seq boxing, value-copy moves,
//! and dead ML-DSA crypto functions the whole-module splice pulled in). This lock drives the
//! pure-byte surface of `postgres.lg` through Logos → Rust → rustc → a NATIVE BINARY that runs, so
//! the AOT tier can never silently rot back to un-compilable again.
//!
//!  ⚠️  This is a COMPILE+RUN gate: if the generated Rust stops compiling, the binary never runs and
//!      the assertion fails loudly. Do not weaken it to a string match.
//! ════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_output;

/// The frame/URL/text surface of `postgres.lg` — the functions that exercised every AOT-Rust codegen
/// bug — compiled to native Rust and RUN, printing a deterministic transcript. `pgParseUrl` slices the
/// credential/host byte ranges (LogosSeq range-index), `pgParseDataRow`/`pgParseRowDescription` return
/// `(Seq, Seq)` tuples (Value::Seq boxing + the dropped slice-end binding), `pgErrorText` threads a
/// destructured `String` through sibling branches (value-copy moves), and the whole thing pulls in the
/// SCRAM/MD5 crypto path (whose dead ML-DSA functions must be pruned, not miscompiled).
#[test]
fn postgres_lg_compiles_and_runs_through_aot_rust() {
    assert_output(
        "## Main\n\
         Let pu be pgParseUrl(\"postgres://alice:secret@db.example.com:5432/store\").\n\
         Inspect pu:\n\
         \x20   When Ok (url):\n\
         \x20       Show url's user.\n\
         \x20       Show url's host.\n\
         \x20       Show url's port.\n\
         \x20       Show url's database.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let bad be pgParseUrl(\"http://nope\").\n\
         Inspect bad:\n\
         \x20   When Ok (u2):\n\
         \x20       Show u2's host.\n\
         \x20   When Err (e2):\n\
         \x20       Show e2.\n\
         Show pgRowsAffected(\"INSERT 0 3\").\n\
         Show pgRowsAffected(\"CREATE TABLE\").\n\
         Show bytesToHex(pgI32be(196608)).\n\
         Show bytesToHex(pgStartupMessage(\"u\", \"d\")).\n",
        "alice\n\
         db.example.com\n\
         5432\n\
         store\n\
         pgParseUrl: the url must start with postgres:// or postgresql://\n\
         3\n\
         0\n\
         00030000\n\
         0000001b0003000075736572007500646174616261736500640000",
    );
}

/// The frame PARSERS — the `(Seq of Text, Seq of Int)` / `(Seq of Text, Seq of Bool)` tuple-returning
/// functions whose element boxing and dropped slice-end binding were the hardest AOT-Rust bugs — built
/// from bytes and destructured, RUN natively. A hand-built RowDescription 'T' body (one field "id",
/// type OID 23) and a DataRow 'D' body (one non-NULL cell "42", one NULL cell) round-trip through the
/// parsers to their column names, type OIDs, cell text, and honest NULL mask.
#[test]
fn postgres_lg_frame_parsers_compile_and_run_through_aot_rust() {
    assert_output(
        "## Main\n\
         Let rd be a new Seq of Int.\n\
         Push 0 to rd.\n\
         Push 1 to rd.\n\
         Push 105 to rd.\n\
         Push 100 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 23 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Push 0 to rd.\n\
         Let (names, oids) be pgParseRowDescription(rd).\n\
         Show length of names.\n\
         Show item 1 of names.\n\
         Show item 1 of oids.\n\
         Let dr be a new Seq of Int.\n\
         Push 0 to dr.\n\
         Push 2 to dr.\n\
         Push 0 to dr.\n\
         Push 0 to dr.\n\
         Push 0 to dr.\n\
         Push 2 to dr.\n\
         Push 52 to dr.\n\
         Push 50 to dr.\n\
         Push 255 to dr.\n\
         Push 255 to dr.\n\
         Push 255 to dr.\n\
         Push 255 to dr.\n\
         Let (cells, mask) be pgParseDataRow(dr).\n\
         Show length of cells.\n\
         Show item 1 of cells.\n\
         Show item 1 of mask.\n\
         Show item 2 of mask.\n",
        "1\nid\n5888\n2\n42\nfalse\ntrue",
    );
}
