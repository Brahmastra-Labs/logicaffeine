//! ★ LOCK — the Rust the compiler writes is Rust a reviewer signs. ★
//!
//! Every benchmark program (`benchmarks/programs/`) and every feature program (`codegen_corpus`) is compiled to
//! Rust and held to:
//! - `cargo clippy -- -D warnings`: rustc's warnings and clippy's default lints are errors. A lint the generated
//!   program has reason to silence is silenced by the generated program, where its reader sees the reason;
//! - every `let` states the type the compiler decided — rustc's inference never decides a LOGOS type;
//! - no `std::hint::assert_unchecked` restates an assumption a dominating statement already made about values
//!   nothing has changed since;
//! - no `if` is decided by literals alone, and no `if` repeats one arm as the other;
//! - what a block, a `match`, an `impl`, a record or an enum holds sits deeper than the line that opens its braces.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test generated_rust_lint_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
mod codegen_corpus;

use std::collections::{BTreeMap, HashSet};

use codegen_corpus::Program;
use logicaffeine_compile::compile::compile_program_full;
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

/// Each program's emitted Rust, by name.
fn emitted(programs: &[Program]) -> Vec<(String, String)> {
    programs
        .iter()
        .map(|p| {
            let out = compile_program_full(&p.source)
                .unwrap_or_else(|e| panic!("{} does not compile to Rust: {e:?}", p.name));
            (p.name.clone(), out.rust_code)
        })
        .collect()
}

/// `programs` build without a single rustc or clippy diagnostic.
fn passes_clippy(programs: &[Program]) {
    let diagnostics = common::clippy_generated(&emitted(programs), &[]);
    if diagnostics.is_empty() {
        return;
    }
    let mut by_program: BTreeMap<&str, Vec<&common::GeneratedDiagnostic>> = BTreeMap::new();
    for d in &diagnostics {
        by_program.entry(&d.program).or_default().push(d);
    }
    let mut report = String::new();
    for (program, found) in &by_program {
        let mut codes: BTreeMap<String, usize> = BTreeMap::new();
        for d in found {
            *codes.entry(d.code.clone().unwrap_or_else(|| d.level.clone())).or_default() += 1;
        }
        report.push_str(&format!("\n== {program}: {} diagnostics {codes:?}\n", found.len()));
        for d in found.iter().take(3) {
            report.push_str(&d.rendered);
        }
    }
    panic!(
        "{} diagnostics on {} of {} programs:{report}",
        diagnostics.len(),
        by_program.len(),
        programs.len()
    );
}

/// The harness runs clippy on each program it is handed and names what it finds under that program: a variable
/// nothing reads is rustc's `unused_variables` in `unread`. A harness that cannot build the programs reports a failure
/// of the project instead, and fails this row.
#[test]
fn the_harness_names_what_clippy_finds_in_each_program() {
    let unread = "fn main() {\n    let unread: i64 = 5;\n}\n".to_string();
    let clean = "fn main() {\n    let read: i64 = 5;\n    println!(\"{read}\");\n}\n".to_string();
    let found = common::clippy_generated(&[("unread".to_string(), unread), ("clean".to_string(), clean)], &[]);
    let named: Vec<(&str, Option<&str>)> = found.iter().map(|d| (d.program.as_str(), d.code.as_deref())).collect();
    assert_eq!(named, vec![("unread", Some("unused_variables"))], "{found:#?}");
}

#[test]
fn the_benchmark_programs_pass_clippy() {
    passes_clippy(&codegen_corpus::benchmark_programs());
}

#[test]
fn the_feature_programs_pass_clippy() {
    passes_clippy(&codegen_corpus::feature_programs());
}

// ---------------------------------------------------------------------------------------------------------------
// The four invariants clippy does not check, read from the emitted file's syntax tree.
// ---------------------------------------------------------------------------------------------------------------

/// What one file breaks, invariant by invariant: each entry a token rendering of the offending syntax.
#[derive(Default, Debug)]
struct Breaks {
    untyped_bindings: Vec<String>,
    restated_assumptions: Vec<String>,
    literal_guards: Vec<String>,
    twin_arms: Vec<String>,
    out_of_line: Vec<String>,
}

