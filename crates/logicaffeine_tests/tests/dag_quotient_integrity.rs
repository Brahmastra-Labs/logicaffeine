//! A quotient's constants are its own.
//!
//! Lean's kernel declares `Quot`, `Quot.mk`, `Quot.lift` and `Quot.ind` in one step, so no other
//! declaration ever holds one of their names and `Quot.lift f h (Quot.mk r a) ⟶ f a` only ever meets
//! the quotient's own constructor. An export states the four as four records. The `dag` core admits
//! each record against the members already declared, each of which must be the quotient's own constant
//! of its kind. It refuses a begun quotient's names to every other declaration, and it computes only on
//! constants admitted as the quotient's own. Every engine decides alike, and so does the `Term` kernel's
//! importer.
//!
//! `fixtures/quotient_forgery/` holds two exports that forge a member, byte for byte as reported
//! (Lean Kernel Arena issue #39): `axiom-quot-mk`, an ordinary axiom named `Quot.mk` on which the lift
//! computes, giving a closed proof of `False` (leanprover/lean-kernel-arena#342, `tests/bugs/axiom-quot-mk`),
//! and `forged-quot-type`, `Quot` an ordinary definition under the other records, giving `0 = 1` (#344).
//! The rest are mutations of the Arena's own `tutorial/130_quotLiftReduction`, whose theorem checks
//! only where the lift computes.

use logicaffeine_kernel::dag::env::{Const, Kind};
use logicaffeine_kernel::dag::export::Observe;
use logicaffeine_kernel::dag::tc::Engine;
use logicaffeine_kernel::dag::{check_export_in, check_export_with, Kernel, Verdict};
use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};
use std::path::PathBuf;

const KINDS: [&str; 4] = ["type", "ctor", "lift", "ind"];

fn fixture(path: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(path),
    )
    .unwrap()
}

fn lift_reduction() -> String {
    fixture("arena_corpus/good/tutorial/130_quotLiftReduction.ndjson")
}

/// Every configuration of the core that admits declarations.
fn engines() -> Vec<(&'static str, fn(&mut Kernel))> {
    #[allow(unused_mut)]
    let mut all: Vec<(&'static str, fn(&mut Kernel))> = vec![
        ("substitution", |k| k.engine = Engine::Classic),
        ("evaluation", |k| k.engine = Engine::Sem),
        ("both engines", |k| k.engine = Engine::Dual),
    ];
    #[cfg(feature = "nbe")]
    all.extend([
        ("closure conversion", (|k: &mut Kernel| k.use_nbe = true) as fn(&mut Kernel)),
        ("both conversions", |k: &mut Kernel| k.dual_conv = true),
    ]);
    all
}

fn on_a_deep_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .unwrap()
        .join()
        .expect("checker thread panicked")
}

fn verdict(export: &str, configure: fn(&mut Kernel)) -> Verdict {
    let export = export.to_string();
    on_a_deep_stack(move || check_export_with(export.as_bytes(), configure).0)
}

fn shown(v: &Verdict) -> String {
    match v {
        Verdict::Accept => "ACCEPT".to_string(),
        Verdict::Reject(e) => format!("reject ({e})"),
        Verdict::Decline(e) => format!("decline ({e})"),
    }
}

/// `export` is rejected by every engine, each time by the rule whose message contains `rule`.
fn rejected_by(what: &str, export: &str, rule: &str) {
    for (engine, configure) in engines() {
        match verdict(export, configure) {
            Verdict::Reject(e) if e.to_string().contains(rule) => {}
            v => panic!("{what}, {engine}: expected a rejection by '{rule}', got {}", shown(&v)),
        }
    }
}

fn accepted(what: &str, export: &str) {
    for (engine, configure) in engines() {
        let v = verdict(export, configure);
        assert!(matches!(v, Verdict::Accept), "{what}, {engine}: {}", shown(&v));
    }
}

