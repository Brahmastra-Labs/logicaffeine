//! Structure η: one checked registration, and η at a symbolic universe level.
//!
//! `Context::register_structure_checked` is the only door that grants structure η. The vernacular
//! `Structure` command (`Repl`) and the environment certificate's `structure` entry both call it, and
//! the Lean reader registers the structures it recognises through it. Every refusal condition is
//! exercised here through the kernel door, the vernacular door and the certificate door, and must
//! be refused identically by all three; `structure_eta_kernel_door` adds the sealed container and
//! the short constructor, and `structure_eta_lean_door` the Lean reader's block-wide `isRec`.
//!
//! Original witness (`fixtures/lean_semantics/LK_PolyStructEta`), accepted by Lean 4.14.0's
//! kernel: `ptEta : ∀ (A : Type u) (p : Pt A), p = Pt.mk p.x p.y := rfl` at a SYMBOLIC level `u`,
//! its mirror `ptEtaSymm`, and the monomorphic `ptEtaNat`. The negative control is the kernel's own
//! refusal, recorded in the compile log: `rfl` against the field-SWAPPED reconstruction
//! (`ptEtaSwapped`) is REJECTED.
use std::collections::BTreeMap;

use logicaffeine_kernel::certificate::{
    CaptureRequest, ConversionDiscipline, EnvironmentBase, EnvironmentCertificate,
    EnvironmentEntry, EnvironmentError, EnvironmentErrorCode, EnvironmentLimits,
};
use logicaffeine_kernel::interface::lean::{self as adapter, emit};
use logicaffeine_kernel::interface::{InterfaceError, Repl};
use logicaffeine_kernel::{
    defeq_for_test, double_check, recheck, Context, DoubleCheck, KernelError, StructInfo,
    StructureRefusal, Term, Universe,
};
use serde_json::Value;

const WITNESS: &str = include_str!("fixtures/lean_semantics/LK_PolyStructEta.ndjson");
const WITNESS_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LK_PolyStructEta.provenance.json");
const WITNESS_SOURCE: &str = include_str!("fixtures/lean_semantics/LK_PolyStructEta.lean");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}

/// The polymorphic point structure, its two projections, and a universe-polymorphic equality
/// (Lean's `Eq.{u} : {α : Sort u} → α → α → Prop`, explicit in its carrier), in the vernacular.
const PT: [&str; 4] = [
    "Inductive PEq.{u} (A : Sort u) (a : A) : A -> Prop := prefl : PEq.{u} A a a.",
    "Inductive Pt.{u} (A : Sort (u+1)) : Sort (u+1) := ptmk : A -> A -> Pt.{u} A.",
    "Definition px.{u} : (A : Sort (u+1)) -> Pt.{u} A -> A := fun (A : Sort (u+1)) (p : Pt.{u} A) => match p return (fun (q : Pt.{u} A) => A) with fun (a : A) (b : A) => a end.",
    "Definition py.{u} : (A : Sort (u+1)) -> Pt.{u} A -> A := fun (A : Sort (u+1)) (p : Pt.{u} A) => match p return (fun (q : Pt.{u} A) => A) with fun (a : A) (b : A) => b end.",
];

fn pt_session(with_eta: bool) -> Repl {
    let mut r = Repl::new();
    for line in PT {
        r.execute(line).unwrap_or_else(|e| panic!("{line}: {e:?}"));
    }
    if with_eta {
        r.execute("Structure Pt := px py.")
            .expect("the projections compute their fields");
    }
    r
}

const ETA_POLY: &str = "Theorem pt_eta.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> PEq.{u+1} (Pt.{u} A) p (ptmk.{u} A (px.{u} A p) (py.{u} A p)) := fun (A : Sort (u+1)) (p : Pt.{u} A) => prefl.{u+1} (Pt.{u} A) p.";
const ETA_POLY_SYMM: &str = "Theorem pt_eta_symm.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> PEq.{u+1} (Pt.{u} A) (ptmk.{u} A (px.{u} A p) (py.{u} A p)) p := fun (A : Sort (u+1)) (p : Pt.{u} A) => prefl.{u+1} (Pt.{u} A) (ptmk.{u} A (px.{u} A p) (py.{u} A p)).";
const ETA_POLY_SWAPPED: &str = "Theorem pt_eta_swapped.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> PEq.{u+1} (Pt.{u} A) p (ptmk.{u} A (py.{u} A p) (px.{u} A p)) := fun (A : Sort (u+1)) (p : Pt.{u} A) => prefl.{u+1} (Pt.{u} A) p.";
const ETA_NAT: &str = "Theorem pt_eta_nat : (p : Pt.{0} Nat) -> PEq.{1} (Pt.{0} Nat) p (ptmk.{0} Nat (px.{0} Nat p) (py.{0} Nat p)) := fun (p : Pt.{0} Nat) => prefl.{1} (Pt.{0} Nat) p.";

