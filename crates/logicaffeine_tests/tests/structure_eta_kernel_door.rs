//! Structure η through the kernel door: its verdict is exactly what it registered.
//!
//! `Context::register_structure_checked` answers `Ok` only when it has granted η. A container that
//! a native nested positivity check has sealed cannot be changed, so every door refuses it by name
//! and leaves the context untouched: a refusal that read as a grant would let a caller rely on η
//! that does not exist. A constructor shorter than its inductive's parameter telescope is refused
//! before anything is built from it. The public metadata door `Context::register_struct_info` is a
//! door too: it grants η only when the same decision grants it, so no caller can hand the kernel a
//! "projection" that does not compute its field and collapse a type onto one of its values.
use std::collections::BTreeMap;

use logicaffeine_kernel::certificate::{
    CaptureRequest, ConversionDiscipline, EnvironmentBase, EnvironmentCertificate,
    EnvironmentEntry, EnvironmentErrorCode, EnvironmentLimits,
};
use logicaffeine_kernel::interface::{InterfaceError, Repl};
use logicaffeine_kernel::{
    infer_type, recheck, Context, KernelError, MutualInductive, StructInfo, StructureRefusal, Term,
    Universe,
};

const CONTAINER: [&str; 2] = [
    "Inductive Bx (A : Type) : Type := bxmk : A -> Bx A.",
    "Definition bxget : (A : Type) -> Bx A -> A := fun (A : Type) (b : Bx A) => match b return (fun (q : Bx A) => A) with fun (a : A) => a end.",
];
const NESTED: &str = "Inductive Tr : Type := trnode : Bx Tr -> Tr.";
const USES: &str = "Definition tr_uses : Tr -> Nat := fun (t : Tr) => Zero.";

fn session(lines: &[&str]) -> Repl {
    let mut r = Repl::new();
    for line in lines {
        r.execute(line).unwrap_or_else(|e| panic!("{line}: {e:?}"));
    }
    r
}

fn sealed() -> StructureRefusal {
    StructureRefusal::Sealed {
        inductive: "Bx".into(),
    }
}

#[test]
fn a_container_is_a_structure_until_a_nested_inductive_seals_it() {
    let mut open = session(&CONTAINER);
    open.execute("Structure Bx := bxget.")
        .expect("an unsealed container is structure-like");
    assert!(open.context().struct_info("Bx").is_some());
    // Declared in this order, the nested inductive does not disturb the structure it nests through.
    open.execute(NESTED).expect("Tr nests through a structure");
    assert!(open.context().struct_info("Bx").is_some());
}

#[test]
fn the_kernel_door_refuses_a_sealed_container_and_leaves_the_context_unchanged() {
    let r = session(&[CONTAINER[0], CONTAINER[1], NESTED]);
    let mut ctx = r.context().clone();
    let refused = ctx.refused_declarations().to_vec();
    assert_eq!(
        ctx.register_structure_checked("Bx", &["bxget".into()]),
        Err(sealed())
    );
    assert!(ctx.struct_info("Bx").is_none(), "no η was granted");
    assert_eq!(
        ctx.refused_declarations(),
        &refused[..],
        "a refusal changes nothing, not even the refusal log"
    );
}

#[test]
fn the_kernel_door_refuses_a_container_sealed_through_the_raw_block_door() {
    let r = session(&CONTAINER);
    let mut ctx = r.context().clone();
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "Tr".into(),
        sort: Term::Sort(Universe::Type(0)),
        num_params: 0,
        constructors: vec![(
            "trnode".into(),
            Term::Pi {
                param: "field".into(),
                param_type: Box::new(Term::App(
                    Box::new(Term::Global("Bx".into())),
                    Box::new(Term::Global("Tr".into())),
                )),
                body_type: Box::new(Term::Global("Tr".into())),
            },
        )],
    }])
    .expect("Tr nests through Bx");
    assert_eq!(
        ctx.register_structure_checked("Bx", &["bxget".into()]),
        Err(sealed())
    );
    assert!(ctx.struct_info("Bx").is_none());
}

