//! The generative differential fuzzer: deterministic, seeded LOGOS program
//! generation across the VM's full feature surface, executed by BOTH engines
//! with outcome equality (output AND error text) asserted.
//!
//! Feature flags gate grammar productions — the flag set is the single source
//! of truth for what the VM claims to support, and grows in lockstep with it.
//! Generated programs are total by construction (bound loops, decreasing
//! recursion, nonzero literal divisors, bind-before-use), EXCEPT in
//! error-injection mode where exactly one trapping construct is planted and
//! the engines must agree on the partial output and the error string.

#![cfg(test)]

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, ClosureBody, Expr, Literal, Pattern, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// Grammar productions, gated per generation run.
#[derive(Clone, Copy)]
pub struct FeatureSet(pub u32);

impl FeatureSet {
    pub const INT_ARITH: u32 = 1 << 0;
    pub const FLOAT: u32 = 1 << 1;
    pub const TEXT: u32 = 1 << 2;
    pub const BOOL_OPS: u32 = 1 << 3;
    pub const LIST: u32 = 1 << 4;
    pub const IF: u32 = 1 << 5;
    pub const WHILE: u32 = 1 << 6;
    pub const REPEAT: u32 = 1 << 7;
    pub const BREAK: u32 = 1 << 8;
    pub const FUNCTIONS: u32 = 1 << 9;
    pub const CLOSURES: u32 = 1 << 10;
    pub const STRUCTS: u32 = 1 << 11;
    pub const BUILTINS: u32 = 1 << 12;
    pub const INTERPOLATION: u32 = 1 << 13;
    pub const ERROR_INJECTION: u32 = 1 << 14;

    pub fn all_supported() -> Self {
        FeatureSet(
            Self::INT_ARITH
                | Self::FLOAT
                | Self::TEXT
                | Self::BOOL_OPS
                | Self::LIST
                | Self::IF
                | Self::WHILE
                | Self::REPEAT
                | Self::BREAK
                | Self::FUNCTIONS
                | Self::CLOSURES
                | Self::STRUCTS
                | Self::BUILTINS
                | Self::INTERPOLATION,
        )
    }

    pub fn has(&self, flag: u32) -> bool {
        self.0 & flag != 0
    }
}

pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        SplitMix64 { state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15) }
    }
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Names bound so far, by kind (the bind-before-use invariant).
struct Scope {
    ints: Vec<Symbol>,
    floats: Vec<Symbol>,
    texts: Vec<Symbol>,
    lists: Vec<Symbol>,
    structs: Vec<Symbol>,
    closures: Vec<Symbol>,
}

pub struct Generated<'a> {
    pub stmts: Vec<Stmt<'a>>,
}