// -------------------------------------------------------------------------------------------------
// η at a symbolic universe level.
// -------------------------------------------------------------------------------------------------

#[test]
fn the_witness_is_the_pinned_lean_4_14_export_and_lean_refuses_the_swapped_control() {
    let p: Value = serde_json::from_str(WITNESS_PROVENANCE).unwrap();
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["records"], WITNESS.lines().count());
    assert_eq!(p["export_header"]["meta"]["lean"]["version"], "4.14.0");
    let header: Value = serde_json::from_str(WITNESS.lines().next().unwrap()).unwrap();
    assert_eq!(header["meta"]["lean"]["version"], "4.14.0");
    assert_eq!(
        p["declarations"],
        serde_json::json!([
            "LK_PSE.Pt",
            "LK_PSE.ptEta",
            "LK_PSE.ptEtaSymm",
            "LK_PSE.ptEtaNat"
        ])
    );
    let log = p["compile_log"].as_str().unwrap();
    assert!(log.contains("ptEtaSwapped: kernel REJECTED"), "{log}");
    assert!(!log.contains("kernel ACCEPTED"), "{log}");
    assert!(
        WITNESS_SOURCE.contains("theorem ptEta (A : Type u) (p : Pt A) : p = Pt.mk p.x p.y := rfl")
    );
}

#[test]
fn the_reader_accepts_lean_structure_eta_at_a_symbolic_level() {
    let (ctx, report) =
        adapter::import_audited(WITNESS).expect("Lean's kernel accepts every declaration");
    for theorem in ["LK_PSE.ptEta", "LK_PSE.ptEtaSymm", "LK_PSE.ptEtaNat"] {
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(theorem))
            .unwrap_or_else(|| panic!("{theorem} audited"));
        assert_eq!(audit.kind, "thm", "{theorem}");
        assert!(audit.axioms.is_empty(), "{theorem}: {:?}", audit.axioms);
    }
    let (levels, _) = ctx
        .poly_declaration(&n("LK_PSE.ptEta"))
        .expect("ptEta is universe-polymorphic");
    assert_eq!(levels.len(), 1, "ptEta keeps its one symbolic level");
    let info = ctx
        .struct_info(&n("LK_PSE.Pt"))
        .expect("Pt is registered as a structure");
    assert_eq!(
        (info.mk.as_str(), info.num_params, info.projections.len()),
        (n("LK_PSE.Pt.mk").as_str(), 1, 2)
    );
}

#[test]
fn the_emitted_polymorphic_eta_theorem_replays_in_a_fresh_session() {
    for theorem in ["LK_PSE.ptEta", "LK_PSE.ptEtaSymm", "LK_PSE.ptEtaNat"] {
        let target = n(theorem);
        let intake = adapter::intake::read(WITNESS, &adapter::intake::STAGE_A)
            .unwrap_or_else(|e| panic!("intake: {e:?}"));
        let closure = intake
            .closure(&target)
            .unwrap_or_else(|e| panic!("closure of {theorem}: {e:?}"));
        let (imported, _) = adapter::import_retaining_closure(WITNESS, &target, &closure)
            .unwrap_or_else(|e| panic!("import retaining {theorem}: {e:?}"));
        let doc = emit::emit_document(&imported, &target)
            .unwrap_or_else(|e| panic!("emission of {theorem}: {e:?}"));
        emit::verify(&doc).unwrap_or_else(|e| {
            panic!(
                "{theorem}: the emitted document replays: {e}\n{}",
                doc.source
            )
        });
    }
}

#[test]
fn structure_eta_holds_at_a_symbolic_universe_level_in_the_vernacular() {
    let mut r = pt_session(true);
    r.execute(ETA_NAT).expect("η at the monomorphic instance");
    r.execute(ETA_POLY)
        .unwrap_or_else(|e| panic!("η at a symbolic level, as Lean's kernel accepts it: {e:?}"));
    r.execute(ETA_POLY_SYMM)
        .unwrap_or_else(|e| panic!("the mirrored η at a symbolic level: {e:?}"));
}

#[test]
fn symbolic_level_eta_is_refused_without_the_structure_and_for_the_swapped_reconstruction() {
    let mut plain = pt_session(false);
    assert!(
        plain.execute(ETA_POLY).is_err(),
        "without structure η a variable is not its reconstruction"
    );
    assert!(plain.execute(ETA_POLY_SYMM).is_err());
    assert!(plain.execute(ETA_NAT).is_err());
    let mut r = pt_session(true);
    let swapped = r.execute(ETA_POLY_SWAPPED);
    assert!(
        swapped.is_err(),
        "Lean's kernel refuses the swapped reconstruction, and so must we: {swapped:?}"
    );
    assert!(r.context().get_universe_poly("pt_eta_swapped").is_none());
}

