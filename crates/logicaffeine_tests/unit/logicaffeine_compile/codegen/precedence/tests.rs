use super::*;

#[test]
fn each_outermost_operator_is_read_at_depth_zero() {
    let cases: &[(&str, Binding)] = &[
        ("x", Binding::Atom),
        ("31i64", Binding::Atom),
        ("1.5e-3", Binding::Atom),
        ("arr.len()", Binding::Atom),
        ("(a + b)", Binding::Atom),
        ("v[i as usize]", Binding::Atom),
        ("logos_add_i64(n, 1)", Binding::Atom),
        ("format!(\"{}{}\", a, b)", Binding::Atom),
        ("Vec::<i64>::with_capacity(n as usize)", Binding::Atom),
        ("Vec::<Vec<i64>>::new()", Binding::Atom),
        ("Point { x: 1, y: 2 }", Binding::Atom),
        ("{ let t = a; t + 1 }", Binding::Atom),
        ("(|| -> Option<i64> { Some(h.checked_mul(31i64)?) })()", Binding::Atom),
        ("r#loop", Binding::Atom),
        ("x.0", Binding::Atom),
        ("foo()?", Binding::Atom),
        ("-x", Binding::Prefix),
        ("!flag", Binding::Prefix),
        ("*arr.get_unchecked(i)", Binding::Prefix),
        ("&mut v[..]", Binding::Prefix),
        ("&&x", Binding::Prefix),
        ("x as i64", Binding::Cast),
        ("x as i64 as f64", Binding::Cast),
        ("a * b", Binding::Product),
        ("a % 7", Binding::Product),
        ("a + b * c", Binding::Sum),
        ("i - 1", Binding::Sum),
        ("x as i64 - 1", Binding::Sum),
        ("1 << n", Binding::Shift),
        ("a & 1023", Binding::BitAnd),
        ("a ^ b", Binding::BitXor),
        ("a | b", Binding::BitOr),
        ("i < n", Binding::Compare),
        ("(a + b) <= c", Binding::Compare),
        ("a == 'x'", Binding::Compare),
        ("c == '\\''", Binding::Compare),
        ("a && b", Binding::And),
        ("a || b && c", Binding::Or),
        ("0..n", Binding::Loosest),
        ("1..=n", Binding::Loosest),
        ("x = 1", Binding::Loosest),
        ("move |x| x + 1", Binding::Loosest),
        ("|x: i64| x * 2", Binding::Loosest),
        ("|| 1", Binding::Loosest),
        ("return x", Binding::Loosest),
        ("if a { 1 } else { 2 }", Binding::Loosest),
        ("match x { Some(v) => v, _ => 0 }", Binding::Loosest),
        ("unsafe { *x.get_unchecked(0) }", Binding::Loosest),
        ("x + if a { 1 } else { 2 }", Binding::Sum),
        ("\"a + b\" == s", Binding::Compare),
        ("s == \"(\"", Binding::Compare),
        ("b\"x\"", Binding::Atom),
    ];
    for (code, want) in cases {
        assert_eq!(binding_of(code), *want, "binding of `{code}`");
    }
}

#[test]
fn an_unbalanced_expression_is_never_trusted_bare() {
    assert_eq!(binding_of("(a + b"), Binding::Loosest);
    assert_eq!(binding_of("a + b)"), Binding::Loosest);
    assert_eq!(binding_of(""), Binding::Loosest);
}

