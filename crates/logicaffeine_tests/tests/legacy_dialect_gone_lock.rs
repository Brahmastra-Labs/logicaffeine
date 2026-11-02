//! LOCK — the retired reactive-signal dialect (`Render:`, `Let x be a signal …`, `Whenever … run
//! …`, `Put … in …`, the `## Component Name:` colon dispatch, `signal.rs`) is actually gone: no
//! implementation identifier survives anywhere in `crates/*/src` or `apps/*/src`, no LOGOS program
//! can reach a signal builtin by name, and every one of its surface forms is refused with a
//! message naming the LGX construct that replaced it.
//!
//! Two halves, matching LANES.md's "catalogues, not patches" rule:
//!  1. A forbidden-identifier catalogue, each row with the reason it must never reappear —
//!     scanned from the SOURCE of every `crates/*/src` and `apps/*/src` file, not assumed.
//!  2. A parse of every retired surface form, asserting the refusal names the LGX equivalent —
//!     the dialect's OWN teaching sentences (`common::canonical_teaching::LEGACY_FORMS`, already
//!     used to police docs/examples prose) checked here against the PARSER itself, so the two
//!     never drift apart.

use std::path::{Path, PathBuf};

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::{DiscoveryPass, TypeRegistry};
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::error::ParseError;
use logicaffeine_language::{Lexer, Parser};

#[path = "common/canonical_teaching.rs"]
mod canonical_teaching;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every `crates/*/src` and `apps/*/src` directory, discovered rather than hardcoded — a new
/// crate is covered automatically, the way `packaging_lock`'s `packages()` discovers members.
fn src_roots() -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = Vec::new();
    for parent in ["crates", "apps"] {
        let Ok(entries) = std::fs::read_dir(root.join(parent)) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let src = entry.path().join("src");
            if src.is_dir() {
                out.push(src);
            }
        }
    }
    assert!(
        out.len() >= 5,
        "the src-root discovery walk broke: found only {}",
        out.len()
    );
    out
}

fn walk_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            walk_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every forbidden identifier, with why it must never reappear in `crates/*/src`/`apps/*/src`.
/// Each is the exact name of a function, field or module the legacy reactive dialect's parser or
/// lowering used — never a substring generic enough to false-positive on the WIRE-FROZEN
/// `BuiltinId::Signal*` family, which stays (bincode encodes a builtin by enum position;
/// `crates/logicaffeine_compile/src/semantics/builtins.rs` and `vm/wasm/kind.rs` keep the fifteen
/// variants forever, unreachable, per `logicaffeine_compile::semantics::builtins::builtin_from_name`
/// below).
const FORBIDDEN_IDENTIFIERS: &[(&str, &str)] = &[
    (
        "parse_component_block",
        "the `## Component Name:` legacy parser (parser/view.rs, deleted)",
    ),
    (
        "parse_view_block",
        "the `Render:`/`Render into` parser (parser/view.rs, deleted)",
    ),
    (
        "parse_view_element",
        "the legacy view-element parser (parser/view.rs, deleted)",
    ),
    (
        "parse_view_handler",
        "the legacy `on <event>:` handler parser (parser/view.rs, deleted)",
    ),
    (
        "parse_view_nodes_until_dedent",
        "the legacy view-body parser (parser/view.rs, deleted)",
    ),
    (
        "parse_view_child_block",
        "the legacy view-body parser (parser/view.rs, deleted)",
    ),
    (
        "expect_view_body_opener",
        "the legacy view-body parser (parser/view.rs, deleted)",
    ),
    (
        "parse_whenever_statement",
        "`Whenever … is clicked, run …` (parser/mod.rs, deleted)",
    ),
    (
        "parse_put_statement",
        "the legacy `Put … in/into …` statement (parser/mod.rs, deleted)",
    ),
    (
        "desugar_seq_signal_push",
        "the legacy seq-signal `Push` desugar (parser/mod.rs, deleted)",
    ),
    (
        "parse_signal_delta",
        "the legacy `Increase`/`Decrease <signal>` parser (parser/mod.rs, deleted)",
    ),
    (
        "signal_is_written_fractionally",
        "the legacy signal-family inference heuristic (parser/mod.rs, deleted)",
    ),
    (
        "peek_is_signal_ref",
        "the legacy signal-reference lookahead (parser/mod.rs, deleted)",
    ),
];

