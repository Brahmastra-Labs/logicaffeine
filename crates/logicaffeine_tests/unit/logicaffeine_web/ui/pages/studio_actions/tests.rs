use super::*;
use crate::ui::examples::{
    compiled_runtime_needs, Expected, ExampleSpec, ALL_CODE_EXAMPLES, ALL_HARDWARE_SPECS, ALL_LOGIC_EXAMPLES,
    ALL_MATH_EXAMPLES,
};
use crate::ui::pages::hardware_report::{compiled, Compiled};

fn of(mode: StudioMode, spec: &ExampleSpec) -> Actions {
    actions_for(mode, Some(spec.vfs_path), spec.source)
}

#[test]
fn a_program_that_needs_the_compiled_runtime_cannot_run_or_debug_but_compiles() {
    let native: Vec<_> = ALL_CODE_EXAMPLES
        .iter()
        .filter(|spec| matches!(spec.expected, Expected::NativeOnlyCompiles))
        .collect();
    assert!(!native.is_empty(), "the registry names no native-only program");
    for spec in native {
        let needs = compiled_runtime_needs(spec.vfs_path)
            .unwrap_or_else(|| panic!("{} does not say what it needs", spec.vfs_path));
        let actions = of(StudioMode::Code, spec);
        for (name, offer) in [("Run", &actions.run), ("Debug", &actions.debug)] {
            let why = offer.reason().unwrap_or_else(|| panic!("{}: {name} is offered, but the browser cannot run it", spec.vfs_path));
            assert!(why.contains(needs), "{}: {name}'s reason does not name {needs:?}: {why:?}", spec.vfs_path);
            assert!(why.contains("compiled runtime"), "{}: {name}'s reason does not say what runs it: {why:?}", spec.vfs_path);
        }
        assert!(actions.compile.is_ready(), "{}: Compile is its one way to use it, and it is withheld", spec.vfs_path);
    }
}

#[test]
fn every_other_program_runs_compiles_and_debugs() {
    for spec in ALL_CODE_EXAMPLES.iter().filter(|spec| !matches!(spec.expected, Expected::NativeOnlyCompiles)) {
        let actions = of(StudioMode::Code, spec);
        assert_eq!(actions, Actions { run: Offer::Ready, compile: Offer::Ready, debug: Offer::Ready }, "{}", spec.vfs_path);
    }
}

/// Compile is offered exactly when it makes Rust, and when it makes none its tooltip carries
/// the very note Compile would have shown.
#[test]
fn a_hardware_design_compiles_only_when_compile_makes_rust() {
    for spec in ALL_HARDWARE_SPECS {
        let actions = of(StudioMode::Hardware, spec);
        assert!(actions.run.is_ready(), "{}: Execute is withheld", spec.vfs_path);
        match compiled(spec.source) {
            Compiled::Rust(_) => assert!(actions.compile.is_ready(), "{}: Compile makes Rust but is withheld", spec.vfs_path),
            Compiled::Nothing(note) => {
                assert_eq!(actions.compile.reason(), Some(note), "{}: Compile makes nothing, so its tooltip says why", spec.vfs_path)
            }
            Compiled::Failed(e) => panic!("{}: a shipped design fails to compile: {e}", spec.vfs_path),
        }
    }
}

/// Compile is offered for a Logic file exactly when extraction has Rust to give — not for a
/// grid puzzle, which extraction leaves to Execute.
#[test]
fn a_logic_file_compiles_only_when_extraction_has_rust() {
    let mut withheld = 0;
    for spec in ALL_LOGIC_EXAMPLES {
        let actions = of(StudioMode::Logic, spec);
        assert!(actions.run.is_ready(), "{}: Execute is withheld", spec.vfs_path);
        let rust = logicaffeine_compile::extract_logic_rust(spec.source).unwrap_or_default();
        if rust.starts_with("// this is a finite-domain puzzle") {
            withheld += 1;
            let why = actions.compile.reason().unwrap_or_else(|| panic!("{}: Compile is offered for a grid puzzle", spec.vfs_path));
            assert!(why.contains("Execute"), "{}: the reason does not point to Execute: {why:?}", spec.vfs_path);
        } else {
            assert!(actions.compile.is_ready(), "{}: extraction has Rust but Compile is withheld", spec.vfs_path);
        }
    }
    assert!(withheld > 0, "no shipped Logic file is a grid puzzle — the rule is untested");
}

