//! Studio easter egg — certified linear-scan register allocation, visualised.
//!
//! Pure data → SVG (no Z3, no JS): a basic block's variable live ranges are laid on an instruction
//! timeline as bars, coloured by the physical register they're assigned. When the block is
//! over-pressure, the bars that provably must spill are flagged red and the certified Hall/clique
//! witness is reported. The allocation comes straight from the certified `register_alloc` engine,
//! so the picture is a faithful view of a re-checkable result — the same engine that crushes Z3 on
//! the colouring encoding, here rendered for a one-glance "watch the allocator decide" demo.

use super::hardware_spec;
use logicaffeine_proof::register_alloc::{allocate, register_pressure, Allocation, LiveRange};

/// A parsed register-allocation spec: named variable live ranges + the physical register budget.
pub struct RegSpec {
    /// Display name per variable index.
    pub names: Vec<String>,
    /// Live ranges (variable index matches `names`).
    pub ranges: Vec<LiveRange>,
    /// Physical register budget.
    pub registers: usize,
}

/// Parse a spec of the form (one `name: start-end` per line, plus a `registers: N` budget):
/// ```text
/// ## Register Allocation
/// registers: 3
/// a: 0-5
/// b: 1-6
/// ```
/// Returns `None` if there is no register budget or no live ranges.
pub fn parse_register_spec(spec: &str) -> Option<RegSpec> {
    let mut registers: Option<usize> = None;
    let mut names = Vec::new();
    let mut ranges = Vec::new();
    for raw in spec.lines() {
        let line = hardware_spec::content(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = hardware_spec::field_value(line, hardware_spec::REGISTERS) {
            // Only accept a well-formed budget; never clobber a valid one with a malformed line.
            if let Ok(r) = rest.parse::<usize>() {
                registers = Some(r);
            }
            continue;
        }
        if let Some((name, rng)) = line.split_once(':') {
            if let Some((s, e)) = rng.trim().split_once('-') {
                if let (Ok(start), Ok(end)) = (s.trim().parse::<i64>(), e.trim().parse::<i64>()) {
                    if start < end {
                        let var = names.len();
                        names.push(name.trim().to_string());
                        ranges.push(LiveRange::new(var, start, end));
                    }
                }
            }
        }
    }
    let registers = registers?;
    if ranges.is_empty() {
        return None;
    }
    Some(RegSpec {
        names,
        ranges,
        registers,
    })
}

/// Whether `spec` is a register-allocation spec (a `registers:` budget plus at least one live range).
/// This is the single source of truth the Studio uses to route Hardware-mode input to the allocator
/// easter egg, so the wiring (panel visibility, output panel, viz panel) can never drift from what
/// the parser actually accepts.
pub fn is_register_alloc_spec(spec: &str) -> bool {
    parse_register_spec(spec).is_some()
}

const REG_COLORS: &[&str] = &[
    "#61afef", "#98c379", "#e5c07b", "#c678dd", "#56b6c2", "#d19a66", "#56c2a8", "#b3a0ff",
];

/// A deterministic linear-scan assignment: variable index `i` → the physical register it is given, or
/// `None` if linear-scan must spill it (no register was free when it became live). This is the data
/// the animated view replays — each variable becomes one bar that lights up in its register's lane
/// while it is live. Re-checkable via [`is_valid_linear_scan`]. It spills *nothing* exactly when the
/// block fits the budget (peak pressure ≤ `registers`), so it agrees with the certified [`allocate`]
/// decision while also showing *which* value gets evicted when it does not.
pub fn linear_scan_assignment(spec: &RegSpec) -> Vec<Option<usize>> {
    let n = spec.ranges.len();
    let mut assignment = vec![None; n];
    // Visit variables in the order they become live (ties: the one that dies first).
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (spec.ranges[i].start, spec.ranges[i].end));
    // The register file: slot `r` currently holds `Some(variable)`, or `None` when free.
    let mut slots: Vec<Option<usize>> = vec![None; spec.registers];
    for &i in &order {
        let start = spec.ranges[i].start;
        // Free every register whose occupant has already died — `[start, end)` is half-open, so an
        // occupant with `end <= start` no longer overlaps the arriving variable.
        for slot in slots.iter_mut() {
            if let Some(occ) = *slot {
                if spec.ranges[occ].end <= start {
                    *slot = None;
                }
            }
        }
        // Take the lowest-numbered free register; if every slot is held, this variable spills.
        if let Some(free) = slots.iter().position(Option::is_none) {
            slots[free] = Some(i);
            assignment[i] = Some(free);
        }
    }
    assignment
}

