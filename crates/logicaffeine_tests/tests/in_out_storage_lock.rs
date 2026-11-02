//! Mutable places are empty during a loan and receive the final value without changing saved aliases.

mod in_out_argument_lock {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

    fn same_result(source: &str, expected: &str) {
        let tw = tw_outcome(source);
        let vm = vm_outcome(source);
        assert_eq!(tw.error, None, "tree-walker fixture:\n{source}");
        assert_eq!(vm.error, None, "VM fixture:\n{source}");
        assert_eq!(tw.output.trim_end_matches('\n'), expected);
        assert_eq!(vm.output.trim_end_matches('\n'), expected);
        let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
            source,
            &[],
            logicaffeine_compile::optimization::Tier::T3,
            None,
        );
        assert_eq!(
            optimized.error, None,
            "optimized VM: {optimized:?}\n{source}"
        );
        assert_eq!(
            optimized.output.trim_end_matches('\n'),
            expected,
            "optimized VM"
        );
    }

    const BUMP: &str = "## To bump (value: mutable Int) -> Int:
    Set value to value + 1.
    Return value.
";

    #[test]
    fn a_mutable_scalar_updates_its_caller() {
        same_result(
            &format!("{BUMP}## Main\nLet mutable n be 4.\nShow bump(n).\nShow n.\n"),
            "5\n5",
        );
    }

    #[test]
    fn a_mutable_scalar_is_returned_on_an_early_return() {
        same_result(
            "## To change (value: mutable Int) -> Int:
    Set value to value + 1.
    If value is greater than 0:
        Return value.
    Set value to 99.
    Return value.
## Main
Let mutable n be 4.
Show change(n).
Show n.
",
            "5\n5",
        );
    }

    #[test]
    fn a_forwarded_mutable_scalar_reaches_the_outer_caller() {
        same_result(
            &format!(
                "{BUMP}
## To twice (value: mutable Int) -> Int:
    Let first be bump(value).
    Return bump(value).
## Main
Let mutable n be 4.
Show twice(n).
Show n.
"
            ),
            "6\n6",
        );
    }

    #[test]
    fn two_distinct_mutable_scalars_are_both_returned() {
        same_result(
            "## To change (left: mutable Int, right: mutable Int) -> Int:
    Set left to left + 2.
    Set right to right + 3.
    Return left + right.
## Main
Let mutable a be 1.
Let mutable b be 2.
Show change(a, b).
Show a.
Show b.
",
            "8\n3\n5",
        );
    }

    #[test]
    fn a_mutable_text_returns_its_new_buffer() {
        same_result(
            "## To append (s: mutable Text) -> Int:
    Set s to s + \"!\".
    Return length of s.
## Main
Let mutable s be \"hello\".
Let saved be s.
Show append(s).
Show s.
Show saved.
",
            "6\nhello!\nhello",
        );
    }

    #[test]
    fn a_mutable_sequence_returns_changes_without_changing_an_alias() {
        same_result(
            "## To change (xs: mutable Seq of Int) -> Int:
    Push 9 to xs.
    Set item 1 of xs to 7.
    Return length of xs.
## Main
Let mutable xs be [1].
Let saved be xs.
Show change(xs).
Show xs.
Show saved.
",
            "2\n[7, 9]\n[1]",
        );
    }

    #[test]
    fn a_mutable_record_field_returns_to_its_record() {
        same_result(
            &format!(
                "{BUMP}
## A Box has:
    a value, which is Int.
## Main
Let mutable b be a new Box with value 4.
Let saved be b.
Show bump(b's value).
Show b's value.
Show saved's value.
"
            ),
            "5\n5\n4",
        );
    }

    #[test]
    fn a_mutable_sequence_item_returns_to_its_sequence() {
        same_result(&format!("{BUMP}## Main\nLet mutable xs be [4].\nLet saved be xs.\nShow bump(item 1 of xs).\nShow item 1 of xs.\nShow item 1 of saved.\n"), "5\n5\n4");
    }

    #[test]
    fn a_value_expression_can_be_passed_without_a_place_to_return_to() {
        same_result(
            &format!("{BUMP}## Main\nLet n be 4.\nShow bump(n + 1).\nShow n.\n"),
            "6\n4",
        );
    }
}

