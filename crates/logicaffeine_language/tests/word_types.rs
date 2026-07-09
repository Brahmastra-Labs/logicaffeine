//! `Word8`/`Word16`/`Word32`/`Word64` are FIRST-CLASS primitive types — the ℤ/2ⁿ machine-word
//! ring that every symmetric primitive computes in (ChaCha over ℤ/2³², Keccak over ℤ/2⁶⁴) and
//! the home of ML-KEM's fixed-width coefficients. A type annotation `Word32` must parse to
//! `TypeExpr::Primitive`, and `Seq of Word32` to `Generic { Seq, [Primitive(Word32)] }` — NOT
//! `Named` (an undefined user type, which would strip the wrapping semantics on the way down).

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{Expr, LogicExpr, NounPhrase, Stmt, Term, ThematicRole, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::{Lexer, Parser};

fn parse(input: &str) -> Vec<Stmt<'static>> {
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

    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();
    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);
    parser.parse_program().expect("program should parse")
}

fn first_fn_sig(
    stmts: &'static [Stmt<'static>],
) -> (&'static [(Symbol, &'static TypeExpr<'static>)], Option<&'static TypeExpr<'static>>) {
    for s in stmts {
        if let Stmt::FunctionDef { params, return_type, .. } = s {
            return (params, *return_type);
        }
    }
    panic!("no function def parsed");
}

#[test]
fn word32_param_and_return_parse_as_primitive_not_named() {
    let stmts: &'static [Stmt<'static>] = Box::leak(parse("## To f (a: Word32) -> Word32:\n    Return a.\n").into_boxed_slice());
    let (params, ret) = first_fn_sig(stmts);
    assert!(
        matches!(params[0].1, TypeExpr::Primitive(_)),
        "Word32 param must be a Primitive type, got {:?}",
        params[0].1
    );
    assert!(
        matches!(ret, Some(TypeExpr::Primitive(_))),
        "Word32 return must be Primitive, got {:?}",
        ret
    );
}

#[test]
fn word64_16_8_all_parse_as_primitive() {
    for ty in ["Word64", "Word16", "Word8"] {
        let src = format!("## To f (a: {ty}) -> {ty}:\n    Return a.\n");
        let stmts: &'static [Stmt<'static>] = Box::leak(parse(&src).into_boxed_slice());
        let (params, _) = first_fn_sig(stmts);
        assert!(
            matches!(params[0].1, TypeExpr::Primitive(_)),
            "{ty} param must be Primitive, got {:?}",
            params[0].1
        );
    }
}

#[test]
fn made_up_type_still_parses_as_named() {
    // Contrast: the primitive gate must DISTINGUISH, not blanket-accept any capitalized word.
    let stmts: &'static [Stmt<'static>] = Box::leak(parse("## To g (a: Glorp) -> Glorp:\n    Return a.\n").into_boxed_slice());
    let (params, _) = first_fn_sig(stmts);
    assert!(
        matches!(params[0].1, TypeExpr::Named(_)),
        "an undefined type must stay Named, got {:?}",
        params[0].1
    );
}

#[test]
fn word32_multiparam_function_parses() {
    // The ChaCha `mix` shape: a multi-param Word32 function with a `xor` body. Must parse.
    let stmts = parse(
        "## To mix (x: Word32) and (y: Word32) -> Word32:\n    Let z be x xor y.\n    Return z.\n",
    );
    assert!(
        stmts.iter().any(|s| matches!(s, Stmt::FunctionDef { .. })),
        "multi-param Word32 function must parse to a FunctionDef"
    );
}

#[test]
fn word32_single_param_xor_body_parses() {
    // Narrow the cause: a single-param Word32 fn whose body uses bare `xor` infix.
    let stmts = parse("## To f (x: Word32) -> Word32:\n    Let z be x xor x.\n    Return z.\n");
    assert!(stmts.iter().any(|s| matches!(s, Stmt::FunctionDef { .. })));
}

#[test]
fn word32_function_then_main_parses() {
    // The full AOT-test shape: a Word32 function followed by `## Main` that calls it.
    let stmts = parse(
        "## To mix (x: Word32) and (y: Word32) -> Word32:\n    Let z be x xor y.\n    Return z.\n## Main\nLet r be mix(word32(1), word32(2)).\nShow r.\n",
    );
    assert!(stmts.iter().any(|s| matches!(s, Stmt::FunctionDef { .. })), "function present");
}

#[test]
fn seq_of_word32_parses_as_generic_over_primitive() {
    let stmts: &'static [Stmt<'static>] = Box::leak(parse("## To h (xs: Seq of Word32) -> Int:\n    Return 0.\n").into_boxed_slice());
    let (params, _) = first_fn_sig(stmts);
    match params[0].1 {
        TypeExpr::Generic { params: inner, .. } => assert!(
            matches!(inner[0], TypeExpr::Primitive(_)),
            "Seq of Word32 element must be Primitive, got {:?}",
            inner[0]
        ),
        other => panic!("Seq of Word32 must be Generic, got {:?}", other),
    }
}