#[test]
fn an_operand_is_parenthesized_exactly_where_rust_would_read_it_otherwise() {
    use Side::{Left, Right};
    let cases: &[(&str, Under, &str)] = &[
        ("a + b", Under::Binary("*", Left), "(a + b)"),
        ("a * b", Under::Binary("+", Left), "a * b"),
        ("a + b", Under::Binary("+", Left), "a + b"),
        ("a + b", Under::Binary("-", Right), "(a + b)"),
        ("a * b", Under::Binary("-", Right), "a * b"),
        ("a < b", Under::Binary("==", Left), "(a < b)"),
        ("a + 1", Under::Binary("<=", Right), "a + 1"),
        ("x as i64", Under::Binary("<", Left), "(x as i64)"),
        ("x as u64", Under::Binary("<<", Left), "(x as u64)"),
        ("x as i64", Under::Binary("<=", Left), "(x as i64)"),
        ("x as i64", Under::Binary(">", Left), "x as i64"),
        ("x as i64", Under::Binary("<", Right), "x as i64"),
        ("x as i64", Under::Binary("+", Left), "x as i64"),
        ("a - *x as i64", Under::Binary("<<", Left), "(a - *x as i64)"),
        ("p | q as i64", Under::Binary("<", Left), "(p | q as i64)"),
        ("a as i64 - b", Under::Binary("<", Left), "a as i64 - b"),
        ("v as Vec<i64>", Under::Binary("<=", Left), "(v as Vec<i64>)"),
        ("i - 1", Under::Cast, "(i - 1)"),
        ("x as i64", Under::Cast, "x as i64"),
        ("-x", Under::Cast, "-x"),
        ("a + b", Under::Prefix, "(a + b)"),
        ("x.abs()", Under::Prefix, "x.abs()"),
        ("-1", Under::Postfix, "(-1)"),
        ("a + b", Under::Postfix, "(a + b)"),
        ("x as f64", Under::Postfix, "(x as f64)"),
        ("v[i]", Under::Postfix, "v[i]"),
        ("a && b", Under::Binary("||", Left), "a && b"),
        ("a || b", Under::Binary("&&", Right), "(a || b)"),
        ("move |x| x", Under::Binary("+", Left), "(move |x| x)"),
        ("unsafe { *p }", Under::Binary("<=", Left), "(unsafe { *p })"),
        ("1 << n", Under::Binary("-", Left), "(1 << n)"),
        ("h * 31", Under::Binary("&", Left), "h * 31"),
    ];
    for (code, under, want) in cases {
        assert_eq!(&*operand(code, *under), *want, "`{code}` under {under:?}");
    }
}

#[test]
fn enclosing_parentheses_come_off_a_whole_expression() {
    assert_eq!(unparenthesized("((n < 2))"), "n < 2");
    assert_eq!(unparenthesized("(n < 2)"), "n < 2");
    assert_eq!(unparenthesized("(a) + (b)"), "(a) + (b)");
    assert_eq!(unparenthesized("(a + b) * (c + d)"), "(a + b) * (c + d)");
    assert_eq!(unparenthesized("()"), "()");
    assert_eq!(unparenthesized("(a, b)"), "(a, b)");
    assert_eq!(unparenthesized("(x,)"), "(x,)");
    assert_eq!(unparenthesized(" (x) "), "x");
    assert_eq!(standalone("(logos_mul_i64(n, 6))"), "logos_mul_i64(n, 6)");
}

#[test]
fn a_condition_keeps_parentheses_only_where_a_brace_would_open_the_body() {
    assert_eq!(condition("(n < 2)"), "n < 2");
    assert_eq!(condition("((i > 0) && ((seed >> 28) == 0))"), "(i > 0) && ((seed >> 28) == 0)");
    assert_eq!(
        condition("(unsafe { *flags.get_unchecked(i) } == false)"),
        "(unsafe { *flags.get_unchecked(i) } == false)"
    );
    assert_eq!(condition("p == Point { x: 1, y: 2 }"), "(p == Point { x: 1, y: 2 })");
    assert_eq!(condition("(v.len() > { 3 })"), "(v.len() > { 3 })");
    assert_eq!(condition("logos_truthy(&(isSpaceByte(bt)))"), "logos_truthy(&(isSpaceByte(bt)))");
}

#[test]
fn an_assignment_of_the_target_under_an_operator_is_that_operator_assigning() {
    assert_eq!(assignment("sum", "sum + i", true), "sum += i");
    assert_eq!(assignment("m", "(m + a * b)", true), "m += a * b");
    assert_eq!(assignment("h", "h & 255", true), "h &= 255");
    assert_eq!(assignment("x", "x + y as i64", true), "x += y as i64");
    assert_eq!(assignment("(*x)", "(*x) - 1", true), "(*x) -= 1");
    assert_eq!(assignment("s", "s << 2", true), "s <<= 2");
}

#[test]
fn an_assignment_whose_value_only_starts_with_the_target_stays_one() {
    assert_eq!(assignment("a", "a - b + c", true), "a = a - b + c");
    assert_eq!(assignment("a", "a + b + c", true), "a = a + b + c");
    assert_eq!(assignment("x", "xy + 1", true), "x = xy + 1");
    assert_eq!(assignment("x", "x && y", true), "x = x && y");
    assert_eq!(assignment("x", "x == y", true), "x = x == y");
    assert_eq!(assignment("x", "y + x", true), "x = y + x");
    assert_eq!(assignment("x", "x.len() as i64", true), "x = x.len() as i64");
}

#[test]
fn a_block_holding_an_expression_alone_stands_without_its_braces() {
    assert_eq!(standalone("{ if c { a } else { b } }"), "if c { a } else { b }");
    assert_eq!(standalone("({ x + 1 })"), "x + 1");
    assert_eq!(standalone("{ (x) }"), "x");
    assert_eq!(standalone("{ match v { Some(w) => w & 1, _ => 0 } }"), "match v { Some(w) => w & 1, _ => 0 }");
    assert_eq!(standalone("{ match v { _ => 1 } as u8 }"), "match v { _ => 1 } as u8");
    assert_eq!(standalone("{ Point { x: 1 }.x }"), "Point { x: 1 }.x");
}

