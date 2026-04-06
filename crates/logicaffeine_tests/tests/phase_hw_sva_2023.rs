//! SVA 2023 Upgrade — IEEE 1800-2023 Compliance Tests
//!
//! Sprint-organized tests for upgrading from IEEE 1800-2017 to IEEE 1800-2023.
//! Spec: SVA_2023_UPGRADE.md. All existing 858 tests must pass unchanged.

use logicaffeine_compile::codegen_sva::sva_model::{
    parse_sva, parse_sva_directive, sva_expr_to_string, sva_exprs_structurally_equivalent,
    SvaExpr, SvaDirective, SvaDirectiveKind,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::{
    BoundedExpr, SvaTranslator, DirectiveRole,
};
use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 22: TRIPLE-QUOTED STRINGS & NEW SYSTEM TASKS (IEEE 5.9, 20.4, 20.17)
// ═══════════════════════════════════════════════════════════════════════════

// ── Triple-Quoted Strings in Action Blocks ──

#[test]
fn triple_quoted_basic() {
    // Triple-quoted string in action block should parse without error
    let d = parse_sva_directive(
        r#"assert property (req |-> ack) else $error("""hello""");"#
    ).unwrap();
    assert_eq!(d.kind, SvaDirectiveKind::Assert);
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("hello"), "action_fail should contain the string content. Got: {}", fail);
}

