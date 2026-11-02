//! Every backend the product ships is a column of one parity table, every statement and expression arm has a
//! probe that reaches it, and the semantic probes run bit for bit on the engines and the generated Rust.
//!
//! A LOGOS program has one meaning. The tree-walker, the bytecode VM with and without its optimizer, the JIT's
//! x86-64 code, the three Futamura projections, the generated Rust under both shipped profiles, both C
//! lowerings, WebAssembly, RISC-V, CUDA, Tenstorrent, the soft core and the spatial fabric are each a column of
//! [`tier_parity`]'s table: standard output, standard error and the way the run ended, compared byte for byte.
//! Nothing is trimmed, and nothing counts unless it was built and run.
//!
//! The denominators are computed, never curated. Statements, expressions and binary operators come from total
//! matches over the AST, backends from the product's `Backend` enum and `largo emit`'s own targets. A new arm or
//! backend reddens this file until a probe reaches it.
//!
//! `LOGOS_PARITY_CENSUS=<path>` makes [`parity_census`] run every program on every backend and write the whole
//! table, one row per (program, backend), with `LOGOS_PARITY_JOBS`, `LOGOS_PARITY_BACKENDS` (names, comma
//! separated) and `LOGOS_PARITY_FILTER` (substrings of the program id, comma separated) to shard it.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use std::collections::{BTreeMap, BTreeSet};

use tier_parity::corpus::{corpus, Corpus};
use tier_parity::kinds::{self, Probe};
use tier_parity::{rust_batch, Backend, Program, Row, Verdict};

fn corpus_once() -> &'static Corpus {
    static C: std::sync::OnceLock<Corpus> = std::sync::OnceLock::new();
    C.get_or_init(corpus)
}

