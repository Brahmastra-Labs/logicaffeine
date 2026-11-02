//! Shared support for the W06 (lane LG) original-witness tests: a pinned Lean 4.14.0 export of a
//! small witness file under `fixtures/lean_scale/`, verified byte for byte against its provenance and
//! its unchanged source, imported once by the production reader, and checked root by root through
//! the bridge-harvest checker (`harvest_support/checker.rs`).
#![allow(dead_code)]

use adapter::reference::kernel_name_of_components;
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{infer_type, recheck, BigInt, Context, Literal, Term};
use serde_json::Value;
use std::path::PathBuf;

pub fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn scale_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lean_scale")
}

/// A dotted Lean name in the kernel's spelling (no `«…»` components occur in the witnesses).
pub fn kernel_spelling(lean: &str) -> String {
    kernel_name_of_components(&lean.split('.').collect::<Vec<_>>())
}

/// A witness export (`declaration_selection = explicit_witness_roots`), verified against its
/// provenance, its unchanged source and the pinned toolchain; its roots are exactly `roots`, and
/// Lean's own `#print axioms` recorded every root as axiom-free.
pub fn witness(stem: &str, roots: &[&str]) -> (String, Value) {
    witness_with_axioms(stem, roots, &vec![&[][..]; roots.len()])
}

