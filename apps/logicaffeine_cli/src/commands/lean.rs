//! `largo lean` — Lean in, LOGOS out, independently checked.
//!
//! ```text
//! largo lean import --project <dir> --target <Qualified.Name> --out <dir>   # the public experience
//! largo lean import --project <dir> --module <Module> --target <Name> --out <dir>  # a Lake (Mathlib) project
//! largo lean import <export.ndjson> --target <Qualified.Name> --out <dir>  # advanced interchange
//! largo lean import … --statement                                           # the statement, not its proof
//! largo lean import <export.ndjson> --arena-protocol                        # benchmark compatibility
//! largo lean replay <package-dir>
//! largo lean check <export.ndjson>                                          # the kernel core, as the Arena runs it
//! ```
//!
//! `import` writes exactly one `logos-lean-result/1` document on stdout and its diagnostics, one JSON
//! object per line, on stderr. Its exit code is the outcome's: 0 verified · 1 invalid source ·
//! 2 unsupported feature · 3 disallowed assumptions · 4 resource limit · 5 infrastructure failure ·
//! 64 usage (no result: nothing ran). `--arena-protocol` is the Lean Kernel Arena protocol, byte for
//! byte (`ACCEPT` and exit 0/1/2).
//!
//! The stages themselves are `logicaffeine_kernel::interface::lean::pipeline`'s; this command owns
//! only what needs the world: running the project's profile's Lean toolchain and pinned exporter in
//! isolation ([`project`]), publishing the package atomically and replaying it in a fresh process
//! ([`publish`]). The profiles are `logicaffeine_kernel::interface::lean::profile`'s registry.

mod process;
mod project;
mod publish;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use logicaffeine_kernel::interface::lean::intake;
use logicaffeine_kernel::interface::lean::pipeline::{
    self, Diagnostic, LeanEvidence, LeanResult, Limits,
};
use logicaffeine_kernel::interface::lean::DiagnosticCode;
use serde_json::{json, Value};

/// The `usage` exit code: nothing ran, so there is no result.
pub(crate) const EXIT_USAGE: i32 = 64;

#[derive(Parser)]
#[command(
    name = "largo lean",
    about = "Lean 4 → LOGOS: import, check independently, publish, replay"
)]
struct LeanCli {
    #[command(subcommand)]
    command: LeanCommand,
}

#[derive(Subcommand)]
enum LeanCommand {
    /// Import a Lean declaration as a checked, published LOGOS package.
    #[command(
        after_help = "Examples:\n  largo lean import --project ./proofs --target Demo.andSwap --out ./Demo.andSwap\n  largo lean import Demo.ndjson --target Demo.andSwap --out ./Demo.andSwap\n  largo lean import Demo.ndjson --arena-protocol"
    )]
    Import(ImportArgs),
    /// Replay a published package in this (fresh) process and print its receipt.
    Replay(ReplayArgs),
    /// Check a whole lean4export stream with the kernel core, as the Lean Kernel Arena runs it.
    #[command(
        after_help = "Exit codes: 0 accepted, 1 rejected (or --bind does not hold), 2 declined, 3 an assumption --assumptions refuses; the reason on stderr, or one JSON document on stdout with --json.\n\nExamples:\n  largo lean check Mathlib.ndjson\n  largo lean check - < Mathlib.ndjson\n  largo lean check --assumptions lean --json Proof.ndjson\n  largo lean check Proof.ndjson --target Demo.thm --bind Original.ndjson"
    )]
    Check(CheckArgs),
}

#[derive(Args)]
struct CheckArgs {
    /// The lean4export 3.1.0 stream (`-`, or nothing, for standard input).
    export: Option<PathBuf>,
    /// Check with both of the core's engines, which must agree on every declaration.
    #[arg(long)]
    dual: bool,
    /// Hold what the declarations depend on to a policy: `lean` allows only propext, Quot.sound and
    /// Classical.choice, in their canonical statements (sorryAx, Lean.ofReduceBool,
    /// Lean.trustCompiler, lcProof and any other axiom are refused); `any` reports without refusing.
    #[arg(long, value_name = "POLICY", value_parser = ["any", "lean"])]
    assumptions: Option<String>,
    /// Write one `logos-lean-check/1` JSON document on stdout.
    #[arg(long)]
    json: bool,
    /// The declaration the policy and the binding concern (repeatable); every declaration when absent.
    #[arg(long, value_name = "Qualified.Name")]
    target: Vec<String>,
    /// An original export: the checked export's --target must state exactly the original's
    /// proposition (its statement fingerprint).
    #[arg(long, value_name = "ORIGINAL", requires = "target")]
    bind: Option<PathBuf>,
}

