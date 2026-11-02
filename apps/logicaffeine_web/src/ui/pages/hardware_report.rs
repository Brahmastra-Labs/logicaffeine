//! What Hardware mode makes of the editor's text: which kind of design it is, and everything the
//! Studio shows for it. One pure reading, so the panels, Execute and Compile can never disagree
//! about what the file is.

use logicaffeine_compile::codegen_sva::hw_pipeline::KgSummary;

/// The kinds of design Hardware mode reads, by what the text names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareKind {
    /// An English temporal spec ("Always, if request is high, …"), synthesized to an SVA.
    EnglishSpec,
    /// Movements and their conflicts, designed into a phase plan and a controller.
    SignalPlan,
    /// A Verilog module with an assertion, model-checked.
    Verilog,
    /// A `registers:` budget and live ranges, allocated and certified.
    RegisterAlloc,
    /// A `pigeons:` count, refuted and certified.
    Pigeonhole,
}

impl HardwareKind {
    /// The title of the panel that holds the result.
    pub fn result_title(self) -> &'static str {
        match self {
            HardwareKind::EnglishSpec => "SVA",
            HardwareKind::SignalPlan => "Signal Plan",
            HardwareKind::Verilog => "Model Check",
            HardwareKind::RegisterAlloc => "Allocation",
            HardwareKind::Pigeonhole => "Proof",
        }
    }

    /// Whether Compile makes something (a Rust runtime monitor), so the result panel offers a
    /// Rust view.
    pub fn compiles(self) -> bool {
        match self {
            HardwareKind::EnglishSpec => true,
            HardwareKind::SignalPlan | HardwareKind::Verilog | HardwareKind::RegisterAlloc | HardwareKind::Pigeonhole => false,
        }
    }
}

/// Which kind of design `content` is; `None` for an empty editor. A spec is the kind whose field
/// it names (`pigeons:`, `registers:`), whatever its values, so a mistake in one is reported as
/// that spec's mistake.
pub fn kind_of(content: &str) -> Option<HardwareKind> {
    use super::hardware_spec::{field_value, PIGEONS, REGISTERS};
    if content.trim().is_empty() {
        return None;
    }
    let names = |field: &str| content.lines().any(|line| field_value(line, field).is_some());
    Some(if names(PIGEONS) {
        HardwareKind::Pigeonhole
    } else if names(REGISTERS) {
        HardwareKind::RegisterAlloc
    } else if content.contains("module") && content.contains("endmodule") {
        HardwareKind::Verilog
    } else if content.to_lowercase().contains("conflict") {
        HardwareKind::SignalPlan
    } else {
        HardwareKind::EnglishSpec
    })
}

/// Whether the editor holds Verilog: a line that opens a `module`, or an `endmodule` — known from
/// the first line of a module being written, before [`kind_of`] can call it finished Verilog.
pub fn is_verilog(content: &str) -> bool {
    content.contains("endmodule")
        || content.lines().any(|line| line.trim_start().strip_prefix("module").is_some_and(|rest| rest.starts_with([' ', '(', '#'])))
}

/// One labelled block of the result panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub label: &'static str,
    pub body: String,
}

/// Everything the Studio shows for a Hardware design.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HardwareReport {
    pub sections: Vec<Section>,
    /// The verdict line and whether it is a success, a failure, or neither.
    pub verdict: Option<(String, Option<bool>)>,
    /// A counterexample when the verdict fails, a witness run when it holds.
    pub trace: Vec<(String, String)>,
    pub signals: Vec<String>,
    pub kg: KgSummary,
    pub error: Option<String>,
}

/// Read `content` as its kind of design.
pub fn analyze(content: &str) -> Option<(HardwareKind, HardwareReport)> {
    let kind = kind_of(content)?;
    let report = match kind {
        HardwareKind::Pigeonhole => pigeonhole(content),
        HardwareKind::RegisterAlloc => allocation(content),
        HardwareKind::Verilog => verilog(content),
        HardwareKind::SignalPlan => signal_plan(content),
        HardwareKind::EnglishSpec => english_spec(content),
    };
    Some((kind, report))
}

fn failed(error: String) -> HardwareReport {
    HardwareReport { error: Some(error), ..HardwareReport::default() }
}

/// The first problem the spec's own service finds, as the report's error.
fn spec_problem(content: &str) -> Option<String> {
    super::hardware_spec::problems(content).into_iter().next().map(|p| p.message)
}

fn pigeonhole(content: &str) -> HardwareReport {
    use super::pigeonhole_viz::{headline, parse_pigeonhole_spec, report};
    match parse_pigeonhole_spec(content) {
        Some(spec) => HardwareReport {
            sections: vec![Section { label: "Certified pigeonhole refutation", body: report(&spec) }],
            verdict: Some((headline(&spec), Some(true))),
            ..HardwareReport::default()
        },
        None => failed(spec_problem(content).unwrap_or_else(|| "A pigeonhole spec is one line, `pigeons: N`.".to_string())),
    }
}

