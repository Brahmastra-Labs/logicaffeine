//! Exact sequence ownership survives every copying and consuming boundary.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
const GROW: &str = r#"## No Inline
## To grow (start: Int) -> Int:
    Let value be start.
    Repeat for i from 1 to 7:
        Set value to value * value.
    Return value.

"#;
fn agrees(body: &str) -> u32 {
    let source = format!("{GROW}{body}");
    let expected = logicaffeine_compile::compile::vm_outcome(&source);
    assert_eq!(expected.error, None, "reference fixture: {source}");
    let held = common::wasm_host::held_integers();
    let got = common::wasm_host::run_outcome(&source).expect("exact sequence module");
    assert_eq!(got.failure, None);
    assert_eq!(
        got.output.lines().collect::<Vec<_>>(),
        expected.output.lines().collect::<Vec<_>>()
    );
    assert_eq!(
        common::wasm_host::held_integers(),
        held,
        "all host integer holds are released"
    );
    got.heap_ptr.unwrap_or(0)
}
#[test]
fn pop_transfers_an_exact_element() {
    agrees("## Main\nLet values be [grow(3), grow(5)].\nPop from values into last.\nShow last.\nShow values.\n");
}
#[test]
fn a_slice_owns_its_exact_elements() {
    agrees("## Main\nLet values be [grow(3), grow(5), 7].\nLet saved be items 1 through 2 of values.\nSet item 1 of values to 41.\nShow saved.\nShow values.\n");
}
#[test]
fn concatenation_accepts_tagged_and_raw_integer_buffers() {
    agrees("## Main\nLet prefix be [grow(3)].\nLet suffix be text_bytes(\"abc\").\nLet joined be prefix + suffix.\nShow joined.\n");
}
#[test]
fn contains_compares_integer_values() {
    agrees("## Main\nLet values be [41, grow(3)].\nShow values contains grow(3).\nShow values contains grow(5).\nShow values contains 41.\n");
}
#[test]
fn a_record_sequence_field_can_receive_an_exact_element() {
    agrees("## A Bag has:\n    a values, which is Seq of Int.\n## Main\nLet bag be a new Bag with values [41].\nPush grow(3) to bag's values.\nShow bag's values.\n");
}
#[test]
fn iteration_owns_a_snapshot_during_source_replacement() {
    agrees("## Main\nLet values be [grow(3), grow(5)].\nRepeat for value in values:\n    Set values to [41].\n    Show value.\n");
}
#[test]
fn returning_from_iteration_releases_the_snapshot() {
    agrees("## No Inline\n## To first (values: Seq of Int) -> Int:\n    Repeat for value in values:\n        Return value.\n    Return 0.\n## Main\nShow first([grow(3), grow(5)]).\n");
}
#[test]
fn nested_sequence_copies_keep_owned_exact_rows() {
    agrees("## Main\nLet rows be [[grow(3)], [grow(5)]].\nLet saved be copy of rows.\nSet item 1 of item 1 of rows to 41.\nShow saved.\nShow rows.\n");
}
#[test]
fn repeated_replacement_and_iteration_keep_a_flat_heap() {
    let body = r#"## No Inline
## To churn (n: Int) -> Int:
    Let total be 0.
    Repeat for i from 1 to n:
        Let values be [grow(3), grow(5)].
        Let saved be copy of values.
        Set item 1 of values to grow(7).
        Repeat for value in saved:
            Set total to total + value.
    Return total.
## Main
Show churn($N).
"#;
    let a = agrees(&body.replace("$N", "100"));
    let b = agrees(&body.replace("$N", "1000"));
    assert_eq!(a, b, "ten times the work keeps the same linear heap");
}

#[test]
fn repeating_an_exact_value_gives_each_slot_its_own_hold() {
    agrees("## Main\nLet values be repeatSeq(grow(3), 3).\nSet item 1 of values to 41.\nShow values.\n");
}
#[test]
fn homogeneous_tuples_show_and_destructure_exact_values() {
    agrees("## Main\nLet values be (grow(3), grow(5)).\nShow values.\nLet (a, b) be values.\nShow a.\nShow b.\n");
}
fn page(checked: bool) {
    let source = format!(
        r#"{GROW}## Component App
State:
    Let values be [0].
Actions:
    To change:
        Set values to [grow(3), grow(5)].
View:
    <div><button id="a" on-click={{change}}>a</button><p>{{values}}</p></div>
## Main
Mount App.
"#
    );
    let mut page = if checked {
        common::wasm_host::mount_interactive_checked(&source)
    } else {
        common::wasm_host::mount_interactive(&source)
    };
    let button = *page
        .listening("click")
        .last()
        .expect("sequence page listens");
    for _ in 0..200 {
        page.dispatch(button, "click", "").expect("sequence action");
    }
    let heap = page.heap_allocated();
    let held = common::wasm_host::held_integers();
    if checked {
        assert_eq!(page.census(), 0);
    }
    for _ in 200..2000 {
        page.dispatch(button, "click", "").expect("sequence action");
    }
    assert_eq!(
        page.heap_allocated(),
        heap,
        "repeated turns keep a flat linear heap"
    );
    assert_eq!(
        common::wasm_host::held_integers(),
        held,
        "no host integers accumulate"
    );
    if checked {
        assert_eq!(page.census(), 0);
    }
}
#[test]
fn repeated_page_turns_reclaim_exact_sequence_values() {
    page(false);
}
#[test]
fn exact_sequence_pages_keep_an_exact_ownership_census() {
    page(true);
}

#[test]
fn byte_interop_reads_integer_values_from_tagged_slots() {
    agrees("## Main\nLet wide be [grow(3)].\nLet bytes be [65, 66, 67].\nShow text_from_bytes(bytes).\nShow item 1 of wide.\nShow bytes.\n");
}
