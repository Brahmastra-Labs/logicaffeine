//! Stdlib-parity — file IO domain. The read/write PRIMITIVES live in file.lg (## To native
//! read/write -> Result). Environmental IO can't have cross-tier byte-identity, so these are
//! verified on the NATIVE compilation path (a real binary doing real filesystem IO), not the
//! sandboxed all-tier interpreter harness. Uses the existing primitives — no reinvention.
#![allow(unused_imports, dead_code)]
mod common;
use common::assert_exact_output;

/// Write text to a file (file.lg `write`), then confirm it succeeded.
#[test]
fn parity_io_write_file() {
    assert_exact_output(
        "## Main\nLet w be write(\"/tmp/logos_parity_wr.txt\", \"payload\").\nInspect w:\n    When Ok (u):\n        Show \"ok\".\n    When Err (e):\n        Show e.\n",
        "ok",
    );
}

/// Read a whole file back (file.lg `read`) after writing it — real filesystem round-trip.
#[test]
fn parity_io_read_file() {
    assert_exact_output(
        "## Main\nLet w be write(\"/tmp/logos_parity_rd.txt\", \"hello io\").\nLet r be read(\"/tmp/logos_parity_rd.txt\").\nInspect r:\n    When Ok (v):\n        Show v.\n    When Err (e):\n        Show e.\n",
        "hello io",
    );
}
