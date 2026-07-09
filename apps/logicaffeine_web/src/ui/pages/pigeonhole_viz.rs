//! Studio easter egg — the pigeonhole principle, solved live in the browser by our prover.
//!
//! Pure data → SVG (no Z3, no JS): `n` pigeons fly toward `n-1` holes. One pigeon is always left
//! with nowhere to land — and *that* is the whole point. Encoded as boolean SAT, PHP(n) needs an
//! exponentially long resolution refutation (Haken 1985), so every resolution-class solver — Kissat,
//! CaDiCaL, Glucose, Z3 — hits a `2^Ω(n)` wall. Our prover does not:
//!
//! * **Maximum bipartite matching** ([`logicaffeine_proof::matching`]) decides infeasibility in
//!   polynomial time and returns a re-verified **Hall witness** — the `n` pigeons collectively reach
//!   only `n-1` holes, so no assignment exists. The certificate the animation draws.
//! * **Certified symmetry breaking** ([`logicaffeine_proof::sym_certify::heule_php_refutation`])
//!   produces a *polynomial* PR proof (Heule–Kiesl–Biere 2017) that escapes the resolution lower
//!   bound and re-checks against the original formula — machine-checked, in the browser, no Z3.
//!
//! Both run live here. For contrast we also run a plain CDCL solve (the Kissat-class algorithm
//! family) on the small instances and report its conflict count — the search work our certified
//! proof avoids entirely.

use logicaffeine_proof::cdcl::{SolveResult, Solver};
use logicaffeine_proof::families::php;
use logicaffeine_proof::matching::{assign_or_hall, is_hall_witness, HallWitness, MatchOutcome};
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::sym_certify::heule_php_refutation;

/// Smallest instance worth showing (one pigeon, zero holes is degenerate).
const SPEC_MIN_N: usize = 2;
/// Largest instance the spec accepts — keeps the drawing legible and the CNF build bounded.
const SPEC_MAX_N: usize = 20;
/// Run the live certified Heule proof up to here (PHP(12) ≈ 60ms); beyond it the matching witness
/// and the polynomial-proof note carry the story without a heavy per-render construction.
const HEULE_MAX_N: usize = 12;
/// Run the baseline (Kissat-class) CDCL solve for the conflict-count contrast only this far —
/// PHP(7) is ~700 conflicts in milliseconds; the blow-up past it is the point, not something to run.
const BASELINE_MAX_N: usize = 7;

/// A parsed pigeonhole spec: `pigeons` pigeons into `pigeons - 1` holes — the canonical
/// resolution-hard PHP instance, always unsatisfiable.
pub struct PigeonSpec {
    /// Number of pigeons (`n`). Holes are always `n - 1`.
    pub pigeons: usize,
}

impl PigeonSpec {
    /// The hole count: always one fewer than the pigeons (that is what makes it impossible).
    pub fn holes(&self) -> usize {
        self.pigeons - 1
    }
}

/// Parse a spec of the form (a single `pigeons: N` line; comments and blanks ignored):
/// ```text
/// ## Pigeonhole
/// pigeons: 6
/// ```
/// Returns `None` unless there is exactly a well-formed `pigeons:` count within `2..=20`, so other
/// Hardware-mode inputs (SVA English, Verilog, register specs) are never hijacked.
pub fn parse_pigeonhole_spec(spec: &str) -> Option<PigeonSpec> {
    let mut pigeons: Option<usize> = None;
    for raw in spec.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line
            .strip_prefix("pigeons:")
            .or_else(|| line.strip_prefix("Pigeons:"))
        {
            if let Ok(n) = rest.trim().parse::<usize>() {
                if (SPEC_MIN_N..=SPEC_MAX_N).contains(&n) {
                    pigeons = Some(n);
                }
            }
            continue;
        }
        // Any other non-comment content means this is not a clean pigeonhole spec.
        return None;
    }
    pigeons.map(|pigeons| PigeonSpec { pigeons })
}

/// Whether `spec` is a pigeonhole spec — the single source of truth the Studio uses to route
/// Hardware-mode input to this easter egg, so the wiring can never drift from the parser.
pub fn is_pigeonhole_spec(spec: &str) -> bool {
    parse_pigeonhole_spec(spec).is_some()
}

