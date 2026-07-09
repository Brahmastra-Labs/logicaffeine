//! LOGOS CLI (largo) — argument parsing and dispatch.
//!
//! This module defines the `largo` command-line surface: the [`Cli`] parser,
//! the [`Commands`] enum, and [`run_cli`], which dispatches each subcommand
//! to its handler in [`crate::commands`].
//!
//! # Architecture
//!
//! The CLI is built on [`clap`] for argument parsing with derive macros.
//! Each command variant in [`Commands`] maps to a handler function in a
//! dedicated module under `commands/` that performs the actual work.
//!
//! # Examples
//!
//! ```bash
//! # Create a new project
//! largo new my_project
//!
//! # Build and run
//! cd my_project
//! largo run
//!
//! # Publish to registry
//! largo login
//! largo publish
//! ```

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::commands;
use crate::ui::{self, ColorMode};

/// Command-line interface for the LOGOS build tool.
///
/// The `Cli` struct is the top-level argument parser for `largo`. It delegates
/// to the [`Commands`] enum for subcommand handling.
///
/// # Usage
///
/// Typically invoked via [`run_cli`] which parses arguments and dispatches
/// to the appropriate handler:
///
/// ```no_run
/// use logicaffeine_cli::cli::run_cli;
///
/// if let Err(e) = run_cli() {
///     eprintln!("Error: {}", e);
///     std::process::exit(1);
/// }
/// ```
/// The version string, flavor-stamped: the full build (Z3 verification
/// statically linked) reports `X.Y.Z (full)` so installs are diagnosable.
#[cfg(feature = "verification")]
const LARGO_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (full)");
/// The lean build reports the bare version.
#[cfg(not(feature = "verification"))]
const LARGO_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "largo")]
#[command(about = "The LOGOS build tool", long_about = None)]
#[command(version = LARGO_VERSION)]
#[command(arg_required_else_help = true)]
#[command(styles = ui::CLAP_STYLES)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,

    /// Suppress informational output (errors still print).
    #[arg(long, short, global = true)]
    pub quiet: bool,

    /// Increase output verbosity (repeatable).
    #[arg(long, short, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// When to use terminal colors.
    #[arg(long, global = true, value_enum, default_value = "auto", value_name = "WHEN")]
    pub color: ColorMode,
}

/// Available CLI subcommands.
///
/// Each variant represents a distinct operation that `largo` can perform.
/// Commands are grouped into three categories:
///
/// ## Project Management
/// - [`New`][Commands::New] - Create a new project in a new directory
/// - [`Init`][Commands::Init] - Initialize a project in the current directory
///
/// ## Build & Run
/// - [`Build`][Commands::Build] - Compile the project
/// - [`Run`][Commands::Run] - Build and execute
/// - [`Check`][Commands::Check] - Type-check without building
/// - [`Verify`][Commands::Verify] - Run Z3 static verification
///
/// ## Package Registry
/// - [`Publish`][Commands::Publish] - Upload package to registry
/// - [`Login`][Commands::Login] - Authenticate with registry
/// - [`Logout`][Commands::Logout] - Remove stored credentials
#[derive(Subcommand)]
pub enum Commands {
    /// Create a new LOGOS project in a new directory.
    ///
    /// Scaffolds a complete project structure including:
    /// - `Largo.toml` manifest file
    /// - `src/main.lg` entry point with a "Hello, world!" example
    /// - `.gitignore` configured for LOGOS projects
    ///
    /// # Example
    ///
    /// ```bash
    /// largo new my_project
    /// cd my_project
    /// largo run
    /// ```
    #[command(after_help = "Examples:\n  largo new hello\n  cd hello\n  largo run")]
    New {
        /// The project name, used for the directory and package name.
        name: String,
    },

    /// Initialize a LOGOS project in the current directory.
    ///
    /// Similar to [`New`][Commands::New] but works in an existing directory.
    /// Creates the manifest and source structure without creating a new folder.
    ///
    /// # Example
    ///
    /// ```bash
    /// mkdir my_project && cd my_project
    /// largo init
    /// ```
    #[command(after_help = "Examples:\n  mkdir app && cd app\n  largo init\n  largo init --name my_app")]
    Init {
        /// Project name. If omitted, uses the current directory name.
        #[arg(long)]
        name: Option<String>,
    },

