//! The post-quantum kernels compute on secret coefficients, and two things leak them through time.
//!
//! **A division.** A `/` or `%` by q, or `rem_euclid(q)`, compiles to a hardware divide whenever
//! LLVM keeps it: wasm32 emits `i32.div_u`/`i32.rem_s` at every opt-level, and x86-64 emits
//! `div`/`idiv` at opt-level 0 and `z`. A divide's latency depends on its operands, and ML-KEM's
//! Compress and Decrypt dividing secret values by q is KyberSlash. The census reads
//! `logicaffeine_system`'s `mldsa.rs`, `mlkem.rs` and `ntt.rs` with `syn`, and a division survives
//! only when its divisor is a power-of-two constant, both operands are constants (folded at compile
//! time), it sits in a `const` item or `const fn`, or [`PUBLIC_DIVISIONS`] names it with its reason.
//!
//! **A branch.** Every function in [`BRANCH_FREE`] reduces, compares or selects secret data, and
//! holds no `if`, `match`, loop condition, `?`, `return`, short-circuit `&&`/`||`, or comparison
//! operator: an early-exit `c == c'` and the `if` that picks K or K̄ after it are the
//! Fujisaki–Okamoto timing oracle of ML-KEM.Decaps. A `for` over a range is allowed: its trip count
//! is public.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::visit::Visit;

/// The modules whose arithmetic the census reads, relative to the workspace root.
const SCANNED: &[&str] = &[
    "crates/logicaffeine_system/src/mldsa.rs",
    "crates/logicaffeine_system/src/mlkem.rs",
    "crates/logicaffeine_system/src/ntt.rs",
];

/// Divisions of public values: `(file, function, the expression with its whitespace removed,
/// why no secret reaches it)`. A division inside a macro body the census cannot parse as
/// expressions is named `macro:<macro name>`. A row that matches no division fails the census.
const PUBLIC_DIVISIONS: &[(&str, &str, &str, &str)] = &[(
    "crates/logicaffeine_system/src/mldsa.rs",
    "bench_verify_breakdown",
    "macro:macro_rules",
    "the dev-only profiler divides elapsed nanoseconds by its iteration count",
)];

/// Functions that compute on secret data without branching on it: `(file, function)`.
const BRANCH_FREE: &[(&str, &str)] = &[
    ("crates/logicaffeine_system/src/mldsa.rs", "reduce32"),
    ("crates/logicaffeine_system/src/mldsa.rs", "caddq"),
    ("crates/logicaffeine_system/src/mldsa.rs", "freeze"),
    ("crates/logicaffeine_system/src/mldsa.rs", "to_signed"),
    ("crates/logicaffeine_system/src/mldsa.rs", "power2round"),
    ("crates/logicaffeine_system/src/mldsa.rs", "decompose"),
    ("crates/logicaffeine_system/src/ntt.rs", "cadd_q"),
    ("crates/logicaffeine_system/src/ntt.rs", "barrett_reduce"),
    ("crates/logicaffeine_system/src/ntt.rs", "csub_q"),
    ("crates/logicaffeine_system/src/ntt.rs", "reduce_wide"),
    (
        "crates/logicaffeine_system/src/ntt.rs",
        "compress_coefficient",
    ),
    ("crates/logicaffeine_system/src/mlkem.rs", "addq"),
    ("crates/logicaffeine_system/src/mlkem.rs", "subq"),
    ("crates/logicaffeine_system/src/mlkem.rs", "ct_eq_mask"),
    ("crates/logicaffeine_system/src/mlkem.rs", "ct_select"),
    ("crates/logicaffeine_system/src/mlkem.rs", "fo_decapsulate"),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/logicaffeine_tests sits two below the root")
        .to_path_buf()
}

fn normalized(tokens: impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// The integer value of every `const NAME: T = <literal or shift of literals>;` in a file.
fn file_constants(file: &syn::File) -> BTreeMap<String, u128> {
    struct Consts(BTreeMap<String, u128>);
    impl<'ast> Visit<'ast> for Consts {
        fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
            if let Some(v) = literal_value(&c.expr) {
                self.0.insert(c.ident.to_string(), v);
            }
            syn::visit::visit_item_const(self, c);
        }
    }
    let mut consts = Consts(BTreeMap::new());
    consts.visit_file(file);
    consts.0
}

