//! The proof crate's axiom footprint and the Lean importer's declaration audit answer one question
//! through two doors: what does this proof rest on, by Lean 4.14's `#print axioms` rule? An axiom
//! is a leaf; definitions and theorems are walked through; Lean's quotient constants `Quot`,
//! `Quot.mk`, `Quot.lift` and `Quot.ind` (`ConstantInfo.quotInfo`) are builtins, never axioms, and
//! `Quot.sound` is an axiom. This file holds the two doors to one answer, and both to Lean's own.
//!
//! * **Lean's word.** The bridge ledger `crates/logicaffeine_tests/tests/fixtures/lean_conformance/bridge/T06-008.json` records what the
//!   pinned Lean 4.14.0 printed for `#print axioms` on every declaration of case T06-008.
//! * **The importer's word.** `interface::lean::import_audited` on that case's pinned export,
//!   verified byte for byte against the SHA-256 its provenance records.
//! * **The proof crate's word.** `VerifiedProof::axiom_footprint` of the same theorem, proved,
//!   certified and kernel-checked through `prove_certify_check`.
//!
//! LOGOS spells Lean's three axioms `propext`, `Classical_choice` and `Quot_sound`. The table that
//! translates them is held to the kernel's own record of the correspondence (`foreign_axiom_note`),
//! so the spelling cannot drift from what the kernel says it registers.

use logicaffeine_kernel::interface::lean::{self as lean, intake};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::quotient::{self, Kind};
use logicaffeine_kernel::{axiom_dependencies, foreign_axiom_note, Context, Term, Universe};
use logicaffeine_proof::verify::{prove_certify_check, AxiomFootprint, VerifiedProof};
use logicaffeine_proof::{ProofExpr, ProofTerm};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// LOGOS spelling → Lean spelling, for the axioms Lean's kernel trusts.
const LEAN_SPELLING: [(&str, &str); 3] = [
    ("Classical_choice", "Classical.choice"),
    ("Quot_sound", "Quot.sound"),
    ("propext", "propext"),
];

/// Lean's quotient constants, in LOGOS spellings.
const LEAN_QUOT_FORMERS: [&str; 4] = ["Quot", "Quot_mk", "Quot_lift", "Quot_ind"];

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}

fn pred(name: &str) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args: vec![ProofTerm::Constant("X".to_string())],
        world: None,
    }
}

fn not(p: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(p))
}

fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
}

fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}

fn footprint_of(ctx: &Context, term: Term) -> AxiomFootprint {
    VerifiedProof {
        derivation: None,
        proof_term: Some(term),
        kernel_ctx: ctx.clone(),
        verified: true,
        verification_error: None,
    }
    .axiom_footprint()
}

/// The footprint's trusted base in Lean spellings. A name the table does not translate keeps its
/// LOGOS spelling, so anything beyond Lean's three shows up by name.
fn trusted_in_lean_spelling(fp: &AxiomFootprint) -> BTreeSet<String> {
    fp.trusted
        .iter()
        .map(|ours| {
            LEAN_SPELLING
                .iter()
                .find(|(logos, _)| logos == ours)
                .map(|(_, lean)| (*lean).to_string())
                .unwrap_or_else(|| ours.clone())
        })
        .collect()
}

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("the import thread")
}

/// The spelling table is the kernel's own record, not a guess: every Lean spelling that differs
/// from ours has a `foreign_axiom_note` naming the LOGOS spelling, and every LOGOS spelling is an
/// assumption a freshly registered prelude declares.
#[test]
fn the_spelling_table_is_the_kernels_own_record() {
    let mut prelude = Context::new();
    StandardLibrary::register(&mut prelude);
    let declared: BTreeSet<String> = prelude
        .iter_declarations()
        .map(|(name, _)| name.to_string())
        .collect();
    for (ours, theirs) in LEAN_SPELLING {
        assert!(
            declared.contains(ours),
            "`{ours}` translates Lean's `{theirs}`, so it must be an ASSUMPTION of the prelude"
        );
        if ours != theirs {
            let note = foreign_axiom_note(theirs)
                .unwrap_or_else(|| panic!("the kernel records no LOGOS spelling for `{theirs}`"));
            assert!(
                note.contains(&format!("`{ours}`")),
                "the kernel's note for `{theirs}` must name `{ours}`: {note}"
            );
        }
    }
}

