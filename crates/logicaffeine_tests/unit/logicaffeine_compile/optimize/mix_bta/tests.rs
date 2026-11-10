use super::*;
use crate::compile::program_to_core_tree;

fn program(src: &str) -> CoreNode {
    program_to_core_tree(src).expect("the program is in the native builder's coverage")
}

fn main_of(p: &CoreNode) -> Vec<CoreNode> {
    match ind(p) {
        Some(("CProg", a)) => items(&a[1]).to_vec(),
        _ => panic!("not a CProg"),
    }
}

fn funcs_of(p: &CoreNode) -> HashMap<String, Func> {
    let Some(("CProg", a)) = ind(p) else { panic!("not a CProg") };
    items(&a[0])
        .iter()
        .filter_map(|f| match ind(f) {
            Some(("CFuncDef", fa)) => Some((
                fa[0].clone(),
                Func {
                    params: items(&fa[1]).iter().filter_map(text).map(str::to_string).collect(),
                    types: items(&fa[2]).iter().filter_map(text).map(str::to_string).collect(),
                    ret: text(&fa[3]).unwrap_or("Nothing").to_string(),
                    body: std::rc::Rc::new(items(&fa[4]).to_vec()),
                },
            )),
            _ => None,
        })
        .map(|(n, f)| (text(&n).unwrap_or("").to_string(), f))
        .collect()
}

fn names(xs: &[&str]) -> HashSet<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_let_ends_a_variables_life_and_a_read_begins_it() {
    let main = main_of(&program("## Main\nLet x be 1.\nLet y be x + 1.\nShow y.\n"));
    assert_eq!(live_before(&main, &HashSet::new()), HashSet::new());
    assert_eq!(live_before(&main[1..], &HashSet::new()), names(&["x"]));
    assert_eq!(live_before(&main[2..], &names(&["z"])), names(&["y", "z"]));
}

#[test]
fn a_loop_head_holds_what_its_next_pass_reads() {
    let main = main_of(&program(
        "## Main\nLet mutable i be 0.\nLet mutable s be 0.\nWhile i is less than 3:\n    Set s to s + i.\n    Set i to i + 1.\nShow s.\n",
    ));
    assert_eq!(live_before(&main[2..], &HashSet::new()), names(&["i", "s"]));
}

#[test]
fn a_branch_keeps_live_what_either_side_reads() {
    let main = main_of(&program(
        "## Main\nLet a be 1.\nLet b be 2.\nIf a is greater than 0:\n    Show a.\nOtherwise:\n    Show b.\n",
    ));
    assert_eq!(live_before(&main[2..], &HashSet::new()), names(&["a", "b"]));
}

#[test]
fn a_static_loop_lifts_only_what_turned_dynamic_at_its_head() {
    let entry: Env = [("kept", Bt::S), ("turned", Bt::S), ("already", Bt::D)]
        .into_iter()
        .map(|(v, b)| (v.to_string(), b))
        .collect();
    let head: Env = [("kept", Bt::S), ("turned", Bt::D), ("already", Bt::D), ("local", Bt::D)]
        .into_iter()
        .map(|(v, b)| (v.to_string(), b))
        .collect();
    assert_eq!(turned_dynamic(&entry, &head), names(&["turned"]));
}

#[test]
fn sibling_binders_of_one_name_keep_their_own_reach() {
    let p = program(
        "## To pick (xs: Seq of Int) and (n: Int) -> Int:\n    If n is greater than 0:\n        Repeat for t in xs:\n            Return t.\n    Let t be n * 2.\n    Return t.\n\n## Main\nShow pick([1], 1).\n",
    );
    let funcs = funcs_of(&p);
    let f = &funcs["pick"];
    let shapes = catalog_shapes(crate::compile::core_types_for_pe_source());
    let mut uses = HashMap::new();
    reach_uses(&f.params, &f.body, &funcs, &shapes, &mut uses);
    let mut reaches_of_t = Vec::new();
    fn collect<'a>(n: &'a CoreNode, out: &mut Vec<&'a CoreNode>) {
        match n {
            CoreNode::Ind(c, a) => {
                if c == "CReturn" {
                    if let Some(e) = a.first() {
                        if matches!(ind(e), Some(("CVar", v)) if text(&v[0]) == Some("t")) {
                            out.push(e);
                        }
                    }
                }
                a.iter().for_each(|x| collect(x, out));
            }
            CoreNode::List(v) => v.iter().for_each(|x| collect(x, out)),
            _ => {}
        }
    }
    let mut returns = Vec::new();
    f.body.iter().for_each(|s| collect(s, &mut returns));
    for r in returns {
        reaches_of_t.push(reach_at(r, &uses));
    }
    assert_eq!(
        reaches_of_t,
        vec![Reach::Strict, Reach::Unbounded],
        "the `Repeat` binder is a part of `xs`; the later `Let t` is computed — each use sees its own binding"
    );
}

