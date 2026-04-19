//! Phase 2 — HW expression operators
//!
//! Precedence and disambiguation tests for the HW-Spec expression extensions.
//! These operators are emitted by the imperative expression parser only when
//! the parser's `hw_context` flag is set (i.e. inside `## Hardware` /
//! `## Property` blocks). Outside those blocks the grammar is unchanged.
//!
//! All tests drive the parser through `parse_imperative_expr` with tokens
//! produced by a source string that opens a `## Hardware` block. The block
//! header wires both the lexer's HW-mode and the parser's `hw_context`, so
//! tests don't have to flip any flags manually.

use logicaffeine_base::{Arena, Interner};
use logicaffeine_language::{
    Lexer, Parser,
    ast::{Expr, BinaryOpKind, UnaryOpKind},
    drs::WorldState,
    arena_ctx::AstContext,
    analysis::TypeRegistry,
};

// ═══════════════════════════════════════════════════════════════════════════
// Test harness
// ═══════════════════════════════════════════════════════════════════════════

fn describe(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Identifier(sym) => format!("id({})", interner.resolve(*sym)),
        Expr::Literal(_) => "lit".to_string(),
        Expr::BinaryOp { op, left, right } => format!(
            "({} {} {})",
            bin_name(*op),
            describe(left, interner),
            describe(right, interner)
        ),
        Expr::UnaryOp { op, operand } => {
            format!("({} {})", un_name(*op), describe(operand, interner))
        }
        Expr::BitSelect { signal, bit } => format!(
            "(bit_select {} {})",
            describe(signal, interner),
            describe(bit, interner)
        ),
        Expr::PartSelect { signal, hi, lo } => format!(
            "(part_select {} {} {})",
            describe(signal, interner),
            describe(hi, interner),
            describe(lo, interner)
        ),
        Expr::HwConcat { parts } => {
            let joined: Vec<String> = parts.iter().map(|p| describe(p, interner)).collect();
            format!("(concat {})", joined.join(" "))
        }
        Expr::Not { operand } => format!("(not {})", describe(operand, interner)),
        other => format!("<unexpected:{:?}>", other),
    }
}

fn bin_name(op: BinaryOpKind) -> &'static str {
    match op {
        BinaryOpKind::BitAnd => "bit_and",
        BinaryOpKind::BitOr => "bit_or",
        BinaryOpKind::BitXor => "bit_xor",
        BinaryOpKind::And => "and",
        BinaryOpKind::Or => "or",
        BinaryOpKind::Add => "add",
        BinaryOpKind::Subtract => "sub",
        BinaryOpKind::Multiply => "mul",
        BinaryOpKind::Divide => "div",
        BinaryOpKind::Shl => "shl",
        BinaryOpKind::Shr => "shr",
        _ => "?bin",
    }
}

fn un_name(op: UnaryOpKind) -> &'static str {
    match op {
        UnaryOpKind::BitNot => "bit_not",
        UnaryOpKind::ReduceAnd => "reduce_and",
        UnaryOpKind::ReduceOr => "reduce_or",
        UnaryOpKind::ReduceXor => "reduce_xor",
    }
}

/// Parse a single expression inside a synthetic `## Hardware` block and
/// return its canonical Lisp-style description. Bails on parse error or
/// unexpected token stream shape.
fn parse_hw_expr(expr_text: &str) -> String {
    let source = format!("## Hardware\n{}\n", expr_text);

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();

    let ast_ctx = AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut interner = Interner::new();
    let mut world_state = WorldState::new();

    let tokens = {
        let mut lexer = Lexer::new(&source, &mut interner);
        lexer.tokenize()
    };

    let expr: &Expr = {
        let mut parser = Parser::new(
            tokens,
            &mut world_state,
            &mut interner,
            ast_ctx,
            TypeRegistry::default(),
        );
        parser.process_block_headers();
        assert!(
            parser.hw_context(),
            "process_block_headers should set hw_context=true after ## Hardware"
        );
        parser
            .parse_imperative_expr()
            .unwrap_or_else(|e| panic!("parse_imperative_expr failed for `{}`: {:?}", expr_text, e))
    };

    describe(expr, &interner)
}

/// Parse the same expression text from a `## Main` block — hw_context stays
/// false, so the HW operators must either fail to parse or fall back to the
/// legacy interpretation. Used for the cross-mode guard test.
fn parse_main_expr(expr_text: &str) -> Result<String, String> {
    let source = format!("## Main\n{}\n", expr_text);

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();

    let ast_ctx = AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut interner = Interner::new();
    let mut world_state = WorldState::new();

    let tokens = {
        let mut lexer = Lexer::new(&source, &mut interner);
        lexer.tokenize()
    };

    let outcome: Result<&Expr, String> = {
        let mut parser = Parser::new(
            tokens,
            &mut world_state,
            &mut interner,
            ast_ctx,
            TypeRegistry::default(),
        );
        parser.process_block_headers();
        assert!(
            !parser.hw_context(),
            "process_block_headers must leave hw_context=false for ## Main"
        );
        match parser.parse_imperative_expr() {
            Ok(expr) => Ok(expr),
            Err(e) => Err(format!("{:?}", e)),
        }
    };

    outcome.map(|e| describe(e, &interner))
}

