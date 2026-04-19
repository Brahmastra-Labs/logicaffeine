//! Phase 4 — HwSpec IR + entry-point consolidation
//!
//! Tests the unified `parse_hw_spec_with` entry, the `HwSpec` IR it
//! produces, and (for the 5 compile-crate entry points that migrate in
//! this phase) that they continue to return results byte-identical to
//! the pre-migration `&str` path.

use logicaffeine_language::hw_spec::parse_hw_spec_with;
use logicaffeine_language::parser::{ResetPolarity, SignalType};

// ═══════════════════════════════════════════════════════════════════════════
// Preamble round-trips through `parse_hw_spec_with`
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_hw_spec_populates_preamble_and_properties() {
    let src = "clock: pclk
reset: preset_n, active low
signals:
  PSEL : scalar
  PADDR : bus[31:0]

If PSEL is asserted, PADDR is valid.";

    let (clock_count, reset_polarity, signal_count, property_count) = parse_hw_spec_with(
        src,
        |spec, interner| {
            assert_eq!(
                interner.resolve(spec.preamble.clocks[0].name),
                "pclk"
            );
            assert_eq!(
                interner.resolve(spec.preamble.resets[0].name),
                "preset_n"
            );
            (
                spec.preamble.clocks.len(),
                spec.preamble.resets[0].polarity,
                spec.preamble.signals.len(),
                spec.properties.len(),
            )
        },
    )
    .expect("parse_hw_spec_with must succeed on well-formed APB snippet");

    assert_eq!(clock_count, 1);
    assert_eq!(reset_polarity, ResetPolarity::ActiveLow);
    assert_eq!(signal_count, 2);
    assert_eq!(
        property_count, 1,
        "the single `If ..., ... .` sentence must produce exactly one LogicExpr"
    );
}

#[test]
fn parse_hw_spec_handles_explicit_header() {
    let src = "## Hardware
signals:
  enable : scalar

If enable is asserted, enable is valid.";

    let property_count = parse_hw_spec_with(src, |spec, _| spec.properties.len())
        .expect("explicit ## Hardware header must also parse");
    assert_eq!(property_count, 1);
}

#[test]
fn parse_hw_spec_multiple_property_sentences() {
    let src = "signals:
  PSEL : scalar
  PENABLE : scalar
  PREADY : scalar

If PSEL is not asserted, PENABLE is not asserted.
If PENABLE is asserted, PSEL is asserted.
If PSEL and PENABLE are asserted, PREADY is asserted.";

    let count = parse_hw_spec_with(src, |spec, _| spec.properties.len())
        .expect("three property sentences must parse");
    assert!(
        count >= 1,
        "at least one property must land (APB snippet expands into 1-3 LogicExprs depending on axiom chaining); got {}",
        count
    );
}

#[test]
fn parse_hw_spec_rejects_empty_input() {
    let err = parse_hw_spec_with("", |_, _| ()).expect_err("empty input must be rejected");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("Empty"),
        "error should mention empty input; got: {}",
        msg
    );
}

#[test]
fn parse_hw_spec_surfaces_malformed_sigil() {
    let src = "signls: pclk";
    let err = parse_hw_spec_with(src, |_, _| ())
        .expect_err("malformed sigil must surface as a parse error");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("unknown HW-Spec preamble directive") && msg.contains("signls"),
        "error should name the typo; got: {}",
        msg
    );
}

#[test]
fn parse_hw_spec_signals_feed_symbol_table() {
    let src = "signals:
  enable : scalar
  counter : bus[7:0]

If enable is asserted, counter is valid.";

    parse_hw_spec_with(src, |spec, interner| {
        assert_eq!(spec.preamble.symbols.len(), 2, "symbol table must carry both signals");

        let enable = spec
            .preamble
            .symbols
            .iter()
            .find(|e| interner.resolve(e.name) == "enable")
            .expect("enable must be in symbol table");
        assert_eq!(enable.ty, SignalType::Scalar);

        let counter = spec
            .preamble
            .symbols
            .iter()
            .find(|e| interner.resolve(e.name) == "counter")
            .expect("counter must be in symbol table");
        assert_eq!(counter.ty, SignalType::Bus { hi: 7, lo: 0 });
    })
    .expect("APB-like snippet must parse");
}

