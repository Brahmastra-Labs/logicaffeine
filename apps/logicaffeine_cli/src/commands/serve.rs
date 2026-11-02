//! `largo serve` — run what the project is, over HTTP.
//!
//! What stands up is the project's [`ServePlan`], read from its one
//! [`ProjectShape`](crate::project::shape::ProjectShape):
//!
//! - a **site** or a **full-stack site** → the pages dev server
//!   ([`crate::commands::serve_web`]): largo owns the public port and serves the built pages and
//!   `static/` from memory; a full-stack site's backend runs as a child process and answers what
//!   the pages do not, while a site answers those requests `404` exactly as a static host would;
//! - a **backend** → the interpreter runs the entry, whose `Serve` binds the port itself
//!   ([`cmd_serve`]);
//! - a **program** → its WebAssembly bundle when it builds to one (or `--watch` asks for the
//!   live-reload loop), else the interpreter runs it.

use crate::project::shape::{Project, ServePlan};

/// The options `largo serve` was run with.
pub(crate) struct ServeOptions<'a> {
    pub watch: bool,
    pub open: bool,
    pub port: Option<u16>,
    pub host: Option<&'a str>,
    pub timeout: Option<&'a str>,
    pub seed: Option<u64>,
    pub args: &'a [String],
}

/// Handle `largo serve`: stand up the project's serve plan.
///
/// `--watch` decides the plan from [`Project::shape_or_best_effort`]: a page mid-typo, or a fresh
/// scaffold nobody has finished writing, must still bind a port and show the located error, never
/// refuse to start. Without `--watch`, [`Project::shape`] stays strict — a project whose shape
/// cannot be decided is refused with that same located error, never a vague one.
pub(crate) fn cmd_serve_project(opts: ServeOptions<'_>) -> Result<(), Box<dyn std::error::Error>> {
    let project = Project::load(&crate::commands::require_project_root()?)?;
    let port = opts.port.unwrap_or(8080);
    let plan = if opts.watch {
        project.shape_or_best_effort().serve_plan()
    } else {
        project.shape()?.serve_plan()
    };
    match plan {
        ServePlan::Pages { proxy } => {
            crate::commands::serve_web::cmd_serve_web(&project, port, opts.open, opts.host, proxy)
        }
        ServePlan::Backend => cmd_serve(opts.args, opts.timeout, opts.seed),
        ServePlan::Program => {
            let entry = project.require_entry()?;
            let dep_roots =
                crate::project::deps::resolve_dep_roots(&project.root, &project.manifest)?;
            let source = crate::closure::assemble_closure_with_deps(entry, dep_roots)
                .map_err(|e| format!("cannot serve — could not assemble the project: {e}"))?
                .source;
            if opts.watch || logicaffeine_compile::compile::compile_to_wasm(&source).is_ok() {
                crate::commands::serve_dom::cmd_serve_dom(&project, port, opts.watch, opts.open)
            } else {
                cmd_serve(opts.args, opts.timeout, opts.seed)
            }
        }
    }
}

/// Handle `largo serve`: interpret the project entry, which serves HTTP.
pub(crate) fn cmd_serve(
    args: &[String],
    timeout: Option<&str>,
    seed: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    // The program's `Serve` statement binds the port and blocks handling
    // requests; the interpreter drives its tokio accept loop inline. No fuel cap
    // (a server runs until stopped), no JSON envelope.
    crate::commands::run::cmd_run_interpret(
        args,
        None,
        timeout,
        seed,
        false,
        logicaffeine_compile::ui_bridge::Engine::Auto,
    )
}

/// Handle the hidden `largo serve --as-backend`: the full-stack dev server's
/// child process. Identical to [`cmd_serve`] (the `Serve` statement binds the
/// port `LOGOS_PORT` names), plus a stdin LIFELINE: the parent holds our
/// stdin pipe open and never writes; EOF means the parent died, so we exit —
/// an orphaned backend can never squat the port after the dev server is
/// killed.
pub(crate) fn cmd_serve_backend(
    args: &[String],
    timeout: Option<&str>,
    seed: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    std::thread::spawn(|| {
        use std::io::Read;
        let mut sink = [0u8; 64];
        let mut stdin = std::io::stdin();
        loop {
            match stdin.read(&mut sink) {
                Ok(0) | Err(_) => std::process::exit(0),
                Ok(_) => {}
            }
        }
    });
    cmd_serve(args, timeout, seed)
}
