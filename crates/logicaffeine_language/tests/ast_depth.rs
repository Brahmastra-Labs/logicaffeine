//! The AST depth gate: a program whose tree nests beyond the depth limit
//! parses to a graceful `AstTooDeep` error — never a stack-overflow abort
//! in any downstream walker, on any surface (CLI, REPL, LSP, Studio).
//!
//! Every test runs in a deliberately SMALL (2 MiB) stack thread: the gate
//! itself must be iterative, and the guarantee must hold in the tightest
//! standard environment (worker threads, wasm), not just an 8 MiB main
//! thread.

use logicaffeine_base::Arena;
use logicaffeine_language::analysis::DiscoveryPass;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::stmt::{Expr, Stmt, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::error::ParseErrorKind;
use logicaffeine_language::{Interner, Lexer, Parser};

/// Parse a program exactly the way `ui_bridge::with_parsed_program` does,
/// inside a 2 MiB thread. Returns the statement count or the error kind.
fn parse_verdict(source: String) -> Result<usize, ParseErrorKind> {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let mut interner = Interner::new();
            let mut lexer = Lexer::new(&source, &mut interner);
            let tokens = lexer.tokenize();

            let type_registry = {
                let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
                discovery.run_full().types
            };

            let expr_arena = Arena::new();
            let term_arena = Arena::new();
            let np_arena = Arena::new();
            let sym_arena = Arena::new();
            let role_arena = Arena::new();
            let pp_arena = Arena::new();
            let stmt_arena: Arena<Stmt> = Arena::new();
            let imperative_expr_arena: Arena<Expr> = Arena::new();
            let type_expr_arena: Arena<TypeExpr> = Arena::new();
            let ctx = AstContext::with_types(
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

            let mut world_state = WorldState::new();
            let mut parser =
                Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
            match parser.parse_program() {
                Ok(stmts) => Ok(stmts.len()),
                Err(e) => Err(e.kind),
            }
        })
        .expect("spawn")
        .join()
        .expect("the 2MiB parse thread must never crash")
}

fn chain_program(terms: usize) -> String {
    let mut src = String::from("## Main\n\nShow 1");
    for _ in 1..terms {
        src.push_str(" + 1");
    }
    src.push_str(".\n");
    src
}

/// A 10,000-term chain is REJECTED by the depth gate — parse returns an
/// `AstTooDeep` error instead of any walker overflowing later.
#[test]
fn long_operator_chain_is_rejected_not_crashed() {
    match parse_verdict(chain_program(10_000)) {
        Err(ParseErrorKind::AstTooDeep { depth, max_depth }) => {
            assert!(depth > max_depth, "reported depth {depth} vs limit {max_depth}");
        }
        Err(other) => panic!("expected AstTooDeep, got {other:?}"),
        Ok(_) => panic!("a 10k-term chain must not pass the gate"),
    }
}

/// Deep parenthesis nesting is equally rejected — including surviving the
/// PARSER's own recursion (this is the shape that recurses at parse time).
#[test]
fn deep_parentheses_are_rejected_not_crashed() {
    let mut src = String::from("## Main\n\nShow ");
    for _ in 0..5_000 {
        src.push('(');
    }
    src.push('1');
    for _ in 0..5_000 {
        src.push(')');
    }
    src.push_str(".\n");
    match parse_verdict(src) {
        Err(_) => {} // any graceful ParseError is acceptable; aborting is not
        Ok(_) => panic!("5k-deep parens must not pass the gate"),
    }
}

/// Ordinary programs — including a healthy 100-term expression — are
/// untouched by the gate.
#[test]
fn ordinary_programs_pass_the_gate() {
    assert!(parse_verdict(chain_program(100)).is_ok(), "100-term chain must parse");
    assert!(
        parse_verdict("## Main\n\nLet x be 5.\nIf x is less than 9:\n    Show x.\n".to_string())
            .is_ok(),
        "ordinary nesting must parse"
    );
}

/// Deep STATEMENT nesting (blocks inside blocks) hits the same gate.
#[test]
fn deep_block_nesting_is_rejected_not_crashed() {
    let mut src = String::from("## Main\n\nLet x be 1.\n");
    let depth = 3_000;
    for level in 0..depth {
        let indent = "    ".repeat(level);
        src.push_str(&format!("{indent}If x is less than 2:\n"));
    }
    let body_indent = "    ".repeat(depth);
    src.push_str(&format!("{body_indent}Show x.\n"));
    match parse_verdict(src) {
        Err(_) => {}
        Ok(_) => panic!("3k-deep block nesting must not pass the gate"),
    }
}
