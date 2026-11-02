//! R6 — to obtain acceptance, an unsupported family is never replaced by an axiom, an index is
//! never erased, and an elaborator `have` marker is never silently dropped.
//!
//! Every positive input here is an original Lean 4.14.0 export (the witness modules live beside
//! their exports in `fixtures/lean_semantics/`, the `have` case is corpus case T00-019). Every
//! Lean-positive family the reader cannot reconstruct is refused AT that family, typed as an
//! unsupported feature, before any declaration that uses it; a family the reader does
//! reconstruct keeps its parameters and indices exactly. The marker controls change one flag of
//! an original export and require the refusal, and the unchanged original the acceptance.
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, recheck, Term, Universe,
};
use serde_json::{json, Value};

struct Witness {
    family: &'static str,
    user: &'static str,
    input: &'static str,
    provenance: &'static str,
    source: &'static str,
    source_sha256: &'static str,
    export_sha256: &'static str,
    bytes: usize,
    block_line: usize,
}

macro_rules! witness {
    ($stem:literal, $module:literal, $family:literal, $user:literal, $source_sha:literal, $sha:literal,
     $bytes:literal, $line:literal) => {
        Witness {
            family: $family,
            user: $user,
            input: include_str!(concat!("fixtures/lean_semantics/", $stem, ".ndjson")),
            provenance: include_str!(concat!(
                "fixtures/lean_semantics/",
                $stem,
                ".provenance.json"
            )),
            source: include_str!(concat!("fixtures/lean_semantics/", $module, ".lean")),
            source_sha256: $source_sha,
            export_sha256: $sha,
            bytes: $bytes,
            block_line: $line,
        }
    };
}

/// A Lean-positive nested family outside the safe kernel fragment: Lean 4.14.0 accepts the
/// `unsafe` block; LOGOS admits only safe declarations.
fn unsafe_family() -> Witness {
    witness!(
        "LF_W03_UnsafeNested",
        "LF_W03_UnsafeNested",
        "LF_W03S.Wild",
        "LF_W03S.wildLeaf",
        "5db77c9cdce6d996d89c4fdb77e271a8249f3b4f1c1c5dd6b65e403b44520471",
        "ccbcd1506b786901cbc5df76984ee54449a8184da287b504bdc4089b0075c553",
        10_197,
        179
    )
}

/// Lean-positive nested families in every W03 dimension, one export each.
fn frontier() -> [Witness; 5] {
    [
        witness!(
            "LF_W03_NestedFrontier.UTree",
            "LF_W03_NestedFrontier",
            "LF_W03N.UTree",
            "LF_W03N.uLeaf",
            "4b33e8b90c8b5af51a736daeea2c0fb4e3c0e68d585328c12576691f3aed8d18",
            "88dfd62f4508dd29d89745ff3ad6f2dc15ed9b1ad192e67ae016158c2efaa43e",
            18_251,
            196
        ),
        witness!(
            "LF_W03_NestedFrontier.VTree",
            "LF_W03_NestedFrontier",
            "LF_W03N.VTree",
            "LF_W03N.vPair",
            "4b33e8b90c8b5af51a736daeea2c0fb4e3c0e68d585328c12576691f3aed8d18",
            "f5a126b6b893972031258bbfe495bcfa80d8cd892e8950ce40df876dfaa36d39",
            44_478,
            765
        ),
        witness!(
            "LF_W03_NestedFrontier.ITree",
            "LF_W03_NestedFrontier",
            "LF_W03N.ITree",
            "LF_W03N.iNode",
            "4b33e8b90c8b5af51a736daeea2c0fb4e3c0e68d585328c12576691f3aed8d18",
            "7a3781fc5c73004fed3264b8db1c02d3da4fda50af40fce3d3a156840cc2f613",
            19_041,
            321
        ),
        witness!(
            "LF_W03_NestedFrontier.DTree",
            "LF_W03_NestedFrontier",
            "LF_W03N.DTree",
            "LF_W03N.dLeaf",
            "4b33e8b90c8b5af51a736daeea2c0fb4e3c0e68d585328c12576691f3aed8d18",
            "121f5e30f1059ff2e510d20ee7e24a4b38ff4bf28034eb4478fb265492c5f891",
            69_273,
            419
        ),
        witness!(
            "LF_W03_NestedFrontier.MTree",
            "LF_W03_NestedFrontier",
            "LF_W03N.MTree",
            "LF_W03N.mEmpty",
            "4b33e8b90c8b5af51a736daeea2c0fb4e3c0e68d585328c12576691f3aed8d18",
            "89ef5709a003f77fcb1106e6713853eaab0ffee65135f52955f17b727b31f787",
            15_743,
            265
        ),
    ]
}