// -------------------------------------------------------------------------------------------------
// The prelude's `Eq` at an explicit level: `refl.{ℓ} X x : Eq.{ℓ} X x x`.
//
// The observation this lane was handed — `refl.{u+1} (Pt.{u} A) p : Eq.{u+1} (Pt.{u} A) p (ptmk.{u}
// A (px.{u} A p) (py.{u} A p))` refused, while the `Pt.{0} Nat` statement checks — is not about η:
// the monomorphic statement that checked was spelled with a BARE `Eq`/`refl`. The prelude's `Eq`
// auto-binds its level (`Eq : Π(A : Sort u) …`), so `refl.{ℓ}`'s instantiated type mentions a bare
// `Eq` whose level the application rule solves from `X`, while the statement says `Eq.{ℓ}`. The
// two are the same constant at the same level — Lean's `@rfl.{ℓ} α a : @Eq.{ℓ} α a a` — and must
// convert; a DIFFERENT explicit level must not.
// -------------------------------------------------------------------------------------------------

const EXPLICIT_REFL_POLY: &str = "Theorem xr.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> Eq.{u+1} (Pt.{u} A) p p := fun (A : Sort (u+1)) (p : Pt.{u} A) => refl.{u+1} (Pt.{u} A) p.";
const EXPLICIT_REFL_NAT: &str = "Theorem xn : (p : Pt.{0} Nat) -> Eq.{1} (Pt.{0} Nat) p p := fun (p : Pt.{0} Nat) => refl.{1} (Pt.{0} Nat) p.";
const EXPLICIT_REFL_PROP: &str = "Theorem xt : Eq.{0} True I I := refl.{0} True I.";
const EXPLICIT_ETA_POLY: &str = "Theorem xe.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> Eq.{u+1} (Pt.{u} A) p (ptmk.{u} A (px.{u} A p) (py.{u} A p)) := fun (A : Sort (u+1)) (p : Pt.{u} A) => refl.{u+1} (Pt.{u} A) p.";
const EXPLICIT_ETA_NAT: &str = "Theorem xen : (p : Pt.{0} Nat) -> Eq.{1} (Pt.{0} Nat) p (ptmk.{0} Nat (px.{0} Nat p) (py.{0} Nat p)) := fun (p : Pt.{0} Nat) => refl.{1} (Pt.{0} Nat) p.";
const EXPLICIT_ETA_BARE_STATEMENT: &str = "Theorem xb.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> Eq (Pt.{u} A) p (ptmk.{u} A (px.{u} A p) (py.{u} A p)) := fun (A : Sort (u+1)) (p : Pt.{u} A) => refl.{u+1} (Pt.{u} A) p.";
const EXPLICIT_ETA_BARE_PROOF: &str = "Theorem xp.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> Eq.{u+1} (Pt.{u} A) p (ptmk.{u} A (px.{u} A p) (py.{u} A p)) := fun (A : Sort (u+1)) (p : Pt.{u} A) => refl (Pt.{u} A) p.";

#[test]
fn an_explicit_level_on_the_statement_converts_with_the_instantiated_proof() {
    let mut r = pt_session(true);
    for theorem in [EXPLICIT_REFL_PROP, EXPLICIT_REFL_NAT, EXPLICIT_REFL_POLY] {
        r.execute(theorem)
            .unwrap_or_else(|e| panic!("reflexivity at an explicit level: {theorem}: {e:?}"));
    }
}

#[test]
fn structure_eta_at_a_symbolic_level_checks_as_it_was_first_spelled() {
    let mut r = pt_session(true);
    for theorem in [
        EXPLICIT_ETA_NAT,
        EXPLICIT_ETA_POLY,
        EXPLICIT_ETA_BARE_STATEMENT,
        EXPLICIT_ETA_BARE_PROOF,
    ] {
        r.execute(theorem)
            .unwrap_or_else(|e| panic!("{theorem}: {e:?}"));
    }
    // The independent re-checker decides the same conversion: it checks the proof against the
    // explicitly-levelled statement as the domain of an ascription, `(λ h : T. h) proof`.
    for name in ["xen", "xe", "xb", "xp"] {
        let (ty, proof) = published(r.context(), name);
        let ascription = Term::App(
            Box::new(Term::Lambda {
                param: "h".into(),
                param_type: Box::new(ty.clone()),
                body: Box::new(Term::Var("h".into())),
            }),
            Box::new(proof),
        );
        recheck(r.context(), &ascription).unwrap_or_else(|e| {
            panic!("{name}: the re-checker converts the proof to its statement: {e:?}")
        });
        assert_eq!(
            double_check(r.context(), &ascription),
            DoubleCheck::Agreed,
            "{name}"
        );
    }
}

/// A checked theorem's statement and proof, as published (Lean publishes a theorem unfoldably).
fn published(ctx: &Context, name: &str) -> (Term, Term) {
    if let Some((_, ty, proof)) = ctx.get_universe_poly(name) {
        return (ty.clone(), proof.clone());
    }
    let ty = ctx
        .get_global(name)
        .unwrap_or_else(|| panic!("{name} declared"))
        .clone();
    let proof = ctx
        .get_definition_body(name)
        .unwrap_or_else(|| panic!("{name} carries its proof"))
        .clone();
    (ty, proof)
}

