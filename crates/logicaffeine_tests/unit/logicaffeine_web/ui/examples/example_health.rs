//! Every shipped Studio example is locked to its documented intended outcome
//! by driving it through the SAME pipeline the Studio uses for its mode.
//! This is the single-source guard: the tests iterate the `ALL_*` registries
//! (the same specs `seed_examples` writes to the VFS), so a broken example
//! cannot ship unnoticed.
use super::*;
use logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec;
use logicaffeine_compile::codegen_sva::hw_pipeline::prove_spec_sva_equivalence;
use logicaffeine_compile::codegen_sva::rtl::parse_transition_system;
use logicaffeine_compile::codegen_sva::signal_design::design_from_spec;
use logicaffeine_compile::{
    compile_for_ui, compile_theorem_for_ui, generate_rust_code, interpret_for_ui_baseline_with_args,
};
use logicaffeine_kernel::interface::Repl;
use logicaffeine_proof::bmc::{BmcOutcome, InductionOutcome};
use logicaffeine_proof::register_alloc::{
    allocate, is_spill_certificate, is_valid_allocation, Allocation,
};

/// Drive one example through its real Studio pipeline and check its
/// intended outcome. Returns `Ok(())` on a pass, `Err(reason)` otherwise.
fn check(spec: &ExampleSpec) -> Result<(), String> {
    match &spec.expected {
        Expected::CompilesToFol(needles) => {
            let r = compile_for_ui(spec.source);
            if let Some(e) = r.error {
                return Err(format!("compile error: {e}"));
            }
            let logic = r.logic.unwrap_or_default();
            if logic.trim().is_empty() {
                return Err("no FOL produced".to_string());
            }
            let simple = r.simple_logic.unwrap_or_default();
            for needle in *needles {
                if !logic.contains(needle) {
                    return Err(format!("primary FOL missing {needle:?}:\n{logic}"));
                }
                if !simple.contains(needle) {
                    return Err(format!("Simple FOL missing {needle:?}:\n{simple}"));
                }
            }
            Ok(())
        }
        Expected::Proves => {
            let r = compile_theorem_for_ui(spec.source);
            if r.verified {
                Ok(())
            } else {
                Err(format!(
                    "theorem '{}' not kernel-verified{}",
                    r.name,
                    r.verification_error
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                ))
            }
        }
        Expected::DerivationNotCertified => {
            let r = compile_theorem_for_ui(spec.source);
            match (r.derivation.is_some(), r.verified) {
                (true, false) => Ok(()),
                (true, true) => Err(
                    "expected an uncertified derivation (honesty demo) but it certified"
                        .to_string(),
                ),
                (false, _) => Err("expected a derivation to be found, none was".to_string()),
            }
        }
        Expected::OutputContains(needles) => {
            // Both Studio Code surfaces must work: it generates Rust (the
            // 🦀 Compile view) AND it runs (the interpreter view).
            match generate_rust_code(spec.source) {
                Ok(rust) if !rust.trim().is_empty() => {}
                Ok(_) => return Err("generated empty Rust".to_string()),
                Err(e) => return Err(format!("codegen error: {e:?}")),
            }
            let result = run_code(spec.source);
            if let Some(e) = result.error {
                return Err(format!("interpreter error: {e}"));
            }
            let out = result.lines.join("\n");
            for needle in *needles {
                if !out.contains(needle) {
                    return Err(format!("output missing {needle:?}; actual output:\n{out}"));
                }
            }
            Ok(())
        }
        Expected::OutputExact(expected) => {
            match generate_rust_code(spec.source) {
                Ok(rust) if !rust.trim().is_empty() => {}
                Ok(_) => return Err("generated empty Rust".to_string()),
                Err(e) => return Err(format!("codegen error: {e:?}")),
            }
            let result = run_code(spec.source);
            if let Some(e) = result.error {
                return Err(format!("interpreter error: {e}"));
            }
            let out = result.lines.join("\n");
            if out != expected.trim_end_matches('\n') {
                return Err(format!(
                    "exact output mismatch;\n--- expected ---\n{}\n--- actual ---\n{out}\n--- end ---",
                    expected.trim_end_matches('\n')
                ));
            }
            Ok(())
        }
        Expected::NativeOnlyCompiles => match generate_rust_code(spec.source) {
            Ok(rust) if !rust.trim().is_empty() => Ok(()),
            Ok(_) => Err("generated empty Rust".to_string()),
            Err(e) => Err(format!("codegen error: {e:?}")),
        },
        // A page compiles to a self-contained wasm-dom module — the path the Studio's Web
        // preview (`assemble_preview`) runs — and its own tests pass.
        Expected::WebPage => {
            match logicaffeine_compile::compile::compile_to_wasm(spec.source) {
                Ok(bytes) if bytes.starts_with(b"\0asm") => {}
                Ok(_) => return Err("wasm-dom module lacks the wasm magic".to_string()),
                Err(e) => return Err(format!("the page failed to compile to wasm-dom: {e:?}")),
            }
            crate::ui::pages::guide::page_verdict(spec.source)
                .map(|_| ())
                .map_err(|e| format!("the page's tests do not pass: {e}"))
        }
        Expected::KernelAllStatementsOk => run_kernel(spec.source, None),
        Expected::KernelAdmitsAt(marker) => run_kernel(spec.source, Some(marker)),
        Expected::TheoryAllVerified => {
            let r = logicaffeine_compile::ui_bridge::compile_theory_for_ui(spec.source);
            if let Some(e) = r.parse_error {
                return Err(format!("theory parse error: {e}"));
            }
            let failed: Vec<String> = r
                .theorems
                .iter()
                .filter(|t| !t.verified)
                .map(|t| format!("{}: {}", t.name, t.error.clone().unwrap_or_default()))
                .collect();
            if r.theorems.is_empty() {
                return Err("theory contained no theorems".to_string());
            }
            if failed.is_empty() {
                Ok(())
            } else {
                Err(format!("uncertified theorems:\n{}", failed.join("\n")))
            }
        }
        Expected::SvaSynthesizes => {
            let synth = synthesize_sva_from_spec(spec.source, "clk")
                .map_err(|e| format!("SVA synthesis failed: {e}"))?;
            let equiv = prove_spec_sva_equivalence(spec.source, &synth.body, 8)
                .map_err(|e| format!("equivalence check failed: {e:?}"))?;
            if equiv.equivalent {
                Ok(())
            } else {
                Err("synthesized SVA not certified equivalent to spec".to_string())
            }
        }
        Expected::SignalPlanSynthesizes => {
            let (_intersection, plan) =
                design_from_spec(spec.source).map_err(|e| format!("signal-design failed: {e}"))?;
            if plan.num_phases >= 1 {
                Ok(())
            } else {
                Err("signal-design produced no phases".to_string())
            }
        }
        Expected::RtlProven => {
            let ts = parse_transition_system(spec.source)
                .map_err(|e| format!("RTL parse failed: {e:?}"))?;
            match ts.prove_invariant(4) {
                InductionOutcome::Proven => Ok(()),
                InductionOutcome::NotInductive => match ts.bmc(28) {
                    BmcOutcome::NoneWithin(_) => Ok(()),
                    other => Err(format!("expected safe, BMC found {other:?}")),
                },
                other => Err(format!("expected Proven, got {other:?}")),
            }
        }
        Expected::RefutesWithCounterexample => {
            let ts = parse_transition_system(spec.source)
                .map_err(|e| format!("RTL parse failed: {e:?}"))?;
            let inv = ts.prove_invariant(4);
            if matches!(inv, InductionOutcome::CounterexampleAt { .. }) {
                return Ok(());
            }
            if matches!(inv, InductionOutcome::NotInductive) {
                if matches!(ts.bmc(28), BmcOutcome::CounterexampleAt { .. }) {
                    return Ok(());
                }
            }
            Err(format!(
                "expected a counterexample, prove_invariant gave {inv:?}"
            ))
        }
        Expected::RegisterAllocFits => {
            let regspec = crate::ui::pages::register_alloc_viz::parse_register_spec(spec.source)
                .ok_or("could not parse register spec")?;
            match allocate(&regspec.ranges, regspec.registers) {
                Allocation::Allocated(reg_of) => {
                    if is_valid_allocation(&regspec.ranges, regspec.registers, &reg_of) {
                        Ok(())
                    } else {
                        Err("allocation is not valid".to_string())
                    }
                }
                Allocation::Spill { .. } => {
                    Err("expected a fitting allocation, got a spill".to_string())
                }
            }
        }
        Expected::SpillsRequired => {
            let regspec = crate::ui::pages::register_alloc_viz::parse_register_spec(spec.source)
                .ok_or("could not parse register spec")?;
            match allocate(&regspec.ranges, regspec.registers) {
                Allocation::Spill { must_spill, .. } => {
                    if is_spill_certificate(&regspec.ranges, regspec.registers, &must_spill) {
                        Ok(())
                    } else {
                        Err("spill certificate does not re-verify".to_string())
                    }
                }
                Allocation::Allocated(_) => {
                    Err("expected a required spill, got a fitting allocation".to_string())
                }
            }
        }
        Expected::UnsatCertified => {
            let pspec = crate::ui::pages::pigeonhole_viz::parse_pigeonhole_spec(spec.source)
                .ok_or("could not parse pigeonhole spec")?;
            let verdict = crate::ui::pages::pigeonhole_viz::solve(&pspec);
            if verdict.certified && !verdict.hall.slots.is_empty() {
                Ok(())
            } else {
                Err("pigeonhole UNSAT was not certified".to_string())
            }
        }
    }
}

