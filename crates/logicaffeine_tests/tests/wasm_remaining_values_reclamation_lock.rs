//! Reclamation of values whose standalone layouts still use raw allocations.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;
use logicaffeine_compile::compile::vm_outcome;

fn parity(source: &str) -> u32 {
    let expected = vm_outcome(source);
    assert_eq!(expected.error, None, "valid VM program:\n{source}");
    let got = run_outcome(source).expect("valid wasm module");
    assert_eq!(got.failure, None, "wasm runs:\n{source}");
    assert_eq!(
        got.output.lines().collect::<Vec<_>>(),
        expected.output.lines().collect::<Vec<_>>(),
        "reclamation preserves every displayed value:\n{source}"
    );
    got.heap_ptr.expect("allocating program has a heap")
}

fn flat(declarations: &str, body: &str) {
    let mut heaps = Vec::new();
    for n in [100, 1000] {
        let source = format!("{declarations}## Main\nRepeat for i from 1 to {n}:\n{body}");
        heaps.push(parity(&source));
    }
    assert_eq!(
        heaps[0], heaps[1],
        "a bounded live set must reuse its storage"
    );
}

#[test]
fn dropped_rational_values_reuse_their_storage() {
    flat("", "    Let ratio: Rational be i / 3.\n    Show ratio.\n");
}

#[test]
fn a_rational_payload_is_not_a_collection_header() {
    flat("## A Box has:\n    a value, which is Rational.\n\n",
         "    Let ratio: Rational be (2 * i + 1) / 4294967296.\n    Let box be a new Box with value ratio.\n    Show box's value.\n");
}

#[test]
fn dropped_counter_replicas_reuse_their_storage() {
    flat("## A Counter is Shared and has:\n    a points, which is Tally.\n\n",
         "    Let counter be a new Counter.\n    Increase counter's points by i.\n    Show counter's points.\n");
}

#[test]
fn dropped_closures_reuse_their_capture_storage() {
    flat(
        "",
        "    Let next be (n: Int) -> n + i.\n    Show next(1).\n",
    );
}

#[test]
fn dropped_closures_release_captured_collections() {
    flat(
        "",
        "    Let xs be [i, i + 1].\n    Let pick be (n: Int) -> item n of xs.\n    Show pick(1).\n",
    );
}

#[test]
fn a_rational_local_survives_nested_allocation_and_collection() {
    parity("## To keep (n: Int) -> Rational:\n    Let saved: Rational be n / 3.\n    Repeat for i from 1 to 1000:\n        Let temporary: Rational be i / 5.\n        Show temporary.\n    Return saved.\n\n## Main\nShow keep(7).\n");
}

#[test]
fn a_rational_record_field_survives_nested_allocation_and_collection() {
    parity("## A Box has:\n    a value, which is Rational.\n\n## To keep (n: Int) -> Rational:\n    Let saved be a new Box with value (n / 3).\n    Repeat for i from 1 to 1000:\n        Let temporary: Rational be i / 5.\n        Show temporary.\n    Return saved's value.\n\n## Main\nShow keep(7).\n");
}

#[test]
fn rational_fields_promote_integer_construction_and_updates() {
    flat("## A Box has:\n    a value, which is Rational.\n\n",
         "    Let mutable box be a new Box with value i.\n    Show box's value.\n    Set box's value to i + 1.\n    Show box's value.\n");
}

#[test]
fn rational_record_copies_keep_shared_payloads_alive() {
    parity("## A Box has:\n    a value, which is Rational.\n\n## To keep (n: Int) -> Rational:\n    Let mutable box be a new Box with value n.\n    Let saved be box.\n    Set box's value to n + 1.\n    Repeat for i from 1 to 1000:\n        Let temporary: Rational be i / 5.\n        Show temporary.\n    Show box's value.\n    Return saved's value.\n\n## Main\nShow keep(7).\n");
}

#[test]
fn rational_fields_keep_their_representation_across_function_calls() {
    parity("## A Box has:\n    a value, which is Rational.\n\n## To read (box: Box) -> Rational:\n    Return box's value.\n\n## Main\nLet box be a new Box with value 42.\nShow read(box).\n");
}

#[test]
fn new_counter_replicas_remain_distinct_after_collection() {
    parity("## A Counter is Shared and has:\n    a points, which is Tally.\n\n## Main\nLet combined be a new Counter.\nRepeat for i from 1 to 1000:\n    Let remote be a new Counter.\n    Increase remote's points by i.\n    Merge remote into combined.\n    Merge remote into combined.\nShow combined's points.\n");
}

#[test]
fn dropped_merged_counters_reuse_their_replica_buffers() {
    flat("## A Counter is Shared and has:\n    a points, which is Tally.\n\n",
         "    Let local be a new Counter.\n    Let remote be a new Counter.\n    Increase local's points by i.\n    Increase remote's points by 2 * i.\n    Merge remote into local.\n    Merge remote into local.\n    Show local's points.\n");
}

#[test]
fn a_counter_record_survives_nested_collection() {
    parity("## A Counter is Shared and has:\n    a points, which is Tally.\n\n## To keep (n: Int) -> Int:\n    Let saved be a new Counter.\n    Increase saved's points by n.\n    Repeat for i from 1 to 1000:\n        Let temporary be a new Counter.\n        Increase temporary's points by i.\n        Show temporary's points.\n    Return saved's points.\n\n## Main\nShow keep(7).\n");
}

#[test]
fn rational_scalar_copies_preserve_values_and_reuse_storage() {
    flat("", "    Let original: Rational be i / 3.\n    Let saved be copy of original.\n    Show original.\n    Show saved.\n");
}

#[test]
fn deep_copies_of_promoted_rational_fields_keep_their_payload_alive() {
    parity("## A Box has:\n    a value, which is Rational.\n\n## To keep (n: Int) -> Rational:\n    Let mutable box be a new Box with value n.\n    Let saved be copy of box.\n    Set box's value to n + 1.\n    Repeat for i from 1 to 1000:\n        Let temporary: Rational be i / 5.\n        Show temporary.\n    Return saved's value.\n\n## Main\nShow keep(7).\n");
}

#[test]
fn deep_copies_of_rational_fields_preserve_fractional_values() {
    flat("## A Box has:\n    a value, which is Rational.\n\n",
         "    Let ratio: Rational be i / 3.\n    Let mutable box be a new Box with value ratio.\n    Let saved be copy of box.\n    Set box's value to i.\n    Show saved's value.\n    Show box's value.\n");
}

#[test]
fn a_live_closure_survives_a_callees_collection() {
    parity("## To churn (n: Int) -> Int:\n    Let mutable total be 0.\n    Repeat for i from 1 to n:\n        Let f be (x: Int) -> x + i.\n        Set total to total + f(1).\n    Return total.\n\n## To keep (seed: Int) -> Int:\n    Let values be [seed, seed + 1].\n    Let kept be (x: Int) -> item 1 of values.\n    Let work be churn(1000).\n    Return kept(0).\n\n## Main\nShow keep(73).\n");
}

#[test]
fn dropped_closures_release_nested_closures() {
    flat("", "    Let first be (n: Int) -> n + i.\n    Let second be (n: Int) -> first(n).\n    Show second(1).\n");
}