#[test]
fn a_recursion_is_one_component_and_a_plain_call_is_none() {
    let p = program(
        "## To even (n: Int) -> Bool:\n    If n equals 0:\n        Return true.\n    Return odd(n - 1).\n\n## To odd (n: Int) -> Bool:\n    If n equals 0:\n        Return false.\n    Return even(n - 1).\n\n## To twice (n: Int) -> Int:\n    Return n * 2.\n\n## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow even(4).\nShow twice(3).\nShow fact(4).\n",
    );
    let cycles = recursive_components(&funcs_of(&p));
    assert_eq!(cycles.get("even"), cycles.get("odd"), "mutual recursion is one component");
    assert!(cycles.contains_key("even") && cycles.contains_key("fact"));
    assert_ne!(cycles.get("even"), cycles.get("fact"));
    assert!(!cycles.contains_key("twice"), "a function on no cycle is in no component");
}

#[test]
fn a_parameter_every_call_threads_unchanged_is_invariant() {
    let p = program(
        "## To walk (table: Seq of Int) and (i: Int) -> Int:\n    If i is greater than (length of table):\n        Return 0.\n    Return (item i of table) + walk(table, i + 1).\n\n## Main\nLet t be [1, 2, 3].\nShow walk(t, 1).\n",
    );
    let invariants = invariant_params(&funcs_of(&p), &main_of(&p));
    assert_eq!(invariants["walk"], vec![true, false]);
}

#[test]
fn a_parameter_a_call_changes_is_not_invariant() {
    let p = program(
        "## To walk (table: Seq of Int) and (i: Int) -> Int:\n    If i is greater than 2:\n        Return 0.\n    Return walk([i], i + 1).\n\n## Main\nLet t be [1, 2, 3].\nShow walk(t, 1).\n",
    );
    let invariants = invariant_params(&funcs_of(&p), &main_of(&p));
    assert_eq!(invariants["walk"], vec![false, false]);
}

#[test]
fn the_catalog_names_each_constructors_enum() {
    let enums = catalog_enums("## A Shape is one of:\n    A Circle with r Int.\n    A Dot.\n\n## A Point has:\n    An x: Int.\n\n## A Colour is one of:\n    An Ochre.\n");
    assert_eq!(enums.get("Circle").map(String::as_str), Some("Shape"));
    assert_eq!(enums.get("Dot").map(String::as_str), Some("Shape"));
    assert_eq!(enums.get("Ochre").map(String::as_str), Some("Colour"));
    assert!(!enums.contains_key("x"), "a record's fields are not constructors");
}

/// Every `BSBlock` under `node` whose statements hold a `CReturn`, and every `BDReturn` under it.
fn returns_under(node: &CoreNode, static_returns: &mut usize, residual_returns: &mut usize) {
    match ind(node) {
        Some(("BSBlock", a)) => {
            fn holds_return(n: &CoreNode) -> bool {
                match n {
                    CoreNode::Ind(c, _) if c == "CReturn" => true,
                    CoreNode::Ind(c, _) if c == "CClosure" => false,
                    CoreNode::Ind(_, args) => args.iter().any(holds_return),
                    CoreNode::List(items) => items.iter().any(holds_return),
                    _ => false,
                }
            }
            if holds_return(&a[0]) {
                *static_returns += 1;
            }
        }
        Some(("BDReturn", _)) => *residual_returns += 1,
        _ => {}
    }
    match node {
        CoreNode::Ind(_, args) => args.iter().for_each(|a| returns_under(a, static_returns, residual_returns)),
        CoreNode::List(items) => items.iter().for_each(|a| returns_under(a, static_returns, residual_returns)),
        _ => {}
    }
}

