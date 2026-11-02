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
    let push1 = Stmt::Push {
        value: b.fnum(1.5),
        collection: b.id(arr),
    };
    let push2 = Stmt::Push {
        value: b.fnum(2.5),
        collection: b.id(arr),
    };
    // sink = item 1 of arr + item 2 of arr
    let read = Stmt::Let {
        var: sink,
        ty: None,
        value: b.bin(
            BinaryOpKind::Add,
            b.index(b.id(arr), b.num(1)),
            b.index(b.id(arr), b.num(2)),
        ),
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
        Stmt::Let {
            var,
            mutable: true,
            value,
            ..
        } => {
            assert_eq!(*var, a1);
            assert!(matches!(value, Expr::Literal(Literal::Float(f)) if *f == 1.5));
        }
        other => panic!("expected Let arr$1, got {other:?}"),
    }
    match &out[1] {
        Stmt::Let {
            var,
            mutable: true,
            value,
            ..
        } => {
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
    let push1 = Stmt::Push {
        value: b.fnum(1.5),
        collection: b.id(arr),
    };
    let push2 = Stmt::Push {
        value: b.fnum(2.5),
        collection: b.id(arr),
    };
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
    let push1 = Stmt::Push {
        value: b.fnum(1.5),
        collection: b.id(arr),
    };
    let read = Stmt::Let {
        var: sink,
        ty: None,
        value: ea.alloc(Expr::Length {
            collection: b.id(arr),
        }),
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
    let push1 = Stmt::Push {
        value: b.fnum(1.5),
        collection: b.id(arr),
    };
    let push2 = Stmt::Push {
        value: b.fnum(2.5),
        collection: b.id(arr),
    };
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

/// `Let mutable name be a new Seq of Float.`
fn float_seq<'a>(ea: &'a Arena<Expr<'a>>, it: &mut Interner, name: Symbol) -> Stmt<'a> {
    let seq = it.intern("Seq");
    let float = it.intern("Float");
    Stmt::Let {
        var: name,
        ty: None,
        value: ea.alloc(Expr::New {
            type_name: seq,
            type_args: vec![TypeExpr::Primitive(float)],
            init_fields: vec![],
        }),
        mutable: true,
    }
}

/// Two arrays of one length swapped: the swap exchanges their elements, one register swap per position.
#[test]
fn a_swap_of_two_arrays_of_one_length_exchanges_their_elements() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (a, bb, sink) = (it.intern("a"), it.intern("b"), it.intern("sink"));
    let b = B { ea: &ea };
    let input = vec![
        float_seq(&ea, &mut it, a),
        Stmt::Push {
            value: b.fnum(1.0),
            collection: b.id(a),
        },
        Stmt::Push {
            value: b.fnum(2.0),
            collection: b.id(a),
        },
        float_seq(&ea, &mut it, bb),
        Stmt::Push {
            value: b.fnum(3.0),
            collection: b.id(bb),
        },
        Stmt::Push {
            value: b.fnum(4.0),
            collection: b.id(bb),
        },
        Stmt::Swap { a, b: bb },
        Stmt::Let {
            var: sink,
            ty: None,
            value: b.bin(
                BinaryOpKind::Add,
                b.index(b.id(a), b.num(1)),
                b.index(b.id(bb), b.num(2)),
            ),
            mutable: false,
        },
    ];
    let (out, changed, left_behind) = scalarize_reporting(input, &ea, &sa, &mut it);
    assert!(changed && left_behind.is_empty());
    let (a1, a2, b1, b2) = (
        it.intern("a$1"),
        it.intern("a$2"),
        it.intern("b$1"),
        it.intern("b$2"),
    );
    let swaps: Vec<(Symbol, Symbol)> = out
        .iter()
        .filter_map(|s| match s {
            Stmt::Swap { a, b } => Some((*a, *b)),
            _ => None,
        })
        .collect();
    assert_eq!(swaps, [(a1, b1), (a2, b2)]);
    assert!(
        !Mentions::of_block(&out).touches("a", &it) && !Mentions::of_block(&out).touches("b", &it)
    );
}

/// An element read inside a record constructor is an element read like any other.
#[test]
fn an_element_read_inside_a_record_constructor_is_rewritten() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (arr, p, point, x) = (
        it.intern("arr"),
        it.intern("p"),
        it.intern("Point"),
        it.intern("x"),
    );
    let b = B { ea: &ea };
    let input = vec![
        float_seq(&ea, &mut it, arr),
        Stmt::Push {
            value: b.fnum(5.0),
            collection: b.id(arr),
        },
        Stmt::Let {
            var: p,
            ty: None,
            value: ea.alloc(Expr::New {
                type_name: point,
                type_args: vec![],
                init_fields: vec![(x, b.index(b.id(arr), b.num(1)))],
            }),
            mutable: false,
        },
    ];
    let (out, changed, left_behind) = scalarize_reporting(input, &ea, &sa, &mut it);
    assert!(changed && left_behind.is_empty());
    let arr1 = it.intern("arr$1");
    match out.last() {
        Some(Stmt::Let {
            value: Expr::New { init_fields, .. },
            ..
        }) => {
            assert!(matches!(init_fields[0].1, Expr::Identifier(s) if *s == arr1));
        }
        other => panic!("expected the record's Let, got {other:?}"),
    }
}

/// `Push 1.0, 2.0 to arr` desugars to a splice of pushes: each is an element, as if written one by one.
#[test]
fn pushes_in_a_splice_define_the_elements() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (arr, sink) = (it.intern("arr"), it.intern("sink"));
    let b = B { ea: &ea };
    let pushes = sa.alloc_slice(vec![
        Stmt::Push {
            value: b.fnum(1.0),
            collection: b.id(arr),
        },
        Stmt::Push {
            value: b.fnum(2.0),
            collection: b.id(arr),
        },
    ]);
    let input = vec![
        float_seq(&ea, &mut it, arr),
        Stmt::Splice { body: pushes },
        Stmt::Let {
            var: sink,
            ty: None,
            value: b.index(b.id(arr), b.num(2)),
            mutable: false,
        },
    ];
    let (out, changed, left_behind) = scalarize_reporting(input, &ea, &sa, &mut it);
    assert!(changed && left_behind.is_empty());
    assert!(
        matches!(out.last(), Some(Stmt::Let { value: Expr::Identifier(s), .. }) if *s == it.intern("arr$2"))
    );
}

