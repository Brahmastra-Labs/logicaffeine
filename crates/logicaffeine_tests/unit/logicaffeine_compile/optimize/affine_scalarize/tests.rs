use super::*;
use crate::ast::stmt::TypeExpr;

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
    fn new_seq(&self, seq: Symbol, elem: Symbol) -> &'a Expr<'a> {
        self.ea.alloc(Expr::New {
            type_name: seq,
            type_args: vec![TypeExpr::Primitive(elem)],
            init_fields: vec![],
        })
    }
}

fn run<'a>(
    input: Vec<Stmt<'a>>,
    ea: &'a Arena<Expr<'a>>,
    sa: &'a Arena<Stmt<'a>>,
    it: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    affine_scalarize_seqs(input, ea, sa, it)
}

/// Recursively evaluate a constant-foldable closed-form expression so a test
/// can assert the EXACT i64 value the substitution produces.
fn eval(e: &Expr) -> i64 {
    const_eval(e).expect("closed form must be constant-foldable")
}

/// Build the graph_bfs shape: an `adjStarts` populated only by `Push i*5`
/// inside a `while i < n` loop that ALSO pushes to a sibling array, then read
/// twice as `item (v+1) of adjStarts`. The array and its push must vanish,
/// the sibling push must survive, and the reads must become `v * 5`.
#[test]
fn affine_csr_array_scalarizes_to_closed_form() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let seq = it.intern("Seq");
    let int = it.intern("Int");
    let adj_starts = it.intern("adjStarts");
    let sibling = it.intern("adjCounts");
    let i = it.intern("i");
    let n = it.intern("n");
    let v = it.intern("v");
    let out_var = it.intern("start");
    let b = B { ea: &ea };

    let decl_sibling = Stmt::Let {
        var: sibling,
        ty: None,
        value: b.new_seq(seq, int),
        mutable: true,
    };
    let decl = Stmt::Let {
        var: adj_starts,
        ty: None,
        value: b.new_seq(seq, int),
        mutable: true,
    };
    let init_i = Stmt::Let {
        var: i,
        ty: None,
        value: b.num(0),
        mutable: true,
    };
    // while i < n: Push i*5 to adjStarts; Push 0 to adjCounts; Set i to i+1
    let push_affine = Stmt::Push {
        value: b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
        collection: b.id(adj_starts),
    };
    let push_sibling = Stmt::Push {
        value: b.num(0),
        collection: b.id(sibling),
    };
    let step = Stmt::Set {
        target: i,
        value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)),
    };
    let body = sa.alloc_slice(vec![push_affine, push_sibling, step]);
    let build = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
        body,
        decreasing: None,
    };
    // Let start be item (v + 1) of adjStarts.
    let read = Stmt::Let {
        var: out_var,
        ty: None,
        value: b.index(
            b.id(adj_starts),
            b.bin(BinaryOpKind::Add, b.id(v), b.num(1)),
        ),
        mutable: true,
    };

    let input = vec![decl_sibling, decl, init_i, build, read];
    let (out, changed) = run(input, &ea, &sa, &mut it);
    assert!(changed, "affine scalarization should fire");

    // adjStarts' declaration is gone; adjCounts' survives.
    assert!(
        !out.iter()
            .any(|s| matches!(s, Stmt::Let { var, .. } if *var == adj_starts)),
        "adjStarts decl must be deleted"
    );
    assert!(
        out.iter()
            .any(|s| matches!(s, Stmt::Let { var, .. } if *var == sibling)),
        "sibling adjCounts decl must survive"
    );

    // The build loop keeps the sibling push and the IV step; the affine push
    // is gone.
    let while_stmt = out
        .iter()
        .find(|s| matches!(s, Stmt::While { .. }))
        .unwrap();
    let Stmt::While { body, .. } = while_stmt else {
        unreachable!()
    };
    assert_eq!(body.len(), 2, "only the affine push is removed");
    assert!(
        !body
            .iter()
            .any(|s| matches!(s, Stmt::Push { collection, .. }
            if matches!(collection, Expr::Identifier(sy) if *sy == adj_starts))),
        "the affine push must be gone"
    );
    assert!(
        body.iter()
            .any(|s| matches!(s, Stmt::Push { collection, .. }
            if matches!(collection, Expr::Identifier(sy) if *sy == sibling))),
        "the sibling push must survive"
    );

    // The read `item (v+1) of adjStarts` became `v * 5` (coeff 5, offset 0,
    // `(v+1)-1 == v`): a Multiply of `v` by 5, no Index left.
    let read_out = out.last().unwrap();
    let Stmt::Let { value, .. } = read_out else {
        panic!("expected Let")
    };
    match value {
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left,
            right,
        } => {
            assert!(matches!(left, Expr::Identifier(s) if *s == v), "lhs is v");
            assert!(
                matches!(right, Expr::Literal(Literal::Number(5))),
                "rhs is 5"
            );
        }
        other => panic!("expected `v * 5`, got {other:?}"),
    }
}

