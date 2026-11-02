use super::*;

#[test]
fn transition_lines_format_with_and_without_a_name() {
    assert_eq!(
        format_transition(3, "fib", ExecTier::Warm),
        "[tier] fn#3 'fib' -> warm"
    );
    assert_eq!(
        format_transition(7, "quicksort", ExecTier::NativeForge),
        "[tier] fn#7 'quicksort' -> native(forge)"
    );
    assert_eq!(
        format_transition(0, "", ExecTier::NativeAot),
        "[tier] fn#0 -> native(aot)"
    );
    assert_eq!(
        format_transition(1, "main", ExecTier::Bytecode),
        "[tier] fn#1 'main' -> bytecode"
    );
}
