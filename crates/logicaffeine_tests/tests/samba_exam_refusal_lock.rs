//! samba_exam_refusal_lock — the row of the Samba book's exam E8 (section 10.8 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! the compiler's own quality) on refusals: every refusal names its statement and its reason, and no program that lowers
//! only in part exits 0.
//!
//! A program here is wrong in a way no compiler can mend — an attention over more positions than its cache holds fails at
//! run time on every tier — beside statements that lower. `largo emit rdu` must refuse it: a nonzero exit, nothing under
//! the bundle's directory, nothing on stdout, and on stderr each refused statement by its index, its binder and its op, with
//! the reason it was refused.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_refusal_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;

use std::path::Path;

/// A program of a rotation-free decode block: a SiLU over the query row (it lowers), then each of `lengths` an attention of
/// that row over an 8-position cache up to its length.
fn program(lengths: &[usize]) -> String {
    let mut s = String::from("## Main\n");
    s += "Let Q be a 1 by 128 tensor of Real.\nLet K be a 8 by 64 tensor of Real.\nLet V be a 8 by 64 tensor of Real.\n";
    for (name, n) in [("Q", 128), ("K", 512), ("V", 512)] {
        for i in 0..n {
            s += &format!("Set item {} of {name} to {:?}.\n", i + 1, ((i * 7 + name.len()) % 11) as f64 / 16.0 - 0.25);
        }
    }
    s += "Let Y be silu of Q.\n";
    for (k, l) in lengths.iter().enumerate() {
        s += &format!("Let C{k} be attention of Y over K and V with 2 heads 1 groups up to {l}.\nShow item 1 of C{k}.\n");
    }
    s
}

/// `largo emit rdu` of `source` into a bundle directory: the exit code, what stdout and stderr say, and the files the
/// directory holds afterward.
fn emit(dir: &Path, source: &str) -> (Option<i32>, String, String, Vec<String>) {
    let file = dir.join("refused.lg");
    std::fs::write(&file, source).unwrap();
    let bundle = dir.join("bundle");
    let o = cli::largo_in(dir, &["emit", "rdu", file.to_str().unwrap(), "-o", bundle.to_str().unwrap()]);
    let mut files = Vec::new();
    let mut dirs = vec![bundle];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else {
                files.push(p.display().to_string());
            }
        }
    }
    (o.status.code(), cli::stdout(&o), cli::strip_ansi(&cli::stderr(&o)), files)
}

/// E8.6 — an attention past its cache beside a statement that lowers, and two such attentions: each refused by name, the
/// program never emitted in part.
#[test]
fn no_partial_program_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    for lengths in [&[9][..], &[9, 12][..]] {
        let (code, out, err, files) = emit(dir.path(), &program(lengths));
        assert!(code.is_some_and(|c| c != 0), "attentions up to {lengths:?} of 8 positions: refused with a nonzero exit, not {code:?}:\n{err}");
        assert!(out.trim().is_empty(), "attentions up to {lengths:?}: nothing emitted on stdout: {out}");
        assert!(files.is_empty(), "attentions up to {lengths:?}: no file of a bundle written: {files:?}");
        for k in 0..lengths.len() {
            assert!(err.contains(&format!("`C{k}` [Attention]")), "attentions up to {lengths:?}: the refusal names `C{k}` and its op:\n{err}");
        }
        assert_eq!(
            err.matches("a literal attention length outside its cache").count(),
            lengths.len(),
            "attentions up to {lengths:?}: each refused statement with its reason:\n{err}"
        );
        assert!(!err.contains("`Y`"), "attentions up to {lengths:?}: the SiLU lowers, and is not named among the refused:\n{err}");
        assert!(err.contains(&format!("cannot host {} of this program's {} tensor statements", lengths.len(), lengths.len() + 1)), "attentions up to {lengths:?}: the refusal counts what it cannot host:\n{err}");
    }
}
