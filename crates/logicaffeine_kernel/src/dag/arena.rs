//! The Lean Kernel Arena's protocol over this core, run the one way every surface runs it: the Arena entry (the
//! `logos_kernel` example's run line), `largo lean check` and this crate's `logos-kernel` binary. One lean4export
//! stream, from a path or standard input, checked by an engine; the process exits 0 when it is accepted, 1 when
//! rejected and 2 when declined, with the reason, or the accepted counts, on standard error.

use super::export::{check_export_in, Observe};
use super::tc::{Engine, Kernel};
use super::Verdict;
use std::io::{BufReader, Read};
use std::path::PathBuf;

/// Deeply nested terms recurse deeply: the check runs on a thread whose stack is sized for them.
pub const STACK_BYTES: usize = 8 << 30;

/// The protocol's exit code for an input that could not be checked at all (it is a decline).
pub const DECLINED: i32 = 2;

/// Check the export at `path` (standard input for `None` or `-`) with `engine` and end the process with the
/// protocol's exit code. The process ends on the checking thread once the verdict is written, so the kernel's memory
/// goes back to the system whole instead of being freed piece by piece.
pub fn run(path: Option<PathBuf>, engine: Engine) -> ! {
    // A large export under THREADS > 1 is checked across cores (forked workers over the shared
    // environment); everything else — THREADS unset or 1, a small export, a non-Linux target —
    // falls through to the serial check below, byte-for-byte today's single-core path.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let threads = super::parallel::threads();
        if threads > 1 {
            let bytes = path
                .as_deref()
                .filter(|p| p.as_os_str() != "-")
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0);
            if bytes >= super::parallel::min_bytes() {
                super::parallel::run(path, engine, threads);
            }
        }
    }
    let worker = std::thread::Builder::new()
        .name("logos-kernel".into())
        .stack_size(STACK_BYTES)
        .spawn(move || {
            let mut bytes = None;
            let input: Box<dyn Read> = match &path {
                Some(p) if p.as_os_str() != "-" => match std::fs::File::open(p) {
                    Ok(f) => {
                        bytes = f.metadata().ok().map(|m| m.len());
                        Box::new(f)
                    }
                    Err(e) => {
                        eprintln!("declined: cannot open {}: {e}", p.display());
                        std::process::exit(DECLINED);
                    }
                },
                _ => Box::new(std::io::stdin()),
            };
            let mut k = Kernel::new();
            k.engine = engine;
            let (verdict, stats) = check_export_in(
                &mut k,
                BufReader::with_capacity(1 << 20, input),
                bytes,
                Observe::default(),
            );
            match &verdict {
                Verdict::Accept => eprintln!(
                    "accepted: {} declarations, {} records; native primitives: {}",
                    stats.declarations,
                    stats.records,
                    stats.validated_primitives.join(" ")
                ),
                Verdict::Reject(e) | Verdict::Decline(e) => eprintln!("{e}"),
            }
            std::process::exit(verdict.exit_code())
        });
    match worker {
        Ok(worker) => {
            let _ = worker.join();
            eprintln!("declined: the checking thread ended without a verdict");
        }
        Err(e) => eprintln!("declined: cannot start the checking thread: {e}"),
    }
    std::process::exit(DECLINED)
}

/// [`run`]'s verdict for a caller that goes on: the whole `export`, held in memory, checked by `engine` on a thread of
/// [`STACK_BYTES`], and the kernel torn down before the verdict returns. The Lean import's independent recheck asks the
/// core this way.
pub fn verdict(export: &str, engine: Engine) -> Verdict {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("logos-kernel".into())
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, || {
                let mut k = Kernel::new();
                k.engine = engine;
                check_export_in(
                    &mut k,
                    export.as_bytes(),
                    Some(export.len() as u64),
                    Observe::default(),
                )
                .0
            });
        match worker.map(|w| w.join()) {
            Ok(Ok(verdict)) => verdict,
            Ok(Err(_)) => Verdict::Decline(super::KernelFailure::resource(
                "the core's checking thread panicked".to_string(),
            )),
            Err(e) => Verdict::Decline(super::KernelFailure::resource(format!(
                "cannot start the core's checking thread: {e}"
            ))),
        }
    })
}