#[test]
fn the_vernacular_door_refuses_a_sealed_container_with_the_kernels_reason() {
    let mut r = session(&[CONTAINER[0], CONTAINER[1], NESTED]);
    match r.execute("Structure Bx := bxget.") {
        Err(InterfaceError::Kernel(KernelError::InvalidDeclaration(message))) => {
            assert_eq!(message, sealed().to_string())
        }
        other => panic!("a sealed container must be refused, not reported granted: {other:?}"),
    }
    assert!(r.context().struct_info("Bx").is_none());
}

#[test]
fn the_certificate_door_refuses_a_sealed_container() {
    let session = session(&[CONTAINER[0], CONTAINER[1], NESTED, USES]);
    let proofs = BTreeMap::new();
    let request = CaptureRequest {
        context: session.context(),
        base: EnvironmentBase::StandardLibrary,
        conversion: ConversionDiscipline::Exact,
        target: "tr_uses",
        opaque_proofs: &proofs,
    };
    let mut certificate =
        EnvironmentCertificate::capture(&request).unwrap_or_else(|e| panic!("capture: {e}"));
    let at = certificate
        .entries
        .iter()
        .position(|e| e.names().contains(&"tr_uses"))
        .expect("tr_uses is captured");
    certificate.entries.insert(
        at,
        EnvironmentEntry::Structure {
            inductive: "Bx".into(),
            projections: vec!["bxget".into()],
        },
    );
    let refused = certificate
        .rebuild(&EnvironmentLimits::default())
        .map(|_| ())
        .expect_err("a sealed container gets no η");
    assert_eq!(
        refused.code,
        EnvironmentErrorCode::IllTyped,
        "{}",
        refused.message
    );
    assert_eq!(refused.message, sealed().to_string());
}

#[test]
fn a_constructor_shorter_than_its_parameters_is_refused_at_the_kernel_door() {
    let mut ctx: Context = Repl::new().context().clone();
    let ty0 = Term::Sort(Universe::Type(0));
    ctx.add_indexed_inductive(
        "Sh",
        Term::Pi {
            param: "A".into(),
            param_type: Box::new(ty0.clone()),
            body_type: Box::new(ty0),
        },
        1,
    );
    ctx.add_constructor(
        "shmk",
        "Sh",
        Term::App(
            Box::new(Term::Global("Sh".into())),
            Box::new(Term::Global("Nat".into())),
        ),
    );
    assert_eq!(
        ctx.register_structure_checked("Sh", &[]),
        Err(StructureRefusal::ShortConstructor {
            constructor: "shmk".into(),
            params: 1
        })
    );
    assert!(ctx.struct_info("Sh").is_none());
}

const BOX: [&str; 3] = [
    "Inductive Box : Type := bmk : Nat -> Box.",
    "Definition bget : Box -> Nat := fun (b : Box) => match b return (fun (q : Box) => Nat) with fun (n : Nat) => n end.",
    "Definition bzero : Box -> Nat := fun (b : Box) => Zero.",
];

fn box_info(projection: &str, field: &str) -> StructInfo {
    StructInfo {
        mk: "bmk".into(),
        num_params: 0,
        projections: vec![projection.into()],
        params: vec![],
        fields: vec![(field.into(), Term::Global("Nat".into()))],
        parents: vec![],
        out_params: vec![],
    }
}

fn g(n: &str) -> Term {
    Term::Global(n.into())
}

fn app(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter()
        .fold(f, |f, x| Term::App(Box::new(f), Box::new(x)))
}

/// `(λ h : Π x:Box. Eq Box x (bmk Zero). h) (λ x : Box. refl Box x)` — well typed exactly when every
/// box is definitionally `bmk Zero`, which constructor injectivity turns into `Zero = n` for all `n`.
fn every_box_is_bmk_zero() -> Term {
    every_box_is(|_| app(g("bmk"), vec![g("Zero")]))
}

