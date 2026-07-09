//! Fixed-size float/int `Seq` scalarization for the RUN path (the interpreter's
//! analog of LLVM's SROA).
//!
//! Motivation (nbody): the kernel keeps its five bodies in seven fixed-size
//! (length-5) `Seq of Float`s, each element read as `item k of arr` — a
//! bounds-checked, reference-counted heap load. After the run-path unroller
//! turns every index into a compile-time constant, those Seqs are touched only
//! at constant offsets, so they carry no information the array representation
//! adds: each element is just a named scalar. This pass replaces such a Seq with
//! `N` scalar locals (`arr$1 … arr$N`), turning every `item k of arr` into a
//! plain variable read the JIT's register allocator can keep in an XMM register.
//! Unrolling alone was a measured loss (a bigger region over the same heap
//! loads); scalarize is the half that pays — it removes the loads.
//!
//! **Conservative by construction.** It reuses `collect_scalarizable_seqs`,
//! which already proves a Seq is push-built, fixed-size, never resized, aliased,
//! escaped, or passed anywhere — its only uses are index reads, `SetIndex`
//! writes, length queries, and its initial pushes. On top of that this pass adds
//! the scalarization-specific requirement that *every* index is a CONSTANT in
//! `1..=N` and that the Seq's length is never queried (a scalar set has no
//! length). Any Seq that fails either check is left entirely untouched.
//!
//! **Value-preserving.** Element `k` (1-based) maps to the value of the `k`-th
//! `Push v to arr` — exactly the position the 1-based `item k of arr` would
//! read. No arithmetic is reassociated; reads and writes keep their order.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{Expr, MatchArm, Stmt};
use crate::codegen::detection::collect_scalarizable_seqs;
use crate::intern::{Interner, Symbol};

/// Replace qualifying fixed-size `Seq` locals with `N` scalar variables.
/// Returns the (possibly) rewritten statements and whether anything changed.
/// When nothing qualifies the ORIGINAL statements are returned untouched, so
/// the pass is a guaranteed no-op on programs without a scalarizable Seq.
pub fn scalarize_seqs<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let cands = collect_scalarizable_seqs(&stmts, interner);
    if cands.is_empty() {
        return (stmts, false);
    }

    // Among the detection candidates keep only those whose EVERY index is a
    // constant in range and whose length is never queried — the extra
    // requirements scalarization (but not the `[T;N]` codegen) imposes.
    let mut disqualified: HashSet<Symbol> = HashSet::new();
    let lens: HashMap<Symbol, usize> = cands.iter().map(|(s, c)| (*s, c.len)).collect();
    scan_index_constraints(&stmts, &lens, &mut disqualified);

    let qualified: HashMap<Symbol, usize> = lens
        .iter()
        .filter(|(s, _)| !disqualified.contains(s))
        .map(|(s, n)| (*s, *n))
        .collect();
    if qualified.is_empty() {
        return (stmts, false);
    }

    // Intern the `arr$k` scalar names (1-based, matching `item k of arr`).
    let mut scalar_names: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
    for (&sym, &n) in &qualified {
        let base = interner.resolve(sym).to_string();
        let names: Vec<Symbol> =
            (1..=n).map(|k| interner.intern(&format!("{base}${k}"))).collect();
        scalar_names.insert(sym, names);
    }

    let out = rewrite_block(stmts, &qualified, &scalar_names, expr_arena, stmt_arena);
    (out, true)
}

/// The element `k` (1-based) of `arr` is defined by the `k`-th `Push v to arr`.
/// `push_counter` tracks, per array, how many pushes have been rewritten so far
/// so the next push targets the next scalar.
struct Rewriter<'a, 'q> {
    qualified: &'q HashMap<Symbol, usize>,
    scalar_names: &'q HashMap<Symbol, Vec<Symbol>>,
    push_counter: HashMap<Symbol, usize>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
}