fn literal_value(e: &syn::Expr) -> Option<u128> {
    match e {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(i),
            ..
        }) => i.base10_parse::<u128>().ok(),
        syn::Expr::Paren(p) => literal_value(&p.expr),
        syn::Expr::Cast(c) => literal_value(&c.expr),
        syn::Expr::Binary(b) if matches!(b.op, syn::BinOp::Shl(_)) => {
            Some(literal_value(&b.left)? << literal_value(&b.right)?)
        }
        _ => None,
    }
}

/// Whether `e` is fixed when the program is compiled: literals, SCREAMING_CASE constants, and casts,
/// parentheses, negations and arithmetic of those.
fn is_constant(e: &syn::Expr) -> bool {
    match e {
        syn::Expr::Lit(_) => true,
        syn::Expr::Path(p) => p.path.segments.last().is_some_and(|s| {
            let name = s.ident.to_string();
            name.len() > 1
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        }),
        syn::Expr::Paren(p) => is_constant(&p.expr),
        syn::Expr::Cast(c) => is_constant(&c.expr),
        syn::Expr::Unary(u) => is_constant(&u.expr),
        syn::Expr::Binary(b) => is_constant(&b.left) && is_constant(&b.right),
        _ => false,
    }
}

/// Whether `e` is a constant power of two, so dividing by it is a shift.
fn is_power_of_two(e: &syn::Expr, consts: &BTreeMap<String, u128>) -> bool {
    let value = literal_value(e).or_else(|| match e {
        syn::Expr::Path(p) => p
            .path
            .segments
            .last()
            .and_then(|s| consts.get(&s.ident.to_string()).copied()),
        syn::Expr::Paren(p) => {
            if is_power_of_two(&p.expr, consts) {
                Some(1)
            } else {
                None
            }
        }
        syn::Expr::Cast(c) => {
            if is_power_of_two(&c.expr, consts) {
                Some(1)
            } else {
                None
            }
        }
        _ => None,
    });
    value.is_some_and(|v| v.is_power_of_two())
}

const DIVIDING_METHODS: &[&str] = &[
    "rem_euclid",
    "div_euclid",
    "checked_div",
    "checked_rem",
    "checked_div_euclid",
    "checked_rem_euclid",
    "wrapping_div",
    "wrapping_rem",
    "wrapping_div_euclid",
    "wrapping_rem_euclid",
    "overflowing_div",
    "overflowing_rem",
    "saturating_div",
    "div_ceil",
    "div_floor",
];

/// One division the census found: where it is and its normalized text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Division {
    function: String,
    expr: String,
    line: usize,
}

struct DivisionCensus<'a> {
    consts: &'a BTreeMap<String, u128>,
    function: Vec<String>,
    const_depth: usize,
    found: Vec<Division>,
}

impl DivisionCensus<'_> {
    fn record(&mut self, expr: impl ToTokens, line: usize) {
        self.record_text(normalized(expr), line);
    }
    fn record_text(&mut self, expr: String, line: usize) {
        if self.const_depth > 0 {
            return;
        }
        self.found.push(Division {
            function: self.function.last().cloned().unwrap_or_default(),
            expr,
            line,
        });
    }
}

