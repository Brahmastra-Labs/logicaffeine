//! The value engine and the substitution engine judge every declaration alike.
//!
//! For every declaration of every accepted Arena file, the value engine must accept its type and
//! its value; and on deliberately mismatched pairs — each value against the type of the next
//! declaration with the same universe parameters — both engines must give the same answer. The
//! pairs are mostly false, so they exercise the refutation paths a valid file never takes.

use logicaffeine_kernel::dag::env::{Kind, Safety};
use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::sem::value::{CtxId, EnvId};
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
        .filter(|c| !c["name"].as_str().unwrap().starts_with("perf/magma"))
        .map(|c| corpus().join(c["path"].as_str().unwrap()))
        .collect()
}

#[derive(Default)]
struct Tally {
    values: u64,
    pairs: u64,
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
        if safety == Safety::Unsafe {
            continue;
        }
        let name = k.s.names.display(c.name);
        t.values += 1;
        let r = k.with_decl(c.lparams.clone(), safety, |k| {
            k.with_sem(|sem| {
                sem.check_type(c.ty)?;
                sem.check_value(value, c.ty)
            })
        });
        if let Err(e) = r {
            t.failures.push(format!(
                "{file} :: {name}: the value engine rejected an accepted declaration: {e}"
            ));
        }
        if let Some(next) = decls.get(i + 1).filter(|n| n.lparams == c.lparams) {
            let classic = k.with_decl(c.lparams.clone(), safety, |k| {
                let vt = k.infer(value, false)?;
                k.is_def_eq(vt, next.ty)
            });
            let sem = k.with_decl(c.lparams.clone(), safety, |k| {
                k.with_sem(|sem| {
                    let vt = sem.infer(EnvId::EMPTY, CtxId::EMPTY, value, false)?;
                    let nt = sem.eval(EnvId::EMPTY, next.ty)?;
                    sem.is_def_eq(vt, nt)
                })
            });
            t.pairs += 1;
            match (&classic, &sem) {
                (Ok(a), Ok(b)) if a == b => {}
                _ => t.failures.push(format!(
                    "{file} :: {name} against the type of {}: substitution {classic:?}, value {sem:?}",
                    k.s.names.display(next.name)
                )),
            }
        }
    }
    t
}

#[test]
fn value_engine_agrees_with_substitution_engine_on_every_declaration() {
    let files = accepted_files();
    assert!(
        files.len() >= 100,
        "the corpus's accepted files are present"
    );
    let results: Vec<Tally> = std::thread::Builder::new()
        .stack_size(8 << 30)
        .spawn(move || files.into_iter().map(check_file).collect())
        .unwrap()
        .join()
        .unwrap();
    let values: u64 = results.iter().map(|t| t.values).sum();
    let pairs: u64 = results.iter().map(|t| t.pairs).sum();
    let failures: Vec<String> = results.into_iter().flat_map(|t| t.failures).collect();
    assert!(
        failures.is_empty(),
        "{} disagreements:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        values >= 2000 && pairs >= 800,
        "{values} values and {pairs} pairs checked"
    );
}