fn rewrite_block<'a>(
    stmts: Vec<Stmt<'a>>,
    qualified: &HashMap<Symbol, usize>,
    scalar_names: &HashMap<Symbol, Vec<Symbol>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut rw = Rewriter {
        qualified,
        scalar_names,
        push_counter: HashMap::new(),
        expr_arena,
        stmt_arena,
    };
    rw.rewrite_stmts(stmts)
}

impl<'a, 'q> Rewriter<'a, 'q> {
    fn rewrite_stmts(&mut self, stmts: Vec<Stmt<'a>>) -> Vec<Stmt<'a>> {
        let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            self.rewrite_stmt(stmt, &mut out);
        }
        out
    }

    fn rewrite_block_ref(&mut self, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        let v = self.rewrite_stmts(block.to_vec());
        self.stmt_arena.alloc_slice(v)
    }

    fn rewrite_stmt(&mut self, stmt: Stmt<'a>, out: &mut Vec<Stmt<'a>>) {
        match stmt {
            // `Let mutable arr be a new Seq …` is dropped: the element scalars
            // are introduced directly at the array's pushes (below), so the
            // empty-Seq declaration has nothing left to bind.
            Stmt::Let { var, .. } if self.qualified.contains_key(&var) => {}
            // `Push v to arr` → `Let mutable arr$k be v` (k = push order). The
            // detection proves every read of element k follows its push, so the
            // defining `Let` always precedes — element k is in scope and carries
            // its real initial value (no separate placeholder/`Set`, which a
            // later CSE pass over the now-scalar reads would value-number against
            // its initializer and fold across a loop-carried mutation).
            Stmt::Push { value, collection }
                if matches!(collection, Expr::Identifier(s) if self.qualified.contains_key(s)) =>
            {
                let sym = match collection {
                    Expr::Identifier(s) => *s,
                    _ => unreachable!(),
                };
                let k = self.push_counter.entry(sym).or_insert(0);
                let target = self.scalar_names[&sym][*k];
                *k += 1;
                let rewritten = self.rewrite_expr(value);
                out.push(Stmt::Let { var: target, ty: None, value: rewritten, mutable: true });
            }
            // `Set item k of arr to x` → `Set arr$k to x`.
            Stmt::SetIndex { collection, index, value }
                if self.scalar_target(collection, index).is_some() =>
            {
                let target = self.scalar_target(collection, index).unwrap();
                let rewritten = self.rewrite_expr(value);
                out.push(Stmt::Set { target, value: rewritten });
            }
            Stmt::Let { var, ty, value, mutable } => {
                out.push(Stmt::Let { var, ty, value: self.rewrite_expr(value), mutable });
            }
            Stmt::Set { target, value } => {
                out.push(Stmt::Set { target, value: self.rewrite_expr(value) });
            }
            Stmt::SetIndex { collection, index, value } => {
                out.push(Stmt::SetIndex {
                    collection: self.rewrite_expr(collection),
                    index: self.rewrite_expr(index),
                    value: self.rewrite_expr(value),
                });
            }
            Stmt::Push { value, collection } => {
                out.push(Stmt::Push {
                    value: self.rewrite_expr(value),
                    collection: self.rewrite_expr(collection),
                });
            }
            Stmt::Show { object, recipient } => {
                out.push(Stmt::Show {
                    object: self.rewrite_expr(object),
                    recipient: self.rewrite_expr(recipient),
                });
            }
            Stmt::Return { value } => {
                out.push(Stmt::Return { value: value.map(|v| self.rewrite_expr(v)) });
            }
            Stmt::RuntimeAssert { condition, hard } => {
                out.push(Stmt::RuntimeAssert { condition: self.rewrite_expr(condition) , hard });
            }
            Stmt::Call { function, args } => {
                let args = args.into_iter().map(|a| self.rewrite_expr(a)).collect();
                out.push(Stmt::Call { function, args });
            }
            Stmt::If { cond, then_block, else_block } => {
                out.push(Stmt::If {
                    cond: self.rewrite_expr(cond),
                    then_block: self.rewrite_block_ref(then_block),
                    else_block: else_block.map(|b| self.rewrite_block_ref(b)),
                });
            }
            Stmt::While { cond, body, decreasing } => {
                out.push(Stmt::While {
                    cond: self.rewrite_expr(cond),
                    body: self.rewrite_block_ref(body),
                    decreasing: decreasing.map(|d| self.rewrite_expr(d)),
                });
            }
            Stmt::Repeat { pattern, iterable, body } => {
                out.push(Stmt::Repeat {
                    pattern,
                    iterable: self.rewrite_expr(iterable),
                    body: self.rewrite_block_ref(body),
                });
            }
            Stmt::Inspect { target, arms, has_otherwise } => {
                let arms = arms
                    .into_iter()
                    .map(|a| MatchArm {
                        enum_name: a.enum_name,
                        variant: a.variant,
                        bindings: a.bindings,
                        body: self.rewrite_block_ref(a.body),
                    })
                    .collect();
                out.push(Stmt::Inspect { target: self.rewrite_expr(target), arms, has_otherwise });
            }
            Stmt::Zone { name, capacity, source_file, body } => {
                out.push(Stmt::Zone {
                    name,
                    capacity,
                    source_file,
                    body: self.rewrite_block_ref(body),
                });
            }
            other => out.push(other),
        }
    }

