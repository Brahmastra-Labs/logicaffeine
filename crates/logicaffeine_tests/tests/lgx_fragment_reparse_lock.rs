//! ★ LOCK — L11: the editor's per-block recovery reparse does not invent cross-block errors. ★
//!
//! `Parser::parse_program` ends with four checks that only mean something once every `##` block
//! in the file has been seen: `Mount <Name>` names a component that exists, a mounted component
//! takes no props, a `## Page` mounts itself, and the whole-file LGX resolution (stores, routes,
//! directives, the render functions every component gains). An editor that reparses ONE block on
//! every keystroke — the fast path a live-typing recovery needs — hands `parse_program` only that
//! one block's tokens, so before this lock's fix, a `Mount Foo.` inside the block being retyped
//! reported "Foo doesn't exist" the instant a keystroke landed, for a `Foo` declared in perfectly
//! valid LGX two blocks up that the reparse simply never saw.
//!
//! `Parser::fragment_mode` (`false` by default — every existing caller, whole-file, is unaffected)
//! gates exactly those four calls off. It does NOT make cross-block LGX resolution correct inside a
//! fragment — `resolve_lgx`'s twelve sub-passes are cross-block-dependent throughout, and giving
//! them the whole program's name tables from inside a fragment reparse is separate, larger work
//! this lock does not claim. It turns a WRONG error into no error for the four checks that were
//! architecturally never going to be fixable per-fragment without a full incremental-resolution
//! redesign — the editor's own next whole-file reparse (on save, or its own debounce) still gives
//! complete, correct checking.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::DiscoveryPass;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::error::ParseError;
use logicaffeine_language::{Lexer, Parser};

fn parse(source: &str, fragment: bool) -> Result<Vec<Stmt<'static>>, ParseError> {
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
    let mut lexer = Lexer::new(source, interner);
    let tokens = lexer.tokenize();
    let registry = {
        let mut discovery = DiscoveryPass::new(&tokens, interner);
        discovery.run_full().types
    };
    let mut parser = Parser::new(tokens, world_state, interner, ctx, registry);
    parser.fragment_mode = fragment;
    parser.parse_program()
}

/// The control: a whole file mounting an undeclared name is refused exactly as before —
/// `fragment_mode` defaults to `false`, so nothing about the ordinary, whole-file path changed.
#[test]
fn a_whole_file_mounting_an_undeclared_name_is_still_refused() {
    let src = "## Main\nMount Ghost.\n";
    let err =
        parse(src, false).expect_err("a whole-file Mount of an undeclared name must be refused");
    assert!(
        format!("{err:?}").contains("Ghost"),
        "the refusal must name the undeclared component: {err:?}"
    );
}

/// The fix: the SAME source, reparsed as a fragment (as the editor's per-block recovery would hand
/// it — just this one block, `Foo`'s own `## Component` declaration living in whatever OTHER block
/// the editor did not include in this reparse), no longer reports the cross-block name as missing.
#[test]
fn a_fragment_mounting_a_name_declared_elsewhere_is_not_falsely_refused() {
    let src = "## Main\nMount Foo.\n";
    let stmts = parse(src, true).unwrap_or_else(|e| {
        panic!("a fragment reparse must not invent a cross-block refusal for a name it simply did not see this block: {e:?}")
    });
    assert!(
        !stmts.is_empty(),
        "the fragment must still produce its own statements"
    );
}

/// A fragment's OWN internal mistakes are still worth catching where they can be — this lock only
/// claims the four WHOLE-FILE checks are gated, not that a fragment parses no differently at all.
/// A malformed sentence inside the one block being retyped is still a parse error, fragment or not.
#[test]
fn a_fragments_own_malformed_sentence_still_fails_to_parse() {
    let src = "## Main\nLet be .\n";
    assert!(
        parse(src, true).is_err(),
        "a fragment must still refuse a sentence that is not valid LOGOS on its own terms"
    );
}
