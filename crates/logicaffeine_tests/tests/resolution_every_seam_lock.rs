//! A program means one thing on every path that runs or emits it. Before any tier sees a program,
//! `resolve_program` rewrites it: a division in a Rational context becomes exact, and a loop driven
//! by an infinitesimal clock becomes the statements that compute its end state. A seam that parses
//! a program and skips that step runs a different program: `Let x: Rational be 7 / 2.` prints `3`
//! there and `7/2` everywhere else, and a supertask never ends.
//!
//! The census reads the compiler's source with `syn`: every function that parses a program with
//! `parse_program()` also calls `resolve_program`, or is named below as a seam that only analyzes
//! what it parsed. The behavioral tests run a Rational division and a supertask through each
//! public entry point that executes or emits a program.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use syn::visit::Visit;

/// Seams that parse a program only to analyze it. Each runs and emits nothing, so there is no
/// resolved program for it to agree with.
const ANALYSIS_ONLY: &[(&str, &str)] = &[
    (
        "crates/logicaffeine_compile/src/compile.rs::classify_source",
        "classifies the program's determinacy",
    ),
    (
        "crates/logicaffeine_compile/src/compile.rs::first_parallel_block_independent",
        "decides whether the branches of the first parallel block are independent",
    ),
    (
        "crates/logicaffeine_compile/src/compile.rs::send_check_source",
        "checks what the program sends between tasks",
    ),
    (
        "crates/logicaffeine_compile/src/compile.rs::enforce_project_scope_with_deps",
        "checks that each file reaches only the names it defines or imports",
    ),
    (
        "crates/logicaffeine_compile/src/compile.rs::verify_no_overhead_source",
        "looks for interpreter dispatch left in a projection's residual program",
    ),
    (
        "crates/logicaffeine_compile/src/compile.rs::count_dispatch",
        "counts interpreter dispatch left in a projection's residual program",
    ),
    (
        "crates/logicaffeine_compile/src/optimize/effects.rs::analyze_source",
        "reports what each function's calls can do",
    ),
    (
        "crates/logicaffeine_compile/src/optimize/bta.rs::analyze_source",
        "reports the binding time of each expression",
    ),
    (
        "crates/logicaffeine_compile/src/ui_bridge.rs::parse_theorem",
        "reads a Logic-mode theorem block",
    ),
    (
        "crates/logicaffeine_compile/src/ui_bridge.rs::compile_theory_for_ui",
        "reads a Logic-mode theory",
    ),
    (
        "crates/logicaffeine_compile/src/ui_bridge.rs::theorem_problem",
        "reads a Logic-mode theorem block",
    ),
    (
        "crates/logicaffeine_lsp/src/pipeline.rs::try_full_parse",
        "the language server's diagnostics",
    ),
    (
        "crates/logicaffeine_lsp/src/pipeline.rs::try_parse_block",
        "the language server's diagnostics",
    ),
];

/// Seams that encode a program as data for the self-interpreter. Core, the language it
/// interprets, is untyped: an encoded `Let` drops its declared type and both divisions encode as
/// Core's one `/`, so there is no Rational context to resolve, and a supertask has no Core form.
const CORE_ENCODINGS: &[(&str, &str)] = &[
    (
        "crates/logicaffeine_compile/src/compile.rs::encode_program_source",
        "encodes the program as Core for the self-interpreter",
    ),
    (
        "crates/logicaffeine_compile/src/compile.rs::encode_program_source_compact",
        "encodes the program as compact Core for the self-interpreter",
    ),
    (
        "crates/logicaffeine_compile/src/compile.rs::program_to_core",
        "builds the program's Core value for the self-interpreter",
    ),
];

/// Seams known to resolve: the census must find each of them, or it is not reading the source.
const KNOWN_RESOLVING: &[&str] = &[
    "crates/logicaffeine_compile/src/ui_bridge.rs::parse_seam_with",
    "crates/logicaffeine_compile/src/ui_bridge.rs::with_optimized_program_tiered_scoped",
    "crates/logicaffeine_compile/src/compile.rs::compile_program_full_with_proven",
    "crates/logicaffeine_compile/src/compile.rs::with_optimized_program",
    "crates/logicaffeine_compile/src/compile.rs::compile_to_c_tiered",
    "crates/logicaffeine_compile/src/compile.rs::rustc_check_artifacts",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the tests crate sits two levels below the workspace root")
        .to_path_buf()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|e| e.expect("directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// One function that parses a program, and whether the same function resolves it.
#[derive(Debug)]
struct ParseSite {
    seam: String,
    resolves: bool,
}

struct Frame {
    name: String,
    parses: bool,
    resolves: bool,
}

struct Census<'f> {
    file: &'f str,
    frames: Vec<Frame>,
    sites: Vec<ParseSite>,
}

impl Census<'_> {
    fn enter(&mut self, name: String) {
        self.frames.push(Frame {
            name,
            parses: false,
            resolves: false,
        });
    }

    fn leave(&mut self) {
        let frame = self.frames.pop().expect("a frame per function");
        if frame.parses {
            self.sites.push(ParseSite {
                seam: format!("{}::{}", self.file, frame.name),
                resolves: frame.resolves,
            });
        }
    }
}

