//! Structure η on a recursor's major premise, taken before the major is reduced (the Arena's
//! `corner-cases/eta-struct-major-*`): every engine expands the major of a structure's recursor into its constructor
//! applied to its projections when the structure lives in a universe that is never `Prop`, or in `Prop` with a
//! recursor that eliminates into every universe, so that a proof with no weak head normal form never has to be reduced.
//!
//! The corpus cases hold each engine to deciding the two majors that have no weak head normal form
//! (`arena_corpus`, `dag_sem_corpus`, `dag_dual_corpus`, `dag_nbe_differential`). This suite holds the rule that says
//! which structures qualify (`Env::eliminates_into_every_sort`, read off each inductive's own recursor), whose `Prop`
//! branch no verdict can show: a proposition's recursor that eliminates only into `Prop` returns a proof either way.

use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::Kernel;
use std::io::BufReader;
use std::path::PathBuf;

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

fn run<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn an_inductive_eliminates_into_every_sort_exactly_when_its_own_recursor_takes_the_motives_universe() {
    run(|| {
        let mut k = load("good/init-prelude.ndjson");
        for (inductive, every, why) in [
            ("Prod", true, "a structure in Type"),
            ("PProd", true, "a structure in any sort but Prop"),
            ("And", true, "a proposition whose fields are proofs eliminates into every universe"),
            ("True", true, "a proposition with no fields eliminates into every universe"),
            ("Eq", true, "an equality eliminates into every universe, though it is no structure"),
            ("Nonempty", false, "a proposition holding data eliminates only into Prop"),
        ] {
            let n = k.s.names.dotted(inductive);
            assert!(k.env.inductive(n).is_some(), "{inductive} is an inductive of the prelude");
            assert_eq!(k.env.eliminates_into_every_sort(n), every, "{inductive}: {why}");
        }
        let rec = k.s.names.dotted("Lean.Syntax.rec_1");
        let nested = k.env.rec(rec).expect("Lean.Syntax.rec_1 is a nested auxiliary recursor").major_induct;
        assert!(
            k.env.eliminates_into_every_sort(nested),
            "the question is the major's own inductive's, whichever recursor asks it"
        );
    })
}