#[derive(Args)]
struct ImportArgs {
    /// A lean4export 3.1.0 export (the advanced interchange input).
    export: Option<PathBuf>,

    /// A Lean project directory: compiled in isolation with the registered profile its
    /// `lean-toolchain` pins (Lean 4.14.0, 4.28.0, 4.33.1, 4.34.0-rc1 or 4.34.1; none pinned: 4.14.0).
    #[arg(long, value_name = "DIR", conflicts_with = "export")]
    project: Option<PathBuf>,

    /// The Lean declaration to import, by its qualified Lean name.
    #[arg(long, value_name = "Qualified.Name")]
    target: Option<String>,

    /// The module declaring the target. Required for a project with Lake dependencies (Mathlib),
    /// where only this module is compiled, against the project's prebuilt build.
    #[arg(long, value_name = "Module.Name", requires = "project")]
    module: Option<String>,

    /// Where to publish the package. Nothing is published unless every stage passes.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,

    /// Import the target's statement, not its proof: a theorem's type or a Prop definition's value,
    /// as a checked LOGOS definition (`logos-lean-result/2`, proof status `not_claimed`).
    #[arg(long)]
    statement: bool,

    /// The Lean Kernel Arena benchmark protocol: `ACCEPT`, exit 0/1/2, nothing else.
    #[arg(long, requires = "export", conflicts_with_all = ["project", "target", "out", "statement"])]
    arena_protocol: bool,

    /// (arena protocol) Report every checking stage as JSON on stderr.
    #[arg(long, requires = "arena_protocol")]
    trace_stages: bool,

    /// (arena protocol) Run the declaration audit.
    #[arg(long, requires = "arena_protocol")]
    audit_declarations: bool,

    /// The Lean executable (the elan shim selects the profile's toolchain).
    #[arg(long, value_name = "PATH", default_value = "lean")]
    lean: PathBuf,

    /// The profile's pinned lean4export, built. Defaults (Lean 4.14.0) to $LOGOS_LEAN4EXPORT, then
    /// ~/.logos/lean/lean4export, and for every profile to the registry entry
    /// $LOGOS_LEAN_EXPORTERS/<version>, else ~/.logos/lean/exporters/<version>
    /// (scripts/lean/install-lean-exporters.py builds it).
    #[arg(long, value_name = "DIR")]
    exporter: Option<PathBuf>,

    /// The largest export accepted, in bytes; checked before it is read.
    #[arg(long, value_name = "BYTES", default_value_t = 1 << 30)]
    max_export_bytes: u64,

    /// The whole run's deadline, in seconds; checked before each stage and enforced on Lean.
    #[arg(long, value_name = "SECONDS", default_value_t = 1800)]
    timeout_secs: u64,
}

#[derive(Args)]
struct ReplayArgs {
    /// The package directory.
    package: PathBuf,
}

