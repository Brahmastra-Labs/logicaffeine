//! Capacity hints are executable optimization IR, not syntax the bytecode tiers may erase.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_c::from_program::compile_program_to_c;
use logicaffeine_compile::compile::{compile_to_c, compile_to_program, program_outcome};
use logicaffeine_compile::vm::Op;

const SRC: &str = r#"## Main
Let n be 17.
Let mutable xs be a new Seq of Int with capacity n.
Push 1 to xs.
Let mutable m be a new Map of Int to Int with capacity n.
Set item 1 of m to 2.
Show xs.
Show m.
"#;

#[test]
fn bytecode_preserves_dynamic_sequence_and_map_capacity() {
    let program = compile_to_program(SRC).expect("capacity fixture compiles");

    let list_capacity = program.code.iter().find_map(|op| match op {
        Op::NewEmptyListWithCapacity { capacity, .. } => Some(*capacity),
        _ => None,
    });
    let map_capacity = program.code.iter().find_map(|op| match op {
        Op::NewEmptyMapWithCapacity { capacity, .. } => Some(*capacity),
        _ => None,
    });

    assert!(
        list_capacity.is_some(),
        "Seq capacity must remain explicit in bytecode"
    );
    assert!(
        map_capacity.is_some(),
        "Map capacity must remain explicit in bytecode"
    );
    assert_eq!(
        list_capacity, map_capacity,
        "both constructors read the same dynamic hint"
    );
}

#[test]
fn vm_and_c_bytecode_consume_capacity_aware_constructors() {
    let program = compile_to_program(SRC).expect("capacity fixture compiles");
    let outcome = program_outcome(&program);
    assert_eq!(outcome.output.trim(), "[1]\n{1: 2}");
    assert!(
        outcome.error.is_none(),
        "VM refused capacity bytecode: {:?}",
        outcome.error
    );

    let c = compile_program_to_c(&program).expect("C-bytecode lowers capacity constructors");
    assert!(
        c.contains("seq_i64_with_capacity(r["),
        "C-bytecode must reserve the sequence instead of lowering to seq_i64_new"
    );
    assert!(
        c.contains("map_i64_i64_with_capacity(r["),
        "C-bytecode must reserve the map instead of lowering to map_i64_i64_new"
    );
}

#[test]
fn direct_c_preserves_capacity_for_every_builtin_map_shape() {
    let src = r#"## Main
Let n be 17.
Let ii be a new Map of Int to Int with capacity n.
Let si be a new Map of Text to Int with capacity n.
Let ss be a new Map of Text to Text with capacity n.
Let is be a new Map of Int to Text with capacity n.
Show (length of ii) + (length of si) + (length of ss) + (length of is).
"#;
    let c = compile_to_c(src).expect("direct C lowers every builtin map shape");
    for helper in [
        "map_i64_i64_with_capacity(",
        "map_str_i64_with_capacity(",
        "map_str_str_with_capacity(",
        "map_i64_str_with_capacity(",
    ] {
        assert!(c.contains(helper), "direct C silently dropped `{helper}`");
    }
}