#[test]
fn a_different_explicit_level_or_a_false_reconstruction_is_still_refused() {
    let mut r = pt_session(true);
    // `Eq.{u+2} (Pt.{u} A)` is well formed by cumulativity, but it is not the constant `refl.{u+1}`
    // proves an instance of (Lean has no cumulativity: there it is ill typed).
    for theorem in [
        "Theorem wl.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> Eq.{u+2} (Pt.{u} A) p p := fun (A : Sort (u+1)) (p : Pt.{u} A) => refl.{u+1} (Pt.{u} A) p.",
        "Theorem wn : (p : Pt.{0} Nat) -> Eq.{2} (Pt.{0} Nat) p p := fun (p : Pt.{0} Nat) => refl.{1} (Pt.{0} Nat) p.",
        "Theorem wb.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> Eq.{u+2} (Pt.{u} A) p p := fun (A : Sort (u+1)) (p : Pt.{u} A) => refl (Pt.{u} A) p.",
        "Theorem ws.{u} : (A : Sort (u+1)) -> (p : Pt.{u} A) -> Eq.{u+1} (Pt.{u} A) p (ptmk.{u} A (py.{u} A p) (px.{u} A p)) := fun (A : Sort (u+1)) (p : Pt.{u} A) => refl.{u+1} (Pt.{u} A) p.",
    ] {
        assert!(r.execute(theorem).is_err(), "{theorem}");
    }
    // Without η the explicitly-levelled reconstruction is refused too.
    let mut plain = pt_session(false);
    assert!(plain.execute(EXPLICIT_ETA_POLY).is_err());
    assert!(plain.execute(EXPLICIT_ETA_NAT).is_err());
}

#[test]
fn a_bare_reference_converts_only_with_the_instantiation_its_arguments_determine() {
    let mut ctx = Repl::new().context().clone();
    // An auto-bound assumption (as the prelude's own `Eq` is) whose level no argument determines.
    ctx.add_declaration(
        "Lift",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body_type: Box::new(Term::Sort(Universe::Var("u".into()))),
        },
    );
    let ctx = &ctx;
    let g = |n: &str| Term::Global(n.into());
    let at = |n: &str, levels: Vec<Universe>| Term::Const {
        name: n.into(),
        levels,
    };
    let app = |f: Term, xs: Vec<Term>| {
        xs.into_iter()
            .fold(f, |f, x| Term::App(Box::new(f), Box::new(x)))
    };
    let zero = || g("Zero");
    let eq_nat = |head: Term| app(head, vec![g("Nat"), zero(), zero()]);
    // `Nat : Type 0`, so the bare `Eq Nat` is `Eq.{1} Nat`, in either order.
    assert!(defeq_for_test(
        ctx,
        &eq_nat(g("Eq")),
        &eq_nat(at("Eq", vec![Universe::Type(0)]))
    ));
    assert!(defeq_for_test(
        ctx,
        &eq_nat(at("Eq", vec![Universe::Type(0)])),
        &eq_nat(g("Eq"))
    ));
    // Not at another level, although `Eq.{2} Nat` is well formed by cumulativity; not at `Prop`.
    assert!(!defeq_for_test(
        ctx,
        &eq_nat(g("Eq")),
        &eq_nat(at("Eq", vec![Universe::Type(1)]))
    ));
    assert!(!defeq_for_test(
        ctx,
        &eq_nat(g("Eq")),
        &eq_nat(at("Eq", vec![Universe::Prop]))
    ));
    // Not with different arguments.
    let one = app(g("Succ"), vec![zero()]);
    assert!(!defeq_for_test(
        ctx,
        &eq_nat(g("Eq")),
        &app(
            at("Eq", vec![Universe::Type(0)]),
            vec![g("Nat"), zero(), one]
        )
    ));
    // A level no argument determines stays unsolved, and nothing is concluded from it.
    assert!(!defeq_for_test(
        ctx,
        &app(g("Lift"), vec![zero()]),
        &app(at("Lift", vec![Universe::Type(0)]), vec![zero()])
    ));
    // An unapplied bare reference determines no level either.
    assert!(!defeq_for_test(
        ctx,
        &g("Eq"),
        &at("Eq", vec![Universe::Type(0)])
    ));
}

// -------------------------------------------------------------------------------------------------
// One door, three callers: every refusal condition, refused identically.
// -------------------------------------------------------------------------------------------------

/// A session holding the ingredients of one structure-η request, and a definition `target` whose
/// dependency closure contains all of them (so an environment certificate captures them).
struct Scenario {
    setup: &'static [&'static str],
    target: &'static str,
    inductive: &'static str,
    projections: &'static [&'static str],
}

