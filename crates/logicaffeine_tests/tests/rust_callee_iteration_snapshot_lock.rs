//! A callee may mutate the source while iteration keeps its original values.
mod common;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

#[test]
fn callee_mutation_preserves_the_original_iteration_values() {
    let source = r#"## No Inline
## To change () -> Int:
    Set item 1 of items to 9.
    Return 1.
## Main
Let mutable items: Seq of Int be [1, 2, 3].
Repeat for x in items:
    Show x.
    Let ignored be change().
Show items.
"#;
    let expected = "1\n2\n3\n[9, 2, 3]";
    for (name, run) in [("tree walker", tw_outcome(source)), ("VM", vm_outcome(source))] {
        assert_eq!(run.error, None, "{name}");
        assert_eq!(run.output.trim_end_matches('\n'), expected, "{name}");
    }
    common::assert_exact_output(source, expected);
}