mod in_out_observation_lock {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

    fn check(source: &str, expected: &str) {
        let tw = tw_outcome(source);
        let vm = vm_outcome(source);
        assert_eq!(tw.error, None, "{tw:?}\n{source}");
        assert_eq!(vm.error, None, "{vm:?}\n{source}");
        assert_eq!(tw.output.trim_end_matches('\n'), expected, "tree-walker");
        assert_eq!(vm.output.trim_end_matches('\n'), expected, "VM");
        let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
            source,
            &[],
            logicaffeine_compile::optimization::Tier::T3,
            None,
        );
        assert_eq!(
            optimized.error, None,
            "optimized VM: {optimized:?}\n{source}"
        );
        assert_eq!(
            optimized.output.trim_end_matches('\n'),
            expected,
            "optimized VM"
        );
    }

    #[test]
    fn a_lent_global_integer_is_nothing_until_the_call_returns() {
        check(
            "## To change (value: mutable Int) -> Int:
    Show n.
    Set value to 7.
    Return 1.
## Main
Let n be 4.
Show change(n).
Show n.
",
            "nothing\n1\n7",
        );
    }

    #[test]
    fn a_lent_global_text_is_nothing_until_the_call_returns() {
        check(
            "## To append (value: mutable Text) -> Int:
    Show s.
    Set value to value + \"!\".
    Return length of value.
## Main
Let s be \"hello\".
Show append(s).
Show s.
",
            "nothing\n6\nhello!",
        );
    }

    #[test]
    fn a_lent_global_list_can_be_mutated_before_and_after_the_call() {
        check(
            "## To change (value: mutable Seq of Int) -> Int:
    Show xs.
    Push 9 to value.
    Return length of value.
## Main
Let xs be [1].
Push 2 to xs.
Show change(xs).
Push 10 to xs.
Show xs.
",
            "nothing\n3\n[1, 2, 9, 10]",
        );
    }

    #[test]
    fn assigning_the_global_during_a_loan_is_observed_before_copyback() {
        check(
            "## To change (value: mutable Int) -> Int:
    Show n.
    Set n to 99.
    Show n.
    Set value to 7.
    Return 1.
## Main
Let n be 4.
Show change(n).
Show n.
",
            "nothing\n99\n1\n7",
        );
    }

    #[test]
    fn an_in_out_integer_can_grow_beyond_a_machine_word() {
        check("## To grow (value: mutable Int) -> Int:
    Repeat for i from 1 to 7:
        Set value to value * value.
    Return value.
## Main
Let n be 3.
Show grow(n).
Show n.
", "11790184577738583171520872861412518665678211592275841109096961\n11790184577738583171520872861412518665678211592275841109096961");
    }
}

mod in_out_exact_place_lock {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

