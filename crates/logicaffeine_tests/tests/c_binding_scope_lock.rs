//! A binding on the C tier means what it means on the interpreter, whatever its type.
//!
//! A LOGOS name may be bound again: a later `Let` of the same name replaces it for the statements that follow,
//! a `Receive` binds its name like a `Let`, and a binding inside a block — a branch, a loop's body, a loop's
//! pattern — lasts as long as that block. The new binding's type may differ from the old one's. C has one
//! declaration per name per block, and a declaration's type is fixed, so the C tier must give a rebinding of
//! another type a variable of its own and let the old one return when a block that shadowed it closes. A
//! counted loop reads its range once, before its variable is bound, and a body's binding of the loop
//! variable's name is the body's, never the count. A loop over a map visits its entries in the order they
//! were inserted, and one loop name binds the whole entry, `(key, value)`.
//!
//! Each program runs on the tree-walker for the reference, pinned per row, then as C.

mod common;

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_c, tw_outcome};

#[derive(Debug, PartialEq)]
struct Outcome {
    lines: Vec<String>,
    failed: bool,
}

fn reference(src: &str) -> Outcome {
    let tw = tw_outcome(src);
    Outcome {
        lines: tw.output.lines().map(str::to_string).collect(),
        failed: tw.error.is_some(),
    }
}

