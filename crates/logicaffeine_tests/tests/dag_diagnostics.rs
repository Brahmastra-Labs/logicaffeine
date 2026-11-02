//! A rejection names the declaration it rejects: a definition by its own name, an inductive block
//! by its first type — whichever engine checks it.

use logicaffeine_kernel::dag::tc::Engine;
use logicaffeine_kernel::dag::{check_export_with, Verdict};
use std::io::BufReader;
use std::path::PathBuf;

fn rejected_declaration(case: &str, engine: Engine) -> Option<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(case);
    let text = std::fs::read(path).unwrap();
    let verdict = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || check_export_with(BufReader::new(&text[..]), |k| k.engine = engine).0)
        .unwrap()
        .join()
        .unwrap();
    match verdict {
        Verdict::Reject(e) => e.declaration.clone(),
        other => panic!(
            "{case}: expected a rejection, got exit {}",
            other.exit_code()
        ),
    }
}

#[test]
fn a_rejected_definition_is_named() {
    for engine in [Engine::Classic, Engine::Sem] {
        assert_eq!(
            rejected_declaration("bad/tutorial/002_badDef.ndjson", engine).as_deref(),
            Some("badDef")
        );
    }
}

#[test]
fn a_rejected_inductive_block_is_named_by_its_first_type() {
    for engine in [Engine::Classic, Engine::Sem] {
        assert_eq!(
            rejected_declaration("bad/tutorial/046_inductBadNonSort.ndjson", engine).as_deref(),
            Some("inductBadNonSort")
        );
    }
}

#[test]
fn a_rejected_theorem_is_named() {
    for engine in [Engine::Classic, Engine::Sem] {
        assert_eq!(
            rejected_declaration("bad/tutorial/012_nonPropThm.ndjson", engine).as_deref(),
            Some("nonPropThm")
        );
    }
}