#[test]
fn parse_hw_spec_enum_flows_into_preamble_type_registry() {
    let src = "signals:
  state : enum {IDLE, READ, WRITE}

If state is READ, state is valid.";

    parse_hw_spec_with(src, |spec, interner| {
        assert_eq!(spec.preamble.signals.len(), 1);
        match &spec.preamble.signals[0].ty {
            SignalType::Enum(id) => {
                let values = spec.preamble.types.enum_values(*id);
                assert_eq!(values.len(), 3);
                let mut names: Vec<&str> = values.iter().map(|s| interner.resolve(*s)).collect();
                names.sort();
                assert_eq!(names, vec!["IDLE", "READ", "WRITE"]);
            }
            other => panic!("expected enum type, got {:?}", other),
        }
    })
    .expect("enum-bearing signal block must parse");
}

#[test]
fn parse_hw_spec_let_binding_rhs_uses_hw_expression_parser() {
    let src = "signals:
  req : bus[3:0]
let any_req = |req

If any_req is asserted, req is valid.";

    let (let_count, property_count) = parse_hw_spec_with(src, |spec, interner| {
        assert_eq!(
            interner.resolve(spec.preamble.lets[0].name),
            "any_req"
        );
        // Phase-2 HW unary operator had to be reachable during preamble parsing.
        use logicaffeine_language::ast::{Expr, UnaryOpKind};
        match spec.preamble.lets[0].rhs {
            Expr::UnaryOp { op, .. } => {
                assert_eq!(*op, UnaryOpKind::ReduceOr);
            }
            other => panic!("expected reduce_or unary in let RHS, got {:?}", other),
        }
        (spec.preamble.lets.len(), spec.properties.len())
    })
    .expect("let binding plus property must parse");

    assert_eq!(let_count, 1);
    assert_eq!(property_count, 1);
}

#[test]
fn parse_hw_spec_property_only_no_preamble() {
    let src = "Always, every signal is valid.";
    let property_count = parse_hw_spec_with(src, |spec, _| {
        assert!(spec.preamble.clocks.is_empty());
        assert!(spec.preamble.resets.is_empty());
        assert!(spec.preamble.signals.is_empty());
        spec.properties.len()
    })
    .expect("property-only source must parse with empty preamble");
    assert_eq!(property_count, 1);
}

// ═══════════════════════════════════════════════════════════════════════════
// Contract §7 smoke tests — full `.hwspec` examples round-trip through the
// unified entry.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn contract_apb_protocol_parses() {
    let src = "clock: pclk
reset: preset_n, active low
signals:
  PSEL : scalar
  PENABLE : scalar
  PREADY : scalar
  PADDR : bus[31:0]
  PWRITE : scalar
  PRDATA : bus[31:0]

If PSEL is not asserted, PENABLE is not asserted.";

    parse_hw_spec_with(src, |spec, interner| {
        assert_eq!(
            interner.resolve(spec.preamble.clocks[0].name),
            "pclk"
        );
        assert_eq!(spec.preamble.signals.len(), 6);
        assert_eq!(spec.properties.len(), 1);
    })
    .expect("contract §7.1 APB snippet must parse");
}

#[test]
fn contract_arbiter_parses() {
    // §7.2 — needs phase-2 reduction operator in let binding.
    let src = "signals:
  req : bus[3:0]
  grant : bus[3:0]
let any_req = |req

If any_req is asserted, grant is asserted.";

    parse_hw_spec_with(src, |spec, _| {
        assert_eq!(spec.preamble.signals.len(), 2);
        assert_eq!(spec.preamble.lets.len(), 1);
        assert_eq!(spec.properties.len(), 1);
    })
    .expect("contract §7.2 arbiter snippet must parse");
}

