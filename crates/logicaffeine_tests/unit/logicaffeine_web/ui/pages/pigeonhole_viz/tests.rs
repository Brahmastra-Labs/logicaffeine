use super::*;
use logicaffeine_proof::matching::is_hall_witness;

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

/// The editor paints `# …` after a value as a comment; the parser reads it as one too.
#[test]
fn a_trailing_comment_is_not_part_of_the_count() {
    let s = parse_pigeonhole_spec("pigeons: 6 # six birds, five holes\n")
        .expect("a commented count parses");
    assert_eq!(s.pigeons, 6);
}

#[test]
fn rejects_out_of_range_and_malformed() {
    assert!(
        parse_pigeonhole_spec("pigeons: 1").is_none(),
        "n=1 is degenerate"
    );
    assert!(
        parse_pigeonhole_spec("pigeons: 99").is_none(),
        "n=99 exceeds the spec cap"
    );
    assert!(
        parse_pigeonhole_spec("pigeons: lots").is_none(),
        "non-numeric count"
    );
    assert!(
        parse_pigeonhole_spec("holes: 3").is_none(),
        "no pigeons line"
    );
    assert!(parse_pigeonhole_spec("").is_none(), "empty input");
}

#[test]
fn recognizes_only_pigeonhole_specs() {
    assert!(is_pigeonhole_spec("pigeons: 6"));
    assert!(is_pigeonhole_spec("## Pigeonhole\npigeons: 8\n"));
    // The OTHER Hardware-mode inputs must NOT be hijacked by the pigeonhole egg:
    assert!(!is_pigeonhole_spec(
        "Always, if request is high, then acknowledge is high."
    ));
    assert!(!is_pigeonhole_spec(
        "module m(input clk); reg a; always @(posedge clk) a <= ~a; assert property (a); endmodule"
    ));
    assert!(!is_pigeonhole_spec("registers: 3\na: 0-5\nb: 1-6"));
    assert!(!is_pigeonhole_spec(
        "ns-through conflicts with ew-through and ew-left."
    ));
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
        assert!(
            v.certified,
            "ours stays certified with 0 conflicts on PHP({n})"
        );
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
    assert!(
        svg.contains("Hall witness"),
        "the certificate must be shown"
    );
    assert!(verdict.starts_with('\u{2713}'), "UNSAT verdict: {verdict}");
    assert!(
        verdict.contains("UNSAT") && verdict.contains("Hall witness"),
        "{verdict}"
    );
    assert!(verdict.contains("no Z3"), "{verdict}");
    assert!(
        verdict.contains("6 pigeons") && verdict.contains("5 holes"),
        "{verdict}"
    );
}

#[test]
fn renders_large_instances_without_running_the_heavy_paths() {
    // Past the live caps the render must still be coherent (matching witness + theory), never panic.
    let spec = PigeonSpec { pigeons: 18 };
    let (svg, verdict) = render(&spec);
    assert!(svg.starts_with("<svg") && svg.contains("</svg>"));
    assert!(svg.contains("Hall witness"));
    let v = solve(&spec);
    assert!(
        v.pr_clauses.is_none(),
        "Heule not reconstructed past the cap"
    );
    assert!(
        v.baseline_conflicts.is_none(),
        "baseline not run past the cap"
    );
    assert!(verdict.starts_with('\u{2713}'));
    assert!(
        verdict.contains("2^\u{03a9}(n)"),
        "theory framing present: {verdict}"
    );
}

#[test]
fn report_agrees_with_the_rendered_verdict() {
    for n in [3usize, 6, 14] {
        let spec = PigeonSpec { pigeons: n };
        let report = report(&spec);
        let (_, verdict) = render(&spec);
        assert!(report.contains("UNSAT"), "{report}");
        assert!(verdict.starts_with('\u{2713}'));
        assert!(report.contains("Hall witness"), "{report}");
    }
}

/// A certified refutation is the prover's success: the verdict leads with it as one.
#[test]
fn the_verdict_reads_as_a_certified_success() {
    for n in [6, 12, SPEC_MAX_N] {
        let (_, verdict) = render(&PigeonSpec { pigeons: n });
        let lead = format!(
            "\u{2713} Certified UNSAT \u{2014} {n} pigeons cannot fit {} holes.",
            n - 1
        );
        assert!(verdict.starts_with(&lead), "PHP({n}): {verdict}");
    }
    let (_, verdict) = render(&PigeonSpec { pigeons: 2 });
    assert!(
        verdict.starts_with("\u{2713} Certified UNSAT \u{2014} 2 pigeons cannot fit 1 hole."),
        "PHP(2): {verdict}"
    );
}

/// One hole, one clause: a count of one reads in the singular wherever the instance is described.
#[test]
fn a_count_of_one_reads_in_the_singular() {
    let spec = PigeonSpec { pigeons: 2 };
    let (svg, verdict) = render(&spec);
    let report = report(&spec);
    for text in [&svg, &verdict, &report] {
        for wrong in [" 1 holes", " 1 PR clauses", "(1 clauses"] {
            assert!(!text.contains(wrong), "{wrong:?} in {text}");
        }
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
    assert!(
        fly(40.0, 120.0, 0.1, 0.3, 0.9, 6.0).contains(r#"additive="sum""#),
        "fly must sum onto the perch"
    );
    assert!(
        flutter(40.0, 100.0, 0.6, 0.8, 0.9, 6.0).contains(r#"additive="sum""#),
        "flutter must sum onto the perch"
    );
}

/// Every `<animateTransform>` in a rendered scene animates the group's `transform`, so every one
/// must be additive or that pigeon leaves its perch and flies from the origin instead.
#[test]
fn every_rendered_transform_animation_is_additive() {
    for n in [3usize, 6, 9, 12, 18, 20] {
        let (svg, _) = render(&PigeonSpec { pigeons: n });
        let transforms = svg.matches("<animateTransform").count();
        // One flight per placed pigeon (`n-1`) plus the doomed pigeon's flutter = `n`.
        assert_eq!(
            transforms, n,
            "expected one transform animation per pigeon for n={n}"
        );
        let additive = svg.matches(r#"additive="sum""#).count();
        assert_eq!(
            additive, transforms,
            "every transform animation must be additive for n={n}"
        );
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
        assert_eq!(
            flights.len(),
            n,
            "one flight per pigeon (placed + doomed), n={n}"
        );
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
            let y: f64 = seg
                .split("y=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            lowest_text = lowest_text.max(y);
        }
        assert!(
            height >= lowest_text,
            "canvas height {height} clips a status line at y={lowest_text} for n={n}"
        );
    }
}
