//! `Connect to <addr> with pad "<path>" as initiator|responder` and the matching `Listen on <addr>
//! with pad "<path>" as responder|initiator` clause parse into an optional `secure` binding on the
//! two network statements — the language surface for the PNP one-time-pad tier. A bare `Connect`/
//! `Listen` keeps `secure = None`, so every existing program parses byte-identically.

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, Literal, LogicExpr, NounPhrase, SecurePad, SecureRole, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::{Lexer, Parser};

/// Parse a program (wrapped in `## Main`, where imperative statements live) and hand back both the
/// statements and the interner, so a test can resolve interned string literals to their text.
fn parse(input: &str) -> (Vec<Stmt<'static>>, &'static Interner) {
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
        expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena,
        stmt_arena, iexpr_arena, type_arena,
    );

    // Imperative statements (Connect/Listen) live under a `## Main` section.
    let src = format!("## Main\n{input}");
    let mut lexer = Lexer::new(&src, interner);
    let tokens = lexer.tokenize();
    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);
    let stmts = parser.parse_program().expect("program should parse");
    drop(parser); // release the &mut Interner borrow before we read it back
    // The interner is leaked (lives for the whole process), so a shared 'static view is sound.
    let interner_ref: &'static Interner = unsafe { &*(interner as *const Interner) };
    (stmts, interner_ref)
}

/// Resolve the pad path of a `SecurePad` (a string-literal expression) to its text.
fn pad_path<'a>(bind: &SecurePad<'a>, interner: &Interner) -> String {
    match bind.pad {
        Expr::Literal(Literal::Text(sym)) => interner.resolve(*sym).to_string(),
        other => panic!("pad path must be a string literal, got {other:?}"),
    }
}

#[test]
fn connect_with_pad_parses_secure_binding_initiator() {
    let (stmts, interner) = parse("Connect to \"relay\" with pad \"secret.pad\" as initiator.");
    let bind = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::ConnectTo { secure, .. } => Some(secure.as_ref()),
            _ => None,
        })
        .expect("a ConnectTo statement")
        .expect("the secure binding is present");
    assert_eq!(bind.role, SecureRole::Initiator, "role parsed as initiator");
    assert_eq!(pad_path(bind, interner), "secret.pad", "pad path is the given file");
}

#[test]
fn listen_with_pad_parses_secure_binding_responder() {
    let (stmts, interner) = parse("Listen on \"me\" with pad \"secret.pad\" as responder.");
    let bind = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::Listen { secure, .. } => Some(secure.as_ref()),
            _ => None,
        })
        .expect("a Listen statement")
        .expect("the secure binding is present");
    assert_eq!(bind.role, SecureRole::Responder, "role parsed as responder");
    assert_eq!(pad_path(bind, interner), "secret.pad", "pad path is the given file");
}

#[test]
fn pad_path_is_distinct_from_the_address() {
    // The pad path must be the token after `with pad`, never the address literal.
    let (stmts, interner) = parse("Connect to \"the-relay-address\" with pad \"the-pad-file\" as initiator.");
    let (address, bind) = stmts
        .iter()
        .find_map(|s| match s {
            Stmt::ConnectTo { address, secure } => secure.as_ref().map(|b| (*address, b)),
            _ => None,
        })
        .expect("a ConnectTo with a secure binding");
    let addr_text = match address {
        Expr::Literal(Literal::Text(sym)) => interner.resolve(*sym).to_string(),
        other => panic!("address should be a string literal, got {other:?}"),
    };
    assert_eq!(addr_text, "the-relay-address", "address is the first literal");
    assert_eq!(pad_path(bind, interner), "the-pad-file", "pad is the literal after `with pad`");
}

#[test]
fn bare_connect_and_listen_have_no_secure_binding() {
    let (stmts, _) = parse("Connect to \"relay\".\nListen on \"me\".");
    let mut saw_connect = false;
    let mut saw_listen = false;
    for s in &stmts {
        match s {
            Stmt::ConnectTo { secure, .. } => {
                assert!(secure.is_none(), "a bare Connect carries no secure binding");
                saw_connect = true;
            }
            Stmt::Listen { secure, .. } => {
                assert!(secure.is_none(), "a bare Listen carries no secure binding");
                saw_listen = true;
            }
            _ => {}
        }
    }
    assert!(saw_connect && saw_listen, "both statements parsed");
}
