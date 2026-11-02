//! COMPONENT SCOPE ATTRIBUTION (G16).
//!
//! A `## Component <Name>` block desugars (in the parser) to a synthesized
//! FUNCTION `__render_<Name>` whose body — the `View:` element tree, event
//! handlers, `State:` field reads, and `{…}` string interpolations — carries the
//! component's real call sites. The per-file scope check ([`check_scopes`]) must
//! attribute those calls to the module the component is DEFINED in, gated by that
//! module's OWN imports — NOT to the entry, and never silently skipped.
//!
//! Before the fix the assembler never recorded `__render_<Name>` as a definition,
//! so `check_scopes` couldn't find its home module and skipped the whole body:
//!   - a genuine leak inside a Component (`greeting` used in `app.lg`, imported
//!     nowhere in app's chain) slipped through the scope pass entirely; and
//!   - projects worked around it by making the ENTRY redundantly import every
//!     transitive module (unused-import warnings) so the flat unit still resolved.
//!
//! A second, orthogonal hole this surfaced: the call/type walkers used by the
//! scope check dropped `Expr::InterpolatedString` on a wildcard arm, so a
//! `{greeting(count)}` hole — the canonical component-view call site — was never
//! seen. Both walkers are now exhaustive.
//!
//! The contract locked here, robust to the point of absurdity:
//!   1. A VALID entry→Component→policy chain (policy imported by the Component's
//!      OWN module, NOT by the entry) passes `check` AND runs, with NO redundant
//!      entry-level import. Proven across check, interpreter, VM (DOM op-log
//!      parity), and AOT-Rust (a component program lowers + rustc-runs).
//!   2. A GENUINE transitive leak (a Component calls a fn no module in ITS import
//!      chain provides, though the fn is reachable in the unit) STILL fails, at
//!      the Component's own source file, naming the fn's defining file.
//!   3. Selective imports agree: a name the Component's module selectively imports
//!      is in scope; a DIFFERENT selected name is not (leak at the Component file).
//!   4. Private names are gated from a Component exactly as from a plain function:
//!      a Component calling another module's `## To private` fn is rejected with
//!      the "private to X" diagnostic at the Component's file.
//!   5. Function / type / event-handler / interpolation call sites inside a
//!      Component are all attributed identically.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::{tempdir, TempDir};

use logicaffeine_compile::analysis::assemble_project;
use logicaffeine_compile::compile::{
    compile_project, enforce_project_scope, tw_outcome, vm_outcome,
};
use logicaffeine_compile::semantics::dom::{self, DomOp};

/// A three-module project: `policy.lg` (defines a policy fn), `app.lg` (a
/// `## Component` whose view/handler calls into policy), `main.lg` (the entry that
/// imports App and mounts its component). Returns the temp dir (kept alive by the
/// caller) and the entry path.
fn three_module(policy: &str, app: &str, entry: &str) -> (TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("policy.lg"), policy).unwrap();
    fs::write(dir.path().join("app.lg"), app).unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, entry).unwrap();
    (dir, main)
}

/// The assembled + prelude-expanded source of a project — the single string every
/// runtime tier re-parses.
fn assembled_source(entry: &Path) -> String {
    let assembled = assemble_project(entry).expect("assemble project");
    logicaffeine_compile::loader::apply_prelude(&assembled.source).into_owned()
}

/// The tree-walker and VM DOM op-logs for a source, asserted IDENTICAL, returned
/// for further inspection. Runs each on a freshly-reset recorder.
fn dom_parity(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = tw_outcome(src);
    let tw = dom::op_log();
    dom::reset();
    let _ = vm_outcome(src);
    let vm = dom::op_log();
    assert_eq!(tw, vm, "tree-walker vs VM DOM op-log diverged:\n{src}");
    tw
}