/// Handle `largo lean …`. Exits the process with the outcome's code; returns only on success.
pub(crate) fn cmd_lean(arguments: Vec<OsString>) -> Result<(), Box<dyn std::error::Error>> {
    let argv = std::iter::once(OsString::from("largo lean")).chain(arguments);
    let code = match LeanCli::try_parse_from(argv) {
        Ok(cli) => match cli.command {
            LeanCommand::Import(args) => import(args),
            LeanCommand::Replay(args) => replay(&args.package),
            LeanCommand::Check(args) => {
                use logicaffeine_kernel::dag::{arena, assumptions::Policy, tc::Engine};
                let _ = std::io::stdout().flush();
                let engine = if args.dual { Engine::Dual } else { Engine::Sem };
                if args.assumptions.is_none()
                    && !args.json
                    && args.target.is_empty()
                    && args.bind.is_none()
                {
                    arena::run(args.export, engine)
                }
                if args.bind.is_some() && args.target.len() != 1 {
                    usage("--bind needs exactly one --target")
                } else {
                    arena::run_checked(
                        args.export,
                        engine,
                        arena::CheckOptions {
                            policy: match args.assumptions.as_deref() {
                                Some("lean") => Policy::Lean,
                                _ => Policy::Any,
                            },
                            json: args.json,
                            targets: args.target,
                            bind: args.bind,
                        },
                    )
                }
            }
        },
        Err(error) => match error.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                print!("{}", error.render());
                0
            }
            _ => usage(error.render().to_string().trim()),
        },
    };
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    if code == 0 {
        Ok(())
    } else {
        std::process::exit(code)
    }
}

/// A usage error: one typed diagnostic on stderr, no result, exit 64.
fn usage(message: &str) -> i32 {
    let diagnostic = json!({
        "code": DiagnosticCode::Usage.as_str(),
        "stage": Value::Null,
        "declaration": Value::Null,
        "line": Value::Null,
        "message": message,
    });
    eprintln!("{diagnostic}");
    EXIT_USAGE
}

fn import(args: ImportArgs) -> i32 {
    if args.arena_protocol {
        let export = args
            .export
            .expect("clap requires the export with --arena-protocol");
        let mut arena_args = vec![export.into_os_string()];
        if args.audit_declarations {
            arena_args.push("--audit-declarations".into());
        }
        if args.trace_stages {
            arena_args.push("--trace-stages".into());
        }
        return pipeline::run_cli(arena_args, &mut std::io::stdout(), &mut std::io::stderr());
    }
    let Some(target) = args.target.clone() else {
        return usage("--target <Qualified.Name> is required");
    };
    if let Err(verdict) = intake::parse_lean_name(&target) {
        return usage(verdict.reason().message());
    }
    let Some(out) = args.out.clone() else {
        return usage("--out <DIR> is required");
    };
    if let Err(message) = publish::check_out(&out) {
        return usage(&message);
    }
    let deadline = Instant::now() + Duration::from_secs(args.timeout_secs);
    let limits = Limits {
        max_export_bytes: args.max_export_bytes,
    };
    // The kernel stages run in this process: a watch ends the run if they outlast the deadline.
    let watched = |lean_accepted: bool, check: &dyn Fn() -> pipeline::Checked| {
        let watch = Deadline::watch(
            deadline,
            args.timeout_secs,
            target.clone(),
            args.statement,
            lean_accepted,
        );
        let checked = check();
        watch.settle();
        checked
    };

    let checked = match (&args.export, &args.project) {
        (Some(export), None) => watched(false, &|| {
            from_export(export, &target, &limits, args.statement)
        }),
        (None, Some(project)) => {
            if !project.is_dir() {
                return usage(&format!(
                    "--project {} is not a directory",
                    project.display()
                ));
            }
            if args.module.is_none() && project::has_lake_dependencies(project) {
                return usage(
                    "--module <Module.Name> is required: the project has Lake dependencies, and only the module declaring the target is compiled",
                );
            }
            if let Some(module) = &args.module {
                if let Err(verdict) = intake::parse_lean_name(module) {
                    return usage(verdict.reason().message());
                }
            }
            let options = project::Options {
                lean: args.lean.clone(),
                exporter: args.exporter.clone(),
                module: args.module.clone(),
                statement: args.statement,
                timeout_secs: args.timeout_secs,
                max_export_bytes: args.max_export_bytes,
            };
            match project::export(project, &target, &options, deadline) {
                Ok(exported) if args.statement => watched(true, &|| {
                    pipeline::check_statement(
                        &exported.export,
                        &target,
                        &exported.evidence,
                        &limits,
                    )
                }),
                Ok(exported) => watched(true, &|| {
                    pipeline::check_export(&exported.export, &target, &exported.evidence, &limits)
                }),
                Err(mut result) if args.statement => {
                    result.statement_requested = true;
                    pipeline::Checked {
                        result: *result,
                        package: None,
                    }
                }
                Err(result) => pipeline::Checked {
                    result: *result,
                    package: None,
                },
            }
        }
        _ => return usage("give exactly one input: an export file or --project <DIR>"),
    };
    // Publication enforces the deadline itself, on the fresh-process replay it waits for.
    let mut result = checked.result;
    publish::conclude(&mut result, checked.package, &out, deadline);
    result.finish();
    emit(&result)
}