fn rows(text: &str) -> Vec<Value> {
    text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn text(rows: &[Value]) -> String {
    rows.iter().map(|r| r.to_string() + "\n").collect()
}

fn quot_row(rows: &[Value], kind: &str) -> usize {
    rows.iter()
        .position(|r| r["quot"]["kind"] == kind)
        .unwrap_or_else(|| panic!("no '{kind}' quotient record"))
}

fn is_declaration(r: &Value) -> bool {
    ["axiom", "def", "thm", "opaque", "quot", "inductive"]
        .iter()
        .any(|k| r.get(*k).is_some())
}

/// The Arena's lift-reduction export with every name, level and expression record ahead of its
/// declarations, each in its own order: a stream whose declarations can be moved among themselves
/// without one citing a record that comes after it.
fn hoisted() -> Vec<Value> {
    let (declarations, records): (Vec<Value>, Vec<Value>) =
        rows(&lift_reduction()).into_iter().partition(is_declaration);
    records.into_iter().chain(declarations).collect()
}

/// The quotient record `row` restated as an ordinary axiom: its name, universe parameters and type.
fn as_axiom(row: &Value) -> Value {
    let mut d = row["quot"].clone();
    d.as_object_mut().unwrap().remove("kind");
    d["isUnsafe"] = json!(false);
    json!({ "axiom": d })
}

/// The member's role, as the kernel names it in a refusal.
fn role(kind: &str) -> &'static str {
    match kind {
        "type" => "type",
        "ctor" => "constructor",
        "lift" => "lift",
        "ind" => "induction principle",
        _ => unreachable!(),
    }
}

fn member(kind: &str) -> &'static str {
    match kind {
        "type" => "Quot",
        "ctor" => "Quot.mk",
        "lift" => "Quot.lift",
        "ind" => "Quot.ind",
        _ => unreachable!(),
    }
}

#[test]
fn the_reported_forgeries_are_rejected_by_every_engine() {
    rejected_by(
        "axiom-quot-mk",
        &fixture("quotient_forgery/axiom-quot-mk.ndjson"),
        "'Quot.mk' is the quotient's constructor",
    );
    rejected_by(
        "forged-quot-type",
        &fixture("quotient_forgery/forged-quot-type.ndjson"),
        "'Quot' is declared, but not as the quotient's type",
    );
}

#[test]
fn the_term_kernels_importer_rejects_the_reported_forgeries() {
    for name in ["axiom-quot-mk", "forged-quot-type"] {
        let export = fixture(&format!("quotient_forgery/{name}.ndjson"));
        let outcome = adapter::check_str(&export);
        assert_eq!(
            outcome.as_ref().map_err(|e| e.code()).err(),
            Some(1),
            "{name}: {outcome:?}"
        );
    }
}

#[test]
fn the_official_quotient_is_admitted_and_computes_in_every_engine() {
    accepted("130_quotLiftReduction", &lift_reduction());
    accepted("130_quotLiftReduction, hoisted", &text(&hoisted()));
    accepted(
        "131_quotIndReduction",
        &fixture("arena_corpus/good/tutorial/131_quotIndReduction.ndjson"),
    );
}

#[test]
fn a_member_stated_as_an_ordinary_axiom_is_rejected() {
    for kind in KINDS {
        let mut r = rows(&lift_reduction());
        let i = quot_row(&r, kind);
        r[i] = as_axiom(&r[i]);
        let rule = if kind == "type" {
            "'Quot' is declared, but not as the quotient's type".to_string()
        } else {
            format!("'{}' is the quotient's {}", member(kind), role(kind))
        };
        rejected_by(&format!("{kind} as an axiom"), &text(&r), &rule);
    }
}

#[test]
fn a_member_declared_before_the_quotient_is_refused_by_its_first_record() {
    for kind in KINDS {
        let mut r = hoisted();
        let prop = r
            .iter()
            .find(|e| e.get("ie").is_some() && e.get("sort") == Some(&json!(0)))
            .expect("the export has Prop")["ie"]
            .clone();
        let i = quot_row(&r, kind);
        let mut axiom = as_axiom(&r[i]);
        axiom["axiom"]["type"] = prop;
        axiom["axiom"]["levelParams"] = json!([]);
        r.remove(i);
        let first = quot_row(&r, if kind == "type" { "ctor" } else { "type" });
        r.insert(first, axiom);
        rejected_by(
            &format!("{kind} declared first"),
            &text(&r),
            &format!(
                "'{}' is declared, but not as the quotient's {}",
                member(kind),
                role(kind)
            ),
        );
    }
}

