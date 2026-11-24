//! A program's mutable binding is `mut` only where the emitted code writes it, or may.
use super::*;

/// `body` as a function whose binding `x` is declared mutable, settled.
fn settled(body: &str) -> String {
    settle_mutability(&format!("fn f() {{\n    let {MAYBE_MUT}x: i64 = 0;\n{body}}}\n"))
}

fn kept_mut(body: &str) -> bool {
    let code = settled(body);
    assert!(!code.contains("@maybe"), "the marker is gone: {code}");
    code.contains("let mut x")
}

#[test]
fn a_binding_only_read_is_not_mut() {
    assert!(!kept_mut("    show(&x);\n    let y: i64 = x + 1;\n"));
    assert!(!kept_mut("    if x == 0 { show(&(x as f64)); }\n"));
    assert!(!kept_mut("    let p = Point { x: x, y: 2 };\n"));
    assert!(!kept_mut("    for x in 0..3 { show(&x); }\n"));
}

#[test]
fn an_assignment_or_a_mutable_borrow_keeps_mut() {
    for body in [
        "    x = 5;\n",
        "    x += 1;\n",
        "    x <<= 2;\n",
        "    if c { x = x * 2; }\n",
        "    std::mem::swap(&mut x, &mut y);\n",
        "    let f = || x += 1;\n",
    ] {
        assert!(kept_mut(body), "{body}");
    }
}

#[test]
fn a_method_an_index_a_call_or_a_macro_argument_may_need_mut() {
    for body in ["    x.push(1);\n", "    x[0] = 1;\n", "    x(1);\n", "    write!(x, \"{}\", 1).unwrap();\n"] {
        assert!(kept_mut(body), "{body}");
    }
}

#[test]
fn an_assertion_reads_its_argument() {
    assert!(!kept_mut("    debug_assert!(x >= 0, \"in range\");\n    assert!(x < 10);\n"));
}

#[test]
fn a_name_inside_a_string_a_comment_or_a_field_is_not_the_binding() {
    assert!(!kept_mut("    show(&\"x = 1\");\n    // x = 2\n    p.x = 3;\n    q::x = 4;\n    let c = \'x\';\n"));
    assert!(!kept_mut("    show(&r#\"x += 1\"#);\n    /* x = 5 */\n"));
}

#[test]
fn a_write_after_the_scope_closes_belongs_to_another_binding() {
    let code = settle_mutability(&format!(
        "fn f() {{\n    {{\n        let {MAYBE_MUT}x: i64 = 0;\n        show(&x);\n    }}\n    x = 1;\n}}\n"
    ));
    assert!(code.contains("let x: i64 = 0;"), "{code}");
}

#[test]
fn the_initializer_reads_an_outer_binding_it_does_not_write() {
    let code = settle_mutability(&format!("fn f() {{\n    let {MAYBE_MUT}x: i64 = {{ x.max(1) }};\n}}\n"));
    assert!(code.contains("let x: i64"), "the initializer's `x` is the outer one: {code}");
}

#[test]
fn every_marked_binding_is_settled_on_its_own() {
    let code = settle_mutability(&format!(
        "fn f() {{\n    let {MAYBE_MUT}a: i64 = 0;\n    let {MAYBE_MUT}b: i64 = 0;\n    b += a;\n}}\n"
    ));
    assert!(code.contains("let a: i64") && code.contains("let mut b: i64"), "{code}");
}

/// `body` after a marked `let x: i64 = 0;`, settled: whether the `let` is kept.
fn literal_let_kept(body: &str) -> bool {
    let (code, _) = settle(&format!("fn f() {{\n    {MAYBE_UNREAD}let x: i64 = 0;\n{body}}}\n"));
    assert!(!code.contains("@unread"), "the marker is gone: {code}");
    code.contains("let x: i64 = 0;")
}

#[test]
fn a_literal_let_nothing_names_is_dropped() {
    assert!(!literal_let_kept("    show(&1);\n"));
    assert!(!literal_let_kept("    p.x = 2;\n    q::x();\n"));
}

#[test]
fn a_literal_let_its_scope_names_is_kept() {
    assert!(literal_let_kept("    show(&x);\n"));
    assert!(literal_let_kept("    println!(\"{x}\");\n"));
    assert!(literal_let_kept("    for i in 0..x { show(&i); }\n"));
    assert!(literal_let_kept("    {\n        let x: i64 = 5;\n        show(&x);\n    }\n    show(&x);\n"));
    assert!(literal_let_kept("    let x: i64 = x + 1;\n"));
}

#[test]
fn a_literal_let_its_scope_only_shadows_is_dropped() {
    assert!(!literal_let_kept("    for x in 0..3 { show(&x); }\n"));
    assert!(!literal_let_kept("    let x: i64 = 5;\n    show(&x);\n"));
    assert!(!literal_let_kept("    {\n        let x: i64 = 5;\n        show(&x);\n    }\n"));
}

#[test]
fn a_reference_binding_written_through_is_not_mut() {
    let code = settle_mutability(&format!(
        "fn f(arr: &mut [i64]) {{\n    let {MAYBE_MUT}result: &mut [i64] = arr;\n    result[0] = 1;\n    result.swap(0, 1);\n}}\n"
    ));
    assert!(code.contains("let result: &mut [i64]"), "{code}");
}

#[test]
fn a_reference_binding_reassigned_is_mut() {
    let code = settle_mutability(&format!(
        "fn f(a: &[i64], b: &[i64]) {{\n    let {MAYBE_MUT}r: &[i64] = a;\n    r = b;\n}}\n"
    ));
    assert!(code.contains("let mut r: &[i64]"), "{code}");
}

/// A copy a `Show` formats, `&{ x }`, reads `x`: it neither keeps `x` mutable nor leaves a literal `x` unread.
#[test]
fn a_shown_copy_reads_its_binding() {
    assert!(!kept_mut("    show(&{ x });\n    show(&format!(\"{}{}\", \"\", { x }));\n"));
    assert!(literal_let_kept("    show(&{ x });\n"));
    assert!(literal_let_kept("    show(&format!(\"{}{}\", \"\", { x }));\n"));
}

/// A call of a method that borrows its receiver shared reads the binding; any other method may write it.
#[test]
fn a_shared_method_call_reads_its_receiver() {
    assert!(!kept_mut("    for v in x.borrow().iter().copied() { show(&v); }\n"));
    assert!(!kept_mut("    let n: i64 = x.len() as i64;\n    show(&x.logos_item_front(1));\n"));
    assert!(!kept_mut("    if x.is_empty() || x.contains(&3) { show(&x.clone()); }\n"));
    assert!(kept_mut("    x.push(1);\n"), "a method off the list may take `&mut self`");
    assert!(kept_mut("    x.borrow_mut().push(1);\n"), "`borrow_mut` is off the list");
    assert!(kept_mut("    x.lenient();\n"), "a name is matched whole");
    assert!(kept_mut("    x.len = 3;\n"), "a field written, not a method called");
    assert!(kept_mut("    show(&x.len());\n    x.cow();\n"));
}
