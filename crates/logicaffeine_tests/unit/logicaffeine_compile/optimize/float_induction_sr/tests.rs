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
}

// pi_leibniz's loop: `While k < n { sum = sum + sign/(2.0*k+1.0); sign = 0-sign; k = k+1 }`.
fn pi_loop<'a>(
    b: &B<'a>,
    sa: &'a Arena<Stmt<'a>>,
    syms: (Symbol, Symbol, Symbol, Symbol),
) -> Stmt<'a> {
    let (k, n, sum, sign) = syms;
    let affine = b.bin(
        BinaryOpKind::Add,
        b.bin(BinaryOpKind::Multiply, b.fl(2.0), b.id(k)),
        b.fl(1.0),
    );
    let term = b.bin(BinaryOpKind::Divide, b.id(sign), affine);
    let set_sum = Stmt::Set {
        target: sum,
        value: b.bin(BinaryOpKind::Add, b.id(sum), term),
    };
    let set_sign = Stmt::Set {
        target: sign,
        value: b.bin(BinaryOpKind::Subtract, b.fl(0.0), b.id(sign)),
    };
    let set_k = Stmt::Set {
        target: k,
        value: b.bin(BinaryOpKind::Add, b.id(k), b.num(1)),
    };
    Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(k), b.id(n)),
        body: sa.alloc_slice(vec![set_sum, set_sign, set_k]),
        decreasing: None,
    }
}

#[test]
fn fires_on_pi_leibniz_and_guards() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (k, n, sum, sign) = (
        it.intern("k"),
        it.intern("n"),
        it.intern("sum"),
        it.intern("sign"),
    );
    let b = B { ea: &ea };
    let loop_stmt = pi_loop(&b, &sa, (k, n, sum, sign));
    let (out, changed) = float_induction_sr_stmts(vec![loop_stmt], &ea, &sa, &mut it);
    assert!(changed, "pi_leibniz affine-float induction must fire");
    // Output is a guarded `If { then: [Let denom, While], else: [While] }`.
    let denom = it.intern("__fsr_k_denom");
    match &out[0] {
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            assert!(
                matches!(then_block[0], Stmt::Let { var, .. } if var == denom),
                "denom seeded"
            );
            assert!(matches!(then_block[1], Stmt::While { .. }), "fast loop");
            let els = else_block.expect("slow path present");
            assert!(
                matches!(els[0], Stmt::While { .. }),
                "original loop preserved"
            );
        }
        other => panic!("expected guarded If, got {other:?}"),
    }
}

#[test]
fn rejects_non_integer_coefficient() {
    // c1 = 0.1 (not integer-valued) -> accumulation would drift -> must NOT fire.
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (k, n, sum) = (it.intern("k"), it.intern("n"), it.intern("sum"));
    let b = B { ea: &ea };
    let affine = b.bin(
        BinaryOpKind::Add,
        b.bin(BinaryOpKind::Multiply, b.fl(0.1), b.id(k)),
        b.fl(1.0),
    );
    let set_sum = Stmt::Set {
        target: sum,
        value: b.bin(BinaryOpKind::Add, b.id(sum), affine),
    };
    let set_k = Stmt::Set {
        target: k,
        value: b.bin(BinaryOpKind::Add, b.id(k), b.num(1)),
    };
    let loop_stmt = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(k), b.id(n)),
        body: sa.alloc_slice(vec![set_sum, set_k]),
        decreasing: None,
    };
    let (_, changed) = float_induction_sr_stmts(vec![loop_stmt], &ea, &sa, &mut it);
    assert!(!changed, "non-integer c1 must be rejected (drift unsound)");
}