    fn check(source: &str, expected: &str) {
        let tw = tw_outcome(source);
        let vm = vm_outcome(source);
        assert_eq!(tw.error, None, "{tw:?}\n{source}");
        assert_eq!(vm.error, None, "{vm:?}\n{source}");
        assert_eq!(tw.output.trim_end_matches('\n'), expected, "tree-walker");
        assert_eq!(vm.output.trim_end_matches('\n'), expected, "VM");
        let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
            source,
            &[],
            logicaffeine_compile::optimization::Tier::T3,
            None,
        );
        assert_eq!(
            optimized.error, None,
            "optimized VM: {optimized:?}\n{source}"
        );
        assert_eq!(
            optimized.output.trim_end_matches('\n'),
            expected,
            "optimized VM"
        );
    }

    #[test]
    fn an_exact_global_is_empty_during_its_loan() {
        check("## To grow (value: mutable Int) -> Int:\n    Show n.\n    Repeat for i from 1 to 7:\n        Set value to value * value.\n    Return value.\n## Main\nLet n be 3.\nShow grow(n).\nShow n.\n", "nothing\n11790184577738583171520872861412518665678211592275841109096961\n11790184577738583171520872861412518665678211592275841109096961");
    }

    #[test]
    fn an_exact_return_reaches_a_record_field() {
        check("## To grow (value: mutable Int) -> Int:\n    Repeat for i from 1 to 7:\n        Set value to value * value.\n    Return value.\n## A Box has:\n    a value, which is Int.\n## Main\nLet b be a new Box with value 3.\nLet saved be b.\nShow grow(b's value).\nShow b's value.\nShow saved's value.\n", "11790184577738583171520872861412518665678211592275841109096961\n11790184577738583171520872861412518665678211592275841109096961\n3");
    }

    #[test]
    fn an_exact_return_reaches_a_list_element() {
        check("## To grow (value: mutable Int) -> Int:\n    Repeat for i from 1 to 7:\n        Set value to value * value.\n    Return value.\n## Main\nLet xs be [3].\nLet saved be xs.\nShow grow(item 1 of xs).\nShow item 1 of xs.\nShow item 1 of saved.\n", "11790184577738583171520872861412518665678211592275841109096961\n11790184577738583171520872861412518665678211592275841109096961\n3");
    }

    #[test]
    fn an_exact_return_reaches_a_forwarded_parameter() {
        check("## To grow (value: mutable Int) -> Int:\n    Repeat for i from 1 to 7:\n        Set value to value * value.\n    Return value.\n## To forward (value: mutable Int) -> Int:\n    Return grow(value).\n## Main\nLet n be 3.\nShow forward(n).\nShow n.\n", "11790184577738583171520872861412518665678211592275841109096961\n11790184577738583171520872861412518665678211592275841109096961");
    }
}

mod in_out_projection_observation_lock {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

    fn check(source: &str, expected: &str) {
        let tw = tw_outcome(source);
        let vm = vm_outcome(source);
        assert_eq!(tw.error, None, "{tw:?}\n{source}");
        assert_eq!(vm.error, None, "{vm:?}\n{source}");
        assert_eq!(tw.output.trim_end_matches('\n'), expected, "tree-walker");
        assert_eq!(vm.output.trim_end_matches('\n'), expected, "VM");
        let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
            source,
            &[],
            logicaffeine_compile::optimization::Tier::T3,
            None,
        );
        assert_eq!(
            optimized.error, None,
            "optimized VM: {optimized:?}\n{source}"
        );
        assert_eq!(
            optimized.output.trim_end_matches('\n'),
            expected,
            "optimized VM"
        );
    }

    #[test]
    fn a_lent_global_record_field_is_nothing() {
        check("## A Box has:\n    a value, which is Int.\n## To change (value: mutable Int) -> Int:\n    Show b's value.\n    Set value to 7.\n    Return 1.\n## Main\nLet b be a new Box with value 4.\nShow change(b's value).\nShow b's value.\n", "nothing\n1\n7");
    }

    #[test]
    fn a_lent_global_sequence_element_is_nothing() {
        check("## To change (value: mutable Int) -> Int:\n    Show item 1 of xs.\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [4].\nShow change(item 1 of xs).\nShow xs.\n", "nothing\n[nothing]\n1\n[7]");
    }

    #[test]
    fn a_lent_nested_global_element_preserves_its_parent() {
        check("## To change (value: mutable Int) -> Int:\n    Show xs.\n    Set value to 7.\n    Return 1.\n## Main\nLet xs be [[4, 5]].\nShow change(item 1 of (item 1 of xs)).\nShow xs.\n", "[[nothing, 5]]\n1\n[[7, 5]]");
    }

    #[test]
    fn a_lent_global_map_entry_is_nothing() {
        check("## To change (value: mutable Int) -> Int:\n    Show item \"key\" of values.\n    Set value to 7.\n    Return 1.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"key\" of values to 4.\nShow change(item \"key\" of values).\nShow item \"key\" of values.\n", "nothing\n1\n7");
    }
}