fn tokens<T: ToTokens>(node: &T) -> String {
    node.to_token_stream().to_string()
}

fn shortened(text: String) -> String {
    if text.chars().count() > 160 {
        format!("{}…", text.chars().take(160).collect::<String>())
    } else {
        text
    }
}

/// The identifiers a token rendering mentions.
fn mentioned(text: &str) -> HashSet<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| w.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_'))
        .map(str::to_string)
        .collect()
}

/// The names a pattern binds.
fn bound_names(pat: &syn::Pat) -> HashSet<String> {
    struct Binds(HashSet<String>);
    impl<'ast> Visit<'ast> for Binds {
        fn visit_pat_ident(&mut self, p: &'ast syn::PatIdent) {
            self.0.insert(p.ident.to_string());
            visit::visit_pat_ident(self, p);
        }
        fn visit_type(&mut self, _: &'ast syn::Type) {}
    }
    let mut binds = Binds(HashSet::new());
    binds.visit_pat(pat);
    binds.0
}

/// The predicate of a `std::hint::assert_unchecked(P)` call.
fn assumption(call: &syn::ExprCall) -> Option<String> {
    let syn::Expr::Path(path) = &*call.func else {
        return None;
    };
    let last = path.path.segments.last()?;
    (last.ident == "assert_unchecked" && call.args.len() == 1).then(|| tokens(&call.args[0]))
}

/// Without its parentheses.
fn bare(expr: &syn::Expr) -> &syn::Expr {
    match expr {
        syn::Expr::Paren(p) => bare(&p.expr),
        syn::Expr::Group(g) => bare(&g.expr),
        other => other,
    }
}

/// A literal, possibly negated.
fn is_literal(expr: &syn::Expr) -> bool {
    match bare(expr) {
        syn::Expr::Lit(_) => true,
        syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => is_literal(&u.expr),
        _ => false,
    }
}

/// A condition literals alone decide: a boolean literal, a comparison of two literals, or a conjunction or
/// disjunction with such a term.
fn decided_by_literals(cond: &syn::Expr) -> bool {
    match bare(cond) {
        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Bool(_), .. }) => true,
        syn::Expr::Binary(b) => match b.op {
            syn::BinOp::And(_) | syn::BinOp::Or(_) => {
                decided_by_literals(&b.left) || decided_by_literals(&b.right)
            }
            syn::BinOp::Lt(_)
            | syn::BinOp::Le(_)
            | syn::BinOp::Gt(_)
            | syn::BinOp::Ge(_)
            | syn::BinOp::Eq(_)
            | syn::BinOp::Ne(_) => is_literal(&b.left) && is_literal(&b.right),
            _ => false,
        },
        _ => false,
    }
}

/// Methods that may shrink or replace what their receiver holds, so a bound on it no longer holds after them.
const SHRINKING: &[&str] = &[
    "pop", "clear", "truncate", "remove", "drain", "retain", "resize", "split_off", "swap_remove", "set_len",
    "cow", "borrow_mut", "take",
];

/// A compound assignment (`+=`, `<<=`, …).
fn assigns(op: &syn::BinOp) -> bool {
    use syn::BinOp::*;
    matches!(
        op,
        AddAssign(_) | SubAssign(_) | MulAssign(_) | DivAssign(_) | RemAssign(_) | BitXorAssign(_)
            | BitAndAssign(_) | BitOrAssign(_) | ShlAssign(_) | ShrAssign(_)
    )
}

/// A closure whose every parameter and whose result state their types: its signature is the binding's type, which
/// Rust gives no other way to write.
fn fully_typed_closure(expr: &syn::Expr) -> bool {
    match bare(expr) {
        syn::Expr::Closure(c) => {
            !matches!(c.output, syn::ReturnType::Default)
                && c.inputs.iter().all(|p| matches!(p, syn::Pat::Type(_)))
        }
        _ => false,
    }
}