impl<'ast> Visit<'ast> for DivisionCensus<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        let is_const = f.sig.constness.is_some();
        self.function.push(f.sig.ident.to_string());
        self.const_depth += is_const as usize;
        syn::visit::visit_item_fn(self, f);
        self.const_depth -= is_const as usize;
        self.function.pop();
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        let is_const = f.sig.constness.is_some();
        self.function.push(f.sig.ident.to_string());
        self.const_depth += is_const as usize;
        syn::visit::visit_impl_item_fn(self, f);
        self.const_depth -= is_const as usize;
        self.function.pop();
    }
    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        self.const_depth += 1;
        syn::visit::visit_item_const(self, c);
        self.const_depth -= 1;
    }
    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        self.const_depth += 1;
        syn::visit::visit_item_static(self, s);
        self.const_depth -= 1;
    }
    fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
        let divides = matches!(
            b.op,
            syn::BinOp::Div(_)
                | syn::BinOp::Rem(_)
                | syn::BinOp::DivAssign(_)
                | syn::BinOp::RemAssign(_)
        );
        let folded = is_constant(&b.left) && is_constant(&b.right);
        if divides && !folded && !is_power_of_two(&b.right, self.consts) {
            let line =
                b.op.to_token_stream()
                    .into_iter()
                    .next()
                    .map_or(0, |t| t.span().start().line);
            self.record(b, line);
        }
        syn::visit::visit_expr_binary(self, b);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if DIVIDING_METHODS.contains(&method.as_str()) {
            let by_power_of_two =
                call.args.len() == 1 && is_power_of_two(&call.args[0], self.consts);
            if !by_power_of_two {
                self.record(call, call.method.span().start().line);
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        // Arguments that parse as expressions (`assert!`, `format!`, `vec!`, …) get the census
        // proper; any other body (a `macro_rules!` definition) is read token by token.
        use syn::parse::Parser;
        let args = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        let repeat = |input: syn::parse::ParseStream<'_>| -> syn::Result<Vec<syn::Expr>> {
            let element: syn::Expr = input.parse()?;
            input.parse::<syn::Token![;]>()?;
            let count: syn::Expr = input.parse()?;
            Ok(vec![element, count])
        };
        let parsed = args
            .parse2(m.tokens.clone())
            .map(|exprs| exprs.into_iter().collect::<Vec<_>>())
            .or_else(|_| repeat.parse2(m.tokens.clone()));
        match parsed {
            Ok(exprs) => exprs.iter().for_each(|e| self.visit_expr(e)),
            Err(_) => {
                if let Some(line) = dividing_punct(m.tokens.clone()) {
                    let name = m
                        .path
                        .segments
                        .last()
                        .map(|s| s.ident.to_string())
                        .unwrap_or_default();
                    self.record_text(format!("macro:{name}"), line);
                }
            }
        }
    }
}

/// The line of the first `/` or `%` operator in a token stream, looking inside every group.
fn dividing_punct(tokens: proc_macro2::TokenStream) -> Option<usize> {
    tokens.into_iter().find_map(|t| match t {
        proc_macro2::TokenTree::Punct(p) if p.as_char() == '/' || p.as_char() == '%' => {
            Some(p.span().start().line)
        }
        proc_macro2::TokenTree::Group(g) => dividing_punct(g.stream()),
        _ => None,
    })
}

fn divisions_in(source: &str) -> Vec<Division> {
    let file = syn::parse_file(source).expect("the module parses");
    let consts = file_constants(&file);
    let mut census = DivisionCensus {
        consts: &consts,
        function: Vec::new(),
        const_depth: 0,
        found: Vec::new(),
    };
    census.visit_file(&file);
    census.found
}

/// What makes a function branch on its data: the construct and its line.
struct BranchCensus {
    found: Vec<(String, usize)>,
}