impl<'ast> Visit<'ast> for Census<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.enter(f.sig.ident.to_string());
        syn::visit::visit_item_fn(self, f);
        self.leave();
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.enter(f.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, f);
        self.leave();
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "parse_program" && call.args.is_empty() {
            if let Some(frame) = self.frames.last_mut() {
                frame.parses = true;
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            if path.path.segments.last().is_some_and(|s| s.ident == "resolve_program") {
                if let Some(frame) = self.frames.last_mut() {
                    frame.resolves = true;
                }
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

/// Every function in the compiler, the apps and the language server that parses a program.
fn parse_sites() -> Vec<ParseSite> {
    let root = workspace_root();
    let mut files = Vec::new();
    for dir in [
        "crates/logicaffeine_compile/src",
        "crates/logicaffeine_lsp/src",
        "apps/logicaffeine_cli/src",
        "apps/logicaffeine_web/src",
    ] {
        rust_sources(&root.join(dir), &mut files);
    }
    let mut sites = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        if !text.contains("parse_program") {
            continue;
        }
        let ast = syn::parse_file(&text)
            .unwrap_or_else(|e| panic!("{} does not parse as Rust: {e}", path.display()));
        let rel = path
            .strip_prefix(&root)
            .expect("under the workspace root")
            .to_string_lossy()
            .replace('\\', "/");
        let mut census = Census {
            file: &rel,
            frames: Vec::new(),
            sites: Vec::new(),
        };
        census.visit_file(&ast);
        sites.extend(census.sites);
    }
    sites
}

fn exempt() -> BTreeSet<&'static str> {
    ANALYSIS_ONLY
        .iter()
        .chain(CORE_ENCODINGS)
        .map(|(seam, _)| *seam)
        .collect()
}

#[test]
fn every_seam_that_parses_a_program_resolves_it() {
    let exempt = exempt();
    let unresolved: Vec<String> = parse_sites()
        .into_iter()
        .filter(|s| !s.resolves && !exempt.contains(s.seam.as_str()))
        .map(|s| s.seam)
        .collect();
    assert!(
        unresolved.is_empty(),
        "these functions parse a program and never call `resolve_program`, so they run or emit a \
         program other than the one every tier runs (a Rational division floors, a supertask never \
         ends). Call `crate::resolve_program::resolve_program` after the parse, as \
         `compile_program_full_with_proven` does; a seam that only analyzes what it parsed belongs \
         in ANALYSIS_ONLY with what it does:\n  {}",
        unresolved.join("\n  ")
    );
}

#[test]
fn every_exemption_names_a_seam_that_parses_and_does_not_resolve() {
    let sites = parse_sites();
    for seam in exempt() {
        let matching: Vec<&ParseSite> = sites.iter().filter(|s| s.seam == seam).collect();
        assert!(
            !matching.is_empty(),
            "`{seam}` is exempt, but no function by that name parses a program any more; remove it"
        );
        assert!(
            matching.iter().all(|s| !s.resolves),
            "`{seam}` is exempt but now resolves what it parses; remove the exemption"
        );
    }
}

#[test]
fn the_census_finds_the_seams_known_to_resolve() {
    let sites = parse_sites();
    for seam in KNOWN_RESOLVING {
        assert!(
            sites.iter().any(|s| s.seam == *seam && s.resolves),
            "the census did not find `{seam}` parsing and resolving a program; it is not reading \
             the compiler's source. Found:\n{sites:#?}"
        );
    }
}

#[test]
fn the_census_attributes_a_parse_to_the_innermost_function() {
    let source = r#"
        fn outer() {
            fn helper() { let _ = 1; }
            let stmts = parser.parse_program();
            resolve_program(&stmts);
        }
        fn analysis() { let _ = parser.parse_program(); }
        impl Seam { fn method(&self) { let _ = self.p.parse_program(); } }
    "#;
    let ast = syn::parse_file(source).expect("the sample parses");
    let mut census = Census {
        file: "sample.rs",
        frames: Vec::new(),
        sites: Vec::new(),
    };
    census.visit_file(&ast);
    let found: Vec<(&str, bool)> = census
        .sites
        .iter()
        .map(|s| (s.seam.as_str(), s.resolves))
        .collect();
    assert_eq!(
        found,
        vec![
            ("sample.rs::outer", true),
            ("sample.rs::analysis", false),
            ("sample.rs::method", false),
        ]
    );
}

// ── the programs ──────────────────────────────────────────────────────────────────────────────

const RATIONAL: &str = "## Main\n\
Let x: Rational be 7 / 2.\n\
Show x.\n\
Let y be 7 / 2.\n\
Show y.\n";

const RATIONAL_SHOWN: &str = "7/2\n3";

const LAMP: &str = "## Main\n\
Let dt be an infinitesimal whose reciprocal is even.\n\
Let t be 0.\n\
Let lamp be 0.\n\
While t < 1:\n\
\x20   Set lamp to 1 - lamp.\n\
\x20   Set t to t + dt.\n\
Show lamp.\n";

const LAMP_SHOWN: &str = "0";

/// `Sleep` puts a program on the asynchronous interpreter.
fn asynchronous(program: &str) -> String {
    program.replacen("## Main\n", "## Main\nSleep 1.\n", 1)
}

fn trimmed(lines: &[String]) -> String {
    lines.join("\n").trim().to_string()
}

/// `run` on a thread of its own: a program that has not finished within a minute fails the test
/// instead of hanging it. A supertask that reaches a tier unresolved runs forever.
fn within_a_minute<T: Send + 'static>(what: &str, run: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)));
        })
        .expect("spawn");
    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(Ok(value)) => value,
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(_) => panic!("{what} did not finish within a minute: the program reached it unresolved"),
    }
}