/// The identifiers a block can change: assignment targets, receivers of a method that may shrink or replace a
/// collection, `&mut` operands, and names a `let` or a loop binds anew.
fn changed_in(block: &syn::Block) -> HashSet<String> {
    struct Changes(HashSet<String>);
    impl<'ast> Visit<'ast> for Changes {
        fn visit_expr_assign(&mut self, e: &'ast syn::ExprAssign) {
            self.0.extend(mentioned(&tokens(&e.left)));
            visit::visit_expr_assign(self, e);
        }
        fn visit_expr_binary(&mut self, e: &'ast syn::ExprBinary) {
            if assigns(&e.op) {
                self.0.extend(mentioned(&tokens(&e.left)));
            }
            visit::visit_expr_binary(self, e);
        }
        fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
            if SHRINKING.contains(&e.method.to_string().as_str()) {
                self.0.extend(mentioned(&tokens(&e.receiver)));
            }
            visit::visit_expr_method_call(self, e);
        }
        fn visit_expr_reference(&mut self, e: &'ast syn::ExprReference) {
            if e.mutability.is_some() {
                self.0.extend(mentioned(&tokens(&e.expr)));
            }
            visit::visit_expr_reference(self, e);
        }
        fn visit_local(&mut self, l: &'ast syn::Local) {
            self.0.extend(bound_names(&l.pat));
            visit::visit_local(self, l);
        }
        fn visit_expr_for_loop(&mut self, e: &'ast syn::ExprForLoop) {
            self.0.extend(bound_names(&e.pat));
            visit::visit_expr_for_loop(self, e);
        }
    }
    let mut changes = Changes(HashSet::new());
    changes.visit_block(block);
    changes.0
}

/// The reviewer: walks a file in order, holding the assumptions each point is dominated by.
struct Review {
    breaks: Breaks,
    /// One set per open block; `unsafe { … }` adds to its enclosing block's set.
    assumed: Vec<HashSet<String>>,
    /// The file's functions whose result is an `impl Trait`: a binding of their call has the type their signature
    /// states and Rust gives no other name.
    opaque_returners: HashSet<String>,
}

impl Review {
    fn calls_an_opaque_returner(&self, expr: &syn::Expr) -> bool {
        let syn::Expr::Call(call) = bare(expr) else {
            return false;
        };
        let syn::Expr::Path(path) = &*call.func else {
            return false;
        };
        path.path
            .get_ident()
            .is_some_and(|f| self.opaque_returners.contains(&f.to_string()))
    }

    fn forget_mentions_of(&mut self, names: &HashSet<String>) {
        for scope in &mut self.assumed {
            scope.retain(|a| mentioned(a).is_disjoint(names));
        }
    }

    fn in_scope<F: FnOnce(&mut Self)>(&mut self, f: F) {
        self.assumed.push(HashSet::new());
        f(self);
        self.assumed.pop();
    }

    /// A loop body sees none of the assumptions about what the body changes (a later iteration runs after the
    /// change), and neither does anything after the loop.
    fn in_loop_body(&mut self, body: &syn::Block, also_changing: HashSet<String>) {
        let mut changing = changed_in(body);
        changing.extend(also_changing);
        self.forget_mentions_of(&changing);
        self.in_scope(|r| visit::visit_block(r, body));
    }
}

