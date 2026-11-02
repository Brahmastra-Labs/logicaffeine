//! Task C: hoisting loop-invariant `item i of arr` reads out of an inner
//! loop. The structural proofs build a nbody-shaped inner `While j` and
//! assert the invariant load is lifted into a guarded preheader, plus the
//! soundness guards: a MUTATED collection is never hoisted, a variant index
//! is never hoisted, and a side-effecting loop condition disables the lift.
use super::*;
use crate::ast::stmt::{BinaryOpKind, Literal};

struct B<'a> {
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
}
impl<'a> B<'a> {
    fn id(&self, s: Symbol) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Identifier(s))
    }
    fn num(&self, n: i64) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Literal(Literal::Number(n)))
    }
    fn bin(&self, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
        self.ea.alloc(Expr::BinaryOp {
            op,
            left: l,
            right: r,
        })
    }
    fn index(&self, coll: &'a Expr<'a>, idx: &'a Expr<'a>) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Index {
            collection: coll,
            index: idx,
        })
    }
    fn block(&self, v: Vec<Stmt<'a>>) -> Block<'a> {
        self.sa.alloc_slice(v)
    }
}

struct Names {
    i: Symbol,
    j: Symbol,
    bx: Symbol,
    bvx: Symbol,
    acc: Symbol,
    dx: Symbol,
}
fn names(it: &mut Interner) -> Names {
    Names {
        i: it.intern("i"),
        j: it.intern("j"),
        bx: it.intern("bx"),
        bvx: it.intern("bvx"),
        acc: it.intern("acc"),
        dx: it.intern("dx"),
    }
}

/// Count `Let … be item _ of <coll>` bindings (a bare indexed-load Let,
/// which is exactly the shape the hoist mints) in a statement tree.
fn count_hoisted(stmts: &[Stmt], coll: Symbol) -> usize {
    stmts
        .iter()
        .map(|s| match s {
            Stmt::Let { value, .. } => matches!(
                value,
                Expr::Index { collection, .. }
                    if matches!(&**collection, Expr::Identifier(c) if *c == coll)
            ) as usize,
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                count_hoisted(then_block, coll) + else_block.map_or(0, |eb| count_hoisted(eb, coll))
            }
            Stmt::While { body, .. } => count_hoisted(body, coll),
            _ => 0,
        })
        .sum()
}

/// Count every `item _ of <coll>` indexed read across the statement tree
/// (including nested Let/Set/If/While bodies and identifiers used as
/// collections), so a hoist that DUPLICATES a read is caught.
fn count_item_reads(stmts: &[Stmt], coll: Symbol) -> usize {
    fn in_expr(e: &Expr, coll: Symbol) -> usize {
        match e {
            Expr::Index { collection, index } => {
                let here = matches!(&**collection, Expr::Identifier(c) if *c == coll) as usize;
                here + in_expr(collection, coll) + in_expr(index, coll)
            }
            Expr::BinaryOp { left, right, .. } => in_expr(left, coll) + in_expr(right, coll),
            Expr::Not { operand } => in_expr(operand, coll),
            _ => 0,
        }
    }
    stmts
        .iter()
        .map(|s| match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, coll),
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => in_expr(collection, coll) + in_expr(index, coll) + in_expr(value, coll),
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                in_expr(cond, coll)
                    + count_item_reads(then_block, coll)
                    + else_block.map_or(0, |eb| count_item_reads(eb, coll))
            }
            Stmt::While { cond, body, .. } => in_expr(cond, coll) + count_item_reads(body, coll),
            _ => 0,
        })
        .sum()
}

/// Does any `item _ of <coll>` indexed read remain anywhere in the tree?
fn has_index_of(stmts: &[Stmt], coll: Symbol) -> bool {
    fn in_expr(e: &Expr, coll: Symbol) -> bool {
        match e {
            Expr::Index { collection, index } => {
                matches!(&**collection, Expr::Identifier(c) if *c == coll)
                    || in_expr(collection, coll)
                    || in_expr(index, coll)
            }
            Expr::BinaryOp { left, right, .. } => in_expr(left, coll) || in_expr(right, coll),
            Expr::Not { operand } => in_expr(operand, coll),
            _ => false,
        }
    }
    stmts.iter().any(|s| match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, coll),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => in_expr(collection, coll) || in_expr(index, coll) || in_expr(value, coll),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            in_expr(cond, coll)
                || has_index_of(then_block, coll)
                || else_block.map_or(false, |eb| has_index_of(eb, coll))
        }
        Stmt::While { cond, body, .. } => in_expr(cond, coll) || has_index_of(body, coll),
        _ => false,
    })
}

/// The nbody inner-loop shape: `While j <= 5: Let dx be item i of bx - item
/// j of bx; Set acc to acc + dx.` `item i of bx` is invariant w.r.t. j and
/// bx is never mutated → it must hoist; `item j of bx` must stay.
fn nbody_inner_loop<'a>(b: &B<'a>, n: &Names) -> Vec<Stmt<'a>> {
    use BinaryOpKind::*;
    let dx_val = b.bin(
        Subtract,
        b.index(b.id(n.bx), b.id(n.i)), // invariant: item i of bx
        b.index(b.id(n.bx), b.id(n.j)), // variant: item j of bx
    );
    let let_dx = Stmt::Let {
        var: n.dx,
        ty: None,
        value: dx_val,
        mutable: false,
    };
    let set_acc = Stmt::Set {
        target: n.acc,
        value: b.bin(Add, b.id(n.acc), b.id(n.dx)),
    };
    let inc_j = Stmt::Set {
        target: n.j,
        value: b.bin(Add, b.id(n.j), b.num(1)),
    };
    let while_j = Stmt::While {
        cond: b.bin(LtEq, b.id(n.j), b.num(5)),
        body: b.block(vec![let_dx, set_acc, inc_j]),
        decreasing: None,
    };
    vec![while_j]
}

