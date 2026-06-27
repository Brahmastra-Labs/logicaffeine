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