/// ★ Three words, one answer. For each theorem of T06-008 that the proof crate can state, Lean's
/// recorded `#print axioms`, the importer's audit of the pinned export and the proof crate's
/// footprint of its own certified proof name the same axioms.
///
/// The cases are chosen so the comparison is about the RULE and not about two different proofs:
/// `em`, `dne` and `byContra` are not intuitionistically provable, so every proof of them rests
/// on the classical base; `dni` is intuitionistic in both systems.
#[test]
fn the_footprint_the_importer_and_lean_name_the_same_axioms() {
    let cases: Vec<(&str, Vec<ProofExpr>, ProofExpr)> = vec![
        ("T06_008.em", vec![], or(pred("p"), not(pred("p")))),
        ("T06_008.dne", vec![not(not(pred("p")))], pred("p")),
        (
            "T06_008.byContra",
            vec![implies(not(pred("p")), pred("q")), not(pred("q"))],
            pred("p"),
        ),
        ("T06_008.dni", vec![pred("p")], not(not(pred("p")))),
    ];

    let ledger: Value =
        serde_json::from_str(
            &std::fs::read_to_string(repository().join(
                "crates/logicaffeine_tests/tests/fixtures/lean_conformance/bridge/T06-008.json",
            ))
            .expect("the bridge ledger"),
        )
        .expect("the ledger parses");
    assert_eq!(
        ledger["lean"], "4.14.0",
        "Lean's word is the pinned toolchain's"
    );
    let lean_axioms = |declaration: &str| -> BTreeSet<String> {
        let row = ledger["declarations"]
            .as_array()
            .expect("declarations")
            .iter()
            .find(|row| row["lean"] == declaration)
            .unwrap_or_else(|| panic!("the ledger records `{declaration}`"));
        row["lean_axioms"]
            .as_array()
            .expect("lean_axioms")
            .iter()
            .map(|a| a.as_str().expect("a Lean name").to_string())
            .collect()
    };

    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lean_conformance/cases_lean414/harvest");
    let export = std::fs::read_to_string(fixtures.join("T06-008.ndjson")).expect("the export");
    let provenance: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures.join("T06-008.provenance.json")).expect("provenance"),
    )
    .expect("provenance parses");
    assert_eq!(
        lean::package::sha256_hex(export.as_bytes()),
        provenance["export_sha256"],
        "the export must be the pinned one, byte for byte"
    );
    assert_eq!(provenance["toolchain"], "leanprover/lean4:v4.14.0");
    let report = on_big_stack(move || lean::import_audited(&export).map(|(_, report)| report))
        .unwrap_or_else(|e| panic!("the pinned export imports: {e:?}"));
    let audited = |declaration: &str| -> BTreeSet<String> {
        let kernel = intake::kernel_name(declaration).expect("a Lean name");
        report
            .declarations
            .iter()
            .find(|d| d.name == kernel)
            .unwrap_or_else(|| panic!("the importer audits `{declaration}`"))
            .axioms
            .iter()
            .map(|a| intake::lean_name(a))
            .collect()
    };

    let mut disagreements = Vec::new();
    for (declaration, premises, goal) in &cases {
        let lean = lean_axioms(declaration);
        let importer = audited(declaration);
        let r = prove_certify_check(premises, goal);
        assert!(
            r.verified,
            "{declaration}: the proof crate certifies it: {:?}",
            r.verification_error
        );
        let ours = trusted_in_lean_spelling(&r.axiom_footprint());
        if importer != lean {
            disagreements.push(format!(
                "{declaration}: importer {importer:?}, Lean {lean:?}"
            ));
        }
        if ours != lean {
            disagreements.push(format!(
                "{declaration}: proof crate {ours:?}, Lean {lean:?}"
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "the doors disagree with Lean's `#print axioms`:\n  {}",
        disagreements.join("\n  ")
    );
}

/// ONE rule for Lean's quotient constants, whichever door registered them. The prelude declares
/// `Quot`, `Quot_mk`, `Quot_lift` and `Quot_ind` itself; the importer installs Lean's through
/// `quotient::install` as `kernel.quot.*`. `#print axioms` names neither, names `Quot.sound` alone
/// for itself, and names `Quot.sound` alone for `funext`, which Lean proves from it.
#[test]
fn both_doors_to_leans_quotient_constants_report_them_as_lean_does() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let u = Universe::Var("u".into());
    let pi = |p: &str, t: Term, b: Term| Term::Pi {
        param: p.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    };
    let v = |n: &str| Term::Var(n.into());
    ctx.add_universe_poly_inductive(
        "LeanEq",
        vec!["u".into()],
        pi(
            "A",
            Term::Sort(u.clone()),
            pi("a", v("A"), pi("b", v("A"), Term::Sort(Universe::Prop))),
        ),
        2,
    );
    let refl_result = [v("A"), v("a"), v("a")].into_iter().fold(
        Term::Const {
            name: "LeanEq".into(),
            levels: vec![u.clone()],
        },
        |f, x| Term::App(Box::new(f), Box::new(x)),
    );
    ctx.add_constructor(
        "LeanEq_refl",
        "LeanEq",
        pi("A", Term::Sort(u), pi("a", v("A"), refl_result)),
    );
    for kind in [Kind::Type, Kind::Constructor, Kind::Lift, Kind::Induction] {
        let params: Vec<String> = if kind == Kind::Lift {
            vec!["u".into(), "v".into()]
        } else {
            vec!["u".into()]
        };
        quotient::install(&mut ctx, kind, params, "LeanEq")
            .unwrap_or_else(|e| panic!("the importer's door installs {kind:?}: {e:?}"));
    }
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );

    let mut named = Vec::new();
    for former in LEAN_QUOT_FORMERS {
        let fp = footprint_of(&ctx, g(former));
        if !(fp.trusted.is_empty() && fp.introduced.is_empty()) {
            named.push(format!("prelude `{former}` -> {fp:?}"));
        }
    }
    for kind in [Kind::Type, Kind::Constructor, Kind::Lift, Kind::Induction] {
        let levels = if kind == Kind::Lift {
            vec![Universe::Type(0), Universe::Type(0)]
        } else {
            vec![Universe::Type(0)]
        };
        let fp = footprint_of(
            &ctx,
            Term::Const {
                name: kind.name().into(),
                levels,
            },
        );
        if !(fp.trusted.is_empty() && fp.introduced.is_empty()) {
            named.push(format!("importer `{}` -> {fp:?}", kind.name()));
        }
    }
    assert!(
        named.is_empty(),
        "Lean's quotient constants are builtins and `#print axioms` names none of them, through \
         either door:\n  {}",
        named.join("\n  ")
    );

    let sound = footprint_of(&ctx, g("Quot_sound"));
    assert_eq!(
        sound.trusted,
        ["Quot_sound"],
        "an axiom rests on itself: {sound:?}"
    );
    let funext = footprint_of(&ctx, g("funext"));
    assert_eq!(
        funext.trusted,
        ["Quot_sound"],
        "Lean's `#print axioms funext` is `[Quot.sound]`: {funext:?}"
    );
}