/// Re-check a linear-scan assignment: every assigned register is within budget, and no two variables
/// that are live at the same time are given the same register — the soundness property any register
/// allocation must satisfy. (Spilled variables, `None`, impose no constraint: they live in memory.)
pub fn is_valid_linear_scan(spec: &RegSpec, assignment: &[Option<usize>]) -> bool {
    if assignment.len() != spec.ranges.len() {
        return false;
    }
    if assignment.iter().flatten().any(|&r| r >= spec.registers) {
        return false;
    }
    for i in 0..spec.ranges.len() {
        for j in (i + 1)..spec.ranges.len() {
            let (a, b) = (spec.ranges[i], spec.ranges[j]);
            let overlap = a.start < b.end && b.start < a.end;
            if overlap && assignment[i].is_some() && assignment[i] == assignment[j] {
                return false;
            }
        }
    }
    true
}

/// Register pressure sampled at each instruction in `tmin..tmax`: `profile[i]` is how many variables
/// are live at instruction `tmin + i`. Its maximum is the register pressure (the fewest registers the
/// block could ever need), which is what the budget line is drawn against — wherever the profile rises
/// above the budget, spilling is unavoidable.
pub fn pressure_profile(spec: &RegSpec) -> Vec<usize> {
    let tmin = spec.ranges.iter().map(|r| r.start).min().unwrap_or(0);
    let tmax = spec.ranges.iter().map(|r| r.end).max().unwrap_or(0);
    (tmin..tmax)
        .map(|t| {
            spec.ranges
                .iter()
                .filter(|r| r.start <= t && t < r.end)
                .count()
        })
        .collect()
}

/// The interference graph's edges: every pair of variables `(i, j)` (`i < j`) whose live ranges
/// overlap, so they cannot share a register. Register allocation is exactly colouring this graph; for
/// straight-line code it is an interval graph (perfect), so its chromatic number equals the largest
/// clique equals the register pressure.
pub fn interference_edges(spec: &RegSpec) -> Vec<(usize, usize)> {
    let mut edges = Vec::new();
    for i in 0..spec.ranges.len() {
        for j in (i + 1)..spec.ranges.len() {
            let (a, b) = (spec.ranges[i], spec.ranges[j]);
            if a.start < b.end && b.start < a.end {
                edges.push((i, j));
            }
        }
    }
    edges
}

/// A looping SMIL opacity pulse: dim outside the live window `[t0, t1]` (both normalised to `0..1` of
/// the animation cycle), bright inside it, with a fast snap at each edge — this is what makes a value
/// "light up" in its register lane exactly while the sweep line is over it.
fn pulse(t0: f64, t1: f64, dur: f64, dim: f64, bright: f64) -> String {
    let a1 = t0.clamp(0.006, 0.99);
    let a0 = (a1 - 0.004).max(0.002);
    let b0 = t1.clamp(0.006, 0.99).max(a1 + 0.002);
    let b1 = (b0 + 0.004).min(0.996);
    format!(
        r#"<animate attributeName="opacity" dur="{dur:.2}s" repeatCount="indefinite" keyTimes="0;{a0:.4};{a1:.4};{b0:.4};{b1:.4};1" values="{dim};{dim};{bright};{bright};{dim};{dim}"/>"#
    )
}