/// Generate one program. `inject_error` plants exactly one trapping construct.
pub fn generate<'a>(
    seed: u64,
    features: FeatureSet,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    ta: &'a Arena<TypeExpr<'a>>,
    it: &mut Interner,
) -> Generated<'a> {
    let mut rng = SplitMix64::new(seed);
    let show_s = it.intern("show");
    let int_ty: &TypeExpr = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
    let point_sym = it.intern("FzPoint");
    let fx = it.intern("fx");
    let fy = it.intern("fy");
    let int_name = it.intern("Int");

    let mut stmts: Vec<Stmt<'a>> = Vec::new();
    let mut scope = Scope {
        ints: Vec::new(),
        floats: Vec::new(),
        texts: Vec::new(),
        lists: Vec::new(),
        structs: Vec::new(),
        closures: Vec::new(),
    };

    // A reusable helper function when FUNCTIONS is on: dbl(n) = n * 2, plus a
    // bounded-recursion countdown.
    let dbl = it.intern("fzDbl");
    let down = it.intern("fzDown");
    if features.has(FeatureSet::FUNCTIONS) {
        let n = it.intern("n");
        let body: &[Stmt] = sa.alloc_slice(vec![Stmt::Return {
            value: Some(bin(ea, BinaryOpKind::Multiply, ident(ea, n), int(ea, 2))),
        }]);
        stmts.push(fndef(dbl, vec![(n, int_ty)], body));

        let base: &[Stmt] = sa.alloc_slice(vec![Stmt::Return { value: Some(int(ea, 0)) }]);
        let rec_body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::If {
                cond: bin(ea, BinaryOpKind::LtEq, ident(ea, n), int(ea, 0)),
                then_block: base,
                else_block: None,
            },
            Stmt::Return {
                value: Some(call(ea, down, vec![bin(ea, BinaryOpKind::Subtract, ident(ea, n), int(ea, 1))])),
            },
        ]);
        stmts.push(fndef(down, vec![(n, int_ty)], rec_body));
    }
    if features.has(FeatureSet::STRUCTS) {
        stmts.push(Stmt::StructDef {
            name: point_sym,
            fields: vec![(fx, int_name, true), (fy, int_name, true)],
            is_portable: false,
        });
    }

    // Seed bindings.
    for k in 0..3u32 {
        let v = it.intern(&format!("fi{k}"));
        stmts.push(let_mut(v, int(ea, rng.below(7) as i64)));
        scope.ints.push(v);
    }
    if features.has(FeatureSet::FLOAT) {
        let v = it.intern("ff0");
        stmts.push(let_mut(v, float(ea, rng.below(8) as f64 * 0.5)));
        scope.floats.push(v);
    }
    if features.has(FeatureSet::TEXT) {
        let v = it.intern("ft0");
        let t = it.intern("fuzz");
        stmts.push(let_mut(v, ea.alloc(Expr::Literal(Literal::Text(t)))));
        scope.texts.push(v);
    }
    if features.has(FeatureSet::LIST) {
        let v = it.intern("fl0");
        let items: Vec<&Expr> = (0..(1 + rng.below(4))).map(|k| int(ea, k as i64)).collect();
        stmts.push(let_mut(v, ea.alloc(Expr::List(items))));
        scope.lists.push(v);
    }
    if features.has(FeatureSet::STRUCTS) {
        let v = it.intern("fp0");
        stmts.push(let_mut(
            v,
            ea.alloc(Expr::New {
                type_name: point_sym,
                type_args: vec![],
                init_fields: vec![(fx, int(ea, rng.below(5) as i64))],
            }),
        ));
        scope.structs.push(v);
    }
    if features.has(FeatureSet::CLOSURES) {
        let v = it.intern("fc0");
        let captured = scope.ints[rng.below(scope.ints.len() as u64) as usize];
        stmts.push(let_mut(
            v,
            ea.alloc(Expr::Closure {
                params: vec![],
                body: ClosureBody::Expression(bin(
                    ea,
                    BinaryOpKind::Add,
                    ident(ea, captured),
                    int(ea, 1),
                )),
                return_type: None,
            }),
        ));
        scope.closures.push(v);
    }

    // Body statements.
    let body_len = 5 + rng.below(10);
    let error_at = if features.has(FeatureSet::ERROR_INJECTION) {
        Some(rng.below(body_len))
    } else {
        None
    };
    for i in 0..body_len {
        if error_at == Some(i) {
            stmts.push(gen_trap(&mut rng, ea, it, &scope));
            continue;
        }
        gen_stmt(&mut rng, features, ea, sa, it, &mut scope, &mut stmts, dbl, down, show_s, fx, fy);
    }

    // Observe everything live.
    for &v in &scope.ints {
        stmts.push(show_stmt(ea, show_s, ident(ea, v)));
    }
    for &v in &scope.floats {
        stmts.push(show_stmt(ea, show_s, ident(ea, v)));
    }
    for &v in &scope.texts {
        stmts.push(show_stmt(ea, show_s, ident(ea, v)));
    }
    for &v in &scope.lists {
        stmts.push(show_stmt(ea, show_s, len_of(ea, v)));
    }
    for &v in &scope.structs {
        stmts.push(show_stmt(ea, show_s, ea.alloc(Expr::FieldAccess { object: ident(ea, v), field: fx })));
        stmts.push(show_stmt(ea, show_s, ea.alloc(Expr::FieldAccess { object: ident(ea, v), field: fy })));
    }

    Generated { stmts }
}

