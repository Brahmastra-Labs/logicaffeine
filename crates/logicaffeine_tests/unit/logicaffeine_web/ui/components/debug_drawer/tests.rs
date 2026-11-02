use super::*;

const PROG: &str = "## Main\n\nLet x be 6.\nLet y be 7.\nShow x + y.";

/// The datapath easter-egg renders the live register file from a real snapshot —
/// an SVG document with cells, and (when stepping) glow/wire classes for the op.
#[test]
fn datapath_svg_renders_from_a_real_snapshot() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.step();
    dbg.step();
    dbg.step();
    let snap = dbg.snapshot();
    let svg = datapath_svg(&snap);
    assert!(
        svg.starts_with("<svg"),
        "produces an SVG document: {svg:.40}"
    );
    assert!(svg.ends_with("</svg>"), "well-formed SVG");
    assert!(svg.contains("rcell"), "renders register cells");
    assert!(svg.contains("ALU"), "renders the engine box");
}

/// Type labels flow all the way through to the UI snapshot — the Variables/Stack
/// tabs read `reg.kind`, so a register holding `6` must report `Int`.
#[test]
fn snapshot_registers_carry_their_type() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let snap = dbg.snapshot();
    let kinds: Vec<&str> = snap
        .frames
        .last()
        .map(|f| f.registers.iter().map(|r| r.kind.as_str()).collect())
        .unwrap_or_default();
    assert!(
        kinds.contains(&"Int"),
        "an Int register is surfaced for the UI: {kinds:?}"
    );
    assert!(
        kinds.iter().all(|k| !k.is_empty()),
        "no register is left untyped: {kinds:?}"
    );
}

/// `esc` keeps SVG/HTML text injection-safe — register values are interpolated
/// into the datapath markup, so `<`/`&` must be entity-escaped.
#[test]
fn esc_escapes_markup_metacharacters() {
    assert_eq!(esc("a<b & c>d"), "a&lt;b &amp; c&gt;d");
}

/// The variable oscilloscope plots a labelled, typed lane per variable with a
/// playhead and held value boxes.
#[test]
fn timeline_svg_plots_each_variable() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let tl = dbg.variable_timeline();
    let (names, lanes) = timeline_svg(&tl);
    for svg in [&names, &lanes] {
        assert!(
            svg.starts_with("<svg") && svg.ends_with("</svg>"),
            "well-formed SVG"
        );
    }
    assert!(lanes.contains("tlph"), "renders a playhead");
    assert!(lanes.contains("tlbox"), "renders held value boxes");
    assert!(names.contains("x : Int"), "labels x's lane with its type");
    assert!(names.contains("y : Int"), "labels y's lane with its type");
}

/// The names column and the lanes beside it are one table: the same rows, the same height.
#[test]
fn timeline_names_line_up_with_their_lanes() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let (names, lanes) = timeline_svg(&dbg.variable_timeline());
    let height = |svg: &str| {
        svg.split("height='")
            .nth(1)
            .and_then(|r| r.split('\'').next())
            .map(str::to_string)
    };
    assert_eq!(
        height(&names),
        height(&lanes),
        "the names column and the lanes differ in height"
    );
    assert!(
        !lanes.contains("x : Int"),
        "lane names belong in the fixed column, not the scrolling lanes"
    );
}

/// The snapshot exposes a first-order-logic reading of the executing op for the
/// overlay line, and the live-proof bridge answers a breakpoint assertion.
#[test]
fn fol_overlay_and_live_proof_reach_the_ui() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    // Live proof: x < y is both true now and proven for every run.
    let r = dbg.assert_at_cursor("x < y");
    assert!(r.parsed && r.now == Some(true));
    assert_eq!(r.verdict, ProofVerdict::ProvenTrue);
    // FOL overlay: stepping surfaces a formula on the addition op.
    let mut dbg2 = Debugger::from_source(PROG).expect("compiles");
    let mut saw_fol = false;
    while dbg2.is_running() {
        if dbg2.snapshot().fol.contains("x + y") {
            saw_fol = true;
        }
        dbg2.step();
    }
    assert!(saw_fol, "the addition's FOL reaches the snapshot");
}

/// The Socratic teaching line reaches the UI — a guiding question that asks the
/// learner to predict the step's outcome.
#[test]
fn socratic_prompt_reaches_the_ui() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    let mut saw = false;
    while dbg.is_running() {
        let q = dbg.snapshot().socratic;
        if q.contains("sum") && q.ends_with('?') {
            saw = true;
        }
        dbg.step();
    }
    assert!(saw, "a Socratic question reaches the snapshot for the UI");
}

/// Proven invariants surface from the Oracle without stepping — x is statically
/// proven constant, with a finite range fact the UI can render.
#[test]
fn proven_invariants_are_available_to_the_ui() {
    let dbg = Debugger::from_source(PROG).expect("compiles");
    let proven = dbg.proven_invariants();
    let x = proven.iter().find(|p| p.name == "x").expect("x is proven");
    assert!(
        x.facts.iter().any(|f| f.contains("[6, 6]")),
        "x proven constant: {:?}",
        x.facts
    );
}

/// The provenance render shows the traced value and its full input lineage as a
/// structured tree.
#[test]
fn causal_html_renders_the_lineage() {
    let mut dbg = Debugger::from_source(PROG).expect("compiles");
    dbg.resume();
    let snap = dbg.snapshot();
    let sum = snap
        .frames
        .last()
        .unwrap()
        .registers
        .iter()
        .find(|r| r.value == "13")
        .unwrap()
        .index;
    let node = dbg.provenance(sum).expect("13 has provenance");
    let html = causal_html(&node);
    assert!(html.contains("= 13"), "shows the traced value: {html}");
    assert!(
        html.contains("= 6") && html.contains("= 7"),
        "shows the input lineage: {html}"
    );
    assert!(html.contains("cz-row"), "structured rows");
}