/// Run a code example through the async baseline interpreter — the exact
/// entry the Studio uses — inside a Tokio runtime so programs that sleep or
/// drive the scheduler run for real (the sync entry has no reactor and would
/// panic on `Sleep`).
fn run_code(src: &str) -> logicaffeine_compile::interpreter::InterpreterResult {
    let rt = logicaffeine_system::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime for code example");
    rt.block_on(interpret_for_ui_baseline_with_args(src, &[]))
}

/// Run every statement of a math example through a fresh persistent kernel
/// `Repl` (exactly as the Studio does). With `admit_marker`, statements are
/// allowed to fail from the first statement containing the marker onward
/// (the documented open/admitted point).
fn run_kernel(source: &str, admit_marker: Option<&str>) -> Result<(), String> {
    let mut repl = Repl::new();
    let mut admitted = false;
    for stmt in crate::ui::pages::studio::parse_math_statements(source) {
        if let Some(marker) = admit_marker {
            if stmt.contains(marker) {
                admitted = true;
            }
        }
        if let Err(e) = repl.execute(&stmt) {
            if admitted {
                return Ok(());
            }
            return Err(format!("statement failed: {stmt}\n  -> {e}"));
        }
    }
    if admit_marker.is_some() && !admitted {
        return Err("admit marker never matched any statement".to_string());
    }
    Ok(())
}