impl Scenario {
    fn session(&self) -> Repl {
        let mut r = Repl::new();
        for line in self.setup {
            r.execute(line).unwrap_or_else(|e| panic!("{line}: {e:?}"));
        }
        r
    }

    fn projections(&self) -> Vec<String> {
        self.projections.iter().map(|p| p.to_string()).collect()
    }

    /// The kernel door itself, on a copy of the session's context.
    fn kernel_door(&self) -> (Context, Result<(), StructureRefusal>) {
        let mut ctx = self.session().context().clone();
        let verdict = ctx.register_structure_checked(self.inductive, &self.projections());
        (ctx, verdict)
    }

    /// The vernacular door: `Structure <inductive> := <projections>.`
    fn vernacular_door(&self) -> (Repl, Result<String, InterfaceError>) {
        let mut r = self.session();
        let command = format!(
            "Structure {} := {}.",
            self.inductive,
            self.projections.join(" ")
        );
        let verdict = r.execute(&command);
        (r, verdict)
    }

    /// The certificate door: capture `target` from the session, insert the `structure` entry right
    /// before it, and rebuild from nothing.
    fn certificate_door(&self) -> Result<Context, EnvironmentError> {
        let session = self.session();
        let proofs = BTreeMap::new();
        let request = CaptureRequest {
            context: session.context(),
            base: EnvironmentBase::StandardLibrary,
            conversion: ConversionDiscipline::Exact,
            target: self.target,
            opaque_proofs: &proofs,
        };
        let mut certificate = EnvironmentCertificate::capture(&request)
            .unwrap_or_else(|e| panic!("capture {}: {e}", self.target));
        let at = certificate
            .entries
            .iter()
            .position(|e| e.names().contains(&self.target))
            .unwrap_or_else(|| panic!("{} is captured", self.target));
        certificate.entries.insert(
            at,
            EnvironmentEntry::Structure {
                inductive: self.inductive.to_string(),
                projections: self.projections(),
            },
        );
        certificate
            .rebuild(&EnvironmentLimits::default())
            .map(|rebuilt| rebuilt.context)
    }

    /// All three doors refuse, with the same refusal: the kernel's typed one, spelled identically
    /// by the vernacular and classified by the certificate. Nothing is registered anywhere.
    fn refused_identically(&self, expected: StructureRefusal) {
        let already = self
            .session()
            .context()
            .struct_info(self.inductive)
            .map(|i| i.projections.clone());
        let (ctx, kernel) = self.kernel_door();
        assert_eq!(
            kernel,
            Err(expected.clone()),
            "kernel door, {}",
            self.inductive
        );
        assert_eq!(
            ctx.struct_info(self.inductive)
                .map(|i| i.projections.clone()),
            already,
            "a refusal registers nothing"
        );

        let (repl, vernacular) = self.vernacular_door();
        match vernacular {
            Err(InterfaceError::Kernel(KernelError::InvalidDeclaration(message))) => {
                assert_eq!(
                    message,
                    expected.to_string(),
                    "vernacular door, {}",
                    self.inductive
                )
            }
            other => panic!(
                "vernacular door, {}: expected the kernel's refusal, got {other:?}",
                self.inductive
            ),
        }
        assert_eq!(
            repl.context()
                .struct_info(self.inductive)
                .map(|i| i.projections.clone()),
            already
        );

        let code = match expected {
            StructureRefusal::NotAnInductive { .. } => EnvironmentErrorCode::UnresolvedObligation,
            StructureRefusal::AlreadyAStructure { .. } => {
                EnvironmentErrorCode::TrustedNameReplacement
            }
            _ => EnvironmentErrorCode::IllTyped,
        };
        match self.certificate_door() {
            Ok(_) => panic!("certificate door, {}: accepted", self.inductive),
            Err(e) => {
                assert_eq!(
                    e.code, code,
                    "certificate door, {}: {}",
                    self.inductive, e.message
                );
                assert_eq!(
                    e.message,
                    expected.to_string(),
                    "certificate door, {}",
                    self.inductive
                );
            }
        }
    }
}

const PT_SETUP: &[&str] = &[
    PT[1],
    PT[2],
    PT[3],
    "Definition pt_uses.{u} : (A : Sort (u+1)) -> Pt.{u} A -> A := fun (A : Sort (u+1)) (p : Pt.{u} A) => px.{u} A (ptmk.{u} A (py.{u} A p) (px.{u} A p)).",
];

fn pt(projections: &'static [&'static str]) -> Scenario {
    Scenario {
        setup: PT_SETUP,
        target: "pt_uses",
        inductive: "Pt",
        projections,
    }
}

fn same_struct_info(a: &StructInfo, b: &StructInfo) -> bool {
    (&a.mk, a.num_params, &a.projections, &a.params, &a.fields)
        == (&b.mk, b.num_params, &b.projections, &b.params, &b.fields)
}

