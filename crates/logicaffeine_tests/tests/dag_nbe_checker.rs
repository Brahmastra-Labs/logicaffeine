//! The NbE checker's verdicts equal the classic checker's.
//!
//! For every declaration of every accepted Arena file the NbE checker must accept its type and
//! its value; and on deliberately mismatched pairs — a value against the next declaration's type —
//! both checkers must say the same thing.
//!
//! And what the NbE checker spends on Arena `perf/fueled-chain` grows with the chain, not with
//! two to the power of it.
#![cfg(feature = "nbe")]

use logicaffeine_kernel::dag::env::{Kind, Safety};
use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::nbe::{Nbe, State};
use logicaffeine_kernel::dag::Kernel;
use std::io::BufReader;
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/arena_corpus")
}

fn accepted_files() -> Vec<PathBuf> {
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(corpus().join("manifest.json")).unwrap())
            .unwrap();
    m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["location"] == "vendored" && c["outcome"] == "accept")
        .filter(|c| {
            !c["name"].as_str().unwrap().starts_with("perf/magma") && c["name"] != "perf/app-lam"
        })
        .map(|c| corpus().join(c["path"].as_str().unwrap()))
        .collect()
}

#[derive(Default)]
struct Tally {
    values: u64,
    mismatched_pairs: u64,
    both_reject: u64,
    failures: Vec<String>,
}

fn check_file(path: PathBuf) -> Tally {
    let mut k = Kernel::new();
    Reader::new(&mut k)
        .run(BufReader::new(std::fs::File::open(&path).unwrap()))
        .expect("file is accepted");
    let file = path.file_name().unwrap().to_string_lossy().to_string();
    let mut decls: Vec<_> = (0..k.s.names.len() as u32)
        .filter_map(|i| {
            k.env
                .get(logicaffeine_kernel::dag::name::NameId(i))
                .cloned()
        })
        .collect();
    decls.sort_by_key(|c| c.name.0);
    let mut t = Tally::default();
    for (i, c) in decls.iter().enumerate() {
        let (value, safety) = match c.kind {
            Kind::Defn { value, safety, .. } => (
                value,
                if safety == Safety::Unsafe {
                    Safety::Unsafe
                } else {
                    Safety::Safe
                },
            ),
            Kind::Thm { value } => (value, Safety::Safe),
            Kind::Opaque { value, .. } => (value, Safety::Safe),
            _ => continue,
        };
        let name = k.s.names.display(c.name);
        k.begin_decl(c.lparams.clone(), safety);
        let mut st = State::new();
        let r = {
            let mut n = Nbe::new(&mut k, &mut st);
            n.check_type(c.ty).and_then(|_| n.check_value(value, c.ty))
        };
        t.values += 1;
        if let Err(e) = r {
            t.failures.push(format!(
                "{file} :: {name}: NbE rejected an accepted declaration: {e}"
            ));
        }
        k.end_decl();
        if let Some(next) = decls.get(i + 1).filter(|n| n.lparams == c.lparams) {
            k.begin_decl(c.lparams.clone(), safety);
            let classic = k
                .infer(value, false)
                .and_then(|vt| k.is_def_eq(vt, next.ty));
            let classic_ok = matches!(classic, Ok(true));
            let mut st = State::new();
            let fast = Nbe::new(&mut k, &mut st).check_value(value, next.ty);
            t.mismatched_pairs += 1;
            if classic_ok != fast.is_ok() {
                t.failures.push(format!(
                    "{file} :: {name} against the type of {}: classic {classic:?}, NbE {fast:?}",
                    k.s.names.display(next.name)
                ));
            }
            if !classic_ok && fast.is_err() {
                t.both_reject += 1;
            }
            k.end_decl();
        }
    }
    t
}

