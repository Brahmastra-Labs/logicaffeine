//! The binary ISA (`vm/binisa.rs`) serializes every op with `bincode`, and `bincode` writes an enum
//! as its variant POSITION. So a builtin's wire id — what an encoded program and the silicon decoder
//! carry for `CallBuiltin` — is its position in `enum BuiltinId`. Inserting a variant anywhere but
//! the end silently renumbers every builtin after it: a program encoded yesterday decodes to
//! different builtins today.
//!
//! `goldens/builtin_wire_ids.txt` is the append-only record of those positions. Nothing in it may
//! ever move; a new builtin is appended to the enum AND to the file.

use logicaffeine_compile::semantics::builtins::BuiltinId;

fn golden() -> Vec<String> {
    include_str!("goldens/builtin_wire_ids.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect()
}

fn wire_id(b: BuiltinId) -> u32 {
    let bytes = bincode::serialize(&b).expect("a builtin serializes");
    u32::from_le_bytes(
        bytes[..4]
            .try_into()
            .expect("bincode writes a u32 variant index"),
    )
}

fn name(b: BuiltinId) -> String {
    serde_json::to_string(&b)
        .expect("a builtin names itself")
        .trim_matches('"')
        .to_string()
}

#[test]
fn every_builtin_keeps_its_wire_id() {
    let golden = golden();
    let mut moved = Vec::new();
    for &b in BuiltinId::ALL {
        let id = wire_id(b) as usize;
        match golden.get(id) {
            Some(expected) if *expected == name(b) => {}
            Some(expected) => moved.push(format!(
                "wire id {id} is `{}` but the record says `{expected}`",
                name(b)
            )),
            None => moved.push(format!(
                "`{}` has wire id {id}, past the end of the record — append it",
                name(b)
            )),
        }
    }
    assert!(
        moved.is_empty(),
        "builtin wire ids moved (append new builtins at the END):\n{}",
        moved.join("\n")
    );
}

#[test]
fn the_record_names_every_builtin_exactly_once() {
    let golden = golden();
    assert_eq!(
        golden.len(),
        BuiltinId::ALL.len(),
        "the record and BuiltinId::ALL must list the same builtins"
    );
    let mut sorted = golden.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        golden.len(),
        "a builtin appears twice in the record"
    );
}