fn allocation(content: &str) -> HardwareReport {
    use super::register_alloc_viz::{allocation_report, linear_scan_assignment, parse_register_spec, render};
    match parse_register_spec(content) {
        Some(spec) if spec_problem(content).is_none() => {
            let fits = linear_scan_assignment(&spec).iter().all(Option::is_some);
            let (_, verdict) = render(&spec);
            HardwareReport {
                sections: vec![Section { label: "Certified register allocation", body: allocation_report(&spec) }],
                verdict: Some((verdict, Some(fits))),
                ..HardwareReport::default()
            }
        }
        _ => failed(spec_problem(content).unwrap_or_else(|| {
            "A register allocation needs at least one live range, `name: first-last`, under its `registers:` budget.".to_string()
        })),
    }
}

fn verilog(content: &str) -> HardwareReport {
    use logicaffeine_compile::codegen_sva::rtl::parse_transition_system;
    use logicaffeine_proof::bmc::{BmcOutcome, InductionOutcome};
    let ts = match parse_transition_system(content) {
        Ok(ts) => ts,
        Err(e) => return failed(format!("RTL parse error: {}", e.message)),
    };
    let names: Vec<String> = ts
        .registers
        .iter()
        .map(|r| if r.width > 1 { format!("{}[{}]", r.name, r.width) } else { r.name.clone() })
        .collect();
    let summary = format!(
        "{} {}: {}.\nChecked in every reachable state by bounded model checking and k-induction, with no Z3.",
        names.len(),
        if names.len() == 1 { "register" } else { "registers" },
        names.join(", ")
    );
    let to_trace = |trace: Vec<(String, bool)>| -> Vec<(String, String)> {
        trace.into_iter().map(|(n, v)| (n, if v { "1" } else { "0" }.to_string())).collect()
    };
    let breaks = |k: u32| format!("\u{2717} Can break at step {k}: the waveform shows the run that breaks it");
    let unexpressible = || ("This property isn't expressible yet. Try a simpler one.".to_string(), None);
    let (verdict, trace) = match ts.prove_invariant(4) {
        InductionOutcome::Proven => (
            ("\u{2713} Always holds: proven for every reachable state (k-induction, no Z3)".to_string(), Some(true)),
            ts.witness_trace(18).map(to_trace).unwrap_or_default(),
        ),
        InductionOutcome::CounterexampleAt { k, trace } => ((breaks(k), Some(false)), to_trace(trace)),
        InductionOutcome::NotInductive => match ts.bmc(28) {
            BmcOutcome::CounterexampleAt { k, trace } => ((breaks(k), Some(false)), to_trace(trace)),
            BmcOutcome::NoneWithin(n) => (
                (format!("No failure found within {n} steps, but this depth can't prove it always holds"), None),
                Vec::new(),
            ),
            BmcOutcome::Unsupported => (unexpressible(), Vec::new()),
        },
        InductionOutcome::Unsupported => (unexpressible(), Vec::new()),
    };
    HardwareReport {
        sections: vec![Section { label: "Transition system", body: summary }],
        verdict: Some(verdict),
        trace,
        ..HardwareReport::default()
    }
}

fn signal_plan(content: &str) -> HardwareReport {
    use logicaffeine_compile::codegen_sva::controller_gen::generate_controller;
    use logicaffeine_compile::codegen_sva::rtl::parse_transition_system;
    use logicaffeine_compile::codegen_sva::signal_design::design_from_spec;
    use logicaffeine_proof::bmc::InductionOutcome;
    let (intersection, plan) = match design_from_spec(content) {
        Ok(designed) => designed,
        Err(e) => return failed(e),
    };
    let phases: Vec<String> = plan
        .groups()
        .iter()
        .enumerate()
        .map(|(p, group)| format!("Phase {}: {}", p + 1, intersection.names(group).join(", ")))
        .collect();
    let verilog = generate_controller(&intersection, &plan);
    let controller_proven = matches!(parse_transition_system(&verilog).map(|ts| ts.prove_invariant(4)), Ok(InductionOutcome::Proven));
    let minimal = if !plan.minimal_certified {
        String::new()
    } else if plan.num_phases <= 1 {
        " \u{00b7} a single phase suffices".to_string()
    } else {
        format!(" \u{00b7} provably minimal ({}-phase impossible)", plan.num_phases - 1)
    };
    let controller_note = if controller_proven { " \u{00b7} generated controller PROVEN conflict-free (k-induction)" } else { "" };
    HardwareReport {
        sections: vec![
            Section { label: "Phase plan", body: phases.join("\n") },
            Section { label: "Generated controller", body: verilog.trim_end().to_string() },
        ],
        verdict: Some((
            format!(
                "\u{2713} Conflict-free {}-phase plan synthesized{minimal}{controller_note} (certified by our SAT solver, no Z3)",
                plan.num_phases
            ),
            Some(true),
        )),
        signals: intersection.movements.clone(),
        ..HardwareReport::default()
    }
}

