//! LOGOS CLI (largo) — argument parsing and dispatch.
//!
//! This module defines the `largo` command-line surface: the [`Cli`] parser,
//! the [`Commands`] enum, and [`run_cli`], which dispatches each subcommand
//! to its handler in `crate::commands`.
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
    #[arg(
        long,
        global = true,
        value_enum,
        default_value = "auto",
        value_name = "WHEN"
    )]
    pub color: ColorMode,
}

/// The execution tier selected by `largo run --interpret`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum RunEngine {
    /// Optimized bytecode VM with native JIT tier-up.
    #[default]
    Jit,
    /// Optimized bytecode VM without native tier-up.
    Vm,
    /// Optimized tree-walking interpreter.
    Tree,
}

impl From<RunEngine> for logicaffeine_compile::ui_bridge::Engine {
    fn from(engine: RunEngine) -> Self {
        match engine {
            RunEngine::Jit => Self::Jit,
            RunEngine::Vm => Self::Vm,
            RunEngine::Tree => Self::TreeWalker,
        }
    }
}

/// Available CLI subcommands.
///
/// Each variant is one operation `largo` can perform, and `largo --help` lists every one.
/// They span project management, building and running, the package registry, logic and
/// proof (such as `logic` and `prove`), and interactive, editor and agent tooling (such as
/// `repl`, `dap` and `mcp`). A sample from the first three groups:
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
    #[command(
        after_help = "Examples:\n  largo new hello\n  cd hello\n  largo run\n\n  largo new mysite --site\n  cd mysite\n  largo serve\n\n  largo new myapp --web\n  cd myapp\n  largo serve"
    )]
    New {
        /// The project name, used for the directory and package name.
        name: String,

        /// Scaffold a full-stack website: a backend of `## Route`s ending in
        /// `Serve on port 8080.` (`src/main.lg`), pages compiled to WASM
        /// (`pages/`), and static assets (`static/`) — all served by one
        /// `largo serve`. Pure convention: no extra manifest keys.
        #[arg(long, conflicts_with = "site")]
        web: bool,

        /// Scaffold a site: pages compiled to WASM (`pages/`) and static assets
        /// (`static/`), and no backend and no entry program — a `## Page`
        /// mounts itself. `largo serve` shows it; `largo build` exports it to
        /// `dist/` for any static host.
        #[arg(long)]
        site: bool,
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
    #[command(
        after_help = "Examples:\n  mkdir app && cd app\n  largo init\n  largo init --name my_app"
    )]
    Init {
        /// Project name. If omitted, uses the current directory name.
        #[arg(long)]
        name: Option<String>,
    },

    /// Build the current project.
    ///
    /// A program or a backend: compiles the LOGOS source to Rust, then invokes
    /// `cargo build` on the generated code; the binary is published at
    /// `target/debug/<name>` or `target/release/<name>`. A full-stack site's
    /// binary carries its pages and assets inside it. A site (pages, no `Serve`
    /// backend) has no program to compile, so it is exported to `dist/` — the
    /// same as `--static`.
    ///
    /// # Verification
    ///
    /// When `--verify` is passed, the build process includes Z3 static
    /// verification of logical constraints. This requires the `verification`
    /// feature enabled at build time (the full build).
    ///
    /// # Example
    ///
    /// ```bash
    /// largo build              # Debug build
    /// largo build --release    # Release build with optimizations
    /// largo build --verify     # Build with Z3 verification
    /// ```
    #[command(
        after_help = "Examples:\n  largo build\n  largo build --release\n  largo build --static        # export pages/ and static/ to dist/\n  largo build --emit wasm\n  largo build --lib --target aarch64-unknown-linux-gnu"
    )]
    Build {
        /// Build with optimizations enabled.
        #[arg(long, short)]
        release: bool,

        /// Run Z3 static verification after compilation (the full build).
        #[arg(long)]
        verify: bool,

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

        /// Emit target: `c`, `c-bytecode`, `wasm`, `wasm-linked`, `isa` or `riscv`.
        /// `c` and `c-bytecode` build native executables from the optimized AST and bytecode respectively.
        /// `wasm` compiles the project straight to a
        /// self-contained `.wasm` module via the built-in backend (no rustc, cargo or wasm-bindgen),
        /// written to `target/<name>.wasm` beside a Node.js host shim. `wasm-linked` additionally links the
        /// real `logicaffeine_base::BigInt` runtime (via `rust-lld`) so overflowing integer arithmetic
        /// computes the exact big number instead of wrapping; it needs the Rust toolchain and a wasm32
        /// `base` build. `isa` writes LOGOS Silicon ISA assembly beside the entry as `<entry>.isa`.
        /// `riscv` (also `risc-v`) writes RV64GC assembly beside the entry as `<entry>.s`, with the
        /// `logos_rt.c` runtime shim next to it. Omit for the default rustc-based Rust build.
        #[arg(long)]
        emit: Option<String>,

        /// Print one `logos-check/1` JSON envelope on stdout: a parse error
        /// reports as a structured diagnostic, a cargo failure as one
        /// `build-failure` diagnostic carrying the raw output.
        #[arg(long)]
        json: bool,

        /// Print the machine-readable build manifest (JSON) instead of building:
        /// the local package identity, feature selection, the resolved
        /// `path`/`git` dependencies, and the transitive closure (every linked
        /// `.lg` file tagged with its source dependency). No cargo/rustc runs.
        #[arg(long)]
        manifest: bool,

        /// Export the site to `dist/` as plain files instead of building a binary —
        /// pre-rendered HTML, the content-addressed module and shim, `static/**`,
        /// `robots.txt`, `sitemap.xml` and a `_headers` file carrying the same cache
        /// rules the binary sends. Deployable to any static host (Pages, Netlify,
        /// Cloudflare, S3). No cargo, no rustc, and no backend required. What a
        /// plain `largo build` does for a site; ask for it on a full-stack site to
        /// export its pages without the backend.
        #[arg(long)]
        r#static: bool,
    },

    /// Run Z3 static verification without building.
    ///
    /// Performs formal verification of logical constraints in the project
    /// using the Z3 SMT solver. This catches logical errors that would be
    /// impossible to detect through testing alone.
    ///
    /// Needs the full build (the `verification` feature).
    ///
    /// # Example
    ///
    /// ```bash
    /// largo verify
    /// ```
    #[command(after_help = "Examples:\n  largo verify")]
    Verify {},

    /// Build and run the current project.
    ///
    /// Equivalent to `largo build` followed by executing the resulting binary.
    /// The exit code of the built program is propagated.
    ///
    /// With `--interpret`, skips the Rust build and runs the program in-process on the
    /// bytecode VM, with JIT tier-up on x86-64 Linux and macOS, for fast feedback during
    /// development. A concurrent or asynchronous program, or a construct the VM compiler
    /// does not take, runs on the tree-walking interpreter instead.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo run              # Debug mode (compile to Rust)
    /// largo run --release    # Release mode
    /// largo run --interpret  # Run in-process on the VM (no Rust build)
    /// ```
    #[command(
        after_help = "Examples:\n  largo run\n  largo run --release\n  largo run --interpret\n  largo run --emit wasm\n  largo run -- input.txt --program-flag"
    )]
    Run {
        /// Build with optimizations enabled.
        #[arg(long, short)]
        release: bool,

        /// Run in-process on the bytecode VM (JIT where available) instead of building Rust.
        /// Gives fast feedback but lacks full compiled-Rust performance.
        #[arg(long, short)]
        interpret: bool,

        /// `wasm` compiles straight to a `.wasm` (built-in backend, no rustc) and runs it via the
        /// emitted host shim (node). `wasm-linked` links the real `BigInt` runtime first (exact
        /// arbitrary-precision integers; needs the Rust toolchain). Compile-and-run in one step.
        #[arg(long, conflicts_with = "interpret")]
        emit: Option<String>,

        /// In-process execution tier. Valid only with `--interpret`.
        #[arg(long, value_enum, value_name = "ENGINE", requires = "interpret")]
        engine: Option<RunEngine>,

        /// Stop after the engine executes N steps (`--interpret` only) —
        /// deterministic, the same op on every run. Exhaustion exits 122 with the
        /// partial output delivered.
        #[arg(long, value_name = "N")]
        max_steps: Option<u64>,

        /// Kill the run after this wall clock (`500ms`, `10s`, `2m`).
        /// Expiry exits 124 (the GNU timeout convention).
        #[arg(long, value_name = "DURATION")]
        timeout: Option<String>,

        /// Scheduler seed for deterministic concurrent runs (sets LOGOS_SEED
        /// for this run and any compiled child).
        #[arg(long)]
        seed: Option<u64>,

        /// Print one `logos-run/1` JSON envelope on stdout (`--interpret`
        /// only): outcome, output lines, error, steps used.
        #[arg(long)]
        json: bool,

        /// Arguments to pass to the program.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Serve the project over HTTP.
    ///
    /// What is served is decided from the project itself. A SITE — `pages/` or
    /// `static/` and no `Serve` backend — gets the dev server: largo owns the
    /// public port, serves the built pages and assets with hot reload, and
    /// answers anything else 404 as a static host would. A FULL-STACK site adds
    /// its `Serve` backend as a child on `LOGOS_PORT`, proxied for everything the
    /// pages do not answer. A BACKEND — a `Serve` program alone — is interpreted
    /// directly and binds its own port. Any other program is built to
    /// WebAssembly and served as a bundle.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo serve
    /// ```
    #[command(
        after_help = "Examples:\n  largo serve                  # site, full-stack site, or backend — decided by the project\n  largo serve --port 3000\n  largo serve --open           # …and open it in your browser\n  largo serve --host 0.0.0.0   # reachable from a phone on the same network\n  largo serve --timeout 30s\n  largo serve --watch          # a program's bundle, rebuilt + live-reloaded on save"
    )]
    Serve {
        /// Kill the server after this wall clock (`500ms`, `10s`, `2m`).
        #[arg(long, value_name = "DURATION")]
        timeout: Option<String>,

        /// Scheduler seed for deterministic runs (sets LOGOS_SEED).
        #[arg(long)]
        seed: Option<u64>,

        /// Serve a program's WebAssembly bundle, rebuilding and live-reloading
        /// the browser on every source save. (A site always watches.)
        #[arg(long)]
        watch: bool,

        /// Open the served site in your default browser (best-effort).
        #[arg(long)]
        open: bool,

        /// Bind address for a site. Default `127.0.0.1` (this machine only);
        /// pass `0.0.0.0` to serve the whole local network, so a phone or a
        /// tablet on the same wifi can open the site.
        #[arg(long, value_name = "ADDR")]
        host: Option<String>,

        /// The public port for a site or a bundle (falls forward to the next
        /// free port if taken). Default 8080.
        #[arg(long)]
        port: Option<u16>,

        /// (internal) Run as the full-stack dev server's backend child: the
        /// interpreter serves the entry on the port `LOGOS_PORT` names, and
        /// exits when the parent's stdin lifeline closes. Spawned by
        /// `largo serve` itself.
        #[arg(long, hide = true)]
        as_backend: bool,

        /// Arguments to pass to the program (readable via `args()`).
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
    #[command(
        after_help = "Examples:\n  largo check\n  largo check --quiet\n  largo check --kernel Demo.andSwap.lg"
    )]
    Check {
        /// A LOGOS kernel-vernacular document to check (with `--kernel`)
        /// instead of the project.
        #[arg(requires = "kernel")]
        file: Option<PathBuf>,

        /// Check FILE as a LOGOS kernel-vernacular literate document through
        /// the kernel's own document door (`Repl::execute_document`): every
        /// block, in order, stopping at the first that does not check.
        #[arg(long, requires = "file", conflicts_with = "deep")]
        kernel: bool,

        /// Also run rustc's analysis over the generated code (the same deep
        /// pass the IDE's flycheck uses) and translate its findings to LOGOS.
        #[arg(long)]
        deep: bool,

        /// Print one `logos-check/1` JSON envelope on stdout (diagnostics
        /// with stable codes, spans, and Socratic explanations). Prose stays
        /// on stderr; exit codes are unchanged.
        #[arg(long)]
        json: bool,
    },

    /// Report which optimizations actually fire when compiling a LOGOS file.
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
    /// largo opts src/main.lg --json --jit -- 1000
    /// ```
    #[command(
        after_help = "Examples:\n  largo opts src/main.lg\n  largo opts src/main.lg --json\n  largo opts src/main.lg --json --jit -- 1000"
    )]
    Opts {
        /// The `.lg` source file to analyze.
        file: PathBuf,

        /// Emit the fired optimizations as JSON (keyword list).
        #[arg(long)]
        json: bool,

        /// Execute through a private native tier and include a distinct JIT
        /// optimizer/lowering/decline trace. Program arguments follow `--`.
        #[arg(long)]
        jit: bool,

        /// Arguments presented to the traced program after `argv[0]`.
        #[arg(last = true)]
        args: Vec<String>,
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
    #[command(
        after_help = "Examples:\n  largo publish --dry-run\n  largo publish\n  largo publish --allow-dirty"
    )]
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
    #[command(
        after_help = "Examples:\n  largo logout\n  largo logout --registry https://registry.example.com"
    )]
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
    #[command(
        after_help = "Examples:\n  largo doctor\n  largo doctor --registry https://registry.example.com"
    )]
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
    #[command(
        after_help = "Examples:\n  largo repl\n  largo repl --logic\n  largo repl --logic --format latex\n  largo repl --load session.lg\n  printf 'Let x be 5.\\nShow x.\\n' | largo repl"
    )]
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
    #[command(
        after_help = "Examples:\n  largo sat instance.cnf\n  largo sat instance.cnf --proof refutation.drat\n  largo sat instance.cnf --stats"
    )]
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
    #[command(
        after_help = "Examples:\n  largo prove                 # prove the project entry\n  largo prove geometry.lg\n  largo prove socrates.lg --trace\n  largo prove tarski.lg --json\n  largo prove socrates.lg --certificate   # emit re-checkable proof certificates\n  largo prove --kernel Demo.andSwap.lg    # a kernel-vernacular document"
    )]
    Prove {
        /// The source file (defaults to the project entry).
        file: Option<PathBuf>,

        /// Check FILE as a LOGOS kernel-vernacular literate document through
        /// the kernel's own document door (`Repl::execute_document`) — the
        /// form `largo lean import` emits.
        #[arg(long, requires = "file", conflicts_with_all = ["trace", "certificate"])]
        kernel: bool,

        /// Show the rendered derivation tree under each proved theorem.
        #[arg(long)]
        trace: bool,

        /// Emit machine-readable JSON results.
        #[arg(long, conflicts_with = "trace")]
        json: bool,

        /// Emit the independently re-checkable proof certificate for every
        /// kernel-proved theorem as JSON (the proof term + the goal + the
        /// theorem's disclosed premises), each re-verified against the kernel.
        /// A third party can re-check these with only the kernel — no prover.
        #[arg(long, conflicts_with_all = ["trace", "json"])]
        certificate: bool,
    },

    /// Translate English to First-Order Logic.
    ///
    /// Compiles a natural-language sentence to formal logic — the LOGOS
    /// logic mode from the terminal. Reads the sentence inline, from
    /// `--file`, or from piped stdin. Prints bare FOL on stdout, so output
    /// pipes cleanly into other tools.
    #[command(
        after_help = "Examples:\n  largo logic \"Every woman loves a man.\"\n  largo logic \"Every woman loves a man.\" --all-readings\n  largo logic \"It might rain.\" --format kripke\n  echo \"Socrates is mortal.\" | largo logic\n  printf 'A farmer owns a donkey.\\nHe feeds it.' | largo logic --discourse"
    )]
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
    #[command(
        after_help = "Examples:\n  largo add math_utils\n  largo add math_utils@1.2\n  largo add logos:std\n  largo add local_lib --path ../local_lib\n  largo add remote --git https://example.com/remote.git"
    )]
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
    #[command(
        after_help = "Examples:\n  largo fmt\n  largo fmt src/main.lg\n  largo fmt --check    # CI gate, writes nothing"
    )]
    Fmt {
        /// Specific files to format (defaults to all project sources).
        paths: Vec<PathBuf>,

        /// Check only: list files that need formatting, exit 1 if any.
        #[arg(long)]
        check: bool,

        /// With --check: print one `logos-fmt/1` JSON envelope on stdout.
        #[arg(long)]
        json: bool,
    },

    /// Emit compiled code without building a binary.
    ///
    /// Prints the generated Rust or C translation of the program, or writes
    /// a self-contained `.wasm` module (built-in backend, no rustc) with its
    /// Node.js host shim. Without FILE, uses the current project's entry;
    /// with FILE, works on any standalone `.lg`/`.md` source.
    #[command(
        after_help = "Examples:\n  largo emit rust\n  largo emit rust -o generated.rs\n  largo emit c standalone.lg\n  largo emit wasm\n  largo emit wasm-linked -o dist/app.wasm\n  largo emit rdu --chip sn50 qwen38_tiny.lg -o out/\n  largo emit rdu --format vllm qwen38_tiny.lg\n  largo emit rdu --model qwen38 -o qwen38_sn50/\n  largo emit rdu --model qwen38 --ctx 4096 -o qwen38_sn50_decode/\n  largo emit rdu --model qwen38 --ctx 4096 --batch 8 -o qwen38_sn50_batch8/\n  largo emit rdu --model qwen38 --ctx 4096 --tp 8 -o qwen38_sn50_tp8/\n  largo emit rdu --model qwen38 --ctx 4096 --prefill 512 -o qwen38_sn50_prefill/"
    )]
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

        /// `emit rdu`: the RDU generation to lower onto — `sn40l` or `sn50` (the default).
        #[arg(long, value_name = "CHIP")]
        chip: Option<String>,

        /// `emit rdu`: the artifact to print bare / add to the bundle — `placement` (the default; the
        /// neutral placement text), `spatial` (Spatial source), `sambaflow` (PyTorch module + compile
        /// scaffold), `vllm` (the Qwen3.8 deployment plan for the chip), `report` (the MODELED II report).
        #[arg(long, value_enum, value_name = "FORMAT")]
        format: Option<crate::commands::emit::RduFormat>,

        /// `emit rdu`: hand the certified artifact to an SDK bridge by name (`mock`, `sambaflow`).
        #[arg(long, value_name = "BRIDGE")]
        bridge: Option<String>,

        /// `emit rdu`: lower a whole model instead of FILE — `qwen38` (Qwen3.8-27B, all 64 layers), `qwen38-tiny`,
        /// `qwen38-mini` (the tiny shape with heads that split over 2 or 4 chips), or a Qwen3.5/3.8 checkpoint's
        /// `.gguf`, its program read from the checkpoint — as one certified program per layer kind plus the head, the
        /// program they are cuts of (`program.lg`) and a manifest binding every layer and naming every program's cut.
        /// Needs `-o DIR`.
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,

        /// `emit rdu --model`: lower the DECODE deployment with a KV cache of CTX positions — an embedding
        /// program, layer kinds whose attention appends to and reads the cache, and a head that emits the next
        /// token; `tok` and `pos` are runtime inputs. Without it, the bundle is the authored forward.
        #[arg(long, value_name = "CTX")]
        ctx: Option<usize>,

        /// `emit rdu --model --ctx`: serve BATCH sequences per decode step — the hidden states stacked `BATCH × d`,
        /// every weight read once for all of them, each sequence with its own KV caches and carried state;
        /// `tok_s{b}` and `pos_s{b}` are runtime inputs. `auto`: the most sequences whose per-step working set stays
        /// in a chip's SRAM (the serving plan). The default is one sequence.
        #[arg(long, value_name = "BATCH")]
        batch: Option<String>,

        /// `emit rdu --model --ctx`: TENSOR-PARALLEL over TP chips — every layer kind one program over the chips,
        /// each chip its own heads and FFN columns and a slice of every weight, the partials summed by two
        /// all-reduces a layer. TP must split the model's heads (Qwen3.8-27B: 1, 2, 4 or 8). `auto`: the widest width
        /// the heads admit in the chip's scale-up domain. The default is one chip.
        #[arg(long, value_name = "TP")]
        tp: Option<String>,

        /// `emit rdu --model --ctx`: PREFILL a chunk of PREFILL prompt tokens of one sequence per step — the chunk's
        /// hidden states stacked `PREFILL × d`, every weight read once for the chunk, the rows taking the sequence's KV
        /// caches and recurrent memories in order, the last row's next token out. The state is the decode bundle's,
        /// so decoding continues from it; `tok_s{b}` and `pos_s{b}` are runtime inputs.
        #[arg(long, value_name = "PREFILL")]
        prefill: Option<usize>,
    },

    /// Bring a model up on every chip from its one program.
    ///
    /// Reads the checkpoint's spec, writes the model's computation as one LOGOS program (`program.lg`) and lowers that
    /// program on each backend asked for: the CPU tiers (the VM's bytecode, the same with its JIT, C, Rust, RISC-V and
    /// WebAssembly), the CUDA lane (one fused `.cu` and the sidecar of constants it reads), the D5005 decode engine (the
    /// sequencer's program), the SN50 (a certified bundle) and the fabric. Every artifact names the cut of the program
    /// it was lowered from (`LOGOS program: <cut>, sha256 <hex>`); a backend that does not run the model refuses it by
    /// name. With `--run`, every chip that brought the model up and is on this machine then generates from a prompt
    /// (or every prompt of a golden, `--prompts`) through the one program, and reports its tokens, its tokens a second
    /// and how many lead the reference's.
    #[command(
        after_help = "Examples:\n  largo bringup models/qwen3.8-27b/Qwen3.8-27B-Q4_K_M.gguf -o qwen38/\n  largo bringup llama.gguf --target cuda --ctx 1024 -o llama/\n  largo bringup qwen3.5-0.8b.gguf --target vm,c,cuda --ctx 64 --run --tokens 8 -o qwen35/\n  largo bringup qwen3.5-0.8b.gguf --target cuda --run --golden benchmarks/models/qwen3.5-0.8b/golden-f32.json -o qwen35/\n  largo bringup qwen3.5-0.8b.gguf --target jit,c --run --golden benchmarks/models/qwen3.5-0.8b/golden-f32.json --prompts all -o qwen35/"
    )]
    Bringup {
        /// The model's GGUF checkpoint.
        checkpoint: PathBuf,
        /// The chips to bring the model up on, comma-separated: `vm`, `jit`, `c`, `rust`, `riscv-qemu`, `wasm`, `cuda`, `d5005`, `sn50`, `fabric`. Every one when absent.
        #[arg(long = "target", value_name = "CHIP", value_delimiter = ',')]
        targets: Vec<String>,
        /// The positions the decode step's caches hold.
        #[arg(long, value_name = "N", default_value_t = 4096)]
        ctx: usize,
        /// The directory the program and every backend's artifacts are written to.
        #[arg(long, short)]
        output: PathBuf,
        /// Then run the model on every chip that brought it up and is on this machine, each judged by how many of its
        /// tokens lead the reference's.
        #[arg(long)]
        run: bool,
        /// The tokens a run generates after the prompt (8, or the golden's continuation, unless given).
        #[arg(long, value_name = "N", requires = "run")]
        tokens: Option<usize>,
        /// The prompt a run starts from: token ids from 0, comma-separated.
        #[arg(long, value_name = "IDS", value_delimiter = ',', default_value = "5,17,2,30", requires = "run", conflicts_with = "golden")]
        prompt: Vec<u32>,
        /// Judge the runs against a golden (`benchmarks/models/<model>/golden-f32.json`): its first prompt is the
        /// prompt and its continuation the tokens to agree with; without it, the model's f64 reference generates them.
        #[arg(long, value_name = "FILE", requires = "run")]
        golden: Option<PathBuf>,
        /// Run the golden's first N prompts (`all`: every one), each from an empty state and judged against its own
        /// continuation.
        #[arg(long, value_name = "N|all", requires = "golden")]
        prompts: Option<String>,
    },

    /// Show what an LGX page became: the ordinary LOGOS its constructs lower to.
    ///
    /// A `## Component` is not run as written: it becomes a render function that writes
    /// the view to a tape (`vdOpen`/`vdText`/`vdClose`) its commit turns into DOM calls, an
    /// action function per action, a mount entry and a dispatcher; a `## Route` becomes a function plus the synthesized
    /// `route(method, path)` walk. When a page behaves in a way its markup does not
    /// explain, the lowering is the answer — this prints it.
    #[command(after_help = "Examples:\n  largo expand pages/app.lg")]
    Expand {
        /// The source file to expand.
        file: PathBuf,
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
    #[command(
        after_help = "Examples:\n  largo completions bash > ~/.local/share/bash-completion/completions/largo\n  largo completions zsh > ~/.zfunc/_largo\n  largo completions fish > ~/.config/fish/completions/largo.fish"
    )]
    Completions {
        /// The shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Re-execute a `.replay` failure artifact.
    ///
    /// A failing `largo run --interpret` writes a self-contained bundle under
    /// `target/replays/` — embedded source, scheduler seed and decision trace,
    /// input transcript, and the failure coordinate. Replaying it re-executes
    /// the program deterministically and reports whether the recorded failure
    /// still reproduces byte-identically, no longer fires (fixed), or diverged
    /// (the program or bundle changed since recording).
    ///
    /// Bundles are portable and committable: a bundle checked into a project's
    /// test corpus is a permanent reproduction of the bug it recorded.
    #[command(
        after_help = "Examples:\n  largo replay target/replays/main-1720000000.replay\n  largo replay bug.replay --verify"
    )]
    Replay {
        /// Path to the `.replay` bundle.
        file: PathBuf,

        /// Treat divergence as a hard error (exit non-zero). Reproduced and
        /// fixed are both coherent verdicts; divergence never is.
        #[arg(long)]
        verify: bool,
    },

    /// Explain a diagnostic code or a language construct, offline.
    ///
    /// Looks the topic up in the error catalog first (`largo explain
    /// zero-index` — every code from `check --json` and the MCP tools), then
    /// in the teach lessons (`largo explain Let`). No project or network
    /// needed.
    #[command(
        after_help = "Examples:\n  largo explain zero-index\n  largo explain use-after-move\n  largo explain Let\n  largo explain zero-index --json"
    )]
    Explain {
        /// An error code (kebab-case) or a construct name.
        topic: String,

        /// Print the entry as one `logos-explain/1` JSON document.
        #[arg(long)]
        json: bool,
    },

    /// Serve the LOGOS toolchain over the Model Context Protocol (stdio).
    ///
    /// Speaks JSON-RPC 2.0, one JSON object per line: check with structured
    /// diagnostics, sandboxed deterministic runs, English→FOL, kernel-certified
    /// proofs, code emission, and optimization reports — plus the quickguide
    /// and docs guides as MCP resources. Made for AI agents; humans want the
    /// plain subcommands.
    #[command(after_help = "Examples:\n  largo mcp\n  claude mcp add logos -- largo mcp")]
    Mcp {
        /// (internal) Run the `logos_run` sandbox worker on ENGINE instead of
        /// serving; source arrives on stdin. Spawned by the server itself.
        #[arg(long, hide = true, value_name = "ENGINE")]
        worker: Option<String>,

        /// (internal) Program arguments for the sandbox worker.
        #[arg(trailing_var_arg = true, hide = true)]
        worker_args: Vec<String>,
    },

    /// Run the project's `## Test` blocks.
    ///
    /// Discovers every `## Test: <prose name>` in the project, runs each in a
    /// fresh, isolated world on the interpreter tier, and prints a
    /// deterministic, name-ordered report. `Expect that` failures
    /// record-and-continue (one run reports everything); runaway tests fail at
    /// a deterministic op budget (the timeout that cannot flake); skips are
    /// loud, need a reason, and fail the run unless budgeted under
    /// `[test] skip_budget` in Largo.toml.
    #[command(
        after_help = "Examples:\n  largo test\n  largo test withdraw\n  largo test --seed 7\n  largo test --format json\n  largo test --json"
    )]
    Test {
        /// Run only tests whose prose name contains this substring.
        filter: Option<String>,

        /// The suite seed (default 0 — reproducibility is the default).
        #[arg(long, default_value_t = 0)]
        seed: u64,

        /// Report format.
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,

        /// Emit the machine-readable CLAIM INVENTORY as JSON: every `## Test`
        /// classified by kind (example / finite-enumeration property) with an
        /// honest status, vacuous flags, disclosed domain bounds, and any
        /// concrete counterexample. A superset of `--format json`.
        #[arg(long)]
        json: bool,

        /// Also run capability-granted tests (`This test may use the real
        /// filesystem.` etc.) — they leave the hermetic no-flake guarantee.
        #[arg(long)]
        live: bool,

        /// Escalate finite-domain `For all` properties to exhaustive proof
        /// (booleans are checked over every case, not sampled) → rank PROVEN.
        #[arg(long)]
        prove: bool,

        /// Cross-tier differential: `all` (or a list of `interpreter`,`vm`) also runs
        /// each passing test's computation on the bytecode VM and compares it byte for
        /// byte, failing if the compiled tier cannot run what the interpreter did (the
        /// F18 "works-on-my-tier" guard). Any other value is refused rather than ignored.
        #[arg(long)]
        tiers: Option<String>,

        /// A mounted test's page tiers also drive a real headless Chrome (`CHROME_BIN`, or the
        /// first of `google-chrome`/`google-chrome-stable`/`chromium`/`chromium-browser`/`chrome`
        /// on `PATH`) and must agree with it, exactly as they already must agree with the lowered
        /// bytecode VM. No Chrome found is a refused run, not a silent skip.
        #[arg(long)]
        browser: bool,
    },

    /// Debug a LOGOS program over the Debug Adapter Protocol (stdio).
    ///
    /// Speaks DAP with `Content-Length` framing: `launch`, breakpoints,
    /// `stackTrace`/`scopes`/`variables`, the stepping verbs, `evaluate`, and —
    /// the headline — real reverse execution (`stepBack`/`reverseContinue`,
    /// `supportsStepBack: true`). A thin serializer over the same `DebugSession`
    /// facade the Studio drawer renders, so an editor and the browser drawer
    /// see the same session. Made for editor integrations; humans want `largo repl`.
    #[command(after_help = "Examples:\n  largo dap")]
    Dap {},

    /// Lean 4 → LOGOS: import a Lean declaration as a checked LOGOS package.
    ///
    /// `largo lean import --project <dir> --target <Name> --out <dir>` compiles
    /// the project with the pinned Lean 4.14.0, exports the target with the
    /// pinned lean4export, checks it with the LOGOS kernel, rechecks it
    /// independently, holds its assumptions to policy, emits it as a LOGOS
    /// document and replays the package in a fresh process. stdout is one
    /// `logos-lean-result/1` document; stderr is NDJSON diagnostics.
    /// `largo lean replay <dir>` replays a published package.
    #[command(
        disable_help_flag = true,
        after_help = "Examples:\n  largo lean import --project ./proofs --target Demo.andSwap --out ./Demo.andSwap\n  largo lean import Demo.ndjson --target Demo.andSwap --out ./out\n  largo lean import Demo.ndjson --arena-protocol\n  largo lean replay ./Demo.andSwap\n  largo lean import --help"
    )]
    Lean {
        /// `import …` or `replay …` (see `largo lean import --help`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        args: Vec<std::ffi::OsString>,
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
        Commands::New { name, web, site } => {
            let scaffold = match (web, site) {
                (true, _) => commands::new::Scaffold::Web,
                (_, true) => commands::new::Scaffold::Site,
                _ => commands::new::Scaffold::Program,
            };
            commands::new::cmd_new(&name, scaffold)
        }
        Commands::Init { name } => commands::new::cmd_init(name.as_deref()),
        Commands::Build { release, verify, lib, target, native_functions, emit, json, manifest, r#static } => {
            if r#static {
                return commands::build::cmd_build_static();
            }
            commands::build::cmd_build(release, verify, lib, target, native_functions, emit, json, manifest)
        }
        Commands::Run { emit: Some(e), args, .. } if e == "wasm" => commands::run::cmd_run_wasm(&args, false),
        Commands::Run { emit: Some(e), args, .. } if e == "wasm-linked" => commands::run::cmd_run_wasm(&args, true),
        Commands::Run { emit: Some(e), release, args, max_steps, timeout, seed, json, .. } => {
            match crate::project::build::Backend::from_emit(&e) {
                Some(backend) => commands::run::cmd_run(
                    release,
                    backend,
                    &args,
                    max_steps,
                    timeout.as_deref(),
                    seed,
                    json,
                ),
                None => Err(format!("unknown --emit target '{e}' (expected 'wasm', 'wasm-linked', 'c', or 'c-bytecode')").into()),
            }
        }
        Commands::Run { interpret, args, max_steps, timeout, seed, json, engine, .. } if interpret => {
            commands::run::cmd_run_interpret(
                &args,
                max_steps,
                timeout.as_deref(),
                seed,
                json,
                engine
                    .map(Into::into)
                    .unwrap_or(logicaffeine_compile::ui_bridge::Engine::Auto),
            )
        }
        Commands::Run { release, args, max_steps, timeout, seed, json, .. } => {
            commands::run::cmd_run(
                release,
                crate::project::build::Backend::Rust,
                &args,
                max_steps,
                timeout.as_deref(),
                seed,
                json,
            )
        }
        Commands::Serve { timeout, seed, watch, open, port, host, as_backend, args } => {
            // The hidden child mode first: the full-stack parent spawned us to
            // run the entry's `Serve` on `LOGOS_PORT` (with a stdin lifeline).
            if as_backend {
                commands::serve::cmd_serve_backend(&args, timeout.as_deref(), seed)
            } else {
                commands::serve::cmd_serve_project(commands::serve::ServeOptions {
                    watch,
                    open,
                    port,
                    host: host.as_deref(),
                    timeout: timeout.as_deref(),
                    seed,
                    args: &args,
                })
            }
        }
        Commands::Check { file: Some(file), kernel: true, json, .. } => commands::kernel_document::run(&file, json, "Checked"),
        Commands::Check { deep, json, .. } => commands::check::cmd_check(deep, json),
        Commands::Opts {
            file,
            json,
            jit,
            args,
        } => commands::opts::cmd_opts(&file, json, jit, &args),
        Commands::Verify {} => commands::verify::cmd_verify(),
        Commands::Publish { registry, dry_run, allow_dirty } => {
            commands::publish::cmd_publish(registry.as_deref(), dry_run, allow_dirty)
        }
        Commands::Login { registry, token } => commands::publish::cmd_login(registry.as_deref(), token),
        Commands::Logout { registry } => commands::publish::cmd_logout(registry.as_deref()),
        Commands::Doctor { registry } => commands::doctor::cmd_doctor(registry),
        Commands::Repl { logic, format, load } => crate::repl::cmd_repl(logic, format, load),
        Commands::Sat { file, proof, stats } => commands::sat::cmd_sat(file, proof, stats),
        Commands::Prove { file: Some(file), kernel: true, json, .. } => commands::kernel_document::run(&file, json, "Proved"),
        Commands::Prove { file, trace, json, certificate, .. } => commands::prove::cmd_prove(file, trace, json, certificate),
        Commands::Logic { sentence, file, format, all_readings, pragmatic, discourse } => {
            commands::logic::cmd_logic(sentence, file, format, all_readings, pragmatic, discourse)
        }
        Commands::Doc { out } => commands::doc::cmd_doc(out),
        Commands::Add { spec, path, git } => commands::deps::cmd_add(spec, path, git),
        Commands::Remove { name } => commands::deps::cmd_remove(name),
        Commands::Fmt { paths, check, json } => commands::fmt::cmd_fmt(paths, check, json),
        Commands::Emit { target, file, output, chip, format, bridge, model, ctx, batch, tp, prefill } => commands::emit::cmd_emit(target, file, output, chip, format, bridge, model, ctx, batch, tp, prefill),
        Commands::Bringup { checkpoint, targets, ctx, output, run, tokens, prompt, golden, prompts } => {
            let run = run.then_some(commands::bringup::RunArgs { tokens, prompt, golden, prompts });
            commands::bringup::cmd_bringup(checkpoint, targets, ctx, output, run)
        }
        Commands::Expand { file } => commands::expand::cmd_expand(file),
        Commands::Clean { all } => commands::clean::cmd_clean(all),
        Commands::Completions { shell } => commands::completions::cmd_completions(shell),
        Commands::Replay { file, verify } => commands::replay::cmd_replay(file, verify),
        Commands::Explain { topic, json } => commands::explain::cmd_explain(&topic, json),
        Commands::Mcp { worker, worker_args } => crate::mcp::cmd_mcp(worker, worker_args),
        Commands::Test { filter, seed, format, json, live, prove, tiers, browser } => {
            commands::test::cmd_test(filter.as_deref(), seed, &format, json, live, prove, tiers.as_deref(), browser)
        }
        Commands::Dap {} => crate::dap::cmd_dap(),
        Commands::Lean { args } => commands::lean::cmd_lean(args),
    }
}