/// Assert every spec in a registry passes `check`, collecting ALL failures so
/// one run surfaces every broken example at once.
fn lock(registry: &[ExampleSpec], mode: &str) {
    let mut failures = Vec::new();
    for spec in registry {
        if let Err(reason) = check(spec) {
            failures.push(format!("  {}: {reason}", spec.vfs_path));
        }
    }
    assert!(
        failures.is_empty(),
        "{} {mode} example(s) failed their lock:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn lock_logic_examples() {
    lock(ALL_LOGIC_EXAMPLES, "logic");
}

#[test]
fn lock_code_examples() {
    lock(ALL_CODE_EXAMPLES, "code");
}

#[test]
fn lock_math_examples() {
    lock(ALL_MATH_EXAMPLES, "math");
}

#[test]
fn lock_hardware_examples() {
    lock(ALL_HARDWARE_SPECS, "hardware");
}

/// Every registry entry, in one iterator — the single source of truth that
/// both `seed_examples` and the locks above consume.
fn all_specs() -> impl Iterator<Item = &'static ExampleSpec> {
    ALL_LOGIC_EXAMPLES
        .iter()
        .chain(ALL_CODE_EXAMPLES)
        .chain(ALL_MATH_EXAMPLES)
        .chain(ALL_HARDWARE_SPECS)
}

/// A page opens where it runs: every `WebPage` example opens in the Web tab and every other
/// Code example in Code — the Studio's open-file path reads the same `StudioMode::for_logos_source`.
#[test]
fn web_pages_open_in_the_web_tab_and_programs_in_code() {
    use crate::ui::state::StudioMode;
    for spec in ALL_CODE_EXAMPLES {
        let want = if matches!(spec.expected, Expected::WebPage) {
            StudioMode::Web
        } else {
            StudioMode::Code
        };
        assert_eq!(
            StudioMode::for_logos_source(spec.source),
            want,
            "{} opens in the wrong Studio mode",
            spec.vfs_path
        );
    }
    assert!(
        ALL_CODE_EXAMPLES
            .iter()
            .filter(|s| matches!(s.expected, Expected::WebPage))
            .count()
            >= 3,
        "the Studio ships its web pages"
    );
}