#[test]
fn a_block_with_a_statement_or_an_item_keeps_its_braces() {
    for code in [
        "{ let a = 1; a + 1 }",
        "{ f(); g() }",
        "{ fn h() {} h() }",
        "{ #[allow(unused)] x }",
        "{ 'outer: loop { break 'outer 1 } }",
        "{}",
        "{ a } + { b }",
        "unsafe { x }",
        "{ if b == 0 { return -1; }\n    a / b }",
        "{ while x < 3 { x += 1 } x }",
    ] {
        assert_eq!(standalone(code), code, "{code}");
    }
}

#[test]
fn a_returned_value_ends_the_body_as_written() {
    assert_eq!(tail("x").as_deref(), Some("x"));
    assert_eq!(tail("(a + b)").as_deref(), Some("a + b"));
    assert_eq!(tail("logos_add_i64(a, b)").as_deref(), Some("logos_add_i64(a, b)"));
    assert_eq!(tail("if c { a } else { b }").as_deref(), Some("if c { a } else { b }"));
    assert_eq!(tail("if c { a } else if d { b } else { e }").as_deref(), Some("if c { a } else if d { b } else { e }"));
    assert_eq!(tail("match v { Some(w) => w, None => 0 }").as_deref(), Some("match v { Some(w) => w, None => 0 }"));
    assert_eq!(tail("unsafe { *v.get_unchecked(0) }").as_deref(), Some("unsafe { *v.get_unchecked(0) }"));
    assert_eq!(tail("{ let a = f(); a + 1 }").as_deref(), Some("{ let a = f(); a + 1 }"));
    assert_eq!(tail("Point { x: 1, y: 2 }").as_deref(), Some("Point { x: 1, y: 2 }"));
}

#[test]
fn a_returned_value_led_by_a_block_like_expression_is_parenthesized() {
    assert_eq!(tail("match v { Some(w) => w, None => 0 } + 1").as_deref(), Some("(match v { Some(w) => w, None => 0 } + 1)"));
    assert_eq!(tail("unsafe { *v.get_unchecked(0) } * 2").as_deref(), Some("(unsafe { *v.get_unchecked(0) } * 2)"));
    assert_eq!(tail("if c { a } else { b } - 1").as_deref(), Some("(if c { a } else { b } - 1)"));
    assert_eq!(tail("{ f() } as i64").as_deref(), Some("({ f() } as i64)"));
    assert_eq!(tail("loop { break 1 }.max(0)").as_deref(), Some("(loop { break 1 }.max(0))"));
}

#[test]
fn a_returned_value_that_takes_a_guard_stays_a_return() {
    for code in [
        "mulAtv(n, &t.borrow(), result.clone())",
        "t.borrow_mut().pop()",
        "COUNTS.lock().unwrap().len() as i64",
        "v.drain(..).sum::<i64>()",
    ] {
        assert_eq!(tail(code), None, "{code}");
    }
}

#[test]
fn an_operand_keeps_the_parentheses_around_it_only_where_it_needs_them() {
    use Side::{Left, Right};
    let cases: &[(&str, Under, &str)] = &[
        ("(a * b)", Under::Binary("+", Left), "a * b"),
        ("((a * b))", Under::Binary("+", Right), "a * b"),
        ("(a + b)", Under::Binary("*", Left), "(a + b)"),
        ("((a + b))", Under::Binary("*", Right), "(a + b)"),
        ("(b - c)", Under::Binary("-", Right), "(b - c)"),
        ("(b - c)", Under::Binary("-", Left), "b - c"),
        ("((x))", Under::Postfix, "x"),
        ("((a + b))", Under::Postfix, "(a + b)"),
        ("(x as i64)", Under::Cast, "x as i64"),
        ("(x as i64)", Under::Binary("<", Left), "(x as i64)"),
        ("(a, b)", Under::Postfix, "(a, b)"),
        ("()", Under::Postfix, "()"),
    ];
    for (code, under, want) in cases {
        assert_eq!(&*operand(code, *under), *want, "`{code}` under {under:?}");
    }
}