#[test]
fn the_real_projections_are_granted_identically_through_every_door() {
    let scenario = pt(&["px", "py"]);
    let (ctx, kernel) = scenario.kernel_door();
    kernel.expect("px and py compute Pt's fields");
    let granted = ctx.struct_info("Pt").expect("registered").clone();
    assert_eq!((granted.mk.as_str(), granted.num_params), ("ptmk", 1));
    assert_eq!(
        granted.projections,
        vec!["px".to_string(), "py".to_string()]
    );
    assert_eq!(granted.fields.len(), 2);
    let (repl, vernacular) = scenario.vernacular_door();
    assert_eq!(vernacular.expect("the vernacular grants it"), "");
    assert!(same_struct_info(
        repl.context().struct_info("Pt").unwrap(),
        &granted
    ));
    let rebuilt = scenario
        .certificate_door()
        .unwrap_or_else(|e| panic!("the certificate grants it: {e}"));
    assert!(same_struct_info(
        rebuilt.struct_info("Pt").unwrap(),
        &granted
    ));
}

#[test]
fn an_undeclared_inductive_is_refused_identically() {
    Scenario {
        setup: PT_SETUP,
        target: "pt_uses",
        inductive: "px",
        projections: &["px", "py"],
    }
    .refused_identically(StructureRefusal::NotAnInductive {
        inductive: "px".into(),
    });
}

#[test]
fn a_second_registration_is_refused_identically() {
    const SETUP: &[&str] = &[PT[1], PT[2], PT[3], "Structure Pt := px py.", PT_SETUP[3]];
    Scenario {
        setup: SETUP,
        target: "pt_uses",
        inductive: "Pt",
        projections: &["px", "py"],
    }
    .refused_identically(StructureRefusal::AlreadyAStructure {
        inductive: "Pt".into(),
    });
}

#[test]
fn two_constructors_are_refused_identically() {
    const SETUP: &[&str] = &[
        "Inductive Two : Type := t1 : Two | t2 : Two.",
        "Definition two_uses : Two := t1.",
    ];
    Scenario {
        setup: SETUP,
        target: "two_uses",
        inductive: "Two",
        projections: &[],
    }
    .refused_identically(StructureRefusal::NotStructureShaped {
        inductive: "Two".into(),
        constructors: 2,
        indices: 0,
    });
}

#[test]
fn an_index_is_refused_identically() {
    const SETUP: &[&str] = &[
        "Inductive Only : Nat -> Type := onz : Only Zero.",
        "Definition only_uses : Only Zero := onz.",
    ];
    Scenario {
        setup: SETUP,
        target: "only_uses",
        inductive: "Only",
        projections: &[],
    }
    .refused_identically(StructureRefusal::NotStructureShaped {
        inductive: "Only".into(),
        constructors: 1,
        indices: 1,
    });
}

#[test]
fn a_directly_recursive_inductive_is_refused_identically() {
    const SETUP: &[&str] = &[
        "Inductive Str : Type := smk : Nat -> Str -> Str.",
        "Definition shd : Str -> Nat := fun (s : Str) => match s return (fun (q : Str) => Nat) with fun (n : Nat) (t : Str) => n end.",
        "Definition stl : Str -> Str := fun (s : Str) => match s return (fun (q : Str) => Str) with fun (n : Nat) (t : Str) => t end.",
        "Definition str_uses : Str -> Nat := fun (s : Str) => shd (stl s).",
    ];
    Scenario {
        setup: SETUP,
        target: "str_uses",
        inductive: "Str",
        projections: &["shd", "stl"],
    }
    .refused_identically(StructureRefusal::Recursive {
        inductive: "Str".into(),
    });
}

#[test]
fn a_recursive_mutual_block_is_refused_for_every_member_identically() {
    // `MB`'s own field is `Nat`, but its block is recursive through `MA`: Lean's `isRec` is a
    // property of the block, so neither member is structure-like.
    const SETUP: &[&str] = &[
        "Inductive MA : Type := mamk : MB -> MA with MB : Type := mbmk : Nat -> MB.",
        "Definition maget : MA -> MB := fun (a : MA) => match a return (fun (q : MA) => MB) with fun (b : MB) => b end.",
        "Definition mbget : MB -> Nat := fun (b : MB) => match b return (fun (q : MB) => Nat) with fun (n : Nat) => n end.",
        "Definition mutual_uses : MA -> Nat := fun (a : MA) => mbget (maget a).",
    ];
    Scenario {
        setup: SETUP,
        target: "mutual_uses",
        inductive: "MA",
        projections: &["maget"],
    }
    .refused_identically(StructureRefusal::Recursive {
        inductive: "MA".into(),
    });
    Scenario {
        setup: SETUP,
        target: "mutual_uses",
        inductive: "MB",
        projections: &["mbget"],
    }
    .refused_identically(StructureRefusal::Recursive {
        inductive: "MB".into(),
    });
}

