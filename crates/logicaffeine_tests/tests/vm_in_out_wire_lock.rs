//! In-out return slots and ownership proofs retain their meanings on the wire.
use logicaffeine_compile::{
    intern::Interner,
    ui_bridge::with_parsed_program,
    vm::{
        binisa::{decode_program, encode_program},
        Compiler, Vm,
    },
};
fn round_trip(source: &str, expected: &str) {
    let bytes = with_parsed_program(source, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("fixture parses");
        let program =
            Compiler::compile_with_types(stmts, interner, Some(types)).expect("fixture compiles");
        encode_program(&program, interner)
    });
    let mut receiver = Interner::new();
    let decoded = decode_program(&bytes, &mut receiver).expect("frame decodes");
    assert_eq!(
        encode_program(&decoded, &receiver),
        bytes,
        "metadata survives re-encoding"
    );
    let mut vm = Vm::new(&decoded);
    vm.run().expect("decoded program runs");
    assert_eq!(vm.output().trim_end_matches('\n'), expected);
}
#[test]
fn a_decoded_in_out_sequence_preserves_a_local_copy() {
    round_trip("## To change (xs: mutable Seq of Int) -> Int:\n    Let saved be xs.\n    Push 9 to xs.\n    Show saved.\n    Return length of xs.\n## Main\nLet xs be [4].\nShow change(xs).\nShow xs.\n", "[4]\n2\n[4, 9]");
}
#[test]
fn a_decoded_in_out_map_preserves_a_local_copy() {
    round_trip("## To change (m: mutable Map of Text to Int) -> Int:\n    Let saved be m.\n    Set item \"x\" of m to 9.\n    Show item \"x\" of saved.\n    Return length of m.\n## Main\nLet m be a new Map of Text to Int.\nSet item \"x\" of m to 4.\nShow change(m).\nShow item \"x\" of m.\n", "4\n1\n9");
}
#[test]
fn a_decoded_in_out_call_returns_a_replaced_sequence() {
    round_trip("## To change (xs: mutable Seq of Int) -> Int:\n    Set xs to [7].\n    Return length of xs.\n## Main\nLet xs be [4].\nShow change(xs).\nShow xs.\n", "1\n[7]");
}
