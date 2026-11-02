//! Wave 4 — transport reliability qualifiers on `Send`.
//!
//! `Send unreliably <msg> to <peer>` marks the message as an unordered, best-effort datagram
//! (may drop or reorder) instead of the reliable, ordered default — the transport dial the media
//! path pairs with FEC so a lost frame recovers without a retransmit stall. Locked at the AST
//! level: the parsed `SendMessage` carries `unreliable: true` exactly when the qualifier is
//! present, and stays `false` (reliable) otherwise. The test bites (a parser that drops or
//! hard-codes the flag diverges from the qualifier).

use logicaffeine_base::{Arena, Interner};
use logicaffeine_language::{
    analysis::DiscoveryPass,
    arena_ctx::AstContext,
    ast::{Expr, Stmt, TypeExpr},
    drs::WorldState,
    Lexer, Parser,
};

/// Parse imperative `source` through the production pipeline (lex → MWE → discovery → parse) and
/// hand the top-level statements to `f`. The statements borrow local arenas, so inspection has to
/// happen inside the closure.
fn with_parsed<R>(source: &str, f: impl FnOnce(&[Stmt]) -> R) -> R {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    let mwe_trie = logicaffeine_language::mwe::build_mwe_trie();
    let tokens = logicaffeine_language::mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);
    let type_registry = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        discovery.run_full().types
    };
    let mut world_state = WorldState::new();
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();
    let ast_ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        type_registry,
    );
    let stmts = parser.parse_program().expect("program must parse");
    f(&stmts)
}

fn send_unreliable_flag(stmts: &[Stmt]) -> bool {
    for s in stmts {
        if let Stmt::SendMessage { unreliable, .. } = s {
            return *unreliable;
        }
    }
    panic!("no SendMessage statement was parsed");
}

/// `Send unreliably …` sets the datagram (unreliable) flag on the message.
#[test]
fn send_unreliably_marks_the_message_as_a_datagram() {
    with_parsed("## Main\nSend unreliably \"frame\" to peer.\n", |stmts| {
        assert!(
            send_unreliable_flag(stmts),
            "the `unreliably` qualifier must set unreliable = true"
        )
    });
}

/// A plain `Send` stays reliable (ordered) — the default, with no qualifier present.
#[test]
fn plain_send_is_reliable_by_default() {
    with_parsed("## Main\nSend \"frame\" to peer.\n", |stmts| {
        assert!(
            !send_unreliable_flag(stmts),
            "a plain Send must stay reliable (unreliable = false)"
        )
    });
}