#[allow(clippy::too_many_arguments)]
fn gen_stmt<'a>(
    rng: &mut SplitMix64,
    features: FeatureSet,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
    scope: &mut Scope,
    out: &mut Vec<Stmt<'a>>,
    dbl: Symbol,
    down: Symbol,
    show_s: Symbol,
    fx: Symbol,
    fy: Symbol,
) {
    match rng.below(12) {
        0 | 1 | 2 => {
            // Mutate an int.
            let v = scope.ints[rng.below(scope.ints.len() as u64) as usize];
            let value = gen_int_expr(rng, features, ea, it, scope, dbl, down);
            out.push(Stmt::Set { target: v, value });
        }
        3 => {
            // New int binding.
            let v = it.intern(&format!("fi{}", scope.ints.len() + 10 + rng.below(90) as usize));
            let value = gen_int_expr(rng, features, ea, it, scope, dbl, down);
            out.push(let_mut(v, value));
            scope.ints.push(v);
        }
        4 if features.has(FeatureSet::IF) => {
            let cond = gen_cond(rng, ea, scope);
            let v = scope.ints[rng.below(scope.ints.len() as u64) as usize];
            let then_blk: &[Stmt] = sa.alloc_slice(vec![Stmt::Set {
                target: v,
                value: gen_int_expr(rng, features, ea, it, scope, dbl, down),
            }]);
            let else_blk = if rng.chance(50) {
                let v2 = scope.ints[rng.below(scope.ints.len() as u64) as usize];
                Some(sa.alloc_slice(vec![Stmt::Set {
                    target: v2,
                    value: gen_int_expr(rng, features, ea, it, scope, dbl, down),
                }]) as &[Stmt])
            } else {
                None
            };
            out.push(Stmt::If { cond, then_block: then_blk, else_block: else_blk });
        }
        5 if features.has(FeatureSet::WHILE) => {
            // Bounded loop: fresh loop var counts to a small literal.
            let lv = it.intern(&format!("fw{}", rng.below(1000)));
            out.push(let_mut(lv, int(ea, 0)));
            let bound = 2 + rng.below(4) as i64;
            let target = scope.ints[rng.below(scope.ints.len() as u64) as usize];
            let mut body_v = vec![
                Stmt::Set {
                    target,
                    value: bin(ea, BinaryOpKind::Add, ident(ea, target), int(ea, 1)),
                },
                Stmt::Set { target: lv, value: bin(ea, BinaryOpKind::Add, ident(ea, lv), int(ea, 1)) },
            ];
            if features.has(FeatureSet::BREAK) && rng.chance(25) {
                let brk_blk: &[Stmt] = sa.alloc_slice(vec![Stmt::Break]);
                body_v.insert(
                    0,
                    Stmt::If {
                        cond: bin(ea, BinaryOpKind::GtEq, ident(ea, lv), int(ea, bound - 1)),
                        then_block: brk_blk,
                        else_block: None,
                    },
                );
            }
            let body: &[Stmt] = sa.alloc_slice(body_v);
            out.push(Stmt::While {
                cond: bin(ea, BinaryOpKind::Lt, ident(ea, lv), int(ea, bound)),
                body,
                decreasing: None,
            });
        }
        6 if features.has(FeatureSet::REPEAT) && !scope.lists.is_empty() => {
            let list = scope.lists[rng.below(scope.lists.len() as u64) as usize];
            let x = it.intern(&format!("fr{}", rng.below(1000)));
            let target = scope.ints[rng.below(scope.ints.len() as u64) as usize];
            let body: &[Stmt] = sa.alloc_slice(vec![Stmt::Set {
                target,
                value: bin(ea, BinaryOpKind::Add, ident(ea, target), ident(ea, x)),
            }]);
            out.push(Stmt::Repeat { pattern: Pattern::Identifier(x), iterable: ident(ea, list), body });
        }
        7 if features.has(FeatureSet::LIST) && !scope.lists.is_empty() => {
            let list = scope.lists[rng.below(scope.lists.len() as u64) as usize];
            let value = gen_int_expr(rng, features, ea, it, scope, dbl, down);
            out.push(Stmt::Push { value, collection: ident(ea, list) });
        }
        8 if features.has(FeatureSet::STRUCTS) && !scope.structs.is_empty() => {
            let sv = scope.structs[rng.below(scope.structs.len() as u64) as usize];
            let field = if rng.chance(50) { fx } else { fy };
            let value = gen_int_expr(rng, features, ea, it, scope, dbl, down);
            out.push(Stmt::SetField { object: ident(ea, sv), field, value });
        }
        9 if features.has(FeatureSet::CLOSURES) && !scope.closures.is_empty() => {
            let c = scope.closures[rng.below(scope.closures.len() as u64) as usize];
            out.push(show_stmt(ea, show_s, ea.alloc(Expr::CallExpr { callee: ident(ea, c), args: vec![] })));
        }
        10 if features.has(FeatureSet::INTERPOLATION) => {
            use crate::ast::stmt::StringPart;
            let v = scope.ints[rng.below(scope.ints.len() as u64) as usize];
            if features.has(FeatureSet::TEXT) && !scope.texts.is_empty() && rng.chance(50) {
                // SELF-REFERENTIAL accumulation — the register-hazard class
                // the PE corpus caught: `Set t to "{t}{v}"`.
                let t = scope.texts[rng.below(scope.texts.len() as u64) as usize];
                let parts = vec![
                    StringPart::Expr { value: ident(ea, t), format_spec: None, debug: false },
                    StringPart::Expr { value: ident(ea, v), format_spec: None, debug: false },
                ];
                out.push(Stmt::Set { target: t, value: ea.alloc(Expr::InterpolatedString(parts)) });
            } else {
                let lit = it.intern("v=");
                let parts = vec![
                    StringPart::Literal(lit),
                    StringPart::Expr { value: ident(ea, v), format_spec: None, debug: false },
                ];
                out.push(show_stmt(ea, show_s, ea.alloc(Expr::InterpolatedString(parts))));
            }
        }
        _ => {
            // Show something.
            let e = gen_int_expr(rng, features, ea, it, scope, dbl, down);
            out.push(show_stmt(ea, show_s, e));
        }
    }
}