#[test]
fn a_non_recursive_mutual_block_member_is_still_a_structure() {
    const SETUP: &[&str] = &[
        "Inductive NA : Type := namk : Nat -> NA with NB : Type := nbmk : Nat -> NB.",
        "Definition naget : NA -> Nat := fun (a : NA) => match a return (fun (q : NA) => Nat) with fun (n : Nat) => n end.",
        "Definition na_uses : NA -> Nat := fun (a : NA) => naget a.",
    ];
    let scenario = Scenario {
        setup: SETUP,
        target: "na_uses",
        inductive: "NA",
        projections: &["naget"],
    };
    scenario
        .kernel_door()
        .1
        .expect("a block with no recursive argument is structure-like");
    scenario
        .vernacular_door()
        .1
        .expect("granted by the vernacular");
    let rebuilt = scenario
        .certificate_door()
        .unwrap_or_else(|e| panic!("granted by the certificate: {e}"));
    assert_eq!(
        rebuilt.struct_info("NA").map(|i| i.projections.clone()),
        Some(vec!["naget".to_string()])
    );
}

#[test]
fn too_few_and_too_many_projections_are_refused_identically() {
    pt(&["px"]).refused_identically(StructureRefusal::ProjectionCount {
        inductive: "Pt".into(),
        fields: 2,
        projections: 1,
    });
    pt(&["px", "py", "px"]).refused_identically(StructureRefusal::ProjectionCount {
        inductive: "Pt".into(),
        fields: 2,
        projections: 3,
    });
}

#[test]
fn a_projection_that_is_not_a_transparent_definition_is_refused_identically() {
    pt(&["px", "ptmk"]).refused_identically(StructureRefusal::NotADefinition {
        projection: "ptmk".into(),
    });
    const SETUP: &[&str] = &[
        PT[1],
        PT[2],
        "Axiom pyax.{u} : (A : Sort (u+1)) -> Pt.{u} A -> A.",
        "Definition pt_ax_uses.{u} : (A : Sort (u+1)) -> Pt.{u} A -> A := fun (A : Sort (u+1)) (p : Pt.{u} A) => pyax.{u} A (ptmk.{u} A (px.{u} A p) (px.{u} A p)).",
    ];
    Scenario {
        setup: SETUP,
        target: "pt_ax_uses",
        inductive: "Pt",
        projections: &["px", "pyax"],
    }
    .refused_identically(StructureRefusal::NotADefinition {
        projection: "pyax".into(),
    });
}

#[test]
fn a_projection_over_other_universe_parameters_is_refused_identically() {
    const SETUP: &[&str] = &[
        PT[1],
        PT[2],
        "Definition qy : (A : Sort 1) -> Pt.{0} A -> A := fun (A : Sort 1) (p : Pt.{0} A) => match p return (fun (q : Pt.{0} A) => A) with fun (a : A) (b : A) => b end.",
        "Definition pt_mono_uses : (A : Sort 1) -> Pt.{0} A -> A := fun (A : Sort 1) (p : Pt.{0} A) => qy A (ptmk.{0} A (px.{0} A p) (px.{0} A p)).",
    ];
    Scenario {
        setup: SETUP,
        target: "pt_mono_uses",
        inductive: "Pt",
        projections: &["px", "qy"],
    }
    .refused_identically(StructureRefusal::UniverseArity {
        projection: "qy".into(),
        inductive: "Pt".into(),
        expected: 1,
        found: 0,
    });
}

const BOX_SETUP: &[&str] = &[
    "Inductive Box : Type := bmk : Nat -> Box.",
    "Definition bget : Box -> Nat := fun (b : Box) => match b return (fun (q : Box) => Nat) with fun (n : Nat) => n end.",
    "Definition wrongdom : Nat -> Nat := fun (n : Nat) => n.",
    "Definition bpart : Box -> Nat -> Nat := fun (b : Box) (n : Nat) => n.",
    "Definition bzero : Box -> Nat := fun (b : Box) => Zero.",
    "Definition box_uses : Box -> Nat := fun (b : Box) => bget b.",
    "Definition box_uses_all : Box -> Nat := fun (b : Box) => wrongdom (bpart b (bzero b)).",
];

fn boxed(projections: &'static [&'static str]) -> Scenario {
    Scenario {
        setup: BOX_SETUP,
        target: "box_uses_all",
        inductive: "Box",
        projections,
    }
}

#[test]
fn a_projection_that_does_not_compute_its_field_is_refused_identically() {
    boxed(&["bzero"]).refused_identically(StructureRefusal::DoesNotCompute {
        projection: "bzero".into(),
        field: 0,
        inductive: "Box".into(),
    });
    // Applied to a `Box`, `wrongdom` is ill typed and `bpart` is a function; neither is the field.
    boxed(&["wrongdom"]).refused_identically(StructureRefusal::DoesNotCompute {
        projection: "wrongdom".into(),
        field: 0,
        inductive: "Box".into(),
    });
    boxed(&["bpart"]).refused_identically(StructureRefusal::DoesNotCompute {
        projection: "bpart".into(),
        field: 0,
        inductive: "Box".into(),
    });
    pt(&["py", "px"]).refused_identically(StructureRefusal::DoesNotCompute {
        projection: "py".into(),
        field: 0,
        inductive: "Pt".into(),
    });
    pt(&["px", "px"]).refused_identically(StructureRefusal::DoesNotCompute {
        projection: "px".into(),
        field: 1,
        inductive: "Pt".into(),
    });
}

