//! `largo opts` — report which optimizations actually fire for a program.

use std::fs;

/// Handle `largo opts <file>`: print the fired/blocker/dependency optimization graph.
pub(crate) fn cmd_opts(
    file: &std::path::Path,
    json: bool,
    trace_jit: bool,
    program_args: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    use logicaffeine_compile::optimization::REGISTRY;
    let source = fs::read_to_string(file)?;

    let (compiled, blockers, dependencies) = crate::compile::optimization_graph(&source);
    // A program a backend rejects is reported at its mistake, as every command reports one.
    let rejected = |pe: logicaffeine_compile::error::ParseError| {
        crate::ui::CliError::new(crate::diagnostics::render_parse_error(
            &pe,
            &crate::diagnostics::Placed::in_file(file, source.clone(), pe.span),
        ))
    };
    let rust_trace = logicaffeine_compile::compile::compile_program_backend_traced(&source)
        .map(|(_, trace)| trace)
        .map_err(rejected)?;
    let c_trace = logicaffeine_compile::compile::compile_to_c_traced(&source)
        .map(|(_, trace)| trace)
        .map_err(rejected)?;
    let c_bytecode_trace = logicaffeine_compile::compile::compile_to_c_via_bytecode_traced(&source)
        .map(|(_, trace)| trace)
        .map_err(rejected)?;
    let run_trace = logicaffeine_compile::compile::optimization_trace_run(&source);
    let vm_trace = logicaffeine_compile::compile::optimization_trace_vm(&source);

    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    let jit_trace = if trace_jit {
        let mut argv = Vec::with_capacity(program_args.len() + 1);
        argv.push(file.display().to_string());
        argv.extend(program_args.iter().cloned());
        let trace = logicaffeine_jit::optimization_trace_jit(&source, &argv);
        if let Some(error) = &trace.outcome.error {
            return Err(
                std::io::Error::other(format!("JIT trace execution failed: {error}")).into(),
            );
        }
        Some(trace)
    } else {
        None
    };
    #[cfg(not(all(target_arch = "x86_64", not(target_os = "windows"))))]
    let jit_trace: Option<()> = {
        if trace_jit {
            return Err(std::io::Error::other(
                "JIT tracing is available only on x86-64 System V targets",
            )
            .into());
        }
        None
    };

    let c = c_trace.all_fired().keywords();
    let c_bytecode = c_bytecode_trace.all_fired().keywords();
    let run = run_trace.all_fired().keywords();
    let vm = vm_trace.all_fired().keywords();
    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    let jit = jit_trace
        .as_ref()
        .map(|trace| trace.shared.all_fired().keywords())
        .unwrap_or_default();
    #[cfg(not(all(target_arch = "x86_64", not(target_os = "windows"))))]
    let jit: Vec<&str> = Vec::new();
    let paths_of = |kw: &str| -> Vec<&'static str> {
        [
            ("compiled", &compiled),
            ("c", &c),
            ("c_bytecode", &c_bytecode),
            ("run", &run),
            ("vm", &vm),
            ("jit", &jit),
        ]
        .into_iter()
        .filter_map(|(path, fired)| fired.contains(&kw).then_some(path))
        .collect()
    };
    let fired: Vec<&str> = REGISTRY
        .iter()
        .filter(|m| !paths_of(m.keyword).is_empty())
        .map(|m| m.keyword)
        .collect();

    if json {
        let stage = |trace: &logicaffeine_compile::optimization::BackendOptimizationTrace| {
            serde_json::json!({
                "optimizer": trace.optimizer.keywords(),
                "lowering": trace.lowering.keywords(),
                "declines": trace.declines.iter().map(|decline| {
                    format!("{}: {}", decline.opt.meta().keyword, decline.reason)
                }).collect::<Vec<_>>(),
                "fallbacks": trace.fallbacks.iter().map(|fallback| {
                    format!("{:?} -> {:?}: {}", fallback.from, fallback.to, fallback.reason)
                }).collect::<Vec<_>>(),
            })
        };
        let mut paths = serde_json::json!({
            "compiled": compiled,
            "c": c,
            "c_bytecode": c_bytecode,
            "run": run,
            "vm": vm,
        });
        let mut stages = serde_json::json!({
            "compiled": stage(&rust_trace),
            "c": stage(&c_trace),
            "c_bytecode": stage(&c_bytecode_trace),
            "run": stage(&run_trace),
            "vm": stage(&vm_trace),
        });
        let mut report = serde_json::json!({
            "fired": fired,
            "paths": paths.clone(),
            "stages": stages.clone(),
            "blockers": blockers,
            "dependencies": dependencies,
        });

        #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
        if let Some(jit_trace) = &jit_trace {
            paths["jit"] = serde_json::json!(jit);
            stages["jit"] = stage(&jit_trace.shared);
            let counts = logicaffeine_jit::JitOptimization::ALL
                .into_iter()
                .map(|opt| (opt.keyword(), jit_trace.native.count(opt)))
                .collect::<std::collections::BTreeMap<_, _>>();
            report["paths"] = paths;
            report["stages"] = stages;
            report["jit_native"] = serde_json::json!({
                "lowering": jit_trace.native.keywords(),
                "counts": counts,
                "functions": {
                    "attempted": jit_trace.function_attempts,
                    "compiled": jit_trace.function_successes,
                },
                "loops": {
                    "attempted": jit_trace.region_attempts,
                    "compiled": jit_trace.region_successes,
                },
                "declines": jit_trace.declines,
                "execution": jit_trace.execution,
            });
        }

        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    if fired.is_empty() {
        println!("No optimizations fired for {}.", file.display());
        return Ok(());
    }

    let fired_set: std::collections::BTreeSet<&str> = fired.iter().copied().collect();
    println!(
        "Optimizations that fired for {} ({} of {}):",
        file.display(),
        fired.len(),
        REGISTRY.len()
    );
    // List in registry order, grouped by category.
    let mut last_group = "";
    for m in REGISTRY {
        if !fired_set.contains(m.keyword) {
            continue;
        }
        if m.group != last_group {
            println!("  {}", m.group);
            last_group = m.group;
        }
        println!("    {:<14} {}", m.keyword, m.label);
        println!("      paths: {}", paths_of(m.keyword).join(", "));
    }
    if !dependencies.is_empty() {
        println!("Dependencies (one fired only because another was on):");
        for (dependent, dep) in &dependencies {
            println!("    {dependent:<14} depends on {dep}");
        }
    }
    if !blockers.is_empty() {
        println!("Blockers (one took precedence, skipping another):");
        for (winner, loser) in &blockers {
            println!("    {winner:<14} blocks    {loser}");
        }
    }
    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    if let Some(jit_trace) = &jit_trace {
        println!(
            "JIT native coverage: {}/{} functions, {}/{} loops compiled",
            jit_trace.function_successes,
            jit_trace.function_attempts,
            jit_trace.region_successes,
            jit_trace.region_attempts,
        );
        println!(
            "JIT native lowerings: {}",
            jit_trace.native.keywords().join(", ")
        );
        if !jit_trace.declines.is_empty() {
            println!("JIT declines: {}", jit_trace.declines);
        }
    }
    Ok(())
}