#[test]
fn triple_quoted_embedded_quote() {
    // Triple-quoted strings allow embedded " without escape
    let d = parse_sva_directive(
        r#"assert property (req) else $error("""say "hello" please""");"#
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains(r#""hello""#), "should contain embedded quotes. Got: {}", fail);
}

#[test]
fn triple_quoted_multiline() {
    // Triple-quoted string spanning multiple lines
    let input = "assert property (req) else $error(\"\"\"line1\nline2\nline3\"\"\");";
    let d = parse_sva_directive(input).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("line1") && fail.contains("line2") && fail.contains("line3"),
        "multiline triple-quoted should be preserved. Got: {}", fail);
}

#[test]
fn triple_quoted_with_escape_n() {
    // \n escape still works inside triple-quoted strings
    let d = parse_sva_directive(
        r#"assert property (req) else $error("""line1\nline2""");"#
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains(r"\n") || fail.contains("line1"),
        "escape sequences should be preserved in raw action block. Got: {}", fail);
}

#[test]
fn triple_quoted_with_escape_t() {
    // \t escape works inside triple-quoted strings
    let d = parse_sva_directive(
        r#"assert property (req) else $error("""col1\tcol2""");"#
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("col1"), "should contain content. Got: {}", fail);
}

#[test]
fn triple_quoted_with_escape_backslash() {
    // \\ escape works inside triple-quoted strings
    let d = parse_sva_directive(
        r#"assert property (req) else $error("""path\\to\\file""");"#
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("path"), "should contain content. Got: {}", fail);
}

#[test]
fn triple_quoted_empty() {
    // Empty triple-quoted string parses
    let d = parse_sva_directive(
        r#"assert property (req) else $error("""""");"#
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("$error"), "should have error task. Got: {}", fail);
}

#[test]
fn triple_quoted_single_char() {
    // Single character triple-quoted string
    let d = parse_sva_directive(
        r#"assert property (req) else $error("""x""");"#
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("x"), "should contain 'x'. Got: {}", fail);
}

#[test]
fn triple_quoted_ieee_example3() {
    // IEEE 1800-2023 p.81 Example 3: triple-quoted with embedded double quotes
    let d = parse_sva_directive(
        r#"assert property (req) $display("""Humpty Dumpty sat on a "wall".""");"#
    ).unwrap();
    let pass = d.action_pass.as_ref().expect("should have pass action");
    assert!(pass.contains("wall"), "IEEE Example 3 content. Got: {}", pass);
}

#[test]
fn triple_quoted_ieee_example4() {
    // IEEE p.81 Example 4: escaped newline in triple-quoted (joins lines)
    let d = parse_sva_directive(
        r#"assert property (req) $display("""Humpty Dumpty sat on a wall. \
Humpty Dumpty had a great fall. """);"#
    ).unwrap();
    let pass = d.action_pass.as_ref().expect("should have pass action");
    assert!(pass.contains("Humpty"), "IEEE Example 4 content. Got: {}", pass);
}

#[test]
fn triple_quoted_ieee_example5() {
    // IEEE p.81 Example 5: \n escape in triple-quoted
    let d = parse_sva_directive(
        r#"assert property (req) $display("""Humpty Dumpty \n sat on a wall.""");"#
    ).unwrap();
    let pass = d.action_pass.as_ref().expect("should have pass action");
    assert!(pass.contains("Humpty"), "IEEE Example 5 content. Got: {}", pass);
}

#[test]
fn triple_quoted_in_error_action() {
    // Full directive with triple-quoted string in $error action
    let d = parse_sva_directive(
        r#"assert property (req |-> ack) else $error("""assertion "req->ack" failed at cycle""");"#
    ).unwrap();
    assert_eq!(d.kind, SvaDirectiveKind::Assert);
    assert!(d.action_fail.is_some(), "should have fail action");
    assert!(d.action_pass.is_none(), "should not have pass action");
}

#[test]
fn triple_quoted_in_display_action() {
    // Triple-quoted in pass action with $display
    let d = parse_sva_directive(
        r#"assert property (req |-> ack) $display("""pass: "ok" done""");"#
    ).unwrap();
    assert!(d.action_pass.is_some(), "should have pass action");
}

#[test]
fn triple_quoted_roundtrip() {
    // Triple-quoted string preserved in action block content
    let d = parse_sva_directive(
        r#"assert property (req) else $error("""fail msg""");"#
    ).unwrap();
    let fail = d.action_fail.as_ref().unwrap();
    // The action block raw text should contain the triple-quoted content
    assert!(fail.contains("fail msg"), "action content should be preserved. Got: {}", fail);
}

#[test]
fn triple_quoted_vs_single_quoted() {
    // Both string forms coexist in same directive
    let d = parse_sva_directive(
        r#"assert property (req) $info("pass") else $error("""fail""");"#
    ).unwrap();
    assert!(d.action_pass.is_some(), "should have pass action (single-quoted)");
    assert!(d.action_fail.is_some(), "should have fail action (triple-quoted)");
}

#[test]
fn triple_quoted_not_boolean() {
    // Triple-quoted string in assertion boolean position is not valid SVA
    // Strings are not boolean expressions in SVA
    let result = parse_sva(r#""""hello""""#);
    assert!(result.is_err(), "Triple-quoted string should not parse as a boolean expression");
}

// ── New System Tasks: $timeunit, $timeprecision, $stacktrace ──

#[test]
fn timeunit_in_action_block() {
    // $timeunit should be accepted in action blocks
    let d = parse_sva_directive(
        "assert property (req |-> ack) else $timeunit;"
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("$timeunit"), "should contain $timeunit. Got: {}", fail);
}

#[test]
fn timeprecision_in_action_block() {
    // $timeprecision should be accepted in action blocks
    let d = parse_sva_directive(
        "assert property (req |-> ack) else $timeprecision;"
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("$timeprecision"), "should contain $timeprecision. Got: {}", fail);
}

#[test]
fn stacktrace_in_action_block() {
    // $stacktrace should be accepted in action blocks
    let d = parse_sva_directive(
        "assert property (req |-> ack) else $stacktrace;"
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("$stacktrace"), "should contain $stacktrace. Got: {}", fail);
}

#[test]
fn timeunit_with_arg() {
    // $timeunit with hierarchical identifier argument
    let d = parse_sva_directive(
        "assert property (req) else $timeunit(top.dut);"
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("$timeunit"), "should contain $timeunit. Got: {}", fail);
}

#[test]
fn timeprecision_with_arg() {
    // $timeprecision with hierarchical identifier argument
    let d = parse_sva_directive(
        "assert property (req) else $timeprecision(top.dut);"
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("$timeprecision"), "should contain $timeprecision. Got: {}", fail);
}

#[test]
fn timeunit_not_in_z3() {
    // $timeunit in action block should NOT affect Z3 translation
    let d = parse_sva_directive(
        "assert property (req |-> ack) else $timeunit;"
    ).unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate_directive(&d);
    // The property should translate normally; action block excluded from Z3
    assert_eq!(result.role, DirectiveRole::Check);
    // The bounded expression should be about req/ack, not about $timeunit
    assert!(!format!("{:?}", result.expr).contains("timeunit"),
        "Z3 encoding should not contain timeunit");
}

#[test]
fn timeprecision_not_in_z3() {
    let d = parse_sva_directive(
        "assert property (req |-> ack) else $timeprecision;"
    ).unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate_directive(&d);
    assert_eq!(result.role, DirectiveRole::Check);
    assert!(!format!("{:?}", result.expr).contains("timeprecision"),
        "Z3 encoding should not contain timeprecision");
}

#[test]
fn stacktrace_not_in_z3() {
    let d = parse_sva_directive(
        "assert property (req |-> ack) else $stacktrace;"
    ).unwrap();
    let mut translator = SvaTranslator::new(3);
    let result = translator.translate_directive(&d);
    assert_eq!(result.role, DirectiveRole::Check);
    assert!(!format!("{:?}", result.expr).contains("stacktrace"),
        "Z3 encoding should not contain stacktrace");
}

#[test]
fn new_tasks_roundtrip() {
    // All three new system tasks should round-trip in action blocks
    for task in &["$timeunit", "$timeprecision", "$stacktrace"] {
        let input = format!("assert property (req) else {};", task);
        let d = parse_sva_directive(&input).unwrap();
        let fail = d.action_fail.as_ref()
            .unwrap_or_else(|| panic!("{} should produce fail action", task));
        assert!(fail.contains(task),
            "{} should round-trip in action block. Got: {}", task, fail);
    }
}

#[test]
fn existing_tasks_unchanged() {
    // Existing system tasks still parse identically
    for task in &["$error(\"msg\")", "$info(\"msg\")", "$fatal(1, \"msg\")", "$warning(\"msg\")", "$display(\"msg\")"] {
        let input = format!("assert property (req) else {};", task);
        let d = parse_sva_directive(&input).unwrap();
        assert!(d.action_fail.is_some(),
            "Existing task {} should still parse. Got action_fail=None", task);
    }
}

#[test]
fn action_block_mixed_old_new() {
    // Old and new system tasks mixed in same directive
    let d = parse_sva_directive(
        r#"assert property (req |-> ack) $info("pass") else $stacktrace;"#
    ).unwrap();
    assert!(d.action_pass.is_some(), "should have pass action ($info)");
    assert!(d.action_fail.is_some(), "should have fail action ($stacktrace)");
}

#[test]
fn triple_quoted_in_cover_action() {
    // Cover property with triple-quoted action block
    let d = parse_sva_directive(
        r#"cover property (req ##1 ack) $display("""covered""");"#
    ).unwrap();
    assert_eq!(d.kind, SvaDirectiveKind::Cover);
    assert!(d.action_pass.is_some(), "cover should have pass action");
}

#[test]
fn triple_quoted_containing_else() {
    // Edge case: triple-quoted string contains the word "else" — parser must not
    // split the action block at this false "else"
    let d = parse_sva_directive(
        r#"assert property (req) $info("""pass else continued""") else $error("real fail");"#
    ).unwrap();
    let pass = d.action_pass.as_ref().expect("should have pass action");
    assert!(pass.contains("continued"),
        "triple-quoted containing 'else' should NOT split at inner else. Got pass: {}", pass);
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("real fail"),
        "actual else should produce correct fail action. Got fail: {}", fail);
}

#[test]
fn triple_quoted_containing_semicolon() {
    // Edge case: triple-quoted string contains semicolons
    let d = parse_sva_directive(
        r#"assert property (req) else $error("""failed; details; here""");"#
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("details"),
        "triple-quoted containing ';' should preserve content. Got: {}", fail);
}

#[test]
fn triple_quoted_containing_parens() {
    // Edge case: triple-quoted string contains parentheses
    let d = parse_sva_directive(
        r#"assert property (req) else $error("""failed (details) here""");"#
    ).unwrap();
    let fail = d.action_fail.as_ref().expect("should have fail action");
    assert!(fail.contains("details"),
        "triple-quoted containing parens should preserve content. Got: {}", fail);
}

#[test]
fn restrict_still_no_action() {
    // Restrict property still forbids action blocks in 2023 (IEEE 16.14.4)
    let result = parse_sva_directive(
        "restrict property (valid) $info(\"ok\");"
    );
    assert!(result.is_err(), "restrict property with action blocks should still be an error in 2023");
}

#[test]
fn sprint22_backwards_compat() {
    // Verify existing directive patterns still work
    let cases = vec![
        "assert property (@(posedge clk) req |=> ack);",
        "assume property (@(posedge clk) !rst);",
        "cover property (@(posedge clk) req ##1 ack);",
        "cover sequence (@(posedge clk) req ##1 ack);",
        "restrict property (@(posedge clk) valid);",
        "a1: assert property (req |-> ack);",
        "assert property (@(posedge clk) disable iff (rst) req |-> ack);",
    ];
    for case in cases {
        let result = parse_sva_directive(case);
        assert!(result.is_ok(), "Existing directive should still parse: '{}'. Error: {:?}", case, result.err());
    }

    // Verify existing expression parsing still works
    let exprs = vec![
        "not req", "req implies ack", "req iff ack",
        "always req", "s_always [2:5] req",
        "req until ack", "strong(req ##1 ack)",
        "$rose(clk)", "$stable(data)", "$countones(bus)",
    ];
    for expr_str in exprs {
        let result = parse_sva(expr_str);
        assert!(result.is_ok(), "Existing expression should still parse: '{}'. Error: {:?}", expr_str, result.err());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 23: ARRAY .map() AND type(this) (IEEE 7.12, 6.23)
// ═══════════════════════════════════════════════════════════════════════════

// ── ArrayMap AST Construction & Roundtrip ──

#[test]
fn array_map_construct_basic() {
    // Construct ArrayMap AST directly (like FieldAccess pattern)
    let expr = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::And(
            Box::new(SvaExpr::LocalVar("x".into())),
            Box::new(SvaExpr::Const(1, 32)),
        )),
    };
    assert!(matches!(expr, SvaExpr::ArrayMap { .. }));
}

#[test]
fn array_map_to_string() {
    let expr = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::LocalVar("x".into())),
    };
    let text = sva_expr_to_string(&expr);
    assert!(text.contains(".map"), "to_string should contain .map. Got: {}", text);
    assert!(text.contains("with"), "to_string should contain 'with'. Got: {}", text);
}

#[test]
fn array_map_roundtrip() {
    // Use Signal (not LocalVar) for the iterator reference, since the parser
    // has no semantic context to distinguish iterator vars from signals.
    let expr = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::Signal("x".into())),
    };
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(
        sva_exprs_structurally_equivalent(&expr, &reparsed),
        "ArrayMap must round-trip: '{}' → {:?} vs {:?}", text, expr, reparsed
    );
}

#[test]
fn array_map_structural_eq_same() {
    let a = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::LocalVar("x".into())),
    };
    let b = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::LocalVar("x".into())),
    };
    assert!(sva_exprs_structurally_equivalent(&a, &b), "Same ArrayMap should be structurally equal");
}

#[test]
fn array_map_structural_eq_different_iterator() {
    let a = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::LocalVar("x".into())),
    };
    let b = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "y".into(),
        with_expr: Box::new(SvaExpr::LocalVar("y".into())),
    };
    assert!(!sva_exprs_structurally_equivalent(&a, &b),
        "Different iterators should NOT be structurally equal");
}

#[test]
fn array_map_structural_eq_different_array() {
    let a = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::LocalVar("x".into())),
    };
    let b = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("B".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::LocalVar("x".into())),
    };
    assert!(!sva_exprs_structurally_equivalent(&a, &b),
        "Different arrays should NOT be structurally equal");
}

#[test]
fn array_map_with_add_expr() {
    // A.map(x) with (x + 1)
    let expr = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("data".into())),
        iterator: "elem".into(),
        with_expr: Box::new(SvaExpr::And(
            Box::new(SvaExpr::LocalVar("elem".into())),
            Box::new(SvaExpr::Const(1, 8)),
        )),
    };
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("data") && text.contains("map") && text.contains("with"),
        "Should render complete map expression. Got: {}", text);
}

#[test]
fn array_map_with_bitwise() {
    // A.map(x) with (x & 8'hFF)
    let expr = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::BitAnd(
            Box::new(SvaExpr::LocalVar("x".into())),
            Box::new(SvaExpr::Const(0xFF, 8)),
        )),
    };
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("map"), "Should contain .map. Got: {}", text);
}

#[test]
fn array_map_in_implication() {
    // req |-> A.map(x) with (x > 0)
    let map_expr = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("data".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::GreaterThan(
            Box::new(SvaExpr::LocalVar("x".into())),
            Box::new(SvaExpr::Const(0, 32)),
        )),
    };
    let impl_expr = SvaExpr::Implication {
        antecedent: Box::new(SvaExpr::Signal("req".into())),
        consequent: Box::new(map_expr),
        overlapping: true,
    };
    let text = sva_expr_to_string(&impl_expr);
    assert!(text.contains("|->") && text.contains("map"),
        "Implication with map should render correctly. Got: {}", text);
}

#[test]
fn array_map_translate_unsupported() {
    // ArrayMap should translate to Unsupported (unknown-size arrays)
    let expr = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::LocalVar("x".into())),
    };
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate_property(&expr);
    // The G-wrapped result may be the Unsupported itself or an And wrapping it
    let debug = format!("{:?}", result.expr);
    assert!(debug.contains("Unsupported"),
        "ArrayMap with unknown size should produce Unsupported. Got: {}", debug);
}

#[test]
fn array_map_unsupported_fails_closed() {
    // Unsupported should map to False in kernel encoding (fail closed)
    let unsupported = BoundedExpr::Unsupported("array map with unknown size".to_string());
    let kernel_term = encode_bounded_expr(&unsupported);
    // The kernel term should be False (fail closed)
    assert!(format!("{:?}", kernel_term).contains("False"),
        "Unsupported should encode to False (fail closed). Got: {:?}", kernel_term);
}

#[test]
fn array_map_parse_dot_map_syntax() {
    // Parser should handle A.map(x) with (x) syntax
    let result = parse_sva("A.map(x) with (x)");
    assert!(result.is_ok(), "A.map(x) with (x) should parse. Error: {:?}", result.err());
    let expr = result.unwrap();
    assert!(matches!(expr, SvaExpr::ArrayMap { .. }),
        "Should parse to ArrayMap variant. Got: {:?}", expr);
}

#[test]
fn array_map_parse_no_iterator() {
    // Default iterator name should be `item`
    let result = parse_sva("A.map() with (item)");
    assert!(result.is_ok(), "A.map() with (item) should parse. Error: {:?}", result.err());
}

#[test]
fn array_map_parse_complex_with() {
    // Complex with expression
    let result = parse_sva("data.map(d) with (d && valid)");
    assert!(result.is_ok(), "Complex map should parse. Error: {:?}", result.err());
}

#[test]
fn array_map_nested() {
    // Nested ArrayMap AST construction
    let inner_map = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::LocalVar("x".into())),
        iterator: "y".into(),
        with_expr: Box::new(SvaExpr::LocalVar("y".into())),
    };
    let outer_map = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(inner_map),
    };
    let text = sva_expr_to_string(&outer_map);
    // Should contain two .map calls
    assert!(text.matches("map").count() >= 2,
        "Nested map should render both .map calls. Got: {}", text);
}

#[test]
fn array_map_with_countones() {
    // $countones(A.map(x) with (x))
    let map_expr = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("bus".into())),
        iterator: "b".into(),
        with_expr: Box::new(SvaExpr::LocalVar("b".into())),
    };
    let expr = SvaExpr::CountOnes(Box::new(map_expr));
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("$countones") && text.contains("map"),
        "System function wrapping map should render. Got: {}", text);
}

#[test]
fn array_method_existing_unchanged() {
    // Existing bit-select should still work: sig[7]
    let expr = SvaExpr::BitSelect {
        signal: Box::new(SvaExpr::Signal("data".into())),
        index: Box::new(SvaExpr::Const(7, 32)),
    };
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("data") && text.contains("7"),
        "BitSelect should still render. Got: {}", text);
}

// ── TypeThis AST Construction & Roundtrip ──

#[test]
fn type_this_construct() {
    let expr = SvaExpr::TypeThis;
    assert!(matches!(expr, SvaExpr::TypeThis));
}

#[test]
fn type_this_to_string() {
    let text = sva_expr_to_string(&SvaExpr::TypeThis);
    assert_eq!(text, "type(this)", "TypeThis should render as 'type(this)'. Got: {}", text);
}

#[test]
fn type_this_roundtrip() {
    let expr = SvaExpr::TypeThis;
    let text = sva_expr_to_string(&expr);
    let reparsed = parse_sva(&text).unwrap();
    assert!(
        sva_exprs_structurally_equivalent(&expr, &reparsed),
        "TypeThis must round-trip: '{}' → {:?} vs {:?}", text, expr, reparsed
    );
}

#[test]
fn type_this_structural_eq() {
    assert!(sva_exprs_structurally_equivalent(&SvaExpr::TypeThis, &SvaExpr::TypeThis),
        "TypeThis should be structurally equal to itself");
}

#[test]
fn type_this_vs_signal() {
    // type(this) is NOT a Signal named "type"
    assert!(!sva_exprs_structurally_equivalent(
        &SvaExpr::TypeThis,
        &SvaExpr::Signal("type".into())
    ), "TypeThis should be distinct from Signal(\"type\")");
}

#[test]
fn type_this_translate_unsupported() {
    let expr = SvaExpr::TypeThis;
    let mut translator = SvaTranslator::new(5);
    let result = translator.translate_property(&expr);
    let debug = format!("{:?}", result.expr);
    assert!(debug.contains("Unsupported"),
        "TypeThis should produce Unsupported. Got: {}", debug);
}

#[test]
fn type_this_parse() {
    // Parser should handle type(this)
    let result = parse_sva("type(this)");
    assert!(result.is_ok(), "type(this) should parse. Error: {:?}", result.err());
    let expr = result.unwrap();
    assert!(matches!(expr, SvaExpr::TypeThis),
        "Should parse to TypeThis. Got: {:?}", expr);
}

#[test]
fn type_this_in_eq() {
    // type(this) == expected in expression context
    let expr = SvaExpr::Eq(
        Box::new(SvaExpr::TypeThis),
        Box::new(SvaExpr::Signal("expected_type".into())),
    );
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("type(this)") && text.contains("=="),
        "TypeThis in equality should render. Got: {}", text);
}

#[test]
fn sprint23_backwards_compat() {
    // Verify existing expression patterns still parse
    let exprs = vec![
        "not req", "req implies ack", "req iff ack",
        "always req", "s_always [2:5] req",
        "$rose(clk)", "$stable(data)", "$onehot(bus)",
        "req |-> ack", "req |=> ##1 ack",
        "strong(req ##1 ack)", "weak(req ##1 ack)",
    ];
    for expr_str in exprs {
        let result = parse_sva(expr_str);
        assert!(result.is_ok(), "Existing expression should still parse: '{}'. Error: {:?}", expr_str, result.err());
    }
}

#[test]
fn sprint23_new_variants_counted() {
    // Verify the new variants exist by constructing them
    let _map = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("A".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::LocalVar("x".into())),
    };
    let _type_this = SvaExpr::TypeThis;
    // If this compiles, the variants exist
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 24: RAND REAL / RAND CONST REAL (IEEE 17.7)
// ═══════════════════════════════════════════════════════════════════════════

use logicaffeine_compile::codegen_sva::sva_model::{CheckerDecl, RandVar, RandVarType};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedSort;

// ── RandVarType Construction ──

#[test]
fn rand_real_construct() {
    let rv = RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: false };
    assert!(!rv.is_const);
    assert!(matches!(rv.var_type, RandVarType::Real));
}

#[test]
fn rand_const_real_construct() {
    let rv = RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: true };
    assert!(rv.is_const);
    assert!(matches!(rv.var_type, RandVarType::Real));
}

#[test]
fn rand_real_vs_bit() {
    let real_rv = RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: false };
    let bit_rv = RandVar { name: "b".into(), var_type: RandVarType::BitVec(1), is_const: false };
    assert_ne!(real_rv.var_type, bit_rv.var_type,
        "Real and BitVec(1) should be distinct types");
}

#[test]
fn rand_real_vs_bitvec() {
    let real_rv = RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: false };
    let bv_rv = RandVar { name: "idx".into(), var_type: RandVarType::BitVec(6), is_const: false };
    assert_ne!(real_rv.var_type, bv_rv.var_type,
        "Real and BitVec(6) should be distinct types");
}

#[test]
fn existing_rand_bit_as_bitvec1() {
    // Existing rand bit should now be RandVarType::BitVec(1)
    let rv = RandVar { name: "d".into(), var_type: RandVarType::BitVec(1), is_const: true };
    assert!(rv.is_const);
    assert!(matches!(rv.var_type, RandVarType::BitVec(1)));
}

#[test]
fn existing_rand_bitvec6() {
    // Existing rand bit [5:0] should now be RandVarType::BitVec(6)
    let rv = RandVar { name: "idx".into(), var_type: RandVarType::BitVec(6), is_const: true };
    assert!(matches!(rv.var_type, RandVarType::BitVec(6)));
}

#[test]
fn existing_rand_const_bit_unchanged() {
    let rv = RandVar { name: "d".into(), var_type: RandVarType::BitVec(1), is_const: true };
    assert!(rv.is_const);
}

// ── Checker with rand real ──

#[test]
fn rand_real_checker_body() {
    let checker = CheckerDecl {
        name: "my_real_check".into(),
        ports: vec![],
        rand_vars: vec![
            RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: false },
        ],
        assertions: vec![],
    };
    assert_eq!(checker.rand_vars.len(), 1);
    assert!(matches!(checker.rand_vars[0].var_type, RandVarType::Real));
}

#[test]
fn rand_const_real_checker() {
    let checker = CheckerDecl {
        name: "const_real_check".into(),
        ports: vec![],
        rand_vars: vec![
            RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: true },
        ],
        assertions: vec![],
    };
    assert!(checker.rand_vars[0].is_const);
}

#[test]
fn rand_real_mixed_with_bit() {
    // Checker with both rand real and rand bit
    let checker = CheckerDecl {
        name: "mixed_check".into(),
        ports: vec![],
        rand_vars: vec![
            RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: false },
            RandVar { name: "b".into(), var_type: RandVarType::BitVec(1), is_const: false },
            RandVar { name: "idx".into(), var_type: RandVarType::BitVec(6), is_const: true },
        ],
        assertions: vec![],
    };
    assert_eq!(checker.rand_vars.len(), 3);
    assert!(matches!(checker.rand_vars[0].var_type, RandVarType::Real));
    assert!(matches!(checker.rand_vars[1].var_type, RandVarType::BitVec(1)));
    assert!(matches!(checker.rand_vars[2].var_type, RandVarType::BitVec(6)));
}

// ── checker_quantifier_structure ──

#[test]
fn rand_real_quantifier_structure() {
    use logicaffeine_compile::codegen_sva::sva_model::checker_quantifier_structure;
    let checker = CheckerDecl {
        name: "quant_check".into(),
        ports: vec![],
        rand_vars: vec![
            RandVar { name: "r_const".into(), var_type: RandVarType::Real, is_const: true },
            RandVar { name: "r_nonconst".into(), var_type: RandVarType::Real, is_const: false },
            RandVar { name: "b_const".into(), var_type: RandVarType::BitVec(1), is_const: true },
        ],
        assertions: vec![],
    };
    let (const_vars, nonconst_vars) = checker_quantifier_structure(&checker);
    assert_eq!(const_vars.len(), 2, "Should have 2 const vars (r_const + b_const)");
    assert_eq!(nonconst_vars.len(), 1, "Should have 1 nonconst var (r_nonconst)");
}

#[test]
fn rand_real_no_width() {
    // Real variant has no width parameter
    let rv = RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: false };
    // Can't access .width anymore — it doesn't exist on RandVarType::Real
    match &rv.var_type {
        RandVarType::Real => {} // no width field
        RandVarType::BitVec(w) => panic!("Expected Real, got BitVec({})", w),
    }
}

// ── BoundedSort::Real ──

#[test]
fn bounded_sort_real_exists() {
    let sort = BoundedSort::Real;
    assert!(matches!(sort, BoundedSort::Real));
}

#[test]
fn bounded_sort_real_distinct_from_int() {
    assert_ne!(BoundedSort::Real, BoundedSort::Int);
}

#[test]
fn bounded_sort_real_distinct_from_bitvec() {
    assert_ne!(BoundedSort::Real, BoundedSort::BitVec(64));
}

// ── Real Literal Parsing ──

#[test]
fn real_literal_decimal() {
    // 1.5 should parse as a real literal (or at least not crash)
    // Real literals appear in checker assume constraints: assume (r > 1.5)
    let result = parse_sva("1.5");
    // May parse as a real const or may need explicit support
    assert!(result.is_ok(), "Real literal 1.5 should parse. Error: {:?}", result.err());
}

#[test]
fn real_literal_scientific() {
    let result = parse_sva("1.2E3");
    assert!(result.is_ok(), "Scientific notation 1.2E3 should parse. Error: {:?}", result.err());
}

#[test]
fn real_literal_negative_exp() {
    let result = parse_sva("1.30e-2");
    assert!(result.is_ok(), "Negative exponent 1.30e-2 should parse. Error: {:?}", result.err());
}

// ── VerifyType::Real ──

#[test]
fn verify_type_real_exists() {
    use logicaffeine_verify::ir::VerifyType;
    let t = VerifyType::Real;
    assert!(matches!(t, VerifyType::Real));
}

#[test]
fn verify_type_real_distinct() {
    use logicaffeine_verify::ir::VerifyType;
    assert_ne!(VerifyType::Real, VerifyType::Int);
    assert_ne!(VerifyType::Real, VerifyType::Bool);
}

// ── Sprint 24 Backwards Compatibility ──

#[test]
fn sprint24_backwards_compat() {
    // Verify existing expression parsing still works
    let exprs = vec![
        "not req", "req implies ack", "req iff ack",
        "always req", "s_always [2:5] req",
        "$rose(clk)", "$stable(data)", "$onehot(bus)",
        "req |-> ack", "strong(req ##1 ack)",
        "type(this)", // Sprint 23
    ];
    for expr_str in exprs {
        let result = parse_sva(expr_str);
        assert!(result.is_ok(), "Existing expression should still parse: '{}'. Error: {:?}", expr_str, result.err());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 25: SEMANTIC AUDIT & CROSS-FEATURE COMPOSITION (IEEE 2023 Errata)
// ═══════════════════════════════════════════════════════════════════════════

// ── 2-State Operator Truth Table Audit ──

#[test]
fn audit_eq_2state() {
    // In 2-state formal, == is standard boolean equality. No X/Z ambiguity.
    let expr = parse_sva("req == ack").unwrap();
    assert!(matches!(expr, SvaExpr::Eq(_, _)),
        "== should parse to Eq in 2-state. Got: {:?}", expr);
    // Verify translation produces standard equality
    let mut t = SvaTranslator::new(3);
    let result = t.translate_property(&expr);
    let debug = format!("{:?}", result.expr);
    assert!(debug.contains("Eq") || debug.contains("eq"),
        "2-state == should translate to equality. Got: {}", debug);
}

#[test]
fn audit_neq_2state() {
    let expr = parse_sva("req != ack").unwrap();
    assert!(matches!(expr, SvaExpr::NotEq(_, _)),
        "!= should parse to NotEq. Got: {:?}", expr);
}

#[test]
fn audit_lt_2state() {
    let expr = parse_sva("a < b").unwrap();
    assert!(matches!(expr, SvaExpr::LessThan(_, _)),
        "< should parse to LessThan. Got: {:?}", expr);
}

#[test]
fn audit_gt_2state() {
    let expr = parse_sva("a > b").unwrap();
    assert!(matches!(expr, SvaExpr::GreaterThan(_, _)),
        "> should parse to GreaterThan. Got: {:?}", expr);
}

#[test]
fn audit_lte_2state() {
    let expr = parse_sva("a <= b").unwrap();
    assert!(matches!(expr, SvaExpr::LessEqual(_, _)),
        "<= should parse to LessEqual. Got: {:?}", expr);
}

#[test]
fn audit_gte_2state() {
    let expr = parse_sva("a >= b").unwrap();
    assert!(matches!(expr, SvaExpr::GreaterEqual(_, _)),
        ">= should parse to GreaterEqual. Got: {:?}", expr);
}

#[test]
fn audit_isunknown_still_false() {
    // $isunknown always returns false in 2-state formal — no X/Z values
    let expr = parse_sva("$isunknown(sig)").unwrap();
    let mut t = SvaTranslator::new(3);
    let result = t.translate_property(&expr);
    let debug = format!("{:?}", result.expr);
    // In 2-state, $isunknown translates to Bool(false) or equivalent
    assert!(debug.contains("Bool(false)") || debug.contains("false"),
        "$isunknown should be false in 2-state. Got: {}", debug);
}

// ── Cross-Feature Composition ──

#[test]
fn cross_triple_quoted_with_real_checker() {
    // Triple-quoted string in action block + rand real in same checker context
    let d = parse_sva_directive(
        r#"assert property (req |-> ack) else $error("""assertion failed""");"#
    ).unwrap();
    assert!(d.action_fail.is_some());
    // rand real checker also works
    let checker = CheckerDecl {
        name: "cross_check".into(),
        ports: vec![],
        rand_vars: vec![
            RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: false },
        ],
        assertions: vec![d],
    };
    assert_eq!(checker.assertions.len(), 1);
    assert!(matches!(checker.rand_vars[0].var_type, RandVarType::Real));
}

#[test]
fn cross_map_with_temporal() {
    // s_eventually combined with ArrayMap
    let map_expr = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("data".into())),
        iterator: "x".into(),
        with_expr: Box::new(SvaExpr::GreaterThan(
            Box::new(SvaExpr::LocalVar("x".into())),
            Box::new(SvaExpr::Const(0, 32)),
        )),
    };
    let temporal = SvaExpr::SEventually(Box::new(map_expr));
    let text = sva_expr_to_string(&temporal);
    assert!(text.contains("s_eventually") && text.contains("map"),
        "Temporal + map should compose. Got: {}", text);
}

#[test]
fn cross_type_this_with_map() {
    // TypeThis and ArrayMap in same expression (via equality)
    let expr = SvaExpr::Eq(
        Box::new(SvaExpr::TypeThis),
        Box::new(SvaExpr::ArrayMap {
            array: Box::new(SvaExpr::Signal("types".into())),
            iterator: "t".into(),
            with_expr: Box::new(SvaExpr::Signal("t".into())),
        }),
    );
    let text = sva_expr_to_string(&expr);
    assert!(text.contains("type(this)") && text.contains("map"),
        "TypeThis + map should compose. Got: {}", text);
}

#[test]
fn cross_real_with_local_var() {
    // rand real value captured in local variable pattern
    let rv = RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: true };
    let seq_action = SvaExpr::SequenceAction {
        expression: Box::new(SvaExpr::Signal("valid".into())),
        assignments: vec![
            ("captured_r".into(), Box::new(SvaExpr::Signal("r".into()))),
        ],
    };
    let text = sva_expr_to_string(&seq_action);
    assert!(text.contains("captured_r"), "Local var should be in output. Got: {}", text);
    assert!(matches!(rv.var_type, RandVarType::Real));
}

#[test]
fn cross_all_2023_compose() {
    // Single test exercising all Sprint 22-24 features together
    // 1. Triple-quoted string in action block
    let d = parse_sva_directive(
        r#"assert property (req |-> ack) else $error("""2023 test""");"#
    ).unwrap();
    assert!(d.action_fail.is_some());
    // 2. ArrayMap variant
    let map = SvaExpr::ArrayMap {
        array: Box::new(SvaExpr::Signal("bus".into())),
        iterator: "b".into(),
        with_expr: Box::new(SvaExpr::Signal("b".into())),
    };
    let _ = sva_expr_to_string(&map);
    // 3. TypeThis variant
    let _ = sva_expr_to_string(&SvaExpr::TypeThis);
    // 4. rand real
    let _rv = RandVar { name: "r".into(), var_type: RandVarType::Real, is_const: false };
    // 5. Real literal
    let real = parse_sva("1.5").unwrap();
    assert!(matches!(real, SvaExpr::RealConst(_)));
    // 6. BoundedSort::Real
    let _sort = BoundedSort::Real;
    // 7. VerifyType::Real
    use logicaffeine_verify::ir::VerifyType;
    let _vt = VerifyType::Real;
}

#[test]
fn full_variant_count() {
    // Verify all new SvaExpr variants exist by constructing each one
    let variants: Vec<SvaExpr> = vec![
        SvaExpr::ArrayMap {
            array: Box::new(SvaExpr::Signal("A".into())),
            iterator: "x".into(),
            with_expr: Box::new(SvaExpr::Signal("x".into())),
        },
        SvaExpr::TypeThis,
        SvaExpr::RealConst(1.5),
    ];
    assert_eq!(variants.len(), 3, "Should have 3 new 2023 variants");
}

#[test]
fn sprint25_full_regression() {
    // Comprehensive backwards compatibility — verify key patterns from all prior sprints
    // Sprint 1: Property connectives
    assert!(parse_sva("not req").is_ok());
    assert!(parse_sva("req implies ack").is_ok());
    assert!(parse_sva("req iff ack").is_ok());
    // Sprint 2: LTL temporal
    assert!(parse_sva("always req").is_ok());
    assert!(parse_sva("req until ack").is_ok());
    // Sprint 3: Strong/weak
    assert!(parse_sva("strong(req ##1 ack)").is_ok());
    // Sprint 7: Directives
    assert!(parse_sva_directive("assert property (req |-> ack);").is_ok());
    assert!(parse_sva_directive("assume property (valid);").is_ok());
    assert!(parse_sva_directive("cover property (req ##1 ack);").is_ok());
    // Sprint 13: System functions
    assert!(parse_sva("$onehot(bus)").is_ok());
    assert!(parse_sva("$countones(bus)").is_ok());
    // Sprint 22: Triple-quoted strings
    assert!(parse_sva_directive(
        r#"assert property (req) else $error("""fail""");"#
    ).is_ok());
    // Sprint 23: ArrayMap
    assert!(parse_sva("A.map(x) with (x)").is_ok());
    assert!(parse_sva("type(this)").is_ok());
    // Sprint 24: Real literals
    assert!(parse_sva("1.5").is_ok());
}