/// The builtin classification is an IDENTITY, never a SPELLING. A context without the prelude
/// that declares its own `Quot_ind` or `kernel.quot.type` has assumed something, and a proof that
/// cites it rests on it — a report keyed on the name would erase exactly that assumption.
#[test]
fn a_same_spelled_assumption_outside_the_quotient_doors_is_still_an_assumption() {
    for name in ["Quot_ind", "Quot_mk", Kind::Type.name()] {
        let mut ctx = Context::new();
        ctx.add_declaration(name, Term::Sort(Universe::Prop));
        let fp = footprint_of(&ctx, g(name));
        assert_eq!(
            fp.introduced,
            [name],
            "`{name}` declared outside the prelude and the importer's quotient door is an \
             ASSUMPTION of this context, and the proof that cites it rests on it: {fp:?}"
        );
        assert!(
            fp.trusted.is_empty(),
            "…and it is not the prelude's: {fp:?}"
        );
    }
}

/// The footprint is a READ of the kernel's closure, adding and dropping nothing: one closure,
/// two doors. A proof-crate-local rule — a name filter here, a special case there — would make
/// the proof crate's `#print axioms` and the REPL's disagree about the same term.
#[test]
fn the_footprint_is_the_kernels_closure_of_the_certified_term() {
    let proofs = [
        prove_certify_check(&[], &or(pred("p"), not(pred("p")))),
        prove_certify_check(&[pred("p")], &not(not(pred("p")))),
        prove_certify_check(&[pred("p")], &pred("p")),
    ];
    for r in &proofs {
        assert!(r.verified, "precondition: {:?}", r.verification_error);
        let term = r
            .proof_term
            .as_ref()
            .expect("verified proofs carry their term");
        let usage = axiom_dependencies(&r.kernel_ctx, term);
        let kernel: BTreeSet<String> = usage.axioms.into_iter().chain(usage.unresolved).collect();
        let fp = r.axiom_footprint();
        let reported: BTreeSet<String> = fp.trusted.into_iter().chain(fp.introduced).collect();
        assert_eq!(
            reported, kernel,
            "the footprint of `{term}` must be the kernel's closure"
        );
    }
}