#[test]
fn a_return_inside_a_dynamic_region_is_residual() {
    let tree = program(
        "## To pick (d: Int) and (s: Int) -> Int:\n    If d is greater than 0:\n        If s is greater than 0:\n            Return s.\n    Return d.\n\n## Main\nShow pick(d, s).\n",
    );
    let catalog = format!("{}\n{}", crate::compile::core_types_for_pe_source(), crate::compile::mix_source_text());
    let two_level = annotate(&tree, &catalog, &["s"], &["d"]).expect("the program annotates");
    let (name, pick) = two_level
        .bfuncs
        .iter()
        .find(|(n, _)| n.starts_with("pick_"))
        .expect("pick is specialized");
    let Some(("BFuncDef", parts)) = ind(pick) else { panic!("`{name}` is not a BFuncDef") };
    let mut static_returns = 0;
    let mut residual_returns = 0;
    returns_under(&parts[7], &mut static_returns, &mut residual_returns);
    assert_eq!(
        static_returns, 0,
        "`{name}` returns while specializing from inside the dynamic `If`: the residual branch would lose its `Return`"
    );
    assert_eq!(residual_returns, 2, "`{name}`'s two returns are both residual: {pick:?}");
}

#[test]
fn the_store_keys_a_text_as_the_self_interpreter_keys_it() {
    let program = format!(
        "{}\n{}\n## Main\nShow coreMapKey(a new VText with value \"fib\").\nShow coreMapKey(a new VText with value \"\").\n",
        crate::compile::core_types_for_pe_source(),
        crate::compile::core_interp_source()
    );
    let shown = crate::compile::interpret_program(&program).expect("the self-interpreter's key of a text");
    assert_eq!(shown.trim_end(), format!("{}\n{}", text_key("fib"), text_key("")), "the store and `coreMapKey` key a text alike");
}

/// Every node of `node` built by `ctor`.
fn built_by<'n>(node: &'n CoreNode, ctor: &str, found: &mut Vec<&'n CoreNode>) {
    if matches!(node, CoreNode::Ind(c, _) if c == ctor) {
        found.push(node);
    }
    match node {
        CoreNode::Ind(_, args) => args.iter().for_each(|a| built_by(a, ctor, found)),
        CoreNode::List(items) => items.iter().for_each(|a| built_by(a, ctor, found)),
        _ => {}
    }
}

#[test]
fn a_sleep_stays_in_the_residual_with_its_duration_known() {
    let tree = program("## To wait (s: Int) -> Int:\n    Sleep s.\n    Sleep 2 seconds.\n    Return s.\n\n## Main\nShow wait(s).\n");
    let catalog = format!("{}\n{}", crate::compile::core_types_for_pe_source(), crate::compile::mix_source_text());
    let two_level = annotate(&tree, &catalog, &["s"], &[]).expect("a program that sleeps annotates");
    let mut residual = Vec::new();
    for (_, f) in &two_level.bfuncs {
        built_by(f, "BDStmt", &mut residual);
    }
    let sleeps: Vec<&&CoreNode> = residual.iter().filter(|n| matches!(ind(n), Some(("BDStmt", a)) if matches!(ind(&a[0]), Some(("CSleep", _))))).collect();
    assert_eq!(sleeps.len(), 2, "each `Sleep` is a residual statement, its static duration lifted into it: {residual:?}");
}

/// The calls a line of LOGOS makes under a short-circuit `or`/`and`, its text literals set aside.
fn calls_under_short_circuits(line: &str) -> bool {
    let mut code = String::new();
    let mut in_text = false;
    for c in line.chars() {
        if c == '"' {
            in_text = !in_text;
            continue;
        }
        if !in_text {
            code.push(c);
        }
    }
    let words: Vec<&str> = code.split_whitespace().collect();
    let short_circuits = words.iter().any(|w| *w == "or" || *w == "and");
    let calls = code.char_indices().any(|(at, c)| {
        c == '(' && code[..at].chars().last().is_some_and(|before| before.is_ascii_alphanumeric())
    });
    short_circuits && calls
}

/// The offline specializer's binding-time analysis annotates no call to specialize under a short-circuit, and the
/// second and third projections specialize `mix`, which carries the self-interpreter for its static work: so the
/// self-interpreter evaluates a call before the condition that combines its answer, never inside it.
#[test]
fn the_self_interpreter_calls_nothing_under_a_short_circuit() {
    let offending: Vec<(usize, &str)> = crate::compile::core_interp_source()
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with('#') && calls_under_short_circuits(line))
        .map(|(at, line)| (at + 1, line.trim()))
        .collect();
    assert!(offending.is_empty(), "core_interp.logos calls under a short-circuit: {offending:?}");
}