fn c_tier(name: &str, src: &str) -> Result<Outcome, String> {
    let c = compile_to_c(src).map_err(|e| format!("does not lower to C: {e:?}"))?;
    let dir = std::env::temp_dir().join(format!("logos_cscope_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (file, bin) = (dir.join("prog.c"), dir.join("prog"));
    std::fs::write(&file, &c).unwrap();
    let out = Command::new("gcc")
        .args(["-O1", "-o"])
        .arg(&bin)
        .arg(&file)
        .arg("-lm")
        .output()
        .unwrap();
    if !out.status.success() {
        return Err(format!(
            "does not build as C:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let run = Command::new(&bin).output().unwrap();
    Ok(Outcome {
        lines: String::from_utf8_lossy(&run.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
        failed: !run.status.success(),
    })
}

/// The rows the C tier still answers otherwise, each with its cause — each still differs, or it leaves this
/// list for the comparison it then owes.
const C_DIFFERS: &[(&str, &str)] = &[];

/// `(name, source, the reference's lines)` — each pinned, so a reference that drifts fails before any tier
/// is compared with it.
const CORPUS: &[(&str, &str, &[&str])] = &[
    (
        "a_later_binding_of_another_type_replaces_the_name",
        "## Main\nLet x be 5.\nShow x.\nLet x be \"five\".\nShow x.\n",
        &["5", "five"],
    ),
    (
        "each_binding_reads_the_one_before_it",
        "## Main\nLet x be 1.5.\nLet x be x * 2.0.\nLet x be \"{x}!\".\nShow x.\n",
        &["3!"],
    ),
    (
        "a_receive_rebinds_a_name_of_another_type",
        "## Main\nLet x be 5.\nLet ch be a Pipe of Int.\nSend 7 into ch.\nReceive x from ch.\nShow x.\n",
        &["7"],
    ),
    (
        "a_branchs_binding_of_another_type_ends_with_the_branch",
        "## Main\nLet x be 5.\nIf length of args() is greater than 0:\n    Let x be \"inner\".\n    Show x.\nShow x + 1.\n",
        &["inner", "6"],
    ),
    (
        "a_loop_variable_of_another_type_ends_with_the_loop",
        "## Main\nLet i be \"outer\".\nRepeat for i from 1 to 2:\n    Show i + 1.\nShow i.\n",
        &["2", "3", "outer"],
    ),
    (
        "a_function_binds_its_parameters_name_again",
        "## To f (n: Int) -> Text:\n    Let n be \"n\".\n    Return n.\n\n## Main\nShow f(3).\n",
        &["n"],
    ),
    (
        "a_loop_bodys_bindings_start_again_each_pass",
        "## Main\nLet mutable i be 0.\nWhile i is less than 2:\n    Let x be i.\n    Show x.\n    Let x be \"again\".\n    \
         Show x.\n    Set i to i + 1.\n",
        &["0", "again", "1", "again"],
    ),
    (
        "a_loops_range_is_read_once",
        "## Main\nLet mutable n be 3.\nRepeat for i from 1 to n:\n    Set n to 1.\n    Show i.\n",
        &["1", "2", "3"],
    ),
    (
        "a_loops_range_reads_the_name_its_variable_shadows",
        "## Main\nLet i be 2.\nRepeat for i from 1 to i + 1:\n    Show i.\n",
        &["1", "2", "3"],
    ),
    (
        "a_bodys_binding_of_the_loop_variable_leaves_the_count_alone",
        "## Main\nRepeat for i from 1 to 3:\n    Let i be i * 10.\n    Show i.\n",
        &["10", "20", "30"],
    ),
    (
        "a_bodys_binding_of_an_element_variable_is_the_bodys",
        "## Main\nLet xs be [1, 2].\nRepeat for x in xs:\n    Let x be x + 1.\n    Show x.\n",
        &["2", "3"],
    ),
    (
        "a_bodys_binding_of_an_element_variable_may_change_its_type",
        "## Main\nLet xs be [1, 2].\nRepeat for x in xs:\n    Let x be \"item {x}\".\n    Show x.\n",
        &["item 1", "item 2"],
    ),
    (
        "one_name_over_a_map_binds_each_entry",
        "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nSet item \"b\" of m to 2.\n\
         Repeat for k in m:\n    Show k.\n",
        &["(a, 1)", "(b, 2)"],
    ),
    (
        "a_map_loop_visits_entries_in_insertion_order",
        "## Main\nLet m be a new Map of Text to Int.\nSet item \"zeta\" of m to 1.\nSet item \"b\" of m to 2.\n\
         Set item \"a\" of m to 3.\nSet item \"x\" of m to 4.\nSet item \"mid\" of m to 5.\nRepeat for (k, v) in m:\n    \
         Show \"{k}={v}\".\n",
        &["zeta=1", "b=2", "a=3", "x=4", "mid=5"],
    ),
    (
        "an_int_keyed_map_loop_visits_entries_in_insertion_order",
        "## Main\nLet m be a new Map of Int to Int.\nSet item 42 of m to 1.\nSet item 7 of m to 2.\nSet item 1000 of m to 3.\n\
         Set item 3 of m to 4.\nRepeat for k in m:\n    Show k.\n",
        &["(42, 1)", "(7, 2)", "(1000, 3)", "(3, 4)"],
    ),
];

#[test]
fn the_reference_shows_what_each_row_pins() {
    for (name, src, lines) in CORPUS {
        let want = Outcome {
            lines: lines.iter().map(|l| l.to_string()).collect(),
            failed: false,
        };
        assert_eq!(
            reference(src),
            want,
            "{name}: the tree-walker no longer shows what the row pins"
        );
    }
}

#[test]
fn the_c_tier_binds_as_the_interpreter_does() {
    if !common::require_tool("gcc") {
        return;
    }
    let mut wrong = Vec::new();
    for (name, src, _) in CORPUS {
        let differs = C_DIFFERS.iter().any(|(n, _)| n == name);
        match c_tier(name, src) {
            Err(why) => wrong.push(format!("{name}: {why}")),
            Ok(got) => match (got == reference(src), differs) {
                (true, true) => {
                    wrong.push(format!("{name}: now agrees on C — take it off C_DIFFERS"))
                }
                (false, false) => wrong.push(format!(
                    "{name}:\n  interpreter {:?}\n  C           {got:?}",
                    reference(src)
                )),
                _ => {}
            },
        }
    }
    for (name, _) in C_DIFFERS {
        if !CORPUS.iter().any(|(n, _, _)| n == name) {
            wrong.push(format!(
                "{name}: listed in C_DIFFERS, but no corpus row has that name"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the C tier binds another way:\n\n{}",
        wrong.join("\n\n")
    );
}