/// The closed form must EXACTLY reproduce the pushed values for every
/// in-range 1-based index — the off-by-one is the whole correctness story.
/// `Push (i*5 + 3)` makes 0-based element p equal `5p + 3`, so 1-based
/// `item k` must equal `5*(k-1) + 3`.
#[test]
fn closed_form_matches_pushed_values_exactly() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let seq = it.intern("Seq");
    let int = it.intern("Int");
    let arr = it.intern("arr");
    let i = it.intern("i");
    let n = it.intern("n");
    let sink = it.intern("sink");
    let b = B { ea: &ea };

    let decl = Stmt::Let {
        var: arr,
        ty: None,
        value: b.new_seq(seq, int),
        mutable: true,
    };
    let init_i = Stmt::Let {
        var: i,
        ty: None,
        value: b.num(0),
        mutable: true,
    };
    // Push (i * 5 + 3) to arr
    let affine = b.bin(
        BinaryOpKind::Add,
        b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
        b.num(3),
    );
    let push = Stmt::Push {
        value: affine,
        collection: b.id(arr),
    };
    let step = Stmt::Set {
        target: i,
        value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)),
    };
    let body = sa.alloc_slice(vec![push, step]);
    let build = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
        body,
        decreasing: None,
    };
    // Read every constant index 1..=4; assert the substituted value equals
    // the value the k-th push computed (element p = k-1 ⟹ 5*(k-1)+3).
    let mut reads = vec![decl, init_i, build];
    for k in 1..=4i64 {
        reads.push(Stmt::Let {
            var: sink,
            ty: None,
            value: b.index(b.id(arr), b.num(k)),
            mutable: true,
        });
    }
    let (out, changed) = run(reads, &ea, &sa, &mut it);
    assert!(changed, "affine scalarization should fire");

    let read_stmts: Vec<_> = out
        .iter()
        .filter(|s| matches!(s, Stmt::Let { var, .. } if *var == sink))
        .collect();
    assert_eq!(read_stmts.len(), 4);
    for (j, s) in read_stmts.iter().enumerate() {
        let k = (j + 1) as i64;
        let Stmt::Let { value, .. } = s else {
            unreachable!()
        };
        let expected = 5 * (k - 1) + 3;
        assert_eq!(
            eval(value),
            expected,
            "item {k} of arr must equal 5*({k}-1)+3 = {expected}"
        );
    }
}

/// A `SetIndex` write on the array disqualifies it — left untouched.
#[test]
fn in_place_write_blocks() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let seq = it.intern("Seq");
    let int = it.intern("Int");
    let arr = it.intern("arr");
    let i = it.intern("i");
    let n = it.intern("n");
    let b = B { ea: &ea };

    let decl = Stmt::Let {
        var: arr,
        ty: None,
        value: b.new_seq(seq, int),
        mutable: true,
    };
    let init_i = Stmt::Let {
        var: i,
        ty: None,
        value: b.num(0),
        mutable: true,
    };
    let push = Stmt::Push {
        value: b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
        collection: b.id(arr),
    };
    let step = Stmt::Set {
        target: i,
        value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)),
    };
    let body = sa.alloc_slice(vec![push, step]);
    let build = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
        body,
        decreasing: None,
    };
    // Set item 1 of arr to 99 — an in-place write after the build.
    let write = Stmt::SetIndex {
        collection: b.id(arr),
        index: b.num(1),
        value: b.num(99),
    };
    let read = Stmt::Let {
        var: it.intern("sink"),
        ty: None,
        value: b.index(b.id(arr), b.num(1)),
        mutable: true,
    };
    let input = vec![decl, init_i, build, write, read];
    let (out, changed) = run(input, &ea, &sa, &mut it);
    assert!(!changed, "an in-place write must block scalarization");
    assert!(out
        .iter()
        .any(|s| matches!(s, Stmt::Let { var, .. } if *var == arr)));
}