impl<'ast> Visit<'ast> for Review {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        let saved = std::mem::take(&mut self.assumed);
        visit::visit_item_fn(self, f);
        self.assumed = saved;
    }

    fn visit_block(&mut self, b: &'ast syn::Block) {
        self.in_scope(|r| visit::visit_block(r, b));
    }

    fn visit_expr_unsafe(&mut self, e: &'ast syn::ExprUnsafe) {
        for stmt in &e.block.stmts {
            self.visit_stmt(stmt);
        }
    }

    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        let saved = std::mem::take(&mut self.assumed);
        visit::visit_expr_closure(self, c);
        self.assumed = saved;
    }

    fn visit_expr_for_loop(&mut self, e: &'ast syn::ExprForLoop) {
        self.visit_expr(&e.expr);
        self.in_loop_body(&e.body, bound_names(&e.pat));
    }

    fn visit_expr_while(&mut self, e: &'ast syn::ExprWhile) {
        self.forget_mentions_of(&changed_in(&e.body));
        self.visit_expr(&e.cond);
        self.in_scope(|r| visit::visit_block(r, &e.body));
    }

    fn visit_expr_loop(&mut self, e: &'ast syn::ExprLoop) {
        self.in_loop_body(&e.body, HashSet::new());
    }

    fn visit_expr_assign(&mut self, e: &'ast syn::ExprAssign) {
        visit::visit_expr_assign(self, e);
        let names = mentioned(&tokens(&e.left));
        self.forget_mentions_of(&names);
    }

    fn visit_expr_binary(&mut self, e: &'ast syn::ExprBinary) {
        visit::visit_expr_binary(self, e);
        if assigns(&e.op) {
            let names = mentioned(&tokens(&e.left));
            self.forget_mentions_of(&names);
        }
    }

    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        visit::visit_expr_method_call(self, e);
        if SHRINKING.contains(&e.method.to_string().as_str()) {
            let names = mentioned(&tokens(&e.receiver));
            self.forget_mentions_of(&names);
        }
    }

    fn visit_expr_reference(&mut self, e: &'ast syn::ExprReference) {
        visit::visit_expr_reference(self, e);
        if e.mutability.is_some() {
            let names = mentioned(&tokens(&e.expr));
            self.forget_mentions_of(&names);
        }
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        visit::visit_expr_call(self, call);
        if let Some(predicate) = assumption(call) {
            if self.assumed.iter().any(|scope| scope.contains(&predicate)) {
                self.breaks.restated_assumptions.push(shortened(predicate));
            } else if let Some(scope) = self.assumed.last_mut() {
                scope.insert(predicate);
            }
        }
    }

    fn visit_local(&mut self, l: &'ast syn::Local) {
        visit::visit_local(self, l);
        // A struct pattern names its type: the struct's declaration states every field's.
        let typed = matches!(l.pat, syn::Pat::Type(_) | syn::Pat::Wild(_) | syn::Pat::Struct(_))
            || l.init.as_ref().is_some_and(|init| {
                fully_typed_closure(&init.expr) || self.calls_an_opaque_returner(&init.expr)
            });
        if !typed {
            self.breaks.untyped_bindings.push(shortened(tokens(l)));
        }
        let names = bound_names(&l.pat);
        self.forget_mentions_of(&names);
    }

    fn visit_expr_if(&mut self, e: &'ast syn::ExprIf) {
        if decided_by_literals(&e.cond) {
            self.breaks.literal_guards.push(shortened(tokens(&e.cond)));
        }
        if let Some((_, otherwise)) = &e.else_branch {
            if let syn::Expr::Block(arm) = &**otherwise {
                if tokens(&e.then_branch) == tokens(&arm.block) {
                    self.breaks
                        .twin_arms
                        .push(shortened(format!("if {} {{ … }}", tokens(&e.cond))));
                }
            }
        }
        self.visit_expr(&e.cond);
        self.visit_block(&e.then_branch);
        if let Some((_, otherwise)) = &e.else_branch {
            self.visit_expr(otherwise);
        }
    }
}

/// What a Rust file breaks.
/// The lines of `file`, read from `rust`, that a block, a `match`, an `impl`, a record or an enum holds on a later line
/// than its opening brace and that do not sit deeper than the line the brace is on. Positions come from the syntax
/// tree, so a line inside a string literal is never one.
fn out_of_line(rust: &str, file: &syn::File) -> Vec<String> {
    struct Layout<'a> {
        lines: Vec<&'a str>,
        found: Vec<String>,
    }
    impl Layout<'_> {
        fn inside(&mut self, open: proc_macro2::LineColumn, starts: impl Iterator<Item = proc_macro2::LineColumn>) {
            let brace_line = self.lines[open.line - 1];
            let depth = brace_line.len() - brace_line.trim_start().len();
            for start in starts {
                if start.line != open.line && start.column <= depth {
                    self.found.push(shortened(format!("{}: {}", start.line, self.lines[start.line - 1].trim())));
                }
            }
        }
    }
    impl<'ast> Visit<'ast> for Layout<'_> {
        fn visit_block(&mut self, b: &'ast syn::Block) {
            self.inside(b.brace_token.span.open().start(), b.stmts.iter().map(|s| s.span().start()));
            visit::visit_block(self, b);
        }
        fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
            self.inside(m.brace_token.span.open().start(), m.arms.iter().map(|a| a.span().start()));
            visit::visit_expr_match(self, m);
        }
        fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
            self.inside(i.brace_token.span.open().start(), i.items.iter().map(|x| x.span().start()));
            visit::visit_item_impl(self, i);
        }
        fn visit_fields_named(&mut self, f: &'ast syn::FieldsNamed) {
            self.inside(f.brace_token.span.open().start(), f.named.iter().map(|x| x.span().start()));
            visit::visit_fields_named(self, f);
        }
        fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
            self.inside(e.brace_token.span.open().start(), e.variants.iter().map(|x| x.span().start()));
            visit::visit_item_enum(self, e);
        }
    }
    let mut layout = Layout { lines: rust.lines().collect(), found: Vec::new() };
    layout.visit_file(file);
    layout.found
}