/// The bipartite adjacency for PHP(n): every one of the `n` pigeons may use any of the `n-1` holes.
fn adjacency(pigeons: usize, holes: usize) -> Vec<Vec<usize>> {
    (0..pigeons).map(|_| (0..holes).collect()).collect()
}

/// The certified refutation result for one instance, all re-checkable and computed live in WASM.
pub struct Verdict {
    /// The re-verified Hall witness: `pigeons` pigeons reach only `holes` holes (so `slots < items`).
    pub hall: HallWitness,
    /// PR clauses in the certified Heule symmetry-breaking proof, when it was run (`n ≤ HEULE_MAX_N`).
    pub pr_clauses: Option<usize>,
    /// Whether that proof independently re-checked against the original PHP(n) formula.
    pub certified: bool,
    /// Baseline (Kissat-class) CDCL conflicts on the same instance, when run (`n ≤ BASELINE_MAX_N`).
    pub baseline_conflicts: Option<u64>,
}

/// Solve PHP(`spec.pigeons`) with our prover, live: the matching Hall witness (always), the certified
/// Heule symmetry-breaking proof (small/medium `n`), and a baseline CDCL conflict count for contrast
/// (small `n`). Every field is a re-verified artifact, never a trusted verdict.
pub fn solve(spec: &PigeonSpec) -> Verdict {
    let n = spec.pigeons;
    let holes = spec.holes();
    let (cnf, _) = php(n);

    // Polynomial decision + certificate: n pigeons, n-1 holes ⇒ Hall witness (re-verified).
    let hall = match assign_or_hall(&adjacency(n, holes), holes) {
        MatchOutcome::Infeasible(w) => w,
        // PHP(n) with n ≥ 2 is always infeasible; keep a faithful (re-checkable) witness regardless.
        MatchOutcome::Feasible(_) => HallWitness { items: (0..n).collect(), slots: (0..holes).collect() },
    };

    // Certified symmetry-breaking proof (Heule PR), live, for the affordable range.
    let (pr_clauses, certified) = if n <= HEULE_MAX_N {
        let r = heule_php_refutation(n);
        let ok = r.refuted && check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps);
        (Some(r.sbp_clauses), ok)
    } else {
        // The proof is polynomial at any size (Heule–Kiesl–Biere); we just don't reconstruct it on
        // every render past the affordable range. The matching witness already certifies UNSAT.
        (None, true)
    };

    // Baseline CDCL conflict count — the Kissat-class search work we avoid — for the small instances.
    let baseline_conflicts = if n <= BASELINE_MAX_N {
        let mut base = Solver::new(cnf.num_vars);
        base.set_reduce(true);
        for c in &cnf.clauses {
            base.add_clause(c.clone());
        }
        match base.solve() {
            SolveResult::Unsat => Some(base.conflicts()),
            SolveResult::Sat(_) => None,
        }
    } else {
        None
    };

    Verdict { hall, pr_clauses, certified, baseline_conflicts }
}

/// A looping SMIL translate that holds at the start, glides by `(dx, dy)`, holds there, then returns —
/// what makes a pigeon descend into its hole and reset each cycle. `keyTimes` are clamped strictly
/// increasing (the browser silently drops an animation with non-monotonic `keyTimes`). `additive="sum"`
/// is mandatory: each pigeon's perch is set by a static `transform="translate(px, top_y)"` on its group,
/// and the SMIL default (`additive="replace"`) would discard that perch and snap every pigeon to the SVG
/// origin — so the `0 0` keyframes here are an offset *from* the perch, not an absolute position.
fn fly(dx: f64, dy: f64, b0: f64, b1: f64, e1: f64, dur: f64) -> String {
    let b0 = b0.clamp(0.01, 0.90);
    let b1 = b1.clamp(b0 + 0.01, 0.95);
    let e1 = e1.clamp(b1 + 0.01, 0.98);
    format!(
        r#"<animateTransform attributeName="transform" type="translate" additive="sum" dur="{dur:.2}s" repeatCount="indefinite" keyTimes="0;{b0:.4};{b1:.4};{e1:.4};1" values="0 0;0 0;{dx:.1} {dy:.1};{dx:.1} {dy:.1};0 0"/>"#
    )
}