#[test]
fn a_cast_is_written_whole_with_its_operand_parenthesized_only_where_needed() {
    assert_eq!(cast("x", "i64"), "(x as i64)");
    assert_eq!(cast("(x)", "u32"), "(x as u32)");
    assert_eq!(cast("((h))", "u32"), "(h as u32)");
    assert_eq!(cast("a + b", "f64"), "((a + b) as f64)");
    assert_eq!(cast("((a + b))", "f64"), "((a + b) as f64)");
    assert_eq!(cast("x as i32", "i64"), "(x as i32 as i64)");
    assert_eq!(cast("-x", "u64"), "(-x as u64)");
    assert_eq!(cast("unsafe { *p }", "i64"), "((unsafe { *p }) as i64)");
    assert_eq!(cast("arr[i as usize]", "i64"), "(arr[i as usize] as i64)");
}

#[test]
fn a_numeric_literal_a_cast_leaves_as_it_is_is_written_as_the_literal_of_the_type() {
    assert_eq!(cast("0f64", "f64"), "0f64");
    assert_eq!(cast("(2.5f64)", "f64"), "2.5f64");
    assert_eq!(cast("7", "u32"), "7u32");
    assert_eq!(cast("(7)", "u32"), "7u32");
    assert_eq!(cast("3", "f64"), "3f64");
    assert_eq!(cast("255", "u8"), "255u8");
    // What the cast would change, round or wrap stays a cast.
    assert_eq!(cast("-1", "u32"), "(-1 as u32)");
    assert_eq!(cast("256", "u8"), "(256 as u8)");
    assert_eq!(cast("1.5f64", "i64"), "(1.5f64 as i64)");
    assert_eq!(cast("1.5f32", "f64"), "(1.5f32 as f64)");
    assert_eq!(cast("9007199254740993", "f64"), "(9007199254740993 as f64)");
    assert_eq!(cast("16777217", "f32"), "(16777217 as f32)");
}

#[test]
fn an_unsafe_block_in_a_condition_needs_no_parentheses() {
    assert_eq!(condition("(c < unsafe { *counts.get_unchecked(v as usize) })"), "c < unsafe { *counts.get_unchecked(v as usize) }");
    assert_eq!(condition("unsafe { *p } < 3"), "unsafe { *p } < 3");
    assert_eq!(condition("(p == Point { x: 1 })"), "(p == Point { x: 1 })");
    assert_eq!(condition("({ a } < b)"), "({ a } < b)");
    assert_eq!(condition("(match x { _ => true })"), "(match x { _ => true })");
}

#[test]
fn an_assignment_operator_takes_its_value_bare() {
    assert_eq!(assignment("sum", "sum + (a * b)", true), "sum += a * b");
    assert_eq!(assignment("sum", "(sum + (a * b))", true), "sum += a * b");
    assert_eq!(assignment("x", "x - (a - b)", true), "x -= a - b");
    assert_eq!(assignment("e", "e - ((m * n) / d.sqrt())", true), "e -= (m * n) / d.sqrt()");
    assert_eq!(assignment("p.x", "(p.x * 2)", true), "p.x *= 2");
}

#[test]
fn a_block_moved_in_keeps_the_text_of_its_string_literals() {
    assert_eq!(indented("for i in 0..n {\n    f(i);\n}\n", "    "), "    for i in 0..n {\n        f(i);\n    }\n");
    assert_eq!(indented("a();\n\nb();", "  "), "  a();\n\n  b();", "an empty line stays empty");
    assert_eq!(indented("let s = \"x\ny\";\nz();\n", "    "), "    let s = \"x\ny\";\n    z();\n");
    assert_eq!(indented("let s = r#\"a\n\"b\"#;\nz();\n", "    "), "    let s = r#\"a\n\"b\"#;\n    z();\n");
    assert_eq!(indented("let s = br\"a\nb\";\n", "    "), "    let s = br\"a\nb\";\n");
    assert_eq!(indented("let s = \"q\\\"\nr\";\n", "    "), "    let s = \"q\\\"\nr\";\n", "an escaped quote");
    assert_eq!(indented("let c = '\"';\nlet d = 1;\n", "    "), "    let c = '\"';\n    let d = 1;\n");
    assert_eq!(indented("fn f<'a>(x: &'a str) {\n}\n", "    "), "    fn f<'a>(x: &'a str) {\n    }\n");
    assert_eq!(indented("// \"a quote\nx();\n", "    "), "    // \"a quote\n    x();\n", "a quote in a comment");
    assert_eq!(indented("/* \"\n */\nx();\n", "    "), "    /* \"\n     */\n    x();\n");
    assert_eq!(indented("let r#type = 1;\nx();\n", "    "), "    let r#type = 1;\n    x();\n", "a raw identifier");
}

/// A target whose type implements only the operators (a `Word32`) is assigned the whole value.
#[test]
fn an_assignment_without_the_assigning_operators_is_written_whole() {
    assert_eq!(assignment("w", "w ^ d", false), "w = w ^ d");
    assert_eq!(assignment("w", "(w + d)", false), "w = w + d");
}