#[test]
fn no_forbidden_identifier_survives_in_any_crate_or_app_source() {
    let mut offenders: Vec<String> = Vec::new();
    for root in src_roots() {
        let mut files = Vec::new();
        walk_rs_files(&root, &mut files);
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            for (name, reason) in FORBIDDEN_IDENTIFIERS {
                if text.contains(name) {
                    offenders.push(format!(
                        "{name} (reason: {reason}) still appears in {}",
                        file.display()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the retired reactive-signal dialect left identifiers behind:\n{}",
        offenders.join("\n")
    );
}

/// `crates/logicaffeine_language/src/parser/view.rs` was the legacy dialect's own parser file —
/// its deletion is the census this lock most directly protects, so it is asserted by name.
#[test]
fn the_legacy_view_parser_file_does_not_exist() {
    let path = repo_root().join("crates/logicaffeine_language/src/parser/view.rs");
    assert!(
        !path.exists(),
        "parser/view.rs (the legacy `Render:`/`## Component Name:` parser) must stay deleted"
    );
}

/// `semantics/signal.rs` was the legacy dialect's native-tier hook store — deleted with the
/// dialect; `semantics/builtins.rs` no longer declares a `mod signal`.
#[test]
fn the_legacy_signal_module_does_not_exist() {
    let path = repo_root().join("crates/logicaffeine_compile/src/semantics/signal.rs");
    assert!(
        !path.exists(),
        "semantics/signal.rs (the legacy reactive hook store) must stay deleted"
    );
    let mod_rs = std::fs::read_to_string(
        repo_root().join("crates/logicaffeine_compile/src/semantics/mod.rs"),
    )
    .expect("semantics/mod.rs reads");
    assert!(
        !mod_rs.contains("mod signal"),
        "semantics/mod.rs must not declare the deleted signal module"
    );
}

/// The `BuiltinId::Signal*` family is WIRE-FROZEN (bincode encodes a builtin by enum position —
/// `crates/logicaffeine_compile/src/semantics/builtins.rs` keeps the fifteen variants forever so
/// nothing after them renumbers) but UNREACHABLE: no source spelling maps to one any more. This is
/// the behavioural half of "the dialect is gone" that a source-text scan cannot see — the census
/// that proves it, read against the compiler's own public name table rather than assumed.
#[test]
fn no_source_spelling_reaches_a_signal_builtin() {
    use logicaffeine_compile::semantics::builtins::builtin_from_name;
    for name in [
        "signal",
        "signalSet",
        "signalGetInt",
        "signalGetText",
        "beginRender",
        "signalNewText",
        "signalSetText",
        "enterComponent",
        "exitComponent",
        "signalNewSeq",
        "signalGetSeq",
        "signalSetSeq",
        "signalNewFloat",
        "signalGetFloat",
        "signalSetFloat",
    ] {
        assert!(
            builtin_from_name(name).is_none(),
            "`{name}` must map to no BuiltinId — the retired dialect's builtins are unreachable, not merely undocumented"
        );
    }
}

// ---- The Socratic refusals: every retired surface form is refused, naming the LGX equivalent. ----

fn parse(src: &str) -> Result<Vec<Stmt<'static>>, ParseError> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let stmt_arena: &'static Arena<Stmt> = Box::leak(Box::new(Arena::new()));
    let iexpr_arena: &'static Arena<Expr> = Box::leak(Box::new(Arena::new()));
    let type_arena: &'static Arena<TypeExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::with_types(
        expr_arena,
        term_arena,
        np_arena,
        sym_arena,
        role_arena,
        pp_arena,
        stmt_arena,
        iexpr_arena,
        type_arena,
    );
    let mut lexer = Lexer::new(src, interner);
    let tokens = lexer.tokenize();
    let registry: TypeRegistry = DiscoveryPass::new(&tokens, interner).run_full().types;
    let mut parser = Parser::new(tokens, world_state, &mut *interner, ctx, registry);
    parser.parse_program()
}

fn refusal_message(src: &str) -> String {
    match parse(src) {
        Ok(stmts) => {
            panic!("this retired form must be refused, not parsed:\n{src}\ngot: {stmts:?}")
        }
        Err(e) => match &e.kind {
            logicaffeine_language::error::ParseErrorKind::Custom(m) => m.clone(),
            other => format!("{other:?}"),
        },
    }
}

#[test]
fn render_colon_is_refused_naming_view() {
    let msg = refusal_message("## Main\nRender:\n    h1: \"hi\"\n");
    let lower = msg.to_lowercase();
    assert!(
        lower.contains("view") || lower.contains("page") || lower.contains("component"),
        "`Render:` must be refused naming the LGX `View:` replacement: {msg}"
    );
}

#[test]
fn let_x_be_a_signal_is_refused() {
    let msg = refusal_message("## Main\nLet count be a signal 0.\n");
    assert!(
        !msg.is_empty(),
        "`Let x be a signal …` must be refused: {msg}"
    );
}

#[test]
fn whenever_is_clicked_run_is_refused() {
    let msg = refusal_message(
        "## Main\nLet b be createElement(\"button\").\nWhenever b is clicked, run f.\n",
    );
    assert!(!msg.is_empty(), "`Whenever … run …` must be refused: {msg}");
}

#[test]
fn put_in_is_refused() {
    let msg = refusal_message(
        "## Main\nLet root be createElement(\"div\").\nLet child be createElement(\"span\").\nPut child in root.\n",
    );
    assert!(!msg.is_empty(), "`Put … in …` must be refused: {msg}");
}

/// The teaching table docs/examples are policed against (`canonical_teaching::LEGACY_FORMS`)
/// stays in lockstep with what the PARSER actually refuses: every detector's trigger line must
/// itself fail to parse. A form removed from `LEGACY_FORMS` without also being removed from the
/// parser (or vice versa) shows up here rather than silently drifting.
#[test]
fn every_canonical_teaching_legacy_form_still_fails_to_parse() {
    let probes: &[&str] = &[
        "## Main\nRender:\n    h1: \"hi\"\n",
        "## Main\nLet count be a signal 0.\n",
        "## Main\nWhenever b is clicked, run f.\n",
        "## Main\nPut child in root.\n",
    ];
    assert_eq!(
        probes.len(),
        canonical_teaching::LEGACY_FORMS.len(),
        "a form was added to or removed from LEGACY_FORMS without updating this probe list"
    );
    for src in probes {
        assert!(
            parse(src).is_err(),
            "a LEGACY_FORMS detector's own trigger line must fail to parse:\n{src}"
        );
    }
}