#[test]
fn hoists_invariant_indexed_load_into_guarded_preheader() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let input = nbody_inner_loop(&b, &n);
    let out = licm_stmts_run(input, &ea, &sa, &mut it);

    // The While is now wrapped in `If cond { Let __licm_load be item i of bx; While … }`.
    let guarded = match &out[..] {
        [Stmt::If {
            then_block,
            else_block: None,
            ..
        }] => *then_block,
        other => panic!("expected a single guarded If, got {other:?}"),
    };
    // Exactly one invariant load (`item i of bx`) hoisted as a fresh Let.
    assert_eq!(
        count_hoisted(guarded, n.bx),
        1,
        "the invariant `item i of bx` must be hoisted into the preheader.\nGot: {guarded:?}"
    );
    // The hoisted Let precedes the (rewritten) While.
    assert!(
        matches!(guarded[0], Stmt::Let { .. }),
        "hoisted load comes first"
    );
    assert!(
        matches!(guarded.last(), Some(Stmt::While { .. })),
        "the loop follows the hoisted loads"
    );
    // The variant `item j of bx` is STILL read inside the loop body —
    // exactly one `item _ of bx` read should remain (the j one); the i one
    // is now an identifier. (Verify the loop still indexes bx for j.)
    if let Some(Stmt::While { body, .. }) = guarded.last() {
        assert!(
            has_index_of(body, n.bx),
            "the variant `item j of bx` must remain in the loop body"
        );
    }
}

#[test]
fn does_not_hoist_when_collection_is_mutated_in_loop() {
    // `Set item i of bvx to …` in the body makes bvx a loop write → its
    // `item i of bvx` read is NOT invariant and must NOT hoist.
    use BinaryOpKind::*;
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let read = b.index(b.id(n.bvx), b.id(n.i)); // item i of bvx
    let let_dx = Stmt::Let {
        var: n.dx,
        ty: None,
        value: b.bin(Add, read, b.num(1)),
        mutable: false,
    };
    // Mutate item i of bvx in the SAME loop.
    let store = Stmt::SetIndex {
        collection: b.id(n.bvx),
        index: b.id(n.i),
        value: b.id(n.dx),
    };
    let inc_j = Stmt::Set {
        target: n.j,
        value: b.bin(Add, b.id(n.j), b.num(1)),
    };
    let while_j = Stmt::While {
        cond: b.bin(LtEq, b.id(n.j), b.num(5)),
        body: b.block(vec![let_dx, store, inc_j]),
        decreasing: None,
    };
    let out = licm_stmts_run(vec![while_j], &ea, &sa, &mut it);
    // No guard wrapper — the load was not hoistable, so the loop is
    // returned unchanged (still a bare While).
    assert!(
        matches!(&out[..], [Stmt::While { .. }]),
        "a mutated collection must NOT be hoisted (no guarded If).\nGot: {out:?}"
    );
}

#[test]
fn does_not_hoist_variant_index() {
    // `item j of bx` (index is the loop variable) is variant → no hoist.
    use BinaryOpKind::*;
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let let_dx = Stmt::Let {
        var: n.dx,
        ty: None,
        value: b.index(b.id(n.bx), b.id(n.j)), // only the variant read
        mutable: false,
    };
    let inc_j = Stmt::Set {
        target: n.j,
        value: b.bin(Add, b.id(n.j), b.num(1)),
    };
    let while_j = Stmt::While {
        cond: b.bin(LtEq, b.id(n.j), b.num(5)),
        body: b.block(vec![let_dx, inc_j]),
        decreasing: None,
    };
    let out = licm_stmts_run(vec![while_j], &ea, &sa, &mut it);
    assert!(
        matches!(&out[..], [Stmt::While { .. }]),
        "a variant index must NOT be hoisted.\nGot: {out:?}"
    );
}

/// A wholly-invariant indexed Let (`Let dx be item i of bx`, no variant
/// co-index) is already lifted by the EXISTING whole-Let hoist; the
/// indexed-load pass must add NO second copy of `item i of bx` — exactly one
/// read survives, never a duplicate.
#[test]
fn indexed_load_hoist_does_not_duplicate_whole_let_invariant_read() {
    use BinaryOpKind::*;
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let n = names(&mut it);
    let b = B { ea: &ea, sa: &sa };
    let let_dx = Stmt::Let {
        var: n.dx,
        ty: None,
        value: b.index(b.id(n.bx), b.id(n.i)), // item i of bx — wholly invariant
        mutable: false,
    };
    let inc_j = Stmt::Set {
        target: n.j,
        value: b.bin(Add, b.id(n.j), b.num(1)),
    };
    let while_j = Stmt::While {
        cond: b.bin(LtEq, b.id(n.j), b.num(5)),
        body: b.block(vec![let_dx, inc_j]),
        decreasing: None,
    };
    let out = licm_stmts_run(vec![while_j], &ea, &sa, &mut it);
    assert_eq!(
        count_item_reads(&out, n.bx),
        1,
        "a wholly-invariant indexed Let must not be duplicated by the load hoist.\nGot: {out:?}"
    );
}