// ── the interpreters ──────────────────────────────────────────────────────────────────────────

fn studio(program: &str) -> (String, Option<String>) {
    let program = program.to_string();
    within_a_minute("the Studio's streaming interpreter", move || {
        let sink = std::rc::Rc::new(std::cell::RefCell::new(|_line: String| {}));
        let result =
            futures::executor::block_on(logicaffeine_compile::interpret_streaming(&program, sink));
        (trimmed(&result.lines), result.error)
    })
}

fn interpreter(program: &str) -> (String, Option<String>) {
    let program = program.to_string();
    within_a_minute("the interpreter", move || {
        let result = futures::executor::block_on(logicaffeine_compile::interpret_for_ui(&program));
        (trimmed(&result.lines), result.error)
    })
}

fn vm_io(program: &str) -> (String, Option<String>) {
    let program = program.to_string();
    within_a_minute("the VM's IO runner", move || {
        let result = futures::executor::block_on(logicaffeine_compile::run_vm_io_async(&program));
        (trimmed(&result.lines), result.error)
    })
}

fn v2_optimized(program: &str) -> (String, Option<String>) {
    let program = program.to_string();
    within_a_minute("the v2-optimized program on the VM", move || {
        logicaffeine_compile::ui_bridge::with_v2_optimized_program(&program, |parsed, interner| {
            match parsed {
                Ok((stmts, types, policies)) => {
                    let (out, err) = logicaffeine_compile::vm::run_to_outcome_with_args(
                        stmts,
                        interner,
                        Some(types),
                        Some(&policies),
                        &[],
                        None,
                    );
                    (out.trim().to_string(), err)
                }
                Err(advice) => (String::new(), Some(advice)),
            }
        })
    })
}

fn first_projection(program: &str) -> String {
    logicaffeine_compile::compile::projection1_source("", "", program)
        .unwrap_or_else(|e| panic!("the first projection refused:\n{program}\n{e}"))
}

fn assert_shows(what: &str, (out, err): (String, Option<String>), expected: &str) {
    assert_eq!(err, None, "{what} failed");
    assert_eq!(out, expected, "{what} printed the wrong thing");
}

#[test]
fn the_studio_divides_a_rational_exactly() {
    assert_shows("the Studio", studio(RATIONAL), RATIONAL_SHOWN);
}

#[test]
fn the_studio_resolves_a_supertask() {
    assert_shows("the Studio", studio(LAMP), LAMP_SHOWN);
}

