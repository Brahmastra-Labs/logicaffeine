//! `largo run` — build-and-execute, interpret, or compile-to-wasm-and-run.

use std::time::Duration;

use crate::commands::build::friendly_build_error;
use crate::commands::emit::build_wasm_module;
use crate::commands::require_project_root;
use crate::project::build::{self, Backend, BuildConfig};
use crate::project::manifest::Manifest;
use crate::ui::CliError;

/// Exit code when the step budget (`--max-steps`) is exhausted.
pub(crate) const EXIT_BUDGET: i32 = 122;
/// Exit code when the wall clock (`--timeout`) expires — the GNU `timeout(1)`
/// convention agents already know.
pub(crate) const EXIT_TIMEOUT: i32 = 124;

/// Parse a human duration: `500ms`, `10s`, `2m`.
pub(crate) fn parse_duration(text: &str) -> Result<Duration, CliError> {
    let trimmed = text.trim();
    let (number, unit): (&str, &str) = trimmed
        .find(|c: char| c.is_ascii_alphabetic())
        .map(|i| trimmed.split_at(i))
        .unwrap_or((trimmed, "s"));
    let value: u64 = number.trim().parse().map_err(|_| {
        CliError::with_hint(
            format!("cannot read the duration '{text}'"),
            "write it like 500ms, 10s, or 2m",
        )
    })?;
    match unit {
        "ms" => Ok(Duration::from_millis(value)),
        "s" | "" => Ok(Duration::from_secs(value)),
        "m" => Ok(Duration::from_secs(value * 60)),
        other => Err(CliError::with_hint(
            format!("unknown duration unit '{other}' in '{text}'"),
            "write it like 500ms, 10s, or 2m",
        )),
    }
}

/// Arm `LOGOS_SEED` for this process (and every child it spawns): the one
/// env var all deterministic consumers — the cooperative scheduler, the
/// seeded `Select` chooser, replay capture, binaries compiled for deterministic replay — read.
fn arm_seed(seed: Option<u64>) {
    if let Some(seed) = seed {
        std::env::set_var("LOGOS_SEED", seed.to_string());
    }
}

