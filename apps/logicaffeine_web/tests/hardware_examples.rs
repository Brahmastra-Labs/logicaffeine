//! Runtime audit of every Hardware-mode example shipped in the Studio.
//!
//! Each `/examples/hardware/*.hw` file (`ui::examples::HARDWARE_EXAMPLES`) is an English
//! hardware spec. In the browser, loading one runs the SAME entrypoints the Studio wires up:
//!   - synthesize SystemVerilog Assertions from the spec (`synthesize_sva_from_spec`),
//!   - certify, in-browser and Z3-free, that the SVA captures the spec
//!     (`prove_spec_sva_equivalence`),
//!   - check the trigger is reachable (`check_spec_vacuity`).
//!
//! This asserts every shipped example actually works on those entrypoints, so a parser or
//! synthesizer change can't silently rot the Hardware tour into "Execute" buttons that error.

use logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec;
use logicaffeine_compile::codegen_sva::hw_pipeline::{
    check_spec_vacuity, prove_spec_sva_equivalence, VacuityReport,
};
use logicaffeine_compile::codegen_sva::rtl::parse_transition_system;
use logicaffeine_proof::bmc::{BmcOutcome, InductionOutcome};
use logicaffeine_web::ui::examples::{HARDWARE_EXAMPLES, RTL_EXAMPLES};

const BOUND: u32 = 8;

#[test]
fn every_hardware_example_synthesizes_and_certifies() {
    use logicaffeine_compile::codegen_sva::signal_design::{design_from_spec, is_valid_coloring};
    assert!(!HARDWARE_EXAMPLES.is_empty(), "the Hardware tour must ship examples");
    for (name, spec) in HARDWARE_EXAMPLES {
        // Signal-design specs ("X conflicts with Y") follow the certified phase-designer path,
        // not the SVA-synthesis path: their plan must be a valid, minimal colouring.
        if spec.to_lowercase().contains("conflict") {
            let (intersection, plan) = design_from_spec(spec)
                .unwrap_or_else(|e| panic!("[{name}] did not design a plan: {e}"));
            assert!(
                is_valid_coloring(&intersection, &plan),
                "[{name}] synthesized plan is not conflict-free"
            );
            assert!(
                plan.minimal_certified,
                "[{name}] phase count is not certified minimal"
            );
            continue;
        }
        // 1. Synthesis must succeed.
        let synth = synthesize_sva_from_spec(spec, "clk")
            .unwrap_or_else(|e| panic!("[{name}] did not synthesize: {e}"));
        assert!(!synth.sva_text.is_empty(), "[{name}] produced empty SVA");

        // 2. The synthesized SVA must be certified equivalent to the spec.
        let equiv = prove_spec_sva_equivalence(spec, &synth.body, BOUND)
            .unwrap_or_else(|e| panic!("[{name}] equivalence proof unavailable: {e}"));
        assert!(
            equiv.equivalent,
            "[{name}] synthesized SVA is NOT certified equivalent to its spec"
        );

        // 3. The example must be non-vacuous (the trigger can fire).
        let vac = check_spec_vacuity(spec, BOUND)
            .unwrap_or_else(|e| panic!("[{name}] vacuity check failed: {e}"));
        assert_eq!(
            vac,
            VacuityReport::NonVacuous,
            "[{name}] is vacuous — a shipped example must exercise its property"
        );
    }
}

#[test]
fn hardware_example_filenames_use_the_hw_extension() {
    for (name, _) in HARDWARE_EXAMPLES {
        assert!(name.ends_with(".hw"), "[{name}] must use the .hw extension so the Studio opens it in Hardware mode");
    }
}

