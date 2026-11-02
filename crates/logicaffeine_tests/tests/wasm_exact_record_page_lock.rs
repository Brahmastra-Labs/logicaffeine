//! Exact integer record fields keep a bounded page heap and exact holder census.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
const PAGE: &str = r#"## A Box has:
    a value, which is Int.
## No Inline
## To grow (start: Int) -> Int:
    Let n be start.
    Repeat for i from 1 to 7:
        Set n to n * n.
    Return n.
## Component App
State:
    Let box be a new Box with value 0.
Actions:
    To change:
        Set box's value to grow(3).
View:
    <div><button id="a" on-click={change}>a</button><p>{box's value}</p></div>

## Main
Mount App.
"#;
fn check(checked: bool) {
    let mut page = if checked {
        common::wasm_host::mount_interactive_checked(PAGE)
    } else {
        common::wasm_host::mount_interactive(PAGE)
    };
    let button = *page.listening("click").last().expect("record page listens");
    for _ in 0..200 {
        page.dispatch(button, "click", "")
            .expect("record action completes");
    }
    let heap = page.heap_allocated();
    let held = common::wasm_host::held_integers();
    if checked {
        assert_eq!(page.census(), 0, "exact ownership after warmup");
    }
    for _ in 200..2000 {
        page.dispatch(button, "click", "")
            .expect("record action completes");
    }
    assert_eq!(
        page.heap_allocated(),
        heap,
        "ten times the turns retain the same heap"
    );
    assert_eq!(
        common::wasm_host::held_integers(),
        held,
        "no host integer accumulates per turn"
    );
    if checked {
        assert_eq!(
            page.census(),
            0,
            "exact ownership after repeated field replacement"
        );
    }
}
#[test]
fn exact_record_fields_reuse_storage_across_page_turns() {
    check(false);
}
#[test]
fn exact_record_fields_keep_an_exact_page_census() {
    check(true);
}