fn rows(input: &str) -> Vec<Value> {
    input
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn text(rows: &[Value]) -> String {
    rows.iter().map(|r| format!("{r}\n")).collect()
}
fn name_id(rows: &[Value], dotted: &str) -> u64 {
    let mut id = 0;
    for part in dotted.split('.') {
        id = rows
            .iter()
            .find(|r| r["str"]["pre"] == id && r["str"]["str"] == part)
            .unwrap_or_else(|| panic!("name {dotted}"))["in"]
            .as_u64()
            .unwrap();
    }
    id
}
/// 1-based line of the record declaring `dotted` (the header is line 1).
fn line_of(rows: &[Value], dotted: &str) -> usize {
    let id = name_id(rows, dotted);
    1 + rows
        .iter()
        .position(|r| {
            r["inductive"]["types"]
                .as_array()
                .is_some_and(|ts| ts.iter().any(|t| t["name"] == id))
                || ["def", "thm", "axiom", "opaque"]
                    .iter()
                    .any(|k| r[k]["name"] == id)
        })
        .unwrap_or_else(|| panic!("{dotted} is declared"))
}
fn pinned(w: &Witness) {
    let p: Value = serde_json::from_str(w.provenance).unwrap();
    assert_eq!(p["source_sha256"], w.source_sha256);
    assert_eq!(p["export_sha256"], w.export_sha256);
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["declarations"], json!([w.family, w.user]));
    assert_eq!(p["records"], w.input.lines().count());
    assert_eq!(w.input.len(), w.bytes);
    let short = w.family.rsplit('.').next().unwrap();
    assert!(
        w.source.contains(&format!("inductive {short}")),
        "the witness source declares {short}"
    );
    assert_eq!(
        line_of(&rows(w.input), w.family),
        w.block_line,
        "{} block line",
        w.family
    );
    assert!(
        line_of(&rows(w.input), w.user) > w.block_line,
        "the user follows its family"
    );
}
fn n(dotted: &str) -> String {
    dotted
        .split('.')
        .map(|part| format!("/s:{part:?}"))
        .collect()
}

#[test]
fn an_unsupported_nested_family_is_declined_not_axiomatized() {
    let w = unsafe_family();
    pinned(&w);
    let rows = rows(w.input);
    let block = w.block_line;
    assert_eq!(
        rows[block - 1]["inductive"]["types"][0]["isUnsafe"],
        json!(true),
        "Lean's own unsafe flag"
    );

    // Declined — a typed unsupported feature — exactly at the family, never later.
    for verdict in [
        adapter::check_str(w.input).expect_err(w.family),
        adapter::import_audited(w.input).err().expect(w.family),
        adapter::check_str_with_declaration_audit(w.input).expect_err(w.family),
    ] {
        assert!(matches!(verdict, Verdict::Decline(_)), "{verdict:?}");
        assert_eq!(verdict.code(), 2);
        assert_eq!(
            verdict.reason().code(),
            DiagnosticCode::UnsupportedFeature,
            "{verdict:?}"
        );
        assert!(
            verdict
                .reason()
                .message()
                .starts_with(&format!("line {block}: unsafe")),
            "{verdict:?}"
        );
    }

    // Nothing stands in for the family: everything before it imports, and it is absent.
    let (prefix, _) = adapter::import_audited(&text(&rows[..block - 1]))
        .expect("the family's dependencies import");
    for name in [w.family, "LF_W03S.Wild.node", "LF_W03S.Wild.rec"] {
        assert!(
            prefix.get_global(&n(name)).is_none(),
            "{name} was not declared by any means"
        );
        assert!(
            prefix.get_universe_poly(&n(name)).is_none(),
            "{name} was not declared by any means"
        );
    }
}