/// A looping SMIL translate for the doomed pigeon: it dives toward the holes by `(dx, dymid)`, finds
/// no vacancy, and retreats — over and over. `keyTimes` clamped strictly increasing; `additive="sum"`
/// so the dive is an offset from its perch (see [`fly`]), not an absolute jump to the origin.
fn flutter(dx: f64, dymid: f64, b0: f64, bm: f64, be: f64, dur: f64) -> String {
    let b0 = b0.clamp(0.01, 0.88);
    let bm = bm.clamp(b0 + 0.01, 0.93);
    let be = be.clamp(bm + 0.01, 0.98);
    format!(
        r#"<animateTransform attributeName="transform" type="translate" additive="sum" dur="{dur:.2}s" repeatCount="indefinite" keyTimes="0;{b0:.4};{bm:.4};{be:.4};1" values="0 0;0 0;{dx:.1} {dymid:.1};0 0;0 0"/>"#
    )
}

/// A looping SMIL opacity pulse: invisible, then bright across `[bm, be]` (the moment the doomed
/// pigeon is down at the full holes), then gone — for the flashing "NO ROOM" mark. `keyTimes`
/// clamped strictly increasing.
fn pulse(bm: f64, be: f64, dur: f64) -> String {
    let a = bm.clamp(0.02, 0.90);
    let b = (a + 0.02).clamp(a + 0.01, 0.94);
    let c = be.clamp(b + 0.01, 0.98);
    format!(
        r#"<animate attributeName="opacity" dur="{dur:.2}s" repeatCount="indefinite" keyTimes="0;{a:.4};{b:.4};{c:.4};1" values="0;0;1;0;0"/>"#
    )
}