fn gen_int_expr<'a>(
    rng: &mut SplitMix64,
    features: FeatureSet,
    ea: &'a Arena<Expr<'a>>,
    it: &mut Interner,
    scope: &Scope,
    dbl: Symbol,
    down: Symbol,
) -> &'a Expr<'a> {
    let atom = |rng: &mut SplitMix64, ea: &'a Arena<Expr<'a>>| -> &'a Expr<'a> {
        if rng.chance(50) {
            int(ea, rng.below(9) as i64)
        } else {
            ident(ea, scope.ints[rng.below(scope.ints.len() as u64) as usize])
        }
    };
    match rng.below(10) {
        0 | 1 | 2 => bin(ea, BinaryOpKind::Add, atom(rng, ea), atom(rng, ea)),
        3 => bin(ea, BinaryOpKind::Subtract, atom(rng, ea), atom(rng, ea)),
        4 => bin(ea, BinaryOpKind::Multiply, atom(rng, ea), int(ea, rng.below(4) as i64)),
        5 => bin(ea, BinaryOpKind::Divide, atom(rng, ea), int(ea, 1 + rng.below(5) as i64)),
        6 => bin(ea, BinaryOpKind::Modulo, atom(rng, ea), int(ea, 1 + rng.below(5) as i64)),
        7 if features.has(FeatureSet::FUNCTIONS) => call(ea, dbl, vec![atom(rng, ea)]),
        8 if features.has(FeatureSet::FUNCTIONS) => call(ea, down, vec![int(ea, rng.below(6) as i64)]),
        9 if features.has(FeatureSet::BUILTINS) => {
            let abs_s = it.intern("abs");
            call(ea, abs_s, vec![bin(ea, BinaryOpKind::Subtract, int(ea, 0), atom(rng, ea))])
        }
        _ => atom(rng, ea),
    }
}