/// Two arrays of one length are swapped, but one has its length queried, so it stays a `Seq`: the swap cannot
/// exchange a scalar set with a `Seq`, the rewrite leaves the other array's name in it, and the check after the
/// rewrite sends that array back too. Nothing is scalarized; the program is unchanged.
#[test]
fn a_swap_partner_kept_as_a_seq_sends_the_other_array_back() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (a, bb, sink, n) = (
        it.intern("a"),
        it.intern("b"),
        it.intern("sink"),
        it.intern("n"),
    );
    let b = B { ea: &ea };
    let input = vec![
        float_seq(&ea, &mut it, a),
        Stmt::Push {
            value: b.fnum(1.0),
            collection: b.id(a),
        },
        float_seq(&ea, &mut it, bb),
        Stmt::Push {
            value: b.fnum(2.0),
            collection: b.id(bb),
        },
        Stmt::Swap { a, b: bb },
        Stmt::Let {
            var: sink,
            ty: None,
            value: b.index(b.id(a), b.num(1)),
            mutable: false,
        },
        Stmt::Let {
            var: n,
            ty: None,
            value: ea.alloc(Expr::Length {
                collection: b.id(bb),
            }),
            mutable: false,
        },
    ];
    let before = input.len();
    let (out, changed, left_behind) = scalarize_reporting(input, &ea, &sa, &mut it);
    assert!(!changed);
    assert_eq!(
        left_behind,
        [a].into_iter().collect(),
        "the length query keeps `b`; the check sends `a` back"
    );
    assert_eq!(out.len(), before);
    assert!(out
        .iter()
        .any(|s| matches!(s, Stmt::Swap { a: x, b: y } if *x == a && *y == bb)));
}

/// An array used inside a form the rewriter has no case for (a concurrent block) is found by the check after the
/// rewrite and kept as a `Seq`: no rewrite ever leaves a program naming a variable it removed.
#[test]
fn an_array_in_a_form_the_rewriter_does_not_know_is_sent_back() {
    let ea = Arena::new();
    let sa = Arena::new();
    let mut it = Interner::new();
    let (arr, sink) = (it.intern("arr"), it.intern("sink"));
    let b = B { ea: &ea };
    let tasks = sa.alloc_slice(vec![Stmt::Let {
        var: sink,
        ty: None,
        value: b.index(b.id(arr), b.num(1)),
        mutable: false,
    }]);
    let input = vec![
        float_seq(&ea, &mut it, arr),
        Stmt::Push {
            value: b.fnum(1.0),
            collection: b.id(arr),
        },
        Stmt::Concurrent { tasks },
    ];
    let (out, changed, left_behind) = scalarize_reporting(input, &ea, &sa, &mut it);
    assert!(!changed);
    assert_eq!(left_behind, [arr].into_iter().collect());
    assert!(
        matches!(out[0], Stmt::Let { var, .. } if var == arr),
        "the Seq declaration survives"
    );
}
