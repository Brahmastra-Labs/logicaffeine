//! LOCK — the first projection of a program that reads its input when it runs.
//!
//! A first projection is a compiler's output: `[[mix(sint, p)]](d) = [[p]](d)` for every input `d`, so the input stays
//! dynamic. The self-interpreter (`core_interp.logos`) receives the program's arguments from `coreRun(main, funcs,
//! args())`, and the specializer meets them as a run-time `args()`: every read of them, every value computed from them,
//! every branch and loop they decide, and every list whose length they set remains for the residual to run.
//!
//! §1 The residual reads the arguments when it runs. For every program and every argument vector here, the textbook
//! first projection prints what the program prints and ends as the program ends — the same lines, the same report in
//! the language's words — and carries no interpreter dispatch.
//!
//! §2 The residual is as fast as the program. Jones-optimal on every input: the residual takes no more steps than the
//! program on the same arguments, counted twice — statements on the tree-walker, instructions on the register VM. The
//! interpreter's own work (dispatch, environment, value boxes, argument intake, index and division checks the host makes
//! anyway, the loops it runs on the program's behalf) leaves nothing behind. Where the self-interpreter's
//! representation still costs steps, `KNOWN_EXCESS` names the program, the bound and the reason, and the lock fails
//! once the excess is gone so the entry is dropped.
//!
//! §3 Generated programs. Seeded programs over one argument — assignments, arithmetic that may overflow, branches,
//! counted loops with conditional breaks, a helper and a recursion — each on several inputs, hold §1 and §2.
//!
//! §4 Generated programs that fail. Seeded programs over one argument that divide, take a modulo and floor-divide by
//! expressions of it, read and write a list whose length it sets at indexes it moves out of range, and sometimes guard
//! a division or an index themselves, each on inputs that zero a divisor or miss an index, hold §1 and §2: the residual
//! stops where the program stops, with the program's report, and the self-interpreter's own checks leave nothing.

mod common;

use logicaffeine_compile::compile;