#[test]
fn a_call_under_a_short_circuit_is_found_and_a_text_is_not() {
    assert!(calls_under_short_circuits("    If coreIsCounter(lv) or coreIsCounter(rv):"));
    assert!(calls_under_short_circuits("    Return (a and f(b))."));
    assert!(!calls_under_short_circuits("    If leftCounts or rightCounts:"));
    assert!(!calls_under_short_circuits("    Return a new VError with msg \"stop (f) or go\"."));
    assert!(!calls_under_short_circuits("    Let x be f(y)."));
}

/// The self-interpreter's functions that take a `mutable` parameter, by name.
fn functions_writing_an_argument(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| line.strip_prefix("## To "))
        .filter(|header| header.contains(": mutable "))
        .filter_map(|header| header.split_whitespace().next().map(str::to_string))
        .collect()
}

/// The offline specializer evaluates a returned static expression for its value: a call inside it that writes back
/// a `mutable` argument would lose the write. So the self-interpreter makes such calls as statements.
#[test]
fn the_self_interpreter_writes_no_argument_inside_a_returned_expression() {
    let source = crate::compile::core_interp_source();
    let writers = functions_writing_an_argument(source);
    assert!(writers.iter().any(|w| w == "coreWriteBack"), "the self-interpreter writes arguments back: {writers:?}");
    let offending: Vec<(usize, &str)> = source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim_start();
            line.starts_with("Return ") && writers.iter().any(|w| line.contains(&format!("{w}(")))
        })
        .map(|(at, line)| (at + 1, line.trim()))
        .collect();
    assert!(offending.is_empty(), "core_interp.logos writes an argument inside a returned expression: {offending:?}");
}

/// A loop only a `Break` under a test of its input ends.
const COUNT_UP_TO: &str = "## To count (d: Int) -> Int:\n    Let mutable i be 0.\n    While true:\n        If i is greater than d:\n            Break.\n        Set i to i + 1.\n    Return i.\n\n## Main\nShow count(d).\n";

/// A loop only a `Break` in an arm of an `Inspect` on a value its input decides ends.
const STEP_UNTIL_STOP: &str = "## A Step is one of:\n    A Go.\n    A Stop.\n\n## To next (i: Int) and (d: Int) -> Step:\n    If i is greater than d:\n        Return a new Stop.\n    Return a new Go.\n\n## To run (d: Int) -> Int:\n    Let mutable i be 0.\n    While true:\n        Inspect next(i, d):\n            When Stop:\n                Break.\n            Otherwise:\n                Set i to i + 1.\n    Return i.\n\n## Main\nShow run(d).\n";

/// A loop its input bounds, which a `Break` under a test of its static argument also ends.
const SCAN_UNLESS_STOPPED: &str = "## To scan (d: Int) and (s: Int) -> Int:\n    Let mutable n be 0.\n    While n is less than d:\n        If s is greater than 0:\n            Break.\n        Set n to n + 1.\n    Return n.\n\n## Main\nShow scan(d, s).\n";

/// A loop over a static list, which a `Break` under a test of its own variable ends, inside a branch its input decides.
const STOP_AT_TWO_IF_ASKED: &str = "## To shown (d: Int) -> Int:\n    If d is greater than 0:\n        Repeat for x in [1, 2, 3]:\n            If x is 2:\n                Break.\n            Show x.\n    Return d.\n\n## Main\nShow shown(d).\n";

/// A function that reads the clock, called beside an input.
const STAMPED: &str = "## To stamp () -> Int:\n    Return now().\n\n## Main\nShow stamp() is greater than d.\n";

/// A function that reads the clock, called with a static argument beside an input.
const LATER_BY_FIVE: &str = "## To later (by: Int) -> Int:\n    Return now() + by.\n\n## Main\nShow later(5) is greater than d.\n";

/// The `Break`s of a two-level tree: those the specializer runs (a `BSBreak`, a static block a `Break` leaves) and those
/// the residual carries (a `BDStmt` of a `CBreak`), and each one on the wrong side of the run — `residual` tells
/// whether a dynamic construct stands between `node` and the innermost loop it is in, which puts its `Break`s in the
/// residual.
#[derive(Default)]
struct Breaks {
    decided: usize,
    residual: usize,
    misplaced: Vec<String>,
}

