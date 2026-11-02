use super::*;

const FITS: &str = "## Register Allocation\nregisters: 2\na: 0-2\nb: 1-3\nc: 3-5\n";
const SPILLS: &str = "registers: 3\nv0: 0-5\nv1: 1-6\nv2: 2-7\nv3: 2-8\n";

/// The timeline is data: it draws at its own size and its panel scrolls, keeping its tick and
/// lane labels readable.
#[test]
fn the_timeline_draws_at_its_own_size() {
    let (svg, _) = render(&parse_register_spec(FITS).unwrap());
    let head = &svg[..svg.find('>').unwrap()];
    let view = head
        .split("viewBox=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let view: Vec<&str> = view.split_whitespace().collect();
    assert!(head.contains(&format!("width=\"{}\"", view[2])), "{head}");
    assert!(head.contains(&format!("height=\"{}\"", view[3])), "{head}");
}

#[test]
fn parses_a_spec() {
    let s = parse_register_spec(FITS).expect("parses");
    assert_eq!(s.registers, 2);
    assert_eq!(s.names, vec!["a", "b", "c"]);
    assert_eq!(s.ranges.len(), 3);
    assert_eq!((s.ranges[0].start, s.ranges[0].end), (0, 2));
}

/// The editor paints `# …` after a value as a comment; the parser reads it as one too.
#[test]
fn a_trailing_comment_is_not_part_of_a_line() {
    let s = parse_register_spec("registers: 3 # the budget\na: 0-2 # first\nb: 1-3\n")
        .expect("a commented spec parses");
    assert_eq!(s.registers, 3);
    assert_eq!(s.names, vec!["a", "b"]);
    assert_eq!((s.ranges[0].start, s.ranges[0].end), (0, 2));
}

#[test]
fn rejects_specs_without_a_budget_or_ranges() {
    assert!(
        parse_register_spec("a: 0-3\nb: 1-4").is_none(),
        "no register budget"
    );
    assert!(parse_register_spec("registers: 2").is_none(), "no ranges");
}

#[test]
fn renders_a_feasible_allocation() {
    let spec = parse_register_spec(FITS).unwrap();
    let (svg, verdict) = render(&spec);
    assert!(svg.starts_with("<svg"), "valid SVG");
    assert!(svg.contains("</svg>"));
    assert!(
        svg.contains(">a<") && svg.contains(">c<"),
        "variable labels present"
    );
    assert!(
        verdict.contains("Allocated") && verdict.contains("certified"),
        "{verdict}"
    );
    assert!(!verdict.contains("spill") && !verdict.contains("Must spill"));
}

#[test]
fn renders_a_spill_with_a_certified_clique() {
    // 4 variables live at once, 3 registers ⇒ spill, and the verdict names the clique.
    let spec = parse_register_spec(SPILLS).unwrap();
    let (svg, verdict) = render(&spec);
    assert!(svg.contains("SPILL"), "spill bars are flagged");
    assert!(
        verdict.contains("Must spill") && verdict.contains("certified clique"),
        "{verdict}"
    );
    assert!(
        verdict.contains("4 variables are live at once"),
        "reports pressure: {verdict}"
    );
}

#[test]
fn parser_is_robust_to_messy_input() {
    // Comments, blank lines, stray whitespace, out-of-order lines, and a malformed second
    // `registers:` line that must NOT clobber the valid budget.
    let messy =
        "# a basic block\n\n  registers:  2  \n\n a :  0-3 \n  registers: oops\n b: 1 - 4 \n";
    let spec = parse_register_spec(messy).expect("messy spec still parses");
    assert_eq!(
        spec.registers, 2,
        "valid budget kept despite a later malformed line"
    );
    assert_eq!(spec.names, vec!["a", "b"]);
    assert_eq!((spec.ranges[1].start, spec.ranges[1].end), (1, 4));
    // Degenerate ranges (start >= end) are dropped, not mis-parsed.
    let with_empty = "registers: 1\nx: 5-5\ny: 0-2\n";
    let s2 = parse_register_spec(with_empty).expect("parses");
    assert_eq!(s2.ranges.len(), 1, "empty range x:5-5 dropped");
    assert_eq!(s2.names, vec!["y"]);
}

#[test]
fn recognizes_only_register_alloc_specs() {
    // Real register-alloc specs route to the easter egg.
    assert!(is_register_alloc_spec(FITS));
    assert!(is_register_alloc_spec(SPILLS));
    // The OTHER Hardware-mode inputs must NOT be hijacked by the allocator:
    assert!(
        !is_register_alloc_spec("Always, if request is high, then acknowledge is high."),
        "an SVA English spec is not a register-alloc spec"
    );
    assert!(
        !is_register_alloc_spec(
            "module m(input clk); reg a; always @(posedge clk) a <= ~a; assert property (a); endmodule"
        ),
        "a Verilog module is not a register-alloc spec"
    );
    assert!(
        !is_register_alloc_spec("ns-through conflicts with ew-through and ew-left."),
        "a signal-design spec is not a register-alloc spec"
    );
    assert!(!is_register_alloc_spec(""), "empty input is not a spec");
    assert!(
        !is_register_alloc_spec("registers: 3"),
        "a budget with no live ranges has nothing to allocate"
    );
    assert!(
        !is_register_alloc_spec("a: 0-4\nb: 1-3"),
        "live ranges with no budget cannot be allocated"
    );
}

#[test]
fn allocation_report_lists_a_register_per_variable_when_it_fits() {
    let spec = parse_register_spec(FITS).unwrap();
    let report = allocation_report(&spec);
    assert!(report.contains("Allocated"), "{report}");
    assert!(report.contains("certified"), "{report}");
    // Every variable is reported with a concrete physical register.
    for name in &spec.names {
        assert!(
            report.contains(&format!("{name} \u{2192} r")),
            "variable {name} missing its register: {report}"
        );
    }
    assert!(
        !report.contains("spill") && !report.contains("Must spill"),
        "{report}"
    );
}

#[test]
fn allocation_report_names_the_spill_clique_when_over_pressure() {
    let spec = parse_register_spec(SPILLS).unwrap();
    let report = allocation_report(&spec);
    assert!(report.contains("Must spill"), "{report}");
    assert!(report.contains("Certified clique"), "{report}");
    assert!(report.contains("live at once"), "{report}");
    // Every member of the over-pressure clique is named.
    for name in &spec.names {
        assert!(
            report.contains(name.as_str()),
            "clique missing {name}: {report}"
        );
    }
}

#[test]
fn report_agrees_with_the_rendered_verdict() {
    // The textual report and the SVG verdict are two views of the SAME certified result, so
    // their feasibility must always agree.
    for text in [FITS, SPILLS] {
        let spec = parse_register_spec(text).unwrap();
        let report = allocation_report(&spec);
        let (_, verdict) = render(&spec);
        assert_eq!(
            report.contains("Allocated"),
            verdict.contains("Allocated"),
            "report/verdict disagree:\n{report}\n{verdict}"
        );
        assert_eq!(
            report.contains("Must spill"),
            verdict.contains("Must spill")
        );
    }
}

#[test]
fn linear_scan_is_valid_and_consistent_with_the_engine() {
    for text in [FITS, SPILLS] {
        let spec = parse_register_spec(text).unwrap();
        let assignment = linear_scan_assignment(&spec);
        assert!(is_valid_linear_scan(&spec, &assignment), "{assignment:?}");
        // Linear-scan spills nothing iff the block fits the budget (peak pressure ≤ registers).
        let fits = register_pressure(&spec.ranges) <= spec.registers;
        assert_eq!(
            assignment.iter().all(Option::is_some),
            fits,
            "spill-set should be empty iff feasible: {assignment:?}"
        );
    }
}

#[test]
fn linear_scan_matches_the_engine_on_random_blocks() {
    let mut s: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for _ in 0..500 {
        let n = (next() % 9) as usize + 1;
        let registers = (next() % 5) as usize + 1;
        let names: Vec<String> = (0..n).map(|i| format!("v{i}")).collect();
        let ranges: Vec<LiveRange> = (0..n)
            .map(|v| {
                let a = (next() % 12) as i64;
                let len = (next() % 6) as i64 + 1;
                LiveRange::new(v, a, a + len)
            })
            .collect();
        let spec = RegSpec {
            names,
            ranges,
            registers,
        };
        let assignment = linear_scan_assignment(&spec);
        // It is ALWAYS a sound, re-checkable allocation...
        assert!(
            is_valid_linear_scan(&spec, &assignment),
            "invalid: {assignment:?}"
        );
        // ...and it spills exactly when the certified engine says spilling is unavoidable.
        let engine_fits = matches!(allocate(&spec.ranges, registers), Allocation::Allocated(_));
        assert_eq!(
            assignment.iter().all(Option::is_some),
            engine_fits,
            "linear-scan/engine feasibility disagree: {assignment:?}"
        );
    }
}

#[test]
fn is_valid_linear_scan_rejects_bad_assignments() {
    let spec = parse_register_spec(SPILLS).unwrap(); // v0..v3 all live together at instr 2
                                                     // Two simultaneously-live variables in the SAME register is unsound.
    assert!(!is_valid_linear_scan(
        &spec,
        &[Some(0), Some(0), Some(1), None]
    ));
    // A register outside the budget is rejected.
    assert!(!is_valid_linear_scan(
        &spec,
        &[Some(99), Some(1), Some(2), None]
    ));
    // A wrong-length assignment is rejected.
    assert!(!is_valid_linear_scan(&spec, &[Some(0)]));
    // The honest linear-scan result always re-checks as valid.
    assert!(is_valid_linear_scan(&spec, &linear_scan_assignment(&spec)));
}

#[test]
fn animated_render_has_a_moving_sweep_and_register_lanes() {
    let spec = parse_register_spec(FITS).unwrap();
    let (svg, _) = render(&spec);
    assert!(svg.contains("<animate"), "no SMIL animation in the SVG");
    assert!(
        svg.contains("animateTransform") && svg.contains("translate"),
        "no moving sweep line"
    );
    assert!(svg.contains("REGISTER FILE"), "no register-file panel");
    for reg in 0..spec.registers {
        assert!(
            svg.contains(&format!(">r{reg}<")),
            "missing lane r{reg}: {svg}"
        );
    }
    assert!(
        !svg.contains("MEM"),
        "a feasible block must not show a spill lane"
    );
}

#[test]
fn animated_render_flags_a_spill_in_a_memory_lane() {
    let spec = parse_register_spec(SPILLS).unwrap();
    let (svg, _) = render(&spec);
    assert!(svg.contains("<animate"), "the spill view is still animated");
    assert!(svg.contains(">MEM<"), "no spill (MEM) lane");
    assert!(svg.contains("SPILL"), "spilled value not flagged");
}

#[test]
fn pressure_profile_peaks_at_the_register_pressure() {
    // The histogram's tallest bar IS the certified register pressure, on the examples...
    for text in [FITS, SPILLS] {
        let spec = parse_register_spec(text).unwrap();
        let profile = pressure_profile(&spec);
        assert_eq!(
            profile.iter().copied().max().unwrap_or(0),
            register_pressure(&spec.ranges),
            "profile peak must equal register pressure: {profile:?}"
        );
    }
    // ...a hand-checked instance: a[0,2) b[1,3) overlap at instr 1, c[3,5) disjoint.
    let spec = parse_register_spec("registers: 2\na: 0-2\nb: 1-3\nc: 3-5").unwrap();
    // a live at {0,1}, b live at {1,2}, c live at {3,4} (half-open [start,end)).
    assert_eq!(pressure_profile(&spec), vec![1, 2, 1, 1, 1]);
}

#[test]
fn pressure_profile_peak_matches_engine_on_random_blocks() {
    let mut s: u64 = 0xD1B54A32D192ED03;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for _ in 0..500 {
        let n = (next() % 9) as usize + 1;
        let names: Vec<String> = (0..n).map(|i| format!("v{i}")).collect();
        let ranges: Vec<LiveRange> = (0..n)
            .map(|v| {
                let a = (next() % 12) as i64;
                let len = (next() % 6) as i64 + 1;
                LiveRange::new(v, a, a + len)
            })
            .collect();
        let spec = RegSpec {
            names,
            ranges,
            registers: 3,
        };
        assert_eq!(
            pressure_profile(&spec).iter().copied().max().unwrap_or(0),
            register_pressure(&spec.ranges),
        );
    }
}

#[test]
fn render_has_a_pressure_panel_with_a_budget_line() {
    let spec = parse_register_spec(FITS).unwrap();
    let (svg, _) = render(&spec);
    assert!(svg.contains("PRESSURE"), "no pressure panel");
    assert!(svg.contains("budget"), "no budget label");
    assert!(
        svg.contains("stroke-dasharray"),
        "budget line is not dashed"
    );
    // A feasible block never exceeds its budget, so nothing is painted over-capacity red.
    assert!(
        !svg.contains("#ff5c66"),
        "feasible block must not show an over-capacity marker"
    );
}

#[test]
fn render_paints_over_capacity_pressure_red_when_spilling() {
    let spec = parse_register_spec(SPILLS).unwrap();
    let (svg, _) = render(&spec);
    assert!(svg.contains("PRESSURE"), "no pressure panel");
    // Over-pressure instructions are flagged red.
    assert!(
        svg.contains("#ff5c66"),
        "spill block must show an over-capacity marker"
    );
}

#[test]
fn interference_edges_finds_overlapping_pairs() {
    // a[0,2) overlaps b[1,3); c[3,5) overlaps neither — one edge.
    let fits = parse_register_spec(FITS).unwrap();
    assert_eq!(interference_edges(&fits), vec![(0, 1)]);
    // v0..v3 all share instruction 2 — a complete graph K4 (6 edges).
    let spills = parse_register_spec(SPILLS).unwrap();
    assert_eq!(interference_edges(&spills).len(), 6);
}

#[test]
fn interference_edges_are_symmetric_with_overlap_on_random_blocks() {
    let mut s: u64 = 0x2545F4914F6CDD1D;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for _ in 0..300 {
        let n = (next() % 7) as usize + 1;
        let names: Vec<String> = (0..n).map(|i| format!("v{i}")).collect();
        let ranges: Vec<LiveRange> = (0..n)
            .map(|v| {
                let a = (next() % 10) as i64;
                let len = (next() % 5) as i64 + 1;
                LiveRange::new(v, a, a + len)
            })
            .collect();
        let spec = RegSpec {
            names,
            ranges,
            registers: 3,
        };
        let edges = interference_edges(&spec);
        // Every edge is a genuine overlap and i<j; and a clique cannot exceed the pressure.
        for &(i, j) in &edges {
            assert!(i < j);
            let (a, b) = (spec.ranges[i], spec.ranges[j]);
            assert!(
                a.start < b.end && b.start < a.end,
                "non-overlap edge {i}-{j}"
            );
        }
    }
}

#[test]
fn render_draws_the_interference_graph() {
    let spec = parse_register_spec(FITS).unwrap();
    let (svg, _) = render(&spec);
    assert!(
        svg.contains("INTERFERENCE GRAPH"),
        "no interference-graph panel"
    );
    assert!(svg.contains("χ ="), "no chromatic-number label");
    assert!(svg.contains("<circle"), "no graph nodes");
    // A feasible block has no certified clique, so no clique ring is drawn.
    assert!(
        !svg.contains("stroke-width=\"3\""),
        "feasible block must not ring a clique"
    );
}

#[test]
fn render_rings_the_spill_clique_in_the_graph() {
    let spec = parse_register_spec(SPILLS).unwrap();
    let (svg, _) = render(&spec);
    assert!(svg.contains("INTERFERENCE GRAPH"));
    // The certified clique nodes are ringed (unique stroke-width="3").
    assert!(
        svg.contains("stroke-width=\"3\""),
        "spill clique not ringed in the graph"
    );
}

#[test]
fn pulse_keytimes_are_strictly_increasing_for_every_window() {
    // The SMIL keyTimes MUST be strictly monotonic in [0,1] or the browser drops the animation;
    // sweep every possible normalised live window and parse the emitted keyTimes back out.
    for a in 0..=20 {
        for b in (a + 1)..=21 {
            let (t0, t1) = (a as f64 / 21.0, b as f64 / 21.0);
            let anim = pulse(t0, t1, 6.0, 0.16, 1.0);
            let kt = anim
                .split("keyTimes=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .expect("has keyTimes");
            let times: Vec<f64> = kt.split(';').map(|x| x.parse().unwrap()).collect();
            assert_eq!(times.len(), 6, "{anim}");
            assert_eq!(times[0], 0.0);
            assert_eq!(*times.last().unwrap(), 1.0);
            for w in times.windows(2) {
                assert!(w[0] < w[1], "keyTimes not increasing at {t0}..{t1}: {kt}");
            }
        }
    }
}

#[test]
fn verdict_tracks_the_certified_engine() {
    // Feasibility in the rendered verdict must agree with the engine's pressure check.
    for spec_text in [FITS, SPILLS] {
        let spec = parse_register_spec(spec_text).unwrap();
        let fits = register_pressure(&spec.ranges) <= spec.registers;
        let (_, verdict) = render(&spec);
        assert_eq!(verdict.contains("Allocated"), fits, "{verdict}");
    }
}
