//! Phase 3 — HW-Spec preamble parser
//!
//! One test per shipped sigil (clock, reset, signals, let, parameters,
//! interfaces) asserting that the produced `HwPreamble` field equals a
//! hand-constructed literal via field-level equality. Plus a loud-failure
//! test proving the preamble parser surfaces a `Custom` error on a
//! malformed sigil such as `signls:` rather than silently reclassifying the
//! line as property text.
//!
//! `sequences:` is intentionally out of scope for phase 3 — its body
//! operators (`;`, `within N cycles`, `repeats N times`) are deferred to
//! V2 per the phase-3 pre-verification decision.

use logicaffeine_base::{Arena, Interner};
use logicaffeine_language::{
    Lexer, Parser,
    ast::Expr,
    drs::WorldState,
    arena_ctx::AstContext,
    analysis::TypeRegistry,
    parser::{
        parse_hw_preamble, ClockEdge, ClockRole, HwPreamble, HwPreambleResult,
        ResetPolarity, SignalType,
    },
};

// ═══════════════════════════════════════════════════════════════════════════
// Test harness: tokenize a synthetic ## Hardware block and drive the preamble
// parser directly.
// ═══════════════════════════════════════════════════════════════════════════

struct Harness {
    interner: Interner,
    world_state: WorldState,
}

impl Harness {
    fn new() -> Self {
        Self {
            interner: Interner::new(),
            world_state: WorldState::new(),
        }
    }

    fn sym(&mut self, s: &str) -> logicaffeine_base::Symbol {
        self.interner.intern(s)
    }
}

/// Parse the preamble and invoke the caller with the HwPreamble. The Parser
/// is dropped before the callback so the borrow of `Interner` is released.
fn with_preamble<R, F>(src: &str, f: F) -> R
where
    F: FnOnce(&HwPreamble<'_>, &Interner) -> R,
{
    let source = format!("## Hardware\n{}\n", src);

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

    let result: HwPreambleResult<'_> = {
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
            "preamble harness requires hw_context=true after ## Hardware"
        );
        parse_hw_preamble(&mut parser)
            .unwrap_or_else(|e| panic!("parse_hw_preamble failed for `{}`: {:?}", src, e))
    };

    f(&result.preamble, &interner)
}