/// [`witness`], where Lean's `#print axioms` recorded `axioms[i]` for `roots[i]`.
pub fn witness_with_axioms(stem: &str, roots: &[&str], axioms: &[&[&str]]) -> (String, Value) {
    assert_eq!(roots.len(), axioms.len(), "one axiom list per root");
    let dir = scale_fixtures();
    let export =
        std::fs::read_to_string(dir.join(format!("{stem}.ndjson"))).expect("pinned export");
    let provenance: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!("{stem}.provenance.json"))).expect("provenance"),
    )
    .expect("provenance parses");
    assert_eq!(
        adapter::package::sha256_hex(export.as_bytes()),
        provenance["export_sha256"],
        "{stem}: export bytes"
    );
    assert_eq!(
        export.lines().count() as u64,
        provenance["records"].as_u64().unwrap(),
        "{stem}: record count"
    );
    let header: Value = serde_json::from_str(export.lines().next().unwrap()).unwrap();
    assert_eq!(header, provenance["export_header"], "{stem}: export header");
    assert_eq!(header["meta"]["lean"]["version"], "4.14.0");
    assert_eq!(provenance["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        provenance["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(
        provenance["exporter_source_sha256"]["Export.lean"],
        "5fecc2ebc45c62b618ec47705d47479117533676ae2f8e1e7e6740d941153224",
        "{stem}: the structural-key exporter"
    );
    assert_eq!(provenance["expression_equality"], "structural");
    assert_eq!(
        provenance["declaration_selection"],
        "explicit_witness_roots"
    );
    assert_eq!(provenance["compiler_exit"], 0);
    assert_eq!(provenance["exporter_exit"], 0);
    let source = std::fs::read(repository().join(provenance["source"].as_str().unwrap()))
        .expect("witness source");
    assert_eq!(
        adapter::package::sha256_hex(&source),
        provenance["source_sha256"],
        "{stem}: the witness source changed"
    );
    let declared: Vec<&str> = provenance["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        declared, roots,
        "{stem}: the export's roots are exactly the witness roots"
    );
    let oracle: Vec<&str> = provenance["lean_print_axioms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let expected: Vec<String> = roots
        .iter()
        .zip(axioms)
        .map(|(r, a)| {
            if a.is_empty() {
                format!("'{r}' does not depend on any axioms")
            } else {
                format!("'{r}' depends on axioms: [{}]", a.join(", "))
            }
        })
        .collect();
    assert_eq!(oracle, expected, "{stem}: Lean's own #print axioms");
    (export, provenance)
}

/// One production import of `export`, retaining the opaque proofs of `theorems` (Lean spellings).
pub fn imported(
    export: &str,
    theorems: &[&str],
) -> Result<(adapter::ImportedContext, adapter::ImportReport), String> {
    let retain: Vec<String> = theorems.iter().map(|r| kernel_spelling(r)).collect();
    adapter::import_audited_retaining(export, &retain).map_err(|e| format!("{e:?}"))
}

/// `(λ(__statement : expected). 0) term`: each checker converts `term`'s type to `expected` itself.
pub fn against(term: &Term, expected: &Term) -> Term {
    Term::App(
        Box::new(Term::Lambda {
            param: "__statement".into(),
            param_type: Box::new(expected.clone()),
            body: Box::new(Term::Lit(Literal::Nat(BigInt::from_u64(0)))),
        }),
        Box::new(term.clone()),
    )
}

pub fn both_accept(ctx: &Context, term: &Term) -> Result<(), String> {
    infer_type(ctx, term).map_err(|e| format!("primary kernel: {e}"))?;
    recheck(ctx, term).map_err(|e| format!("independent kernel: {e:?}"))?;
    Ok(())
}

pub fn both_refuse(ctx: &Context, term: &Term) -> bool {
    infer_type(ctx, term).is_err() && recheck(ctx, term).is_err()
}

/// Replace the last argument of an application spine.
pub fn with_last_argument(statement: &Term, change: impl FnOnce(&Term) -> Term) -> Term {
    match statement {
        Term::App(f, a) => Term::App(f.clone(), Box::new(change(a))),
        other => panic!("the statement is an application: {other}"),
    }
}

/// Every natural literal `from` becomes `to`, everywhere in `t`.
pub fn map_literal(t: &Term, from: u64, to: u64) -> Term {
    fn go(t: &Term, from: &BigInt, to: u64) -> Term {
        let b = |t: &Term| Box::new(go(t, from, to));
        match t {
            Term::Lit(Literal::Nat(n)) if n == from => {
                Term::Lit(Literal::Nat(BigInt::from_u64(to)))
            }
            Term::App(f, a) => Term::App(b(f), b(a)),
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: b(param_type),
                body_type: b(body_type),
            },
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: b(param_type),
                body: b(body),
            },
            Term::Let {
                name,
                ty,
                value,
                body,
            } => Term::Let {
                name: name.clone(),
                ty: b(ty),
                value: b(value),
                body: b(body),
            },
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: b(discriminant),
                motive: b(motive),
                cases: cases.iter().map(|c| go(c, from, to)).collect(),
            },
            Term::Fix { name, body } => Term::Fix {
                name: name.clone(),
                body: b(body),
            },
            Term::MutualFix { defs, index } => Term::MutualFix {
                defs: defs
                    .iter()
                    .map(|(n, d)| (n.clone(), go(d, from, to)))
                    .collect(),
                index: *index,
            },
            other => other.clone(),
        }
    }
    go(t, &BigInt::from_u64(from), to)
}

/// How many natural literals equal to `value` occur in `t` (a control must actually alter something).
pub fn literal_occurrences(t: &Term, value: u64) -> usize {
    let value = BigInt::from_u64(value);
    fn go(t: &Term, value: &BigInt) -> usize {
        match t {
            Term::Lit(Literal::Nat(n)) => usize::from(n == value),
            Term::App(f, a) => go(f, value) + go(a, value),
            Term::Pi {
                param_type,
                body_type,
                ..
            } => go(param_type, value) + go(body_type, value),
            Term::Lambda {
                param_type, body, ..
            } => go(param_type, value) + go(body, value),
            Term::Let {
                ty, value: v, body, ..
            } => go(ty, value) + go(v, value) + go(body, value),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                go(discriminant, value)
                    + go(motive, value)
                    + cases.iter().map(|c| go(c, value)).sum::<usize>()
            }
            Term::Fix { body, .. } => go(body, value),
            Term::MutualFix { defs, .. } => defs.iter().map(|(_, d)| go(d, value)).sum(),
            _ => 0,
        }
    }
    go(t, &value)
}