/// The whole run's deadline, enforced: Lean and the exporter are killed when it passes, the stages
/// check it before they start, and a kernel stage still running when it passes ends the run with a
/// `resource_limit` result — never a process that outlives the budget it was given. Publication
/// enforces it on the fresh-process replay itself, so the watch ends before publication begins.
struct Deadline {
    settled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Deadline {
    fn watch(
        deadline: Instant,
        seconds: u64,
        target: String,
        statement: bool,
        lean_accepted: bool,
    ) -> Deadline {
        use std::sync::atomic::{AtomicBool, Ordering};
        let settled = std::sync::Arc::new(AtomicBool::new(false));
        let watching = settled.clone();
        std::thread::spawn(move || {
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(200));
                if watching.load(Ordering::SeqCst) {
                    return;
                }
            }
            if watching.swap(true, Ordering::SeqCst) {
                return;
            }
            let mut result = if statement {
                LeanResult::statement(&target)
            } else {
                LeanResult::new(&target)
            };
            if lean_accepted {
                result.pass("lean_accepted", "Lean compiled the project in isolation");
            }
            result.fail(
                "translated",
                Diagnostic::new(
                    "translated",
                    DiagnosticCode::ResourceLimit,
                    format!(
                        "the declared deadline of {seconds} s passed before the translation and its checks finished"
                    ),
                ),
            );
            std::process::exit(emit(&result));
        });
        Deadline { settled }
    }

    /// The run finished in time: the watchdog stands down (or has already ended the process).
    fn settle(&self) {
        if self.settled.swap(true, std::sync::atomic::Ordering::SeqCst) {
            // The watchdog won the race and is ending the process with its result.
            loop {
                std::thread::park();
            }
        }
    }
}

/// Export mode: the budget is checked against the file's size before a byte is read.
fn from_export(path: &Path, target: &str, limits: &Limits, statement: bool) -> pipeline::Checked {
    let refused = |code: DiagnosticCode, message: String| {
        let mut result = if statement {
            LeanResult::statement(target)
        } else {
            LeanResult::new(target)
        };
        result.fail("translated", Diagnostic::new("translated", code, message));
        pipeline::Checked {
            result,
            package: None,
        }
    };
    let size = match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(_) => {
            return refused(
                DiagnosticCode::Infrastructure,
                format!("{} is not a file", path.display()),
            )
        }
        Err(e) => {
            return refused(
                DiagnosticCode::Infrastructure,
                format!("cannot read {}: {e}", path.display()),
            )
        }
    };
    if size > limits.max_export_bytes {
        return refused(
            DiagnosticCode::ResourceLimit,
            format!(
                "the export is {size} bytes; the declared budget is {} bytes",
                limits.max_export_bytes
            ),
        );
    }
    match std::fs::read_to_string(path) {
        Ok(text) if statement => {
            pipeline::check_statement(&text, target, &LeanEvidence::NotObserved, limits)
        }
        Ok(text) => pipeline::check_export(&text, target, &LeanEvidence::NotObserved, limits),
        Err(e) => refused(
            DiagnosticCode::Infrastructure,
            format!("cannot read {}: {e}", path.display()),
        ),
    }
}

/// Print the result (stdout) and its diagnostics (stderr); return the outcome's exit code.
fn emit(result: &LeanResult) -> i32 {
    let document = result.to_json();
    {
        let mut stderr = std::io::stderr().lock();
        for diagnostic in &result.diagnostics {
            let _ = writeln!(stderr, "{}", diagnostic.to_json());
        }
    }
    println!("{document}");
    result.exit_code()
}

fn replay(package: &Path) -> i32 {
    let (document, code) = publish::replay_directory(package);
    println!("{document}");
    code
}