/// Whether a Core IR block `node` holds a `Break` that leaves it, outside every loop, closure and task body in it.
fn leaves_by_break(node: &CoreNode) -> bool {
    match ind(node) {
        Some(("CBreak", _)) => true,
        Some(("CWhile" | "CRepeat" | "CRepeatRange" | "CClosure" | "CLaunchTask", _)) => false,
        Some((_, args)) => args.iter().any(leaves_by_break),
        None => items(node).iter().any(leaves_by_break),
    }
}

fn breaks_under(node: &CoreNode, residual: bool, found: &mut Breaks) {
    let nested: Option<(&[CoreNode], bool)> = match ind(node) {
        Some(("BSBreak", _)) => {
            found.decided += 1;
            if residual {
                found.misplaced.push("a `Break` the specializer runs, past a residual construct".to_string());
            }
            return;
        }
        Some(("BSBlock", a)) => {
            if leaves_by_break(&a[0]) {
                found.decided += 1;
                if residual {
                    found.misplaced.push(format!("a static block that breaks, past a residual construct: {:?}", a[0]));
                }
            }
            return;
        }
        Some(("BDStmt", a)) => {
            if matches!(a.first().and_then(ind), Some(("CBreak", _))) {
                found.residual += 1;
                if !residual {
                    found.misplaced.push("a residual `Break` of a loop the specializer unrolls".to_string());
                }
            }
            return;
        }
        Some(("BSWhile", a)) => Some((&a[1..2], false)),
        Some(("BSRepeat", a)) => Some((&a[2..3], false)),
        Some(("BSRepeatRange", a)) => Some((&a[3..4], false)),
        Some(("BDWhile", a)) => Some((&a[1..2], true)),
        Some(("BDRepeat", a)) => Some((&a[2..3], true)),
        Some(("BDRepeatRange", a)) => Some((&a[3..4], true)),
        Some(("BDIf", a)) => Some((&a[1..3], true)),
        Some(("BDInspect", a)) => Some((&a[1..2], true)),
        _ => None,
    };
    match (nested, node) {
        (Some((blocks, inside)), _) => blocks.iter().for_each(|b| breaks_under(b, inside, found)),
        (None, CoreNode::Ind(_, args)) => args.iter().for_each(|a| breaks_under(a, residual, found)),
        (None, CoreNode::List(elems)) => elems.iter().for_each(|a| breaks_under(a, residual, found)),
        _ => {}
    }
}

/// `src` annotated with `dynamic` left to the run, and the body of its one variant of the function named `function`.
fn variant_body(src: &str, statics: &[&str], dynamic: &[&str], function: &str) -> (String, CoreNode) {
    let catalog = format!("{}\n{}", crate::compile::core_types_for_pe_source(), crate::compile::mix_source_text());
    let two_level = annotate(&program(src), &catalog, statics, dynamic).expect("the program annotates");
    let prefix = format!("{function}_");
    let found: Vec<&(String, CoreNode)> = two_level.bfuncs.iter().filter(|(n, _)| n.starts_with(&prefix)).collect();
    assert_eq!(found.len(), 1, "one variant of `{function}`: {:?}", found.iter().map(|(n, _)| n).collect::<Vec<_>>());
    let (name, f) = found[0];
    let Some(("BFuncDef", parts)) = ind(f) else { panic!("`{name}` is not a BFuncDef") };
    (name.clone(), parts[7].clone())
}

fn built(node: &CoreNode, ctor: &str) -> usize {
    let mut found = Vec::new();
    built_by(node, ctor, &mut found);
    found.len()
}

#[test]
fn a_break_under_a_run_time_test_makes_its_loop_residual() {
    let (name, body) = variant_body(COUNT_UP_TO, &[], &["d"], "count");
    let mut found = Breaks::default();
    breaks_under(&body, false, &mut found);
    assert!(found.misplaced.is_empty(), "`{name}`: {:?}\n{body:?}", found.misplaced);
    assert_eq!(
        (built(&body, "BSWhile"), built(&body, "BDWhile")),
        (0, 1),
        "`{name}` unrolls a loop whose passes the run decides, and specializing it never ends: {body:?}"
    );
    assert_eq!((found.decided, found.residual), (0, 1), "`{name}`'s `Break` is the residual loop's: {body:?}");
}

