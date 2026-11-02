//! A declaration's own terms live in the store's scratch region, whose ids are recycled when the
//! declaration ends. The value engine's session outlives declarations, so nothing it remembers
//! may be keyed on a scratch id: the next declaration's different term, given the same id, would
//! be answered with the old term's value.

use logicaffeine_kernel::dag::env::Safety;
use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::expr::ExprId;
use logicaffeine_kernel::dag::level::LevelsId;
use logicaffeine_kernel::dag::sem::value::{EnvId, Val};
use logicaffeine_kernel::dag::Kernel;
use std::io::BufReader;
use std::path::PathBuf;
use std::rc::Rc;

fn load(path: &str) -> Kernel {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(path);
    let mut k = Kernel::new();
    Reader::new(&mut k)
        .run(BufReader::new(std::fs::File::open(file).unwrap()))
        .expect("the case is accepted");
    k
}

/// Builds a term with `make` inside a fresh declaration and evaluates it, twice: the second
/// declaration's term reuses the first's scratch id. Returns both ids and both values' shapes.
fn evaluate_in_two_declarations(
    k: &mut Kernel,
    make: impl Fn(&mut Kernel, u64) -> ExprId,
) -> [(ExprId, Val); 2] {
    [7, 9].map(|n| {
        k.with_decl(Rc::from(Vec::new()), Safety::Safe, |k| {
            let e = make(k, n);
            assert!(
                e.is_scratch(),
                "a term built during a declaration is scratch"
            );
            k.with_sem(|sem| {
                let v = sem.eval(EnvId::EMPTY, e)?;
                Ok((e, sem.st.arena.get(v)))
            })
        })
        .unwrap()
    })
}

#[test]
fn a_sort_is_never_answered_by_a_recycled_scratch_id() {
    let mut k = Kernel::new();
    let [(e1, v1), (e2, v2)] = evaluate_in_two_declarations(&mut k, |k, n| {
        let l = k.s.levels.of_nat(n + 100);
        k.s.sort(l)
    });
    assert_eq!(
        e1, e2,
        "the second declaration's sort reuses the first's scratch id"
    );
    let (Val::Sort(l1), Val::Sort(l2)) = (v1, v2) else {
        panic!("sorts evaluate to sorts")
    };
    assert_ne!(l1, l2, "Sort 109 is not Sort 107");
    assert_eq!(l2, k.s.levels.of_nat(109));
}

#[test]
fn a_constant_is_never_answered_by_a_recycled_scratch_id() {
    let mut k = load("good/init-prelude.ndjson");
    let rec = k.s.names.dotted("Nat.rec");
    let [(e1, v1), (e2, v2)] = evaluate_in_two_declarations(&mut k, |k, n| {
        let l = k.s.levels.of_nat(n + 100);
        let ls = k.s.levels.list(&[l]);
        k.s.cnst(rec, ls)
    });
    assert_eq!(
        e1, e2,
        "the second declaration's constant reuses the first's scratch id"
    );
    assert_ne!(v1, v2, "Nat.rec.{{107}} is not Nat.rec.{{109}}");
    let at = |k: &mut Kernel, v: Val| -> LevelsId {
        let Val::Neu { head, .. } = v else {
            panic!("a constant evaluates to a neutral")
        };
        k.with_sem(|sem| match sem.st.arena.head(head) {
            logicaffeine_kernel::dag::sem::value::Head::Const { levels, .. } => Ok(levels),
            h => panic!("a constant head, not {h:?}"),
        })
        .unwrap()
    };
    let l109 = k.s.levels.of_nat(109);
    let want = k.s.levels.list(&[l109]);
    assert_eq!(
        at(&mut k, v2),
        want,
        "the second declaration evaluates its own constant"
    );
}
