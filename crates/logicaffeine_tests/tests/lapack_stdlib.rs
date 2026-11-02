//! LOGOS-LAPACK — the `lapack.lg` stdlib surface (decompositions built on the naive kernels).
//!
//! Reachable today with only the `sqrt` builtin: Cholesky (`potrf`), LU with partial pivoting
//! (`getrf`/`getrs`/`gesv`), and QR via Householder reflectors (`geqrf`/`qrSolve`). Each is the
//! textbook algorithm — no blocking, no LDA gymnastics — and is locked three ways: exact
//! hand-computed factors, an algebraic reconstruction property (`L·Lᵀ==A`, `A·x==b`, `Q·R==A`)
//! computed in-LOGOS, and cross-tier agreement (`assert_all_tiers` enforces interp==VM).

mod common;
use common::assert_all_tiers;

// ---- Cholesky: potrf (A = L·Lᵀ, L lower-triangular) ----

#[test]
fn potrf_factors_2x2_spd() {
    // A=[[4,2],[2,5]] → L11=2, L21=1, L22=sqrt(5-1)=2 → L=[[2,0],[1,2]].
    assert_all_tiers(
        "## Main\nShow potrf(2, [4.0, 2.0, 2.0, 5.0]).\n",
        "[2, 0, 1, 2]",
    );
}

#[test]
fn potrf_factors_3x3_spd() {
    // A=[[4,2,2],[2,5,3],[2,3,6]] → L=[[2,0,0],[1,2,0],[1,1,2]].
    assert_all_tiers(
        "## Main\nShow potrf(3, [4.0, 2.0, 2.0, 2.0, 5.0, 3.0, 2.0, 3.0, 6.0]).\n",
        "[2, 0, 0, 1, 2, 0, 1, 1, 2]",
    );
}

#[test]
fn potrf_reconstructs_original_spd() {
    // The defining property: L·Lᵀ == A. `recon` rebuilds the product in-LOGOS from potrf's L
    // (upper-triangle zeros make the sum range naturally over p ≤ min(i,j)).
    assert_all_tiers(
        "## To recon (n: Int, el: Seq of Float) -> Seq of Float:\n\
         \x20   Let mutable out be a new Seq of Float.\n\
         \x20   Repeat for i from 1 to n:\n\
         \x20       Repeat for j from 1 to n:\n\
         \x20           Let mutable s be 0.0.\n\
         \x20           Repeat for p from 1 to n:\n\
         \x20               Set s to s + (item ((i - 1) * n + p) of el) * (item ((j - 1) * n + p) of el).\n\
         \x20           Push s to out.\n\
         \x20   Return out.\n\
         \n\
         ## Main\n\
         Let el be potrf(3, [4.0, 2.0, 2.0, 2.0, 5.0, 3.0, 2.0, 3.0, 6.0]).\n\
         Show recon(3, el).\n",
        "[4, 2, 2, 2, 5, 3, 2, 3, 6]",
    );
}