/// Render the certified allocation of `spec` as `(svg, verdict)`. The SVG is an animated linear-scan:
/// a live-range timeline above an animated register file, with a sweep line crossing both so you can
/// watch each value claim and release its register (and, when over capacity, drop to a spill lane).
/// The verdict is the plain-language, certified summary.
pub fn render(spec: &RegSpec) -> (String, String) {
    use std::fmt::Write as _;
    let alloc = allocate(&spec.ranges, spec.registers);
    let pressure = register_pressure(&spec.ranges);
    let assignment = linear_scan_assignment(spec);
    let spilled_any = assignment.iter().any(Option::is_none);
    let n = spec.ranges.len();
    let tmin = spec.ranges.iter().map(|r| r.start).min().unwrap_or(0);
    let tmax = spec.ranges.iter().map(|r| r.end).max().unwrap_or(1);
    let span = (tmax - tmin).max(1) as f64;

    // Geometry: a variable timeline stacked above a register-file panel, sharing the instruction axis.
    let left = 86.0_f64;
    let plot_w = 462.0_f64;
    let x = |t: i64| left + (t - tmin) as f64 / span * plot_w;
    let nt = |t: i64| (t - tmin) as f64 / span; // normalised 0..1 for animation timing

    let top1 = 46.0_f64;
    let trow_h = 22.0_f64;
    let tl_bottom = top1 + n as f64 * trow_h;
    let rf_label_y = tl_bottom + 20.0;
    let top_rf = tl_bottom + 30.0;
    let lane_h = 24.0_f64;
    let lanes = spec.registers + usize::from(spilled_any);
    let rf_bottom = top_rf + lanes as f64 * lane_h;

    // Pressure histogram panel: how many values are live at each instruction, against the budget line.
    let profile = pressure_profile(spec);
    let max_p = profile
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .max(spec.registers)
        .max(1);
    let pr_label_y = rf_bottom + 20.0;
    let pr_top = rf_bottom + 28.0;
    let pr_h = 56.0_f64;
    let pr_bottom = pr_top + pr_h;

    // Interference-graph panel: the graph register allocation is really colouring.
    let graph_label_y = pr_bottom + 36.0;
    let graph_top = pr_bottom + 44.0;
    let graph_h = 188.0_f64;
    let gcx = 300.0_f64;
    let gcy = graph_top + graph_h / 2.0;
    let gradius = (graph_h / 2.0 - 30.0).min(120.0);
    let legend_y = graph_top + graph_h + 2.0;

    let height = legend_y + 16.0;
    // One animation loop sweeps the whole block; tie its length to the program length, within reason.
    let dur = (span * 0.6).clamp(4.0, 12.0);

    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg viewBox="0 0 600 {height:.0}" width="600" height="{height:.0}" xmlns="http://www.w3.org/2000/svg" font-family="ui-monospace, monospace" font-size="12">"#
    );
    let _ = write!(
        s,
        r##"<rect x="0" y="0" width="600" height="{height:.0}" fill="#0f1117"/>"##
    );
    let _ = write!(
        s,
        r##"<text x="12" y="24" fill="#e5e7eb" font-size="14">Register allocation · {} registers · pressure {pressure}{}</text>"##,
        spec.registers,
        if spilled_any {
            "  \u{26A0} over capacity"
        } else {
            ""
        }
    );

    // Faint instruction gridlines + tick labels spanning every panel.
    let grid_top = top1 - 6.0;
    let grid_bot = pr_bottom + 2.0;
    let mut t = tmin;
    while t <= tmax {
        let gx = x(t);
        let _ = write!(
            s,
            r##"<line x1="{gx:.1}" y1="{grid_top:.0}" x2="{gx:.1}" y2="{grid_bot:.0}" stroke="#1e2230" stroke-width="1"/>"##
        );
        let _ = write!(
            s,
            r##"<text x="{gx:.1}" y="{:.0}" fill="#3b4252" font-size="9" text-anchor="middle">{t}</text>"##,
            grid_bot + 12.0
        );
        t += 1;
    }

    // ---- Timeline panel: one solid bar per variable, coloured by the register it is given. ----
    for i in 0..n {
        let r = spec.ranges[i];
        let y = top1 + i as f64 * trow_h;
        let _ = write!(
            s,
            r##"<text x="10" y="{:.0}" fill="#abb2bf">{}</text>"##,
            y + 14.0,
            spec.names[i]
        );
        let bx = x(r.start);
        let bw = (x(r.end) - bx).max(5.0);
        let (fill, tag, stroke) = match assignment[i] {
            Some(reg) => (REG_COLORS[reg % REG_COLORS.len()], format!("r{reg}"), ""),
            None => (
                "#e06c75",
                "SPILL".to_string(),
                r##" stroke="#ff5c66" stroke-width="1.5""##,
            ),
        };
        let _ = write!(
            s,
            r#"<rect x="{bx:.1}" y="{:.0}" width="{bw:.1}" height="16" rx="4" fill="{fill}"{stroke}/>"#,
            y + 2.0
        );
        let _ = write!(
            s,
            r##"<text x="{:.1}" y="{:.0}" fill="#10131a" font-size="11">{tag}</text>"##,
            bx + 4.0,
            y + 14.0
        );
    }

    // ---- Register-file panel: a lane per register (plus MEM for spills); occupants pulse bright as
    //       the sweep crosses their live range, so you watch each register fill and free. ----
    let _ = write!(
        s,
        r##"<text x="12" y="{rf_label_y:.0}" fill="#7a8290" font-size="11" letter-spacing="0.08em">REGISTER FILE</text>"##
    );
    for reg in 0..spec.registers {
        let y = top_rf + reg as f64 * lane_h;
        let colour = REG_COLORS[reg % REG_COLORS.len()];
        let _ = write!(
            s,
            r##"<rect x="{left:.0}" y="{:.1}" width="{plot_w:.0}" height="{:.1}" rx="4" fill="#161a24"/>"##,
            y + 2.0,
            lane_h - 6.0
        );
        let _ = write!(
            s,
            r##"<text x="12" y="{:.0}" fill="{colour}">r{reg}</text>"##,
            y + 16.0
        );
        for i in 0..n {
            if assignment[i] == Some(reg) {
                let r = spec.ranges[i];
                let bx = x(r.start);
                let bw = (x(r.end) - bx).max(5.0);
                let bar = pulse(nt(r.start), nt(r.end), dur, 0.16, 1.0);
                let txt = pulse(nt(r.start), nt(r.end), dur, 0.25, 1.0);
                let _ = write!(
                    s,
                    r#"<rect x="{bx:.1}" y="{:.1}" width="{bw:.1}" height="{:.1}" rx="4" fill="{colour}" opacity="0.16">{bar}</rect>"#,
                    y + 3.0,
                    lane_h - 8.0
                );
                let _ = write!(
                    s,
                    r##"<text x="{:.1}" y="{:.0}" fill="#10131a" font-size="11" opacity="0.25">{}{txt}</text>"##,
                    bx + 5.0,
                    y + 16.0,
                    spec.names[i]
                );
            }
        }
    }
    if spilled_any {
        let y = top_rf + spec.registers as f64 * lane_h;
        let _ = write!(
            s,
            r##"<rect x="{left:.0}" y="{:.1}" width="{plot_w:.0}" height="{:.1}" rx="4" fill="#2a1417"/>"##,
            y + 2.0,
            lane_h - 6.0
        );
        let _ = write!(
            s,
            r##"<text x="12" y="{:.0}" fill="#e06c75">MEM</text>"##,
            y + 16.0
        );
        for i in 0..n {
            if assignment[i].is_none() {
                let r = spec.ranges[i];
                let bx = x(r.start);
                let bw = (x(r.end) - bx).max(5.0);
                let bar = pulse(nt(r.start), nt(r.end), dur, 0.3, 1.0);
                let _ = write!(
                    s,
                    r##"<rect x="{bx:.1}" y="{:.1}" width="{bw:.1}" height="{:.1}" rx="4" fill="#e06c75" stroke="#ff5c66" stroke-width="1.2" opacity="0.3">{bar}</rect>"##,
                    y + 3.0,
                    lane_h - 8.0
                );
                let _ = write!(
                    s,
                    r##"<text x="{:.1}" y="{:.0}" fill="#2a1417" font-size="11">{} SPILL</text>"##,
                    bx + 5.0,
                    y + 16.0,
                    spec.names[i]
                );
            }
        }
    }

    // ---- Pressure panel: a per-instruction live-count histogram against the budget line. Bars that
    //       rise above the budget are red — exactly the instructions at which spilling is forced. ----
    let _ = write!(
        s,
        r##"<text x="12" y="{pr_label_y:.0}" fill="#7a8290" font-size="11" letter-spacing="0.08em">PRESSURE</text>"##
    );
    let bar_y = |p: usize| pr_bottom - (p as f64 / max_p as f64) * pr_h;
    for (i, &p) in profile.iter().enumerate() {
        let t = tmin + i as i64;
        let bx = x(t);
        let bw = (x(t + 1) - bx - 1.0).max(2.0);
        let y = bar_y(p);
        let colour = if p > spec.registers {
            "#ff5c66"
        } else {
            "#5aa0e0"
        };
        let _ = write!(
            s,
            r##"<rect x="{bx:.1}" y="{y:.1}" width="{bw:.1}" height="{:.1}" rx="1.5" fill="{colour}" opacity="0.85"/>"##,
            pr_bottom - y
        );
    }
    let by = bar_y(spec.registers);
    let _ = write!(
        s,
        r##"<line x1="{left:.0}" y1="{by:.1}" x2="{:.0}" y2="{by:.1}" stroke="#e5c07b" stroke-width="1.2" stroke-dasharray="5 4"/>"##,
        left + plot_w
    );
    let _ = write!(
        s,
        r##"<text x="8" y="{:.1}" fill="#e5c07b" font-size="9">budget {}</text>"##,
        by + 3.0,
        spec.registers
    );

    // ---- The sweep line: a translating beam crossing every panel, looping with the occupancy pulses.
    let beam_top = top1 - 6.0;
    let beam_bot = pr_bottom + 2.0;
    let _ = write!(
        s,
        r##"<g><rect x="{left:.0}" y="{beam_top:.0}" width="7" height="{:.0}" fill="#f8fafc" opacity="0.10"/><line x1="{left:.0}" y1="{beam_top:.0}" x2="{left:.0}" y2="{beam_bot:.0}" stroke="#f8fafc" stroke-width="2" opacity="0.85"/><animateTransform attributeName="transform" type="translate" from="0 0" to="{plot_w:.0} 0" dur="{dur:.2}s" repeatCount="indefinite"/></g>"##,
        beam_bot - beam_top
    );

    // ---- Interference graph: nodes = variables on a ring, edges = overlapping lifetimes; nodes are
    //       coloured by the register they get, and the certified spill clique (if any) is ringed red.
    //       A node glows in sync with the sweep while its value is live. ----
    let clique: std::collections::HashSet<usize> = match &alloc {
        Allocation::Spill { must_spill, .. } => must_spill.iter().copied().collect(),
        Allocation::Allocated(_) => Default::default(),
    };
    let _ = write!(
        s,
        r##"<text x="12" y="{graph_label_y:.0}" fill="#7a8290" font-size="11" letter-spacing="0.08em">INTERFERENCE GRAPH · χ = {pressure} (registers needed)</text>"##
    );
    let mut pos: Vec<(f64, f64)> = Vec::with_capacity(n);
    for i in 0..n {
        if n <= 1 {
            pos.push((gcx, gcy));
        } else {
            let ang =
                -std::f64::consts::FRAC_PI_2 + 2.0 * std::f64::consts::PI * i as f64 / n as f64;
            pos.push((gcx + gradius * ang.cos(), gcy + gradius * ang.sin()));
        }
    }
    for &(i, j) in &interference_edges(spec) {
        let (x1, y1) = pos[i];
        let (x2, y2) = pos[j];
        let (stroke, w, op) = if clique.contains(&i) && clique.contains(&j) {
            ("#ff5c66", 2.5, 0.9)
        } else {
            ("#3a4150", 1.2, 0.55)
        };
        let _ = write!(
            s,
            r##"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="{stroke}" stroke-width="{w}" opacity="{op}"/>"##
        );
    }
    for i in 0..n {
        let (px, py) = pos[i];
        let fill = match assignment[i] {
            Some(reg) => REG_COLORS[reg % REG_COLORS.len()],
            None => "#e06c75",
        };
        if clique.contains(&i) {
            let _ = write!(
                s,
                r##"<circle cx="{px:.1}" cy="{py:.1}" r="17" fill="none" stroke="#ff5c66" stroke-width="3"/>"##
            );
        }
        let glow = pulse(
            nt(spec.ranges[i].start),
            nt(spec.ranges[i].end),
            dur,
            0.5,
            1.0,
        );
        let _ = write!(
            s,
            r##"<circle cx="{px:.1}" cy="{py:.1}" r="14" fill="{fill}" stroke="#0f1117" stroke-width="1.5" opacity="0.5">{glow}</circle>"##
        );
        let _ = write!(
            s,
            r##"<text x="{px:.1}" y="{:.1}" fill="#10131a" font-size="11" text-anchor="middle">{}</text>"##,
            py + 4.0,
            spec.names[i]
        );
    }

    // ---- Legend ----
    let mut lx = 12.0_f64;
    for reg in 0..spec.registers.min(REG_COLORS.len()) {
        let _ = write!(
            s,
            r##"<rect x="{lx:.0}" y="{:.0}" width="10" height="10" rx="2" fill="{}"/>"##,
            legend_y - 9.0,
            REG_COLORS[reg]
        );
        let _ = write!(
            s,
            r##"<text x="{:.0}" y="{legend_y:.0}" fill="#9aa3b2" font-size="10">r{reg}</text>"##,
            lx + 14.0
        );
        lx += 42.0;
    }
    if spilled_any {
        let _ = write!(
            s,
            r##"<rect x="{lx:.0}" y="{:.0}" width="10" height="10" rx="2" fill="#e06c75"/>"##,
            legend_y - 9.0
        );
        let _ = write!(
            s,
            r##"<text x="{:.0}" y="{legend_y:.0}" fill="#9aa3b2" font-size="10">spill</text>"##,
            lx + 14.0
        );
        lx += 50.0;
    }
    let _ = write!(
        s,
        r##"<text x="{lx:.0}" y="{legend_y:.0}" fill="#9aa3b2" font-size="10">red clique = cannot share</text>"##
    );

    let _ = write!(s, "</svg>");

    let verdict = match &alloc {
        Allocation::Allocated(_) => format!(
            "\u{2713} Allocated with {} registers, certified: no two simultaneously-live variables share a register.",
            spec.registers
        ),
        Allocation::Spill { pressure, must_spill } => {
            let names: Vec<&str> = must_spill
                .iter()
                .filter_map(|v| spec.names.get(*v).map(String::as_str))
                .collect();
            format!(
                "\u{26A0} Must spill: {pressure} variables are live at once but only {} registers exist. {{{}}} all mutually interfere, so they cannot share {} registers (certified clique).",
                spec.registers,
                names.join(", "),
                spec.registers
            )
        }
    };
    (s, verdict)
}

