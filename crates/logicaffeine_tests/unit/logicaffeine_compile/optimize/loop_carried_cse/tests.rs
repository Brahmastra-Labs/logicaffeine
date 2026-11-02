use super::*;

struct B<'a> {
    ea: &'a Arena<Expr<'a>>,
}
impl<'a> B<'a> {
    fn id(&self, s: Symbol) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Identifier(s))
    }
    fn fl(&self, f: f64) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Literal(Literal::Float(f)))
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
    fn sq(&self, s: Symbol) -> &'a Expr<'a> {
        self.bin(BinaryOpKind::Multiply, self.id(s), self.id(s))
    }
}

/// Count `Set <temp> to <op> * <op>` statements in a block.
fn count_square_assigns(block: &[Stmt], temp: Symbol) -> usize {
    block
        .iter()
        .filter(|s| {
            matches!(s, Stmt::Set { target, value }
            if *target == temp
            && matches!(value, Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right }
                if matches!((left, right), (Expr::Identifier(a), Expr::Identifier(b)) if a == b)))
        })
        .count()
}

/// The ONLY surviving `o * o` in the block is the single `Set <temp> to o*o`
/// the pass inserts — no other statement recomputes the square.
fn only_square_is_temp_assign(block: &[Stmt], o: Symbol, temp: Symbol) -> bool {
    block.iter().all(|s| {
        if matches!(s, Stmt::Set { target, .. } if *target == temp) {
            true
        } else {
            !stmt_has_self_square(s, o)
        }
    })
}

/// Build mandelbrot's escape loop and assert the temp is introduced once per
/// iteration and every `zr*zr` / `zi*zi` recomputation is replaced.
#[test]
fn mandelbrot_escape_loop_hoists_squares() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let zr = it.intern("zr");
    let zi = it.intern("zi");
    let cr = it.intern("cr");
    let ci = it.intern("ci");
    let zr2 = it.intern("zr2");
    let zi2 = it.intern("zi2");
    let iter = it.intern("iter");
    let is_inside = it.intern("isInside");
    let b = B { ea: &ea };

    // Pre-loop: Let mutable zr be 0.0.  Let mutable zi be 0.0.
    let decl_zr = Stmt::Let {
        var: zr,
        ty: None,
        value: b.fl(0.0),
        mutable: true,
    };
    let decl_zi = Stmt::Let {
        var: zi,
        ty: None,
        value: b.fl(0.0),
        mutable: true,
    };
    let decl_iter = Stmt::Let {
        var: iter,
        ty: None,
        value: b.num(0),
        mutable: true,
    };

    // Body.
    // Let zr2 be zr*zr - zi*zi + cr.
    let let_zr2 = Stmt::Let {
        var: zr2,
        ty: None,
        value: b.bin(
            BinaryOpKind::Add,
            b.bin(BinaryOpKind::Subtract, b.sq(zr), b.sq(zi)),
            b.id(cr),
        ),
        mutable: false,
    };
    // Let zi2 be 2.0 * zr * zi + ci.
    let let_zi2 = Stmt::Let {
        var: zi2,
        ty: None,
        value: b.bin(
            BinaryOpKind::Add,
            b.bin(
                BinaryOpKind::Multiply,
                b.bin(BinaryOpKind::Multiply, b.fl(2.0), b.id(zr)),
                b.id(zi),
            ),
            b.id(ci),
        ),
        mutable: false,
    };
    let set_zr = Stmt::Set {
        target: zr,
        value: b.id(zr2),
    };
    let set_zi = Stmt::Set {
        target: zi,
        value: b.id(zi2),
    };
    // If zr*zr + zi*zi > 4.0: Set isInside to 0. Set iter to 50.
    let guard = Stmt::If {
        cond: b.bin(
            BinaryOpKind::Gt,
            b.bin(BinaryOpKind::Add, b.sq(zr), b.sq(zi)),
            b.fl(4.0),
        ),
        then_block: sa.alloc_slice(vec![
            Stmt::Set {
                target: is_inside,
                value: b.num(0),
            },
            Stmt::Set {
                target: iter,
                value: b.num(50),
            },
        ]),
        else_block: None,
    };
    let step = Stmt::Set {
        target: iter,
        value: b.bin(BinaryOpKind::Add, b.id(iter), b.num(1)),
    };
    let body = sa.alloc_slice(vec![let_zr2, let_zi2, set_zr, set_zi, guard, step]);
    let loop_stmt = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(iter), b.num(50)),
        body,
        decreasing: None,
    };

    let input = vec![decl_zr, decl_zi, decl_iter, loop_stmt];
    let (out, changed) = loop_carried_cse_stmts(input, &ea, &sa, &mut it);
    assert!(
        changed,
        "the transform must fire on mandelbrot's escape loop"
    );

    let zr_sq = it.intern("__lcse_zr_sq");
    let zi_sq = it.intern("__lcse_zi_sq");

    // Two seeds (zrSq=0.0, ziSq=0.0) precede the loop.
    let seeds: Vec<&Stmt> = out
        .iter()
        .filter(|s| {
            matches!(s, Stmt::Let { var, value, .. }
            if (*var == zr_sq || *var == zi_sq)
            && matches!(value, Expr::Literal(Literal::Float(f)) if *f == 0.0))
        })
        .collect();
    assert_eq!(
        seeds.len(),
        2,
        "both squared-temp seeds must precede the loop"
    );

    // The loop body: each temp is assigned exactly once, and no `o*o`
    // recomputation survives anywhere in the body.
    let Stmt::While { body, cond, .. } = out.last().unwrap() else {
        panic!("the last statement must remain the While loop");
    };
    assert_eq!(
        count_square_assigns(body, zr_sq),
        1,
        "zr*zr computed once per iter"
    );
    assert_eq!(
        count_square_assigns(body, zi_sq),
        1,
        "zi*zi computed once per iter"
    );
    assert!(
        only_square_is_temp_assign(body, zr, zr_sq),
        "the only zr*zr left is the single temp assignment"
    );
    assert!(
        only_square_is_temp_assign(body, zi, zi_sq),
        "the only zi*zi left is the single temp assignment"
    );

    // The exit guard must now read the temps, not recompute the products.
    // (The `cond` is the loop guard `iter < 50`; the escape `If` is inside the
    // body — assert both the body's escape If reads the temps.)
    let _ = cond;
    let escape = body
        .iter()
        .find_map(|s| match s {
            Stmt::If { cond, .. } => Some(*cond),
            _ => None,
        })
        .expect("escape guard remains");
    // The compared expression is now `__lcse_zr_sq + __lcse_zi_sq`.
    assert!(
        matches!(escape, Expr::BinaryOp { op: BinaryOpKind::Gt, left, .. }
            if matches!(left, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                if matches!((left, right),
                    (Expr::Identifier(a), Expr::Identifier(c)) if *a == zr_sq && *c == zi_sq))),
        "the escape guard must compare the hoisted temps"
    );
}

