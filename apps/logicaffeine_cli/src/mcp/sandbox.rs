//! Sandboxed execution for the MCP `logos_run` tool.
//!
//! Agent-generated code cannot be trusted not to loop forever, so it never
//! runs inside the long-lived server process. The tool spawns the current
//! `largo` binary back as `largo mcp --worker <engine>` — source on stdin,
//! program output on stdout — and enforces the wall clock from the parent
//! with a poll-and-kill deadline. A timed-out child dies cleanly; the server
//! answers the next request as if nothing happened.
//!
//! The engine labels are STRICT (G12): `tw` runs the tree-walker, `vm` runs
//! the bytecode VM — never each other. The worker reports WHICH engine ran on
//! a marker stderr line the parent lifts into the payload (`engine_ran`), and
//! a program the requested engine cannot run is a TYPED REFUSAL (the distinct
//! exit `REFUSAL_EXIT`, `engine_ran: null`, `refused: true`, the reason in
//! `error`) — never a silent substitution.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Default wall-clock limit for one run.
const DEFAULT_TIMEOUT_MS: u64 = 5_000;
/// Hard ceiling a caller cannot exceed.
const MAX_TIMEOUT_MS: u64 = 60_000;
/// The worker's distinct exit for a strict-engine refusal: the requested
/// engine cannot run this program, and NOTHING ran in its place.
const REFUSAL_EXIT: i32 = 123;
/// The stderr marker line carrying the worker's honest engine identity.
const ENGINE_MARKER: &str = "logos-strict-engine: ";

/// `logos_run`: execute a program in a killed-on-deadline subprocess.
pub fn run(args: &Value) -> Result<Value, String> {
    let (source, _) = super::tools::resolve_source(args)?;

    let step_budget = args.get("step_budget").and_then(Value::as_u64);
    let engine = args.get("engine").and_then(Value::as_str).unwrap_or("tw");
    if engine != "tw" && engine != "vm" {
        return Err(format!("unknown engine '{engine}' (tw | vm)"));
    }
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .min(MAX_TIMEOUT_MS);
    let seed = args.get("seed").and_then(Value::as_u64);
    let program_args: Vec<String> = args
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    // Classify before running so the payload names the program's determinacy
    // class — and so unparseable source fails here with the Socratic story
    // instead of a dead subprocess.
    let determinacy = match crate::compile::classify_source(&source) {
        Ok(d) => format!("{d:?}"),
        Err(parse_error) => {
            let interner = logicaffeine_language::Interner::new();
            return Err(logicaffeine_language::socratic_explanation(
                &parse_error,
                &interner,
            ));
        }
    };

    let exe = std::env::current_exe().map_err(|e| format!("cannot find largo itself: {e}"))?;
    let mut command = Command::new(exe);
    command
        .arg("mcp")
        .arg("--worker")
        .arg(engine)
        .args(&program_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(seed) = seed {
        command.env("LOGOS_SEED", seed.to_string());
    }
    if let Some(budget) = step_budget {
        // The worker arms the fuel on its own thread (fuel is thread-local);
        // exhaustion comes back as the distinct exit 122.
        command.env("LOGOS_MCP_STEP_BUDGET", budget.to_string());
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot spawn the run worker: {e}"))?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(source.as_bytes())
        .map_err(|e| format!("cannot hand the worker its source: {e}"))?;
    // stdin drops closed here — the worker sees EOF and starts running.

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let timed_out = loop {
        match child.try_wait() {
            Ok(Some(_)) => break false,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                break true;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("lost track of the run worker: {e}"));
            }
        }
    };

    let output = child
        .wait_with_output()
        .map_err(|e| format!("cannot collect worker output: {e}"))?;
    let stdout_lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    // Lift the worker's engine-identity marker out of stderr: the payload
    // reports which engine ACTUALLY ran (never assumed from the label).
    let mut engine_ran: Option<String> = None;
    let stderr_text = String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| {
            if let Some(identity) = line.strip_prefix(ENGINE_MARKER) {
                if engine_ran.is_none() && identity != "none" {
                    engine_ran = Some(identity.to_string());
                }
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if timed_out {
        return Err(format!(
            "the program exceeded its {timeout_ms}ms wall-clock limit and was killed \
             (partial output: {} line(s))",
            stdout_lines.len()
        ));
    }

    let exit_code = output.status.code().unwrap_or(-1);
    let refused = exit_code == REFUSAL_EXIT;
    Ok(json!({
        "stdout_lines": stdout_lines,
        "error": if stderr_text.is_empty() { Value::Null } else { Value::String(stderr_text) },
        "exit_code": exit_code,
        "timed_out": false,
        "determinacy": determinacy,
        "strict": true,
        "engine_requested": engine,
        "engine_ran": match engine_ran {
            Some(e) => Value::String(e),
            None => Value::Null,
        },
        "refused": refused,
    }))
}

/// The worker half: `largo mcp --worker <engine> [program args…]`.
///
/// Reads the program from stdin, runs it STRICTLY on the named engine (`tw` =
/// tree-walker, anything else = the label's default; `vm` = bytecode VM —
/// never a silent substitute), prints its output lines to stdout, reports the
/// honest engine identity on a marker stderr line, and exits: 0 clean, 1 on a
/// runtime error, 122 on budget exhaustion, [`REFUSAL_EXIT`] when the
/// requested engine refused the program. Only ever spawned by [`run`] above.
pub fn worker(engine: &str, program_args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    let mut source = String::new();
    std::io::stdin().read_to_string(&mut source)?;

    // argv[0] is the program name, as on a compiled binary.
    let mut argv = Vec::with_capacity(program_args.len() + 1);
    argv.push("mcp-sandbox".to_string());
    argv.extend(program_args.iter().cloned());

    // Fuel is thread-local: arm it here, on the thread that runs the engine.
    let fuel = std::env::var("LOGOS_MCP_STEP_BUDGET")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(logicaffeine_compile::interpreter::install_fuel);

    let outcome = match engine {
        "vm" => logicaffeine_compile::run_strict_vm_with_args(&source, &argv),
        _ => logicaffeine_compile::run_strict_tw_with_args(&source, &argv),
    };

    // The honest identity — what ACTUALLY ran — for the parent's payload.
    eprintln!(
        "{ENGINE_MARKER}{}",
        outcome
            .engine_ran
            .map(logicaffeine_compile::EngineId::label)
            .unwrap_or("none")
    );
    if !outcome.output.is_empty() {
        for line in outcome.output.split('\n') {
            println!("{line}");
        }
    }
    if fuel.is_some_and(|f| f.exhausted.get()) {
        if let Some(error) = outcome.error {
            eprintln!("{error}");
        }
        // Budget exhaustion is a bound doing its job — the distinct exit the
        // parent maps into the tool payload (same code as `largo run`).
        std::process::exit(122);
    }
    if outcome.refused() {
        if let Some(error) = outcome.error {
            eprintln!("{error}");
        }
        // The typed refusal: the requested engine cannot run this program and
        // NOTHING ran in its place.
        std::process::exit(REFUSAL_EXIT);
    }
    if let Some(error) = outcome.error {
        eprintln!("{error}");
        std::process::exit(1);
    }
    Ok(())
}
