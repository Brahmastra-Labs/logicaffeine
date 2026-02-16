//! E2E Codegen Tests: Maps
//!
//! Mirrors e2e_maps.rs but compiles through the Rust codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_map_create_empty() {
    assert_exact_output(
        r#"## Main
Let prices be a new Map of Text to Int.
Show "ok".
"#,
        "ok",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_map_set_and_get() {
    assert_exact_output(
        r#"## Main
Let mut prices be a new Map of Text to Int.
Set item "iron" of prices to 100.
Let cost be item "iron" of prices.
Show cost.
"#,
        "100",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_map_multiple_keys() {
    assert_exact_output(
        r#"## Main
Let mut inventory be a new Map of Text to Int.
Set item "iron" of inventory to 50.
Set item "copper" of inventory to 30.
Set item "gold" of inventory to 10.
Let total be item "iron" of inventory + item "copper" of inventory + item "gold" of inventory.
Show total.
"#,
        "90",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_map_overwrite_key() {
    assert_exact_output(
        r#"## Main
Let mut prices be a new Map of Text to Int.
Set item "iron" of prices to 100.
Set item "iron" of prices to 200.
Let cost be item "iron" of prices.
Show cost.
"#,
        "200",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_map_with_text_values() {
    assert_exact_output(
        r#"## Main
Let mut names be a new Map of Text to Text.
Set item "player1" of names to "Alice".
Let name be item "player1" of names.
Show name.
"#,
        "Alice",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_map_bracket_get() {
    assert_exact_output(
        r#"## Main
Let mut prices be a new Map of Text to Int.
Set prices["iron"] to 100.
Let cost be prices["iron"].
Show cost.
"#,
        "100",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_map_bracket_set() {
    assert_exact_output(
        r#"## Main
Let mut prices be a new Map of Text to Int.
Set prices["copper"] to 50.
Set prices["copper"] to 75.
Let cost be prices["copper"].
Show cost.
"#,
        "75",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_map_mixed_syntax() {
    assert_exact_output(
        r#"## Main
Let mut inventory be a new Map of Text to Int.
Set item "iron" of inventory to 10.
Set inventory["copper"] to 20.
Let total be item "iron" of inventory + inventory["copper"].
Show total.
"#,
        "30",
    );
}