#[test]
fn an_axiomatized_stand_in_could_not_hide_behind_an_accepted_import() {
    // What approximation would look like, on a family the reader DOES reconstruct: the block
    // rewritten as axioms (every family and constructor) imports only as explicit assumptions,
    // which the user's audit names. The reconstructed original's user has none.
    let w = &frontier()[4];
    pinned(w);
    let rows = rows(w.input);
    let block = w.block_line;
    let (_, report) = adapter::import_audited(w.input).expect("the reconstructed original");
    let honest = report
        .declarations
        .iter()
        .find(|d| d.name == n(w.user))
        .unwrap();
    assert!(
        honest.axioms.is_empty(),
        "Lean: {} depends on no axioms",
        w.user
    );
    let record = &rows[block - 1]["inductive"];
    let mut stand_ins = Vec::new();
    let mut names = Vec::new();
    for item in record["types"]
        .as_array()
        .unwrap()
        .iter()
        .chain(record["ctors"].as_array().unwrap())
    {
        stand_ins.push(json!({"axiom": {"name": item["name"], "type": item["type"],
            "levelParams": item["levelParams"], "isUnsafe": false}}));
        names.push(item["name"].clone());
    }
    let mut approximated = rows[..block - 1].to_vec();
    approximated.extend(stand_ins);
    approximated.extend(rows[block..].iter().cloned());
    let report = adapter::check_str_with_declaration_audit(&text(&approximated))
        .expect("the axiomatized stand-in imports as explicit assumptions");
    let user = report
        .declarations
        .iter()
        .find(|d| d.name == n(w.user))
        .unwrap();
    assert!(
        !user.axioms.is_empty(),
        "stand-in axioms are visible in the user's assumptions"
    );
    let spelled = |id: &Value| -> String {
        let mut parts = Vec::new();
        let mut cur = id.as_u64().unwrap();
        while cur != 0 {
            let row = rows.iter().find(|r| r["in"] == cur).unwrap();
            parts.push(row["str"]["str"].as_str().unwrap().to_string());
            cur = row["str"]["pre"].as_u64().unwrap();
        }
        parts.reverse();
        n(&parts.join("."))
    };
    let stand_in_names: Vec<String> = names.iter().map(spelled).collect();
    assert!(
        user.axioms.iter().all(|a| stand_in_names.contains(a)),
        "{:?} ⊄ {stand_in_names:?}",
        user.axioms
    );
}

#[test]
fn supported_nested_families_are_reconstructed_as_families_not_reported_absent() {
    for w in frontier() {
        pinned(&w);
        let (ctx, report) = adapter::import_audited(w.input).expect(w.family);
        let family = n(w.family);
        assert!(
            ctx.is_inductive(&family),
            "{family} is a family, not an axiom"
        );
        assert!(
            ctx.get_universe_poly(&format!("{family}/s:\"rec\""))
                .is_some(),
            "{family} has its recursor"
        );
        assert!(
            ctx.get_universe_poly(&format!("{family}/s:\"rec_1\""))
                .is_some(),
            "{family} has its nested recursor"
        );
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(w.user))
            .unwrap();
        assert!(
            audit.axioms.is_empty(),
            "Lean: {} depends on no axioms; got {:?}",
            w.user,
            audit.axioms
        );
        let usage = axiom_dependencies_of(&ctx, &family).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{family}: {usage:?}"
        );
        for ty in [
            infer_type(&ctx, &Term::Global(n(w.user))).unwrap(),
            recheck(&ctx, &Term::Global(n(w.user))).unwrap(),
        ] {
            let mut head = &ty;
            while let Term::App(f, _) = head {
                head = f;
            }
            assert!(
                matches!(head, Term::Global(h) | Term::Const { name: h, .. } if *h == family),
                "{}: {ty}",
                w.user
            );
        }
    }
}

/// The domains of the first `names.len()` binders, each binder renamed to the caller's name.
fn domains(mut ty: Term, names: &[&str]) -> Vec<Term> {
    let mut out = Vec::new();
    for name in names {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = ty
        else {
            panic!("short telescope")
        };
        out.push(*param_type);
        ty = logicaffeine_kernel::substitute(&body_type, &param, &Term::Var((*name).into()));
    }
    out
}

