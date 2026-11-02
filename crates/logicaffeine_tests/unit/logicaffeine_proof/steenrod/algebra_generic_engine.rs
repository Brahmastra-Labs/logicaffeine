use super::*;

/// ONE machine, every spectrum it can reach. The resolution engine is now generic over the algebra.
/// Two checks:
///   (1) SELF-CONSISTENCY: the generic path with the full Steenrod algebra reproduces the sphere's chart
///       (the same one the rest of the suite pins) — so the generalization changed nothing for `𝒜`.
///   (2) NEW SPECTRUM: pointing it at `A(0) = ⟨Sq¹⟩ = F₂[Sq¹]/(Sq¹²)`, the exterior algebra on one
///       degree-1 class, computes the Adams `E₂` for **HZ** (integral homology). Its Ext is the pure
///       polynomial `F₂[h_0]`: `Ext^{s,t} = 1` exactly on the diagonal `t = s` (the `h_0`-tower,
///       `π_0(HZ) = Z`), `0` elsewhere — and critically `Ext^{1,2} = 0` (no `h_1`), which certifies it is
///       HZ and not the sphere.
#[test]
fn the_engine_is_algebra_generic_and_computes_hz_via_a0() {
    // (1) self-consistency on the sphere.
    let sphere = ext_over(&SteenrodAlgebra, 6, 12);
    assert_eq!(
        sphere,
        ext(6, 12),
        "generic engine on 𝒜 reproduces the sphere"
    );
    assert_eq!(
        (sphere[1][1], sphere[1][2], sphere[1][4]),
        (1, 1, 1),
        "sphere has h_0, h_1, h_2"
    );

    // (2) HZ via A(0) — a genuinely different spectrum from the same machine.
    let a0 = build_subalgebra(&[vec![1]], 1);
    assert_eq!(a0.dimension(), 2, "A(0) = {{1, Sq¹}} is 2-dimensional");
    let hz = ext_over(&a0, 12, 14);
    for s in 0..=12 {
        for t in 0..=14 {
            assert_eq!(
                hz[s][t],
                usize::from(t == s),
                "Ext_A(0)^{{{s},{t}}} = F₂[h_0] diagonal"
            );
        }
    }
    assert_eq!(
        hz[1][2], 0,
        "HZ has NO h_1 — the engine distinguishes HZ from the sphere"
    );
}

/// **ko, landed.** With the combination-basis, `A(1) = ⟨Sq¹, Sq²⟩` is faithfully represented (its elements
/// like `Sq²Sq³ = Sq⁵ + Sq⁴Sq¹` are sums of admissibles, not single monomials) — it is genuinely
/// 8-dimensional. The same resolution engine then computes the Adams `E₂` for **ko**:
///   * its only one-line classes are `h_0 = [Sq¹]` and `h_1 = [Sq²]` — `Ext^{1,4} = 0`, NO `h_2` (the
///     sphere has one), and no higher `h_i`, because the only indecomposables of `A(1)` are `Sq¹, Sq²`;
///   * the infinite `h_0`-tower `Ext^{s,s} = 1` gives `π_0(ko) = Z`.
/// So one machine now reaches three spectra: the sphere (`𝒜`), HZ (`A(0)`), and ko (`A(1)`).
#[test]
fn the_engine_lands_ko_via_the_combination_basis_a1() {
    let a1 = build_subalgebra(&[vec![1], vec![2]], 6);
    assert_eq!(
        a1.dimension(),
        8,
        "A(1) is 8-dimensional (the combination basis sees it correctly)"
    );

    let ko = ext_over(&a1, 12, 16);
    assert_eq!(ko[0][0], 1, "Ext^{{0,0}} = Z/2");
    assert_eq!(ko[1][1], 1, "h_0 = [Sq¹]");
    assert_eq!(ko[1][2], 1, "h_1 = [Sq²]");
    assert_eq!(
        ko[1][4], 0,
        "ko has NO h_2 — distinguishes ko from the sphere"
    );
    assert_eq!(ko[1][8], 0, "ko has NO h_3");
    for t in 3..=16 {
        assert_eq!(
            ko[1][t], 0,
            "the only A(1)-indecomposables are Sq¹, Sq² (t={t})"
        );
    }
    for s in 0..=12 {
        assert_eq!(
            ko[s][s], 1,
            "the infinite h_0-tower: π_0(ko)=Z at (s,s)={s}"
        );
    }
}

