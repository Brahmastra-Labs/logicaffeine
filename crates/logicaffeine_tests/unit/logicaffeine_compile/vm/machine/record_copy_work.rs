//! Count actual record-field copying independently of machine timing.
use super::*;

fn run(source: &str, expected: &str) -> (usize, usize) {
    let program = crate::compile::compile_to_program(source).expect("record program compiles");
    let mut vm = Vm::new(&program);
    vm.run().expect("record program runs");
    assert_eq!(
        vm.output().trim_end(),
        expected,
        "the program's exact answer stays unchanged"
    );
    vm.record_field_copy_work()
}

#[test]
fn an_unaliased_local_record_does_not_copy_its_growing_field() {
    for n in [32, 256] {
        let source = format!(
            "## A Box has:\n    a tape, which is Seq of Int.
## To poke(n: Int) -> Int:
    Let b be a new Box with tape (a new Seq of Int).
    Let mutable i be 1.
    While i <= n:
        Push i to b's tape.
        Set i to i + 1.
    Set i to 1.
    While i <= n:
        Let v be item i of b's tape.
        Set item i of b's tape to v + 1.
        Set i to i + 1.
    Return item n of b's tape.
## Main\nShow poke({n}).\n"
        );
        let (copies, elements) = run(&source, &(n + 1).to_string());
        assert_eq!(
            (copies, elements),
            (0, 0),
            "no second program value owns this list; dead scratch reads must not copy it at n={n}"
        );
    }
}

#[test]
fn an_unaliased_record_in_a_global_list_does_not_copy_its_growing_field() {
    for n in [32, 256] {
        let source = format!(
            "## A Box has:\n    a tape, which is Seq of Int.
## To fill(n: Int):
    Let mutable i be 1.
    While i <= n:
        Push i to (item 1 of boxes)'s tape.
        Set i to i + 1.
## Main
Let mutable boxes be a new Seq of Box.
Push a new Box with tape (a new Seq of Int) to boxes.
fill({n}).
Show length of (item 1 of boxes)'s tape.\n"
        );
        let (copies, elements) = run(&source, &n.to_string());
        assert_eq!(
            (copies, elements),
            (0, 0),
            "a path through a record does not add another program owner at n={n}"
        );
    }
}

#[test]
fn a_real_field_alias_still_keeps_its_old_elements() {
    let (copies, elements) = run(
        "## A Box has:\n    a tape, which is Seq of Int.
## Main
Let b be a new Box with tape [1, 2].
Let saved be b's tape.
Push 3 to b's tape.
Show saved.
Show b's tape.\n",
        "[1, 2]\n[1, 2, 3]",
    );
    assert!(
        copies >= 1 && elements >= 2,
        "this real alias requires copying, so the work counter must observe it"
    );
}

#[test]
fn a_named_field_alias_survives_a_call_that_mutates_the_global_record() {
    let _ = run("## A Box has:\n    a tape, which is Seq of Int.\n## To fill ():\n    Push 3 to (item 1 of boxes)'s tape.\n## Main\nLet mutable boxes be a new Seq of Box.\nPush a new Box with tape [1, 2] to boxes.\nLet saved be (item 1 of boxes)'s tape.\nfill().\nShow saved.\nShow (item 1 of boxes)'s tape.\n",
        "[1, 2]\n[1, 2, 3]");
}

#[test]
fn an_earlier_argument_survives_a_later_argument_call() {
    let _ = run("## A Box has:\n    a tape, which is Seq of Int.\n## To fill () -> Int:\n    Push 3 to (item 1 of boxes)'s tape.\n    Return 0.\n## To count (xs: Seq of Int, ignored: Int) -> Int:\n    Return length of xs.\n## Main\nLet mutable boxes be a new Seq of Box.\nPush a new Box with tape [1, 2] to boxes.\nShow count((item 1 of boxes)'s tape, fill()).\nShow (item 1 of boxes)'s tape.\n",
        "2\n[1, 2, 3]");
}