fn review(rust: &str) -> Breaks {
    let file = syn::parse_file(rust).unwrap_or_else(|e| panic!("the emitted Rust does not parse: {e}"));
    struct OpaqueReturners(HashSet<String>);
    impl<'ast> Visit<'ast> for OpaqueReturners {
        fn visit_signature(&mut self, sig: &'ast syn::Signature) {
            if let syn::ReturnType::Type(_, ty) = &sig.output {
                if matches!(**ty, syn::Type::ImplTrait(_)) {
                    self.0.insert(sig.ident.to_string());
                }
            }
        }
    }
    let mut opaque = OpaqueReturners(HashSet::new());
    opaque.visit_file(&file);
    let mut reviewer = Review {
        breaks: Breaks::default(),
        assumed: Vec::new(),
        opaque_returners: opaque.0,
    };
    reviewer.visit_file(&file);
    reviewer.breaks.out_of_line = out_of_line(rust, &file);
    reviewer.breaks
}

/// Every program of both corpora holds the invariant `pick` reads.
fn every_program_holds(invariant: &str, pick: fn(&Breaks) -> &Vec<String>) {
    let programs = codegen_corpus::all_programs();
    let mut report = String::new();
    let mut total = 0;
    let mut broken = 0;
    for (name, rust) in emitted(&programs) {
        let breaks = review(&rust);
        let found = pick(&breaks);
        if found.is_empty() {
            continue;
        }
        total += found.len();
        broken += 1;
        report.push_str(&format!("\n== {name}: {}\n", found.len()));
        for f in found.iter().take(4) {
            report.push_str(&format!("   {f}\n"));
        }
    }
    assert!(
        total == 0,
        "{total} breaks of `{invariant}` in {broken} of {} programs:{report}",
        programs.len()
    );
}

#[test]
fn every_binding_states_its_type() {
    every_program_holds("every let states its type", |b| &b.untyped_bindings);
}

#[test]
fn no_assumption_is_restated() {
    every_program_holds("no assumption restated", |b| &b.restated_assumptions);
}

#[test]
fn no_branch_is_decided_by_literals() {
    every_program_holds("no if decided by literals", |b| &b.literal_guards);
}

#[test]
fn no_branch_repeats_its_other_arm() {
    every_program_holds("no if with twin arms", |b| &b.twin_arms);
}

#[test]
fn every_block_holds_its_lines_inside_its_braces() {
    every_program_holds("a block's lines sit inside its braces", |b| &b.out_of_line);
}

// ---------------------------------------------------------------------------------------------------------------
// The reviewer itself: each invariant is seen broken, and a file that keeps it is not flagged.
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_reviewer_sees_an_untyped_binding_and_passes_a_typed_one() {
    let b = review(
        "fn f() {
            let a = 1i64;
            let b: i64 = 2;
            let _ = a;
            let (c, d) = (1, 2);
            let g = move |x: i64| -> i64 { x * 2 };
            let h = move |x: i64| x * 2;
            let k = move |x| -> i64 { x };
            let add5 = adder(5);
            let other = elsewhere(5);
            let P { x, .. } = &p;
        }
        fn adder(n: i64) -> impl Fn(i64) -> i64 { move |x: i64| -> i64 { x + n } }
        fn elsewhere(n: i64) -> i64 { n }",
    );
    assert_eq!(b.untyped_bindings.len(), 5, "{:?}", b.untyped_bindings);
    assert!(b.untyped_bindings[0].starts_with("let a"));
    assert!(b.untyped_bindings[1].starts_with("let (c , d)"));
    assert!(b.untyped_bindings[2].starts_with("let h"));
    assert!(b.untyped_bindings[3].starts_with("let k"));
    assert!(b.untyped_bindings[4].starts_with("let other"));
}