#[test]
fn the_sync_interpreter_divides_a_rational_exactly() {
    assert_shows("the interpreter", interpreter(RATIONAL), RATIONAL_SHOWN);
}

#[test]
fn an_asynchronous_program_divides_a_rational_exactly() {
    assert_shows(
        "the asynchronous interpreter",
        interpreter(&asynchronous(RATIONAL)),
        RATIONAL_SHOWN,
    );
}

#[test]
fn an_asynchronous_program_resolves_a_supertask() {
    assert_shows(
        "the asynchronous interpreter",
        interpreter(&asynchronous(LAMP)),
        LAMP_SHOWN,
    );
}

#[test]
fn the_vm_io_runner_divides_a_rational_exactly() {
    assert_shows("the VM's IO runner", vm_io(RATIONAL), RATIONAL_SHOWN);
}

#[test]
fn the_vm_io_runner_resolves_a_supertask() {
    assert_shows("the VM's IO runner", vm_io(LAMP), LAMP_SHOWN);
}

#[test]
fn the_v2_optimized_program_divides_a_rational_exactly() {
    assert_shows("the v2-optimized program", v2_optimized(RATIONAL), RATIONAL_SHOWN);
}

#[test]
fn the_v2_optimized_program_resolves_a_supertask() {
    assert_shows("the v2-optimized program", v2_optimized(LAMP), LAMP_SHOWN);
}

#[test]
fn the_first_projection_keeps_a_rational_division_exact() {
    let residual = first_projection(RATIONAL);
    let agreed = common::run_all_tiers_within(&residual, Duration::from_secs(60));
    assert_eq!(agreed.error, None, "the residual program fails:\n{residual}");
    assert_eq!(agreed.output.trim(), RATIONAL_SHOWN, "the residual program:\n{residual}");
}

#[test]
fn the_first_projection_keeps_every_declared_type() {
    let residual = first_projection(
        "## Main\n\
         Let x: Rational be 7 / 2.\n\
         Let mutable n: Int be 5.\n\
         Let names: Seq of Text be a new Seq of Text.\n\
         Let counts be a new Map of Text to Int.\n\
         Set n to n + 1.\n\
         Push \"a\" to names.\n\
         Set item \"a\" of counts to n.\n\
         Show x.\n\
         Show n.\n\
         Show length of names.\n\
         Show item \"a\" of counts.\n",
    );
    for declared in [
        "Let x: Rational be ",
        "Let mutable n: Int be ",
        "Let names: Seq of Text be a new Seq of Text.",
        "Let counts be a new Map of Text to Int.",
    ] {
        assert!(
            residual.contains(declared),
            "the residual program drops a declared type (`{declared}`):\n{residual}"
        );
    }
}

#[test]
fn the_first_projection_keeps_the_fields_a_value_is_made_with() {
    let program = "## A Point has:\n\
                   \x20   An x: Int.\n\
                   \x20   A y: Int.\n\
                   \n\
                   ## A Tree is either:\n\
                   \x20   A Leaf.\n\
                   \x20   A Node with value Int and left Tree and right Tree.\n\
                   \n\
                   ## To sum (t: Tree) -> Int:\n\
                   \x20   Inspect t:\n\
                   \x20       When Leaf: Return 0.\n\
                   \x20       When Node (v, l, r):\n\
                   \x20           Return v + sum(l) + sum(r).\n\
                   \n\
                   ## Main\n\
                   Let n be length of args().\n\
                   Let p be a new Point with x n + 3 and y n + 4.\n\
                   Let leaf be a new Leaf.\n\
                   Let t be a new Node with value n + 1 and left (a new Node with value n + 2 and left leaf and right leaf) and right leaf.\n\
                   Show p's x + p's y.\n\
                   Show sum(t).\n";
    let reference = common::run_all_tiers(program);
    assert_eq!(reference.error, None);
    let residual = first_projection(program);
    let agreed = common::run_all_tiers_within(&residual, Duration::from_secs(60));
    assert_eq!(agreed.error, None, "the residual program fails:\n{residual}");
    assert_eq!(
        agreed.output.trim(),
        reference.output.trim(),
        "the residual program prints something else:\n{residual}"
    );
}

#[test]
fn the_first_projection_resolves_a_supertask() {
    let residual = within_a_minute("the first projection", || first_projection(LAMP));
    let agreed = common::run_all_tiers_within(&residual, Duration::from_secs(60));
    assert_eq!(agreed.error, None, "the residual program fails:\n{residual}");
    assert_eq!(agreed.output.trim(), LAMP_SHOWN, "the residual program:\n{residual}");
}