/// One program and the argument vectors it runs on (after the program's name).
struct Case {
    name: &'static str,
    program: &'static str,
    inputs: &'static [&'static [&'static str]],
}

const fn case(name: &'static str, program: &'static str, inputs: &'static [&'static [&'static str]]) -> Case {
    Case { name, program, inputs }
}

const ARGUMENTS_CORPUS: &[Case] = &[
    case("a_parsed_argument_doubled", "## Main\nLet n be parseInt(item 2 of args()).\nShow n * 2.\n", &[&["7"], &["0"], &["-3"]]),
    case(
        "a_sum_up_to_an_argument",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable total be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.\n",
        &[&["0"], &["7"], &["300"]],
    ),
    case(
        "the_collatz_steps_of_an_argument",
        "## Main\nLet mutable n be parseInt(item 2 of args()).\nLet mutable steps be 0.\nWhile n is greater than 1:\n    If n % 2 equals 0:\n        Set n to n / 2.\n    Otherwise:\n        Set n to 3 * n + 1.\n    Set steps to steps + 1.\nShow steps.\n",
        &[&["1"], &["7"], &["27"]],
    ),
    case(
        "a_nested_sum_up_to_an_argument",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable total be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    Let mutable j be 1.\n    While j is at most i:\n        Set total to total + i * j.\n        Set j to j + 1.\n    Set i to i + 1.\nShow total.\n",
        &[&["0"], &["4"], &["40"]],
    ),
    case(
        "a_break_on_an_argument",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable i be 0.\nWhile true:\n    Set i to i + 1.\n    If i * i is greater than n:\n        Break.\nShow i.\n",
        &[&["0"], &["10"], &["10000"]],
    ),
    case(
        "a_function_of_an_argument",
        "## To twice (k: Int) -> Int:\n    Return k * 2.\n\n## Main\nLet a be twice(parseInt(item 2 of args())).\nShow twice(a) + 1.\n",
        &[&["7"], &["-1"]],
    ),
    case(
        "a_recursion_on_an_argument",
        "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(parseInt(item 2 of args())).\n",
        &[&["0"], &["7"], &["20"]],
    ),
    case(
        "a_tree_recursion_on_an_argument",
        "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(parseInt(item 2 of args())).\n",
        &[&["0"], &["10"], &["18"]],
    ),
    case(
        "a_recursion_inside_a_range_loop",
        "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to n:\n    Push fib(i) to xs.\nShow xs.\n",
        &[&["0"], &["7"], &["15"]],
    ),
    case(
        "a_list_grown_to_an_argument",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nLet mutable i be 1.\nWhile i is at most n:\n    Push i * i to xs.\n    Set i to i + 1.\nShow length of xs.\nShow xs.\n",
        &[&["0"], &["7"], &["200"]],
    ),
    case(
        "a_list_summed_by_a_function",
        "## To total (xs: Seq of Int) -> Int:\n    Let mutable s be 0.\n    Repeat for x in xs:\n        Set s to s + x.\n    Return s.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to n:\n    Push i * 3 to xs.\nShow total(xs).\n",
        &[&["0"], &["6"], &["100"]],
    ),
    case(
        "a_list_read_at_a_moving_index",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to n:\n    Push i * 2 to xs.\nLet mutable s be 0.\nLet mutable k be 1.\nWhile k is at most n:\n    Set s to s + item k of xs.\n    Set k to k + 1.\nShow s.\n",
        &[&["0"], &["7"], &["90"]],
    ),
    case(
        "a_division_by_an_argument",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet d be parseInt(item 3 of args()).\nLet mutable s be 0.\nLet mutable k be 1.\nWhile k is at most n:\n    Set s to s + k / d + k % d.\n    Set k to k + 1.\nShow s.\n",
        &[&["7", "3"], &["30", "7"], &["4", "0"]],
    ),
    case(
        "a_text_grown_to_an_argument",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable t be \"\".\nLet mutable i be 0.\nWhile i is less than n:\n    Set t to t + \"ab\".\n    Set i to i + 1.\nShow t.\nShow length of t.\n",
        &[&["0"], &["5"]],
    ),
    case("the_arguments_concatenated", "## Main\nLet a be item 2 of args().\nLet b be item 3 of args().\nShow a + b.\n", &[&["ab", "cd"], &["", "x"]]),
    case("the_arguments_walked", "## Main\nShow length of args().\nRepeat for a in args():\n    Show a.\n", &[&[], &["x"], &["x", "y", "z"]]),
    case(
        "a_copy_read_after_a_branch_writes_the_original",
        "## Main\nLet mutable n be parseInt(item 2 of args()).\nLet m be n.\nIf n % 2 equals 0:\n    Set n to n * 3.\nOtherwise:\n    Set n to n + 1.\nShow m.\nShow n.\n",
        &[&["4"], &["7"]],
    ),
    case(
        "a_failure_after_an_earlier_show",
        "## Main\nLet d be parseInt(item 2 of args()).\nLet a be 10 / d.\nLet b be 5.\nShow b.\nShow a.\n",
        &[&["2"], &["0"]],
    ),
    case(
        "a_copy_of_the_arguments_grown",
        "## Main\nLet mutable xs be args().\nPush \"q\" to xs.\nShow xs.\nShow args().\nShow length of args().\n",
        &[&["a", "b"], &[]],
    ),
    case(
        "lists_of_every_scalar_kind",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable ts be a new Seq of Text.\nLet mutable rs be a new Seq of Real.\nLet mutable bs be a new Seq of Bool.\nLet mutable es be a new Seq of Int.\nRepeat for i from 1 to n:\n    Push \"t{i}\" to ts.\n    Push i / 2.0 to rs.\n    Push i % 2 equals 0 to bs.\nShow ts.\nShow rs.\nShow bs.\nShow es.\n",
        &[&["4"], &["0"]],
    ),
    case(
        "a_list_of_lists",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xss be a new Seq of Seq of Int.\nRepeat for i from 1 to n:\n    Let mutable row be a new Seq of Int.\n    Repeat for j from 1 to i:\n        Push j to row.\n    Push row to xss.\nShow xss.\n",
        &[&["3"], &["0"]],
    ),
    case(
        "a_break_before_the_body_finishes",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable i be 0.\nLet mutable s be 0.\nWhile i is less than n:\n    Set i to i + 1.\n    If i % 5 equals 0:\n        Break.\n    Set s to s + i.\nShow i.\nShow s.\n",
        &[&["3"], &["12"]],
    ),
    case(
        "a_list_grown_while_it_is_walked",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to n:\n    Push i to xs.\nRepeat for x in xs:\n    Push x * 10 to xs.\nShow xs.\n",
        &[&["3"], &["0"]],
    ),
    case(
        "an_index_the_arguments_choose",
        "## Main\nLet k be parseInt(item 2 of args()).\nLet xs be [10, 20, 30].\nShow item k of xs.\n",
        &[&["2"], &["-1"], &["5"], &["0"]],
    ),
    case(
        "a_list_and_its_alias_apart",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to n:\n    Push i to xs.\nLet ys be xs.\nPush 99 to xs.\nShow ys.\nShow xs.\n",
        &[&["3"], &["0"]],
    ),
    case(
        "a_range_variable_written_in_its_body",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable t be 0.\nRepeat for i from 1 to n:\n    Set t to t + i.\n    Set i to i * 100.\nShow t.\n",
        &[&["5"], &["0"]],
    ),
    case(
        "a_call_inside_a_range_loop",
        "## To pick (k: Int) -> Int:\n    If k is greater than 3:\n        Return k * 2.\n    Return k - 1.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable s be 0.\nRepeat for i from 1 to n:\n    Set s to s + pick(i).\nShow s.\n",
        &[&["6"], &["0"]],
    ),
    case(
        "a_recursive_procedure_writing_through_a_mutable_parameter",
        "## To bumpTimes (x: mutable Int) and (k: Int):\n    If k is at most 0:\n        Return.\n    Set x to x + 1.\n    bumpTimes(x, k - 1).\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable x be n * 10.\nLet a be x.\nbumpTimes(x, n).\nShow a.\nShow x.\n",
        &[&["3"], &["0"]],
    ),
    case(
        "a_procedure_writing_its_mutable_parameter_once",
        "## To clampUp (x: mutable Int) and (lo: Int):\n    If x is less than lo:\n        Set x to lo.\n        Return.\n    Set x to x + 1.\n\n## Main\nLet mutable v be parseInt(item 2 of args()).\nclampUp(v, 5).\nShow v.\n",
        &[&["2"], &["9"]],
    ),
    case(
        "a_function_writing_its_mutable_parameter_and_returning",
        "## To bump (x: mutable Int) -> Int:\n    Set x to x + 1.\n    Return x * 2.\n\n## Main\nLet mutable x be parseInt(item 2 of args()).\nLet r be bump(x).\nShow r.\nShow x.\n",
        &[&["4"], &["-3"]],
    ),
    case(
        "a_recursive_procedure_that_shows",
        "## To countdown (k: Int):\n    If k is at most 0:\n        Show \"liftoff\".\n        Return.\n    Show k.\n    countdown(k - 1).\n\n## Main\ncountdown(parseInt(item 2 of args())).\n",
        &[&["0"], &["3"]],
    ),
    case(
        "two_mutable_parameters_written_by_a_recursion",
        "## To step (a: mutable Int) and (b: mutable Int) and (k: Int):\n    If k is at most 0:\n        Return.\n    Set a to a + b.\n    Set b to a - b.\n    step(a, b, k - 1).\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable a be 1.\nLet mutable b be 0.\nstep(a, b, n).\nShow a.\nShow b.\n",
        &[&["0"], &["10"], &["40"]],
    ),
    case(
        "a_list_filled_through_a_mutable_parameter",
        "## To fill (xs: mutable Seq of Int) and (k: Int):\n    If k is at most 0:\n        Return.\n    Push k to xs.\n    fill(xs, k - 1).\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nfill(xs, n).\nShow length of xs.\nShow xs.\n",
        &[&["0"], &["5"]],
    ),
    case(
        "a_return_from_inside_a_loop_over_a_list",
        "## To firstOver (xs: Seq of Int) and (t: Int) -> Int:\n    Repeat for x in xs:\n        If x is greater than t:\n            Return x.\n    Return -1.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to n:\n    Push i * i to xs.\nShow firstOver(xs, 10).\nShow firstOver(xs, 1000000).\n",
        &[&["0"], &["5"]],
    ),
    case(
        "a_test_on_a_recursive_calls_result",
        "## To halvings (n: Int) -> Int:\n    If n is at most 1:\n        Return 0.\n    Let r be halvings(n / 2).\n    If r is greater than 3:\n        Return r * 10.\n    Return r + 1.\n\n## Main\nShow halvings(parseInt(item 2 of args())).\n",
        &[&["1"], &["9"], &["100000"]],
    ),
    case(
        "a_mutual_recursion_entered_at_the_even_side",
        "## To isEven (n: Int) -> Bool:\n    If n equals 0:\n        Return true.\n    Return isOdd(n - 1).\n\n## To isOdd (n: Int) -> Bool:\n    If n equals 0:\n        Return false.\n    Return isEven(n - 1).\n\n## Main\nLet n be parseInt(item 2 of args()).\nShow isEven(n).\nShow isOdd(n).\n",
        &[&["0"], &["7"]],
    ),
    case(
        "a_mutual_recursion_entered_at_the_odd_side",
        "## To isEven (n: Int) -> Bool:\n    If n equals 0:\n        Return true.\n    Return isOdd(n - 1).\n\n## To isOdd (n: Int) -> Bool:\n    If n equals 0:\n        Return false.\n    Return isEven(n - 1).\n\n## Main\nLet n be parseInt(item 2 of args()).\nShow isOdd(n).\nShow isEven(n).\n",
        &[&["0"], &["7"]],
    ),
    case(
        "a_mutual_recursion_entered_twice_at_one_side",
        "## To isEven (n: Int) -> Bool:\n    If n equals 0:\n        Return true.\n    Return isOdd(n - 1).\n\n## To isOdd (n: Int) -> Bool:\n    If n equals 0:\n        Return false.\n    Return isEven(n - 1).\n\n## Main\nLet n be parseInt(item 2 of args()).\nShow isEven(n).\nShow isEven(n + 1).\n",
        &[&["0"], &["7"]],
    ),
    case(
        "a_floor_division_a_modulo_and_a_power_by_an_argument",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet d be parseInt(item 3 of args()).\nLet mutable s be 0.\nLet mutable k be 1.\nWhile k is at most n:\n    Set s to s + k // d + k % d + k ** d.\n    Set k to k + 1.\nShow s.\n",
        &[&["5", "2"], &["5", "0"], &["5", "-1"], &["0", "0"]],
    ),
    case(
        "a_real_divided_by_a_counter",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable s be 0.0.\nLet mutable k be 1.\nWhile k is at most n:\n    Set s to s + 1.5 / k.\n    Set k to k + 1.\nShow s.\n",
        &[&["5"], &["0"]],
    ),
    case(
        "the_characters_of_an_argument_read_by_index",
        "## Main\nLet t be item 2 of args().\nLet n be parseInt(item 3 of args()).\nLet mutable k be 1.\nLet mutable out be \"\".\nWhile k is at most n:\n    Set out to out + item k of t.\n    Set k to k + 1.\nShow out.\n",
        &[&["hello", "5"], &["hello", "6"], &["héllo", "5"], &["héllo", "6"]],
    ),
    case(
        "a_list_written_at_a_counter",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to n:\n    Push 0 to xs.\nLet mutable k be 1.\nWhile k is at most n:\n    Set item k of xs to k * k.\n    Set k to k + 1.\nShow xs.\n",
        &[&["4"], &["0"]],
    ),
    case(
        "a_guard_before_dividing_a_duration",
        "## Main\nLet d be parseInt(item 2 of args()).\nLet t be 3 seconds.\nIf d equals 0:\n    Fail with \"Division by zero\".\nShow t / d.\n",
        &[&["0"], &["2"]],
    ),
    case(
        "a_guard_before_dividing_a_text",
        "## Main\nLet d be parseInt(item 2 of args()).\nLet t be \"abc\".\nIf d equals 0:\n    Fail with \"Division by zero\".\nShow t / d.\n",
        &[&["0"], &["2"]],
    ),
    case(
        "a_guard_before_a_real_modulo",
        "## Main\nLet d be parseInt(item 2 of args()).\nLet x be 7.5.\nIf d equals 0:\n    Fail with \"Modulo by zero\".\nShow x % d.\n",
        &[&["0"], &["2"]],
    ),
    case(
        "a_guard_narrower_than_its_list",
        "## Main\nLet i be parseInt(item 2 of args()).\nLet xs be [10, 20, 30, 40, 50].\nIf i is greater than 2:\n    Fail with \"Index {i} out of bounds\".\nShow item i of xs.\n",
        &[&["2"], &["3"], &["6"], &["-1"]],
    ),
    case(
        "a_guard_before_a_short_circuit_division",
        "## Main\nLet d be parseInt(item 2 of args()).\nLet flag be d equals 0.\nIf d equals 0:\n    Fail with \"Division by zero\".\nIf flag or (10 / d is greater than 1):\n    Show \"yes\".\n",
        &[&["0"], &["5"]],
    ),
    case(
        "a_guard_before_an_overflow_and_a_division",
        "## Main\nLet d be parseInt(item 2 of args()).\nLet a be 9223372036854775807.\nLet b be parseInt(item 3 of args()).\nIf d equals 0:\n    Fail with \"Division by zero\".\nShow (a + b) + (10 / d).\n",
        &[&["0", "1"], &["2", "1"], &["2", "-5"]],
    ),
    case(
        "a_guard_before_a_map_read",
        "## Main\nLet k be parseInt(item 2 of args()).\nLet mutable m be a new Map of Int to Int.\nSet item 7 of m to 5.\nSet item 1 of m to 6.\nIf (k equals 0) or (k is greater than (length of m)) or (k is less than (0 - (length of m))):\n    Fail with \"Index {k} out of bounds\".\nShow item k of m.\n",
        &[&["7"], &["1"], &["2"]],
    ),
    case(
        "a_guard_before_a_short_circuit_and_division",
        "## Main\nLet d be parseInt(item 2 of args()).\nLet flag be d is not 0.\nIf d equals 0:\n    Fail with \"Division by zero\".\nIf flag and (10 / d is greater than 1):\n    Show \"yes\".\nShow \"done\".\n",
        &[&["0"], &["5"]],
    ),
    case(
        "a_short_circuit_or_whose_right_side_divides",
        "## Main\nLet d be parseInt(item 2 of args()).\nLet flag be d is less than 0.\nIf flag or (10 / d is greater than 1):\n    Show \"yes\".\nShow \"done\".\n",
        &[&["5"], &["0"], &["-3"]],
    ),
    case(
        "a_literal_list_written_under_a_run_time_branch",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be [1, 2, 3, 4, 5].\nIf n is greater than 2:\n    Set item 2 of xs to 9.\nShow xs.\n",
        &[&["4"], &["1"]],
    ),
    case(
        "a_literal_list_written_in_a_run_time_loop",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be [1, 2, 3, 4, 5].\nRepeat for j from 1 to n:\n    Set item 2 of xs to j.\nShow xs.\n",
        &[&["0"], &["4"]],
    ),
    case(
        "a_literal_list_written_under_a_branch_in_a_loop",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be [1, 2, 3, 4, 5].\nRepeat for j from 1 to n:\n    If j is greater than 2:\n        Set item 2 of xs to j.\nShow xs.\n",
        &[&["0"], &["4"]],
    ),
    case(
        "a_literal_list_written_on_both_branches",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be [1, 2, 3].\nIf n is greater than 2:\n    Set item 2 of xs to 9.\nOtherwise:\n    Set item 2 of xs to 7.\nShow xs.\n",
        &[&["4"], &["1"]],
    ),
    case(
        "one_item_read_from_a_list_written_under_a_branch",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be [1, 2, 3].\nIf n is greater than 2:\n    Set item 2 of xs to 9.\nShow item 2 of xs.\n",
        &[&["4"], &["1"]],
    ),
    case(
        "a_list_written_under_a_branch_then_measured",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be [1, 2, 3].\nIf n is greater than 2:\n    Set item 2 of xs to 9.\nShow length of xs.\nShow xs.\n",
        &[&["4"], &["1"]],
    ),
    case(
        "a_record_field_written_under_a_branch",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable p be a new Point with x 1 and y 2.\nIf n is greater than 2:\n    Set p's y to 9.\nShow p's x.\nShow p's y.\n",
        &[&["1"], &["3"]],
    ),
    case(
        "a_record_field_written_in_a_loop",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable p be a new Point with x 1 and y 2.\nRepeat for j from 1 to n:\n    Set p's y to p's y + j.\nShow p's x.\nShow p's y.\n",
        &[&["0"], &["3"], &["7"]],
    ),
    case(
        "a_variant_replaced_under_a_branch",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable s be a new Circle with radius 1.\nIf n is greater than 2:\n    Set s to a new Circle with radius n.\nInspect s:\n    When Circle (r):\n        Show r.\n    When Square (d):\n        Show d.\n",
        &[&["1"], &["3"]],
    ),
    case(
        "a_tuple_replaced_under_a_branch",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable t be (1, 2, 3).\nIf n is greater than 2:\n    Set t to (1, n, 3).\nShow item 1 of t.\nShow item 2 of t.\n",
        &[&["1"], &["3"]],
    ),
    case(
        "a_pushed_list_written_under_a_branch",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nPush 1 to xs.\nPush 2 to xs.\nIf n is greater than 2:\n    Set item 1 of xs to n.\nShow xs.\n",
        &[&["1"], &["3"]],
    ),
    case(
        "a_map_entry_written_under_a_branch",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nIf n is greater than 2:\n    Set item \"a\" of m to n.\nShow item \"a\" of m.\n",
        &[&["1"], &["3"]],
    ),
    case(
        "a_list_of_texts_written_in_a_loop",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable names be [\"ann\", \"bob\", \"cy\"].\nRepeat for j from 1 to n:\n    If j is greater than 1:\n        Set item 3 of names to \"j{j}\".\nShow names.\n",
        &[&["0"], &["1"], &["3"]],
    ),
    case(
        "a_list_of_lists_with_a_row_replaced",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be [[1, 2], [3, 4]].\nIf n is greater than 2:\n    Set item 2 of xs to [n, n].\nShow length of xs.\nShow xs.\n",
        &[&["1"], &["3"]],
    ),
    case(
        "an_index_the_guard_refuses_read_under_a_run_time_branch",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nPush n to xs.\nLet a be 0.\nIf n is greater than 2:\n    Show item a of xs.\nShow n.\n",
        &[&["1"], &["3"]],
    ),
    case(
        "an_index_the_guard_refuses_written_under_a_run_time_branch",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable xs be a new Seq of Int.\nPush n to xs.\nLet a be 0.\nIf n is greater than 2:\n    Set item a of xs to 5.\n    Show xs.\nOtherwise:\n    Show \"small\".\nShow n.\n",
        &[&["1"], &["3"]],
    ),
    case(
        "an_if_whose_branches_are_dead_but_whose_test_divides",
        "## Main\nLet n be parseInt(item 2 of args()).\nIf (5 / n) is greater than 1:\n    Let unused be 1.\nShow \"after\".\n",
        &[&["0"], &["2"], &["abc"]],
    ),
    case(
        "a_loop_whose_body_is_dead_but_whose_bound_divides",
        "## Main\nLet n be parseInt(item 2 of args()).\nRepeat for i from 1 to (5 / n):\n    Let unused be i.\nShow \"after\".\n",
        &[&["0"], &["2"], &["abc"]],
    ),
    case(
        "a_short_circuit_and_whose_right_call_always_fails",
        "## To bad (k: Int) -> Int:\n    Return k / 0.\n\n## Main\nLet n be parseInt(item 2 of args()).\nIf n is greater than 5 and bad(n) is greater than 0:\n    Show \"big\".\nShow \"done\".\n",
        &[&["1"], &["9"]],
    ),
    case(
        "a_short_circuit_or_whose_right_call_always_fails",
        "## To bad (k: Int) -> Int:\n    Return k / 0.\n\n## Main\nLet n be parseInt(item 2 of args()).\nIf n is less than 5 or bad(n) is greater than 0:\n    Show \"small\".\nShow \"done\".\n",
        &[&["1"], &["9"]],
    ),
    case(
        "a_short_circuit_whose_right_call_fails_on_a_known_argument",
        "## To bad (k: Int) -> Int:\n    Return k / 0.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet z be 0.\nIf n is greater than 5 and bad(z) is greater than 0:\n    Show \"big\".\nShow \"done\".\n",
        &[&["1"], &["9"]],
    ),
    case(
        "a_variant_whose_tag_the_run_decides",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Dot.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable s be a new Dot.\nIf n is greater than 2:\n    Set s to a new Circle with radius (n + 1).\nInspect s:\n    When Circle (r):\n        Show \"circle\".\n    When Dot:\n        Show \"dot\".\n",
        &[&["1"], &["5"]],
    ),
    case(
        "a_push_in_the_arms_of_a_variant_whose_tag_the_run_decides",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Dot.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable s be a new Dot.\nIf n is greater than 2:\n    Set s to a new Circle with radius n.\nLet mutable xs be [1].\nInspect s:\n    When Circle (r):\n        Push r to xs.\n    When Dot:\n        Push 0 to xs.\nShow xs.\n",
        &[&["1"], &["5"]],
    ),
    case(
        "a_variant_set_under_a_run_time_branch_and_never_read",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Dot.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable s be a new Dot.\nIf n is greater than 2:\n    Set s to a new Circle with radius (n + 1).\nShow \"done\".\n",
        &[&["1"], &["5"]],
    ),
    case(
        "a_variant_of_three_tags_set_under_two_run_time_branches",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n    A Dot.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable s be a new Dot.\nIf n is greater than 2:\n    Set s to a new Circle with radius n.\nIf n is greater than 6:\n    Set s to a new Square with side (n * 2).\nInspect s:\n    When Circle (r):\n        Show \"circle {r}\".\n    When Square (d):\n        Show \"square {d}\".\n    When Dot:\n        Show \"dot\".\nShow s.\n",
        &[&["1"], &["4"], &["9"]],
    ),
    case(
        "a_variant_its_arms_replace_then_inspected_again",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Dot.\n\n## Main\nLet n be parseInt(item 2 of args()).\nLet mutable s be a new Dot.\nIf n is greater than 2:\n    Set s to a new Circle with radius n.\nInspect s:\n    When Circle (r):\n        Set s to a new Circle with radius (r + 1).\n    When Dot:\n        Set s to a new Circle with radius 0.\nInspect s:\n    When Circle (q):\n        Show q.\n    When Dot:\n        Show \"dot\".\n",
        &[&["1"], &["5"]],
    ),
];

const ARGUMENT_PROGRAMS: usize = 77;

/// Programs whose residual still costs steps the program does not take, the most it may cost (tree-walker statements,
/// VM instructions) on any input of the corpus, and why the self-interpreter's representation costs it.
const KNOWN_EXCESS: &[(&str, u64, u64, &str)] = &[
    ("a_guard_before_a_map_read", 37, 61, "a map read at a run-time key keeps the self-interpreter's own map: its key text and its pair, no host map, and its generic display"),
    ("a_list_of_lists_with_a_row_replaced", 0, 1, "the VM moves the row's variable into the list literal's register run, where the program loaded its constants in place"),
    ("a_list_of_texts_written_in_a_loop", 0, 2, "a list of texts with a decided item is shown as its spelled-out text (a hole cannot hold its quotes), one concatenation more than the program's own display when the loop runs at most once"),
    ("a_loop_whose_body_is_dead_but_whose_bound_divides", 0, 1, "the residual holds the dead loop's bound in a binding, one VM store where the program's loop header read it"),
    ("a_push_in_the_arms_of_a_variant_whose_tag_the_run_decides", 0, 5, "the residual tests the sum's constructors in the order the merge met them, the program its arms in its own order"),
    ("a_return_from_inside_a_loop_over_a_list", 0, 6, "the residual specializes the function once per threshold the program passes, and the VM reloads the folded threshold in each iteration where the program reads its parameter's register"),
    ("a_test_on_a_recursive_calls_result", 0, 1, "the recursion's first level is unrolled into the main block, which binds the parsed argument once: one VM instruction the program's call spends passing it"),
    ("a_variant_its_arms_replace_then_inspected_again", 1, 6, "the residual's sum value is rebuilt in the arms that replace it and tested in the merge's order"),
    ("a_variant_of_three_tags_set_under_two_run_time_branches", 1, 16, "the residual shows the variant by its constructor's spelled-out text where the program's VM shows its own variant in one instruction, and tests constructors in the merge's order"),
    ("a_variant_whose_tag_the_run_decides", 0, 2, "the residual tests the sum's constructors in the order the merge met them, the program its arms in its own order"),
    ("an_if_whose_branches_are_dead_but_whose_test_divides", 0, 1, "the residual holds the dead branch's test in a binding, one VM store where the program jumped on it"),
];

/// Programs whose residual still carries interpreter dispatch, the exact count, and why.
const KNOWN_DISPATCH: &[(&str, usize, &str)] = &[
    ("a_guard_before_a_map_read", 52, "a map read at a run-time key keeps the self-interpreter's own map: its key text and its pair, no host map, and its generic display"),
];

fn argv(program_args: &[&str]) -> Vec<String> {
    std::iter::once("main".to_string()).chain(program_args.iter().map(|a| a.to_string())).collect()
}

/// What a run printed and how it ended.
fn lines_of(source: &str, argv: &[String]) -> (String, Option<String>) {
    let run = compile::tw_lines_with_args(source, argv);
    (run.lines.join("\n"), run.error)
}

/// The steps a run took on the tree-walker (`vm == false`, statements) or the register VM (instructions), and what it
/// printed.
fn steps_of(source: &str, argv: &[String], vm: bool) -> (u64, String, Option<String>) {
    let fuel = logicaffeine_compile::interpreter::install_fuel(1u64 << 40);
    let out = if vm {
        compile::run_strict_vm_with_args(source, argv)
    } else {
        compile::run_strict_tw_with_args(source, argv)
    };
    let used = fuel.budget - fuel.remaining.get();
    logicaffeine_compile::interpreter::clear_fuel();
    (used, out.output, out.error)
}

fn residual_of(program: &str) -> Result<String, String> {
    compile::textbook_projection1(program)
}

/// §1 for one case: every way its residual differs from the program on its inputs, or carries dispatch.
fn behavior_failures(c: &Case) -> Vec<String> {
    let residual = match residual_of(c.program) {
        Ok(r) => r,
        Err(e) => return vec![format!("`{}`: the first projection failed: {e}", c.name)],
    };
    let mut failures = Vec::new();
    let dispatch = compile::count_dispatch(&residual);
    let allowed = KNOWN_DISPATCH.iter().find(|(n, _, _)| *n == c.name).map(|(_, d, _)| *d).unwrap_or(0);
    if dispatch != allowed {
        failures.push(format!(
            "`{}`: the residual carries {dispatch} unit(s) of interpreter dispatch, the lock holds {allowed}\n--- residual ---\n{residual}",
            c.name
        ));
    }
    for input in c.inputs {
        let argv = argv(input);
        let program_run = lines_of(c.program, &argv);
        let residual_run = lines_of(&residual, &argv);
        if program_run != residual_run {
            failures.push(format!(
                "`{}` on {input:?}: the program prints {program_run:?}, its residual {residual_run:?}\n--- residual ---\n{residual}",
                c.name
            ));
        }
    }
    failures
}

/// §2 for one case: the most steps its residual takes beyond the program on any input, tree-walker and VM, and every
/// input where the residual's VM run differs from the program's.
fn excess_of(c: &Case) -> Result<(i64, i64), String> {
    let residual = residual_of(c.program).map_err(|e| format!("`{}`: the first projection failed: {e}", c.name))?;
    let (mut tw_most, mut vm_most) = (i64::MIN, i64::MIN);
    for input in c.inputs {
        let argv = argv(input);
        let (rt, ro, re) = steps_of(&residual, &argv, false);
        let (pt, po, pe) = steps_of(c.program, &argv, false);
        if (ro.as_str(), re.as_deref()) != (po.as_str(), pe.as_deref()) {
            return Err(format!("`{}` on {input:?}: on the tree-walker the program prints {po:?} {pe:?}, its residual {ro:?} {re:?}", c.name));
        }
        let (rv, rvo, rve) = steps_of(&residual, &argv, true);
        let (pv, pvo, pve) = steps_of(c.program, &argv, true);
        if (rvo.as_str(), rve.as_deref()) != (pvo.as_str(), pve.as_deref()) {
            return Err(format!("`{}` on {input:?}: on the VM the program prints {pvo:?} {pve:?}, its residual {rvo:?} {rve:?}\n--- residual ---\n{residual}", c.name));
        }
        tw_most = tw_most.max(rt as i64 - pt as i64);
        vm_most = vm_most.max(rv as i64 - pv as i64);
    }
    Ok((tw_most, vm_most))
}

#[test]
fn the_first_projection_reads_the_programs_arguments_when_it_runs() {
    assert!(ARGUMENTS_CORPUS.len() >= ARGUMENT_PROGRAMS, "the arguments corpus shrank to {} below its floor {ARGUMENT_PROGRAMS}", ARGUMENTS_CORPUS.len());
    let failures: Vec<String> = ARGUMENTS_CORPUS.iter().flat_map(behavior_failures).collect();
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n═══\n"));
}

#[test]
fn the_first_projection_takes_no_more_steps_than_the_program() {
    let mut failures = Vec::new();
    for c in ARGUMENTS_CORPUS {
        let (tw, vm) = match excess_of(c) {
            Ok(e) => e,
            Err(e) => {
                failures.push(e);
                continue;
            }
        };
        match KNOWN_EXCESS.iter().find(|(n, _, _, _)| *n == c.name) {
            Some((_, tw_bound, vm_bound, why)) => {
                if tw > *tw_bound as i64 || vm > *vm_bound as i64 {
                    failures.push(format!(
                        "`{}`: the residual takes {tw} tree-walker statement(s) and {vm} VM instruction(s) beyond the program; the lock allows {tw_bound} and {vm_bound} ({why})",
                        c.name
                    ));
                }
                if tw <= 0 && vm <= 0 {
                    failures.push(format!("`{}`: the residual is now as fast as the program — drop it from KNOWN_EXCESS", c.name));
                }
            }
            None => {
                if tw > 0 || vm > 0 {
                    failures.push(format!(
                        "`{}`: the residual takes {tw} tree-walker statement(s) and {vm} VM instruction(s) beyond the program",
                        c.name
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n═══\n"));
}

#[test]
fn every_known_excess_and_dispatch_names_a_corpus_program() {
    for (name, _, _, why) in KNOWN_EXCESS {
        assert!(ARGUMENTS_CORPUS.iter().any(|c| c.name == *name), "KNOWN_EXCESS names `{name}`, which the corpus does not hold");
        assert!(!why.is_empty(), "KNOWN_EXCESS `{name}` gives no reason");
    }
    for (name, _, why) in KNOWN_DISPATCH {
        assert!(ARGUMENTS_CORPUS.iter().any(|c| c.name == *name), "KNOWN_DISPATCH names `{name}`, which the corpus does not hold");
        assert!(!why.is_empty(), "KNOWN_DISPATCH `{name}` gives no reason");
    }
}

#[path = "common/generated_programs.rs"]
mod generated_programs;

use generated_programs::{generated_leaf, Rng, GENERATED_VARS};

/// A generated program that reads its first argument as `n`.
fn generated_program(seed: u64) -> String {
    generated_programs::generated_program(seed, "parseInt(item 2 of args())")
}

const GENERATED_INPUTS: &[&[&str]] = &[&["0"], &["7"], &["29"]];

/// §3 for `seeds`: every way a generated program's residual differs from the program, carries dispatch, or takes more
/// steps than the program on an input.
fn generated_failures(seeds: std::ops::Range<u64>) -> Vec<String> {
    seeded_failures(seeds, generated_program, GENERATED_INPUTS)
}

/// Every way the residual of `make(seed)` differs from the program, carries dispatch, or takes more steps than the
/// program on one of `inputs`, for each seed.
fn seeded_failures(seeds: std::ops::Range<u64>, make: fn(u64) -> String, inputs: &[&[&str]]) -> Vec<String> {
    let mut failures = Vec::new();
    for seed in seeds {
        let program = make(seed);
        let residual = match residual_of(&program) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("seed {seed}: the first projection failed: {e}\n--- program ---\n{program}"));
                continue;
            }
        };
        let dispatch = compile::count_dispatch(&residual);
        if dispatch != 0 {
            failures.push(format!("seed {seed}: the residual carries {dispatch} unit(s) of interpreter dispatch\n--- program ---\n{program}--- residual ---\n{residual}"));
        }
        for input in inputs {
            let argv = argv(input);
            let (rt, ro, re) = steps_of(&residual, &argv, false);
            let (pt, po, pe) = steps_of(&program, &argv, false);
            if (ro.as_str(), re.as_deref()) != (po.as_str(), pe.as_deref()) {
                failures.push(format!("seed {seed} on {input:?}: the program prints {po:?} {pe:?}, its residual {ro:?} {re:?}\n--- program ---\n{program}--- residual ---\n{residual}"));
                continue;
            }
            let (rv, _, _) = steps_of(&residual, &argv, true);
            let (pv, _, _) = steps_of(&program, &argv, true);
            if rt > pt || rv > pv {
                failures.push(format!(
                    "seed {seed} on {input:?}: the residual takes {rt} statement(s) and {rv} instruction(s), the program {pt} and {pv}\n--- program ---\n{program}--- residual ---\n{residual}"
                ));
            }
        }
    }
    failures
}

fn assert_generated(seeds: std::ops::Range<u64>) {
    let failures = generated_failures(seeds);
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n═══\n"));
}

#[test]
fn generated_programs_1_to_10_read_their_arguments_when_they_run_and_take_no_more_steps() {
    assert_generated(1..11);
}

#[test]
fn generated_programs_11_to_20_read_their_arguments_when_they_run_and_take_no_more_steps() {
    assert_generated(11..21);
}

#[test]
fn generated_programs_21_to_30_read_their_arguments_when_they_run_and_take_no_more_steps() {
    assert_generated(21..31);
}

fn failing_divisor(rng: &mut Rng) -> String {
    match rng.below(3) {
        0 => format!("(n - {})", rng.below(9)),
        1 => format!("({} - {})", rng.pick(&GENERATED_VARS), rng.below(5)),
        _ => format!("(n % {})", rng.below(5) + 2),
    }
}

fn failing_index(rng: &mut Rng) -> String {
    match rng.below(3) {
        0 => format!("((n % 9) - {})", rng.below(4)),
        1 => rng.pick(&GENERATED_VARS).to_string(),
        _ => format!("(n - {})", rng.below(6)),
    }
}

fn failing_expr(rng: &mut Rng) -> String {
    match rng.below(6) {
        0 => generated_leaf(rng),
        1 => format!("({} {} {})", generated_leaf(rng), rng.pick(&["/", "//", "%"]), failing_divisor(rng)),
        2 => format!("item {} of xs", failing_index(rng)),
        _ => format!("({} {} {})", generated_leaf(rng), rng.pick(&["+", "-"]), generated_leaf(rng)),
    }
}

fn failing_cond(rng: &mut Rng) -> String {
    format!(
        "{} is {} {}",
        failing_expr(rng),
        rng.pick(&["less than", "greater than", "at most", "at least"]),
        failing_expr(rng)
    )
}

fn failing_statements(rng: &mut Rng, out: &mut Vec<String>, count: usize, indent: usize, loops: usize) {
    let pad = "    ".repeat(indent);
    for _ in 0..count {
        let shape = rng.below(if indent < 2 { 7 } else { 4 });
        let v = *rng.pick(&GENERATED_VARS);
        match shape {
            0 | 1 => out.push(format!("{pad}Set {v} to {}.", failing_expr(rng))),
            2 => out.push(format!("{pad}Set item {} of xs to {}.", failing_index(rng), failing_expr(rng))),
            3 => {
                let d = failing_divisor(rng);
                out.push(format!("{pad}If {d} equals 0:"));
                out.push(format!("{pad}    Fail with \"Division by zero\"."));
                out.push(format!("{pad}Set {v} to {} / {d}.", generated_leaf(rng)));
            }
            4 => {
                let i = failing_index(rng);
                out.push(format!("{pad}If ({i} equals 0) or ({i} is greater than (length of xs)):"));
                out.push(format!("{pad}    Fail with \"Index {{{i}}} out of bounds\"."));
                out.push(format!("{pad}Set {v} to item {i} of xs."));
            }
            5 => {
                out.push(format!("{pad}If {}:", failing_cond(rng)));
                let then_count = 1 + rng.below(2) as usize;
                failing_statements(rng, out, then_count, indent + 1, loops);
                out.push(format!("{pad}Otherwise:"));
                let else_count = 1 + rng.below(2) as usize;
                failing_statements(rng, out, else_count, indent + 1, loops);
            }
            _ => {
                out.push(format!("{pad}Repeat for j{loops} from 1 to (n % 4):"));
                let body_count = 1 + rng.below(2) as usize;
                failing_statements(rng, out, body_count, indent + 1, loops + 1);
            }
        }
    }
}

/// A program over `Int` that reads its first argument as `n`, builds a list of `n % 7` items, and divides, takes a
/// modulo and floor-divides by expressions of `n` and of its variables, reads and writes the list at indexes `n` moves
/// out of range, guards some divisions and reads itself, and shows every variable and the list.
fn failing_program(seed: u64) -> String {
    let mut rng = Rng(seed);
    let mut out = vec![
        "## Main".to_string(),
        "Let n be parseInt(item 2 of args()).".to_string(),
        "Let mutable xs be a new Seq of Int.".to_string(),
        "Repeat for i from 1 to (n % 7):".to_string(),
        "    Push i * 3 to xs.".to_string(),
    ];
    for v in GENERATED_VARS {
        out.push(format!("Let mutable {v} be {}.", rng.below(7)));
    }
    let count = 3 + rng.below(4) as usize;
    failing_statements(&mut rng, &mut out, count, 0, 0);
    for v in GENERATED_VARS {
        out.push(format!("Show {v}."));
    }
    out.push("Show xs.".to_string());
    out.join("\n") + "\n"
}

/// Inputs that zero `n - k` and `n % k` for small `k`, leave the list empty, and move indexes past both of its ends.
const FAILING_INPUTS: &[&[&str]] = &[&["0"], &["2"], &["3"], &["7"], &["29"]];

fn assert_failing(seeds: std::ops::Range<u64>) {
    let failures = seeded_failures(seeds, failing_program, FAILING_INPUTS);
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n═══\n"));
}

#[test]
fn failing_programs_1_to_10_stop_where_the_program_stops_and_take_no_more_steps() {
    assert_failing(1..11);
}

#[test]
fn failing_programs_11_to_20_stop_where_the_program_stops_and_take_no_more_steps() {
    assert_failing(11..21);
}

#[test]
fn failing_programs_21_to_30_stop_where_the_program_stops_and_take_no_more_steps() {
    assert_failing(21..31);
}