#[test]
fn a_member_declared_again_after_the_quotient_is_rejected() {
    for kind in KINDS {
        let mut again = rows(&lift_reduction());
        again.push(again[quot_row(&again, kind)].clone());
        rejected_by(
            &format!("{kind} record twice"),
            &text(&again),
            &format!("'{}' has already been declared", member(kind)),
        );
        let mut axiom = rows(&lift_reduction());
        axiom.push(as_axiom(&axiom[quot_row(&axiom, kind)]));
        rejected_by(
            &format!("{kind} as an axiom after the quotient"),
            &text(&axiom),
            &format!("'{}' has already been declared", member(kind)),
        );
    }
}

#[test]
fn a_record_follows_the_constants_its_canonical_type_names() {
    for (order, rule) in [
        (["ctor", "type", "lift", "ind"], "quotient constant declared before 'Quot'"),
        (["lift", "type", "ctor", "ind"], "quotient constant declared before 'Quot'"),
        (["ind", "type", "ctor", "lift"], "quotient constant declared before 'Quot'"),
        (["type", "ind", "ctor", "lift"], "quotient constant declared before 'Quot.mk'"),
    ] {
        rejected_by(&format!("{order:?}"), &reordered(order), rule);
    }
    accepted(
        "the lift before the constructor",
        &reordered(["type", "lift", "ctor", "ind"]),
    );
}

#[test]
fn a_quotient_computes_only_once_its_four_constants_are_admitted() {
    let mut r = rows(&lift_reduction());
    r.remove(quot_row(&r, "ind"));
    rejected_by("without Quot.ind", &text(&r), "quotLiftReduction");
}

/// The Arena's lift-reduction export, hoisted, with its quotient records in `order`.
fn reordered(order: [&str; 4]) -> String {
    let r = hoisted();
    let slots: Vec<usize> = KINDS.iter().map(|k| quot_row(&r, k)).collect();
    let mut out = r.clone();
    for (slot, kind) in slots.iter().zip(order) {
        out[*slot] = r[quot_row(&r, kind)].clone();
    }
    text(&out)
}

/// Checks the Arena's lift-reduction export in two parts on one kernel: its declarations but the
/// theorem, then — after `forge` restates one of the admitted quotient constants as an ordinary axiom,
/// the quotient left installed — the theorem, which checks only where the lift computes.
fn theorem_after(configure: fn(&mut Kernel), forge: Option<&'static str>) -> Verdict {
    let r = rows(&lift_reduction());
    let theorem = r.iter().rposition(is_declaration).unwrap();
    assert!(r[theorem].get("thm").is_some());
    let prefix: Vec<Value> = r
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != theorem)
        .map(|(_, v)| v.clone())
        .collect();
    let suffix: Vec<Value> = r
        .iter()
        .enumerate()
        .filter(|(i, v)| *i == theorem || !is_declaration(v))
        .map(|(_, v)| v.clone())
        .collect();
    let (prefix, suffix) = (text(&prefix), text(&suffix));
    on_a_deep_stack(move || {
        let mut k = Kernel::new();
        configure(&mut k);
        let (v, _) = check_export_in(&mut k, prefix.as_bytes(), None, Observe::default());
        assert!(matches!(v, Verdict::Accept), "the prefix: {}", shown(&v));
        if let Some(name) = forge {
            let n = match name {
                "Quot.mk" => k.wk.quot_mk,
                "Quot.lift" => k.wk.quot_lift,
                _ => unreachable!(),
            };
            let c = (**k.env.get(n).expect("admitted")).clone();
            k.env.remove(n);
            k.env.add(Const {
                kind: Kind::Axiom { is_unsafe: false },
                ..c
            });
            assert!(k.env.quot_init, "the quotient stays installed");
        }
        check_export_in(&mut k, suffix.as_bytes(), None, Observe::default()).0
    })
}

#[test]
fn the_lift_computes_only_on_the_quotients_own_constants() {
    for (engine, configure) in engines() {
        let v = theorem_after(configure, None);
        assert!(matches!(v, Verdict::Accept), "{engine}, the control: {}", shown(&v));
        for forged in ["Quot.mk", "Quot.lift"] {
            let v = theorem_after(configure, Some(forged));
            assert!(
                matches!(v, Verdict::Reject(_)),
                "{engine}: with {forged} an ordinary axiom the lift must not compute, got {}",
                shown(&v)
            );
        }
    }
}