/// The Studio's Syntax Tree for every Logic example is written for a reader: no node is a
/// Rust `Debug` dump (`NeoEvent(NeoEventData { event_var: Symbol(55), … })`).
#[test]
fn every_logic_example_has_a_readable_syntax_tree() {
    fn labels(node: &crate::AstNode, out: &mut Vec<String>) {
        out.push(node.label.clone());
        for child in &node.children {
            labels(child, out);
        }
    }
    let raw = [
        "Data {",
        "Symbol(",
        ": [(",
        "suppress_existential",
        "Variable(",
    ];
    let mut dumps = Vec::new();
    for spec in ALL_LOGIC_EXAMPLES {
        let mut all = Vec::new();
        if let Some(ast) = logicaffeine_compile::compile_for_ui(spec.source).ast {
            labels(&ast, &mut all);
        }
        for label in all
            .into_iter()
            .filter(|l| raw.iter().any(|r| l.contains(r)))
        {
            dumps.push(format!(
                "{}: {}",
                spec.vfs_path,
                label.chars().take(80).collect::<String>()
            ));
        }
    }
    assert!(
        dumps.is_empty(),
        "{} syntax tree node(s) are debug dumps:\n{}",
        dumps.len(),
        dumps.join("\n")
    );
}

/// A connective in the Syntax Tree names itself: `→`, never the `?` of an operator the tree's
/// symbol table did not know ("If any dog barks, John runs." drew two).
#[test]
fn every_syntax_tree_connective_has_its_symbol() {
    fn unknown(node: &crate::AstNode, path: &str, out: &mut Vec<String>) {
        if node.label == "?" && node.node_type != "question" {
            out.push(format!("{path}: a {} drawn as ?", node.node_type));
        }
        for child in &node.children {
            unknown(child, path, out);
        }
    }
    let mut found = Vec::new();
    for spec in ALL_LOGIC_EXAMPLES {
        if let Some(ast) = logicaffeine_compile::compile_for_ui(spec.source).ast {
            unknown(&ast, spec.vfs_path, &mut found);
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// Every Code example builds through the compiler's own path — `compile_to_rust`, which
/// type-checks, as `largo build` does — not only the Studio's Compile view. The generics
/// example read a field its struct had silently lost, and failed here while the
/// interpreter ran it. A page (Web mode) is not a native program: the AOT-Rust backend
/// declines the DOM by design, and the run lock above covers it on the tree-walker.
#[test]
fn every_code_example_compiles_through_the_type_checked_path() {
    use crate::ui::state::StudioMode;
    let failures: Vec<String> = ALL_CODE_EXAMPLES
        .iter()
        .filter(|spec| StudioMode::for_path(spec.vfs_path) != Some(StudioMode::Web))
        .filter_map(|spec| {
            logicaffeine_compile::compile::compile_to_rust(spec.source)
                .err()
                .map(|e| format!("{}: {:?}", spec.vfs_path, e.kind))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} example(s) do not compile:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Ratchet: the per-mode counts are locked, so an example cannot be added or
/// removed without deliberately updating this test.
#[test]
fn registry_counts_are_locked() {
    assert_eq!(ALL_LOGIC_EXAMPLES.len(), 25, "logic example count changed");
    assert_eq!(ALL_CODE_EXAMPLES.len(), 86, "code example count changed");
    assert_eq!(ALL_MATH_EXAMPLES.len(), 28, "math example count changed");
    assert_eq!(
        ALL_HARDWARE_SPECS.len(),
        22,
        "hardware example count changed"
    );
}

/// Ratchet: every `vfs_path` is unique, lives under `/examples/<mode>/`, has the
/// extension its mode requires, and carries non-empty source — so a copy-paste,
/// a misfiled example, or a wrong-extension path fails the build.
#[test]
fn every_vfs_path_is_well_formed_and_unique() {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for spec in all_specs() {
        assert!(
            seen.insert(spec.vfs_path),
            "duplicate vfs_path: {}",
            spec.vfs_path
        );
        let (dir, exts): (&str, &[&str]) = match spec.mode {
            Mode::Logic => ("/examples/logic/", &[".logic"]),
            Mode::Code => ("/examples/code/", &[".logos"]),
            Mode::Math => ("/examples/math/", &[".logos"]),
            Mode::Hardware => ("/examples/hardware/", &[".hw", ".v"]),
        };
        assert!(
            spec.vfs_path.starts_with(dir),
            "{} is not under {dir}",
            spec.vfs_path
        );
        assert!(
            exts.iter().any(|e| spec.vfs_path.ends_with(e)),
            "{} has the wrong extension for {:?}",
            spec.vfs_path,
            spec.mode
        );
        assert!(
            !spec.source.trim().is_empty(),
            "{} has empty source",
            spec.vfs_path
        );
    }
}

/// The Studio has no mode switch — the file you open IS the mode — so every shipped
/// example must resolve to a mode, and the one its registry declares.
#[test]
fn every_example_opens_in_its_declared_mode() {
    use crate::ui::state::StudioMode;
    for spec in all_specs() {
        let opens_in = StudioMode::for_path(spec.vfs_path);
        let declared = match spec.mode {
            Mode::Logic => StudioMode::Logic,
            Mode::Math => StudioMode::Math,
            Mode::Hardware => StudioMode::Hardware,
            // A reactive web app is Code-language source the Studio shows as a live preview.
            Mode::Code if spec.vfs_path.starts_with("/examples/code/web/") => StudioMode::Web,
            Mode::Code => StudioMode::Code,
        };
        assert_eq!(
            opens_in,
            Some(declared),
            "{} opens in the wrong mode",
            spec.vfs_path
        );
    }
}

/// With no mode switch, a mode no example opens in is a mode nobody can reach.
#[test]
fn every_studio_mode_is_reachable_from_an_example() {
    use crate::ui::state::StudioMode;
    for mode in StudioMode::ALL {
        assert!(
            all_specs().any(|spec| StudioMode::for_path(spec.vfs_path) == Some(mode)),
            "no shipped example opens in {mode:?} mode, so the Studio can never show it"
        );
    }
}

/// The pit of success: `seed_examples` writes EXACTLY the registry — no more, no
/// less. Drive it through a real filesystem VFS and compare the seeded
/// `/examples/**` files to the registry `vfs_path`s. A stray `vfs.write`, a
/// forgotten example, or a path typo fails here.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn seeding_writes_exactly_the_registry() {
    use std::collections::HashSet;
    use std::path::Path;

    let root = std::env::temp_dir().join(format!("logos_seed_ratchet_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let vfs = logicaffeine_system::fs::NativeVfs::new(root.clone());
    let rt = logicaffeine_system::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");
    rt.block_on(super::seed_examples(&vfs))
        .expect("seed_examples failed");

    fn collect(dir: &Path, base: &Path, out: &mut HashSet<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    collect(&p, base, out);
                } else if let Ok(rel) = p.strip_prefix(base) {
                    out.insert(format!("/{}", rel.to_string_lossy().replace('\\', "/")));
                }
            }
        }
    }
    let mut seeded = HashSet::new();
    collect(&root.join("examples"), &root, &mut seeded);
    let _ = std::fs::remove_dir_all(&root);

    let registry: HashSet<String> = all_specs().map(|s| s.vfs_path.to_string()).collect();
    let missing: Vec<&String> = registry.difference(&seeded).collect();
    let extra: Vec<&String> = seeded.difference(&registry).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "seed_examples drifted from the registry.\n  MISSING (in registry, not seeded): {missing:?}\n  EXTRA (seeded, not in registry): {extra:?}"
    );
}

/// Every folder the Studio shows must contain files. `load_dir_recursive` renders
/// a node for every directory in the seeded VFS — including empty ones — so a stray
/// `create_dir_all` for a category with no examples, or a vestigial scratch folder,
/// surfaces as an empty folder in the file tree. Walk the whole seeded tree and fail
/// on any directory that has no entries.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn no_seeded_folder_is_empty() {
    use std::path::Path;

    let root = std::env::temp_dir().join(format!("logos_empty_dir_ratchet_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let vfs = logicaffeine_system::fs::NativeVfs::new(root.clone());
    let rt = logicaffeine_system::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");
    rt.block_on(super::seed_examples(&vfs))
        .expect("seed_examples failed");

    fn empty_dirs(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let children: Vec<_> = match std::fs::read_dir(dir) {
            Ok(entries) => entries.flatten().map(|e| e.path()).collect(),
            Err(_) => return,
        };
        if children.is_empty() {
            let rel = dir
                .strip_prefix(base)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            out.push(format!("/{rel}"));
            return;
        }
        for child in children {
            if child.is_dir() {
                empty_dirs(&child, base, out);
            }
        }
    }
    let mut empties = Vec::new();
    empty_dirs(&root, &root, &mut empties);
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        empties.is_empty(),
        "seed_examples creates empty folders (they render as empty nodes in the Studio tree): {empties:?}"
    );
}

/// AUDIT (host-only, print report): generate the Rust for every Logic + Math
/// example and actually `rustc`-compile it, categorizing program vs note vs
/// degraded (`unsupported`) stub. Answers "is anything not compiling to Rust?"
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn audit_logic_math_rust_compiles() {
    use logicaffeine_compile::{extract_logic_rust, extract_math_rust_from_source};
    use std::process::Command;

    fn first_error(stderr: &[u8]) -> String {
        String::from_utf8_lossy(stderr)
            .lines()
            .find(|l| l.contains("error"))
            .unwrap_or("(unknown)")
            .to_string()
    }

    fn rustc_check(name: &str, code: &str) -> Result<(), String> {
        // The name is a vfs_path; strip characters rustc rejects in a crate name.
        let safe = name.replace(['/', '.', '-'], "_");
        let src = std::env::temp_dir().join(format!("logos_audit_{safe}.rs"));
        std::fs::write(&src, code).map_err(|e| e.to_string())?;

        // A self-contained program with a `main` (the math "🦀 Compile" output
        // carries a self-verifying main) is compiled to a binary AND RUN, so its
        // kernel-checked asserts actually execute — the extraction's runtime
        // fidelity, not just that it type-checks. A bounded wait keeps a runaway
        // program from hanging the suite. Everything else is compile-checked as a
        // library.
        if code.contains("fn main") {
            let bin = std::env::temp_dir().join(format!("logos_audit_{safe}_bin"));
            let o = Command::new("rustc")
                .args(["--edition", "2021", "-A", "warnings", "-o"])
                .arg(&bin)
                .arg(&src)
                .output()
                .map_err(|e| e.to_string())?;
            if !o.status.success() {
                return Err(first_error(&o.stderr));
            }
            let mut child = Command::new(&bin).spawn().map_err(|e| e.to_string())?;
            let mut waited = 0u32;
            let status = loop {
                match child.try_wait().map_err(|e| e.to_string())? {
                    Some(s) => break Some(s),
                    None if waited >= 100 => {
                        let _ = child.kill();
                        break None;
                    }
                    None => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        waited += 1;
                    }
                }
            };
            let _ = std::fs::remove_file(&bin);
            match status {
                Some(s) if s.success() => Ok(()),
                Some(_) => Err("compiled but its self-checking main FAILED at runtime".to_string()),
                None => Err("compiled but did not finish within 10s".to_string()),
            }
        } else {
            let out = std::env::temp_dir().join(format!("logos_audit_{safe}.rmeta"));
            let o = Command::new("rustc")
                .args([
                    "--edition",
                    "2021",
                    "--crate-type",
                    "lib",
                    "--emit=metadata",
                    "-A",
                    "warnings",
                    "-o",
                ])
                .arg(&out)
                .arg(&src)
                .output()
                .map_err(|e| e.to_string())?;
            if o.status.success() {
                Ok(())
            } else {
                Err(first_error(&o.stderr))
            }
        }
    }

    fn report(kind: &str, items: &[(&str, String)], failures: &mut Vec<String>) {
        println!("\n=== {kind} examples → Rust ===");
        for (name, code) in items {
            // A note is a comment-only output (the honest "not data" result).
            let is_program = code.contains("fn ") || code.contains("enum ");
            if !is_program {
                println!("  {name:30} NOTE (not a program — by design)");
                continue;
            }
            match rustc_check(name, code) {
                Ok(()) => println!("  {name:30} compiles ✓"),
                Err(e) => {
                    println!("  {name:30} DOES NOT COMPILE ✗ — {e}");
                    failures.push(format!("{kind}/{name}: {e}"));
                }
            }
        }
    }

    let logic: Vec<(&str, String)> = ALL_LOGIC_EXAMPLES
        .iter()
        .map(|s| {
            (
                s.vfs_path,
                extract_logic_rust(s.source).unwrap_or_else(|e| format!("// err: {e}")),
            )
        })
        .collect();
    let math: Vec<(&str, String)> = ALL_MATH_EXAMPLES
        .iter()
        .map(|s| (s.vfs_path, extract_math_rust_from_source(s.source)))
        .collect();
    let mut failures = Vec::new();
    report("LOGIC", &logic, &mut failures);
    report("MATH", &math, &mut failures);
    // Every shipped example must compile to valid Rust OR be an honest note —
    // never broken Rust the user would hit on Compile.
    assert!(
        failures.is_empty(),
        "{} example(s) emit Rust that does not compile:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
