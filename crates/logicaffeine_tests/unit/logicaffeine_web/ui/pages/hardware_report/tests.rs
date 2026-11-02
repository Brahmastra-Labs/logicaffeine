use super::*;
use crate::ui::examples::{Expected, ALL_HARDWARE_SPECS};

fn source(file: &str) -> &'static str {
    ALL_HARDWARE_SPECS
        .iter()
        .find(|spec| spec.vfs_path.ends_with(file))
        .unwrap_or_else(|| panic!("no shipped {file}"))
        .source
}

#[test]
fn a_module_is_verilog_from_its_first_line() {
    assert!(is_verilog("module m(input clk);"), "a module being written");
    assert!(is_verilog("// an arbiter\nmodule arbiter (input clk);\n  reg g;"));
    assert!(is_verilog("module m #(parameter W = 8) (input clk);"), "a parameterised module");
    assert!(is_verilog("  reg q;\nendmodule"), "a module whose header was deleted is still Verilog");
    for spec in ALL_HARDWARE_SPECS.iter().filter(|s| s.vfs_path.ends_with(".v")) {
        assert!(is_verilog(spec.source), "{} is Verilog", spec.vfs_path);
    }
}

#[test]
fn english_that_mentions_a_module_is_not_verilog() {
    for text in [
        "Always, if request is high, then acknowledge is high.",
        "Movements: ns-through, ew-through.\nns-through conflicts with ew-through.",
        "The module is idle until start is high.",
        "Module A is high.",
        "modules: 3",
    ] {
        assert!(!is_verilog(text), "{text:?} is not Verilog");
    }
    for spec in ALL_HARDWARE_SPECS.iter().filter(|s| s.vfs_path.ends_with(".hw")) {
        assert!(!is_verilog(spec.source), "{} is a spec", spec.vfs_path);
    }
}

fn report(file: &str) -> (HardwareKind, HardwareReport) {
    analyze(source(file)).unwrap_or_else(|| panic!("{file} reads as no design"))
}

fn labels(report: &HardwareReport) -> Vec<&'static str> {
    report.sections.iter().map(|s| s.label).collect()
}