// ── the compiled paths: each emitted program is built with rustc and run ─────────────────────

fn assert_runs(what: &str, rust: &str, deps: &[logicaffeine_compile::CrateDependency]) {
    let run = common::run_rust_project(rust, deps, &[]);
    assert!(run.success, "{what}'s program does not build or run:\n{}\n{rust}", run.stderr);
    assert_eq!(run.stdout.trim(), RATIONAL_SHOWN, "{what}'s program:\n{rust}");
}

#[test]
fn e2e_a_project_build_divides_a_rational_exactly() {
    let dir = tempfile::tempdir().expect("temp dir");
    let main = dir.path().join("main.lg");
    std::fs::write(&main, RATIONAL).expect("write main.lg");
    let out = logicaffeine_compile::compile::compile_project(&main)
        .unwrap_or_else(|e| panic!("the project does not compile: {e:?}"));
    assert_runs("`largo build`", &out.rust_code, &out.dependencies);
}

#[test]
fn e2e_a_checked_compile_divides_a_rational_exactly() {
    let rust = logicaffeine_compile::compile::compile_to_rust_checked(RATIONAL)
        .unwrap_or_else(|e| panic!("the checked compile refused: {e:?}"));
    assert_runs("the checked compile", &rust, &[]);
}

#[test]
fn e2e_the_studio_rust_view_divides_a_rational_exactly() {
    let rust = logicaffeine_compile::generate_rust_code(RATIONAL)
        .unwrap_or_else(|e| panic!("the Studio's Rust view refused: {e:?}"));
    assert_runs("the Studio's Rust view", &rust, &[]);
}

#[cfg(feature = "verification")]
#[test]
fn e2e_a_verified_compile_divides_a_rational_exactly() {
    let rust = logicaffeine_compile::compile::compile_to_rust_verified(RATIONAL)
        .unwrap_or_else(|e| panic!("the verified compile refused: {e:?}"));
    assert_runs("the verified compile", &rust, &[]);
}

/// A function the VM hands to compiled native code: `n / 2` taken twice is `n` only when the
/// division is exact.
const HALVES: &str = "## To halves (n: Int) -> Int:\n\
\x20   Let r: Rational be n / 2.\n\
\x20   Let twice: Rational be r + r.\n\
\x20   Let whole: Rational be n.\n\
\x20   If twice equals whole:\n\
\x20       Return 1.\n\
\x20   Return 0.\n\
\n\
## Main\n\
Show halves(7).\n";

#[test]
fn e2e_a_native_function_divides_a_rational_exactly() {
    use logicaffeine_compile::compile::{build_native_cdylib_cached, compile_function_to_native_rust};
    use logicaffeine_compile::ui_bridge::with_parsed_program;
    use logicaffeine_compile::vm::aot_tier::load_aot_native;
    use logicaffeine_compile::vm::{Compiler, Vm};
    use logicaffeine_language::ast::Stmt;
    use std::sync::atomic::Ordering;

    let reference = common::run_all_tiers(HALVES);
    assert_eq!(reference.error, None);
    assert_eq!(reference.output.trim(), "1", "the interpreters divide exactly");

    let cache = tempfile::tempdir().expect("cache dir");
    let (shown, calls) = with_parsed_program(HALVES, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let sym = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { name, .. } if interner.resolve(*name) == "halves" => Some(*name),
                _ => None,
            })
            .expect("halves is defined");
        let index = *program.fn_index.get(&sym).expect("halves is compiled") as usize;
        let module = compile_function_to_native_rust(HALVES, "halves")
            .expect("parses")
            .expect("halves is in the native subset");
        let so = build_native_cdylib_cached(&module.rust, "halves", cache.path())
            .unwrap_or_else(|e| panic!("the native module does not build: {e}\n{}", module.rust));
        let (native, calls) =
            load_aot_native(&so, &module.symbol, module.arity).expect("the native module loads");
        let forge = logicaffeine_jit::ForgeTier::new();
        let mut vm = Vm::new(&program)
            .with_native_tier(&forge)
            .with_policy_ctx(&policies, interner);
        vm.install_aot_native(index, native);
        vm.run().expect("runs");
        (vm.output().trim().to_string(), calls.load(Ordering::Relaxed))
    });
    assert!(calls > 0, "the VM never called the native function");
    assert_eq!(shown, "1", "the native function floors the division");
}