impl<'ast> Visit<'ast> for BranchCensus {
    fn visit_expr_if(&mut self, e: &'ast syn::ExprIf) {
        self.found
            .push(("`if`".into(), e.if_token.span.start().line));
        syn::visit::visit_expr_if(self, e);
    }
    fn visit_expr_match(&mut self, e: &'ast syn::ExprMatch) {
        self.found
            .push(("`match`".into(), e.match_token.span.start().line));
        syn::visit::visit_expr_match(self, e);
    }
    fn visit_expr_while(&mut self, e: &'ast syn::ExprWhile) {
        self.found
            .push(("`while`".into(), e.while_token.span.start().line));
        syn::visit::visit_expr_while(self, e);
    }
    fn visit_expr_loop(&mut self, e: &'ast syn::ExprLoop) {
        self.found
            .push(("`loop`".into(), e.loop_token.span.start().line));
        syn::visit::visit_expr_loop(self, e);
    }
    fn visit_expr_try(&mut self, e: &'ast syn::ExprTry) {
        self.found
            .push(("`?`".into(), e.question_token.span.start().line));
        syn::visit::visit_expr_try(self, e);
    }
    fn visit_expr_return(&mut self, e: &'ast syn::ExprReturn) {
        self.found
            .push(("`return`".into(), e.return_token.span.start().line));
        syn::visit::visit_expr_return(self, e);
    }
    fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
        use syn::BinOp::*;
        let what = match b.op {
            And(_) => Some("`&&`"),
            Or(_) => Some("`||`"),
            Eq(_) => Some("`==`"),
            Ne(_) => Some("`!=`"),
            Lt(_) => Some("`<`"),
            Le(_) => Some("`<=`"),
            Gt(_) => Some("`>`"),
            Ge(_) => Some("`>=`"),
            _ => None,
        };
        if let Some(what) = what {
            let line =
                b.op.to_token_stream()
                    .into_iter()
                    .next()
                    .map_or(0, |t| t.span().start().line);
            self.found.push((what.into(), line));
        }
        syn::visit::visit_expr_binary(self, b);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if [
            "eq",
            "ne",
            "lt",
            "le",
            "gt",
            "ge",
            "cmp",
            "partial_cmp",
            "min",
            "max",
            "clamp",
        ]
        .contains(&method.as_str())
        {
            self.found
                .push((format!("`.{method}()`"), call.method.span().start().line));
        }
        syn::visit::visit_expr_method_call(self, call);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        let name = m
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if !["debug_assert", "debug_assert_eq"].contains(&name.as_str()) {
            self.found.push((
                format!("`{name}!`"),
                m.path.segments[0].ident.span().start().line,
            ));
        }
    }
}

/// The body of function `name` in `file`, wherever it is declared.
fn find_fn<'a>(file: &'a syn::File, name: &str) -> Option<&'a syn::Block> {
    struct Finder<'a, 'n> {
        name: &'n str,
        found: Option<&'a syn::Block>,
    }
    impl<'a> Visit<'a> for Finder<'a, '_> {
        fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
            if f.sig.ident == self.name {
                self.found = Some(&f.block);
            }
            syn::visit::visit_item_fn(self, f);
        }
        fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
            if f.sig.ident == self.name {
                self.found = Some(&f.block);
            }
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
    let mut finder = Finder { name, found: None };
    finder.visit_file(file);
    finder.found
}

fn branches_in(block: &syn::Block) -> Vec<(String, usize)> {
    let mut census = BranchCensus { found: Vec::new() };
    census.visit_block(block);
    census.found
}

