//! `programs` — the LOGOS source of the reference programs the spatial lane is anchored on.
//!
//! ★ WHY A PROGRAM AND NOT A KERNEL. Everything else in `spatial/` describes computation as IR; these
//! are the ORDINARY LOGOS programs the IR is built from, kept as source so the anchor is the language
//! rather than any lowering of it. The GEMM oracle below is a scalar integer matmul: compiled to C it
//! gives a card-free ground truth for every Tenstorrent number, and recognized by `tensor_recognize`
//! it is the same program the spatial pipeline lowers — which is what makes "the accelerator agrees
//! with the oracle" a statement about one program rather than two.
//!
//! Salvaged from `codegen_tt/matrix.rs` during the retirement; `codegen_tt` re-exports the name so callers
//! are unmoved.

pub const GEMM_ORACLE_LOGOS: &str = r#"## Main
Let n be 16.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Let mutable j be 0.
    While j is less than n:
        Push ((i * n + j) % 255) - 128 to a.
        Push ((j * n + i) % 255) - 128 to b.
        Push 0 to c.
        Set j to j + 1.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable k be 0.
    While k is less than n:
        Let mutable j be 0.
        While j is less than n:
            Let idx be i * n + j + 1.
            Set item idx of c to item idx of c + item (i * n + k + 1) of a * item (k * n + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Set i to 1.
While i is at most n * n:
    Set checksum to checksum + item i of c.
    Set i to i + 1.
Show checksum.
"#;

/// The canonical LOGOS matmul the recognizer lifts: `C(m×n) = A(m×k)·B(k×n)` over `Seq of Int` with the
/// deterministic fills `a[p] = (p % 7) - 3`, `b[p] = (p % 11) - 5`, and a `Show checksum` of `C` — the
/// same program the spatial locks anchor on (`tests/common/spatial.rs`), here so a report generator can
/// lower it without depending on a test tree.
pub fn gemm_program(m: usize, n: usize, k: usize) -> String {
    format!(
        r#"## Main
Let rows be {m}.
Let cols be {n}.
Let mid be {k}.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable p be 0.
While p is less than rows * mid:
    Push (p % 7) - 3 to a.
    Set p to p + 1.
Set p to 0.
While p is less than mid * cols:
    Push (p % 11) - 5 to b.
    Set p to p + 1.
Set p to 0.
While p is less than rows * cols:
    Push 0 to c.
    Set p to p + 1.
Let mutable i be 0.
While i is less than rows:
    Let mutable k be 0.
    While k is less than mid:
        Let mutable j be 0.
        While j is less than cols:
            Let idx be i * cols + j + 1.
            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Let mutable q be 1.
While q is at most rows * cols:
    Set checksum to checksum + item q of c.
    Set q to q + 1.
Show checksum.
"#
    )
}