    /// Build the current project.
    ///
    /// Compiles the LOGOS source to Rust, then invokes `cargo build` on the
    /// generated code. The resulting binary is placed in `target/debug/` or
    /// `target/release/` depending on the mode.
    ///
    /// # Verification
    ///
    /// When `--verify` is passed, the build process includes Z3 static
    /// verification of logical constraints. This requires:
    /// - A Pro+ license (via `--license` or `LOGOS_LICENSE` env var)
    /// - The `verification` feature enabled at build time
    ///
    /// # Example
    ///
    /// ```bash
    /// largo build              # Debug build
    /// largo build --release    # Release build with optimizations
    /// largo build --verify     # Build with Z3 verification
    /// ```
    #[command(after_help = "Examples:\n  largo build\n  largo build --release\n  largo build --emit wasm\n  largo build --lib --target aarch64-unknown-linux-gnu")]
    Build {
        /// Build with optimizations enabled.
        #[arg(long, short)]
        release: bool,

        /// Run Z3 static verification after compilation.
        /// Requires a Pro+ license.
        #[arg(long)]
        verify: bool,

        /// License key for verification.
        /// Can also be set via the `LOGOS_LICENSE` environment variable.
        #[arg(long)]
        license: Option<String>,

        /// Build as a library instead of an executable.
        /// Generates `lib.rs` with `crate-type = ["cdylib"]` instead of a binary.
        #[arg(long)]
        lib: bool,

        /// Target triple for cross-compilation.
        /// Use "wasm" as shorthand for "wasm32-unknown-unknown".
        #[arg(long)]
        target: Option<String>,

        /// Pre-build every `is exported for native` function into the AOT-native
        /// tier bundle (a cached cdylib per function) under `.logos-native/`.
        #[arg(long)]
        native_functions: bool,

        /// Emit target. `wasm` compiles the project DIRECTLY to a self-contained `.wasm` module via the
        /// built-in backend (no rustc / cargo / wasm-bindgen — milliseconds), written to
        /// `target/<name>.wasm`. `wasm-linked` additionally links the real `logicaffeine_base::BigInt`
        /// runtime (via `rust-lld`) so overflowing integer arithmetic computes the exact big number
        /// instead of wrapping — needs the Rust toolchain + a wasm32 `base` build. Omit for the default
        /// rustc-based Rust build.
        #[arg(long)]
        emit: Option<String>,
    },

    /// Run Z3 static verification without building.
    ///
    /// Performs formal verification of logical constraints in the project
    /// using the Z3 SMT solver. This catches logical errors that would be
    /// impossible to detect through testing alone.
    ///
    /// Requires a Pro+ license.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo verify --license sub_xxxxx
    /// # Or with environment variable:
    /// export LOGOS_LICENSE=sub_xxxxx
    /// largo verify
    /// ```
    #[command(after_help = "Examples:\n  largo verify --license sub_xxxxx\n  LOGOS_LICENSE=sub_xxxxx largo verify")]
    Verify {
        /// License key for verification.
        /// Can also be set via the `LOGOS_LICENSE` environment variable.
        #[arg(long)]
        license: Option<String>,
    },

