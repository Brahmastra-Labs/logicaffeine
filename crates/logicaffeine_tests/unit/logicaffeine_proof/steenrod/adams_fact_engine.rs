use super::*;

/// The self-igniting harvester: from one resolution it auto-collects the whole low Adams chart — every
/// fact computed, none asserted. The landmark proofs that the ignition really fires:
///   * the ring relation `h_1³ = h_0²h_2` surfaces *by collision* — two product-paths (`h0·h0·h2` and
///     `h1·h1·h1`) reach the same `Ext^{3,6}` generator and the engine records their equality with no
///     prior knowledge that they coincide,
///   * the `Sq⁰` doubling `h0 ↦ h1` and the products `h0 · h2 = h0h2` are collected automatically,
///   * the exact 2-local stable stem `π_3 = Z/8` is read from the `h₀`-tower.
/// The full catalog is banked to `logs/derived_facts/adams_chart.txt`.
#[test]
fn the_engine_self_ignites_and_auto_collects_the_adams_chart() {
    let facts = harvest_adams_facts(4, 10);

    let has = |needle: &str| facts.iter().any(|f| f.to_string() == needle);
    assert!(
        has("h0h0h2 = h1h1h1"),
        "the ring relation h_1³ = h_0²h_2 was auto-discovered by collision"
    );
    assert!(
        has("Sq⁰(h0) = h1"),
        "the doubling h0 ↦ h1 was auto-collected"
    );
    assert!(
        has("Sq⁰(h0h2) = h1h3"),
        "the mixed-product doubling was auto-collected"
    );
    assert!(
        has("h0 · h2 = h0h2"),
        "the product h_0·h_2 was auto-collected"
    );
    assert!(
        has("h0 · h1 = 0"),
        "the Adem-adjacency vanishing h_0·h_1 = 0 was auto-collected"
    );
    assert!(
        has("π_3^s ⊗ Z₍₂₎ = Z/2^3"),
        "the exact 2-local stem π_3 = Z/8 was auto-collected"
    );
    assert!(
        facts.iter().any(|f| matches!(
            f,
            AdamsFact::StableGroup {
                stem: 0,
                truncated: true,
                ..
            }
        )),
        "π_0 = Z₍₂₎ is honestly flagged as a truncated/infinite h₀-tower, not a false finite group"
    );

    let relations = facts
        .iter()
        .filter(|f| matches!(f, AdamsFact::Relation { .. }))
        .count();
    let doublings = facts
        .iter()
        .filter(|f| matches!(f, AdamsFact::Doubling { .. }))
        .count();
    assert!(
        relations >= 1 && doublings >= 5,
        "engine nets ≥1 relation and ≥5 doublings"
    );
    assert!(
        facts.len() >= 30,
        "engine auto-collected ≥30 facts from one ignition: {}",
        facts.len()
    );

    // Bank the catalog to the repo (best-effort; the assertions above are the real verification).
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let body: String = facts.iter().map(|f| format!("{f}\n")).collect();
        let _ = std::fs::write(dir.join("adams_chart.txt"), body);
    }
}

/// Lock-in: the full `(4,10)` catalog is frozen exactly. Any change to the resolution, the ring product,
/// the diagonal, the doubling, or the stable-group reader that perturbs a single derived fact fails here.
/// This is the banked ground truth of the symmetry-breaking engine.
#[test]
fn the_harvested_catalog_is_frozen_as_ground_truth() {
    let got: Vec<String> = harvest_adams_facts(4, 10)
        .iter()
        .map(|f| f.to_string())
        .collect();
    let expected = "\
dim Ext^{0,0} = 1
dim Ext^{1,1} = 1
dim Ext^{1,2} = 1
dim Ext^{1,4} = 1
dim Ext^{1,8} = 1
dim Ext^{2,2} = 1
dim Ext^{2,4} = 1
dim Ext^{2,5} = 1
dim Ext^{2,8} = 1
dim Ext^{2,9} = 1
dim Ext^{2,10} = 1
dim Ext^{3,3} = 1
dim Ext^{3,6} = 1
dim Ext^{3,10} = 1
dim Ext^{4,4} = 1
h0 · h0 = h0h0
h0 · h2 = h0h2
h0 · h3 = h0h3
h0h0 · h0 = h0h0h0
h0h0 · h2 = h0h0h2
h0h0 · h3 = h0h0h3
h0h0h0 · h0 = h0h0h0h0
h1 · h1 = h1h1
h1 · h3 = h1h3
h2 · h2 = h2h2
h0 · h1 = 0
h0h0 · h1 = 0
h0h0h0 · h1 = 0
h0h0h0 · h2 = 0
h0h0h2 · h0 = 0
h0h0h2 · h1 = 0
h0h0h2 · h2 = 0
h0h2 · h1 = 0
h0h2 · h2 = 0
h1 · h0 = 0
h1 · h2 = 0
h1h1 · h0 = 0
h1h1 · h2 = 0
h2 · h1 = 0
h2h2 · h0 = 0
h2h2 · h1 = 0
h0h0h2 = h0h2h0
h0h0h2 = h1h1h1
h0h0h3 = h0h3h0
h0h2 = h2h0
h0h3 = h3h0
h1h3 = h3h1
Sq⁰(h0) = h1
Sq⁰(h0h0) = h1h1
Sq⁰(h0h2) = h1h3
Sq⁰(h1) = h2
Sq⁰(h1h1) = h2h2
Sq⁰(h2) = h3
π_0^s ⊗ Z₍₂₎ ⊇ Z/2^5 (h₀-tower hits the resolution ceiling — infinite/truncated)
π_1^s ⊗ Z₍₂₎ = Z/2^1
π_2^s ⊗ Z₍₂₎ = Z/2^1
π_3^s ⊗ Z₍₂₎ = Z/2^3
π_6^s ⊗ Z₍₂₎ = Z/2^1";
    assert_eq!(
        got.join("\n"),
        expected,
        "the banked Adams-chart catalog drifted from ground truth"
    );
}