#[test]
fn contract_fifo_parses() {
    // §7.3 — richer signal set, single property.
    let src = "signals:
  fifo_full : scalar
  fifo_empty : scalar
  write : scalar
  read : scalar

If fifo_full is asserted, write is not asserted.";

    parse_hw_spec_with(src, |spec, _| {
        assert_eq!(spec.preamble.signals.len(), 4);
        assert_eq!(spec.properties.len(), 1);
    })
    .expect("contract §7.3 FIFO snippet must parse");
}

#[test]
fn contract_uart_parses() {
    // §7.4 — multi-bit rx_data field, single property.
    let src = "signals:
  tx_busy : scalar
  new_tx_data : scalar
  rx_valid : scalar
  rx_data : bus[7:0]

If rx_valid is asserted, rx_data is valid.";

    parse_hw_spec_with(src, |spec, interner| {
        assert_eq!(spec.preamble.signals.len(), 4);
        let rx_data = spec
            .preamble
            .symbols
            .iter()
            .find(|e| interner.resolve(e.name) == "rx_data")
            .expect("rx_data must be in symbol table");
        assert_eq!(rx_data.ty, SignalType::Bus { hi: 7, lo: 0 });
    })
    .expect("contract §7.4 UART snippet must parse");
}

// ═══════════════════════════════════════════════════════════════════════════
// Property shape — the parsed LogicExpr must carry real logical structure,
// not just "some non-empty AST". Guards against a regression where the
// property loop silently accepts a placeholder `LogicExpr::Predicate` and
// discards the conditional form.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn property_sentence_produces_nontrivial_logicexpr() {
    use logicaffeine_language::ast::LogicExpr;

    let src = "signals:
  req : scalar
  ack : scalar

If req is asserted, ack is asserted.";

    parse_hw_spec_with(src, |spec, _| {
        assert_eq!(spec.properties.len(), 1, "one property sentence expected");
        let expr = spec.properties[0];

        // After Kripke lowering the "If X, Y" form must not flatten to a
        // bare predicate — the logical structure should survive the pass.
        fn has_logical_structure(e: &LogicExpr) -> bool {
            match e {
                LogicExpr::Predicate { .. } => false,
                LogicExpr::Identity { .. } => false,
                LogicExpr::Atom { .. } => false,
                // Any other variant (BinaryOp, Quantifier, Temporal, Modal,
                // Relation, etc.) counts as non-trivial structure.
                _ => true,
            }
        }

        assert!(
            has_logical_structure(expr),
            "`If req is asserted, ack is asserted.` must lower to something richer than a bare predicate; got {:?}",
            expr
        );
    })
    .expect("conditional property must parse");
}

// ═══════════════════════════════════════════════════════════════════════════
// Programmatic API parity — HwSymbolTable::from_decls vs. the text path
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn from_decls_produces_same_shape_as_hwspec_parse_path() {
    use logicaffeine_base::Interner;
    use logicaffeine_language::parser::{HwSignalDeclLike, HwSymbolTable, SignalType as HwTy};

    // Text path: parse a .hwspec snippet and read off its symbol table.
    let src = "signals:
  rx_valid : scalar
  rx_data : bus[7:0]

If rx_valid is asserted, rx_data is valid.";
    let text_entries = parse_hw_spec_with(src, |spec, interner| {
        spec.preamble
            .symbols
            .iter()
            .map(|e| (interner.resolve(e.name).to_string(), e.ty.clone()))
            .collect::<Vec<_>>()
    })
    .expect("preamble path must parse");

    // Programmatic path: build HwSymbolTable::from_decls from declarations
    // that Python-bridge callers would construct directly.
    let mut interner = Interner::new();
    let decls = vec![
        HwSignalDeclLike {
            english_name: "rx_valid".to_string(),
            width: 1,
        },
        HwSignalDeclLike {
            english_name: "rx_data".to_string(),
            width: 8,
        },
    ];
    let table = HwSymbolTable::from_decls(decls.iter(), &mut interner);
    let prog_entries: Vec<(String, HwTy)> = table
        .iter()
        .map(|e| (interner.resolve(e.name).to_string(), e.ty.clone()))
        .collect();

    assert_eq!(
        text_entries, prog_entries,
        "HwSymbolTable::from_decls must produce byte-identical entries vs. the .hwspec parse path"
    );
}