#[test]
fn no_secret_path_divides_by_a_modulus() {
    let root = workspace_root();
    let mut wrong = Vec::new();
    let mut matched: BTreeSet<(&str, &str, &str)> = BTreeSet::new();
    for path in SCANNED {
        let source =
            std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        for d in divisions_in(&source) {
            let named = PUBLIC_DIVISIONS.iter().find(|(file, function, expr, _)| {
                file == path && *function == d.function && *expr == d.expr
            });
            match named {
                Some((file, function, expr, _)) => {
                    matched.insert((file, function, expr));
                }
                None => wrong.push(format!(
                    "{path}:{} in `{}`: `{}` divides by a value that is not a power-of-two constant",
                    d.line, d.function, d.expr
                )),
            }
        }
    }
    for (file, function, expr, _) in PUBLIC_DIVISIONS {
        if !matched.contains(&(*file, *function, *expr)) {
            wrong.push(format!(
                "{file}: PUBLIC_DIVISIONS names `{expr}` in `{function}`, which the source no longer holds — delete the row"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} division(s):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn every_secret_path_reduction_and_select_is_branch_free() {
    let root = workspace_root();
    let mut parsed: BTreeMap<&str, syn::File> = BTreeMap::new();
    let mut wrong = Vec::new();
    for (path, function) in BRANCH_FREE {
        let file = parsed.entry(path).or_insert_with(|| {
            let source =
                std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
            syn::parse_file(&source).expect("the module parses")
        });
        match find_fn(file, function) {
            None => wrong.push(format!(
                "{path}: no function `{function}` (BRANCH_FREE names it)"
            )),
            Some(body) => {
                for (what, line) in branches_in(body) {
                    wrong.push(format!(
                        "{path}:{line} in `{function}`: {what} branches on its data"
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} branch(es):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn the_division_census_flags_a_divide_by_q_and_keeps_shifts_and_folded_constants() {
    let source = r#"
        const Q: i32 = 3329;
        const N: usize = 256;
        const ML: usize = 5;
        const HALF: i32 = Q / 2;
        fn secret(x: i32, i: usize, d: u64) -> i32 {
            let a = x % Q;
            let b = x / 3329;
            let c = x.rem_euclid(Q);
            let mut e = x;
            e %= Q;
            let f = (i / ML) as i32;
            let g = (d / (Q as u64)) as i32;
            a + b + c + e + f + g
        }
        fn public(x: i32, i: usize) -> i32 {
            let a = x / 16;
            let b = (i % 2) as i32;
            let c = Q / 2;
            let e = (N / 4 * 5) as i32;
            let f = x >> 4;
            let g = i.div_ceil(8) as i32;
            let h = x.rem_euclid(256);
            a + b + c + e + f + g + h
        }
        const fn table() -> i32 { 1753 * 1753 % Q }
        fn in_macros(x: i32) -> i32 {
            assert!(x % Q == 0, "{}", x / 7);
            macro_rules! m { ($e:expr) => { $e % Q }; }
            debug_assert!(x / 16 >= 0);
            let v = vec![0u8; (x / 4) as usize];
            let w = vec![0u8; (x / 3) as usize];
            m!(x) + v.len() as i32 + w.len() as i32
        }
    "#;
    let found: Vec<(String, String)> = divisions_in(source)
        .into_iter()
        .map(|d| (d.function, d.expr))
        .collect();
    assert_eq!(
        found,
        vec![
            ("secret".to_string(), "x%Q".to_string()),
            ("secret".to_string(), "x/3329".to_string()),
            ("secret".to_string(), "x.rem_euclid(Q)".to_string()),
            ("secret".to_string(), "e%=Q".to_string()),
            ("secret".to_string(), "i/ML".to_string()),
            ("secret".to_string(), "d/(Qasu64)".to_string()),
            ("in_macros".to_string(), "x%Q".to_string()),
            ("in_macros".to_string(), "x/7".to_string()),
            ("in_macros".to_string(), "macro:macro_rules".to_string()),
            ("in_macros".to_string(), "x/3".to_string()),
        ],
        "every divide by a non-power-of-two runtime value, and nothing else"
    );
}

#[test]
fn the_branch_census_flags_control_flow_short_circuits_and_comparisons() {
    let source = r#"
        fn branchy(a: &[u8], b: &[u8], x: i32) -> u8 {
            if x > 0 { return 1; }
            let same = a == b;
            let first = a.iter().zip(b).all(|(p, q)| p != q) && x < 3;
            let m = match x { 0 => 1, _ => 2 };
            while x < 0 {}
            loop { break; }
            let y = Some(1)?;
            let z = x.max(3);
            assert!(x == 0);
            (same as u8) + (first as u8) + m + y + z as u8
        }
        fn branch_free(a: &[u8], b: &[u8]) -> u8 {
            let mut diff = 0u8;
            for i in 0..a.len() {
                diff |= a[i] ^ b[i];
            }
            debug_assert!(a.len() == b.len());
            ((diff as u16).wrapping_sub(1) >> 8) as u8
        }
    "#;
    let file = syn::parse_file(source).expect("the sample parses");
    let found: Vec<String> = branches_in(find_fn(&file, "branchy").unwrap())
        .into_iter()
        .map(|(what, _)| what)
        .collect();
    for want in [
        "`if`",
        "`>`",
        "`return`",
        "`==`",
        "`!=`",
        "`&&`",
        "`<`",
        "`match`",
        "`while`",
        "`loop`",
        "`?`",
        "`.max()`",
        "`assert!`",
    ] {
        assert!(
            found.contains(&want.to_string()),
            "the census misses {want}: {found:?}"
        );
    }
    assert_eq!(
        branches_in(find_fn(&file, "branch_free").unwrap()),
        Vec::<(String, usize)>::new(),
        "a range `for`, xor/or accumulation and a mask shift are branch-free"
    );
}