fn jobs() -> usize {
    std::env::var("LOGOS_PARITY_JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .unwrap_or(4)
}

fn selected_backends() -> Vec<Backend> {
    match std::env::var("LOGOS_PARITY_BACKENDS") {
        Ok(names) if !names.trim().is_empty() => names
            .split(',')
            .map(|n| {
                let n = n.trim();
                Backend::ALL
                    .into_iter()
                    .find(|b| b.name() == n)
                    .unwrap_or_else(|| panic!("no backend is named `{n}`"))
            })
            .collect(),
        _ => Backend::ALL.to_vec(),
    }
}

/// The programs whose id contains one of `LOGOS_PARITY_FILTER`'s comma-separated alternatives, or every
/// program when it is unset.
fn selected_programs(programs: &[Program]) -> Vec<Program> {
    let filter = std::env::var("LOGOS_PARITY_FILTER").unwrap_or_default();
    let alternatives: Vec<&str> = filter
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .collect();
    programs
        .iter()
        .filter(|p| alternatives.is_empty() || alternatives.iter().any(|a| p.id.contains(a)))
        .cloned()
        .collect()
}

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

#[test]
fn every_backend_is_listed_once_by_a_distinct_name() {
    let names: BTreeSet<&str> = Backend::ALL.iter().map(|b| b.name()).collect();
    assert_eq!(
        names.len(),
        Backend::ALL.len(),
        "two backends share a name, or one is listed twice"
    );
}

#[test]
fn every_backend_the_product_lowers_to_is_a_column() {
    for b in logicaffeine_compile::compile::Backend::ALL {
        assert!(
            Backend::ALL.contains(&Backend::of_product(b)),
            "the product's {b:?} has no column"
        );
    }
}

#[test]
fn every_target_largo_emits_is_a_column() {
    use clap::CommandFactory;
    let cli = logicaffeine_cli::cli::Cli::command();
    let emit = cli
        .find_subcommand("emit")
        .expect("largo has an `emit` command");
    let target = emit
        .get_arguments()
        .find(|a| a.get_id() == "target")
        .expect("`emit` takes a target");
    let names: Vec<String> = target
        .get_possible_values()
        .iter()
        .map(|v| v.get_name().to_string())
        .collect();
    assert!(
        names.len() >= 10,
        "read only {} emit targets: {names:?}",
        names.len()
    );
    let missing: Vec<&String> = names
        .iter()
        .filter(|n| Backend::of_emit_target(n).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "`largo emit` targets with no column in the parity sweep: {missing:?}"
    );
}

#[test]
fn every_statement_and_expression_kind_is_listed_once() {
    let stmts: BTreeSet<_> = kinds::ALL_STMT_KINDS.iter().collect();
    assert_eq!(
        stmts.len(),
        kinds::ALL_STMT_KINDS.len(),
        "ALL_STMT_KINDS lists a kind twice"
    );
    assert_eq!(
        stmts.len(),
        86,
        "a `Stmt` kind is classified but not listed, so no probe of it ever runs"
    );
    let exprs: BTreeSet<_> = kinds::ALL_EXPR_KINDS.iter().collect();
    assert_eq!(
        exprs.len(),
        kinds::ALL_EXPR_KINDS.len(),
        "ALL_EXPR_KINDS lists a kind twice"
    );
    assert_eq!(
        exprs.len(),
        30,
        "an `Expr` kind is classified but not listed, so no probe of it ever runs"
    );
    let binops: BTreeSet<_> = kinds::ALL_BINOPS
        .iter()
        .map(kinds::classify_binop)
        .collect();
    assert_eq!(
        binops.len(),
        24,
        "a binary operator is classified but not listed"
    );
}

/// A probe that never contains the construct it is filed under proves nothing about that construct. A
/// lowered-away probe holds the opposite: the construct is written in its source and gone after the front end,
/// so no backend ever receives it.
#[test]
fn every_probe_reaches_the_arm_it_names() {
    let mut wrong = Vec::new();
    let mut check = |construct: String, probe: Probe, has: &dyn Fn(&kinds::Reached) -> bool| {
        let (programs, lowered_away): (&[&str], bool) = match probe {
            Probe::Programs(p)
            | Probe::Tests(p)
            | Probe::Pages(p)
            | Probe::Servers(p)
            | Probe::RustOnly(p) => (p, false),
            Probe::LoweredAway(p) => (p, true),
            Probe::Open(_) => return,
        };
        for (i, program) in programs.iter().enumerate() {
            match kinds::reached(program) {
                Err(e) => wrong.push(format!("{construct}/{i} does not parse: {}", e.lines().next().unwrap_or(""))),
                Ok(r) if lowered_away && has(&r) => {
                    wrong.push(format!("{construct}/{i} still contains a {construct} after the front end, so a backend receives it"))
                }
                Ok(r) if !lowered_away && !has(&r) => {
                    wrong.push(format!("{construct}/{i} never contains a {construct}; it has {:?}", r.stmts))
                }
                Ok(_) => {}
            }
        }
    };
    for &k in kinds::ALL_STMT_KINDS {
        check(format!("stmt/{k:?}"), kinds::stmt_probe(k), &|r| {
            r.stmts.contains(&k)
        });
    }
    for &k in kinds::ALL_EXPR_KINDS {
        check(format!("expr/{k:?}"), kinds::expr_probe(k), &|r| {
            r.exprs.contains(&k)
        });
    }
    assert!(
        wrong.is_empty(),
        "{} probes do not exercise what they are filed under:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
}

/// A probe is written to exercise a construct, so one that does not parse exercises nothing. (A program of
/// the corpus the front end rejects is not a defect: it runs on every backend, which must all reject it alike.)
#[test]
fn every_probe_parses() {
    let bad: Vec<String> = tier_parity::corpus::probes()
        .into_iter()
        .filter_map(|(id, source, _)| {
            kinds::reached(source)
                .err()
                .map(|why| format!("{id}: {}", why.lines().next().unwrap_or("")))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "{} probes do not parse:\n  {}",
        bad.len(),
        bad.join("\n  ")
    );
}

/// The shipped release profile is fully classified, so the batch build is the shipped build in every key that
/// can change what a program does.
#[test]
fn the_shipped_release_profile_is_fully_classified() {
    let dir = tier_parity::scratch_dir("release-profile");
    let keys = rust_batch::shipped_release_profile(&dir).expect("the shipped release profile");
    assert!(
        keys.iter().any(|(k, _)| k == "panic"),
        "the shipped profile names no panic strategy: {keys:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The semantic probes ([`tier_parity::semantic`]) on the engines and the generated Rust under both shipped
/// profiles: the backends whose fast paths each rule was first written into. The full sweep runs them on
/// every backend.
#[test]
fn the_semantic_probes_run_bit_for_bit_on_the_engines_and_the_rust_backends() {
    let programs: Vec<Program> = tier_parity::semantic::SEMANTIC_PROBES
        .iter()
        .map(|(name, source)| Program::new(format!("semantic/{name}"), *source))
        .collect();
    let backends = [
        Backend::TreeWalker,
        Backend::Vm,
        Backend::Interpret,
        Backend::RustDebug,
        Backend::RustRelease,
    ];
    let rows = tier_parity::sweep(&programs, &backends, jobs())
        .unwrap_or_else(|why| panic!("the Rust batch build could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| !r.matches())
        .map(|r| {
            format!(
                "{} on {}: {} {}",
                r.program,
                r.backend.name(),
                r.status(),
                r.detail()
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "{} (probe, backend) runs differ from the VM:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// A worker's scratch lives in its job's directory, which its parent removes with the job: no worker leaves a
/// directory of its own behind in the temporary directory.
#[test]
fn a_worker_keeps_its_scratch_in_its_job() {
    let job = std::env::temp_dir().join(format!("logos-parity-scratch-lock-{}", std::process::id()));
    std::env::set_var("LOGOS_PARITY_JOB", &job);
    let root = tier_parity::scratch_root();
    std::env::remove_var("LOGOS_PARITY_JOB");
    assert_eq!(root, job.join("scratch"), "a worker's scratch is its job's");
    let _ = std::fs::remove_dir_all(&job);
}

/// A sweep that builds Rust releases the project and the target it built in when it is done: a test process
/// leaves no build behind.
#[test]
fn a_sweep_leaves_no_rust_build_behind() {
    let programs = vec![Program::new("tidy/one", "## Main\nShow 1.\n")];
    let rows = tier_parity::sweep(&programs, &[Backend::RustDebug], jobs())
        .unwrap_or_else(|why| panic!("the Rust batch build could not start: {why}"));
    assert!(rows.iter().all(|r| r.matches()), "the program runs on Rust as on the VM");
    let scratch = tier_parity::scratch_root();
    assert!(!scratch.join("rust-batch").exists(), "the batch's project outlived its sweep");
    assert!(!scratch.join("rust-batch-target").exists(), "the batch's target outlived its sweep");
}

/// Run the sweep over the selected programs and backends.
fn sweep() -> Vec<Row> {
    let c = corpus_once();
    let mut programs = selected_programs(&c.programs);
    let rust_only = selected_programs(&c.rust_only);
    programs.extend(rust_only.iter().cloned());
    let backends = selected_backends();
    let mut rows = tier_parity::sweep(&programs, &backends, jobs())
        .unwrap_or_else(|why| panic!("the Rust batch build could not start: {why}"));
    let ids: BTreeSet<&str> = rust_only.iter().map(|p| p.id.as_str()).collect();
    judge_rust_only(&mut rows, &ids);
    rows
}

/// Tristen's ruling on `Escape to Rust:`: a Rust-only program means what it does on the two Rust backends, which
/// must both run it and agree with each other; every other backend refuses it exactly as the VM does, which the
/// sweep already judges against the VM.
fn judge_rust_only(rows: &mut [Row], ids: &BTreeSet<&str>) {
    use tier_parity::End;
    let is_rust = |b: Backend| matches!(b, Backend::RustDebug | Backend::RustRelease);
    let rust: BTreeMap<(String, Backend), Verdict> = rows
        .iter()
        .filter(|r| ids.contains(r.program.as_str()) && is_rust(r.backend))
        .map(|r| ((r.program.clone(), r.backend), r.verdict.clone()))
        .collect();
    for r in rows
        .iter_mut()
        .filter(|r| ids.contains(r.program.as_str()) && is_rust(r.backend))
    {
        let other = if r.backend == Backend::RustDebug {
            Backend::RustRelease
        } else {
            Backend::RustDebug
        };
        r.difference = match (&r.verdict, rust.get(&(r.program.clone(), other))) {
            (Verdict::Observed(o), _) if o.end != End::Finished => Some(format!(
                "a Rust backend must run a Rust-only program; it ended {:?}",
                o.end
            )),
            (Verdict::Observed(o), Some(Verdict::Observed(p))) => o
                .difference(p)
                .map(|d| format!("the two Rust backends differ: {d}")),
            _ => None,
        };
    }
}

/// The census: every (program, backend) row of the sweep, plus the dimensions the sweep does not yet run.
#[test]
#[ignore = "the census writer: LOGOS_PARITY_CENSUS=<path> writes the table and its summary"]
fn parity_census() {
    let Ok(path) = std::env::var("LOGOS_PARITY_CENSUS") else {
        eprintln!("parity_census: set LOGOS_PARITY_CENSUS=<path> to write the table");
        return;
    };
    let c = corpus_once();
    let rows = sweep();
    let mut table = tier_parity::census_tsv(&rows);
    let dimensions: [(&str, &Vec<Program>); 3] = [
        ("test", &c.tests),
        ("page", &c.pages),
        ("server", &c.servers),
    ];
    for (dimension, programs) in dimensions {
        for p in selected_programs(programs) {
            table.push_str(&format!("{}\t({dimension})\tUNRUN\tthe sweep does not yet run a {dimension} program on every backend\n", p.id));
        }
    }
    for (construct, why) in &c.open {
        table.push_str(&format!(
            "{construct}\t-\tOPEN\t{}\n",
            why.replace(['\t', '\n'], " ")
        ));
    }
    for (id, why) in &c.unparsed_probes {
        table.push_str(&format!(
            "{id}\t-\tUNPARSED-PROBE\t{}\n",
            why.lines().next().unwrap_or("").replace('\t', " ")
        ));
    }
    std::fs::write(&path, table).expect("write the census");
    let summary = format!(
        "programs {}  rust-only {}  tests {}  pages {}  servers {}  open {}  unparsed probes {}\n{}",
        c.programs.len(),
        c.rust_only.len(),
        c.tests.len(),
        c.pages.len(),
        c.servers.len(),
        c.open.len(),
        c.unparsed_probes.len(),
        tier_parity::census_summary(&rows)
    );
    std::fs::write(format!("{path}.summary"), &summary).expect("write the summary");
    eprintln!("parity_census: wrote {path}\n{summary}");
}