#[test]
fn an_index_is_never_erased() {
    // The indexed container `Vec` precedes `VTree` in its export and is reconstructed exactly:
    // one parameter, one index, the constructor's result at `n + 1`.
    let vtree = &frontier()[1];
    let rows = rows(vtree.input);
    let prefix = text(&rows[..vtree.block_line - 1]);
    let (ctx, _) = adapter::import_audited(&prefix).expect("Vec imports");
    let vec = n("LF_W03N.Vec");
    assert_eq!(
        (
            ctx.inductive_num_params(&vec),
            ctx.inductive_num_indices(&vec)
        ),
        (1, 1)
    );
    let ty0 = || Term::Sort(Universe::Type(0));
    let nat = || Term::Global("Nat".into());
    let arity = Term::Pi {
        param: "α".into(),
        param_type: Box::new(ty0()),
        body_type: Box::new(Term::Pi {
            param: "n".into(),
            param_type: Box::new(nat()),
            body_type: Box::new(ty0()),
        }),
    };
    assert!(defeq_for_test(&ctx, ctx.get_global(&vec).unwrap(), &arity));

    // Erasing the index, or promoting it to a uniform parameter, is rejected: the constructors
    // return `Vec α 0` and `Vec α (n + 1)`, which no parameter can be.
    let at = line_of(&rows, "LF_W03N.Vec") - 1;
    for (params, indices) in [(2, 0), (1, 0), (0, 2)] {
        let mut forged = rows[..vtree.block_line - 1].to_vec();
        forged[at]["inductive"]["types"][0]["numParams"] = json!(params);
        forged[at]["inductive"]["types"][0]["numIndices"] = json!(indices);
        let verdict = adapter::check_str(&text(&forged)).expect_err("forged index split");
        assert!(
            matches!(verdict, Verdict::Reject(_)),
            "({params},{indices}): {verdict:?}"
        );
        assert_eq!(verdict.reason().code(), DiagnosticCode::InvalidProof);
    }

    // Nesting THROUGH that indexed container keeps the index an index: `VTree.node` stores a
    // `Vec VTree 2`, yet the auxiliary recursor's motive ranges over EVERY index — it is never
    // specialized to 2 as if the index were a parameter.
    let (ctx, _) = adapter::import_audited(vtree.input).unwrap();
    let vec_of = |i: Term| {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global(vec.clone())),
                Box::new(Term::Global(n("LF_W03N.VTree"))),
            )),
            Box::new(i),
        )
    };
    let (_, aux, _) = ctx.get_universe_poly(&n("LF_W03N.VTree.rec_1")).unwrap();
    let motive_2 = domains(aux.clone(), &["m1", "m2"])[1].clone();
    let expected = Term::Pi {
        param: "i".into(),
        param_type: Box::new(nat()),
        body_type: Box::new(Term::Pi {
            param: "t".into(),
            param_type: Box::new(vec_of(Term::Var("i".into()))),
            body_type: Box::new(Term::Sort(Universe::Var(n("u")))),
        }),
    };
    assert!(
        defeq_for_test(&ctx, &motive_2, &expected),
        "motive_2 ranges over the index: {motive_2}"
    );
    // An indexed nested FAMILY keeps its index too; its nested occurrence `List (ITree 0)`
    // keeps the literal index inside the container instance.
    let itree = &frontier()[2];
    let (ctx, _) = adapter::import_audited(itree.input).unwrap();
    assert_eq!(ctx.inductive_num_indices(&n("LF_W03N.ITree")), 1);
    let (_, rec, _) = ctx.get_universe_poly(&n("LF_W03N.ITree.rec")).unwrap();
    let motive_1 = domains(rec.clone(), &["m1"])[0].clone();
    let itree_at = |i: Term| Term::App(Box::new(Term::Global(n("LF_W03N.ITree"))), Box::new(i));
    assert!(
        defeq_for_test(
            &ctx,
            &motive_1,
            &Term::Pi {
                param: "i".into(),
                param_type: Box::new(nat()),
                body_type: Box::new(Term::Pi {
                    param: "t".into(),
                    param_type: Box::new(itree_at(Term::Var("i".into()))),
                    body_type: Box::new(Term::Sort(Universe::Var(n("u"))))
                })
            }
        ),
        "{motive_1}"
    );
}