#[test]
fn a_break_in_an_arm_of_a_run_time_inspect_makes_its_loop_residual() {
    let (name, body) = variant_body(STEP_UNTIL_STOP, &[], &["d"], "run");
    let mut found = Breaks::default();
    breaks_under(&body, false, &mut found);
    assert!(found.misplaced.is_empty(), "`{name}`: {:?}\n{body:?}", found.misplaced);
    assert_eq!(
        (built(&body, "BSWhile"), built(&body, "BDWhile")),
        (0, 1),
        "`{name}` unrolls a loop whose passes the run decides, and specializing it never ends: {body:?}"
    );
    assert_eq!((found.decided, found.residual), (0, 1), "`{name}`'s `Break` is the residual loop's: {body:?}");
}

#[test]
fn a_break_the_specializer_decides_inside_a_residual_loop_stays_in_it() {
    let (name, body) = variant_body(SCAN_UNLESS_STOPPED, &["s"], &["d"], "scan");
    let mut found = Breaks::default();
    breaks_under(&body, false, &mut found);
    assert!(found.misplaced.is_empty(), "`{name}`: {:?}\n{body:?}", found.misplaced);
    assert_eq!(built(&body, "BDWhile"), 1, "`{name}`'s loop is the run's: {body:?}");
    assert_eq!(
        (found.decided, found.residual),
        (0, 1),
        "`{name}`'s `Break` under its static test ends the residual loop, so the residual carries it: {body:?}"
    );
}

#[test]
fn a_break_under_static_control_unrolls_its_loop_inside_a_residual_branch() {
    let (name, body) = variant_body(STOP_AT_TWO_IF_ASKED, &[], &["d"], "shown");
    let mut found = Breaks::default();
    breaks_under(&body, false, &mut found);
    assert!(found.misplaced.is_empty(), "`{name}`: {:?}\n{body:?}", found.misplaced);
    assert_eq!(built(&body, "BDIf"), 1, "`{name}`'s branch is the run's: {body:?}");
    assert_eq!(
        (built(&body, "BSRepeat"), built(&body, "BDRepeat")),
        (1, 0),
        "`{name}`'s loop over a static list breaks under its own static test, so the specializer unrolls it: {body:?}"
    );
    assert_eq!((found.decided, found.residual), (1, 0), "`{name}`'s `Break` ends the unrolling: {body:?}");
}

#[test]
fn every_break_of_the_projections_two_level_programs_falls_on_the_side_its_control_puts_it() {
    for (label, two_level) in [
        ("the self-interpreter", crate::compile::self_interpreter_two_level()),
        ("the specializer", crate::compile::specializer_two_level()),
    ] {
        let two_level = two_level.unwrap_or_else(|e| panic!("{label} annotates: {e}"));
        let mut total = Breaks::default();
        breaks_under(&two_level.main, false, &mut total);
        for (name, f) in &two_level.bfuncs {
            let Some(("BFuncDef", parts)) = ind(f) else { panic!("`{name}` is not a BFuncDef") };
            let mut found = Breaks::default();
            breaks_under(&parts[7], false, &mut found);
            total.decided += found.decided;
            total.residual += found.residual;
            total.misplaced.extend(found.misplaced.into_iter().map(|m| format!("`{name}`: {m}")));
        }
        assert!(
            total.misplaced.is_empty(),
            "{label}: {} `Break`s fall on the wrong side of the run:\n{}",
            total.misplaced.len(),
            total.misplaced.join("\n")
        );
        assert!(total.decided + total.residual > 0, "{label} holds no `Break` to check");
    }
}

/// The run-time reads inside the static parts of a two-level tree: every expression the specializer evaluates (a
/// `BStatic`, a `BLift`, a static statement's test, collection, bounds or arguments) and every statement it runs (a
/// `BSBlock`).
fn static_reads_of_the_run(node: &CoreNode, functions: &HashSet<String>, found: &mut Vec<String>) {
    let evaluated: &[usize] = match ind(node) {
        Some(("BStatic" | "BLift" | "BSBlock" | "BSReturn", _)) => &[0],
        Some(("BSIf" | "BSWhile" | "BSInspect", _)) => &[0],
        Some(("BSRepeat", _)) => &[1],
        Some(("BSRepeatRange", _)) => &[1, 2],
        Some(("BSCallLet", _)) => &[2],
        Some(("BSCallS", _)) => &[1],
        _ => &[],
    };
    if let Some((ctor, args)) = ind(node) {
        for &i in evaluated {
            if let Some(read) = args.get(i).and_then(|a| crate::compile::run_time_input(a, functions, false)) {
                found.push(format!("`{ctor}` reads {read} while specializing: {:?}", args[i]));
            }
        }
    }
    match node {
        CoreNode::Ind(_, args) => args.iter().for_each(|a| static_reads_of_the_run(a, functions, found)),
        CoreNode::List(elems) => elems.iter().for_each(|a| static_reads_of_the_run(a, functions, found)),
        _ => {}
    }
}