/// Every shipped register-allocation example must open in Hardware mode (`.hw`), parse, render a
/// valid SVG, and carry a verdict that agrees with the certified allocator — so the easter egg can't
/// rot into a broken example on Execute.
#[test]
fn every_regalloc_example_parses_renders_and_matches_the_engine() {
    use logicaffeine_proof::register_alloc::register_pressure;
    use logicaffeine_web::ui::examples::REGALLOC_EXAMPLES;
    use logicaffeine_web::ui::pages::register_alloc_viz::{
        allocation_report, parse_register_spec, render,
    };
    assert!(!REGALLOC_EXAMPLES.is_empty(), "the register-allocation tour must ship examples");
    for (name, spec) in REGALLOC_EXAMPLES {
        assert!(
            name.ends_with(".hw"),
            "[{name}] must be .hw so the Studio opens it in Hardware mode (.regalloc/.logos won't route here)"
        );
        let parsed = parse_register_spec(spec).unwrap_or_else(|| panic!("[{name}] did not parse"));
        let (svg, verdict) = render(&parsed);
        assert!(svg.starts_with("<svg") && svg.contains("</svg>"), "[{name}] produced invalid SVG");
        // The rendered verdict must track the certified register pressure.
        let fits = register_pressure(&parsed.ranges) <= parsed.registers;
        assert_eq!(
            verdict.contains("Allocated"),
            fits,
            "[{name}] verdict disagrees with the certified engine: {verdict}"
        );
        if !fits {
            assert!(verdict.contains("certified clique"), "[{name}] spill must cite its clique: {verdict}");
        }
        // The output-panel report is the other half of the demo: it must be non-empty and agree
        // with the same certified result the timeline renders.
        let report = allocation_report(&parsed);
        assert!(!report.is_empty(), "[{name}] empty allocation report");
        if fits {
            assert!(report.contains("Allocated"), "[{name}] report: {report}");
        } else {
            assert!(
                report.contains("Must spill") && report.contains("Certified clique"),
                "[{name}] report: {report}"
            );
        }
    }
}

/// The Studio routes Hardware-mode input to the register-allocation easter egg via
/// `is_register_alloc_spec` (the same predicate that drives panel visibility, the output panel, and
/// the timeline). It MUST fire on every register-alloc example and on NONE of the SVA-design or RTL
/// examples — otherwise the easter egg either fails to appear or silently hijacks a real hardware
/// spec. This locks the routing exhaustively over the whole shipped Hardware corpus.
#[test]
fn register_alloc_routing_is_exclusive_over_the_whole_corpus() {
    use logicaffeine_web::ui::examples::REGALLOC_EXAMPLES;
    use logicaffeine_web::ui::pages::register_alloc_viz::is_register_alloc_spec;
    for (name, spec) in REGALLOC_EXAMPLES {
        assert!(is_register_alloc_spec(spec), "[{name}] must route to the register allocator");
    }
    for (name, spec) in HARDWARE_EXAMPLES {
        assert!(
            !is_register_alloc_spec(spec),
            "[{name}] SVA/design spec must NOT route to the register allocator"
        );
    }
    for (name, src) in RTL_EXAMPLES {
        assert!(
            !is_register_alloc_spec(src),
            "[{name}] Verilog must NOT route to the register allocator"
        );
    }
}