/// **tmf, landed.** `A(2) = ⟨Sq¹, Sq², Sq⁴⟩` is 64-dimensional — the combination basis handles it as
/// easily as A(1). The same engine computes the Adams `E₂` for **tmf** (topological modular forms):
///   * the one-line carries `h_0, h_1, h_2` (degrees 1, 2, 4 — the three generators) and NOTHING else:
///     `Ext^{1,4} = 1` so tmf HAS `h_2` (unlike ko), but `Ext^{1,8} = 0` so it has NO `h_3` (unlike the
///     sphere). That `{h_0,h_1,h_2}`-and-stop is the exact tmf fingerprint, sitting between ko and S.
///   * the infinite `h_0`-tower `Ext^{s,s} = 1` gives `π_0(tmf) = Z`.
/// Four spectra now, one machine: S (`𝒜`), HZ (`A(0)`), ko (`A(1)`), tmf (`A(2)`).
#[test]
fn the_engine_lands_tmf_via_the_combination_basis_a2() {
    let a2 = build_subalgebra(&[vec![1], vec![2], vec![4]], 23);
    assert_eq!(a2.dimension(), 64, "A(2) is 64-dimensional");

    let tmf = ext_over(&a2, 10, 12);
    assert_eq!(tmf[0][0], 1, "Ext^{{0,0}} = Z/2");
    assert_eq!(tmf[1][1], 1, "h_0 = [Sq¹]");
    assert_eq!(tmf[1][2], 1, "h_1 = [Sq²]");
    assert_eq!(tmf[1][4], 1, "h_2 = [Sq⁴] — tmf HAS h_2 (unlike ko)");
    assert_eq!(
        tmf[1][8], 0,
        "tmf has NO h_3 — distinguishes tmf from the sphere"
    );
    for t in 3..=12 {
        if t != 4 {
            assert_eq!(tmf[1][t], 0, "only h_0,h_1,h_2 on the tmf one-line (t={t})");
        }
    }
    for s in 0..=10 {
        assert_eq!(
            tmf[s][s], 1,
            "the infinite h_0-tower: π_0(tmf)=Z at (s,s)={s}"
        );
    }
}

/// **A(3), chromatic height 3 — dim 1024, built in a blink.** `A(3) = ⟨Sq¹,Sq²,Sq⁴,Sq⁸⟩` is the next rung
/// of the chromatic ladder above tmf. The fast echelon-BFS build (no more O(span²)) constructs all 1024
/// basis elements in a fraction of a second. Its one-line fingerprint carries `h_0, h_1, h_2, h_3` —
/// `Ext^{1,8} = 1` so it HAS `h_3` (unlike tmf), but `Ext^{1,16} = 0` so NO `h_4`. The exact A(3) signature.
#[test]
fn the_engine_lands_a3_height_three_dim_1024() {
    let a3 = build_subalgebra(&[vec![1], vec![2], vec![4], vec![8]], 72);
    assert_eq!(a3.dimension(), 1024, "A(3) is 1024-dimensional");

    let e = ext_over(&a3, 8, 16);
    assert_eq!(
        (e[1][1], e[1][2], e[1][4], e[1][8]),
        (1, 1, 1, 1),
        "one-line h_0,h_1,h_2,h_3 (degrees 1,2,4,8)"
    );
    assert_eq!(
        e[1][16], 0,
        "A(3) has NO h_4 — distinguishes it from the sphere"
    );
    for t in 3..=15 {
        if ![4, 8].contains(&t) {
            assert_eq!(e[1][t], 0, "only h_0..h_3 on the A(3) one-line (t={t})");
        }
    }
    for s in 0..=8 {
        assert_eq!(e[s][s], 1, "the infinite h_0-tower: π_0 = Z at (s,s)={s}");
    }
}

/// **A(4), chromatic height 4.** `A(4) = ⟨Sq¹,Sq²,Sq⁴,Sq⁸,Sq¹⁶⟩` — full dimension `2¹⁰ = 32768`. We build
/// it through degree 34 (enough to read the one-line and confirm `h_5` is absent at degree 32; the fast
/// pivot-indexed echelon build keeps even this large algebra well under a second). The one-line carries
/// `h_0, h_1, h_2, h_3, h_4` (degrees 1,2,4,8,16) — it HAS `h_4` (unlike A(3)) but `Ext^{1,32} = 0`, NO
/// `h_5`. The height-4 fingerprint, five rungs up the chromatic ladder from the sphere.
#[test]
#[ignore = "heavy (~15s): A(4) resolution to degree 32 over a 633-dim algebra — the height-4 crush, on demand"]
fn the_engine_lands_a4_height_four() {
    let a4 = build_subalgebra(&[vec![1], vec![2], vec![4], vec![8], vec![16]], 34);
    let e = ext_over(&a4, 6, 32);
    assert_eq!(
        (e[1][1], e[1][2], e[1][4], e[1][8], e[1][16]),
        (1, 1, 1, 1, 1),
        "one-line h_0..h_4 at degrees 1,2,4,8,16"
    );
    assert_eq!(
        e[1][32], 0,
        "A(4) has NO h_5 — it carries h_4 (unlike A(3)) and stops there"
    );
    for s in 0..=6 {
        assert_eq!(e[s][s], 1, "the infinite h_0-tower: π_0 = Z at (s,s)={s}");
    }
}