mod in_out_field_ownership_lock {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

    fn check(source: &str, expected: &str) {
        let tw = tw_outcome(source);
        let vm = vm_outcome(source);
        assert_eq!(tw.error, None, "{tw:?}\n{source}");
        assert_eq!(vm.error, None, "{vm:?}\n{source}");
        assert_eq!(tw.output.trim_end_matches('\n'), expected, "tree-walker");
        assert_eq!(vm.output.trim_end_matches('\n'), expected, "VM");
        let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
            source,
            &[],
            logicaffeine_compile::optimization::Tier::T3,
            None,
        );
        assert_eq!(
            optimized.error, None,
            "optimized VM: {optimized:?}\n{source}"
        );
        assert_eq!(
            optimized.output.trim_end_matches('\n'),
            expected,
            "optimized VM"
        );
    }

    #[test]
    fn a_nullable_field_copies_a_live_text_builder() {
        check("## A Box has:\n    a value, which is Text.\n## To source (n: Int) -> Text:\n    Return \"x\" + n.\n## To change (value: mutable Text) -> Int:\n    Show b's value.\n    Set value to value + \"!\".\n    Return length of value.\n## Main\nLet text be source(7).\nLet b be a new Box with value text.\nSet text to text + \"?\".\nShow text.\nShow b's value.\nShow change(b's value).\nShow b's value.\nShow text.\n", "x7?\nx7\nnothing\n3\nx7!\nx7?");
    }
}

mod in_out_field_direct_write_lock {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

    fn check(source: &str, expected: &str) {
        let tw = tw_outcome(source);
        let vm = vm_outcome(source);
        assert_eq!(tw.error, None, "{tw:?}\n{source}");
        assert_eq!(vm.error, None, "{vm:?}\n{source}");
        assert_eq!(tw.output.trim_end_matches('\n'), expected, "tree-walker");
        assert_eq!(vm.output.trim_end_matches('\n'), expected, "VM");
        let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
            source,
            &[],
            logicaffeine_compile::optimization::Tier::T3,
            None,
        );
        assert_eq!(
            optimized.error, None,
            "optimized VM: {optimized:?}\n{source}"
        );
        assert_eq!(
            optimized.output.trim_end_matches('\n'),
            expected,
            "optimized VM"
        );
    }

    #[test]
    fn direct_push_to_a_nullable_field_preserves_a_record_copy() {
        check("## A Box has:\n    a values, which is Seq of Int.\n## To change (values: mutable Seq of Int) -> Int:\n    Show b's values.\n    Push 7 to values.\n    Return length of values.\n## Main\nLet b be a new Box with values [4].\nLet saved be b.\nPush 9 to b's values.\nShow b's values.\nShow saved's values.\nShow change(b's values).\nShow b's values.\nShow saved's values.\n", "[4, 9]\n[4]\nnothing\n3\n[4, 9, 7]\n[4]");
    }

    #[test]
    fn direct_map_write_to_a_nullable_field_preserves_a_record_copy() {
        check("## A Box has:\n    a values, which is Map of Text to Int.\n## To change (values: mutable Map of Text to Int) -> Int:\n    Show b's values.\n    Set item \"key\" of values to 7.\n    Return length of values.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"key\" of values to 4.\nLet b be a new Box with values values.\nLet saved be b.\nSet item \"key\" of b's values to 9.\nShow item \"key\" of b's values.\nShow item \"key\" of saved's values.\nShow change(b's values).\nShow item \"key\" of b's values.\nShow item \"key\" of saved's values.\n", "9\n4\nnothing\n1\n7\n4");
    }
}

mod in_out_field_values_lock {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