#[test]
fn a_projection_that_computes_its_field_only_through_an_ill_typed_application_is_refused_identically(
) {
    // `wbad A (wmk A a)` reduces to `a` — the match is over `Wr`, whatever its parameter — but it
    // is ill typed: `wbad` projects `Wr Nat`, not `Wr A`. Only the typing condition refuses it.
    const SETUP: &[&str] = &[
        "Inductive Wr (A : Type) : Type := wmk : A -> Wr A.",
        "Definition wget : (A : Type) -> Wr A -> A := fun (A : Type) (w : Wr A) => match w return (fun (q : Wr A) => A) with fun (a : A) => a end.",
        "Definition wbad : (A : Type) -> Wr Nat -> Nat := fun (A : Type) (w : Wr Nat) => match w return (fun (q : Wr Nat) => Nat) with fun (n : Nat) => n end.",
        "Definition wr_uses : Nat := wbad Nat (wmk Nat (wget Nat (wmk Nat Zero))).",
    ];
    Scenario {
        setup: SETUP,
        target: "wr_uses",
        inductive: "Wr",
        projections: &["wbad"],
    }
    .refused_identically(StructureRefusal::WrongFieldType {
        projection: "wbad".into(),
        field: 0,
        inductive: "Wr".into(),
    });
    let genuine = Scenario {
        setup: SETUP,
        target: "wr_uses",
        inductive: "Wr",
        projections: &["wget"],
    };
    genuine
        .kernel_door()
        .1
        .expect("the parametric projection is granted");
    genuine
        .vernacular_door()
        .1
        .expect("granted by the vernacular");
    genuine
        .certificate_door()
        .unwrap_or_else(|e| panic!("granted by the certificate: {e}"));
}

#[test]
fn the_certificate_refuses_eta_for_an_inductive_or_projection_it_did_not_introduce() {
    // The standard library's `Prod` is a base inductive: the certificate's own scoping rule.
    let proofs = BTreeMap::new();
    let session = pt(&["px", "py"]).session();
    let request = CaptureRequest {
        context: session.context(),
        base: EnvironmentBase::StandardLibrary,
        conversion: ConversionDiscipline::Exact,
        target: "pt_uses",
        opaque_proofs: &proofs,
    };
    let certificate = EnvironmentCertificate::capture(&request).unwrap();
    let at = certificate
        .entries
        .iter()
        .position(|e| e.names().contains(&"pt_uses"))
        .unwrap();
    let with = |inductive: &str, projections: &[&str]| {
        let mut c = certificate.clone();
        c.entries.insert(
            at,
            EnvironmentEntry::Structure {
                inductive: inductive.into(),
                projections: projections.iter().map(|p| p.to_string()).collect(),
            },
        );
        c.rebuild(&EnvironmentLimits::default())
            .map(|_| ())
            .unwrap_err()
    };
    let borrowed = with("Nat", &[]);
    assert_eq!(
        borrowed.code,
        EnvironmentErrorCode::UnresolvedObligation,
        "{}",
        borrowed.message
    );
    assert!(
        borrowed
            .message
            .contains("not an inductive declared before it"),
        "{}",
        borrowed.message
    );
    let borrowed_projection = with("Pt", &["px", "add"]);
    assert_eq!(
        borrowed_projection.code,
        EnvironmentErrorCode::IllTyped,
        "{}",
        borrowed_projection.message
    );
    assert!(
        borrowed_projection
            .message
            .contains("add is not a definition declared before the structure"),
        "{}",
        borrowed_projection.message
    );
}

#[test]
fn the_lean_reader_registers_its_structures_through_the_same_door() {
    // What the reader registered, the kernel door re-derives exactly from the imported context.
    let (ctx, _) = adapter::import_audited(WITNESS).unwrap();
    let pt = n("LK_PSE.Pt");
    let imported = ctx
        .struct_info(&pt)
        .expect("registered by the reader")
        .clone();
    let mut fresh = ctx.clone();
    assert_eq!(
        fresh.register_structure_checked(&pt, &imported.projections),
        Err(StructureRefusal::AlreadyAStructure {
            inductive: pt.clone()
        }),
        "the reader's registration is final"
    );
    assert_eq!(
        imported
            .params
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        vec!["structure.parameter.0"]
    );
    assert_eq!(
        imported
            .fields
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        vec!["structure.field.0", "structure.field.1"]
    );
}