/// Compile is withheld for a Math file that declares nothing to extract — a `## Theory`
/// development, or a file of `Check`s and `Eval`s — and never for one extraction has Rust for.
/// (Whether a file's definitions are data or proofs is known only by running it, so a file of
/// proofs keeps Compile, which then says it found nothing to extract.)
#[test]
fn a_math_file_compiles_unless_it_declares_nothing() {
    let mut withheld = 0;
    let mut theories = 0;
    for spec in ALL_MATH_EXAMPLES {
        let actions = of(StudioMode::Math, spec);
        assert!(actions.run.is_ready(), "{}: Execute is withheld", spec.vfs_path);
        if spec.source.lines().any(|line| line.trim_start().starts_with("## Theory")) {
            theories += 1;
            assert!(!actions.compile.is_ready(), "{}: a theory has nothing to compile, yet Compile is offered", spec.vfs_path);
        }
        let Some(why) = actions.compile.reason() else { continue };
        withheld += 1;
        assert!(why.contains("Definition") && why.contains("Inductive"), "{}: the reason does not say what Compile needs: {why:?}", spec.vfs_path);
        let mut repl = logicaffeine_kernel::interface::Repl::new();
        for statement in crate::ui::pages::studio::parse_math_statements(spec.source) {
            let _ = repl.execute(&statement);
        }
        let rust = logicaffeine_compile::extract_math_rust(repl.context()).unwrap_or_default();
        assert!(rust.starts_with("// nothing defined yet"), "{}: Compile is withheld, but extraction has Rust for it", spec.vfs_path);
    }
    assert!(theories > 0, "no shipped Math file is a theory — the rule is untested");
    assert!(withheld > theories, "no shipped Math file of Checks and Evals is withheld — the rule is untested");
}

#[test]
fn an_empty_editor_offers_nothing_and_says_so() {
    for mode in [StudioMode::Logic, StudioMode::Code, StudioMode::Math, StudioMode::Hardware] {
        for content in ["", "   \n\t\n"] {
            let actions = actions_for(mode, None, content);
            for (name, offer) in [("run", &actions.run), ("compile", &actions.compile), ("debug", &actions.debug)] {
                let why = offer.reason().unwrap_or_else(|| panic!("{mode:?}: {name} is offered on an empty editor"));
                assert!(why.contains("empty"), "{mode:?}: {name}'s reason does not say the editor is empty: {why:?}");
            }
        }
    }
}

#[test]
fn every_withheld_action_says_why() {
    let every: Vec<(StudioMode, &ExampleSpec)> = ALL_CODE_EXAMPLES
        .iter()
        .map(|s| (StudioMode::Code, s))
        .chain(ALL_LOGIC_EXAMPLES.iter().map(|s| (StudioMode::Logic, s)))
        .chain(ALL_MATH_EXAMPLES.iter().map(|s| (StudioMode::Math, s)))
        .chain(ALL_HARDWARE_SPECS.iter().map(|s| (StudioMode::Hardware, s)))
        .collect();
    for (mode, spec) in every {
        let actions = of(mode, spec);
        for offer in [&actions.run, &actions.compile, &actions.debug] {
            if let Some(why) = offer.reason() {
                assert!(why.trim().len() > 10 && why.ends_with('.'), "{}: a withheld action's reason is not a sentence: {why:?}", spec.vfs_path);
            }
        }
    }
}

#[test]
fn a_user_file_is_judged_by_its_text() {
    let verilog = "module m(input clk);\n  reg a;\n  assert property (a == a);\nendmodule\n";
    assert!(!actions_for(StudioMode::Hardware, Some("/workspace/mine.v"), verilog).compile.is_ready());
    let spec = "Always, if request is high, then acknowledge is high.";
    assert!(actions_for(StudioMode::Hardware, Some("/workspace/mine.hw"), spec).compile.is_ready());
    let program = "## Main\nShow 1.\n";
    let actions = actions_for(StudioMode::Code, Some("/workspace/mine.logos"), program);
    assert!(actions.run.is_ready() && actions.debug.is_ready() && actions.compile.is_ready());
}
