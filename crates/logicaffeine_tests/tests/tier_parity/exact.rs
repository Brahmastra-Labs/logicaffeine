//! The exact extension's corpus: the programs every backend that holds a value of the extension is held to
//! (`exact_on_c_lock`, `exact_on_wasm_lock`). The extension's semantic probes come from [`super::semantic`]; these
//! add the places a compiled program holds one of its values, and one program that never reaches it.

use super::Program;

/// Programs that cross every place a compiled program holds a value of the extension: a binding, a parameter and
/// a result, a field, an item and a run of items of a list, an index and a bound it gives, a loop variable, text it
/// joins, a condition it decides, a failure it reports.
pub const PROGRAMS: &[(&str, &str)] = &[
    (
        "context/a_root_and_its_square",
        "## Main\nLet x: Algebraic be sqrt(2).\nShow x.\nShow x * x.\nShow sqrt(1 / 2).\nLet y: Algebraic be sqrt(3 / 4).\nShow y.\n",
    ),
    (
        "context/the_constants",
        "## Main\nLet x: ExactReal be pi * pi + 1.\nShow x.\nLet y: ExactReal be ln(e).\nShow y.\nLet z: ExactReal be exp(1) * exp(0 - 1).\nShow z.\nShow x > 10.\nShow x < 11.\n",
    ),
    (
        "context/an_ordinal",
        "## Main\nLet w: Ordinal be omega + 1.\nShow w.\nLet v: Ordinal be 1 + omega.\nShow v.\nShow w > v.\nLet p: Ordinal be omega ** 2.\nShow p.\nShow exactOmega().\n",
    ),
    (
        "context/a_parameter_makes_its_argument_exact",
        "## To half (x: ExactReal) -> ExactReal:\n    Return x / 2.\n\n## Main\nShow half(sqrt(2)).\nShow half(pi).\nShow half(3).\n",
    ),
    (
        "boundary/a_field",
        "## A Body has:\n    a position, which is Hyperreal.\n    a label, which is Text.\n\n## Main\nLet dt be an infinitesimal.\nLet b be a new Body with position dt and label \"probe\".\nShow b's position * 2.\nShow b's label.\n",
    ),
    (
        "boundary/a_field_declared_exact_holds_a_standard_number",
        "## A Box has:\n    a v, which is Hyperreal.\n\n## Main\nLet b be a new Box with v 3.\nShow b's v / 2.\nShow b's v.\nLet e be a new Box.\nShow e's v.\n",
    ),
    (
        "boundary/text_joins_an_exact_value",
        "## Main\nLet x: Algebraic be sqrt(8).\nShow \"x = \" + x.\nShow x + \" units\".\nShow \"[{x}]\".\n",
    ),
    (
        "boundary/a_list_walked_and_grown",
        "## Main\nLet mutable xs be [sqrt(2), 1, 1 / 2].\nLet r: Algebraic be sqrt(2).\nPush r * r to xs.\nShow xs.\nLet mutable total: Algebraic be 0.\nRepeat for x in xs:\n    Set total to total + x.\nShow total.\nShow item 1 of xs.\nShow length of xs.\nShow xs contains 2.\n",
    ),
    (
        "boundary/a_run_of_a_list_and_an_exact_index",
        "## Main\nLet dt be an infinitesimal.\nLet xs be [dt, dt * 2, 3, dt + 1].\nShow items 2 through 3 of xs.\nLet ys be items 1 through 2 of xs.\nShow ys.\nShow items 3 through 9 of xs.\nShow items 3 through 2 of xs.\nLet h: Hyperreal be 2.\nLet ws be [10, 20, 30].\nShow item h of ws.\nShow items h through 3 of ws.\nRepeat for i from 1 to h:\n    Show i.\n",
    ),
    (
        "boundary/an_exact_fraction_names_no_item",
        "## Main\nLet dt be an infinitesimal.\nLet r: Rational be 1 / 2.\nLet q be standardPart(dt + r).\nShow q.\nLet w be standardPart(dt + 2).\nLet ws be [10, 20, 30].\nShow item w of ws.\nShow \"before\".\nShow item q of ws.\n",
    ),
    (
        "boundary/a_fraction_names_no_item",
        "## Main\nLet q: Hyperreal be 1 / 2.\nLet ws be [10, 20].\nShow \"before\".\nShow item q of ws.\n",
    ),
    (
        "boundary/a_fraction_bounds_no_run",
        "## Main\nLet q: Hyperreal be 3 / 2.\nLet ws be [10, 20, 30].\nShow \"before\".\nShow items 1 through q of ws.\n",
    ),
    (
        "boundary/a_copy_keeps_its_own_value",
        "## Main\nLet dt be an infinitesimal.\nLet mutable xs be [dt, 2].\nLet ys be xs.\nPush 3 to xs.\nShow xs.\nShow ys.\nSet item 1 of xs to 7.\nShow xs.\nShow ys.\n",
    ),
    (
        "boundary/a_function_called_in_a_loop",
        "## To step (x: Hyperreal) -> Hyperreal:\n    Return x + x * x.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable x: Hyperreal be dt.\nRepeat for i from 1 to 3:\n    Set x to step(x).\nShow x.\n",
    ),
    (
        "decide/a_while_over_an_exact_value",
        "## Main\nLet mutable x: Algebraic be sqrt(2).\nLet mutable n be 0.\nWhile x < 10:\n    Set x to x + sqrt(2).\n    Set n to n + 1.\nShow n.\nShow x.\n",
    ),
    (
        "decide/an_undecided_condition_stops",
        "## Main\nLet dt be an infinitesimal.\nLet n be 1 / dt.\nShow \"before\".\nIf n % 2 = 1:\n    Show \"odd\".\n",
    ),
    (
        "decide/equal_and_not_equal",
        "## Main\nLet a: Algebraic be sqrt(2) * sqrt(3).\nLet b: Algebraic be sqrt(6).\nShow a = b.\nShow a != b.\nShow a is approximately 2.449.\n",
    ),
    (
        "fail/a_negative_root",
        "## Main\nShow \"before\".\nLet x: Algebraic be sqrt(0 - 9).\nShow x.\n",
    ),
    (
        "fail/the_logarithm_of_zero",
        "## Main\nLet x: ExactReal be ln(0).\nShow x.\n",
    ),
    (
        "boundary/a_field_read_out_and_grown_keeps_the_record_as_it_was",
        "## A Bag has:\n    an items, which is Seq of Hyperreal.\n\n## Main\nLet dt be an infinitesimal.\nLet b be a new Bag with items [dt].\nLet mutable xs be b's items.\nPush dt * 2 to xs.\nShow xs.\nShow b's items.\n",
    ),
    (
        "boundary/a_loop_variable_over_machine_values_lands_exact",
        "## Main\nLet dt be an infinitesimal.\nLet mutable x be dt.\nRepeat for i in [1, 2, 3]:\n    Set x to i.\nShow x.\nShow x + dt.\n",
    ),
    (
        "boundary/an_exact_index_reads_a_machine_list",
        "## Main\nLet dt be an infinitesimal.\nLet xs be [10, 20, 30].\nLet mutable i be dt.\nIf i is infinitesimal:\n    Set i to 2.\nShow item i of xs.\nLet mutable n be dt.\nIf n is infinitesimal:\n    Set n to 3.\nRepeat for k from 1 to n:\n    Show k.\n",
    ),
    (
        "boundary/a_field_grown_in_place_leaves_a_copy_of_the_record_as_it_was",
        "## A Bag has:\n    an items, which is Seq of Hyperreal.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable b be a new Bag with items [dt].\nLet c be b.\nPush dt * 3 to b's items.\nShow b's items.\nShow c's items.\n",
    ),
    (
        "boundary/a_field_item_set_in_place_leaves_a_copy_of_the_record_as_it_was",
        "## A Bag has:\n    an items, which is Seq of Hyperreal.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable b be a new Bag with items [dt, dt].\nLet c be b.\nSet item 1 of b's items to dt * 5.\nShow b's items.\nShow c's items.\n",
    ),
    (
        "boundary/a_global_holds_its_own_copy",
        "## To show_xs:\n    Show xs.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable xs be [dt].\nLet mutable a be [dt, dt].\nSet xs to a.\nPush dt * 2 to a.\nCall show_xs.\nShow a.\n",
    ),
    (
        "boundary/a_global_stored_from_a_lent_list_keeps_its_own_copy",
        "## To keep (v: mutable Seq of Hyperreal):\n    Set xs to v.\n    Push 7 to v.\n\n## To show_xs:\n    Show xs.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable xs be [dt].\nLet mutable a be [dt, dt].\nCall keep with a.\nCall show_xs.\nShow a.\n",
    ),
    (
        "boundary/a_tuple_among_exact_values_stays_a_tuple",
        "## Main\nLet dt be an infinitesimal.\nLet mutable xs be [(1, 2)].\nPush dt to xs.\nShow xs.\nShow item 1 of xs.\n",
    ),
    (
        "boundary/a_declared_int_parameter_joins_text_as_its_exact_value",
        "## To describe (x: Int) -> Text:\n    Return \"x is \" + x.\n\n## Main\nLet dt be an infinitesimal.\nShow describe(dt).\nShow describe(4).\n",
    ),
    (
        "boundary/a_declared_list_parameter_joins_text_as_its_exact_value",
        "## To describe (xs: Seq of Real) -> Text:\n    Return \"xs is \" + xs.\n\n## Main\nLet dt be an infinitesimal.\nShow describe([dt, 2.5]).\nShow describe([1.5, 2.5]).\n",
    ),
    (
        "boundary/a_mutable_parameter_hands_its_exact_list_back",
        "## To grow (v: mutable Seq of Hyperreal):\n    Push 7 to v.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable a be [dt].\nCall grow with a.\nShow a.\nCall grow with a.\nShow a.\n",
    ),
    (
        "boundary/a_tuple_holds_an_exact_value",
        "## Main\nLet dt be an infinitesimal.\nLet p be (dt, \"x\").\nShow p.\nShow item 1 of p * 2.\nShow item 2 of p.\n",
    ),
    (
        "boundary/a_map_holds_exact_values",
        "## Main\nLet dt be an infinitesimal.\nLet mutable m be a new Map of Text to Hyperreal.\nSet item \"a\" of m to dt.\nSet item \"b\" of m to 2.\nShow m.\nShow item \"a\" of m * 3.\nShow item \"b\" of m + dt.\nShow m contains \"a\".\n",
    ),
    (
        "boundary/a_map_keyed_by_int_holds_exact_values",
        "## Main\nLet dt be an infinitesimal.\nLet mutable m be a new Map of Int to Hyperreal.\nSet item 1 of m to dt.\nSet item 2 of m to 5.\nShow m.\nShow item 1 of m * 3.\nShow item 2 of m + dt.\nShow m contains 2.\n",
    ),
    (
        "boundary/a_list_stored_in_a_map_keeps_its_own_copy",
        "## Main\nLet dt be an infinitesimal.\nLet mutable m be a new Map of Text to Seq of Hyperreal.\nLet mutable xs be [dt].\nSet item \"a\" of m to xs.\nPush 2 to xs.\nShow xs.\nShow item \"a\" of m.\nLet mutable ys be item \"a\" of m.\nPush 3 to ys.\nShow ys.\nShow m.\n",
    ),
    (
        "boundary/a_global_a_function_makes_exact_holds_the_exact_value",
        "## To make_exact:\n    Let dt be an infinitesimal.\n    Set x to dt.\n\n## To show_x:\n    Show x + 1.\n\n## Main\nLet mutable x be 3.\nCall show_x.\nCall make_exact.\nCall show_x.\n",
    ),
    (
        "boundary/a_global_holding_an_exact_value_takes_a_standard_number_from_a_function",
        "## To reset:\n    Set x to 3.\n\n## To show_x:\n    Show x + 1.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable x be dt.\nCall show_x.\nCall reset.\nCall show_x.\n",
    ),
    (
        "boundary/a_deep_copy_of_an_exact_list_is_its_own",
        "## Main\nLet dt be an infinitesimal.\nLet mutable xs be [dt, 2].\nLet mutable ys be copy of xs.\nPush 3 to ys.\nShow xs.\nShow ys.\n",
    ),
    (
        "churn/a_thousand_exact_values",
        "## Main\nLet r: Algebraic be sqrt(2).\nLet mutable total: Algebraic be 0.\nRepeat for i from 1 to 1000:\n    Set total to total + r * i.\nShow total.\n",
    ),
];

/// A program that never names the extension: a backend links and imports nothing of the value runtime for it.
pub const PLAIN: &str = "## Main\nLet xs be [3, 1, 2].\nLet mutable total be 0.\nRepeat for x in xs:\n    Set total to total + x.\nShow total.\nShow 7 / 2.\nLet r: Rational be 1 / 3.\nShow r.\n";

/// The extension's semantic probes, which every backend runs alike.
pub fn probes() -> Vec<Program> {
    super::semantic::SEMANTIC_PROBES
        .iter()
        .filter(|(name, _)| name.starts_with("nonstandard/") || name.starts_with("supertask/"))
        .map(|(name, source)| Program::new(format!("semantic/{name}"), *source))
        .collect()
}

/// [`PROGRAMS`] as programs of the harness.
pub fn programs() -> Vec<Program> {
    PROGRAMS
        .iter()
        .map(|(id, source)| Program::new(format!("exact/{id}"), *source))
        .collect()
}
