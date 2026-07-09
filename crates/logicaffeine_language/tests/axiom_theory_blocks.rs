//! `## Axiom` and `## Theory` blocks parse to `Stmt::Axiom` / `Stmt::Theory`, and the
//! captured formal body round-trips through the formal-formula parser — proving the
//! surface seam delivers exactly the `ProofExpr` the prover consumes (the seam for an
//! axiomatic base like Tarski geometry).

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{LogicExpr, NounPhrase, Stmt, Term, ThematicRole};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::{Lexer, Parser};
use logicaffeine_proof::formula::parse_formula;
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn parse_program(input: &str) -> Vec<Stmt<'static>> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));
    let ctx = AstContext::new(expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena);

    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();
    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);
    parser.parse_program().expect("program should parse")
}

fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn cong(a: ProofTerm, b: ProofTerm, c: ProofTerm, d: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: "Cong".to_string(), args: vec![a, b, c, d], world: None }
}
fn forall(vars: &[&str], body: ProofExpr) -> ProofExpr {
    vars.iter().rev().fold(body, |acc, var| ProofExpr::ForAll {
        variable: var.to_string(),
        body: Box::new(acc),
    })
}

#[test]
fn axiom_block_parses_and_body_round_trips_to_proof_expr() {
    let stmts = parse_program("## Axiom flip: for all a b, Cong(a, b, b, a).\n");

    let axiom = stmts
        .iter()
        .find_map(|s| if let Stmt::Axiom(a) = s { Some(a) } else { None })
        .expect("## Axiom should parse to a Stmt::Axiom");

    assert!(
        axiom.name.eq_ignore_ascii_case("flip"),
        "axiom name should be 'flip', got {:?}",
        axiom.name
    );

    // The captured body must be exactly the formal formula the prover consumes.
    let parsed = parse_formula(&axiom.formula)
        .unwrap_or_else(|e| panic!("axiom body must parse as a formula; body={:?} err={e}", axiom.formula));
    let expected = forall(&["a", "b"], cong(v("a"), v("b"), v("b"), v("a")));
    assert_eq!(parsed, expected, "captured body text was {:?}", axiom.formula);
}

#[test]
fn theory_block_parses_with_its_name() {
    let stmts = parse_program("## Theory Tarski\n");
    let theory = stmts
        .iter()
        .find_map(|s| if let Stmt::Theory(t) = s { Some(t) } else { None })
        .expect("## Theory should parse to a Stmt::Theory");
    assert!(
        theory.name.eq_ignore_ascii_case("tarski"),
        "theory name should be 'Tarski', got {:?}",
        theory.name
    );
}

#[test]
fn axiom_block_with_implication_round_trips() {
    let stmts = parse_program(
        "## Axiom transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).\n",
    );
    let axiom = stmts
        .iter()
        .find_map(|s| if let Stmt::Axiom(a) = s { Some(a) } else { None })
        .expect("## Axiom should parse to a Stmt::Axiom");
    let parsed = parse_formula(&axiom.formula)
        .unwrap_or_else(|e| panic!("body={:?} err={e}", axiom.formula));
    let expected = forall(
        &["a", "b", "c", "d", "e", "f"],
        ProofExpr::Implies(
            Box::new(ProofExpr::And(
                Box::new(cong(v("a"), v("b"), v("c"), v("d"))),
                Box::new(cong(v("a"), v("b"), v("e"), v("f"))),
            )),
            Box::new(cong(v("c"), v("d"), v("e"), v("f"))),
        ),
    );
    assert_eq!(parsed, expected, "captured body text was {:?}", axiom.formula);
}

#[test]
fn theory_block_collects_a_multiline_development_body() {
    // A multi-line `## Theory` body (indented, blank-line-separated) must be collected as
    // clean text that the formal-development parser can consume — proving layout tokens
    // (Newline/Indent/Dedent) are stripped, not emitted, by the block collector.
    use logicaffeine_proof::development::parse_development;

    let program = "\
## Theory Tarski

Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).
Axiom inner_transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
Theorem symmetry cites reflexivity: prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
";
    let stmts = parse_program(program);
    let theory = stmts
        .iter()
        .find_map(|s| if let Stmt::Theory(t) = s { Some(t) } else { None })
        .expect("## Theory should parse to a Stmt::Theory");
    assert!(theory.name.eq_ignore_ascii_case("tarski"));

    let dev = parse_development(&theory.body)
        .unwrap_or_else(|e| panic!("collected theory body must parse; body={:?} err={e}", theory.body));
    assert_eq!(dev.axioms.len(), 2, "two axioms collected; body={:?}", theory.body);
    assert_eq!(dev.theorems.len(), 2, "two theorems collected; body={:?}", theory.body);
    assert_eq!(dev.theorems[1].cites, vec!["reflexivity".to_string()]);
}