    /// Build and run the current project.
    ///
    /// Equivalent to `largo build` followed by executing the resulting binary.
    /// The exit code of the built program is propagated.
    ///
    /// With `--interpret`, skips Rust compilation and uses the tree-walking
    /// interpreter for sub-second feedback during development.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo run              # Debug mode (compile to Rust)
    /// largo run --release    # Release mode
    /// largo run --interpret  # Interpret directly (no compilation)
    /// ```
    #[command(after_help = "Examples:\n  largo run\n  largo run --release\n  largo run --interpret\n  largo run --emit wasm\n  largo run -- input.txt --program-flag")]
    Run {
        /// Build with optimizations enabled.
        #[arg(long, short)]
        release: bool,

        /// Run using the interpreter instead of compiling to Rust.
        /// Provides sub-second feedback but lacks full Rust performance.
        #[arg(long, short)]
        interpret: bool,

        /// `wasm` compiles DIRECTLY to a `.wasm` (built-in backend, no rustc) and runs it via the
        /// emitted host shim (node). `wasm-linked` links the real `BigInt` runtime first (exact
        /// arbitrary-precision integers; needs the Rust toolchain). Compile-and-run in one step.
        #[arg(long, conflicts_with = "interpret")]
        emit: Option<String>,

        /// Arguments to pass to the program.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Check the project for errors without producing a binary.
    ///
    /// Parses and type-checks the LOGOS source without invoking the full
    /// build pipeline. Useful for quick validation during development.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo check
    /// ```
    #[command(after_help = "Examples:\n  largo check\n  largo check --quiet")]
    Check {
        /// Also run rustc's analysis over the generated code (the same deep
        /// pass the IDE's flycheck uses) and translate its findings to LOGOS.
        #[arg(long)]
        deep: bool,
    },

    /// Report which optimizations actually FIRE when compiling a LOGOS file.
    ///
    /// Compiles the file on the AOT, run-path, and VM-compile paths with the
    /// firing trace on, and lists the optimizations that genuinely changed the
    /// program (not merely the ones that are enabled). Useful for understanding
    /// and auditing what the compiler did to a given program.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo opts src/main.lg
    /// largo opts src/main.lg --json
    /// ```
    #[command(after_help = "Examples:\n  largo opts src/main.lg\n  largo opts src/main.lg --json")]
    Opts {
        /// The `.lg` source file to analyze.
        file: PathBuf,

        /// Emit the fired optimizations as JSON (keyword list).
        #[arg(long)]
        json: bool,
    },

    /// Publish the package to the LOGOS registry.
    ///
    /// Packages the project as a tarball and uploads it to the specified
    /// registry. Requires authentication via `largo login`.
    ///
    /// # Pre-flight Checks
    ///
    /// Before publishing, the command verifies:
    /// - The entry point exists
    /// - No uncommitted git changes (unless `--allow-dirty`)
    /// - Valid authentication token
    ///
    /// # Example
    ///
    /// ```bash
    /// largo publish              # Publish to default registry
    /// largo publish --dry-run    # Validate without uploading
    /// ```
    #[command(after_help = "Examples:\n  largo publish --dry-run\n  largo publish\n  largo publish --allow-dirty")]
    Publish {
        /// Registry URL. Defaults to `registry.logicaffeine.com`.
        #[arg(long)]
        registry: Option<String>,

        /// Perform all validation without actually uploading.
        /// Useful for testing the publish process.
        #[arg(long)]
        dry_run: bool,

        /// Allow publishing with uncommitted git changes.
        /// By default, publishing requires a clean working directory.
        #[arg(long)]
        allow_dirty: bool,
    },

    /// Authenticate with the package registry.
    ///
    /// Stores an API token for the specified registry. The token is saved
    /// in `~/.config/logos/credentials.toml` with restricted permissions.
    ///
    /// # Token Acquisition
    ///
    /// Tokens can be obtained from the registry's web interface:
    /// 1. Visit `{registry}/auth/github` to authenticate
    /// 2. Generate an API token from your profile
    /// 3. Provide it via `--token` or interactive prompt
    ///
    /// # Example
    ///
    /// ```bash
    /// largo login                       # Interactive prompt
    /// largo login --token tok_xxxxx     # Non-interactive
    /// ```
    #[command(after_help = "Examples:\n  largo login\n  largo login --token lgr_xxxxx")]
    Login {
        /// Registry URL. Defaults to `registry.logicaffeine.com`.
        #[arg(long)]
        registry: Option<String>,

        /// API token. If omitted, prompts for input on stdin.
        #[arg(long)]
        token: Option<String>,
    },

    /// Remove stored credentials for a registry.
    ///
    /// Deletes the authentication token from the local credentials file.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo logout
    /// ```
    #[command(after_help = "Examples:\n  largo logout\n  largo logout --registry https://registry.example.com")]
    Logout {
        /// Registry URL. Defaults to `registry.logicaffeine.com`.
        #[arg(long)]
        registry: Option<String>,
    },

    /// Diagnose the environment largo runs in.
    ///
    /// Checks the Rust toolchain (needed by `build`/`run`), the wasm32
    /// target, node (for `--emit wasm`), the verification flavor, registry
    /// reachability and credentials, update freshness, and — inside a
    /// project — manifest health. Degradations are warnings; only a broken
    /// project fails. Works offline.
    #[command(after_help = "Examples:\n  largo doctor\n  largo doctor --registry https://registry.example.com")]
    Doctor {
        /// Registry URL to probe (defaults to the LOGOS registry).
        #[arg(long)]
        registry: Option<String>,
    },

    /// Start the interactive LOGOS REPL.
    ///
    /// Two modes in one session: imperative statements against a
    /// persistent interpreter session (`logos>`), and English→FOL logic
    /// mode with discourse-aware anaphora (`logic>`). Type `:help` inside
    /// for the meta-commands (`:mode`, `:format`, `:readings`, `:vars`,
    /// `:save`, …). Works on a pipe too — no terminal required.
    #[command(after_help = "Examples:\n  largo repl\n  largo repl --logic\n  largo repl --logic --format latex\n  largo repl --load session.lg\n  printf 'Let x be 5.\\nShow x.\\n' | largo repl")]
    Repl {
        /// Start in logic mode (English → FOL).
        #[arg(long)]
        logic: bool,

        /// Initial logic output format.
        #[arg(long, value_enum)]
        format: Option<crate::commands::logic::LogicFormat>,

        /// Load a saved session/program on startup.
        #[arg(long)]
        load: Option<PathBuf>,
    },

    /// Solve a DIMACS CNF with the certified SAT engine.
    ///
    /// The SAT Competition interface as a largo verb: prints
    /// `s SATISFIABLE` with a `v` model or `s UNSATISFIABLE`, optionally
    /// exporting a DRAT/DPR/SR refutation for external checkers
    /// (drat-trim). Exit codes follow the competition convention:
    /// 10 = SAT, 20 = UNSAT, 1 = error.
    #[command(after_help = "Examples:\n  largo sat instance.cnf\n  largo sat instance.cnf --proof refutation.drat\n  largo sat instance.cnf --stats")]
    Sat {
        /// The DIMACS CNF file to solve.
        file: PathBuf,

        /// Write the UNSAT certificate here (DRAT; DPR/SR for symmetry routes).
        #[arg(long)]
        proof: Option<PathBuf>,

        /// Print solver statistics to stderr.
        #[arg(long)]
        stats: bool,
    },

    /// Prove the theorems in a LOGOS source file (kernel-certified).
    ///
    /// Runs `## Theory` developments (formal Axiom/Theorem declarations,
    /// proved in citation order) and English `## Theorem` blocks
    /// (Given/Prove/Proof) through the proof engine. Every ✓ is certified
    /// by the type-theory kernel — a mere derivation never counts.
    #[command(after_help = "Examples:\n  largo prove                 # prove the project entry\n  largo prove geometry.lg\n  largo prove socrates.lg --trace\n  largo prove tarski.lg --json")]
    Prove {
        /// The source file (defaults to the project entry).
        file: Option<PathBuf>,

        /// Show the rendered derivation tree under each proved theorem.
        #[arg(long)]
        trace: bool,

        /// Emit machine-readable JSON results.
        #[arg(long, conflicts_with = "trace")]
        json: bool,
    },

    /// Translate English to First-Order Logic.
    ///
    /// Compiles a natural-language sentence to formal logic — the LOGOS
    /// logic mode from the terminal. Reads the sentence inline, from
    /// `--file`, or from piped stdin. Prints bare FOL on stdout, so output
    /// pipes cleanly into other tools.
    #[command(after_help = "Examples:\n  largo logic \"Every woman loves a man.\"\n  largo logic \"Every woman loves a man.\" --all-readings\n  largo logic \"It might rain.\" --format kripke\n  echo \"Socrates is mortal.\" | largo logic\n  printf 'A farmer owns a donkey.\\nHe feeds it.' | largo logic --discourse")]
    Logic {
        /// The English sentence to translate.
        sentence: Option<String>,

        /// Read the sentence (or discourse) from a file.
        #[arg(long, short, conflicts_with = "sentence")]
        file: Option<PathBuf>,

        /// Output format for the logical form.
        #[arg(long, value_enum, default_value = "unicode")]
        format: crate::commands::logic::LogicFormat,

        /// Show every reading (quantifier scopes + parse forest), numbered.
        #[arg(long, conflicts_with = "discourse")]
        all_readings: bool,

        /// Enrich with scalar implicature (pragmatic strengthening).
        #[arg(long)]
        pragmatic: bool,

        /// Treat each input line as one sentence of a discourse with
        /// shared anaphora context.
        #[arg(long)]
        discourse: bool,
    },

    /// Generate documentation from the project's `##` blocks.
    ///
    /// Renders a markdown reference from the literate structure of the
    /// entry file: `## To` signatures, type definitions, notes, examples,
    /// and formal blocks — in source order. `## Main` is omitted.
    #[command(after_help = "Examples:\n  largo doc\n  largo doc --out book")]
    Doc {
        /// Output directory (defaults to `target/doc`).
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Add a dependency to Largo.toml.
    ///
    /// Accepts `name` (any version), `name@version`, or `logos:name` (the
    /// registry URI form). `--path` and `--git` record local and git
    /// dependencies. Edits preserve the manifest's comments and formatting.
    #[command(after_help = "Examples:\n  largo add math_utils\n  largo add math_utils@1.2\n  largo add logos:std\n  largo add local_lib --path ../local_lib\n  largo add remote --git https://example.com/remote.git")]
    Add {
        /// The dependency: `name`, `name@version`, or `logos:name`.
        spec: String,

        /// Use a local path dependency.
        #[arg(long, conflicts_with = "git")]
        path: Option<String>,

        /// Use a git dependency.
        #[arg(long, conflicts_with = "path")]
        git: Option<String>,
    },

    /// Remove a dependency from Largo.toml.
    ///
    /// Deletes the named entry from `[dependencies]`, leaving the rest of
    /// the manifest byte-identical.
    #[command(after_help = "Examples:\n  largo remove math_utils")]
    Remove {
        /// The dependency name to remove.
        name: String,
    },

    /// Format LOGOS source files.
    ///
    /// Applies the canonical style (4-space indentation, no tabs, no
    /// trailing whitespace) — the same rules the language server uses.
    /// Without paths, formats the whole project; with paths, exactly those
    /// files. `--check` writes nothing and exits 1 if anything would change.
    #[command(after_help = "Examples:\n  largo fmt\n  largo fmt src/main.lg\n  largo fmt --check    # CI gate, writes nothing")]
    Fmt {
        /// Specific files to format (defaults to all project sources).
        paths: Vec<PathBuf>,

        /// Check only: list files that need formatting, exit 1 if any.
        #[arg(long)]
        check: bool,
    },

    /// Emit compiled code without building a binary.
    ///
    /// Prints the generated Rust or C translation of the program, or writes
    /// a self-contained `.wasm` module (built-in backend, no rustc) with its
    /// Node.js host shim. Without FILE, uses the current project's entry;
    /// with FILE, works on any standalone `.lg`/`.md` source.
    #[command(after_help = "Examples:\n  largo emit rust\n  largo emit rust -o generated.rs\n  largo emit c standalone.lg\n  largo emit wasm\n  largo emit wasm-linked -o dist/app.wasm")]
    Emit {
        /// What to emit.
        #[arg(value_enum)]
        target: crate::commands::emit::EmitTarget,

        /// A standalone source file (defaults to the project entry).
        file: Option<PathBuf>,

        /// Write to this path instead of stdout (rust/c) or the default
        /// module path (wasm).
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Remove build artifacts.
    ///
    /// Deletes the project's `target/` directory. With `--all`, also removes
    /// the `.logos-native/` compiled-function bundle cache produced by
    /// `largo build --native-functions`.
    #[command(after_help = "Examples:\n  largo clean\n  largo clean --all")]
    Clean {
        /// Also remove the `.logos-native/` bundle cache.
        #[arg(long)]
        all: bool,
    },

    /// Generate shell completions for largo.
    ///
    /// Writes a completion script for the given shell to stdout. Source it
    /// from your shell's configuration to get tab completion for every
    /// largo command and flag.
    #[command(after_help = "Examples:\n  largo completions bash > ~/.local/share/bash-completion/completions/largo\n  largo completions zsh > ~/.zfunc/_largo\n  largo completions fish > ~/.config/fish/completions/largo.fish")]
    Completions {
        /// The shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Reserved for the LOGOS test framework (coming in a future release).
    #[command(hide = true)]
    Test {
        /// Ignored; the verb is reserved.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        args: Vec<String>,
    },
}

/// Parse CLI arguments and execute the corresponding command.
///
/// This is the main entry point for the `largo` CLI. It parses command-line
/// arguments using [`clap`], then dispatches to the appropriate handler
/// function based on the subcommand.
///
/// # Errors
///
/// Returns an error if:
/// - The project structure is invalid (missing `Largo.toml`)
/// - File system operations fail
/// - Build or compilation fails
/// - Registry operations fail (authentication, network, etc.)
///
/// # Example
///
/// ```no_run
/// use logicaffeine_cli::cli::run_cli;
///
/// fn main() {
///     if let Err(e) = run_cli() {
///         eprintln!("Error: {}", e);
///         std::process::exit(1);
///     }
/// }
/// ```
pub fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    ui::init(cli.color, cli.quiet, cli.verbose);

    match cli.command {
        Commands::New { name } => commands::new::cmd_new(&name),
        Commands::Init { name } => commands::new::cmd_init(name.as_deref()),
        Commands::Build { release, verify, license, lib, target, native_functions, emit } => {
            commands::build::cmd_build(release, verify, license, lib, target, native_functions, emit)
        }
        Commands::Run { emit: Some(e), args, .. } if e == "wasm" => commands::run::cmd_run_wasm(&args, false),
        Commands::Run { emit: Some(e), args, .. } if e == "wasm-linked" => commands::run::cmd_run_wasm(&args, true),
        Commands::Run { emit: Some(e), .. } => {
            Err(format!("unknown --emit target '{e}' (expected 'wasm' or 'wasm-linked')").into())
        }
        Commands::Run { interpret, args, .. } if interpret => commands::run::cmd_run_interpret(&args),
        Commands::Run { release, args, .. } => commands::run::cmd_run(release, &args),
        Commands::Check { deep } => commands::check::cmd_check(deep),
        Commands::Opts { file, json } => commands::opts::cmd_opts(&file, json),
        Commands::Verify { license } => commands::verify::cmd_verify(license),
        Commands::Publish { registry, dry_run, allow_dirty } => {
            commands::publish::cmd_publish(registry.as_deref(), dry_run, allow_dirty)
        }
        Commands::Login { registry, token } => commands::publish::cmd_login(registry.as_deref(), token),
        Commands::Logout { registry } => commands::publish::cmd_logout(registry.as_deref()),
        Commands::Doctor { registry } => commands::doctor::cmd_doctor(registry),
        Commands::Repl { logic, format, load } => crate::repl::cmd_repl(logic, format, load),
        Commands::Sat { file, proof, stats } => commands::sat::cmd_sat(file, proof, stats),
        Commands::Prove { file, trace, json } => commands::prove::cmd_prove(file, trace, json),
        Commands::Logic { sentence, file, format, all_readings, pragmatic, discourse } => {
            commands::logic::cmd_logic(sentence, file, format, all_readings, pragmatic, discourse)
        }
        Commands::Doc { out } => commands::doc::cmd_doc(out),
        Commands::Add { spec, path, git } => commands::deps::cmd_add(spec, path, git),
        Commands::Remove { name } => commands::deps::cmd_remove(name),
        Commands::Fmt { paths, check } => commands::fmt::cmd_fmt(paths, check),
        Commands::Emit { target, file, output } => commands::emit::cmd_emit(target, file, output),
        Commands::Clean { all } => commands::clean::cmd_clean(all),
        Commands::Completions { shell } => commands::completions::cmd_completions(shell),
        Commands::Test { .. } => Err(ui::CliError::with_hint(
            "`largo test` is reserved for the LOGOS test framework (coming in a future release)",
            "run `largo check` to validate your project today",
        )
        .exit_code(ui::EXIT_USAGE)
        .into()),
    }
}