/// A textual allocation report for the Studio output panel: the physical register assigned to each
/// variable, or — when the block is over-pressure — the spill verdict naming its mutually-interfering
/// clique. This is the "compiler back-end output" view that pairs with the [`render`] timeline; the
/// per-variable registers come from the same [`linear_scan_assignment`] the animation replays, and
/// the feasibility/clique verdict from the certified [`allocate`] result, so the two views agree.
pub fn allocation_report(spec: &RegSpec) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    match allocate(&spec.ranges, spec.registers) {
        Allocation::Allocated(_) => {
            let assignment = linear_scan_assignment(spec);
            let _ = writeln!(
                out,
                "Allocated {} variable{} into {} register{}, certified: no two simultaneously-live variables share a register.",
                spec.names.len(),
                if spec.names.len() == 1 { "" } else { "s" },
                spec.registers,
                if spec.registers == 1 { "" } else { "s" },
            );
            for (i, name) in spec.names.iter().enumerate() {
                if let Some(reg) = assignment.get(i).copied().flatten() {
                    let _ = writeln!(out, "  {name} \u{2192} r{reg}");
                }
            }
        }
        Allocation::Spill {
            pressure,
            must_spill,
        } => {
            let clique: Vec<&str> = must_spill
                .iter()
                .filter_map(|v| spec.names.get(*v).map(String::as_str))
                .collect();
            let _ = writeln!(
                out,
                "Must spill: {pressure} variables are live at once but only {} register{} exist.",
                spec.registers,
                if spec.registers == 1 { "" } else { "s" },
            );
            let _ = write!(
                out,
                "Certified clique (all mutually interfere): {}",
                clique.join(", ")
            );
        }
    }
    out
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/register_alloc_viz/tests.rs"]
mod tests;