    /// If `collection`/`index` is a constant index `item k of arr` on a
    /// qualified array, the target scalar `arr$k`; else None.
    fn scalar_target(&self, collection: &Expr<'a>, index: &Expr<'a>) -> Option<Symbol> {
        let sym = match collection {
            Expr::Identifier(s) if self.qualified.contains_key(s) => *s,
            _ => return None,
        };
        let k = const_index(index)?;
        let n = self.qualified[&sym];
        if k >= 1 && k <= n as i64 {
            Some(self.scalar_names[&sym][(k - 1) as usize])
        } else {
            None
        }
    }

    fn rewrite_expr(&self, expr: &'a Expr<'a>) -> &'a Expr<'a> {
        match expr {
            // `item k of arr` (constant k) → `arr$k`.
            Expr::Index { collection, index } => {
                if let Some(target) = self.scalar_target(collection, index) {
                    return self.expr_arena.alloc(Expr::Identifier(target));
                }
                self.expr_arena.alloc(Expr::Index {
                    collection: self.rewrite_expr(collection),
                    index: self.rewrite_expr(index),
                })
            }
            Expr::BinaryOp { op, left, right } => self.expr_arena.alloc(Expr::BinaryOp {
                op: *op,
                left: self.rewrite_expr(left),
                right: self.rewrite_expr(right),
            }),
            Expr::Not { operand } => {
                self.expr_arena.alloc(Expr::Not { operand: self.rewrite_expr(operand) })
            }
            Expr::Call { function, args } => self.expr_arena.alloc(Expr::Call {
                function: *function,
                args: args.iter().map(|a| self.rewrite_expr(a)).collect(),
            }),
            Expr::CallExpr { callee, args } => self.expr_arena.alloc(Expr::CallExpr {
                callee: self.rewrite_expr(callee),
                args: args.iter().map(|a| self.rewrite_expr(a)).collect(),
            }),
            Expr::Slice { collection, start, end } => self.expr_arena.alloc(Expr::Slice {
                collection: self.rewrite_expr(collection),
                start: self.rewrite_expr(start),
                end: self.rewrite_expr(end),
            }),
            Expr::Length { collection } => self
                .expr_arena
                .alloc(Expr::Length { collection: self.rewrite_expr(collection) }),
            Expr::Copy { expr } => {
                self.expr_arena.alloc(Expr::Copy { expr: self.rewrite_expr(expr) })
            }
            Expr::Give { value } => {
                self.expr_arena.alloc(Expr::Give { value: self.rewrite_expr(value) })
            }
            Expr::Contains { collection, value } => self.expr_arena.alloc(Expr::Contains {
                collection: self.rewrite_expr(collection),
                value: self.rewrite_expr(value),
            }),
            Expr::Union { left, right } => self.expr_arena.alloc(Expr::Union {
                left: self.rewrite_expr(left),
                right: self.rewrite_expr(right),
            }),
            Expr::Intersection { left, right } => self.expr_arena.alloc(Expr::Intersection {
                left: self.rewrite_expr(left),
                right: self.rewrite_expr(right),
            }),
            Expr::Range { start, end } => self.expr_arena.alloc(Expr::Range {
                start: self.rewrite_expr(start),
                end: self.rewrite_expr(end),
            }),
            Expr::FieldAccess { object, field } => self.expr_arena.alloc(Expr::FieldAccess {
                object: self.rewrite_expr(object),
                field: *field,
            }),
            Expr::List(items) => self
                .expr_arena
                .alloc(Expr::List(items.iter().map(|i| self.rewrite_expr(i)).collect())),
            Expr::Tuple(items) => self
                .expr_arena
                .alloc(Expr::Tuple(items.iter().map(|i| self.rewrite_expr(i)).collect())),
            Expr::OptionSome { value } => {
                self.expr_arena.alloc(Expr::OptionSome { value: self.rewrite_expr(value) })
            }
            Expr::WithCapacity { value, capacity } => self.expr_arena.alloc(Expr::WithCapacity {
                value: self.rewrite_expr(value),
                capacity: self.rewrite_expr(capacity),
            }),
            Expr::InterpolatedString(parts) => {
                let parts = parts
                    .iter()
                    .map(|p| match p {
                        crate::ast::stmt::StringPart::Expr { value, format_spec, debug } => {
                            crate::ast::stmt::StringPart::Expr {
                                value: self.rewrite_expr(value),
                                format_spec: *format_spec,
                                debug: *debug,
                            }
                        }
                        crate::ast::stmt::StringPart::Literal(s) => {
                            crate::ast::stmt::StringPart::Literal(*s)
                        }
                    })
                    .collect();
                self.expr_arena.alloc(Expr::InterpolatedString(parts))
            }
            // Leaves and forms that cannot transitively hold a qualified index
            // (their child positions are guaranteed clear because the detection
            // already disqualifies any array appearing in them) — returned as-is.
            other => other,
        }
    }
}

