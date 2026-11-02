//! Application spines: `app_args`, `app_num_args` and `app_fn` answer from the length the store
//! records when it interns an application, and must agree with walking the `App` chain — for every
//! expression of a real export, for scratch expressions, and past the longest length a node can
//! record.

use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::expr::{ExprId, Node};
use logicaffeine_kernel::dag::Kernel;
use std::io::BufReader;
use std::path::PathBuf;

fn walk(k: &Kernel, mut e: ExprId) -> (ExprId, Vec<ExprId>) {
    let mut args = Vec::new();
    while let Node::App(f, a) = k.s.node(e) {
        args.push(a);
        e = f;
    }
    args.reverse();
    (e, args)
}

fn agree(k: &Kernel, e: ExprId) {
    let (head, args) = walk(k, e);
    let (h, a) = k.s.app_args(e);
    assert_eq!(h, head, "head of {e:?}");
    assert_eq!(a.as_slice(), args.as_slice(), "arguments of {e:?}");
    assert_eq!(k.s.app_num_args(e), args.len(), "argument count of {e:?}");
    assert_eq!(k.s.app_fn(e), head, "app_fn of {e:?}");
}

fn prelude() -> Kernel {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus/good/init-prelude.ndjson");
    let mut k = Kernel::new();
    let mut r = Reader::new(&mut k);
    r.parse_only = true;
    r.run(BufReader::new(std::fs::File::open(path).unwrap()))
        .unwrap();
    k
}

#[test]
fn every_expression_of_an_export_agrees_with_the_walk() {
    let k = prelude();
    let n = k.s.expr_count();
    assert!(
        n > 10_000,
        "the prelude interns tens of thousands of expressions, got {n}"
    );
    let mut apps = 0;
    for i in 0..n as u32 {
        let e = ExprId(i);
        apps += matches!(k.s.node(e), Node::App(..)) as usize;
        agree(&k, e);
    }
    assert!(
        apps > 1_000,
        "the check must see many applications, saw {apps}"
    );
}

#[test]
fn scratch_applications_agree_with_the_walk() {
    let mut k = prelude();
    k.s.begin_scratch();
    let x = k.s.bvar(0);
    let y = k.s.bvar(1);
    let mut e = x;
    for i in 0..40 {
        e = k.s.app(e, if i % 2 == 0 { y } else { x });
        agree(&k, e);
    }
    k.s.end_scratch();
}

#[test]
fn a_spine_longer_than_a_node_can_count_still_agrees() {
    let mut k = Kernel::new();
    let x = k.s.bvar(0);
    let y = k.s.bvar(1);
    let mut e = x;
    for i in 0..70_000u32 {
        e = k.s.app(e, if i % 3 == 0 { y } else { x });
    }
    agree(&k, e);
    let Node::App(f, _) = k.s.node(e) else {
        unreachable!()
    };
    agree(&k, f);
}