// ═══════════════════════════════════════════════════════════════════════════
// Unary operators
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn reduce_or_is_unary() {
    assert_eq!(parse_hw_expr("|req"), "(reduce_or id(req))");
}

#[test]
fn reduce_and_is_unary() {
    assert_eq!(parse_hw_expr("&req"), "(reduce_and id(req))");
}

#[test]
fn reduce_xor_is_unary() {
    assert_eq!(parse_hw_expr("^data"), "(reduce_xor id(data))");
}

#[test]
fn bit_not_is_unary() {
    assert_eq!(parse_hw_expr("~enable"), "(bit_not id(enable))");
}

// ═══════════════════════════════════════════════════════════════════════════
// Precedence — `|` < `^` < `&`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn and_or_precedence_and_binds_tighter() {
    // `req & valid | grant` → `(req & valid) | grant`
    assert_eq!(
        parse_hw_expr("req & valid | grant"),
        "(bit_or (bit_and id(req) id(valid)) id(grant))"
    );
}

#[test]
fn unary_then_binary_or() {
    // `|a | b` → ReduceOr(a) | b
    assert_eq!(
        parse_hw_expr("|a | b"),
        "(bit_or (reduce_or id(a)) id(b))"
    );
}

#[test]
fn mixed_three_ops_full_precedence() {
    // `a | b & c ^ d` → `a | ((b & c) ^ d)` under `|` < `^` < `&`
    assert_eq!(
        parse_hw_expr("a | b & c ^ d"),
        "(bit_or id(a) (bit_xor (bit_and id(b) id(c)) id(d)))"
    );
}

#[test]
fn paren_override_groups_or_first() {
    // `(a | b) & c` → BitAnd(BitOr(a,b), c)
    assert_eq!(
        parse_hw_expr("(a | b) & c"),
        "(bit_and (bit_or id(a) id(b)) id(c))"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Nested unary
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn reduce_xor_of_reduce_or() {
    // `^|a` → ReduceXor(ReduceOr(a))
    assert_eq!(
        parse_hw_expr("^|a"),
        "(reduce_xor (reduce_or id(a)))"
    );
}

#[test]
fn bit_not_of_reduce_and() {
    // `~&req` → BitNot(ReduceAnd(req))
    assert_eq!(
        parse_hw_expr("~&req"),
        "(bit_not (reduce_and id(req)))"
    );
}

#[test]
fn bit_not_of_reduce_or() {
    // `~|req` → BitNot(ReduceOr(req))
    assert_eq!(
        parse_hw_expr("~|req"),
        "(bit_not (reduce_or id(req)))"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Bit-select / Part-select
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bit_select_single_index() {
    // `data[7]` → BitSelect(data, 7)
    assert_eq!(parse_hw_expr("data[7]"), "(bit_select id(data) lit)");
}

#[test]
fn part_select_range() {
    // `data[7:4]` → PartSelect(data, 7, 4)
    assert_eq!(
        parse_hw_expr("data[7:4]"),
        "(part_select id(data) lit lit)"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Concatenation
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn concat_three_identifiers() {
    // `{valid, ready, error}` → HwConcat([valid, ready, error])
    assert_eq!(
        parse_hw_expr("{valid, ready, error}"),
        "(concat id(valid) id(ready) id(error))"
    );
}

#[test]
fn concat_with_inner_bit_select() {
    // `{a[1], b}` → HwConcat([BitSelect(a, 1), b])
    assert_eq!(
        parse_hw_expr("{a[1], b}"),
        "(concat (bit_select id(a) lit) id(b))"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Cross-mode guard — the HW operators must not fire outside `## Hardware`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn hw_operators_do_not_fire_outside_hw_block() {
    // Inside a ## Main block, `|a` must NOT be parsed as ReduceOr.
    // Either the parser rejects the leading `|` or interprets it as something
    // legacy — but it must NOT yield a `reduce_or`.
    match parse_main_expr("|a") {
        Ok(description) => {
            assert!(
                !description.contains("reduce_or")
                    && !description.contains("bit_or")
                    && !description.contains("bit_and")
                    && !description.contains("bit_not"),
                "HW operator leaked into ## Main output: {}",
                description
            );
        }
        Err(_) => {
            // Acceptable: parser rejects the lone `|` outside a HW block.
        }
    }
}