/// A loop whose carried operand's new value READS its old value (`Set o to
/// o + 1`) must NOT fire — the guard's `o*o` over the new value differs from
/// the next iteration's recomputation point in general, and the narrow square
/// identity is only sound when the new value is independent of the old.
#[test]
fn self_referential_update_does_not_fire() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let o = it.intern("o");
    let acc = it.intern("acc");
    let b = B { ea: &ea };

    let decl = Stmt::Let {
        var: o,
        ty: None,
        value: b.num(0),
        mutable: true,
    };
    // Let acc be o*o.   Set o to o + 1.   If o*o > 100: ...
    let recompute = Stmt::Let {
        var: acc,
        ty: None,
        value: b.sq(o),
        mutable: false,
    };
    let step = Stmt::Set {
        target: o,
        value: b.bin(BinaryOpKind::Add, b.id(o), b.num(1)),
    };
    let guard = Stmt::If {
        cond: b.bin(BinaryOpKind::Gt, b.sq(o), b.num(100)),
        then_block: sa.alloc_slice(vec![Stmt::Break]),
        else_block: None,
    };
    let body = sa.alloc_slice(vec![recompute, step, guard]);
    let loop_stmt = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(o), b.num(1000)),
        body,
        decreasing: None,
    };
    let (_, changed) = loop_carried_cse_stmts(vec![decl, loop_stmt], &ea, &sa, &mut it);
    assert!(
        !changed,
        "must not fire when the operand's update reads itself"
    );
}

/// When the operand's entry value is not a known literal, the seed cannot be
/// formed and the square is skipped (conservative).
#[test]
fn unknown_entry_value_skips() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let o = it.intern("o");
    let src = it.intern("src");
    let nv = it.intern("nv");
    let acc = it.intern("acc");
    let b = B { ea: &ea };

    // Let o be src.   (entry value unknown)
    let decl = Stmt::Let {
        var: o,
        ty: None,
        value: b.id(src),
        mutable: true,
    };
    let recompute = Stmt::Let {
        var: acc,
        ty: None,
        value: b.sq(o),
        mutable: false,
    };
    let set = Stmt::Set {
        target: o,
        value: b.id(nv),
    };
    let guard = Stmt::If {
        cond: b.bin(BinaryOpKind::Gt, b.sq(o), b.num(4)),
        then_block: sa.alloc_slice(vec![Stmt::Break]),
        else_block: None,
    };
    let body = sa.alloc_slice(vec![recompute, set, guard]);
    let loop_stmt = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(o), b.num(10)),
        body,
        decreasing: None,
    };
    let (_, changed) = loop_carried_cse_stmts(vec![decl, loop_stmt], &ea, &sa, &mut it);
    assert!(
        !changed,
        "must not fire when the entry value is not a known literal"
    );
}
