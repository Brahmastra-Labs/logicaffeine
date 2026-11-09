//! `largo lean` — Lean in, LOGOS out, independently checked.
//!
//! ```text
//! largo lean import --project <dir> --target <Qualified.Name> --out <dir>   # the public experience
//! largo lean import <export.ndjson> --target <Qualified.Name> --out <dir>  # advanced interchange
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
//! only what needs the world: running the pinned Lean toolchain in isolation ([`project`]),
//! publishing the package atomically and replaying it in a fresh process ([`publish`]).

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
    #[command(after_help = "Exit codes: 0 accepted, 1 rejected, 2 declined; the reason on stderr.\n\nExamples:\n  largo lean check Mathlib.ndjson\n  largo lean check - < Mathlib.ndjson")]
    Check(CheckArgs),
}

#[derive(Args)]
struct CheckArgs {
    /// The lean4export 3.1.0 stream (`-`, or nothing, for standard input).
    export: Option<PathBuf>,
    /// Check with both of the core's engines, which must agree on every declaration.
    #[arg(long)]
    dual: bool,
}

#[derive(Args)]
struct ImportArgs {
    /// A lean4export 3.1.0 export (the advanced interchange input).
    export: Option<PathBuf>,

    /// A Lean project directory: compiled in isolation with the pinned Lean 4.14.0.
    #[arg(long, value_name = "DIR", conflicts_with = "export")]
    project: Option<PathBuf>,

    /// The Lean declaration to import, by its qualified Lean name.
    #[arg(long, value_name = "Qualified.Name")]
    target: Option<String>,

    /// Where to publish the package. Nothing is published unless every stage passes.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,

    /// The Lean Kernel Arena benchmark protocol: `ACCEPT`, exit 0/1/2, nothing else.
    #[arg(long, requires = "export", conflicts_with_all = ["project", "target", "out"])]
    arena_protocol: bool,

    /// (arena protocol) Report every checking stage as JSON on stderr.
    #[arg(long, requires = "arena_protocol")]
    trace_stages: bool,

    /// (arena protocol) Run the declaration audit.
    #[arg(long, requires = "arena_protocol")]
    audit_declarations: bool,

    /// The Lean executable (the elan shim selects the pinned 4.14.0 toolchain).
    #[arg(long, value_name = "PATH", default_value = "lean")]
    lean: PathBuf,

    /// The pinned lean4export checkout (revision 20c4a64a, built).
    /// Defaults to $LOGOS_LEAN4EXPORT, then ~/.logos/lean/lean4export.
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
                use logicaffeine_kernel::dag::{arena, tc::Engine};
                let _ = std::io::stdout().flush();
                arena::run(args.export, if args.dual { Engine::Dual } else { Engine::Sem })
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

    let checked = match (&args.export, &args.project) {
        (Some(export), None) => from_export(export, &target, &limits),
        (None, Some(project)) => {
            if !project.is_dir() {
                return usage(&format!(
                    "--project {} is not a directory",
                    project.display()
                ));
            }
            let options = project::Options {
                lean: args.lean.clone(),
                exporter: exporter_dir(args.exporter.as_deref()),
                timeout_secs: args.timeout_secs,
                max_export_bytes: args.max_export_bytes,
            };
            match project::export(project, &target, &options, deadline) {
                Ok(exported) => {
                    pipeline::check_export(&exported.export, &target, &exported.evidence, &limits)
                }
                Err(result) => pipeline::Checked {
                    result: *result,
                    package: None,
                },
            }
        }
        _ => return usage("give exactly one input: an export file or --project <DIR>"),
    };
    let mut result = checked.result;
    publish::conclude(&mut result, checked.package, &out, deadline);
    result.finish();
    emit(&result)
}

/// Export mode: the budget is checked against the file's size before a byte is read.
fn from_export(path: &Path, target: &str, limits: &Limits) -> pipeline::Checked {
    let refused = |code: DiagnosticCode, message: String| {
        let mut result = LeanResult::new(target);
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
        Ok(text) => pipeline::check_export(&text, target, &LeanEvidence::NotObserved, limits),
        Err(e) => refused(
            DiagnosticCode::Infrastructure,
            format!("cannot read {}: {e}", path.display()),
        ),
    }
}

/// The pinned exporter's directory: the flag, then `$LOGOS_LEAN4EXPORT`, then
/// `~/.logos/lean/lean4export`.
fn exporter_dir(flag: Option<&Path>) -> Option<PathBuf> {
    flag.map(Path::to_path_buf)
        .or_else(|| std::env::var_os("LOGOS_LEAN4EXPORT").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|home| home.join(".logos/lean/lean4export")))
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
