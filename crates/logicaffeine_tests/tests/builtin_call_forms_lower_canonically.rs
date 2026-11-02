//! `length(xs)` and `copy(xs)` are the "also works" spellings of `length of xs` and
//! `copy of xs` (LOGOS_QUICKGUIDE §5.2): one meaning, lowered to one expression —
//! `Expr::Length` / `Expr::Copy` — so every tier, the type checker and the ownership
//! checker read the call exactly as they read the natural form.
//!
//! The parser used to keep them as plain calls. The interpreter resolved those as builtins,
//! but the AOT backend emitted `length(items.clone())` — a free function the generated crate
//! does not have — so the Studio's bubble-sort showed Rust that would not build, and the
//! ownership checker read the call as giving `items` away.

mod common;

use common::{assert_all_tiers, assert_compiled_equals_interpreted};
use logicaffeine_base::{Arena, Interner};
use logicaffeine_language::{
    analysis::TypeRegistry,
    arena_ctx::AstContext,
    ast::{Expr, Stmt},
    drs::WorldState,
    Lexer, Parser,
};

/// The expression bound by the program's first `Let`, described by its variant.
fn first_let_value(source: &str) -> String {
    let mut interner = Interner::new();
    let tokens = Lexer::new(source, &mut interner).tokenize();
    let mut world_state = WorldState::new();
    let (expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena) =
        (Arena::new(), Arena::new(), Arena::new(), Arena::new(), Arena::new(), Arena::new());
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_arena = Arena::new();
    let ctx = AstContext::with_types(
        &expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena,
        &stmt_arena, &imperative_expr_arena, &type_arena,
    );
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, TypeRegistry::default());
    parser.process_block_headers();
    let stmts = parser.parse_program().expect("parses");
    let value = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::Let { value, .. } => Some(*value),
            _ => None,
        })
        .expect("the program has a Let");
    match value {
        Expr::Length { .. } => "Length".to_string(),
        Expr::Copy { .. } => "Copy".to_string(),
        Expr::Call { .. } => "Call".to_string(),
        other => format!("{other:?}").split(|c: char| !c.is_alphanumeric()).next().unwrap_or("").to_string(),
    }
}

#[test]
fn length_called_like_a_function_is_the_length_of_its_argument() {
    assert_eq!(first_let_value("## Main\nLet n be length([4, 5]).\nShow n.\n"), "Length");
    assert_eq!(first_let_value("## Main\nLet n be length of [4, 5].\nShow n.\n"), "Length");
}

#[test]
fn copy_called_like_a_function_is_a_copy_of_its_argument() {
    assert_eq!(first_let_value("## Main\nLet c be copy([4, 5]).\nShow c.\n"), "Copy");
    assert_eq!(first_let_value("## Main\nLet c be copy of [4, 5].\nShow c.\n"), "Copy");
}

const LENGTH_CALL: &str = "## Main\nLet items be [4, 5, 6].\nLet n be length(items).\nShow n.\nShow length(\"abc\").\nShow items.\n";
const COPY_CALL: &str = "## Main\nLet items be [1, 2].\nLet mutable c be copy(items).\nPush 3 to c.\nShow items.\nShow c.\n";

#[test]
fn length_called_like_a_function_agrees_on_every_tier() {
    assert_all_tiers(LENGTH_CALL, "3\n3\n[4, 5, 6]");
}

#[test]
fn copy_called_like_a_function_agrees_on_every_tier() {
    assert_all_tiers(COPY_CALL, "[1, 2]\n[1, 2, 3]");
}

#[test]
fn length_called_like_a_function_builds_and_runs_as_rust() {
    assert_compiled_equals_interpreted(LENGTH_CALL);
}

#[test]
fn copy_called_like_a_function_builds_and_runs_as_rust() {
    assert_compiled_equals_interpreted(COPY_CALL);
}

/// Anything but exactly one argument is not the natural form's meaning, and keeps the
/// builtin's own arity error.
#[test]
fn length_with_two_arguments_is_still_an_arity_error() {
    let agreed = common::run_all_tiers("## Main\nShow length([1], [2]).\n");
    let error = agreed.error.expect("length takes one argument");
    assert!(error.contains("length"), "{error}");
}

/// The call form reads as the builtin only while the program has no function of that name: a
/// program's own `copy` or `length` is called, as any declared function is, and the natural
/// form still means the builtin.
const OWN_COPY: &str = "## To copy (xs: Seq of Int) -> Seq of Int:\n    Let mutable out be a new Seq of Int.\n    Push 99 to out.\n    Return out.\n\n## Main\nLet items be [1, 2].\nShow copy(items).\nShow copy of items.\n";
const OWN_LENGTH: &str = "## To length (xs: Seq of Int) -> Int:\n    Return 7.\n\n## Main\nLet items be [1, 2].\nShow length(items).\nShow length of items.\n";

#[test]
fn a_declared_copy_runs_its_own_body_on_every_tier() {
    assert_all_tiers(OWN_COPY, "[99]\n[1, 2]");
}

#[test]
fn a_declared_length_runs_its_own_body_on_every_tier() {
    assert_all_tiers(OWN_LENGTH, "7\n2");
}