// ==== the elaborator `have` marker ====

const HAVE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T00-019.sharing.structural.ndjson");

fn have_rows() -> Vec<Value> {
    assert_eq!(
        HAVE.len(),
        include_str!("fixtures/lean_conformance/cases_lean414/T00-019.sharing.structural.ndjson")
            .len()
    );
    rows(HAVE)
}

#[test]
fn a_let_fun_marker_is_never_silently_dropped() {
    let rows = have_rows();
    // Without a marker: Lean 4.14's `have` is the `letFun` constant, imported as exported.
    let via_have = n("T00_019.viaHave");
    let (ctx, report) = adapter::import_audited_retaining_source(HAVE, &via_have).unwrap();
    let (_, _, body) = ctx.retained_source().unwrap();
    let mut head = body;
    while let Term::App(f, _) = head {
        head = f;
    }
    assert!(
        matches!(head, Term::Const { name, .. } if name == &n("letFun")),
        "the exported letFun application is kept, not rewritten: {body}"
    );
    let (levels, _, value) = ctx
        .get_universe_poly(&n("letFun"))
        .expect("letFun is the exported definition");
    assert_eq!(levels.len(), 2);
    assert!(
        matches!(value, Term::Lambda { .. }),
        "letFun keeps its transparent value: {value}"
    );
    assert_eq!(
        report
            .metadata_losses
            .iter()
            .map(|m| m.key.as_str())
            .collect::<Vec<_>>(),
        vec!["borrowed"]
    );

    // The same export, the `have` value wrapped in an elaborator `letFun` annotation: declined.
    let mut marked = rows.clone();
    let have_id = name_id(&rows, "T00_019.viaHave");
    let at = rows
        .iter()
        .position(|r| r["def"]["name"] == have_id)
        .unwrap();
    let value = rows[at]["def"]["value"].clone();
    let next = rows.iter().filter_map(|r| r["ie"].as_u64()).max().unwrap() + 1;
    marked.insert(at, json!({"ie": next, "mdata": {"expr": value, "data": {"letFun": "Lean.DataValue.ofBool true"}}}));
    marked[at + 1]["def"]["value"] = json!(next);
    let verdict = adapter::check_str(&text(&marked)).unwrap_err();
    assert!(matches!(verdict, Verdict::Decline(_)), "{verdict:?}");
    assert_eq!(verdict.reason().code(), DiagnosticCode::UnsupportedFeature);
    assert!(
        verdict
            .reason()
            .message()
            .starts_with(&format!("line {}: letFun", at + 1)),
        "{verdict:?}"
    );

    // Any other key on the same node is transparent and reported exactly: the refusal is specific.
    let mut annotated = marked.clone();
    annotated[at]["mdata"]["data"] = json!({"pp.analysis": "Lean.DataValue.ofBool true"});
    let report = adapter::check_str_with_report(&text(&annotated)).unwrap();
    assert!(report
        .metadata_losses
        .iter()
        .any(|m| m.expression == next as usize && m.key == "pp.analysis"));

    // The later elaborator's `have` — a `nondep` let. viaLet's let is `nondep: false` and imports;
    // flipping only that flag is declined, not imported as a transparent let.
    let let_at = rows.iter().position(|r| r.get("letE").is_some()).unwrap();
    assert_eq!(rows[let_at]["letE"]["nondep"], json!(false));
    adapter::check_str(HAVE).expect("the unmarked original");
    let mut nondep = rows.clone();
    nondep[let_at]["letE"]["nondep"] = json!(true);
    let verdict = adapter::check_str(&text(&nondep)).unwrap_err();
    assert!(matches!(verdict, Verdict::Decline(_)), "{verdict:?}");
    assert_eq!(verdict.reason().code(), DiagnosticCode::UnsupportedFeature);
    assert!(
        verdict
            .reason()
            .message()
            .starts_with(&format!("line {}: a nondep", let_at + 1)),
        "{verdict:?}"
    );
}