#[test]
fn the_reviewer_sees_an_assumption_restated_in_one_block_and_under_an_enclosing_one() {
    let b = review(
        "fn f(v: &[i64], i: i64) -> i64 {
            unsafe { std::hint::assert_unchecked(i < v.len() as i64); }
            let a: i64 = v[i as usize];
            unsafe { std::hint::assert_unchecked(i < v.len() as i64); }
            if a > 0 { unsafe { std::hint::assert_unchecked(i < v.len() as i64); } }
            a
        }",
    );
    assert_eq!(b.restated_assumptions.len(), 2, "{:?}", b.restated_assumptions);
}

#[test]
fn the_reviewer_lets_a_changed_value_be_assumed_again() {
    let b = review(
        "fn f(v: &mut Vec<i64>, mut i: i64) {
            unsafe { std::hint::assert_unchecked(i < v.len() as i64); }
            i = i + 1;
            unsafe { std::hint::assert_unchecked(i < v.len() as i64); }
            unsafe { std::hint::assert_unchecked(0 < v.len() as i64); }
            v.pop();
            unsafe { std::hint::assert_unchecked(0 < v.len() as i64); }
            for i in 0..3 { unsafe { std::hint::assert_unchecked(i < v.len() as i64); } }
            let mut k: i64 = 0;
            while k < 3 {
                unsafe { std::hint::assert_unchecked(k < 9); }
                k += 1;
            }
            if i > 0 { unsafe { std::hint::assert_unchecked(i > 0); } }
            unsafe { std::hint::assert_unchecked(i > 0); }
        }",
    );
    assert!(b.restated_assumptions.is_empty(), "{:?}", b.restated_assumptions);
}

#[test]
fn the_reviewer_sees_a_loop_body_restating_an_assumption_nothing_in_it_changes() {
    let b = review(
        "fn f(v: &[i64], n: i64) {
            unsafe { std::hint::assert_unchecked(n <= v.len() as i64); }
            for i in 0..n { unsafe { std::hint::assert_unchecked(n <= v.len() as i64); } }
        }",
    );
    assert_eq!(b.restated_assumptions.len(), 1, "{:?}", b.restated_assumptions);
}

#[test]
fn the_reviewer_sees_literal_guards_and_passes_a_real_one() {
    let b = review(
        "fn f(n: i64) {
            if (5 <= 4611686018427387904_i64) { g(); }
            if (5 <= 1152921504606846976_i64) && (n <= 1152921504606846976_i64) { g(); }
            if true { g(); }
            if (-3 < 0) { g(); }
            if n <= 4611686018427387904_i64 { g(); }
        }",
    );
    assert_eq!(b.literal_guards.len(), 4, "{:?}", b.literal_guards);
}

#[test]
fn the_reviewer_sees_twin_arms_and_passes_different_ones() {
    let b = review(
        "fn f(n: i64) {
            if n < 9 { g(n); } else { g(n); }
            if n < 9 { g(n); } else { h(n); }
            if n < 9 { g(n); } else if n < 10 { g(n); } else { h(n); }
        }",
    );
    assert_eq!(b.twin_arms.len(), 1, "{:?}", b.twin_arms);
}

#[test]
fn the_reviewer_sees_a_line_at_its_braces_level_and_passes_one_inside_them() {
    let b = review(
        "fn f(a: &mut Vec<i64>, n: i64) {
    {
        let a: &mut Vec<i64> = &mut *a;
    for i in 0..n {
        a.push(i);
    }
    }
    match n {
    0 => g(),
        _ => g(),
    }
    let s = \"a text
across lines\";
    if n < 9 {
        g();
    } else {
        h();
    }
}",
    );
    assert_eq!(
        b.out_of_line,
        vec!["4: for i in 0..n {".to_string(), "9: 0 => g(),".to_string()],
        "a loop at its scope's brace and an arm at its match's; a string's second line is the string's"
    );
}