/// Variant of [`with_preamble`] that expects a parse error and yields the
/// error message. Used for the loud-failure test.
fn preamble_error(src: &str) -> String {
    let source = format!("## Hardware\n{}\n", src);

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

    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        &mut interner,
        ast_ctx,
        TypeRegistry::default(),
    );
    parser.process_block_headers();
    match parse_hw_preamble(&mut parser) {
        Ok(r) => panic!(
            "expected parse error for `{}`, got preamble with {} signals / {} clocks",
            src,
            r.preamble.signals.len(),
            r.preamble.clocks.len()
        ),
        Err(e) => format!("{:?}", e),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// clock:
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn clock_sigil_records_single_clock() {
    with_preamble("clock: pclk", |preamble, interner| {
        assert_eq!(preamble.clocks.len(), 1, "expected one clock");
        let clk = &preamble.clocks[0];
        assert_eq!(interner.resolve(clk.name), "pclk");
        assert_eq!(clk.role, ClockRole::Primary);
        assert_eq!(clk.edge, ClockEdge::Posedge);
    });
}

#[test]
fn clocks_sigil_records_multiple_clocks_with_roles() {
    with_preamble("clocks: clk_a, clk_b", |preamble, interner| {
        assert_eq!(preamble.clocks.len(), 2);
        assert_eq!(interner.resolve(preamble.clocks[0].name), "clk_a");
        assert_eq!(preamble.clocks[0].role, ClockRole::Primary);
        assert_eq!(interner.resolve(preamble.clocks[1].name), "clk_b");
        assert_eq!(preamble.clocks[1].role, ClockRole::Secondary);
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// reset:
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn reset_sigil_default_polarity_is_active_high() {
    with_preamble("reset: rst", |preamble, interner| {
        assert_eq!(preamble.resets.len(), 1);
        let rst = &preamble.resets[0];
        assert_eq!(interner.resolve(rst.name), "rst");
        assert_eq!(rst.polarity, ResetPolarity::ActiveHigh);
    });
}

#[test]
fn reset_sigil_active_low_recorded() {
    with_preamble("reset: rst_n, active low", |preamble, interner| {
        assert_eq!(preamble.resets.len(), 1);
        let rst = &preamble.resets[0];
        assert_eq!(interner.resolve(rst.name), "rst_n");
        assert_eq!(rst.polarity, ResetPolarity::ActiveLow);
    });
}

#[test]
fn reset_sigil_active_high_explicit() {
    with_preamble("reset: rst, active high", |preamble, _| {
        assert_eq!(preamble.resets[0].polarity, ResetPolarity::ActiveHigh);
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// signals:
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn signals_block_scalar_and_bus() {
    let src = "signals:\n  PSEL : scalar\n  data : bus[31:0]";
    with_preamble(src, |preamble, interner| {
        assert_eq!(preamble.signals.len(), 2, "expected two signals");
        assert_eq!(interner.resolve(preamble.signals[0].name), "PSEL");
        assert_eq!(preamble.signals[0].ty, SignalType::Scalar);
        assert_eq!(interner.resolve(preamble.signals[1].name), "data");
        assert_eq!(
            preamble.signals[1].ty,
            SignalType::Bus { hi: 31, lo: 0 }
        );
    });
}

#[test]
fn signals_block_enum_registers_anonymous_type() {
    let src = "signals:\n  state : enum {IDLE, READ, WRITE}";
    with_preamble(src, |preamble, interner| {
        assert_eq!(preamble.signals.len(), 1);
        let state = &preamble.signals[0];
        assert_eq!(interner.resolve(state.name), "state");
        match &state.ty {
            SignalType::Enum(id) => {
                let values = preamble.types.enum_values(*id);
                assert_eq!(values.len(), 3, "enum must have three variants");
                assert_eq!(interner.resolve(values[0]), "IDLE");
                assert_eq!(interner.resolve(values[1]), "READ");
                assert_eq!(interner.resolve(values[2]), "WRITE");
            }
            other => panic!("expected SignalType::Enum, got {:?}", other),
        }
        assert_eq!(
            preamble.types.len(),
            1,
            "preamble's anonymous-enum type registry should hold exactly one enum"
        );
    });
}

#[test]
fn signals_block_populates_symbol_table() {
    let src = "signals:\n  a : scalar\n  b : bus[7:0]";
    with_preamble(src, |preamble, interner| {
        assert_eq!(preamble.symbols.len(), 2);
        let a_sym = preamble
            .symbols
            .iter()
            .find(|e| interner.resolve(e.name) == "a")
            .expect("signal `a` must be in the symbol table");
        assert_eq!(a_sym.ty, SignalType::Scalar);
        let b_sym = preamble
            .symbols
            .iter()
            .find(|e| interner.resolve(e.name) == "b")
            .expect("signal `b` must be in the symbol table");
        assert_eq!(b_sym.ty, SignalType::Bus { hi: 7, lo: 0 });
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// let bindings — parsed via parse_imperative_expr with hw_context
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn let_binding_with_reduction_or() {
    let src = "signals:\n  req : bus[3:0]\nlet any_req = |req";
    with_preamble(src, |preamble, interner| {
        assert_eq!(preamble.lets.len(), 1);
        let binding = &preamble.lets[0];
        assert_eq!(interner.resolve(binding.name), "any_req");
        match binding.rhs {
            Expr::UnaryOp { .. } => { /* shape is enough; phase 2 covers precedence */ }
            other => panic!("expected Expr::UnaryOp for `|req`, got {:?}", other),
        }
    });
}

#[test]
fn let_binding_with_bitwise_or() {
    let src = "signals:\n  grant : bus[3:0]\nlet active = grant[0] | grant[1]";
    with_preamble(src, |preamble, interner| {
        assert_eq!(preamble.lets.len(), 1);
        assert_eq!(interner.resolve(preamble.lets[0].name), "active");
        match preamble.lets[0].rhs {
            Expr::BinaryOp { op, .. } => {
                use logicaffeine_language::ast::BinaryOpKind;
                assert_eq!(*op, BinaryOpKind::BitOr);
            }
            other => panic!("expected BinaryOp, got {:?}", other),
        }
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// parameters:
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parameters_block_records_typed_parameters() {
    let src = "parameters:\n  WIDTH : scalar\n  DEPTH : bus[15:0]";
    with_preamble(src, |preamble, interner| {
        assert_eq!(preamble.parameters.len(), 2);
        assert_eq!(interner.resolve(preamble.parameters[0].name), "WIDTH");
        assert_eq!(preamble.parameters[0].ty, SignalType::Scalar);
        assert_eq!(interner.resolve(preamble.parameters[1].name), "DEPTH");
        assert_eq!(
            preamble.parameters[1].ty,
            SignalType::Bus { hi: 15, lo: 0 }
        );
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// interfaces:
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn interfaces_block_records_nested_fields() {
    let src = "interfaces:\n  AXIStream:\n    VALID : scalar\n    DATA : bus[31:0]";
    with_preamble(src, |preamble, interner| {
        assert_eq!(preamble.interfaces.len(), 1);
        let iface = &preamble.interfaces[0];
        assert_eq!(interner.resolve(iface.name), "AXIStream");
        assert_eq!(iface.fields.len(), 2);
        assert_eq!(interner.resolve(iface.fields[0].name), "VALID");
        assert_eq!(iface.fields[0].ty, SignalType::Scalar);
        assert_eq!(interner.resolve(iface.fields[1].name), "DATA");
        assert_eq!(iface.fields[1].ty, SignalType::Bus { hi: 31, lo: 0 });
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// Loud failure on malformed sigil
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn misspelled_sigil_is_loud_failure() {
    let msg = preamble_error("signls: foo");
    assert!(
        msg.contains("unknown HW-Spec preamble directive")
            && msg.contains("signls"),
        "expected a loud error on malformed sigil, got: {}",
        msg
    );
}

#[test]
fn random_word_not_followed_by_colon_ends_preamble_cleanly() {
    // If the first non-blank line of the block is not a sigil at all (no
    // trailing colon), the preamble parser should simply yield an empty
    // preamble and leave the tokens for the property-sentence parser.
    with_preamble("req is asserted", |preamble, _| {
        assert!(preamble.clocks.is_empty());
        assert!(preamble.resets.is_empty());
        assert!(preamble.signals.is_empty());
        assert!(preamble.lets.is_empty());
        assert!(preamble.parameters.is_empty());
        assert!(preamble.interfaces.is_empty());
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// Combined preamble — smoke test covering multiple sigils in one block
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn combined_preamble_populates_every_section() {
    let src = "clock: pclk
reset: preset_n, active low
signals:
  PSEL : scalar
  PADDR : bus[31:0]
let any_psel = |PSEL";
    with_preamble(src, |preamble, interner| {
        assert_eq!(preamble.clocks.len(), 1);
        assert_eq!(interner.resolve(preamble.clocks[0].name), "pclk");

        assert_eq!(preamble.resets.len(), 1);
        assert_eq!(preamble.resets[0].polarity, ResetPolarity::ActiveLow);

        assert_eq!(preamble.signals.len(), 2);
        assert_eq!(
            preamble.signals[1].ty,
            SignalType::Bus { hi: 31, lo: 0 }
        );

        assert_eq!(preamble.lets.len(), 1);
        assert_eq!(interner.resolve(preamble.lets[0].name), "any_psel");

        assert_eq!(preamble.symbols.len(), 2);
    });
}

// Silence warnings about unused helpers if any test is pruned later.
#[allow(dead_code)]
fn _keep_harness(_: Harness) {}