fn gen_cond<'a>(rng: &mut SplitMix64, ea: &'a Arena<Expr<'a>>, scope: &Scope) -> &'a Expr<'a> {
    let l = ident(ea, scope.ints[rng.below(scope.ints.len() as u64) as usize]);
    let r = int(ea, rng.below(8) as i64);
    let op = match rng.below(6) {
        0 => BinaryOpKind::Lt,
        1 => BinaryOpKind::Gt,
        2 => BinaryOpKind::LtEq,
        3 => BinaryOpKind::GtEq,
        4 => BinaryOpKind::Eq,
        _ => BinaryOpKind::NotEq,
    };
    bin(ea, op, l, r)
}

/// One trapping construct — the engines must agree on partial output + error.
fn gen_trap<'a>(
    rng: &mut SplitMix64,
    ea: &'a Arena<Expr<'a>>,
    it: &mut Interner,
    scope: &Scope,
) -> Stmt<'a> {
    let v = scope.ints[rng.below(scope.ints.len() as u64) as usize];
    match rng.below(5) {
        0 => Stmt::Set { target: v, value: bin(ea, BinaryOpKind::Divide, ident(ea, v), int(ea, 0)) },
        1 => Stmt::Set { target: v, value: bin(ea, BinaryOpKind::Modulo, ident(ea, v), int(ea, 0)) },
        2 if !scope.lists.is_empty() => {
            let list = scope.lists[rng.below(scope.lists.len() as u64) as usize];
            Stmt::Set {
                target: v,
                value: ea.alloc(Expr::Index { collection: ident(ea, list), index: int(ea, 999) }),
            }
        }
        3 => Stmt::Set {
            target: it.intern(&format!("fzGhost{}", rng.below(1000))),
            value: int(ea, 1),
        },
        _ => Stmt::Set {
            target: v,
            value: bin(ea, BinaryOpKind::Multiply, ea.alloc(Expr::Literal(Literal::Boolean(true))), int(ea, 1)),
        },
    }
}

// ---- AST shorthands ---------------------------------------------------------

fn int<'a>(ea: &'a Arena<Expr<'a>>, n: i64) -> &'a Expr<'a> {
    ea.alloc(Expr::Literal(Literal::Number(n)))
}
fn float<'a>(ea: &'a Arena<Expr<'a>>, f: f64) -> &'a Expr<'a> {
    ea.alloc(Expr::Literal(Literal::Float(f)))
}
fn ident<'a>(ea: &'a Arena<Expr<'a>>, s: Symbol) -> &'a Expr<'a> {
    ea.alloc(Expr::Identifier(s))
}
fn bin<'a>(
    ea: &'a Arena<Expr<'a>>,
    op: BinaryOpKind,
    l: &'a Expr<'a>,
    r: &'a Expr<'a>,
) -> &'a Expr<'a> {
    ea.alloc(Expr::BinaryOp { op, left: l, right: r })
}
fn call<'a>(ea: &'a Arena<Expr<'a>>, f: Symbol, args: Vec<&'a Expr<'a>>) -> &'a Expr<'a> {
    ea.alloc(Expr::Call { function: f, args })
}
fn len_of<'a>(ea: &'a Arena<Expr<'a>>, v: Symbol) -> &'a Expr<'a> {
    ea.alloc(Expr::Length { collection: ident(ea, v) })
}
fn let_mut<'a>(var: Symbol, value: &'a Expr<'a>) -> Stmt<'a> {
    Stmt::Let { var, ty: None, value, mutable: true }
}
fn show_stmt<'a>(ea: &'a Arena<Expr<'a>>, show_s: Symbol, e: &'a Expr<'a>) -> Stmt<'a> {
    Stmt::Show { object: e, recipient: ident(ea, show_s) }
}
fn fndef<'a>(
    name: Symbol,
    params: Vec<(Symbol, &'a TypeExpr<'a>)>,
    body: &'a [Stmt<'a>],
) -> Stmt<'a> {
    Stmt::FunctionDef {
        name,
        generics: vec![],
        params,
        body,
        return_type: None,
        is_native: false,
        native_path: None,
        is_exported: false,
        export_target: None,
        opt_flags: std::collections::HashSet::new(),
    }
}
