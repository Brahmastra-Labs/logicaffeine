//! Generics, fully supported: every program in `generics_corpus.in` must print exactly its expected
//! output on the tree-walker, the VM, the AOT-Rust tier and the C tier (WASM runs the same corpus in
//! `wasm_aot_lock.rs`). Each tier sweep reports EVERY failing program at once, so one run is the
//! whole picture.

mod common;
use common::{compile_logos, run_all_tiers};

const CORPUS: &[(&str, &str, &str)] = include!("generics_corpus.in");

fn report(tier: &str, failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{} of {} generic programs fail on the {tier} tier:\n\n{}",
        failures.len(),
        CORPUS.len(),
        failures.join("\n\n")
    );
}

#[test]
fn the_corpus_is_broad() {
    assert!(
        CORPUS.len() >= 40,
        "the generics corpus must stay broad, has {}",
        CORPUS.len()
    );
    let mut names: Vec<&str> = CORPUS.iter().map(|(n, _, _)| *n).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), CORPUS.len(), "corpus program names are unique");
}

#[test]
fn generics_on_the_tree_walker_and_vm() {
    let failures = CORPUS
        .iter()
        .filter_map(|(name, src, expected)| {
            let outcome = std::panic::catch_unwind(|| run_all_tiers(src));
            match outcome {
                Err(p) => Some(format!(
                    "{name}: tiers disagree or crash: {}\n{src}",
                    panic_text(&p)
                )),
                Ok(r) if r.error.is_some() => {
                    Some(format!("{name}: error {:?}\n{src}", r.error.unwrap()))
                }
                Ok(r) if r.output.trim() != *expected => Some(format!(
                    "{name}: printed {:?}, expected {expected:?}\n{src}",
                    r.output.trim()
                )),
                Ok(_) => None,
            }
        })
        .collect();
    report("tree-walker/VM", failures);
}

#[test]
fn generics_on_the_rust_tier() {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(6);
    let chunk = CORPUS.len().div_ceil(threads);
    let failures: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = CORPUS
            .chunks(chunk)
            .map(|batch| {
                s.spawn(move || {
                    batch
                        .iter()
                        .filter_map(|(name, src, expected)| {
                            let built = compile_logos(src);
                            if !built.success {
                                return Some(format!("{name}: does not build: {}\n{src}", built.stderr.lines().take(12).collect::<Vec<_>>().join("\n")));
                            }
                            let out = std::process::Command::new(&built.binary_path).output().expect("runs");
                            let stdout = String::from_utf8_lossy(&out.stdout);
                            if !out.status.success() || stdout.trim() != *expected {
                                return Some(format!(
                                    "{name}: printed {:?} (status {}), expected {expected:?}\n{src}",
                                    stdout.trim(),
                                    out.status
                                ));
                            }
                            None
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    report("AOT-Rust", failures);
}

#[test]
fn generics_on_the_c_tier() {
    let failures = CORPUS
        .iter()
        .filter_map(|(name, src, expected)| {
            let c_code = match logicaffeine_compile::compile::compile_to_c(src) {
                Ok(c) => c,
                Err(e) => return Some(format!("{name}: C lowering refused: {e:?}\n{src}")),
            };
            let dir = tempfile::tempdir().expect("temp dir");
            let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
            std::fs::write(&c_path, &c_code).expect("write C");
            let gcc = std::process::Command::new("gcc")
                .args(["-O2", "-o"])
                .arg(&bin)
                .arg(&c_path)
                .arg("-lm")
                .output()
                .expect("gcc runs");
            if !gcc.status.success() {
                let err = String::from_utf8_lossy(&gcc.stderr);
                return Some(format!(
                    "{name}: C does not build: {}\n{src}",
                    err.lines().take(8).collect::<Vec<_>>().join("\n")
                ));
            }
            let out = std::process::Command::new(&bin).output().expect("runs");
            let stdout = String::from_utf8_lossy(&out.stdout);
            (!out.status.success() || stdout.trim() != *expected).then(|| {
                format!(
                    "{name}: printed {:?} (status {}), expected {expected:?}\n{src}",
                    stdout.trim(),
                    out.status
                )
            })
        })
        .collect();
    report("C", failures);
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}
