//! LOGOS CLI (largo) - Standalone binary
//!
//! This is the entry point for the `largo` command-line tool.
//!
//! The binary is a thin wrapper around [`logicaffeine_cli::run_cli`],
//! handling error display and exit codes. All command logic is
//! implemented in the library crate for testability.
//!
//! # Exit Codes
//!
//! - `0` - Success
//! - `1` - Error (message printed to stderr)
//! - `2` - Usage error (clap argument errors)
//!
//! Commands may set other codes through
//! [`CliError::exit_code`](logicaffeine_cli::ui::CliError).

fn main() {
    // The copy-and-patch JIT becomes the process-wide native tier: every
    // interpreted program the CLI runs gets hot-function and hot-loop
    // tier-up. Only x86_64 System V (Linux + macOS) links it; on Windows,
    // aarch64 and WASM the tier is absent and largo runs the bytecode VM.
    // `LOGOS_NO_JIT=1` skips installation so programs run on the VM bytecode
    // interpreter alone — a fallback when the JIT miscompiles a hot loop, and
    // the knob the differential gate uses to compare AOT against the unjitted
    // reference engine.
    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    logicaffeine_jit::segv_trace_install();
    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    if std::env::var_os("LOGOS_NO_JIT").is_none() {
        logicaffeine_jit::install();
    }

    // Run the CLI on a worker whose stack is SIZED FROM the AST depth limit,
    // so `LOGOS_MAX_AST_DEPTH` genuinely works: raising the limit raises the
    // stack that honors it (the main thread's stack is fixed by `ulimit -s`
    // at exec and can't follow the knob). 40 KiB/level covers the fattest
    // measured walker frame with margin; the pages are virtual until
    // touched, so a large reservation costs nothing on shallow programs.
    let stack_bytes = logicaffeine_language::ast_depth::max_ast_depth()
        .saturating_mul(40 * 1024)
        .max(16 * 1024 * 1024);
    let outcome = std::thread::Builder::new()
        .name("largo".into())
        .stack_size(stack_bytes)
        .spawn(|| {
            // Errors render inside the worker (their type isn't Send);
            // structured CLI errors carry a hint and their own exit code,
            // anything else renders as a bare `error:` line and exits 1.
            match logicaffeine_cli::run_cli() {
                Ok(()) => 0,
                Err(e) => logicaffeine_cli::ui::render_error(e.as_ref()),
            }
        })
        .expect("spawn the largo worker")
        .join();

    match outcome {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        // A panicked worker already printed its panic message.
        Err(_) => std::process::exit(101),
    }
}