    fn check(source: &str, expected: &str) {
        let tw = tw_outcome(source);
        let vm = vm_outcome(source);
        assert_eq!(tw.error, None, "{tw:?}\n{source}");
        assert_eq!(vm.error, None, "{vm:?}\n{source}");
        assert_eq!(tw.output.trim_end_matches('\n'), expected, "tree-walker");
        assert_eq!(vm.output.trim_end_matches('\n'), expected, "VM");
        let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
            source,
            &[],
            logicaffeine_compile::optimization::Tier::T3,
            None,
        );
        assert_eq!(
            optimized.error, None,
            "optimized VM: {optimized:?}\n{source}"
        );
        assert_eq!(
            optimized.output.trim_end_matches('\n'),
            expected,
            "optimized VM"
        );
    }

    #[test]
    fn default_record_fields_start_present_before_their_loan() {
        check("## A Box has:\n    a value, which is Int.\n## To change (value: mutable Int) -> Int:\n    Show b's value.\n    Set value to 7.\n    Return 1.\n## Main\nLet b be a new Box.\nShow b's value.\nShow change(b's value).\nShow b's value.\n", "0\nnothing\n1\n7");
    }

    #[test]
    fn a_saved_empty_field_stays_empty_after_the_field_is_written() {
        check("## A Box has:\n    a value, which is Int.\n## To change (value: mutable Int) -> Int:\n    Let saved be b's value.\n    Set b's value to 99.\n    Show saved.\n    Show b's value.\n    Show saved is equal to nothing.\n    Set value to 7.\n    Return 1.\n## Main\nLet b be a new Box with value 4.\nShow change(b's value).\nShow b's value.\n", "nothing\n99\ntrue\n1\n7");
    }

    #[test]
    fn a_lent_text_field_returns_its_new_value() {
        check("## A Box has:\n    a value, which is Text.\n## To change (value: mutable Text) -> Int:\n    Show b's value.\n    Set value to value + \"!\".\n    Return length of value.\n## Main\nLet b be a new Box with value \"hi\".\nLet saved be b.\nShow change(b's value).\nShow b's value.\nShow saved's value.\n", "nothing\n3\nhi!\nhi");
    }
}

mod in_out_field_collection_lock {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

    fn check(source: &str, expected: &str) {
        let tw = tw_outcome(source);
        let vm = vm_outcome(source);
        assert_eq!(tw.error, None, "{tw:?}\n{source}");
        assert_eq!(vm.error, None, "{vm:?}\n{source}");
        assert_eq!(tw.output.trim_end_matches('\n'), expected, "tree-walker");
        assert_eq!(vm.output.trim_end_matches('\n'), expected, "VM");
        let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
            source,
            &[],
            logicaffeine_compile::optimization::Tier::T3,
            None,
        );
        assert_eq!(
            optimized.error, None,
            "optimized VM: {optimized:?}\n{source}"
        );
        assert_eq!(
            optimized.output.trim_end_matches('\n'),
            expected,
            "optimized VM"
        );
    }

    #[test]
    fn copying_a_record_copies_its_nullable_sequence_buffer() {
        check("## A Box has:\n    a values, which is Seq of Int.\n## To change (values: mutable Seq of Int) -> Int:\n    Show b's values.\n    Set item 1 of values to 9.\n    Push 7 to values.\n    Return length of values.\n## Main\nLet b be a new Box with values [4].\nLet saved be b.\nShow change(b's values).\nShow b's values.\nShow saved's values.\n", "nothing\n2\n[9, 7]\n[4]");
    }

    #[test]
    fn a_record_copied_during_a_sequence_loan_keeps_nothing() {
        check("## A Box has:\n    a values, which is Seq of Int.\n## To change (values: mutable Seq of Int) -> Int:\n    Set snapshot to b.\n    Push 7 to values.\n    Return length of values.\n## Main\nLet b be a new Box with values [4].\nLet snapshot be a new Box with values [1].\nShow change(b's values).\nShow b's values.\nShow snapshot's values.\n", "2\n[4, 7]\nnothing");
    }

    #[test]
    fn copying_a_record_copies_its_nullable_map_buffer() {
        check("## A Box has:\n    a values, which is Map of Text to Int.\n## To change (values: mutable Map of Text to Int) -> Int:\n    Show b's values.\n    Set item \"key\" of values to 9.\n    Return length of values.\n## Main\nLet values be a new Map of Text to Int.\nSet item \"key\" of values to 4.\nLet b be a new Box with values values.\nLet saved be b.\nShow change(b's values).\nShow item \"key\" of b's values.\nShow item \"key\" of saved's values.\n", "nothing\n1\n9\n4");
    }
}

