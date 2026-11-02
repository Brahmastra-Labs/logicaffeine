//! `largo verify` — Z3 static verification (feature-gated: the full build carries Z3).

use crate::commands::require_project_root;
use crate::ui;

/// Handle `largo verify`: run verification against the current project.
pub(crate) fn cmd_verify() -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;

    run_verification(&project_root)?;
    // ★ SAY WHAT WAS VERIFIED. "Verification passed" over a command that had checked nothing is
    // how this went unnoticed; naming the subject means a reader can tell whether their `pages/`
    // were included (they are not — `largo check` compiles those).
    ui::info("Verification passed: the project entry and everything it imports");
    Ok(())
}

/// Run Z3 verification over the project entry.
#[cfg(feature = "verification")]
pub(crate) fn run_verification(project_root: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("Running Z3 verification...");
    verify_project_source(project_root)
}

/// Verify the project's assembled source — the work this command exists to do.
///
/// ★ THIS USED TO VERIFY NOTHING. It read the entry file into a variable it discarded, asked Z3
/// whether `true` is valid, and printed "Verification passed". A paying customer could run it on a
/// program with a provable violation and be told it was fine — a check that cannot fail, sold as a
/// feature, which is the worst shape a bug in this codebase takes.
///
/// The pass it should always have called already existed: `compile_to_rust_verified` runs escape
/// analysis, the Z3 static-verification pass and the type checker over real source. It is reached
/// through the SAME closure assembly `largo check` uses, so an `Import` resolves here exactly as it
/// does there and a multi-file project is verified whole rather than one file deep.
#[cfg(feature = "verification")]
pub(crate) fn verify_project_source(
    project_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::project::manifest::Manifest;

    let manifest = Manifest::load(project_root)?;
    let entry_path = crate::commands::resolve_entry_path(project_root, &manifest)?;
    let dep_roots = crate::project::deps::resolve_dep_roots(project_root, &manifest)?;
    let closure = crate::closure::assemble_closure_with_deps(&entry_path, dep_roots)
        .map_err(|e| crate::ui::CliError::new(format!("could not assemble project: {e}")))?;

    let rust =
        logicaffeine_compile::compile::compile_to_rust_verified(&closure.source).map_err(|e| {
            crate::ui::CliError::new(logicaffeine_language::socratic_explanation(
                &e,
                &logicaffeine_language::Interner::new(),
            ))
        })?;

    // ★ `Ok` PROVES NOTHING. A program this backend REFUSES still comes back as `Ok` — the refusal
    // rides in the returned Rust as a `compile_error!`, so a plain `?` here would rebuild the very
    // hole this function was written to close, one layer further in.
    if let Some(why) = embedded_refusal(&rust) {
        return Err(crate::ui::CliError::new(format!(
            "this program cannot be compiled, so it cannot be verified: {why}"
        ))
        .into());
    }
    Ok(())
}

/// The message inside an emitted `compile_error!("…")`, if the generated Rust carries one.
///
/// A refusal is not an error return — it is a well-formed Rust string whose compilation fails on
/// purpose. Anything reading `compile_to_rust*` as a pass/fail signal has to look for this.
#[cfg(feature = "verification")]
fn embedded_refusal(rust: &str) -> Option<String> {
    let at = rust.find("compile_error!")?;
    let rest = &rust[at..];
    let open = rest.find('"')?;
    let body = &rest[open + 1..];
    let end = body.find('"')?;
    Some(body[..end].to_string())
}

/// Stub for builds without the `verification` feature: explain how to get it.
#[cfg(not(feature = "verification"))]
pub(crate) fn run_verification(_project_root: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    Err(crate::ui::CliError::with_hint(
        "this largo build does not include Z3 verification",
        "install the full build: `curl -fsSL https://logicaffeine.com/install.sh | sh -s -- --full` \
         (or build from source with `cargo install logicaffeine-cli --features verification`)",
    )
    .into())
}

#[cfg(all(test, feature = "verification"))]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/verify/tests.rs"]
mod tests;
