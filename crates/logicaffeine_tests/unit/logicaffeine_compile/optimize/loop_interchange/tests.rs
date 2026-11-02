use super::*;

struct B<'a> {
    ea: &'a Arena<Expr<'a>>,
}
impl<'a> B<'a> {
    fn id(&self, s: Symbol) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Identifier(s))
    }
    fn num(&self, v: i64) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Literal(Literal::Number(v)))
    }
    fn bin(&self, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
        self.ea.alloc(Expr::BinaryOp {
            op,
            left: l,
            right: r,
        })
    }
    fn idxof(&self, coll: Symbol, i: &'a Expr<'a>) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Index {
            collection: self.id(coll),
            index: i,
        })
    }
}

#[test]
fn interchanges_ijk_matmul_reduction() {
    use BinaryOpKind::{Add, Lt, Modulo, Multiply};
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (i, j, k, n, a, b, c) = (
        it.intern("i"),
        it.intern("j"),
        it.intern("k"),
        it.intern("n"),
        it.intern("a"),
        it.intern("b"),
        it.intern("c"),
    );
    let d = B { ea: &ea };
    // i*n+j+1, i*n+k+1, k*n+j+1
    let idx_c = d.bin(
        Add,
        d.bin(Add, d.bin(Multiply, d.id(i), d.id(n)), d.id(j)),
        d.num(1),
    );
    let idx_a = d.bin(
        Add,
        d.bin(Add, d.bin(Multiply, d.id(i), d.id(n)), d.id(k)),
        d.num(1),
    );
    let idx_b = d.bin(
        Add,
        d.bin(Add, d.bin(Multiply, d.id(k), d.id(n)), d.id(j)),
        d.num(1),
    );
    let prod = d.bin(Multiply, d.idxof(a, idx_a), d.idxof(b, idx_b));
    let sum = d.bin(Add, d.idxof(c, idx_c), prod);
    let val = d.bin(Modulo, sum, d.num(1000000007));
    let red = Stmt::SetIndex {
        collection: d.id(c),
        index: idx_c,
        value: val,
    };
    let k_incr = Stmt::Set {
        target: k,
        value: d.bin(Add, d.id(k), d.num(1)),
    };
    let k_while = Stmt::While {
        cond: d.bin(Lt, d.id(k), d.id(n)),
        body: sa.alloc_slice(vec![red, k_incr]),
        decreasing: None,
    };
    let k_init = Stmt::Let {
        var: k,
        ty: None,
        value: d.num(0),
        mutable: true,
    };
    let j_incr = Stmt::Set {
        target: j,
        value: d.bin(Add, d.id(j), d.num(1)),
    };
    let j_while = Stmt::While {
        cond: d.bin(Lt, d.id(j), d.id(n)),
        body: sa.alloc_slice(vec![k_init, k_while, j_incr]),
        decreasing: None,
    };
    let j_init = Stmt::Let {
        var: j,
        ty: None,
        value: d.num(0),
        mutable: true,
    };

    let (out, changed) = interchange_stmts(vec![j_init, j_while], &ea, &sa, &mut it);
    assert!(
        changed,
        "the ijk reduction must interchange to put j innermost"
    );
    // The swapped nest inits the OUTER loop over k (`Let k ...; While k ...`).
    assert!(
        matches!(&out[0], Stmt::Let { var, .. } if *var == k),
        "outer init is now k"
    );
    assert!(
        matches!(&out[1], Stmt::While { .. }),
        "followed by the k While"
    );
}