#[test]
fn every_hardware_example_is_the_kind_its_registry_says() {
    let mut wrong = Vec::new();
    for spec in ALL_HARDWARE_SPECS {
        let expected = match spec.expected {
            Expected::SvaSynthesizes => HardwareKind::EnglishSpec,
            Expected::SignalPlanSynthesizes => HardwareKind::SignalPlan,
            Expected::RtlProven | Expected::RefutesWithCounterexample => HardwareKind::Verilog,
            Expected::RegisterAllocFits | Expected::SpillsRequired => HardwareKind::RegisterAlloc,
            Expected::UnsatCertified => HardwareKind::Pigeonhole,
            _ => panic!("{} carries a promise Hardware mode does not keep", spec.vfs_path),
        };
        if kind_of(spec.source) != Some(expected) {
            wrong.push(format!("{}: {:?}, not {expected:?}", spec.vfs_path, kind_of(spec.source)));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn an_empty_editor_is_no_design() {
    assert_eq!(kind_of(""), None);
    assert_eq!(kind_of("  \n\t\n"), None);
    assert!(analyze("").is_none());
}

/// A spec that names a field is that spec even when a value is wrong, and says what is wrong,
/// instead of falling through to the English reader's "not a hardware property".
#[test]
fn a_spec_that_names_a_field_is_that_spec_even_when_a_value_is_wrong() {
    let (kind, report) = analyze("pigeons: 99\n").expect("a design");
    assert_eq!(kind, HardwareKind::Pigeonhole);
    assert!(report.error.as_deref().is_some_and(|e| e.contains("out of range")), "{:?}", report.error);
    let (kind, report) = analyze("registers: 3\n").expect("a design");
    assert_eq!(kind, HardwareKind::RegisterAlloc);
    assert!(report.error.as_deref().is_some_and(|e| e.contains("live range")), "{:?}", report.error);
}

#[test]
fn each_kind_titles_its_result_panel_for_what_it_holds() {
    assert_eq!(HardwareKind::EnglishSpec.result_title(), "SVA");
    assert_eq!(HardwareKind::SignalPlan.result_title(), "Signal Plan");
    assert_eq!(HardwareKind::Verilog.result_title(), "Model Check");
    assert_eq!(HardwareKind::RegisterAlloc.result_title(), "Allocation");
    assert_eq!(HardwareKind::Pigeonhole.result_title(), "Proof");
}

#[test]
fn an_english_spec_is_synthesized_to_an_sva() {
    let (kind, report) = report("handshake.hw");
    assert_eq!(kind, HardwareKind::EnglishSpec);
    assert_eq!(labels(&report), ["SystemVerilog Assertion", "PSL"]);
    assert!(report.sections[0].body.contains("assert property"), "{:?}", report.sections[0]);
    assert_eq!(report.verdict.as_ref().map(|v| v.1), Some(Some(true)), "{:?}", report.verdict);
}

#[test]
fn a_signal_plan_is_labelled_as_one() {
    let (kind, report) = report("intersection-design.hw");
    assert_eq!(kind, HardwareKind::SignalPlan);
    assert_eq!(labels(&report), ["Phase plan", "Generated controller"]);
    assert!(report.sections[0].body.starts_with("Phase 1:"), "{:?}", report.sections[0]);
    assert!(report.sections[1].body.contains("module controller"), "{:?}", report.sections[1]);
    assert!(!report.signals.is_empty());
    assert_eq!(report.verdict.as_ref().map(|v| v.1), Some(Some(true)), "{:?}", report.verdict);
}

#[test]
fn verilog_is_model_checked_not_called_an_sva() {
    let (kind, report) = report("arbiter.v");
    assert_eq!(kind, HardwareKind::Verilog);
    assert_eq!(labels(&report), ["Transition system"]);
    let summary = &report.sections[0].body;
    assert!(summary.contains("g0") && summary.contains("turn"), "the summary names the registers: {summary}");
    assert!(!summary.contains("//"), "the summary is prose, not code comments: {summary}");
    assert_eq!(report.verdict.as_ref().map(|v| v.1), Some(Some(true)), "{:?}", report.verdict);
    assert!(!report.trace.is_empty(), "a proven design animates a witness run");
}

/// The counterexample is drawn in the next panel, not below the verdict.
#[test]
fn a_counterexample_verdict_points_at_the_waveform() {
    for file in ["queue-jam.v", "bad-arbiter.v", "traffic-crash.v", "door-interlock-bug.v"] {
        let (_, report) = report(file);
        let (text, ok) = report.verdict.clone().unwrap_or_else(|| panic!("{file} has no verdict"));
        assert_eq!(ok, Some(false), "{file}: {text}");
        assert!(text.contains("step") && text.contains("waveform"), "{file}: {text}");
        assert!(!text.contains("below"), "{file}: {text}");
        assert!(!report.trace.is_empty(), "{file} has no counterexample to draw");
    }
}

#[test]
fn a_pigeonhole_refutation_is_a_certified_success() {
    let (_, report) = report("pigeonhole.hw");
    let (text, ok) = report.verdict.clone().expect("a verdict");
    assert_eq!(ok, Some(true), "{text}");
    assert!(text.starts_with("\u{2713} Certified UNSAT"), "{text}");
    assert_eq!(labels(&report), ["Certified pigeonhole refutation"]);
}

#[test]
fn an_allocation_carries_its_certified_report() {
    let (_, fits) = report("register-alloc-fits.hw");
    assert_eq!(labels(&fits), ["Certified register allocation"]);
    assert_eq!(fits.verdict.as_ref().map(|v| v.1), Some(Some(true)), "{:?}", fits.verdict);
    let (_, spills) = report("register-alloc-spill.hw");
    assert_eq!(spills.verdict.as_ref().map(|v| v.1), Some(Some(false)), "{:?}", spills.verdict);
}

#[test]
fn only_an_english_spec_compiles_to_rust() {
    assert!(HardwareKind::EnglishSpec.compiles());
    for kind in [HardwareKind::SignalPlan, HardwareKind::Verilog, HardwareKind::RegisterAlloc, HardwareKind::Pigeonhole] {
        assert!(!kind.compiles(), "{kind:?}");
    }
    match compiled(source("handshake.hw")) {
        Compiled::Rust(rust) => assert!(rust.contains("Monitor"), "{rust}"),
        other => panic!("the English spec did not compile: {other:?}"),
    }
}

/// Compile on a design with no Rust to make says so; it never tells the reader their
/// Verilog or signal plan is "not a hardware property".
#[test]
fn compile_never_blames_a_design_that_has_no_rust() {
    for file in ["arbiter.v", "intersection-design.hw", "register-alloc-fits.hw", "pigeonhole.hw"] {
        match compiled(source(file)) {
            Compiled::Nothing(note) => {
                assert!(!note.is_empty(), "{file}");
                assert!(!note.contains("not a hardware property"), "{file}: {note}");
            }
            other => panic!("{file}: {other:?}"),
        }
    }
}