/// The constant value of an index expression (1-based), if it folds to an
/// integer literal — including the `k - 1` shapes the unroller can leave.
fn const_index(expr: &Expr) -> Option<i64> {
    crate::loop_shape::const_eval_i64(expr)
}

// ---------------------------------------------------------------------------
// Scalarization-specific disqualification: non-constant / out-of-range index,
// or any length query.
// ---------------------------------------------------------------------------

fn scan_index_constraints(
    stmts: &[Stmt],
    lens: &HashMap<Symbol, usize>,
    disq: &mut HashSet<Symbol>,
) {
    for s in stmts {
        scan_stmt(s, lens, disq);
    }
}

fn scan_stmt(s: &Stmt, lens: &HashMap<Symbol, usize>, disq: &mut HashSet<Symbol>) {
    match s {
        Stmt::SetIndex { collection, index, value } => {
            check_index(collection, index, lens, disq);
            scan_expr(index, lens, disq);
            scan_expr(value, lens, disq);
        }
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => scan_expr(value, lens, disq),
        Stmt::Push { value, collection } => {
            scan_expr(value, lens, disq);
            scan_expr(collection, lens, disq);
        }
        Stmt::Show { object, recipient } => {
            scan_expr(object, lens, disq);
            scan_expr(recipient, lens, disq);
        }
        Stmt::Give { object, recipient } => {
            scan_expr(object, lens, disq);
            scan_expr(recipient, lens, disq);
        }
        Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            scan_expr(value, lens, disq);
            scan_expr(collection, lens, disq);
        }
        Stmt::SetField { object, value, .. } => {
            scan_expr(object, lens, disq);
            scan_expr(value, lens, disq);
        }
        Stmt::RuntimeAssert { condition, .. } => scan_expr(condition, lens, disq),
        Stmt::Return { value } => {
            if let Some(v) = value {
                scan_expr(v, lens, disq);
            }
        }
        Stmt::Call { args, .. } => {
            for a in args {
                scan_expr(a, lens, disq);
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            scan_expr(cond, lens, disq);
            scan_index_constraints(then_block, lens, disq);
            if let Some(eb) = else_block {
                scan_index_constraints(eb, lens, disq);
            }
        }
        Stmt::While { cond, body, decreasing } => {
            scan_expr(cond, lens, disq);
            if let Some(d) = decreasing {
                scan_expr(d, lens, disq);
            }
            scan_index_constraints(body, lens, disq);
        }
        Stmt::Repeat { iterable, body, .. } => {
            scan_expr(iterable, lens, disq);
            scan_index_constraints(body, lens, disq);
        }
        Stmt::Inspect { target, arms, .. } => {
            scan_expr(target, lens, disq);
            for a in arms {
                scan_index_constraints(a.body, lens, disq);
            }
        }
        Stmt::Zone { body, .. } => scan_index_constraints(body, lens, disq),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            scan_index_constraints(tasks, lens, disq)
        }
        _ => {}
    }
}