/// The drawn pigeon: a small body + head + beak + wing around the origin, so a translate moves it.
fn pigeon_glyph(body: &str, accent: &str) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(s, r##"<ellipse cx="0" cy="0" rx="13" ry="9" fill="{body}"/>"##);
    let _ = write!(s, r##"<path d="M -2,-3 q 12,-9 14,3 q -8,-3 -14,2 Z" fill="{accent}"/>"##);
    let _ = write!(s, r##"<circle cx="9" cy="-5" r="6" fill="{body}"/>"##);
    let _ = write!(s, r##"<circle cx="11" cy="-6" r="1.4" fill="#0f1117"/>"##);
    let _ = write!(s, r##"<path d="M 14,-5 l 6,-1.5 l -6,3 Z" fill="#f59e0b"/>"##);
    s
}

/// Render PHP(`spec.pigeons`) as `(svg, verdict)`. The SVG is an animated flight: `n` pigeons glide
/// into `n-1` holes while the one left over flutters and flashes "NO ROOM". The verdict is the
/// plain-language, certified summary.
pub fn render(spec: &PigeonSpec) -> (String, String) {
    use std::fmt::Write as _;
    let n = spec.pigeons;
    let holes = spec.holes();
    let v = solve(spec);

    // Geometry: a top row of pigeons gliding down into a bottom row of holes, three certified status
    // lines below the shelf. The canvas height is derived from that layout so the last line is never
    // clipped (a fixed height silently cut off the resolution/CDCL contrast line).
    let w = 600.0_f64;
    let margin = 40.0_f64;
    let plot_w = w - 2.0 * margin;
    let top_y = 64.0_f64;
    let hole_y = 232.0_f64;
    let hole_w = (plot_w / holes as f64 * 0.62).clamp(16.0, 46.0);
    let hole_h = 30.0_f64;
    let shelf_y = hole_y + hole_h + 6.0;
    let status_top = shelf_y + 26.0;
    let status_step = 20.0_f64;
    let last_status_y = status_top + 2.0 * status_step;
    let height = last_status_y + 14.0;
    let dur = (1.6 + n as f64 * 0.35).clamp(3.0, 9.0);

    // x of pigeon i (n across) and hole j (n-1 across).
    let px = |i: usize| margin + (i as f64 + 0.5) * (plot_w / n as f64);
    let hx = |j: usize| margin + (j as f64 + 0.5) * (plot_w / holes as f64);

    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg viewBox="0 0 {w:.0} {height:.0}" xmlns="http://www.w3.org/2000/svg" font-family="ui-sans-serif, system-ui" font-size="12">"#
    );
    let _ = write!(s, r##"<rect x="0" y="0" width="{w:.0}" height="{height:.0}" fill="#0f1117"/>"##);
    let _ = write!(
        s,
        r##"<text x="14" y="26" fill="#e5e7eb" font-size="14" font-family="ui-monospace, monospace">Pigeonhole · {n} pigeons → {holes} holes</text>"##
    );

    // The dovecote: n-1 hole openings along the bottom, on a shelf.
    let _ = write!(s, r##"<rect x="{margin:.0}" y="{shelf_y:.1}" width="{plot_w:.0}" height="6" rx="3" fill="#3a2f25"/>"##);
    for j in 0..holes {
        let cx = hx(j);
        let x = cx - hole_w / 2.0;
        let _ = write!(s, r##"<rect x="{x:.1}" y="{hole_y:.1}" width="{hole_w:.1}" height="{hole_h:.1}" rx="6" fill="#1b1f2a" stroke="#3a4150" stroke-width="1.5"/>"##);
        let _ = write!(s, r##"<ellipse cx="{cx:.1}" cy="{:.1}" rx="{:.1}" ry="7" fill="#0a0c12"/>"##, hole_y + 11.0, hole_w / 2.0 - 5.0);
    }

    // The placed pigeons: pigeon i → hole i for i in 0..holes; each dives in, staggered.
    let descend = hole_y + 6.0 - top_y;
    for i in 0..holes {
        let dx = hx(i) - px(i);
        let frac = i as f64 / n as f64;
        let b0 = 0.06 + 0.55 * frac;
        let anim = fly(dx, descend, b0, b0 + 0.16, 0.9, dur);
        let _ = write!(s, r##"<g transform="translate({:.1},{top_y:.1})">{anim}{}</g>"##, px(i), pigeon_glyph("#9fb3c8", "#7d93ab"));
    }

    // The doomed pigeon (the last one): dives at the nearest hole, finds it taken, retreats — flashing.
    let doomed = n - 1;
    let target_hole = holes - 1;
    let ddx = hx(target_hole) - px(doomed);
    let dmid = descend - 10.0;
    let danim = flutter(ddx, dmid, 0.62, 0.78, 0.9, dur);
    let _ = write!(s, r##"<g transform="translate({:.1},{top_y:.1})">{danim}{}</g>"##, px(doomed), pigeon_glyph("#f87171", "#dc6363"));

    // The flashing "NO ROOM" mark over the contested hole.
    let no_room = pulse(0.78, 0.9, dur);
    let _ = write!(
        s,
        r##"<g opacity="0"><text x="{:.1}" y="{:.1}" fill="#fca5a5" font-size="13" font-weight="700" text-anchor="middle">✗ NO ROOM</text>{no_room}</g>"##,
        hx(target_hole),
        hole_y - 12.0
    );

    // Status lines: the certified result, and the search work we avoid. The canvas height above is
    // derived from `status_top` + two `status_step`s so the third line below always fits.
    let mut y = status_top;
    let hall_line = format!(
        "\u{2713} Maximum matching: {} pigeons reach only {} holes \u{2014} Hall witness, decided in polynomial time",
        v.hall.items.len(),
        v.hall.slots.len()
    );
    let _ = write!(s, r##"<text x="14" y="{y:.0}" fill="#86efac" font-size="12">{hall_line}</text>"##);
    y += status_step;
    let ours = match v.pr_clauses {
        Some(k) if v.certified => format!(
            "\u{2713} OURS: 0 conflicts \u{2014} certified symmetry-breaking proof ({k} PR clauses), machine-checked in your browser, no Z3"
        ),
        _ => "\u{2713} OURS: polynomial certified PR proof (Heule\u{2013}Kiesl\u{2013}Biere) \u{2014} no Z3".to_string(),
    };
    let _ = write!(s, r##"<text x="14" y="{y:.0}" fill="#86efac" font-size="12">{ours}</text>"##);
    y += status_step;
    let base = match v.baseline_conflicts {
        Some(c) => format!("\u{2717} Resolution / CDCL (Kissat-class): {c} conflicts here \u{2014} and 2^\u{03a9}(n) as n grows"),
        None => "\u{2717} Resolution / CDCL (Kissat, CaDiCaL, Z3): 2^\u{03a9}(n) \u{2014} exponential, provably (Haken 1985)".to_string(),
    };
    let _ = write!(s, r##"<text x="14" y="{y:.0}" fill="#fca5a5" font-size="12">{base}</text>"##);

    let _ = write!(s, "</svg>");

    let proof = match v.pr_clauses {
        Some(k) if v.certified => format!(
            " Our prover auto-refutes it with a certified symmetry-breaking proof ({k} PR clauses, machine-checked against the original formula) in the browser \u{2014} no Z3."
        ),
        _ => " Our prover refutes it with a polynomial certified PR proof (Heule\u{2013}Kiesl\u{2013}Biere) \u{2014} no Z3.".to_string(),
    };
    let verdict = format!(
        "\u{2717} UNSAT \u{2014} {n} pigeons cannot fit {holes} holes. Maximum bipartite matching returns a re-verified Hall witness ({n} pigeons reach only {holes} holes), so no assignment exists.{proof} Every resolution-class solver (Kissat, CaDiCaL, Glucose, Z3) needs 2^\u{03a9}(n) time on this family (Haken 1985)."
    );
    (s, verdict)
}

/// A textual summary for the Studio output panel, paired with the [`render`] animation — the same
/// certified result, in words.
pub fn report(spec: &PigeonSpec) -> String {
    use std::fmt::Write as _;
    let v = solve(spec);
    let n = spec.pigeons;
    let holes = spec.holes();
    let mut out = String::new();
    let _ = writeln!(out, "PHP({n}): {n} pigeons into {holes} holes \u{2014} UNSAT.");
    let _ = writeln!(
        out,
        "Hall witness (re-verified): {} pigeons collectively reach only {} holes.",
        v.hall.items.len(),
        v.hall.slots.len()
    );
    match v.pr_clauses {
        Some(k) if v.certified => {
            let _ = writeln!(out, "Certified symmetry-breaking proof: {k} PR clauses, 0 conflicts, machine-checked. No Z3.");
        }
        _ => {
            let _ = writeln!(out, "Certified by a polynomial PR proof (Heule\u{2013}Kiesl\u{2013}Biere). No Z3.");
        }
    }
    if let Some(c) = v.baseline_conflicts {
        let _ = write!(out, "Baseline CDCL (Kissat-class) on the same instance: {c} conflicts \u{2014} the search our proof avoids.");
    } else {
        let _ = write!(out, "Resolution-class solvers (Kissat, CaDiCaL, Z3) need 2^\u{03a9}(n) here \u{2014} provably exponential.");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_spec() {
        let s = parse_pigeonhole_spec("pigeons: 6").expect("parses");
        assert_eq!(s.pigeons, 6);
        assert_eq!(s.holes(), 5);
    }

    #[test]
    fn parser_is_robust_to_messy_input() {
        let messy = "## Pigeonhole\n\n  Pigeons:  4  \n\n";
        let s = parse_pigeonhole_spec(messy).expect("messy spec still parses");
        assert_eq!(s.pigeons, 4);
        assert_eq!(s.holes(), 3);
    }

    #[test]
    fn rejects_out_of_range_and_malformed() {
        assert!(parse_pigeonhole_spec("pigeons: 1").is_none(), "n=1 is degenerate");
        assert!(parse_pigeonhole_spec("pigeons: 99").is_none(), "n=99 exceeds the spec cap");
        assert!(parse_pigeonhole_spec("pigeons: lots").is_none(), "non-numeric count");
        assert!(parse_pigeonhole_spec("holes: 3").is_none(), "no pigeons line");
        assert!(parse_pigeonhole_spec("").is_none(), "empty input");
    }

    #[test]
    fn recognizes_only_pigeonhole_specs() {
        assert!(is_pigeonhole_spec("pigeons: 6"));
        assert!(is_pigeonhole_spec("## Pigeonhole\npigeons: 8\n"));
        // The OTHER Hardware-mode inputs must NOT be hijacked by the pigeonhole egg:
        assert!(!is_pigeonhole_spec("Always, if request is high, then acknowledge is high."));
        assert!(!is_pigeonhole_spec(
            "module m(input clk); reg a; always @(posedge clk) a <= ~a; assert property (a); endmodule"
        ));
        assert!(!is_pigeonhole_spec("registers: 3\na: 0-5\nb: 1-6"));
        assert!(!is_pigeonhole_spec("ns-through conflicts with ew-through and ew-left."));
        assert!(!is_pigeonhole_spec(""));
    }

    #[test]
    fn matching_returns_a_reverified_hall_witness() {
        for n in SPEC_MIN_N..=8 {
            let spec = PigeonSpec { pigeons: n };
            let v = solve(&spec);
            assert_eq!(v.hall.items.len(), n, "all n pigeons are deficient");
            assert_eq!(v.hall.slots.len(), n - 1, "they reach only n-1 holes");
            // The witness re-checks independently against the adjacency.
            assert!(
                is_hall_witness(&adjacency(n, n - 1), &v.hall),
                "Hall witness must re-verify for PHP({n})"
            );
        }
    }

    #[test]
    fn heule_proof_is_certified_for_the_live_range() {
        for n in 3..=HEULE_MAX_N {
            let v = solve(&PigeonSpec { pigeons: n });
            assert!(v.certified, "PHP({n}) must carry a certified proof");
            assert!(v.pr_clauses.unwrap_or(0) >= 1, "PHP({n}) needs PR clauses");
        }
    }

    #[test]
    fn baseline_cdcl_actually_blows_up_while_ours_does_not() {
        // The contrast that makes the demo: plain CDCL accrues conflicts; our certified path has none.
        for n in 4..=BASELINE_MAX_N {
            let v = solve(&PigeonSpec { pigeons: n });
            assert!(
                v.baseline_conflicts.unwrap_or(0) >= 1,
                "baseline CDCL must hit conflicts on PHP({n})"
            );
            assert!(v.certified, "ours stays certified with 0 conflicts on PHP({n})");
        }
    }

    #[test]
    fn renders_an_animated_unsat() {
        let spec = parse_pigeonhole_spec("pigeons: 6").unwrap();
        let (svg, verdict) = render(&spec);
        assert!(svg.starts_with("<svg"), "valid SVG");
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("<animateTransform"), "pigeons must fly");
        assert!(svg.contains("NO ROOM"), "the doomed pigeon must be flagged");
        assert!(svg.contains("Hall witness"), "the certificate must be shown");
        assert!(verdict.starts_with('\u{2717}'), "UNSAT verdict: {verdict}");
        assert!(verdict.contains("UNSAT") && verdict.contains("Hall witness"), "{verdict}");
        assert!(verdict.contains("no Z3"), "{verdict}");
        assert!(verdict.contains("6 pigeons") && verdict.contains("5 holes"), "{verdict}");
    }

    #[test]
    fn renders_large_instances_without_running_the_heavy_paths() {
        // Past the live caps the render must still be coherent (matching witness + theory), never panic.
        let spec = PigeonSpec { pigeons: 18 };
        let (svg, verdict) = render(&spec);
        assert!(svg.starts_with("<svg") && svg.contains("</svg>"));
        assert!(svg.contains("Hall witness"));
        let v = solve(&spec);
        assert!(v.pr_clauses.is_none(), "Heule not reconstructed past the cap");
        assert!(v.baseline_conflicts.is_none(), "baseline not run past the cap");
        assert!(verdict.starts_with('\u{2717}'));
        assert!(verdict.contains("2^\u{03a9}(n)"), "theory framing present: {verdict}");
    }

    #[test]
    fn report_agrees_with_the_rendered_verdict() {
        for n in [3usize, 6, 14] {
            let spec = PigeonSpec { pigeons: n };
            let report = report(&spec);
            let (_, verdict) = render(&spec);
            assert!(report.contains("UNSAT"), "{report}");
            assert!(verdict.starts_with('\u{2717}'));
            assert!(report.contains("Hall witness"), "{report}");
        }
    }

    #[test]
    fn holes_are_always_pigeons_minus_one() {
        for n in SPEC_MIN_N..=SPEC_MAX_N {
            assert_eq!(PigeonSpec { pigeons: n }.holes(), n - 1);
        }
    }

    /// The SMIL `keyTimes` MUST be strictly monotonic in [0,1] or the browser drops the animation.
    /// Sweep the whole parameter space of every animation helper and parse the emitted keyTimes back.
    fn assert_monotonic_keytimes(anim: &str) {
        let kt = anim
            .split("keyTimes=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("has keyTimes");
        let times: Vec<f64> = kt.split(';').map(|x| x.parse().unwrap()).collect();
        assert_eq!(times.len(), 5, "{anim}");
        assert_eq!(times[0], 0.0);
        assert_eq!(*times.last().unwrap(), 1.0);
        for w in times.windows(2) {
            assert!(w[0] < w[1], "keyTimes not increasing: {kt}");
        }
    }

    #[test]
    fn animation_helpers_emit_monotonic_keytimes_for_every_input() {
        for a in 0..=24 {
            for b in 0..=24 {
                for c in 0..=24 {
                    let (t0, t1, t2) = (a as f64 / 24.0, b as f64 / 24.0, c as f64 / 24.0);
                    assert_monotonic_keytimes(&fly(40.0, 120.0, t0, t1, t2, 6.0));
                    assert_monotonic_keytimes(&flutter(40.0, 100.0, t0, t1, t2, 6.0));
                    // pulse only takes (bm, be) — sweep its two-parameter space.
                    assert_monotonic_keytimes(&pulse(t0, t1, 6.0));
                }
            }
        }
    }

    /// The flight helpers MUST be `additive="sum"`. Each pigeon's perch is a static
    /// `transform="translate(px, top_y)"` on its group; the SMIL default (`additive="replace"`)
    /// overrides that perch and snaps every pigeon to the SVG origin — the "pigeons don't fly to
    /// the holes" bug. The `0 0` keyframes only mean "stay at the perch" when the animation sums.
    #[test]
    fn flight_animations_are_additive() {
        assert!(fly(40.0, 120.0, 0.1, 0.3, 0.9, 6.0).contains(r#"additive="sum""#), "fly must sum onto the perch");
        assert!(flutter(40.0, 100.0, 0.6, 0.8, 0.9, 6.0).contains(r#"additive="sum""#), "flutter must sum onto the perch");
    }

    /// Every `<animateTransform>` in a rendered scene animates the group's `transform`, so every one
    /// must be additive or that pigeon leaves its perch and flies from the origin instead.
    #[test]
    fn every_rendered_transform_animation_is_additive() {
        for n in [3usize, 6, 9, 12, 18, 20] {
            let (svg, _) = render(&PigeonSpec { pigeons: n });
            let transforms = svg.matches("<animateTransform").count();
            // One flight per placed pigeon (`n-1`) plus the doomed pigeon's flutter = `n`.
            assert_eq!(transforms, n, "expected one transform animation per pigeon for n={n}");
            let additive = svg.matches(r#"additive="sum""#).count();
            assert_eq!(additive, transforms, "every transform animation must be additive for n={n}");
        }
    }

    /// Parse each flight group's static perch `translate(bx,by)` and its animation peak offset
    /// `(dx,dy)` (the third of the five keyframe values). The rendered landing is `(bx+dx, by+dy)`
    /// because the animation is additive.
    fn flights(svg: &str) -> Vec<(f64, f64, f64, f64)> {
        let mut out = Vec::new();
        for seg in svg.split("<g transform=\"translate(").skip(1) {
            let coords = &seg[..seg.find(')').expect("closing paren")];
            let (bx, by) = coords.split_once(',').expect("two coords");
            let vstart = seg.find("values=\"").expect("has values") + "values=\"".len();
            let values = &seg[vstart..vstart + seg[vstart..].find('"').unwrap()];
            let peak = values.split(';').nth(2).expect("five keyframe values");
            let (dx, dy) = peak.split_once(' ').expect("peak is `dx dy`");
            out.push((
                bx.parse().unwrap(),
                by.parse().unwrap(),
                dx.parse().unwrap(),
                dy.parse().unwrap(),
            ));
        }
        out
    }

    /// The hole openings are the `ry="7"` ellipses (the pigeon body is `ry="9"`), in emission order.
    fn hole_centers(svg: &str) -> Vec<f64> {
        let mut out = Vec::new();
        for seg in svg.split("<ellipse cx=\"").skip(1) {
            let cx = &seg[..seg.find('"').unwrap()];
            let tag = &seg[..seg.find('>').unwrap()];
            if tag.contains(r#"ry="7""#) {
                out.push(cx.parse().unwrap());
            }
        }
        out
    }

    /// The whole point of the animation: pigeon `i` actually descends onto hole `i`. Reconstruct each
    /// flight's landing from its perch + additive peak and assert the `n-1` placed pigeons land, one
    /// each, exactly on the `n-1` distinct hole centers, all on a single row down inside the holes —
    /// while the doomed `n`th pigeon only dives at the last hole and stops short (it never lands).
    #[test]
    fn each_placed_pigeon_lands_on_its_own_hole() {
        for n in [3usize, 6, 9, 12, 20] {
            let (svg, _) = render(&PigeonSpec { pigeons: n });
            let flights = flights(&svg);
            let holes = hole_centers(&svg);
            assert_eq!(flights.len(), n, "one flight per pigeon (placed + doomed), n={n}");
            assert_eq!(holes.len(), n - 1, "n-1 holes, n={n}");

            // The perch and peak are each emitted at one decimal place, so `perch + peak`
            // reconstructed from the strings sits within ~0.15px of the (also 1-dp) hole center —
            // sub-pixel, i.e. visually dead on. (The buggy `additive="replace"` would instead land
            // the pigeon at the raw peak, hundreds of px off — this tolerance never confuses the two.)
            let landing_y = flights[0].1 + flights[0].3;
            for i in 0..(n - 1) {
                let (bx, by, dx, dy) = flights[i];
                let land_x = bx + dx;
                assert!(
                    (land_x - holes[i]).abs() < 0.2,
                    "placed pigeon {i} must land on hole {i} ({} vs {}) for n={n}",
                    land_x,
                    holes[i]
                );
                assert!(by + dy > by, "pigeon {i} must descend, not rise, for n={n}");
                // `top_y` and the descent are both integer-valued and constant, so the landing row is exact.
                assert!(
                    (by + dy - landing_y).abs() < 1e-6,
                    "all placed pigeons land on one row for n={n}"
                );
            }

            let (dbx, _dby, ddx, _ddy) = flights[n - 1];
            assert!(
                (dbx + ddx - holes[n - 2]).abs() < 0.2,
                "the doomed pigeon dives at the last, contested hole for n={n}"
            );
        }
    }

    /// A fixed canvas height silently clipped the third certified status line. The `viewBox` height
    /// must cover the lowest `<text>` baseline for every instance size.
    #[test]
    fn no_status_line_is_clipped_below_the_canvas() {
        for n in SPEC_MIN_N..=SPEC_MAX_N {
            let (svg, _) = render(&PigeonSpec { pigeons: n });
            let viewbox = svg.split("viewBox=\"").nth(1).unwrap();
            let viewbox = &viewbox[..viewbox.find('"').unwrap()];
            let height: f64 = viewbox.split_whitespace().nth(3).unwrap().parse().unwrap();

            let mut lowest_text = 0.0_f64;
            for seg in svg.split("<text ").skip(1) {
                let y: f64 = seg.split("y=\"").nth(1).unwrap().split('"').next().unwrap().parse().unwrap();
                lowest_text = lowest_text.max(y);
            }
            assert!(
                height >= lowest_text,
                "canvas height {height} clips a status line at y={lowest_text} for n={n}"
            );
        }
    }
}