/// `(λ h : Π x:Box. Eq Box x (rebuild x). h) (λ x : Box. refl Box x)`.
fn every_box_is(rebuild: impl Fn(Term) -> Term) -> Term {
    let x = || Term::Var("x".into());
    let statement = Term::Pi {
        param: "x".into(),
        param_type: Box::new(g("Box")),
        body_type: Box::new(app(g("Eq"), vec![g("Box"), x(), rebuild(x())])),
    };
    let proof = Term::Lambda {
        param: "x".into(),
        param_type: Box::new(g("Box")),
        body: Box::new(app(g("refl"), vec![g("Box"), x()])),
    };
    Term::App(
        Box::new(Term::Lambda {
            param: "h".into(),
            param_type: Box::new(statement),
            body: Box::new(Term::Var("h".into())),
        }),
        Box::new(proof),
    )
}

#[test]
fn the_metadata_door_refuses_a_projection_that_does_not_compute_its_field() {
    let mut ctx = session(&BOX).context().clone();
    assert!(
        infer_type(&ctx, &every_box_is_bmk_zero()).is_err(),
        "without η a box is not its constant reconstruction"
    );
    let refused = ctx.refused_declarations().len();
    ctx.register_struct_info("Box", box_info("bzero", "value"));
    assert!(
        ctx.struct_info("Box").is_none(),
        "`bzero` is not a projection of `Box`, so no η is granted"
    );
    let expected = StructureRefusal::DoesNotCompute {
        projection: "bzero".into(),
        field: 0,
        inductive: "Box".into(),
    };
    assert_eq!(
        ctx.refused_declarations().len(),
        refused + 1,
        "the refusal is recorded, as every unchecked door records one"
    );
    assert_eq!(
        ctx.refused_declarations().last(),
        Some(&("Box".to_string(), expected.to_string()))
    );
    assert!(
        infer_type(&ctx, &every_box_is_bmk_zero()).is_err(),
        "every box is still not `bmk Zero`"
    );
    assert!(
        recheck(&ctx, &every_box_is_bmk_zero()).is_err(),
        "in either kernel"
    );
}

#[test]
fn the_metadata_door_refuses_a_constructor_or_arity_that_is_not_the_inductives() {
    for (label, info) in [
        (
            "another constructor",
            StructInfo {
                mk: "Succ".into(),
                ..box_info("bget", "value")
            },
        ),
        (
            "another parameter count",
            StructInfo {
                num_params: 1,
                ..box_info("bget", "value")
            },
        ),
    ] {
        let mut ctx = session(&BOX).context().clone();
        let refused = ctx.refused_declarations().len();
        ctx.register_struct_info("Box", info);
        assert!(ctx.struct_info("Box").is_none(), "{label}");
        assert_eq!(ctx.refused_declarations().len(), refused + 1, "{label}");
    }
}

#[test]
fn the_metadata_door_grants_a_real_projection_and_keeps_the_callers_metadata() {
    let mut ctx = session(&BOX).context().clone();
    let refused = ctx.refused_declarations().len();
    ctx.register_struct_info("Box", box_info("bget", "value"));
    let info = ctx
        .struct_info("Box")
        .expect("`bget` computes the field")
        .clone();
    assert_eq!(
        (info.mk.as_str(), info.num_params, info.projections.clone()),
        ("bmk", 0, vec!["bget".to_string()])
    );
    assert_eq!(
        info.fields
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        ["value"],
        "the caller's field names are kept"
    );
    assert_eq!(ctx.refused_declarations().len(), refused);
    // η now holds, and still only for the true reconstruction.
    let eta = every_box_is(|x| app(g("bmk"), vec![app(g("bget"), vec![x])]));
    infer_type(&ctx, &eta).unwrap_or_else(|e| panic!("every box is its own reconstruction: {e:?}"));
    recheck(&ctx, &eta).unwrap_or_else(|e| panic!("in either kernel: {e:?}"));
    assert!(infer_type(&ctx, &every_box_is_bmk_zero()).is_err());
    let second = ctx.refused_declarations().len();
    ctx.register_struct_info("Box", box_info("bget", "value"));
    assert_eq!(
        ctx.refused_declarations().len(),
        second + 1,
        "a second grant is refused and recorded"
    );
}