/// The un-auto-able boundary, found by the engine: extend the range and the product-sweep leaves a
/// generator it cannot name. The first such class is `c₀` at `Ext^{3,11}` (stem 8) — the first Ext class
/// that is NOT a product of the indecomposables `h_i`. This is exactly where the secondary (Massey)
/// structure begins; the primary symmetry breaking provably runs out here.
#[test]
fn the_first_un_auto_able_class_is_c0_at_ext_3_11() {
    let secondary = secondary_classes(6, 12);
    assert!(
        !secondary.is_empty(),
        "extending the range must expose un-auto-able classes"
    );
    assert_eq!(
        secondary[0],
        (3, 11),
        "the first non-product class is c₀ at Ext^{{3,11}}: {secondary:?}"
    );
    // and it really is invisible at the smaller range the rest of the engine runs at
    assert!(
        secondary_classes(4, 10).is_empty(),
        "no non-product classes appear within the (4,10) window"
    );
}

/// Auto-collect the WIDE frontier by pure symmetry breaking (no proving): products, the relations they
/// surface by collision, the un-auto-able secondary classes, and the stable stems — gathered to a larger
/// range than the doubling-bearing catalog can afford. The engine gathers the secondary frontier itself:
/// `c₀ = Ext^{3,11}` is auto-collected as a `Secondary` fact, and the whole catalog is banked.
#[test]
fn the_engine_auto_collects_the_wide_frontier_including_the_secondary_classes() {
    let facts = harvest_secondary_facts(6, 13);
    let has = |needle: &str| facts.iter().any(|f| f.to_string() == needle);

    // The auto-discovered secondary frontier — gathered, not proven.
    assert!(
        has("Ext^{3,11} has a non-product (secondary/Massey) class — un-auto-able by the product sweep"),
        "c₀ at Ext^{{3,11}} auto-collected as a Secondary fact"
    );
    let secondary_facts: Vec<&AdamsFact> = facts
        .iter()
        .filter(|f| matches!(f, AdamsFact::Secondary { .. }))
        .collect();
    assert!(
        !secondary_facts.is_empty(),
        "the frontier auto-collected ≥1 secondary class"
    );
    assert!(
        matches!(secondary_facts[0], AdamsFact::Secondary { s: 3, t: 11 }),
        "the first secondary class is c₀ at Ext^{{3,11}}"
    );

    // BROKEN TO THE NEXT GROUP: c₀ is seeded as a fresh indecomposable and the verified product symmetry
    // sweeps onward from it — no Massey computation. The sweep reaches the OTHER detected secondary class
    // as c₀·h₁ (self-consistent with the independent detection), and derives the chart fact h₀·c₀ = 0.
    assert!(
        has("c0 · h1 = c0h1"),
        "the product symmetry broke into the c₀-family automatically"
    );
    assert!(
        has("c0 · h0 = 0"),
        "h₀·c₀ = 0 auto-derived by the verified product, not proven by hand"
    );

    // Still gathering the primary structure too (the relation discovered by collision, the stable stems).
    assert!(
        has("h0h0h2 = h1h1h1"),
        "the ring relation is still auto-collected at the wide range"
    );
    assert!(has("π_3^s ⊗ Z₍₂₎ = Z/2^3"), "π_3 = Z/8 auto-collected");
    assert!(
        facts.len() >= 60,
        "wide auto-collection gathered ≥60 facts: {}",
        facts.len()
    );

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let body: String = facts.iter().map(|f| format!("{f}\n")).collect();
        let _ = std::fs::write(dir.join("adams_frontier.txt"), body);
    }
}

/// Symmetry-breaking the secondary generators out of the shadows: a deeper sweep detects and names FOUR
/// indecomposables the product structure cannot reach — `Ext^{3,11}`, `Ext^{5,14}`, `Ext^{5,16}`,
/// `Ext^{4,18}` (classically `c₀, Ph₁, Ph₂, d₀`, though the engine only knows them as detected
/// generators) — then the verified product symmetry sweeps each family and even surfaces a relation
/// ACROSS two of the new families by collision (`c1·h1·h1 = c2·h0·h0`). All auto-collected, none proven.
#[test]
fn the_engine_breaks_the_secondary_families_out_of_the_shadows() {
    let secondary = secondary_classes(9, 18);
    for bidegree in [(3, 11), (5, 14), (5, 16), (4, 18)] {
        assert!(
            secondary.contains(&bidegree),
            "detected the secondary generator at Ext^{bidegree:?}: {secondary:?}"
        );
    }

    let facts = harvest_secondary_facts(9, 18);
    let has = |needle: &str| facts.iter().any(|f| f.to_string() == needle);

    // Each new family is swept by the verified product symmetry.
    assert!(has("c1 · h1 = c1h1"), "the c1 family auto-sweeps");
    assert!(has("c2 · h0 = c2h0"), "the c2 family auto-sweeps");
    // A relation ACROSS two newly-broken-out families, discovered by product-path collision.
    assert!(
        has("c1h1h1 = c2h0h0"),
        "a cross-family relation auto-discovered by collision"
    );
    // Real chart vanishings on c₀, auto-derived.
    assert!(
        has("c0 · h0 = 0") && has("c0 · h2 = 0"),
        "h₀·c₀ = h₂·c₀ = 0 auto-derived"
    );

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let body: String = facts.iter().map(|f| format!("{f}\n")).collect();
        let _ = std::fs::write(dir.join("adams_frontier_deep.txt"), body);
    }
}
