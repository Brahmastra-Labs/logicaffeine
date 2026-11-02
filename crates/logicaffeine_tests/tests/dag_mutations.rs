//! Every single-point corruption of a valid export's inductive declarations is rejected.
//!
//! The kernel derives an inductive family's recursors, rules and flags itself and admits the
//! export's claims only if they are identical. This takes valid Arena exports, applies each
//! small mutation an adversary could make to the claimed metadata (a recursor's K flag, its
//! counts, a rule's arity or body, a constructor's field count or index, the recursive and
//! reflexive flags, a dropped or duplicated declaration), and requires a REJECTION for every
//! one: an acceptance would be unsound, and a decline would be a claim-free dodge.

use logicaffeine_kernel::dag::{check_export, Verdict};
use serde_json::Value;
use std::io::BufReader;
use std::path::PathBuf;

fn corpus(path: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(path);
    std::fs::read_to_string(p).expect("vendored Arena case")
}

fn verdict(text: String) -> Verdict {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || check_export(BufReader::new(text.as_bytes())).0)
        .unwrap()
        .join()
        .unwrap()
}

/// The export as records, and the index of each `inductive` record.
fn records(text: &str) -> (Vec<Value>, Vec<usize>) {
    let recs: Vec<Value> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let inds = recs
        .iter()
        .enumerate()
        .filter(|(_, r)| r.get("inductive").is_some())
        .map(|(i, _)| i)
        .collect();
    (recs, inds)
}