#[test]
fn a_read_of_the_run_is_residual_where_the_program_has_a_dynamic_input() {
    let catalog = format!("{}\n{}", crate::compile::core_types_for_pe_source(), crate::compile::mix_source_text());
    let two_level = annotate(&program(STAMPED), &catalog, &[], &["d"]).expect("the program annotates");
    let mut calls = Vec::new();
    built_by(&two_level.main, "BDCallLet", &mut calls);
    assert!(
        calls.iter().any(|c| matches!(ind(c), Some((_, a)) if text(&a[1]).is_some_and(|f| f.starts_with("stamp")))),
        "the call of a function that reads the clock is residual: {:?}",
        two_level.main
    );
    let functions: HashSet<String> = ["stamp".to_string()].into_iter().collect();
    let mut found = Vec::new();
    static_reads_of_the_run(&two_level.main, &functions, &mut found);
    for (_, f) in &two_level.bfuncs {
        static_reads_of_the_run(f, &functions, &mut found);
    }
    assert!(found.is_empty(), "the specializer would read the clock: {found:?}");
}

/// With no dynamic input the whole run happens while specializing, and the projections refuse a program whose run
/// reads the run before they specialize it: a read stays where the rest of the run is.
#[test]
fn a_program_with_no_dynamic_input_runs_wholly_while_specializing() {
    let catalog = format!("{}\n{}", crate::compile::core_types_for_pe_source(), crate::compile::mix_source_text());
    let two_level = annotate(&program(STAMPED), &catalog, &["d"], &[]).expect("the program annotates");
    let mut calls = Vec::new();
    built_by(&two_level.main, "BDCallLet", &mut calls);
    assert!(calls.is_empty(), "a program with every input static leaves nothing residual: {:?}", two_level.main);
}

/// A call whose every argument is static stays residual when a read of the run decides its result: it is specialized on
/// those arguments, never handed them as dynamic ones to lift into the residual.
#[test]
fn a_residual_call_is_specialized_on_its_static_arguments() {
    let catalog = format!("{}\n{}", crate::compile::core_types_for_pe_source(), crate::compile::mix_source_text());
    let two_level = annotate(&program(LATER_BY_FIVE), &catalog, &[], &["d"]).expect("the program annotates");
    let mut calls = Vec::new();
    built_by(&two_level.main, "BDCallLet", &mut calls);
    let later: Vec<&&CoreNode> =
        calls.iter().filter(|c| matches!(ind(c), Some((_, a)) if text(&a[1]).is_some_and(|f| f.starts_with("later")))).collect();
    assert_eq!(later.len(), 1, "one residual call of `later`: {:?}", two_level.main);
    let Some((_, a)) = ind(later[0]) else { unreachable!() };
    assert_eq!(text(&a[1]), Some("later"), "the call keeps its static parameter: {:?}", later[0]);
    assert!(
        items(&a[2]).iter().all(|arg| matches!(ind(arg), Some(("BStatic", _)))),
        "its argument stays static: {:?}",
        later[0]
    );
}

#[test]
fn the_specializers_two_level_program_reads_the_run_only_in_its_residual() {
    let two_level = crate::compile::specializer_two_level().expect("the specializer annotates");
    let functions: HashSet<String> = two_level.funcs.iter().map(|(n, _)| n.clone()).collect();
    let mut found = Vec::new();
    static_reads_of_the_run(&two_level.main, &functions, &mut found);
    for (name, f) in &two_level.bfuncs {
        let mut here = Vec::new();
        static_reads_of_the_run(f, &functions, &mut here);
        found.extend(here.into_iter().map(|m| format!("`{name}`: {m}")));
    }
    assert!(
        found.is_empty(),
        "{} static parts of the specializer's two-level program read the run, so the compiler and the compiler \
         generator would hold the moment they were built:\n{}",
        found.len(),
        found.join("\n")
    );
}
