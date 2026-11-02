//! `SharedMap from K to V` — the key-value CRDT — on every tier.
//!
//! Keys are an observed-remove set (a concurrent write beats a remove: add-wins). Each value is a
//! last-writer register stamped with the map's own Lamport time: a write that has SEEN another write
//! beats it, and two concurrent writes resolve to the larger value. No wall clock, no replica id — so
//! two replicas that have seen the same writes hold the same map, whatever order they merged in, on
//! the tree-walker, the VM, and the compiled binary alike. A merged map has no global insertion order,
//! so it renders and iterates by key.
//!
//! The Studio's Shared Map example once failed with "Cannot index into Nothing" (the interpreter had no
//! map CRDT) and its compiled form did not build (`ORMap<String, i64>`: a plain `i64` cannot merge).

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_all_tiers};

const WAREHOUSE: &str =
    "## Definition\nA Warehouse is Shared and has:\n    stock: a SharedMap from Text to Int.\n\n";

fn program(main: &str) -> String {
    format!("{WAREHOUSE}## Main\n{main}")
}

/// `(name, main body, exact output)`.
const CASES: &[(&str, &str, &str)] = &[
    (
        "studio_example",
        "Let mutable w be a new Warehouse.\nSet w's stock[\"wood\"] to 50.\nSet w's stock[\"stone\"] to 30.\nShow w's stock[\"wood\"].\n",
        "50",
    ),
    (
        "surface",
        "Let mutable w be a new Warehouse.\n\
         Show length of w's stock.\n\
         Set w's stock[\"wood\"] to 50.\n\
         Set w's stock[\"stone\"] to 30.\n\
         Set w's stock[\"clay\"] to 7.\n\
         Set w's stock[\"wood\"] to 55.\n\
         Show w's stock[\"wood\"].\n\
         Show item \"clay\" of w's stock.\n\
         Show length of w's stock.\n\
         Show w's stock contains \"stone\".\n\
         Show w's stock contains \"iron\".\n\
         Show w's stock.\n\
         Remove \"stone\" from w's stock.\n\
         Show w's stock contains \"stone\".\n\
         Show length of w's stock.\n\
         Show w's stock.\n\
         Repeat for (name, count) in w's stock:\n\
         \x20   Show name + \" = \" + format(count).\n",
        "0\n55\n7\n3\ntrue\nfalse\n{clay: 7, stone: 30, wood: 55}\nfalse\n2\n{clay: 7, wood: 55}\nclay = 7\nwood = 55",
    ),
    (
        "merge_converges_in_either_order",
        "Let mutable a be a new Warehouse.\n\
         Set a's stock[\"wood\"] to 50.\n\
         Set a's stock[\"stone\"] to 30.\n\
         Let mutable b be a new Warehouse.\n\
         Set b's stock[\"wood\"] to 99.\n\
         Set b's stock[\"iron\"] to 4.\n\
         Let mutable ab be a.\n\
         Merge b into ab.\n\
         Let mutable ba be b.\n\
         Merge a into ba.\n\
         Show ab's stock.\n\
         Show ba's stock.\n",
        "{iron: 4, stone: 30, wood: 99}\n{iron: 4, stone: 30, wood: 99}",
    ),
    (
        "a_write_that_saw_the_merge_wins",
        "Let mutable a be a new Warehouse.\n\
         Set a's stock[\"wood\"] to 50.\n\
         Let mutable b be a new Warehouse.\n\
         Set b's stock[\"wood\"] to 99.\n\
         Merge b into a.\n\
         Set a's stock[\"wood\"] to 1.\n\
         Merge a into b.\n\
         Show a's stock[\"wood\"].\n\
         Show b's stock[\"wood\"].\n",
        "1\n1",
    ),
    (
        "merge_is_idempotent",
        "Let mutable a be a new Warehouse.\n\
         Set a's stock[\"wood\"] to 50.\n\
         Let mutable b be a new Warehouse.\n\
         Set b's stock[\"clay\"] to 3.\n\
         Merge b into a.\n\
         Merge b into a.\n\
         Merge a into a.\n\
         Show a's stock.\n\
         Show length of a's stock.\n",
        "{clay: 3, wood: 50}\n2",
    ),
    (
        "a_concurrent_write_beats_a_remove",
        "Let mutable a be a new Warehouse.\n\
         Set a's stock[\"wood\"] to 50.\n\
         Let mutable b be a new Warehouse.\n\
         Merge a into b.\n\
         Remove \"wood\" from a's stock.\n\
         Set b's stock[\"wood\"] to 60.\n\
         Merge b into a.\n\
         Show a's stock.\n",
        "{wood: 60}",
    ),
    (
        "a_remove_that_saw_the_write_holds",
        "Let mutable a be a new Warehouse.\n\
         Set a's stock[\"wood\"] to 50.\n\
         Let mutable b be a new Warehouse.\n\
         Merge a into b.\n\
         Remove \"wood\" from b's stock.\n\
         Merge b into a.\n\
         Show a's stock contains \"wood\".\n\
         Show length of a's stock.\n",
        "false\n0",
    ),
];