/// What `largo lean check` asks beyond the Arena protocol: an assumption [`Policy`], a JSON
/// document instead of the protocol's text, and a statement binding.
///
/// [`Policy`]: super::assumptions::Policy
#[cfg(feature = "lean")]
#[derive(Clone, Debug)]
pub struct CheckOptions {
    pub policy: super::assumptions::Policy,
    pub json: bool,
    /// The declarations the policy is held to; empty: every declaration of the export.
    pub targets: Vec<String>,
    /// An original export whose statement of the (one) target the checked export must state.
    pub bind: Option<PathBuf>,
}

/// The exit code of an accepted export whose declarations depend on an assumption the policy
/// refuses — the `disallowed_assumptions` code of `largo lean import`.
pub const DISALLOWED: i32 = 3;

/// The document schema of `largo lean check --json`.
pub const CHECK_SCHEMA: &str = "logos-lean-check/1";

/// Check the export at `path` with `engine` as [`run`] does, then hold it to `options`, and end the
/// process: 0 accepted (within the policy, bound when asked) · 1 rejected, or a binding that does
/// not hold · 2 declined · 3 a dependency the policy refuses. With `json`, one `logos-lean-check/1`
/// document on standard output; otherwise the protocol's text on standard error.
#[cfg(feature = "lean")]
pub fn run_checked(path: Option<PathBuf>, engine: Engine, options: CheckOptions) -> ! {
    let worker = std::thread::Builder::new()
        .name("logos-kernel".into())
        .stack_size(STACK_BYTES)
        .spawn(move || {
            let (document, code) = check_with(path, engine, &options);
            if options.json {
                println!("{document}");
            } else {
                eprintln!("{}", document["message"].as_str().unwrap_or_default());
                for dependency in document["assumptions"]["dependencies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|d| !d["refused"].is_null())
                {
                    eprintln!(
                        "disallowed: {} ({}), through {}",
                        dependency["axiom"].as_str().unwrap_or_default(),
                        dependency["refused"].as_str().unwrap_or_default(),
                        dependency["path"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|p| p.as_str())
                            .collect::<Vec<_>>()
                            .join(" → ")
                    );
                }
            }
            let _ = std::io::Write::flush(&mut std::io::stdout());
            std::process::exit(code)
        });
    match worker {
        Ok(worker) => {
            let _ = worker.join();
            eprintln!("declined: the checking thread ended without a verdict");
        }
        Err(e) => eprintln!("declined: cannot start the checking thread: {e}"),
    }
    std::process::exit(DECLINED)
}

#[cfg(feature = "lean")]
fn check_with(
    path: Option<PathBuf>,
    engine: Engine,
    options: &CheckOptions,
) -> (serde_json::Value, i32) {
    use serde_json::json;
    let declined = |message: String| {
        (
            json!({"schema": CHECK_SCHEMA, "verdict": "declined", "exit": DECLINED, "message": message,
                   "assumptions": null, "binding": null}),
            DECLINED,
        )
    };
    let text = match &path {
        Some(p) if p.as_os_str() != "-" => match std::fs::read_to_string(p) {
            Ok(text) => text,
            Err(e) => return declined(format!("declined: cannot read {}: {e}", p.display())),
        },
        _ => {
            let mut text = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut text) {
                return declined(format!("declined: cannot read standard input: {e}"));
            }
            text
        }
    };
    let mut k = Kernel::new();
    k.engine = engine;
    let (verdict, stats) = check_export_in(
        &mut k,
        text.as_bytes(),
        Some(text.len() as u64),
        Observe::default(),
    );
    let engine_name = if engine == Engine::Dual {
        "dual"
    } else {
        "sem"
    };
    let (name, mut code, message) = match &verdict {
        Verdict::Accept => (
            "accepted",
            0,
            format!(
                "accepted: {} declarations, {} records; native primitives: {}",
                stats.declarations,
                stats.records,
                stats.validated_primitives.join(" ")
            ),
        ),
        Verdict::Reject(e) => ("rejected", 1, e.to_string()),
        Verdict::Decline(e) => ("declined", DECLINED, e.to_string()),
    };
    let mut name = name.to_string();
    let mut message = message;
    let mut assumptions = serde_json::Value::Null;
    if matches!(verdict, Verdict::Accept) {
        let mut targets = Vec::new();
        let mut missing = Vec::new();
        for target in &options.targets {
            let id = crate::interface::lean::intake::parse_lean_name(target)
                .ok()
                .map(|components| {
                    components
                        .iter()
                        .fold(super::name::NameId::ANON, |pre, c| match c {
                            crate::interface::lean::intake::Component::Str(s) => {
                                k.s.names.str(pre, s)
                            }
                            crate::interface::lean::intake::Component::Num(n) => {
                                k.s.names.num(pre, *n)
                            }
                        })
                });
            if let Some(id) = id.filter(|id| k.env.contains(*id)) {
                targets.push(id);
            } else {
                missing.push(target.clone());
            }
        }
        if !missing.is_empty() {
            return (
                json!({"schema": CHECK_SCHEMA, "verdict": "rejected", "exit": 1, "engine": engine_name,
                       "message": format!("the export declares no {}", missing.join(", ")),
                       "declarations": stats.declarations, "records": stats.records,
                       "assumptions": null, "binding": null}),
                1,
            );
        }
        let audit = super::assumptions::audit(
            &mut k,
            options.policy,
            (!targets.is_empty()).then_some(targets.as_slice()),
        );
        if !audit.admitted() {
            name = "disallowed_assumptions".into();
            code = DISALLOWED;
            let refused: Vec<String> = audit.refusals().map(|d| d.axiom.clone()).collect();
            message = format!(
                "{message}; the {} policy refuses {}",
                options.policy.as_str(),
                refused.join(", ")
            );
        }
        assumptions = json!({
            "policy": options.policy.as_str(),
            "allowed": match options.policy {
                super::assumptions::Policy::Lean => json!(super::assumptions::LEAN_AXIOMS),
                super::assumptions::Policy::Any => json!(null),
            },
            "scope": if options.targets.is_empty() { json!("every declaration") } else { json!(options.targets) },
            "dependencies": audit.dependencies.iter().map(|d| json!({
                "axiom": d.axiom, "path": d.path, "refused": d.refused,
            })).collect::<Vec<_>>(),
            "unused": audit.unused,
        });
    }
    let mut binding = serde_json::Value::Null;
    if let Some(original) = &options.bind {
        let target = options.targets.first().cloned().unwrap_or_default();
        let bound = std::fs::read_to_string(original)
            .map_err(|e| format!("cannot read {}: {e}", original.display()))
            .and_then(|original_text| {
                let theirs =
                    crate::interface::lean::statement::fingerprint(&original_text, &target)
                        .map_err(|v| {
                            format!(
                                "the original states no statement of {target}: {}",
                                v.reason().message()
                            )
                        })?;
                let ours = crate::interface::lean::statement::fingerprint(&text, &target).map_err(
                    |v| {
                        format!(
                            "the checked export states no statement of {target}: {}",
                            v.reason().message()
                        )
                    },
                )?;
                Ok((theirs, ours))
            });
        binding = match bound {
            Ok((theirs, ours)) => {
                let holds = theirs == ours;
                if !holds && code == 0 {
                    name = "unbound".into();
                    code = 1;
                    message = format!(
                        "{message}; but its {target} does not state the original's proposition"
                    );
                }
                json!({"target": target, "original": original.display().to_string(),
                       "original_fingerprint": theirs, "fingerprint": ours, "bound": holds})
            }
            Err(why) => {
                if code == 0 {
                    name = "unbound".into();
                    code = 1;
                    message = format!("{message}; the binding cannot be established: {why}");
                }
                json!({"target": target, "original": original.display().to_string(),
                       "bound": false, "message": why})
            }
        };
    }
    (
        json!({
            "schema": CHECK_SCHEMA,
            "verdict": name,
            "exit": code,
            "engine": engine_name,
            "message": message,
            "declarations": stats.declarations,
            "records": stats.records,
            "assumptions": assumptions,
            "binding": binding,
        }),
        code,
    )
}