/// Every shipped pigeonhole example must open in Hardware mode (`.hw`), parse, render a valid SVG,
/// and carry an UNSAT verdict certified by our prover (Hall witness + symmetry-breaking proof) — so
/// the easter egg can't rot into a broken example on Execute.
#[test]
fn every_pigeonhole_example_solves_and_certifies() {
    use logicaffeine_web::ui::examples::PIGEONHOLE_EXAMPLES;
    use logicaffeine_web::ui::pages::pigeonhole_viz::{parse_pigeonhole_spec, render, report, solve};
    assert!(!PIGEONHOLE_EXAMPLES.is_empty(), "the pigeonhole tour must ship examples");
    for (name, spec) in PIGEONHOLE_EXAMPLES {
        assert!(
            name.ends_with(".hw"),
            "[{name}] must be .hw so the Studio opens it in Hardware mode"
        );
        let parsed = parse_pigeonhole_spec(spec).unwrap_or_else(|| panic!("[{name}] did not parse"));
        let (svg, verdict) = render(&parsed);
        assert!(svg.starts_with("<svg") && svg.contains("</svg>"), "[{name}] produced invalid SVG");
        assert!(svg.contains("<animateTransform"), "[{name}] is not animated");
        assert!(verdict.starts_with('\u{2717}'), "[{name}] must be UNSAT: {verdict}");
        assert!(verdict.contains("Hall witness") && verdict.contains("no Z3"), "[{name}] verdict: {verdict}");
        // The solver result behind the picture must be a re-verified Hall witness, and (in the live
        // range) a certified proof.
        let v = solve(&parsed);
        assert_eq!(v.hall.items.len(), parsed.pigeons);
        assert_eq!(v.hall.slots.len(), parsed.holes());
        assert!(v.certified, "[{name}] must carry a certified refutation");
        let report = report(&parsed);
        assert!(report.contains("UNSAT") && report.contains("Hall witness"), "[{name}] report: {report}");
    }
}

/// The Studio routes Hardware-mode input to the pigeonhole easter egg via `is_pigeonhole_spec`. It
/// MUST fire on every pigeonhole example and on NONE of the SVA-design, RTL, or register-allocation
/// examples — and conversely no pigeonhole spec may route to the register allocator. This locks the
/// routing exhaustively over the whole shipped Hardware corpus (pigeonhole is checked FIRST in the
/// dispatch, so any overlap would silently hijack a real spec).
#[test]
fn pigeonhole_routing_is_exclusive_over_the_whole_corpus() {
    use logicaffeine_web::ui::examples::{PIGEONHOLE_EXAMPLES, REGALLOC_EXAMPLES};
    use logicaffeine_web::ui::pages::pigeonhole_viz::is_pigeonhole_spec;
    use logicaffeine_web::ui::pages::register_alloc_viz::is_register_alloc_spec;
    for (name, spec) in PIGEONHOLE_EXAMPLES {
        assert!(is_pigeonhole_spec(spec), "[{name}] must route to the pigeonhole solver");
        assert!(!is_register_alloc_spec(spec), "[{name}] must NOT route to the register allocator");
    }
    for (name, spec) in HARDWARE_EXAMPLES {
        assert!(!is_pigeonhole_spec(spec), "[{name}] SVA/design spec must NOT route to the pigeonhole solver");
    }
    for (name, src) in RTL_EXAMPLES {
        assert!(!is_pigeonhole_spec(src), "[{name}] Verilog must NOT route to the pigeonhole solver");
    }
    for (name, spec) in REGALLOC_EXAMPLES {
        assert!(!is_pigeonhole_spec(spec), "[{name}] register spec must NOT route to the pigeonhole solver");
    }
}

/// Every shipped RTL example must parse into a transition system and reach a DEFINITE verdict
/// (proven invariant, a counterexample, or bounded-clean) — never Unsupported, so the Verilog
/// → BMC tour can't ship a module that errors on Execute.
#[test]
fn every_rtl_example_parses_and_model_checks() {
    assert!(!RTL_EXAMPLES.is_empty(), "the RTL tour must ship examples");
    for (name, src) in RTL_EXAMPLES {
        // The Studio routes `module … endmodule` content to the RTL path.
        assert!(
            src.contains("module") && src.contains("endmodule"),
            "[{name}] must be Verilog the Studio recognizes as RTL"
        );
        let ts = parse_transition_system(src)
            .unwrap_or_else(|e| panic!("[{name}] RTL did not parse: {}", e.message));
        let inv = ts.prove_invariant(4);
        assert_ne!(inv, InductionOutcome::Unsupported, "[{name}] invariant check unsupported");
        if !matches!(inv, InductionOutcome::Proven) {
            assert_ne!(ts.bmc(12), BmcOutcome::Unsupported, "[{name}] BMC unsupported");
        }
    }
}