#[test]
fn shared_map_on_the_tree_walker_and_vm() {
    let mut failures = Vec::new();
    for (name, main, out) in CASES {
        let src = program(main);
        let r = run_all_tiers(&src);
        if (r.output.trim(), r.error.as_deref()) != (*out, None) {
            failures.push(format!(
                "{name}: output {:?}, error {:?}; expected {out:?}\n{src}",
                r.output.trim(),
                r.error
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn shared_map_when_compiled() {
    for (name, main, out) in CASES {
        eprintln!("compiled: {name}");
        assert_compiled_equals_interpreted_eq(&program(main), out);
    }
}

#[test]
fn a_missing_key_traps_like_a_map() {
    let r = run_all_tiers(&program(
        "Let w be a new Warehouse.\nShow \"before\".\nShow w's stock[\"iron\"].\n",
    ));
    assert_eq!(r.output.trim(), "before");
    assert_eq!(r.error.as_deref(), Some("Key 'iron' not found in map"));
}

#[test]
fn other_key_and_value_types() {
    let src = "## Definition\nA Board is Shared and has:\n    names: a SharedMap from Int to Text.\n    flags: a SharedMap from Text to Bool.\n\n\
               ## Main\nLet mutable b be a new Board.\nSet b's names[2] to \"two\".\nSet b's names[1] to \"one\".\n\
               Set b's flags[\"on\"] to true.\nSet b's flags[\"off\"] to false.\nShow b's names.\nShow b's flags.\n\
               Show b's names[1] + \"!\".\n";
    let out = "{1: one, 2: two}\n{off: false, on: true}\none!";
    let r = run_all_tiers(src);
    assert_eq!((r.output.trim(), r.error.as_deref()), (out, None));
    assert_compiled_equals_interpreted_eq(src, out);
}

/// Every shared COLLECTION iterates, in the order every replica agrees on: a set sorted, a sequence
/// in its convergent order. A counter is a number, not a collection.
#[test]
fn every_shared_collection_iterates() {
    let src = "## Definition\nA Room is Shared and has:\n    tags: a SharedSet of Text.\n    log: a SharedSequence of Text.\n\n\
               ## Main\nLet mutable r be a new Room.\nAdd \"pear\" to r's tags.\nAdd \"apple\" to r's tags.\n\
               Append \"first\" to r's log.\nAppend \"second\" to r's log.\n\
               Repeat for t in r's tags:\n    Show t.\nRepeat for line in r's log:\n    Show line.\n";
    let out = "apple\npear\nfirst\nsecond";
    let r = run_all_tiers(src);
    assert_eq!((r.output.trim(), r.error.as_deref()), (out, None));
    assert_compiled_equals_interpreted_eq(src, out);
}

/// ★ SPLIT (Tristen APPROVED, 2026-09-25) from the single, now-stale
/// `every_backend_without_the_map_crdt_refuses_it_loudly`, once the WASM backend gained a REAL
/// single-replica `SharedMap` lowering: a `SharedMap` program that never calls `Merge` is no
/// longer something every non-native backend refuses, so one test can no longer assert both
/// halves at once. This half is the CAPABILITY: a single-replica `SharedMap` (writes and reads,
/// no `Merge`) must compile to wasm AND, run for real under `node`'s V8 (not just checked for a
/// successful compile), produce EXACTLY the tree-walker's/VM's output. The other half —
/// `every_backend_without_the_map_crdt_merge_refuses_it_loudly`, below — is the REFUSAL.
#[test]
fn a_single_replica_shared_map_runs_identically_on_wasm_tree_walker_and_vm() {
    use logicaffeine_compile::compile::compile_to_wasm;
    use logicaffeine_compile::web_host::wasm_host_shim;
    if !common::require_tool("node") {
        return;
    }
    let src = program("Let mutable w be a new Warehouse.\nSet w's stock[\"wood\"] to 50.\nShow w's stock[\"wood\"].\n");
    let reference = run_all_tiers(&src);
    assert_eq!(
        reference.error, None,
        "the reference tiers must run a single-replica SharedMap"
    );
    let wasm = compile_to_wasm(&src).unwrap_or_else(|e| {
        panic!("a single-replica SharedMap must compile to wasm: {e:?}\n{src}")
    });
    let dir = std::env::temp_dir().join(format!("logos-shared-map-wasm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("probe.wasm"), &wasm).expect("write wasm");
    std::fs::write(dir.join("probe.mjs"), wasm_host_shim("probe")).expect("write shim");
    let out = std::process::Command::new("node")
        .arg(dir.join("probe.mjs"))
        .output()
        .expect("node preflighted present but failed to spawn");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "the wasm module must run a single-replica SharedMap program:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stdout.trim_end(),
        reference.output.trim_end(),
        "wasm must display exactly what the tree-walker and VM agree on"
    );
}

/// The REFUSAL half of the split above: a program that actually calls `Merge` on a `SharedMap`
/// must be refused, BY NAME, on every backend that cannot merge one — including WASM, whose
/// single-replica lowering (proven above) still has no CRDT merge algorithm. The refusal must
/// speak to the AUTHOR: what they wrote ("merging a SharedMap"), never an internal type name like
/// `CrdtMerge` — a message is worse than none if the reader cannot connect it to their own source.
#[test]
fn every_backend_without_the_map_crdt_merge_refuses_it_loudly() {
    use logicaffeine_compile::compile;
    let src = program(
        "Let mutable a be a new Warehouse.\nSet a's stock[\"wood\"] to 50.\n\
         Let mutable b be a new Warehouse.\nSet b's stock[\"wood\"] to 99.\n\
         Merge b into a.\nShow a's stock[\"wood\"].\n",
    );
    let src = src.as_str();
    let outcomes: Vec<(&str, Result<(), String>)> = vec![
        (
            "C",
            compile::compile_to_c(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "C (from bytecode)",
            compile::compile_to_c_via_bytecode(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "RISC-V",
            compile::compile_to_riscv(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "WASM",
            compile::compile_to_wasm(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "soft core",
            logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
    ];
    let wrong: Vec<String> = outcomes
        .into_iter()
        .filter_map(|(backend, r)| match r {
            Err(e) if e.contains("CrdtMerge") => Some(format!(
                "{backend}: names the internal type `CrdtMerge`, not what the author wrote: {e}"
            )),
            Err(e) if e.to_lowercase().contains("sharedmap") || e.contains("SharedMap") => None,
            Err(e) => Some(format!("{backend}: refused without naming SharedMap: {e}")),
            Ok(()) => Some(format!(
                "{backend}: lowered a `Merge` of a SharedMap it cannot merge"
            )),
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