/// A read/write at `item index of collection`: if `collection` is a candidate
/// array, its index must be a constant in `1..=N`, else disqualify.
fn check_index(
    collection: &Expr,
    index: &Expr,
    lens: &HashMap<Symbol, usize>,
    disq: &mut HashSet<Symbol>,
) {
    if let Expr::Identifier(sym) = collection {
        if let Some(&n) = lens.get(sym) {
            match const_index(index) {
                Some(k) if k >= 1 && k <= n as i64 => {}
                _ => {
                    disq.insert(*sym);
                }
            }
        }
    }
}

fn scan_expr(e: &Expr, lens: &HashMap<Symbol, usize>, disq: &mut HashSet<Symbol>) {
    match e {
        Expr::Index { collection, index } => {
            check_index(collection, index, lens, disq);
            scan_expr(collection, lens, disq);
            scan_expr(index, lens, disq);
        }
        // A scalar set has no length — a `length of arr` query cannot be
        // honored, so any candidate it touches is disqualified.
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                if lens.contains_key(sym) {
                    disq.insert(*sym);
                }
            }
            scan_expr(collection, lens, disq);
        }
        Expr::Slice { collection, start, end } => {
            scan_expr(collection, lens, disq);
            scan_expr(start, lens, disq);
            scan_expr(end, lens, disq);
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            scan_expr(left, lens, disq);
            scan_expr(right, lens, disq);
        }
        Expr::Contains { collection, value } => {
            scan_expr(collection, lens, disq);
            scan_expr(value, lens, disq);
        }
        Expr::Not { operand } => scan_expr(operand, lens, disq),
        Expr::FieldAccess { object, .. } => scan_expr(object, lens, disq),
        Expr::Copy { expr } => scan_expr(expr, lens, disq),
        Expr::Give { value } | Expr::OptionSome { value } => scan_expr(value, lens, disq),
        Expr::WithCapacity { value, capacity } => {
            scan_expr(value, lens, disq);
            scan_expr(capacity, lens, disq);
        }
        Expr::Call { args, .. } | Expr::CallExpr { args, .. } => {
            for a in args {
                scan_expr(a, lens, disq);
            }
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items {
                scan_expr(i, lens, disq);
            }
        }
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    scan_expr(value, lens, disq);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::stmt::{BinaryOpKind, Literal, TypeExpr};

    struct B<'a> {
        ea: &'a Arena<Expr<'a>>,
    }
    impl<'a> B<'a> {
        fn id(&self, s: Symbol) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Identifier(s))
        }
        fn num(&self, n: i64) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Literal(Literal::Number(n)))
        }
        fn fnum(&self, n: f64) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Literal(Literal::Float(n)))
        }
        fn bin(&self, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
            self.ea.alloc(Expr::BinaryOp { op, left: l, right: r })
        }
        fn index(&self, coll: &'a Expr<'a>, idx: &'a Expr<'a>) -> &'a Expr<'a> {
            self.ea.alloc(Expr::Index { collection: coll, index: idx })
        }
    }

    fn run<'a>(
        input: Vec<Stmt<'a>>,
        ea: &'a Arena<Expr<'a>>,
        sa: &'a Arena<Stmt<'a>>,
        it: &mut Interner,
    ) -> (Vec<Stmt<'a>>, bool) {
        scalarize_seqs(input, ea, sa, it)
    }

    /// End-to-end: a length-2 float Seq built by two pushes, then read at
    /// constant indices and written at a constant index, fully scalarizes.
    #[test]
    fn scalarizes_const_indexed_float_seq() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let float = it.intern("Float");
        let arr = it.intern("arr");
        let sink = it.intern("sink");
        let b = B { ea: &ea };

        let decl = Stmt::Let {
            var: arr,
            ty: None,
            value: ea.alloc(Expr::New {
                type_name: seq,
                type_args: vec![TypeExpr::Primitive(float)],
                init_fields: vec![],
            }),
            mutable: true,
        };
        let push1 = Stmt::Push { value: b.fnum(1.5), collection: b.id(arr) };
        let push2 = Stmt::Push { value: b.fnum(2.5), collection: b.id(arr) };
        // sink = item 1 of arr + item 2 of arr
        let read = Stmt::Let {
            var: sink,
            ty: None,
            value: b.bin(BinaryOpKind::Add, b.index(b.id(arr), b.num(1)), b.index(b.id(arr), b.num(2))),
            mutable: true,
        };
        // Set item 1 of arr to 9.0
        let write = Stmt::SetIndex {
            collection: b.id(arr),
            index: b.num(1),
            value: b.fnum(9.0),
        };

        let input = vec![decl, push1, push2, read, write];
        let (out, changed) = run(input, &ea, &sa, &mut it);
        assert!(changed, "scalarization should fire");

        let a1 = it.intern("arr$1");
        let a2 = it.intern("arr$2");

        // The `new Seq` Let is dropped; the two pushes become the element
        // initializers `Let mutable arr$1 be 1.5`, `Let mutable arr$2 be 2.5`.
        match &out[0] {
            Stmt::Let { var, mutable: true, value, .. } => {
                assert_eq!(*var, a1);
                assert!(matches!(value, Expr::Literal(Literal::Float(f)) if *f == 1.5));
            }
            other => panic!("expected Let arr$1, got {other:?}"),
        }
        match &out[1] {
            Stmt::Let { var, mutable: true, value, .. } => {
                assert_eq!(*var, a2);
                assert!(matches!(value, Expr::Literal(Literal::Float(f)) if *f == 2.5));
            }
            other => panic!("expected Let arr$2, got {other:?}"),
        }
        // The read uses arr$1 + arr$2, no Index left.
        match &out[2] {
            Stmt::Let { value, .. } => match value {
                Expr::BinaryOp { left, right, .. } => {
                    assert!(matches!(left, Expr::Identifier(s) if *s == a1));
                    assert!(matches!(right, Expr::Identifier(s) if *s == a2));
                }
                other => panic!("expected BinaryOp, got {other:?}"),
            },
            other => panic!("expected Let, got {other:?}"),
        }
        // SetIndex became `Set arr$1 to 9.0`.
        assert!(matches!(out[3], Stmt::Set { target, .. } if target == a1));
        // No Seq declaration, push, or index survives.
        assert_eq!(out.len(), 4);
    }

    /// A non-constant index disqualifies the whole array — left untouched.
    #[test]
    fn variable_index_is_left_alone() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let float = it.intern("Float");
        let arr = it.intern("arr");
        let i = it.intern("i");
        let sink = it.intern("sink");
        let b = B { ea: &ea };

        let decl = Stmt::Let {
            var: arr,
            ty: None,
            value: ea.alloc(Expr::New {
                type_name: seq,
                type_args: vec![TypeExpr::Primitive(float)],
                init_fields: vec![],
            }),
            mutable: true,
        };
        let push1 = Stmt::Push { value: b.fnum(1.5), collection: b.id(arr) };
        let push2 = Stmt::Push { value: b.fnum(2.5), collection: b.id(arr) };
        // sink = item i of arr  (variable index → disqualify)
        let read = Stmt::Let {
            var: sink,
            ty: None,
            value: b.index(b.id(arr), b.id(i)),
            mutable: true,
        };
        let input = vec![decl, push1, push2, read];
        let (out, changed) = run(input, &ea, &sa, &mut it);
        assert!(!changed, "variable index must block scalarization");
        // The Seq declaration is preserved verbatim.
        assert!(matches!(out[0], Stmt::Let { var, .. } if var == arr));
    }

    /// A `length of arr` query disqualifies — a scalar set has no length.
    #[test]
    fn length_query_is_left_alone() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let float = it.intern("Float");
        let arr = it.intern("arr");
        let sink = it.intern("sink");
        let b = B { ea: &ea };

        let decl = Stmt::Let {
            var: arr,
            ty: None,
            value: ea.alloc(Expr::New {
                type_name: seq,
                type_args: vec![TypeExpr::Primitive(float)],
                init_fields: vec![],
            }),
            mutable: true,
        };
        let push1 = Stmt::Push { value: b.fnum(1.5), collection: b.id(arr) };
        let read = Stmt::Let {
            var: sink,
            ty: None,
            value: ea.alloc(Expr::Length { collection: b.id(arr) }),
            mutable: true,
        };
        let input = vec![decl, push1, read];
        let (_out, changed) = run(input, &ea, &sa, &mut it);
        assert!(!changed, "length query must block scalarization");
    }

    /// The `k - 1` index shape the unroller leaves still folds to a constant.
    #[test]
    fn folded_constant_index_qualifies() {
        let ea = Arena::new();
        let sa = Arena::new();
        let mut it = Interner::new();
        let seq = it.intern("Seq");
        let float = it.intern("Float");
        let arr = it.intern("arr");
        let sink = it.intern("sink");
        let b = B { ea: &ea };

        let decl = Stmt::Let {
            var: arr,
            ty: None,
            value: ea.alloc(Expr::New {
                type_name: seq,
                type_args: vec![TypeExpr::Primitive(float)],
                init_fields: vec![],
            }),
            mutable: true,
        };
        let push1 = Stmt::Push { value: b.fnum(1.5), collection: b.id(arr) };
        let push2 = Stmt::Push { value: b.fnum(2.5), collection: b.id(arr) };
        // item (3 - 1) of arr  →  arr$2
        let read = Stmt::Let {
            var: sink,
            ty: None,
            value: b.index(b.id(arr), b.bin(BinaryOpKind::Subtract, b.num(3), b.num(1))),
            mutable: true,
        };
        let input = vec![decl, push1, push2, read];
        let (out, changed) = run(input, &ea, &sa, &mut it);
        assert!(changed, "constant-folding index must scalarize");
        let a2 = it.intern("arr$2");
        match out.last().unwrap() {
            Stmt::Let { value, .. } => {
                assert!(matches!(value, Expr::Identifier(s) if *s == a2));
            }
            other => panic!("expected Let, got {other:?}"),
        }
    }
}