mod captured_places {
    //! A mutable parameter lends a place, including packed and nested collection slots.
    //! Expected outputs are independent of either evaluator.
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

    fn check(source: &str, expected: &str) {
        let tw = tw_outcome(source);
        let vm = vm_outcome(source);
        assert_eq!(tw.error, None, "tree-walker: {tw:?}\n{source}");
        assert_eq!(vm.error, None, "VM: {vm:?}\n{source}");
        assert_eq!(
            tw.output.trim_end_matches('\n'),
            expected,
            "tree-walker\n{source}"
        );
        assert_eq!(vm.output.trim_end_matches('\n'), expected, "VM\n{source}");
        let optimized = logicaffeine_compile::compile::vm_outcome_tiered(
            source,
            &[],
            logicaffeine_compile::optimization::Tier::T3,
            None,
        );
        assert_eq!(optimized.error, None, "optimized VM: {optimized:?}");
        assert_eq!(
            optimized.output.trim_end_matches('\n'),
            expected,
            "optimized VM"
        );
    }

    const BUMP: &str = "## To bump (value: mutable Int) -> Int:
    Set value to value + 1.
    Return value.
";

    #[test]
    fn a_packed_integer_element_is_lent_and_returned() {
        check(&format!("{BUMP}## Main\nLet values be [4].\nLet saved be values.\nShow bump(item 1 of values).\nShow values.\nShow saved.\n"), "5\n[5]\n[4]");
    }

    #[test]
    fn a_negative_index_lends_the_same_element_that_reading_it_selects() {
        check(&format!("{BUMP}## Main\nLet values be [2, 4].\nShow bump(item -1 of values).\nShow values.\n"), "5\n[2, 5]");
    }

    #[test]
    fn a_packed_real_element_is_lent_and_returned() {
        check(
            "## To change (value: mutable Real) -> Int:
    Set value to 2.5.
    Return 1.
## Main
Let values be [1.5].
Show change(item 1 of values).
Show values.
",
            "1\n[2.5]",
        );
    }

    #[test]
    fn a_packed_text_element_is_lent_and_returned() {
        check(
            "## To change (value: mutable Text) -> Int:
    Set value to value + \"!\".
    Return 1.
## Main
Let values be [\"a\"].
Let saved be values.
Show change(item 1 of values).
Show item 1 of values.
Show item 1 of saved.
",
            "1\na!\na",
        );
    }

    #[test]
    fn nested_records_receive_the_final_parameter_value() {
        check(
            &format!(
                "{BUMP}
## A Box has:
    a value, which is Int.
## A Shelf has:
    a box, which is Box.
## Main
Let shelf be a new Shelf with box (a new Box with value 4).
Let saved be shelf.
Show bump(shelf's box's value).
Show shelf's box's value.
Show saved's box's value.
"
            ),
            "5\n5\n4",
        );
    }

    #[test]
    fn the_callee_observes_a_lent_global_as_nothing() {
        check(
            "## To change (value: mutable Int) -> Int:
    Show n.
    Set value to 7.
    Return 1.
## Main
Let n be 4.
Show change(n).
Show n.
",
            "nothing\n1\n7",
        );
    }
}
