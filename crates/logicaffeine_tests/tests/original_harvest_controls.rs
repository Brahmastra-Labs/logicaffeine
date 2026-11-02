//! The bridge-harvest checker is not vacuous: every property it certifies is refused when broken.
//! Controls run against the pinned T00-003 harvest fixture (`T00_003.refersToBase : Nat := base`).

#[path = "harvest_support/checker.rs"]
mod checker;
use checker::{check_root, fixture, Root};
use logicaffeine_kernel::interface::lean::reference::Reference;

const ROOT: Root = Root {
    lean: "T00_003.refersToBase",
    kind: "def",
    axioms: &[],
};

fn original() -> String {
    fixture("T00-003").0
}

/// The export with `refersToBase`'s declaration record rewritten by `edit`.
fn corrupted(edit: impl Fn(&mut serde_json::Value)) -> String {
    let export = original();
    let name_index = export
        .lines()
        .find_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            (v["str"]["str"] == "refersToBase").then(|| v["in"].as_u64())?
        })
        .expect("the export names refersToBase");
    let mut touched = 0;
    let out: Vec<String> = export
        .lines()
        .map(|l| {
            let mut v: serde_json::Value = serde_json::from_str(l).unwrap();
            if v["def"]["name"].as_u64() == Some(name_index) {
                edit(&mut v["def"]);
                touched += 1;
                v.to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    assert_eq!(touched, 1, "exactly one declaration record is corrupted");
    out.join("\n") + "\n"
}

#[test]
fn the_unaltered_root_is_checked() {
    let export = original();
    assert!(check_root(&export, &Reference::read(&export).unwrap(), &ROOT).is_ok());
}

#[test]
fn a_wrong_lean_axiom_record_is_refused() {
    let export = original();
    let lying = Root {
        lean: ROOT.lean,
        kind: ROOT.kind,
        axioms: &["propext"],
    };
    let err = check_root(&export, &Reference::read(&export).unwrap(), &lying)
        .err()
        .expect("refused");
    assert!(err.contains("#print axioms"), "{err}");
}

#[test]
fn a_wrong_declaration_kind_is_refused() {
    let export = original();
    let lying = Root {
        lean: ROOT.lean,
        kind: "theorem",
        axioms: &[],
    };
    assert!(check_root(&export, &Reference::read(&export).unwrap(), &lying).is_err());
}

#[test]
fn a_body_swapped_for_one_of_another_type_is_refused() {
    // `value` 73 is the universe-polymorphic `OfNat.ofNat` instance body, not a `Nat`.
    let export = corrupted(|d| d["value"] = serde_json::json!(73));
    let reference = Reference::read(&export).unwrap();
    assert!(check_root(&export, &reference, &ROOT).is_err());
}

#[test]
fn a_statement_swapped_for_another_type_is_refused() {
    // `type` 69 is the statement of `OfNat.ofNat`; the body `base : Nat` does not inhabit it.
    let export = corrupted(|d| d["type"] = serde_json::json!(69));
    let reference = Reference::read(&export).unwrap();
    assert!(check_root(&export, &reference, &ROOT).is_err());
}

#[test]
#[should_panic(expected = "export bytes")]
fn a_fixture_whose_bytes_changed_is_refused() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lean_conformance/cases_lean414/harvest");
    let tmp = tempfile_dir();
    for suffix in ["ndjson", "provenance.json"] {
        std::fs::copy(
            dir.join(format!("T00-003.{suffix}")),
            tmp.join(format!("T00-003.{suffix}")),
        )
        .unwrap();
    }
    let path = tmp.join("T00-003.ndjson");
    let mut bytes = std::fs::read_to_string(&path).unwrap();
    bytes.push('\n');
    std::fs::write(&path, bytes).unwrap();
    checker::fixture_in(&tmp, "T00-003");
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("harvest-control-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