#[test]
fn the_nbe_checker_agrees_with_the_classic_checker_on_the_arena_corpus() {
    let mut total = Tally::default();
    for f in accepted_files() {
        let t = std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn(move || check_file(f))
            .unwrap()
            .join()
            .unwrap();
        total.values += t.values;
        total.mismatched_pairs += t.mismatched_pairs;
        total.both_reject += t.both_reject;
        total.failures.extend(t.failures);
    }
    eprintln!(
        "checked {} values; {} mismatched pairs, {} rejected by both",
        total.values, total.mismatched_pairs, total.both_reject
    );
    assert!(total.values > 1_500, "only {} values checked", total.values);
    assert!(
        total.both_reject > 300,
        "only {} mismatched pairs were rejected by both",
        total.both_reject
    );
    assert!(
        total.failures.is_empty(),
        "{} failures:\n{}",
        total.failures.len(),
        total.failures[..total.failures.len().min(40)].join("\n")
    );
}

/// The values the NbE checker creates to check `name` (its type and its value) in `k`.
fn values_to_check(k: &mut Kernel, name: &str) -> usize {
    let n = k.s.names.dotted(name);
    let c = k
        .env
        .get(n)
        .cloned()
        .unwrap_or_else(|| panic!("'{name}' is admitted"));
    let Kind::Thm { value } = c.kind else {
        panic!("'{name}' is a theorem")
    };
    k.begin_decl(c.lparams.clone(), Safety::Safe);
    let mut st = State::new();
    let r = {
        let mut n = Nbe::new(k, &mut st);
        n.check_type(c.ty).and_then(|_| n.check_value(value, c.ty))
    };
    k.end_decl();
    r.unwrap_or_else(|e| panic!("NbE rejected {name}: {e}"));
    st.arena.len()
}

/// `chainN_datF` states that a program of N binds, each followed by an `unless … throw` guard,
/// run in the fuel-indexed monad `Fueled` and read at a fuel, is the program run in `Except`.
/// Checking it compares two programs whose every `bind` carries a proof (`Subtype.mk p h`) whose
/// type mentions the rest of the program twice, under binders, and checks a proof term in which
/// the program occurs under many binders it does not mention. What is built under a binder is
/// built again, identically, under the next binder of that type at that depth; and a subterm is
/// one value (and one check) wherever the variables it mentions agree — so a guard adds work in
/// proportion to the program, and the total grows polynomially. Meeting every binder with a
/// variable never seen before, keeping with a subterm the variables it does not mention, or
/// checking a subterm again at each of its occurrences each doubles the work per guard (chain12
/// alone then takes hours).
///
/// Measured as the values created, which is deterministic: `chain6` must stay within a bound
/// (checked first, so that an exponential checker fails in seconds instead of exhausting the
/// machine on the longer chains), and three more guards must cost at most half again what the
/// previous three cost — doubling per guard makes it eight times.
#[test]
fn the_nbe_checker_does_not_double_its_work_per_guard() {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let mut k = Kernel::new();
            let path = corpus().join("good/perf/fueled-chain.ndjson");
            Reader::new(&mut k).run(BufReader::new(std::fs::File::open(path).unwrap())).expect("file is accepted");
            let v6 = values_to_check(&mut k, "chain6_datF");
            assert!(v6 < FUELED_CHAIN6_BOUND, "chain6_datF took {v6} values (bound {FUELED_CHAIN6_BOUND})");
            let v9 = values_to_check(&mut k, "chain9_datF");
            let v12 = values_to_check(&mut k, "chain12_datF");
            eprintln!("values: chain6 {v6}, chain9 {v9}, chain12 {v12}");
            assert!(v6 < v9 && v9 < v12, "longer chains cost more: {v6}, {v9}, {v12}");
            assert!(
                2 * (v12 - v9) <= 3 * (v9 - v6),
                "three more guards cost more than half again what the previous three cost: {v6}, {v9}, {v12}"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// About five times what `chain6_datF` takes (20 924 values); a checker that doubles its work per
/// guard takes millions.
const FUELED_CHAIN6_BOUND: usize = 100_000;