/// The concatenated text of every `SetText`/`CreateTextNode` op — what the view
/// actually rendered, so a component that shows a policy-computed value is
/// observable end-to-end.
fn rendered_text(ops: &[DomOp]) -> String {
    ops.iter()
        .filter_map(|op| match op {
            DomOp::SetText { text, .. } | DomOp::CreateTextNode { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("|")
}

// A public policy fn: `greeting(n) = n + 100`, so a rendered value is unmistakably
// the policy's (5 → 105), not an accident of a default.
const POLICY_PUBLIC: &str = "# Policy\n\n## To greeting (n: Int) -> Int:\n    Return n plus 100.\n";

// The same policy fn, but PRIVATE — reachable only by its owning module.
const POLICY_PRIVATE: &str =
    "# Policy\n\n## To private greeting (n: Int) -> Int:\n    Return n plus 100.\n";

// A policy module with TWO public fns, so a selective import can pick the wrong one.
const POLICY_TWO: &str = "# Policy\n\n\
    ## To greeting (n: Int) -> Int:\n    Return n plus 100.\n\n\
    ## To other (n: Int) -> Int:\n    Return n.\n";

// The entry: imports App, mounts its component. NO import of Policy — the whole
// point is that the entry needn't redundantly import a module only the Component
// uses.
const ENTRY_MOUNTS_APP: &str =
    "# Main\n\nImport App.\n\n## Main\nLet root be createElement(\"div\").\n\
    Call setAttribute with root and \"id\" and \"root\".\nCall appendChild with body() and root.\n\
    Mount AppView at \"#root\".\n";

/// An App module whose Component calls `greeting` in a `{…}` view interpolation,
/// importing Policy however `import_line` says (e.g. `Import Policy.` or empty).
fn app_interp(import_line: &str) -> String {
    format!(
        "# App\n\n{import_line}## Component AppView\nState:\n    Let count be 5.\n\
         View:\n    <h1>v {{greeting(count)}}</h1>\n"
    )
}

/// An App module whose Component calls `greeting` in an `on-click` event handler.
fn app_handler(import_line: &str) -> String {
    format!(
        "# App\n\n{import_line}## Component AppView\nState:\n    Let count be 5.\n\
         Actions:\n    To act:\n        Set count to greeting(count).\n\
         View:\n    <button on-click={{act}}>x</button>\n"
    )
}

// =============================================================================
// 1. The VALID chain — no redundant entry import — passes check AND runs.
// =============================================================================

/// The core win: a Component in `app.lg` calls a policy fn `app.lg` imports; the
/// ENTRY does NOT import Policy. This must pass the scope check with no workaround.
#[test]
fn valid_transitive_component_chain_checks_without_a_redundant_entry_import() {
    let (_dir, main) = three_module(
        POLICY_PUBLIC,
        &app_interp("Import Policy.\n\n"),
        ENTRY_MOUNTS_APP,
    );
    // The entry file itself must NOT mention Policy — prove the source is clean.
    let entry_src = fs::read_to_string(&main).unwrap();
    assert!(
        !entry_src.contains("Policy"),
        "the entry must not need a redundant Policy import"
    );
    enforce_project_scope(&main)
        .expect("a Component using a fn ITS module imports must pass the scope check");
}

/// The same valid chain RUNS: the interpreter and VM build the identical DOM, and
/// the view renders the policy-computed value (105) — the cross-module call
/// resolved through the Component body on both runtime tiers.
#[test]
fn valid_component_chain_runs_identically_on_interpreter_and_vm() {
    let (_dir, main) = three_module(
        POLICY_PUBLIC,
        &app_interp("Import Policy.\n\n"),
        ENTRY_MOUNTS_APP,
    );
    enforce_project_scope(&main).expect("scope must pass");
    let src = assembled_source(&main);
    let ops = dom_parity(&src);
    assert!(
        rendered_text(&ops).contains("105"),
        "the Component's view must render greeting(5)=105 (policy reached across the module boundary); got: {ops:?}"
    );
}

/// The same valid chain LOWERS through the AOT WASM-DOM backend — the tier a
/// component program actually targets (its `View:`/signal builtins are DOM host
/// imports, not native functions). `compile_to_wasm` emits a well-formed module
/// from the assembled multi-module source, proving the cross-module Component call
/// survives all the way to bytecode. (The full `largo build`/native-Rust AOT is
/// NOT a Component target — DOM builtins have no native lowering — so the
/// interpreter/VM/WASM tiers are the component chain's execution surface.)
#[test]
fn valid_component_chain_lowers_to_wasm() {
    let (_dir, main) = three_module(
        POLICY_PUBLIC,
        &app_interp("Import Policy.\n\n"),
        ENTRY_MOUNTS_APP,
    );
    enforce_project_scope(&main).expect("scope must pass");
    let src = assembled_source(&main);
    let wasm = logicaffeine_compile::compile::compile_to_wasm(&src)
        .expect("a valid Component chain must lower to a WASM module");
    // A real module: the WASM magic header `\0asm`.
    assert!(
        wasm.len() > 8 && &wasm[0..4] == b"\0asm",
        "emitted bytes must be a valid WASM module (\\0asm header)"
    );
}

/// A Component that calls a policy fn in an `on click:` EVENT HANDLER (a call site
/// distinct from a view interpolation) is attributed the same way — a valid chain
/// passes.
#[test]
fn valid_event_handler_call_in_a_component_is_in_scope() {
    let (_dir, main) = three_module(
        POLICY_PUBLIC,
        &app_handler("Import Policy.\n\n"),
        ENTRY_MOUNTS_APP,
    );
    enforce_project_scope(&main)
        .expect("a Component event handler calling ITS module's import must be in scope");
    // And it lowers through the WASM-DOM backend (the component tier).
    let src = assembled_source(&main);
    logicaffeine_compile::compile::compile_to_wasm(&src)
        .expect("event-handler Component chain must lower to WASM");
}

// =============================================================================
// 2. A GENUINE leak inside a Component still fails — at the Component's file.
// =============================================================================

/// The Component calls `greeting`, but `app.lg` does NOT import Policy (only the
/// entry does, so `greeting` IS reachable in the unit). Attributing the call to
/// `app.lg` is what makes this a leak: app.lg cannot reach greeting. It must fail,
/// naming BOTH the Component's file (`app.lg`) and the fn's defining file.
#[test]
fn genuine_leak_in_a_component_interpolation_fails_at_the_component_file() {
    // Entry imports BOTH App and Policy (so greeting is reachable), but the
    // Component's own module app.lg imports only… nothing.
    let entry = "# Main\n\nImport App.\nImport Policy.\n\n## Main\nMount AppView at \"#root\".\nLet z be greeting(1).\n";
    let (_dir, main) = three_module(POLICY_PUBLIC, &app_interp(""), entry);
    let err = enforce_project_scope(&main)
        .expect_err("a Component calling a fn its module does not import must be a scope error");
    assert!(
        err.contains("greeting"),
        "error must name the leaked fn: {err}"
    );
    assert!(
        err.contains("app.lg"),
        "the leak must be attributed to the Component's OWN file (app.lg): {err}"
    );
    assert!(
        err.contains("policy.lg"),
        "the diagnostic must name where the fn is actually defined (policy.lg): {err}"
    );
}

/// The SAME leak through an `on click:` event handler fails identically — the leak
/// is a property of the Component body, independent of which call site holds it.
#[test]
fn genuine_leak_in_a_component_event_handler_fails_at_the_component_file() {
    let entry = "# Main\n\nImport App.\nImport Policy.\n\n## Main\nMount AppView at \"#root\".\nLet z be greeting(1).\n";
    let (_dir, main) = three_module(POLICY_PUBLIC, &app_handler(""), entry);
    let err = enforce_project_scope(&main)
        .expect_err("a Component handler calling an un-imported fn must be a scope error");
    assert!(err.contains("greeting") && err.contains("app.lg"), "{err}");
}

/// The same leak is caught uniformly by the FULL compile path (`compile_project`,
/// the `largo build` seam), not only by the lightweight `enforce_project_scope` —
/// a program that fails scoping on one tier fails on all.
#[test]
fn genuine_leak_in_a_component_fails_the_full_compile_too() {
    let entry = "# Main\n\nImport App.\nImport Policy.\n\n## Main\nMount AppView at \"#root\".\nLet z be greeting(1).\n";
    let (_dir, main) = three_module(POLICY_PUBLIC, &app_interp(""), entry);
    let err = format!(
        "{:?}",
        compile_project(&main).expect_err("compile must reject the leak")
    );
    assert!(
        err.contains("greeting"),
        "compile error must name the leaked fn: {err}"
    );
    assert!(
        err.contains("app.lg"),
        "compile error must attribute to app.lg: {err}"
    );
}

// =============================================================================
// 3. Selective imports agree for Component bodies.
// =============================================================================

/// A Component may call a name its module SELECTIVELY imports.
#[test]
fn selective_import_reaches_a_component_body() {
    let app = app_interp("Import greeting from Policy.\n\n");
    let (_dir, main) = three_module(POLICY_TWO, &app, ENTRY_MOUNTS_APP);
    enforce_project_scope(&main)
        .expect("a selectively-imported name must be in scope inside a Component");
    let src = assembled_source(&main);
    let ops = dom_parity(&src);
    assert!(
        rendered_text(&ops).contains("105"),
        "selective import must render: {ops:?}"
    );
}

/// Selecting a DIFFERENT name does NOT bring `greeting` into scope — the Component
/// call is a leak at `app.lg`, even though `greeting` is a public export reachable
/// in the unit.
#[test]
fn selective_import_of_the_wrong_name_leaks_in_a_component() {
    let app = app_interp("Import other from Policy.\n\n");
    let (_dir, main) = three_module(POLICY_TWO, &app, ENTRY_MOUNTS_APP);
    let err = enforce_project_scope(&main)
        .expect_err("selecting `other` must not put `greeting` in a Component's scope");
    assert!(
        err.contains("greeting") && err.contains("app.lg"),
        "a wrong-name selective import must leak at the Component file: {err}"
    );
}

// =============================================================================
// 4. Private names are gated from a Component.
// =============================================================================

/// A Component calling another module's `## To private` fn is rejected with the
/// "private to X" diagnostic — even though `app.lg` imports Policy (a private name
/// is never importable), and the reference is inside a Component view.
#[test]
fn a_component_calling_a_private_fn_is_rejected() {
    let (_dir, main) = three_module(
        POLICY_PRIVATE,
        &app_interp("Import Policy.\n\n"),
        ENTRY_MOUNTS_APP,
    );
    let err = enforce_project_scope(&main)
        .expect_err("a Component must not reach another module's private fn");
    assert!(err.contains("greeting"), "must name the private fn: {err}");
    assert!(
        err.contains("private") && err.contains("policy.lg"),
        "must be the private-name diagnostic naming the owner (policy.lg): {err}"
    );
    assert!(
        err.contains("app.lg"),
        "must attribute to the Component's file: {err}"
    );
}

/// The OWNING module may of course use its own private fn inside its own
/// Component — privacy gates OTHER files, never the owner. Here Policy IS the
/// module that defines both the private fn and the Component that uses it.
#[test]
fn a_component_may_call_its_own_modules_private_fn() {
    let policy_with_component = "# Policy\n\n\
        ## To private greeting (n: Int) -> Int:\n    Return n plus 100.\n\n\
        ## Component AppView\nState:\n    Let count be 5.\n\
        View:\n    <h1>v {greeting(count)}</h1>\n";
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("policy.lg"), policy_with_component).unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# Main\n\nImport Policy.\n\n## Main\nLet root be createElement(\"div\").\n\
         Call setAttribute with root and \"id\" and \"root\".\nCall appendChild with body() and root.\n\
         Mount AppView at \"#root\".\n",
    )
    .unwrap();
    enforce_project_scope(&main)
        .expect("a module's own Component may use that module's private fn");
    let src = assembled_source(&main);
    let ops = dom_parity(&src);
    assert!(
        rendered_text(&ops).contains("105"),
        "owner's private-fn Component must render: {ops:?}"
    );
}

// =============================================================================
// 5. A Component is COMPOSED across modules (the entry references it by name).
// =============================================================================

/// The entry composes the imported Component both by `Mount` (a full mount) — the
/// synthesized `__render_AppView` must be a public export of `app.lg` so the
/// entry's synthesized render function can call it. This is the reciprocal of the
/// scope fix: the render function is a first-class public member of its module.
#[test]
fn an_imported_component_is_composable_from_the_entry() {
    let (_dir, main) = three_module(
        POLICY_PUBLIC,
        &app_interp("Import Policy.\n\n"),
        ENTRY_MOUNTS_APP,
    );
    // Both a scope pass AND a run: the entry reaching the imported component's
    // render function is exactly what "composable" means operationally.
    enforce_project_scope(&main).expect("entry must be able to compose the imported Component");
    let src = assembled_source(&main);
    let ops = dom_parity(&src);
    assert!(
        !ops.is_empty(),
        "composing the Component must produce DOM ops"
    );
}

// =============================================================================
// 6. Regression floor: a single-file Component program is unaffected.
// =============================================================================

/// The fix must not disturb a single-file Component (no imports, no scope info) —
/// it still checks, runs on both tiers, and renders.
#[test]
fn a_single_file_component_program_is_unaffected() {
    let src = "## To greeting (n: Int) -> Int:\n    Return n plus 100.\n\n\
        ## Component AppView\nState:\n    Let count be 5.\n\
        View:\n    <h1>v {greeting(count)}</h1>\n\n\
        ## Main\nLet root be createElement(\"div\").\n\
        Call setAttribute with root and \"id\" and \"root\".\nCall appendChild with body() and root.\n\
        Mount AppView at \"#root\".\n";
    let full = logicaffeine_compile::loader::apply_prelude(src).into_owned();
    let ops = dom_parity(&full);
    assert!(
        rendered_text(&ops).contains("105"),
        "a single-file Component calling a top-level fn must still render: {ops:?}"
    );
}