/// Handle `largo run` (default path): build with cargo, then execute.
pub(crate) fn cmd_run(
    release: bool,
    backend: Backend,
    args: &[String],
    max_steps: Option<u64>,
    timeout: Option<&str>,
    seed: Option<u64>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if max_steps.is_some() {
        return Err(CliError::with_hint(
            "--max-steps is an engine feature of `--interpret`",
            "compiled binaries run at native speed; bound them with --timeout instead",
        )
        .into());
    }
    if json {
        return Err(CliError::with_hint(
            "--json is the interpreter run report",
            "run with `--interpret --json`",
        )
        .into());
    }
    arm_seed(seed);
    let deadline = timeout.map(parse_duration).transpose()?;
    let project_root = require_project_root()?;

    let config = BuildConfig {
        project_dir: project_root,
        release,
        lib_mode: false,
        target: None,
    };

    let result = build::build_backend(config, backend).map_err(friendly_build_error)?;

    let exit_code = match deadline {
        None => build::run(&result, args)?,
        Some(deadline) => {
            let mut child = std::process::Command::new(&result.binary_path)
                .args(args)
                .spawn()
                .map_err(|e| format!("cannot start {}: {e}", result.binary_path.display()))?;
            let end = std::time::Instant::now() + deadline;
            loop {
                match child.try_wait()? {
                    Some(status) => break status.code().unwrap_or(1),
                    None if std::time::Instant::now() >= end => {
                        let _ = child.kill();
                        let _ = child.wait();
                        eprintln!(
                            "error: the program exceeded its {timeout:?} wall clock and was killed",
                            timeout = timeout.unwrap_or("?")
                        );
                        std::process::exit(EXIT_TIMEOUT);
                    }
                    None => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        }
    };

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

/// One interpreter execution's full account: the engine result plus the fuel
/// verdict when a budget was armed.
struct EngineOutcome {
    result: logicaffeine_compile::ui_bridge::InterpreterResult,
    /// `(exhausted, steps_used)` when `--max-steps` was armed.
    fuel: Option<(bool, u64)>,
}

/// Run the selected interpreter tier with optional fuel — on THIS thread
/// (fuel and engine selection are thread-local by design).
fn run_engine_on(
    source: &str,
    argv: &[String],
    max_steps: Option<u64>,
    engine: logicaffeine_compile::ui_bridge::Engine,
    scope: &logicaffeine_compile::analysis::ScopeInfo,
) -> EngineOutcome {
    logicaffeine_compile::ui_bridge::set_engine(engine);
    let handles = max_steps.map(logicaffeine_compile::interpreter::install_fuel);
    let result = if matches!(
        engine,
        logicaffeine_compile::ui_bridge::Engine::Jit | logicaffeine_compile::ui_bridge::Engine::Vm
    ) {
        // These tiers are sync-only.  Check project scope on the AST their run
        // already parses instead of running the full front end, reparsing to
        // detect async, then parsing a third time for execution.
        logicaffeine_compile::ui_bridge::interpret_for_ui_sync_with_args_scoped(source, argv, scope)
    } else {
        // The current-thread reactor is needed by Auto/tree programs that may
        // Connect, Serve or Sleep. `block_on` stays on this thread, where fuel
        // and engine selection are installed.
        let runtime = logicaffeine_system::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the LOGOS runtime starts");
        runtime.block_on(logicaffeine_compile::interpret_for_ui_with_args(
            source, argv,
        ))
    };
    let fuel = handles.map(|h| (h.exhausted.get(), h.budget - h.remaining.get()));
    logicaffeine_compile::interpreter::clear_fuel();
    EngineOutcome { result, fuel }
}

/// Handle `largo run --interpret`: run the program in-process on the bytecode VM (the tree-walker for
/// concurrent programs), without any Rust build.
pub(crate) fn cmd_run_interpret(
    args: &[String],
    max_steps: Option<u64>,
    timeout: Option<&str>,
    seed: Option<u64>,
    json: bool,
    engine: logicaffeine_compile::ui_bridge::Engine,
) -> Result<(), Box<dyn std::error::Error>> {
    arm_seed(seed);
    let deadline = timeout.map(parse_duration).transpose()?;
    let project_root = require_project_root()?;

    let manifest = Manifest::load(&project_root)?;
    let entry_path = crate::commands::resolve_entry_path(&project_root, &manifest)?;
    // Link the declared local (`path`/`git`) dependencies so an imported dep's
    // functions resolve on the interpreter exactly as they do under `largo build`.
    let dep_roots = crate::project::deps::resolve_dep_roots(&project_root, &manifest)?;
    // Assemble the whole project (entry + imported modules across files/directories)
    // into one compilation unit so imported functions and types resolve on the
    // interpreter exactly as they do under `largo build`.
    let source =
        logicaffeine_compile::analysis::assemble_project_with_deps(&entry_path, dep_roots.clone())
            .map_err(|e| format!("could not assemble project: {e}"))?;
    let scope = std::sync::Arc::new(source.scope);
    let source = source.source;
    let explicit_sync_tier = matches!(
        engine,
        logicaffeine_compile::ui_bridge::Engine::Jit | logicaffeine_compile::ui_bridge::Engine::Vm
    );
    if !explicit_sync_tier {
        crate::diagnostics::front_end(&entry_path, &dep_roots)?;
    }

    // `## Require <opt>` on the interpreter: the tree-walk tier applies no optimizations, so a
    // declared requirement cannot be satisfied here — fail loudly (consistent with the compiled
    // tiers) rather than silently ignore it.
    if let Some(msg) = logicaffeine_compile::ui_bridge::interpret_unsatisfiable_require(&source) {
        return Err(msg.into());
    }

    // Build the argv the program's `args()` sees: index 0 is the program name
    // (the compiled binary's `env::args()[0]`), then the user arguments — so
    // `item 2 of args()` is the first user argument on the interpreter exactly
    // as on the native binary.
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(manifest.package.name.clone());
    argv.extend(args.iter().cloned());

    // Compiled-native tier (HOTSWAP §Axis-3): if the program annotates functions
    // `is exported for native`, load them as rustc -O3 machine code (cached, so a
    // pre-built `largo build --native-functions` bundle is a cache hit) and queue them
    // so the VM dispatches those functions to compiled native from their first call.
    // Absent / unbuildable ⇒ nothing queued ⇒ runs on VM+JIT, no gap at the seam.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let names = logicaffeine_compile::compile::native_export_function_names(&source);
        if !names.is_empty() {
            let cache_dir = project_root.join(".logos-native");
            let natives = logicaffeine_compile::compile::aot_load_bundle(&source, &cache_dir);
            if !natives.is_empty() {
                logicaffeine_compile::ui_bridge::set_pending_aot_natives(natives);
            }
        }
    }

    let outcome = match deadline {
        None => run_engine_on(&source, &argv, max_steps, engine, &scope),
        Some(deadline) => {
            // The engine runs on a watchdog thread; expiry abandons it and
            // exits — the process is about to die anyway, so nothing leaks
            // past it.
            let (tx, rx) = std::sync::mpsc::channel();
            let source_for_engine = source.clone();
            let argv_for_engine = argv.clone();
            let scope_for_engine = std::sync::Arc::clone(&scope);
            std::thread::spawn(move || {
                let _ = tx.send(run_engine_on(
                    &source_for_engine,
                    &argv_for_engine,
                    max_steps,
                    engine,
                    &scope_for_engine,
                ));
            });
            match rx.recv_timeout(deadline) {
                Ok(outcome) => outcome,
                Err(_) => {
                    if json {
                        println!(
                            "{}",
                            crate::diagnostics::run_envelope("timeout", &[], None, None)
                        );
                    }
                    eprintln!(
                        "error: the program exceeded its {} wall clock and was killed",
                        timeout.unwrap_or("?")
                    );
                    std::process::exit(EXIT_TIMEOUT);
                }
            }
        }
    };
    // The fast scoped path preserves rich, source-mapped parse/scope
    // diagnostics by paying for the legacy diagnostic front end only when the
    // attempted execution actually failed. Runtime failures pass through.
    if explicit_sync_tier && outcome.result.error.is_some() {
        crate::diagnostics::front_end(&entry_path, &dep_roots)?;
    }
    let EngineOutcome { result, fuel } = outcome;
    let steps_used = fuel.map(|(_, used)| used);
    let budget_exceeded = fuel.is_some_and(|(exhausted, _)| exhausted);

    if json {
        let outcome_name = if budget_exceeded {
            "budget-exceeded"
        } else if result.error.is_some() {
            "runtime-error"
        } else {
            "ok"
        };
        println!(
            "{}",
            crate::diagnostics::run_envelope(
                outcome_name,
                &result.lines,
                result.error.as_deref(),
                steps_used,
            )
        );
    } else {
        use std::io::Write as _;
        let stdout = std::io::stdout();
        let mut out = std::io::BufWriter::with_capacity(64 * 1024, stdout.lock());
        for line in &result.lines {
            if writeln!(out, "{line}").is_err() {
                break;
            }
        }
        let _ = out.flush();
    }

    if budget_exceeded {
        // Exhaustion is a bound doing its job, not a program failure: no
        // replay artifact, a distinct exit code, the partial output already
        // delivered above.
        if let Some(err) = &result.error {
            eprintln!("{}", err);
        }
        std::process::exit(EXIT_BUDGET);
    }

    if let Some(err) = result.error {
        eprintln!("{}", err);
        // Every failure is a replayable artifact: capture the run as a
        // self-contained `.replay` bundle (embedded source + seed + schedule)
        // under target/replays/. Capture re-executes on the deterministic
        // artifact engine; if that engine doesn't reproduce the failure
        // (engine-specific error), no artifact is written — never a lie.
        let seed = std::env::var("LOGOS_SEED")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        if let Some(bundle) = logicaffeine_compile::replay::capture_failure(
            &manifest.package.entry,
            &source,
            seed,
            &argv,
        ) {
            if let Some(path) = crate::commands::replay::write_artifact(&project_root, &bundle) {
                // A failed program's stderr is its reason, exactly as every compiled backend writes it; the
                // artifact is announced to a person at a terminal, never into a pipe that reads the program.
                if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
                    eprintln!("recording: {}", path.display());
                    eprintln!("replay it: largo replay {}", path.display());
                }
            }
        }
        std::process::exit(1);
    }

    Ok(())
}

/// `largo run --emit wasm [args…]` — compile to wasm and run it in ONE step through the emitted host
/// shim (node), passing `args` to the program's `args()`. Compile-and-run with no Rust toolchain.
pub(crate) fn cmd_run_wasm(
    args: &[String],
    linked: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;
    let (mjs, _) = build_wasm_module(&project_root, linked)?;
    let status = std::process::Command::new("node")
        .arg(&mjs)
        .args(args)
        .status()
        .map_err(|e| format!("could not launch node to run the wasm (is node installed?): {e}"))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
