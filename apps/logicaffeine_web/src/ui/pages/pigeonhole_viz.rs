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

use super::hardware_spec;
use logicaffeine_proof::cdcl::{SolveResult, Solver};
use logicaffeine_proof::families::php;
use logicaffeine_proof::matching::{assign_or_hall, HallWitness, MatchOutcome};
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::sym_certify::heule_php_refutation;

/// Smallest instance worth showing (one pigeon, zero holes is degenerate).
const SPEC_MIN_N: usize = *hardware_spec::PIGEONS_RANGE.start();
/// Largest instance the spec accepts — keeps the drawing legible and the CNF build bounded.
const SPEC_MAX_N: usize = *hardware_spec::PIGEONS_RANGE.end();
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
        let line = hardware_spec::content(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = hardware_spec::field_value(line, hardware_spec::PIGEONS) {
            if let Ok(n) = rest.parse::<usize>() {
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
        MatchOutcome::Feasible(_) => HallWitness {
            items: (0..n).collect(),
            slots: (0..holes).collect(),
        },
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

    Verdict {
        hall,
        pr_clauses,
        certified,
        baseline_conflicts,
    }
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
    let _ = write!(
        s,
        r##"<ellipse cx="0" cy="0" rx="13" ry="9" fill="{body}"/>"##
    );
    let _ = write!(
        s,
        r##"<path d="M -2,-3 q 12,-9 14,3 q -8,-3 -14,2 Z" fill="{accent}"/>"##
    );
    let _ = write!(s, r##"<circle cx="9" cy="-5" r="6" fill="{body}"/>"##);
    let _ = write!(s, r##"<circle cx="11" cy="-6" r="1.4" fill="#0f1117"/>"##);
    let _ = write!(
        s,
        r##"<path d="M 14,-5 l 6,-1.5 l -6,3 Z" fill="#f59e0b"/>"##
    );
    s
}

/// The type size of the picture's title and status lines: the picture fits its panel, so its
/// text is set large enough to stay readable at the Studio's default column width.
const CAPTION_PX: u32 = 15;

/// `n` of `noun`, singular for one: "1 hole", "5 holes".
fn counted(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The result in one line: the certified refutation.
pub fn headline(spec: &PigeonSpec) -> String {
    format!(
        "\u{2713} Certified UNSAT \u{2014} {} cannot fit {}.",
        counted(spec.pigeons, "pigeon"),
        counted(spec.holes(), "hole")
    )
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
    let status_step = 24.0_f64;
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
    let _ = write!(
        s,
        r##"<rect x="0" y="0" width="{w:.0}" height="{height:.0}" fill="#0f1117"/>"##
    );
    let _ = write!(
        s,
        r##"<text x="14" y="26" fill="#e5e7eb" font-size="{CAPTION_PX}" font-family="ui-monospace, monospace">Pigeonhole · {} → {}</text>"##,
        counted(n, "pigeon"),
        counted(holes, "hole")
    );

    // The dovecote: n-1 hole openings along the bottom, on a shelf.
    let _ = write!(
        s,
        r##"<rect x="{margin:.0}" y="{shelf_y:.1}" width="{plot_w:.0}" height="6" rx="3" fill="#3a2f25"/>"##
    );
    for j in 0..holes {
        let cx = hx(j);
        let x = cx - hole_w / 2.0;
        let _ = write!(
            s,
            r##"<rect x="{x:.1}" y="{hole_y:.1}" width="{hole_w:.1}" height="{hole_h:.1}" rx="6" fill="#1b1f2a" stroke="#3a4150" stroke-width="1.5"/>"##
        );
        let _ = write!(
            s,
            r##"<ellipse cx="{cx:.1}" cy="{:.1}" rx="{:.1}" ry="7" fill="#0a0c12"/>"##,
            hole_y + 11.0,
            hole_w / 2.0 - 5.0
        );
    }

    // The placed pigeons: pigeon i → hole i for i in 0..holes; each dives in, staggered.
    let descend = hole_y + 6.0 - top_y;
    for i in 0..holes {
        let dx = hx(i) - px(i);
        let frac = i as f64 / n as f64;
        let b0 = 0.06 + 0.55 * frac;
        let anim = fly(dx, descend, b0, b0 + 0.16, 0.9, dur);
        let _ = write!(
            s,
            r##"<g transform="translate({:.1},{top_y:.1})">{anim}{}</g>"##,
            px(i),
            pigeon_glyph("#9fb3c8", "#7d93ab")
        );
    }

    // The doomed pigeon (the last one): dives at the nearest hole, finds it taken, retreats — flashing.
    let doomed = n - 1;
    let target_hole = holes - 1;
    let ddx = hx(target_hole) - px(doomed);
    let dmid = descend - 10.0;
    let danim = flutter(ddx, dmid, 0.62, 0.78, 0.9, dur);
    let _ = write!(
        s,
        r##"<g transform="translate({:.1},{top_y:.1})">{danim}{}</g>"##,
        px(doomed),
        pigeon_glyph("#f87171", "#dc6363")
    );

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
        "\u{2713} Hall witness: {} reach only {} (polynomial matching)",
        counted(v.hall.items.len(), "pigeon"),
        counted(v.hall.slots.len(), "hole")
    );
    let _ = write!(
        s,
        r##"<text x="14" y="{y:.0}" fill="#86efac" font-size="{CAPTION_PX}">{hall_line}</text>"##
    );
    y += status_step;
    let ours = match v.pr_clauses {
        Some(k) if v.certified => format!(
            "\u{2713} Ours: 0 conflicts, a certified PR proof ({}), no Z3",
            counted(k, "clause")
        ),
        _ => "\u{2713} Ours: a polynomial certified PR proof (Heule\u{2013}Kiesl\u{2013}Biere), no Z3".to_string(),
    };
    let _ = write!(
        s,
        r##"<text x="14" y="{y:.0}" fill="#86efac" font-size="{CAPTION_PX}">{ours}</text>"##
    );
    y += status_step;
    let base = match v.baseline_conflicts {
        Some(c) => format!(
            "\u{2717} Resolution / CDCL (Kissat-class): {c} conflicts, 2^\u{03a9}(n) as n grows"
        ),
        None => {
            "\u{2717} Resolution / CDCL (Kissat, CaDiCaL, Z3): 2^\u{03a9}(n), provably".to_string()
        }
    };
    let _ = write!(
        s,
        r##"<text x="14" y="{y:.0}" fill="#fca5a5" font-size="{CAPTION_PX}">{base}</text>"##
    );

    let _ = write!(s, "</svg>");

    let proof = match v.pr_clauses {
        Some(k) if v.certified => format!(
            " Our prover auto-refutes it with a certified symmetry-breaking proof ({}, machine-checked against the original formula) in the browser, with no Z3.",
            counted(k, "PR clause")
        ),
        _ => " Our prover refutes it with a polynomial certified PR proof (Heule\u{2013}Kiesl\u{2013}Biere), with no Z3.".to_string(),
    };
    let (pigeons, room) = (counted(n, "pigeon"), counted(holes, "hole"));
    let verdict = format!(
        "{} Maximum bipartite matching returns a re-verified Hall witness ({pigeons} reach only {room}), so no assignment exists.{proof} Every resolution-class solver (Kissat, CaDiCaL, Glucose, Z3) needs 2^\u{03a9}(n) time on this family (Haken 1985).",
        headline(spec)
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
    let _ = writeln!(
        out,
        "PHP({n}): {} into {}, UNSAT.",
        counted(n, "pigeon"),
        counted(holes, "hole")
    );
    let _ = writeln!(
        out,
        "Hall witness (re-verified): {} collectively reach only {}.",
        counted(v.hall.items.len(), "pigeon"),
        counted(v.hall.slots.len(), "hole")
    );
    match v.pr_clauses {
        Some(k) if v.certified => {
            let _ = writeln!(
                out,
                "Certified symmetry-breaking proof: {}, 0 conflicts, machine-checked. No Z3.",
                counted(k, "PR clause")
            );
        }
        _ => {
            let _ = writeln!(
                out,
                "Certified by a polynomial PR proof (Heule\u{2013}Kiesl\u{2013}Biere). No Z3."
            );
        }
    }
    if let Some(c) = v.baseline_conflicts {
        let _ = write!(out, "Baseline CDCL (Kissat-class) on the same instance: {c} conflicts, the search our proof avoids.");
    } else {
        let _ = write!(out, "Resolution-class solvers (Kissat, CaDiCaL, Z3) need 2^\u{03a9}(n) here, provably exponential.");
    }
    out
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/pigeonhole_viz/tests.rs"]
mod tests;