/// A `length of arr` query disqualifies — the closed form has no length.
#[test]
fn length_query_blocks() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let seq = it.intern("Seq");
    let int = it.intern("Int");
    let arr = it.intern("arr");
    let i = it.intern("i");
    let n = it.intern("n");
    let b = B { ea: &ea };

    let decl = Stmt::Let {
        var: arr,
        ty: None,
        value: b.new_seq(seq, int),
        mutable: true,
    };
    let init_i = Stmt::Let {
        var: i,
        ty: None,
        value: b.num(0),
        mutable: true,
    };
    let push = Stmt::Push {
        value: b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
        collection: b.id(arr),
    };
    let step = Stmt::Set {
        target: i,
        value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)),
    };
    let body = sa.alloc_slice(vec![push, step]);
    let build = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
        body,
        decreasing: None,
    };
    let read = Stmt::Let {
        var: it.intern("len"),
        ty: None,
        value: ea.alloc(Expr::Length {
            collection: b.id(arr),
        }),
        mutable: true,
    };
    let input = vec![decl, init_i, build, read];
    let (_out, changed) = run(input, &ea, &sa, &mut it);
    assert!(!changed, "a length query must block scalarization");
}

/// A non-affine pushed value (`i * i`) disqualifies the array.
#[test]
fn non_affine_push_blocks() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let seq = it.intern("Seq");
    let int = it.intern("Int");
    let arr = it.intern("arr");
    let i = it.intern("i");
    let n = it.intern("n");
    let b = B { ea: &ea };

    let decl = Stmt::Let {
        var: arr,
        ty: None,
        value: b.new_seq(seq, int),
        mutable: true,
    };
    let init_i = Stmt::Let {
        var: i,
        ty: None,
        value: b.num(0),
        mutable: true,
    };
    let push = Stmt::Push {
        value: b.bin(BinaryOpKind::Multiply, b.id(i), b.id(i)),
        collection: b.id(arr),
    };
    let step = Stmt::Set {
        target: i,
        value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)),
    };
    let body = sa.alloc_slice(vec![push, step]);
    let build = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
        body,
        decreasing: None,
    };
    let read = Stmt::Let {
        var: it.intern("sink"),
        ty: None,
        value: b.index(b.id(arr), b.num(1)),
        mutable: true,
    };
    let input = vec![decl, init_i, build, read];
    let (_out, changed) = run(input, &ea, &sa, &mut it);
    assert!(!changed, "a non-affine push (i*i) must block scalarization");
}

/// A bare reference to the array (an alias/escape) disqualifies it.
#[test]
fn alias_blocks() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let seq = it.intern("Seq");
    let int = it.intern("Int");
    let arr = it.intern("arr");
    let alias = it.intern("alias");
    let i = it.intern("i");
    let n = it.intern("n");
    let b = B { ea: &ea };

    let decl = Stmt::Let {
        var: arr,
        ty: None,
        value: b.new_seq(seq, int),
        mutable: true,
    };
    let init_i = Stmt::Let {
        var: i,
        ty: None,
        value: b.num(0),
        mutable: true,
    };
    let push = Stmt::Push {
        value: b.bin(BinaryOpKind::Multiply, b.id(i), b.num(5)),
        collection: b.id(arr),
    };
    let step = Stmt::Set {
        target: i,
        value: b.bin(BinaryOpKind::Add, b.id(i), b.num(1)),
    };
    let body = sa.alloc_slice(vec![push, step]);
    let build = Stmt::While {
        cond: b.bin(BinaryOpKind::Lt, b.id(i), b.id(n)),
        body,
        decreasing: None,
    };
    // Let alias be arr — a bare escape.
    let escape = Stmt::Let {
        var: alias,
        ty: None,
        value: b.id(arr),
        mutable: true,
    };
    let input = vec![decl, init_i, build, escape];
    let (_out, changed) = run(input, &ea, &sa, &mut it);
    assert!(!changed, "a bare alias must block scalarization");
}