fn render(recs: &[Value]) -> String {
    recs.iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// All single-point mutations of the inductive record at `idx`, each with a description.
fn mutations(recs: &[Value], idx: usize) -> Vec<(String, Vec<Value>)> {
    let mut out = Vec::new();
    let ind = &recs[idx]["inductive"];
    let mut push = |what: String, f: &dyn Fn(&mut Value)| {
        let mut copy = recs.to_vec();
        f(&mut copy[idx]["inductive"]);
        if copy[idx] != recs[idx] {
            out.push((what, copy));
        }
    };
    let bump = |v: &mut Value, by: i64| {
        let n = v.as_i64().unwrap();
        *v = Value::from((n + by).max(0));
    };
    for (r, rec) in ind["recs"].as_array().unwrap().iter().enumerate() {
        push(format!("recs[{r}].k flipped"), &|i| {
            let k = i["recs"][r]["k"].as_bool().unwrap();
            i["recs"][r]["k"] = Value::from(!k);
        });
        for field in ["numParams", "numIndices", "numMotives", "numMinors"] {
            push(format!("recs[{r}].{field} + 1"), &|i| {
                bump(&mut i["recs"][r][field], 1)
            });
            if rec[field].as_i64().unwrap() > 0 {
                push(format!("recs[{r}].{field} - 1"), &|i| {
                    bump(&mut i["recs"][r][field], -1)
                });
            }
        }
        let rules = rec["rules"].as_array().unwrap();
        for (j, rule) in rules.iter().enumerate() {
            push(format!("recs[{r}].rules[{j}].nfields + 1"), &|i| {
                bump(&mut i["recs"][r]["rules"][j]["nfields"], 1)
            });
            if rule["nfields"].as_i64().unwrap() > 0 {
                push(format!("recs[{r}].rules[{j}].nfields - 1"), &|i| {
                    bump(&mut i["recs"][r]["rules"][j]["nfields"], -1)
                });
            }
            for (k2, other) in rules.iter().enumerate() {
                if k2 != j && other["rhs"] != rule["rhs"] {
                    let rhs = other["rhs"].clone();
                    push(
                        format!("recs[{r}].rules[{j}].rhs := rules[{k2}].rhs"),
                        &|i| i["recs"][r]["rules"][j]["rhs"] = rhs.clone(),
                    );
                }
                if k2 != j && other["ctor"] != rule["ctor"] {
                    let ctor = other["ctor"].clone();
                    push(
                        format!("recs[{r}].rules[{j}].ctor := rules[{k2}].ctor"),
                        &|i| i["recs"][r]["rules"][j]["ctor"] = ctor.clone(),
                    );
                }
            }
            if rules.len() > 0 {
                push(format!("recs[{r}].rules[{j}] dropped"), &|i| {
                    i["recs"][r]["rules"].as_array_mut().unwrap().remove(j);
                });
            }
        }
        push(format!("recs[{r}] dropped"), &|i| {
            i["recs"].as_array_mut().unwrap().remove(r);
        });
        push(format!("recs[{r}] duplicated"), &|i| {
            let dup = i["recs"][r].clone();
            i["recs"].as_array_mut().unwrap().push(dup);
        });
    }
    for (c, ctor) in ind["ctors"].as_array().unwrap().iter().enumerate() {
        push(format!("ctors[{c}].numFields + 1"), &|i| {
            bump(&mut i["ctors"][c]["numFields"], 1)
        });
        if ctor["numFields"].as_i64().unwrap() > 0 {
            push(format!("ctors[{c}].numFields - 1"), &|i| {
                bump(&mut i["ctors"][c]["numFields"], -1)
            });
        }
        push(format!("ctors[{c}].numParams + 1"), &|i| {
            bump(&mut i["ctors"][c]["numParams"], 1)
        });
        push(format!("ctors[{c}].cidx + 1"), &|i| {
            bump(&mut i["ctors"][c]["cidx"], 1)
        });
        push(format!("ctors[{c}].isUnsafe flipped"), &|i| {
            let u = i["ctors"][c]["isUnsafe"].as_bool().unwrap();
            i["ctors"][c]["isUnsafe"] = Value::from(!u);
        });
    }
    for (t, _) in ind["types"].as_array().unwrap().iter().enumerate() {
        for flag in ["isRec", "isReflexive", "isUnsafe"] {
            push(format!("types[{t}].{flag} flipped"), &|i| {
                let b = i["types"][t][flag].as_bool().unwrap();
                i["types"][t][flag] = Value::from(!b);
            });
        }
        for field in ["numIndices", "numNested", "numParams"] {
            push(format!("types[{t}].{field} + 1"), &|i| {
                bump(&mut i["types"][t][field], 1)
            });
        }
    }
    out
}

/// Families covering the recursor shapes: enumeration, recursive, indexed, structure, K-like
/// equality, reflexive, and a nested/mutual family where the corpus has one.
const FAMILIES: &[&str] = &[
    "good/tutorial/062_emptyRec.ndjson",
    "good/tutorial/063_boolRec.ndjson",
    "good/tutorial/064_twoBoolRec.ndjson",
    "good/tutorial/065_andRec.ndjson",
    "good/tutorial/066_prodRec.ndjson",
    "good/tutorial/067_pprodRec.ndjson",
    "good/tutorial/068_punitRec.ndjson",
    "good/tutorial/069_eqRec.ndjson",
    "good/tutorial/070_nRec.ndjson",
    "good/tutorial/071_rbTreeRef.ndjson",
    "good/tutorial/074_existsRec.ndjson",
    "good/tutorial/076_sortElimPropRec.ndjson",
    "good/tutorial/099_ruleK.ndjson",
    "good/tutorial/121_rTreeRec.ndjson",
    "good/tutorial/123_accRecType.ndjson",
];

#[test]
fn every_single_point_corruption_of_an_inductive_is_rejected() {
    let mut checked = 0;
    let mut failures = Vec::new();
    for family in FAMILIES {
        let text = corpus(family);
        assert!(
            matches!(verdict(text.clone()), Verdict::Accept),
            "{family} is a valid export"
        );
        let (recs, inds) = records(&text);
        assert!(!inds.is_empty(), "{family} declares an inductive family");
        assert!(
            matches!(verdict(render(&recs)), Verdict::Accept),
            "{family}: re-rendering the untouched records must still be accepted"
        );
        for idx in inds {
            for (what, mutant) in mutations(&recs, idx) {
                checked += 1;
                match verdict(render(&mutant)) {
                    Verdict::Reject(e) if e.message.starts_with("malformed export") => failures
                        .push(format!(
                            "{family}: {what}: rejected as malformed, not on its claim: {e}"
                        )),
                    Verdict::Reject(_) => {}
                    Verdict::Accept => {
                        failures.push(format!("{family}: {what}: ACCEPTED (unsound)"))
                    }
                    Verdict::Decline(e) => failures.push(format!(
                        "{family}: {what}: declined instead of rejected: {e}"
                    )),
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} mutations not rejected:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        checked >= 400,
        "the matrix covers every family: {checked} mutations"
    );
}

/// Declaration-level corruptions that are invalid by construction, whatever the declaration:
/// a theorem whose proof is replaced by a sort (a proof of a proposition cannot be a type), a
/// declaration with a duplicated universe parameter, and a declaration renamed onto an earlier
/// one.
fn declaration_mutations(recs: &[Value]) -> Vec<(String, Vec<Value>)> {
    let sort_expr = recs
        .iter()
        .find(|r| r.get("sort").is_some() && r.get("ie").is_some())
        .map(|r| r["ie"].clone())
        .expect("the export defines a sort");
    let mut out = Vec::new();
    let mut seen_names: Vec<Value> = Vec::new();
    for (i, r) in recs.iter().enumerate() {
        for kind in ["thm", "def", "axiom", "opaque"] {
            let Some(d) = r.get(kind) else { continue };
            if kind == "thm" {
                let mut copy = recs.to_vec();
                copy[i][kind]["value"] = sort_expr.clone();
                out.push((format!("record {i}: theorem proof := a sort"), copy));
            }
            if let Some(ps) = d["levelParams"].as_array().filter(|ps| !ps.is_empty()) {
                let mut copy = recs.to_vec();
                let mut dup = ps.clone();
                dup.push(ps[0].clone());
                copy[i][kind]["levelParams"] = Value::from(dup);
                out.push((
                    format!("record {i}: {kind} with a duplicated universe parameter"),
                    copy,
                ));
            }
            if let Some(prev) = seen_names.last() {
                let mut copy = recs.to_vec();
                copy[i][kind]["name"] = prev.clone();
                out.push((
                    format!("record {i}: {kind} renamed onto an earlier declaration"),
                    copy,
                ));
            }
            seen_names.push(d["name"].clone());
        }
    }
    out
}

const DECLARATION_FILES: &[&str] = &[
    "good/tutorial/001_basicDef.ndjson",
    "good/tutorial/013_thmProof.ndjson",
    "good/tutorial/018_levelParams.ndjson",
    "good/tutorial/030_peano1.ndjson",
    "good/tutorial/078_boolRecEqns.ndjson",
    "good/tutorial/080_nRecReduction.ndjson",
    "good/tutorial/104_proofIrrelevance.ndjson",
    "good/tutorial/112_structEta.ndjson",
    "good/tutorial/130_quotLiftReduction.ndjson",
    "good/perf/shared-subterm.ndjson",
];

#[test]
fn every_invalid_by_construction_declaration_is_rejected() {
    let mut checked = 0;
    let mut failures = Vec::new();
    for file in DECLARATION_FILES {
        let text = corpus(file);
        let (recs, _) = records(&text);
        assert!(
            matches!(verdict(render(&recs)), Verdict::Accept),
            "{file} re-renders to a valid export"
        );
        for (what, mutant) in declaration_mutations(&recs) {
            checked += 1;
            match verdict(render(&mutant)) {
                Verdict::Reject(e) if e.message.starts_with("malformed export") => failures.push(
                    format!("{file}: {what}: rejected as malformed, not on its claim: {e}"),
                ),
                Verdict::Reject(_) => {}
                Verdict::Accept => failures.push(format!("{file}: {what}: ACCEPTED (unsound)")),
                Verdict::Decline(e) => {
                    failures.push(format!("{file}: {what}: declined instead of rejected: {e}"))
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} mutations not rejected:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        checked >= 20,
        "the declaration family is exercised: {checked} mutations"
    );
}