/// The SVA property a synthesis names, for the PSL and Rust emitters.
fn property(synth: &logicaffeine_compile::codegen_sva::fol_to_sva::SynthesizedSva) -> logicaffeine_compile::codegen_sva::SvaProperty {
    use logicaffeine_compile::codegen_sva::{sanitize_property_name, SvaAssertionKind, SvaProperty};
    let kind = match synth.kind.as_str() {
        "cover" => SvaAssertionKind::Cover,
        "assume" => SvaAssertionKind::Assume,
        _ => SvaAssertionKind::Assert,
    };
    let name = if synth.signals.is_empty() { "p_property".to_string() } else { sanitize_property_name(&synth.signals.join("_")) };
    SvaProperty { name, clock: "clk".to_string(), body: synth.body.clone(), kind }
}

fn english_spec(content: &str) -> HardwareReport {
    use logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec;
    use logicaffeine_compile::codegen_sva::hw_pipeline::{check_spec_vacuity, kg_summary, prove_spec_sva_equivalence, VacuityReport};
    let synth = match synthesize_sva_from_spec(content, "clk") {
        Ok(synth) => synth,
        Err(e) => return failed(e),
    };
    let psl = logicaffeine_compile::codegen_sva::emit_psl_property(&property(&synth));
    let vacuity = match check_spec_vacuity(content, 8) {
        Ok(VacuityReport::Vacuous) => " \u{00b7} \u{26A0} never triggers (vacuous)",
        Ok(VacuityReport::NonVacuous) => " \u{00b7} the trigger can actually fire",
        _ => "",
    };
    let (verdict, trace) = match prove_spec_sva_equivalence(content, &synth.body, 8) {
        Ok(r) if r.equivalent => ((format!("\u{2713} Matches the spec: certified, kernel-checked, no Z3{vacuity}"), Some(true)), Vec::new()),
        Ok(r) => {
            let trace = r.counterexample.unwrap_or_default();
            let shown = trace.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ");
            ((format!("\u{2717} Not equivalent to the spec. Counterexample: {shown}{vacuity}"), Some(false)), trace)
        }
        Err(e) => ((format!("Proof unavailable: {e}{vacuity}"), None), Vec::new()),
    };
    HardwareReport {
        sections: vec![
            Section { label: "SystemVerilog Assertion", body: synth.sva_text.clone() },
            Section { label: "PSL", body: psl },
        ],
        verdict: Some(verdict),
        trace,
        signals: synth.signals.clone(),
        kg: kg_summary(content).unwrap_or_default(),
        error: None,
    }
}

/// What Compile makes of a design.
#[derive(Clone, Debug, PartialEq)]
pub enum Compiled {
    /// A Rust runtime monitor.
    Rust(String),
    /// Nothing to compile, and why: the note the Studio shows instead.
    Nothing(&'static str),
    /// The design could not be read.
    Failed(String),
}

/// Why Compile makes nothing for a design of `kind` — `None` for an English spec, which compiles to
/// a Rust runtime monitor. The toolbar shows the same words on the disabled button.
pub fn nothing_to_compile(kind: HardwareKind) -> Option<&'static str> {
    match kind {
        HardwareKind::EnglishSpec => None,
        HardwareKind::SignalPlan => Some(
            "A signal plan compiles to hardware, not Rust: its generated controller is the Verilog in the Signal Plan panel, proven conflict-free.",
        ),
        HardwareKind::Verilog => Some(
            "Verilog is already hardware, so there is no Rust monitor to generate. Execute model-checks its assertion.",
        ),
        HardwareKind::RegisterAlloc => Some("A register allocation has nothing to compile. Execute certifies it."),
        HardwareKind::Pigeonhole => Some("A pigeonhole instance has nothing to compile. Execute certifies its refutation."),
    }
}

/// Compile `content`: an English spec becomes a Rust runtime monitor; every other design says why
/// there is nothing to compile, instead of blaming the design.
pub fn compiled(content: &str) -> Compiled {
    let Some(kind) = kind_of(content) else { return Compiled::Nothing("Write a design to compile.") };
    if let Some(note) = nothing_to_compile(kind) {
        return Compiled::Nothing(note);
    }
    match logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec(content, "clk") {
        Ok(synth) => Compiled::Rust(logicaffeine_compile::codegen_sva::emit_rust_monitor(&property(&synth))),
        Err(e) => Compiled::Failed(e),
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/hardware_report/tests.rs"]
mod tests;
