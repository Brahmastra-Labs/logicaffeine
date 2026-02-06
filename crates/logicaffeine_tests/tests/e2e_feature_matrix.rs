//! E2E Tests: Feature Interaction Matrix
//!
//! Tests that exercise combinations of features that work individually but
//! whose interactions are untested. These are the gaps where real codegen bugs
//! hide: ownership, borrowing, type inference, and clone issues at feature boundaries.

mod common;
use common::{run_logos, assert_output};

// =============================================================================
// Category 1: Functions + Collections Integration
// =============================================================================

#[test]
fn e2e_function_takes_map() {
    assert_output(
        r#"## To totalCost (prices: Map of Text to Int) -> Int:
    Let a be item "apple" of prices.
    Let b be item "bread" of prices.
    Return a + b.

## Main
Let mut prices be a new Map of Text to Int.
Set item "apple" of prices to 90.
Set item "bread" of prices to 100.
Show totalCost(prices).
"#,
        "190",
    );
}

#[test]
fn e2e_function_returns_map() {
    assert_output(
        r#"## To makeConfig -> Map of Text to Int:
    Let mut m be a new Map of Text to Int.
    Set item "timeout" of m to 42.
    Return m.

## Main
Let config be makeConfig().
Show item "timeout" of config.
"#,
        "42",
    );
}

#[test]
fn e2e_function_takes_set() {
    assert_output(
        r#"## To hasAdmin (users: Set of Text) -> Bool:
    Return users contains "admin".

## Main
Let s be a new Set of Text.
Add "admin" to s.
Add "guest" to s.
If hasAdmin(s):
    Show "found".
"#,
        "found",
    );
}

#[test]
fn e2e_function_builds_list_in_loop() {
    assert_output(
        r#"## To evens (n: Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let i be 1.
    While i is at most n:
        If i / 2 * 2 equals i:
            Push i to result.
        Set i to i + 1.
    Return result.

## Main
Show evens(10).
"#,
        "[2, 4, 6, 8, 10]",
    );
}

#[test]
fn e2e_function_takes_seq_returns_seq() {
    assert_output(
        r#"## To reversed (items: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let i be length of items.
    While i is greater than 0:
        Push item i of items to result.
        Set i to i - 1.
    Return result.

## Main
Show reversed([1, 2, 3]).
"#,
        "[3, 2, 1]",
    );
}

#[test]
fn e2e_function_with_loop_and_conditional() {
    assert_output(
        r#"## To countOnes (items: Seq of Int) -> Int:
    Let count be 0.
    Repeat for x in items:
        If x equals 1:
            Set count to count + 1.
    Return count.

## Main
Show countOnes([1, 2, 1, 3, 1]).
"#,
        "3",
    );
}

// =============================================================================
// Category 2: Structs + Collections Integration
// =============================================================================

#[test]
fn e2e_struct_with_seq_field_push() {
    let source = r#"
## A Basket has:
    An items: Seq of Int.

## Main
Let mutable b be a new Basket.
Push 10 to b's items.
Push 20 to b's items.
Push 30 to b's items.
Show length of b's items.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("3"),
        "Expected '3' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_struct_with_seq_field_iterate() {
    let source = r#"
## A Basket has:
    An items: Seq of Int.

## Main
Let mutable b be a new Basket.
Push 10 to b's items.
Push 20 to b's items.
Push 30 to b's items.
Let sum be 0.
Repeat for x in b's items:
    Set sum to sum + x.
Show sum.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("60"),
        "Expected '60' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_collection_of_structs_field_sum() {
    let source = r#"
## A Point has:
    An x: Int.
    A y: Int.

## Main
Let points be a new Seq of Point.
Push a new Point with x 10 and y 1 to points.
Push a new Point with x 20 and y 2 to points.
Push a new Point with x 30 and y 3 to points.
Let sum be 0.
Repeat for p in points:
    Set sum to sum + p's x.
Show sum.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("60"),
        "Expected '60' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_struct_computation_function() {
    assert_output(
        r#"## A Rect has:
    A width: Int.
    A height: Int.

## To area (r: Rect) -> Int:
    Return r's width * r's height.

## Main
Let r be a new Rect with width 10 and height 20.
Show area(r).
"#,
        "200",
    );
}

#[test]
fn e2e_function_creates_struct_list() {
    let source = r#"
## A Point has:
    An x: Int.
    A y: Int.

## To makePoints (n: Int) -> Seq of Point:
    Let result be a new Seq of Point.
    Repeat for i from 1 to n:
        Push a new Point with x i and y i to result.
    Return result.

## Main
Let pts be makePoints(3).
Show length of pts.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("3"),
        "Expected '3' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

// =============================================================================
// Category 3: Enums + Complex Control Flow
// =============================================================================

#[test]
fn e2e_inspect_inside_repeat_loop() {
    let source = r#"
## A Shape is one of:
    A Circle with radius Int.
    A Square with side Int.

## Main
Let shapes be a new Seq of Shape.
Push a new Circle with radius 10 to shapes.
Push a new Square with side 20 to shapes.
Push a new Circle with radius 30 to shapes.
Let sum be 0.
Repeat for shape in shapes:
    Inspect shape:
        When Circle (r): Set sum to sum + r.
        When Square (side): Set sum to sum + side.
Show sum.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("60"),
        "Expected '60' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_enum_function_return_branch() {
    assert_output(
        r#"## A Sign is one of:
    A Positive.
    A Negative.
    A Zero.

## To classify (n: Int) -> Sign:
    If n is greater than 0:
        Return a new Positive.
    If n is less than 0:
        Return a new Negative.
    Return a new Zero.

## Main
Let s be classify(5).
Inspect s:
    When Positive: Show "positive".
    When Negative: Show "negative".
    When Zero: Show "zero".
"#,
        "positive",
    );
}

#[test]
fn e2e_recursive_tree_height() {
    let source = r#"
## A Tree is one of:
    A Leaf with value Int.
    A Node with left Tree and right Tree.

## To height (t: Tree) -> Int:
    Inspect t:
        When Leaf (v): Return 1.
        When Node (l, r):
            Let lh be height(l).
            Let rh be height(r).
            If lh is greater than rh:
                Return lh + 1.
            Return rh + 1.

## Main
Let a be a new Leaf with value 1.
Let b be a new Leaf with value 2.
Let c be a new Leaf with value 3.
Let left be a new Node with left a and right b.
Let tree be a new Node with left left and right c.
Show height(tree).
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("3"),
        "Expected '3' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_tree_sum_leaves() {
    let source = r#"
## A Tree is one of:
    A Leaf with value Int.
    A Node with left Tree and right Tree.

## To sumLeaves (t: Tree) -> Int:
    Inspect t:
        When Leaf (v): Return v.
        When Node (l, r):
            Return sumLeaves(l) + sumLeaves(r).

## Main
Let a be a new Leaf with value 1.
Let b be a new Leaf with value 2.
Let c be a new Leaf with value 3.
Let left be a new Node with left a and right b.
Let tree be a new Node with left left and right c.
Show sumLeaves(tree).
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("6"),
        "Expected '6' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

// =============================================================================
// Category 4: Algorithms Using Multiple Features
// =============================================================================

#[test]
fn e2e_binary_search() {
    assert_output(
        r#"## To binarySearch (items: Seq of Int) and (target: Int) -> Int:
    Let low be 1.
    Let high be length of items.
    While low is at most high:
        Let mid be (low + high) / 2.
        Let val be item mid of items.
        If val equals target:
            Return mid.
        If val is less than target:
            Set low to mid + 1.
        Otherwise:
            Set high to mid - 1.
    Return 0.

## Main
Let sorted be [10, 20, 30, 40, 50].
Show binarySearch(sorted, 40).
"#,
        "4",
    );
}

#[test]
fn e2e_insertion_sort() {
    assert_output(
        r#"## To insertionSort (items: Seq of Int) -> Seq of Int:
    Let result be copy of items.
    Let n be length of result.
    Let i be 2.
    While i is at most n:
        Let key be item i of result.
        Let j be i - 1.
        Let pos be i.
        While j is greater than 0:
            If item j of result is greater than key:
                Set item (j + 1) of result to item j of result.
                Set pos to j.
                Set j to j - 1.
            Otherwise:
                Set j to 0.
        Set item pos of result to key.
        Set i to i + 1.
    Return result.

## Main
Show insertionSort([3, 1, 5, 2, 8]).
"#,
        "[1, 2, 3, 5, 8]",
    );
}

#[test]
fn e2e_stack_data_structure() {
    let source = r#"
## A Stack has:
    An elements: Seq of Int.

## To push_stack (s: Stack) and (val: Int) -> Stack:
    Let mutable result be a new Stack.
    Let mutable elems be copy of s's elements.
    Push val to elems.
    Set result's elements to elems.
    Return result.

## Main
Let mutable s be a new Stack.
Set s to push_stack(s, 1).
Set s to push_stack(s, 2).
Set s to push_stack(s, 3).
Show length of s's elements.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("3"),
        "Expected '3' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_max_of_list() {
    assert_output(
        r#"## To maxOf (items: Seq of Int) -> Int:
    Let best be item 1 of items.
    Repeat for x in items:
        If x is greater than best:
            Set best to x.
    Return best.

## Main
Show maxOf([3, 99, 7, 42, 1]).
"#,
        "99",
    );
}

#[test]
fn e2e_count_occurrences() {
    assert_output(
        r#"## To countOf (items: Seq of Int) and (target: Int) -> Int:
    Let count be 0.
    Repeat for x in items:
        If x equals target:
            Set count to count + 1.
    Return count.

## Main
Show countOf([1, 2, 3, 2, 4, 2], 2).
"#,
        "3",
    );
}

#[test]
fn e2e_dot_product() {
    assert_output(
        r#"## To dot (a: Seq of Int) and (b: Seq of Int) -> Int:
    Let sum be 0.
    Let i be 1.
    While i is at most length of a:
        Set sum to sum + item i of a * item i of b.
        Set i to i + 1.
    Return sum.

## Main
Show dot([1, 2, 3], [4, 5, 6]).
"#,
        "32",
    );
}

// =============================================================================
// Category 5: Concurrency + Structured Data
// =============================================================================

#[test]
fn e2e_pipe_send_receive_multiple() {
    let source = r#"
## To producer (ch: Int):
    Send 1 into ch.
    Send 2 into ch.
    Send 3 into ch.
    Send 4 into ch.
    Send 5 into ch.

## Main
    Let ch be a Pipe of Int.
    Launch a task to producer with ch.
    Let sum be 0.
    Repeat for i from 1 to 5:
        Receive x from ch.
        Set sum to sum + x.
    Show sum.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("15"),
        "Expected '15' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_concurrent_struct_computation() {
    let source = r#"
## A Point has:
    An x: Int.
    A y: Int.

## To makePoint (val: Int) -> Point:
    Sleep 10.
    Return a new Point with x val and y val.

## Main
    Attempt all of the following:
        Let p1 be makePoint(10).
        Let p2 be makePoint(20).
    Show p1's x + p2's x.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("30"),
        "Expected '30' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_parallel_computation() {
    let source = r#"
## To compute (n: Int) -> Int:
    Let sum be 0.
    Repeat for i from 1 to n:
        Set sum to sum + i.
    Return sum.

## Main
    Simultaneously:
        Let a be compute(4).
        Let b be compute(5).
    Show a + b.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("25"),
        "Expected '25' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_select_receive_with_computation() {
    let source = r#"
## To sender (ch: Int):
    Sleep 10.
    Send 42 into ch.

## Main
    Let ch be a Pipe of Int.
    Launch a task to sender with ch.
    Await the first of:
        Receive val from ch:
            Show val * 2.
        After 5 seconds:
            Show "timeout".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("84"),
        "Expected '84' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

// =============================================================================
// Category 6: Multi-Feature Real-World Integration
// =============================================================================

#[test]
fn e2e_enum_state_machine() {
    assert_output(
        r#"## A State is one of:
    A Start.
    A Running with step Int.
    A Finished.

## Main
Let mutable state be a new Start.
Let mutable done be false.
While done equals false:
    Inspect state:
        When Start:
            Set state to a new Running with step 1.
        When Running (s):
            If s is greater than 3:
                Set state to a new Finished.
            Otherwise:
                Set state to a new Running with step s + 1.
        When Finished:
            Set done to true.
Show "done".
"#,
        "done",
    );
}

#[test]
fn e2e_multi_function_pipeline() {
    assert_output(
        r#"## To generate (n: Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Repeat for i from 1 to n:
        Push i to result.
    Return result.

## To filterEvens (items: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Repeat for x in items:
        If x / 2 * 2 equals x:
            Push x to result.
    Return result.

## To sumAll (items: Seq of Int) -> Int:
    Let sum be 0.
    Repeat for x in items:
        Set sum to sum + x.
    Return sum.

## Main
Let nums be generate(10).
Let evens be filterEvens(nums).
Show sumAll(evens).
"#,
        "30",
    );
}

#[test]
fn e2e_struct_with_enum_field() {
    assert_output(
        r#"## A Priority is one of:
    A High with level Int.
    A Low.

## A Job has:
    A name: Text.
    A priority: Priority.

## Main
Let t be a new Job with name "deploy" and priority a new High with level 10.
Inspect t's priority:
    When High (lvl): Show lvl.
    When Low: Show "low".
"#,
        "10",
    );
}

#[test]
fn e2e_policy_with_struct_function() {
    assert_output(
        r#"## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## To process (u: User):
    Check that the u is admin.
    Show "allowed".

## Main
Let u be a new User with role "admin".
process(u).
"#,
        "allowed",
    );
}

#[test]
fn e2e_refinement_in_function() {
    assert_output(
        r#"## To squarePositive (n: Int) -> Int:
    Let x: Int where x > 0 be n.
    Return x * x.

## Main
Show squarePositive(5).
"#,
        "25",
    );
}

// =============================================================================
// Category 7: FEATURE_MATRIX Gap Coverage - Functions Return Collections
// =============================================================================

#[test]
fn e2e_gap_1_function_returns_map_basic() {
    // Gap: Functions returning Map - ownership transfer through return
    assert_output(
        r#"## To makeConfig -> Map of Text to Int:
    Let m be a new Map of Text to Int.
    Set item "timeout" of m to 5000.
    Return m.

## Main
Let config be makeConfig().
Show item "timeout" of config.
"#,
        "5000",
    );
}

#[test]
fn e2e_gap_2_function_returns_set_basic() {
    // Gap: Functions returning Set - ownership transfer
    assert_output(
        r#"## To makeTags -> Set of Text:
    Let s be a new Set of Text.
    Add "rust" to s.
    Return s.

## Main
Let tags be makeTags().
If tags contains "rust":
    Show "found".
"#,
        "found",
    );
}

#[test]
fn e2e_gap_3_function_returns_map_iteration() {
    // Gap: Functions returning Map with iteration - now supports tuple destructuring!
    assert_output(
        r#"## To makeScores -> Map of Text to Int:
    Let m be a new Map of Text to Int.
    Set item "alice" of m to 10.
    Set item "bob" of m to 20.
    Return m.

## Main
Let scores be makeScores().
Let total be 0.
Repeat for (name, score) in scores:
    Set total to total + score.
Show total.
"#,
        "30",
    );
}

#[test]
fn e2e_gap_4_function_returns_set_mutation() {
    // Gap: Functions returning Set with subsequent mutation
    let source = r#"
## To makeTags -> Set of Text:
    Let s be a new Set of Text.
    Add "rust" to s.
    Return s.

## Main
Let mutable tags be makeTags().
Add "logos" to tags.
If tags contains "logos":
    Show "found".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("found"),
        "Expected 'found' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

// =============================================================================
// Category 8: FEATURE_MATRIX Gap Coverage - Structs + Collections Nested Mutation
// =============================================================================

#[test]
fn e2e_gap_5_struct_seq_push_basic() {
    // Gap: Struct with Seq field, Push operation - nested field mutation
    let source = r#"
## A TodoList has:
    An items: Seq of Text.

## Main
Let mutable list be a new TodoList.
Push "buy milk" to list's items.
Push "write tests" to list's items.
Show length of list's items.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("2"),
        "Expected '2' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_gap_6_struct_map_mutation() {
    // Gap: Struct with Map field, mutation - nested collection mutation
    // FIXED: Parser now correctly handles "Map of Text to Int" syntax with both type parameters
    let source = r#"
## A Config has:
    A settings: Map of Text to Int.

## Main
Let mutable cfg be a new Config.
Set item "timeout" of cfg's settings to 100.
Show item "timeout" of cfg's settings.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("100"),
        "Expected '100' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_gap_7_struct_set_field_mutation() {
    // Gap: Struct with Set field, Add operation
    let source = r#"
## A Profile has:
    A tags: Set of Text.

## Main
Let mutable prof be a new Profile.
Add "developer" to prof's tags.
Add "rust" to prof's tags.
If prof's tags contains "rust":
    Show "found".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("found"),
        "Expected 'found' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

// =============================================================================
// Category 9: FEATURE_MATRIX Gap Coverage - Give Through Function Parameters
// =============================================================================

#[test]
fn e2e_gap_8_give_to_function_basic() {
    // Gap: Give ownership to function parameter - ownership transfer through call statement
    // FIXED: Parser now handles Give keyword in Call argument position
    let source = r#"
## To consume (data: Seq of Int):
    Show length of data.

## Main
Let items be [1, 2, 3].
Call consume with Give items.
"#;
    assert_output(source, "3");
}

#[test]
fn e2e_gap_9_give_multiple_params() {
    // Gap: Give multiple parameters - multiple ownership transfers in single call
    // FIXED: Parser now handles Give keyword in Call argument position
    let source = r#"
## To combine (a: Seq of Int) and (b: Seq of Int):
    Show (length of a) + (length of b).

## Main
Let x be [1, 2].
Let y be [3, 4, 5].
Call combine with Give x and Give y.
"#;
    assert_output(source, "5");
}

#[test]
fn e2e_gap_10_give_struct_to_function() {
    // Gap: Give struct ownership to function
    // FIXED: Parser now handles Give keyword in Call argument position
    let source = r#"
## A Message has:
    A content: Text.

## To consume (msg: Message):
    Show msg's content.

## Main
Let m be a new Message with content "hello".
Call consume with Give m.
"#;
    assert_output(source, "hello");
}

// =============================================================================
// Category 10B: EXTREME Give Tests - Complex Expressions
// =============================================================================

#[test]
fn e2e_extreme_give_field_access() {
    // Extreme: Give with field access
    let source = r#"
## A Config has:
    A data: Seq of Int.

## To process (items: Seq of Int):
    Show length of items.

## Main
Let mutable cfg be a new Config.
Push 1 to cfg's data.
Push 2 to cfg's data.
Push 3 to cfg's data.
Call process with Give cfg's data.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("3"),
        "Expected '3' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
#[ignore] // Parser limitation: Give with function call expressions not supported
fn e2e_extreme_give_from_call() {
    // Extreme: Give result of function call
    // Bug: Parser doesn't support `Give <function_call>()` syntax
    // Error: ExpectedExpression at "Give makeData()"
    // Fix: Would need to extend parse_call_arg to handle Call expressions after Give
    let source = r#"
## To makeData -> Seq of Int:
    Let result be [1, 2, 3].
    Return result.

## To consume (data: Seq of Int) -> Int:
    Return length of data.

## Main
Show Call consume with Give makeData().
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("3"),
        "Expected '3' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
#[ignore] // Parser limitation: Give with complex index expressions not supported
fn e2e_extreme_give_collection_item() {
    // Extreme: Give item from collection
    // Bug: Parser doesn't support `Give item N of collection` syntax
    // Error: ExpectedIdentifier when parsing "item 1 of lists" after Give
    // Fix: Would need to extend parse_call_arg to handle Index expressions after Give
    let source = r#"
## To process (nums: Seq of Int):
    Show length of nums.

## Main
Let lists be a new Seq of (Seq of Int).
Push [1, 2, 3] to lists.
Push [4, 5] to lists.
Call process with Give item 1 of lists.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("3"),
        "Expected '3' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

// =============================================================================
// Category 10: FEATURE_MATRIX Gap Coverage - Deep Generics (3+ levels)
// =============================================================================

#[test]
fn e2e_gap_11_extreme_3level_nesting() {
    // Extreme test: 3-level nesting WITHOUT parentheses using workaround
    // Tests that type aliases work for deep nesting
    let source = r#"
## A Row has:
    An items: Seq of Int.

## A Matrix has:
    An rows: Seq of Row.

## Main
Let m be a new Matrix.
Let r1 be a new Row.
Push 1 to r1's items.
Push 2 to r1's items.
Push r1 to m's rows.
Show length of m's rows.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("1"),
        "Expected '1' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_gap_11_extreme_4level_nesting() {
    // Extreme test: 4-level nesting using type aliases
    let source = r#"
## A Cell has:
    A value: Int.

## A Row has:
    An items: Seq of Cell.

## A Matrix has:
    An rows: Seq of Row.

## A Cube has:
    An matrices: Seq of Matrix.

## Main
Let cube be a new Cube.
Let m be a new Matrix.
Let r be a new Row.
Let c be a new Cell with value 42.
Push c to r's items.
Push r to m's rows.
Push m to cube's matrices.
Show length of cube's matrices.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("1"),
        "Expected '1' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_gap_11_deep_generics_nested_seq() {
    // Gap: Deep generics - Seq of Seq of Int (2-level nesting)
    // Tests parenthesized type expressions: Seq of (Seq of Int)
    let source = r#"
## Main
Let matrix be a new Seq of (Seq of Int).
Let row1 be [1, 2, 3].
Let row2 be [4, 5, 6].
Push row1 to matrix.
Push row2 to matrix.
Show length of matrix.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("2"),
        "Expected '2' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

#[test]
fn e2e_gap_12_deep_generics_map_of_seq() {
    // Gap: Deep generics - Map of Text to Seq of Int
    // Tests parenthesized type expressions with Map: Map of Text to (Seq of Int)
    let source = r#"
## Main
Let data be a new Map of Text to (Seq of Int).
Let nums be [10, 20, 30].
Set item "scores" of data to nums.
Let retrieved be item "scores" of data.
Show length of retrieved.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source,
        result.rust_code,
        result.stderr
    );
    assert!(
        result.stdout.trim().contains("3"),
        "Expected '3' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        result.stdout.trim(),
        result.rust_code
    );
}

// =============================================================================
// Category 11: Copy Semantics
// =============================================================================

#[test]
fn e2e_copy_of_map() {
    // Bug A target: copy of Map generates .to_vec() which doesn't exist on HashMap
    assert_output(
        r#"## Main
Let m be a new Map of Text to Int.
Set item "x" of m to 1.
Let m2 be copy of m.
Set item "x" of m to 999.
Show item "x" of m2.
"#,
        "1",
    );
}

#[test]
fn e2e_copy_of_set() {
    // Bug A target: copy of Set generates .to_vec() which doesn't exist on HashSet
    assert_output(
        r#"## Main
Let s be a new Set of Text.
Add "hello" to s.
Let s2 be copy of s.
Add "world" to s.
If s2 contains "world":
    Show "shared".
Otherwise:
    Show "independent".
"#,
        "independent",
    );
}

#[test]
fn e2e_copy_of_seq_independence() {
    // Baseline: copy of Vec should already work - verify independence
    assert_output(
        r#"## Main
Let a be [1, 2, 3].
Let mutable b be copy of a.
Set item 1 of b to 99.
Show item 1 of a.
"#,
        "1",
    );
}

#[test]
fn e2e_copy_of_struct_seq_field() {
    // Copy struct's collection field, then mutate independently
    assert_output(
        r#"## A Basket has:
    An items: Seq of Int.

## Main
Let mutable b be a new Basket.
Push 10 to b's items.
Push 20 to b's items.
Let snapshot be copy of b's items.
Push 30 to b's items.
Show length of snapshot.
"#,
        "2",
    );
}

#[test]
fn e2e_copy_seq_mutate_both() {
    // Copy a seq, mutate both independently
    let source = r#"
## Main
Let mutable a be [1, 2, 3].
Let mutable b be copy of a.
Set item 1 of a to 99.
Set item 1 of b to 77.
Show item 1 of a.
Show item 1 of b.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    let output = result.stdout.trim();
    assert!(
        output.contains("99") && output.contains("77"),
        "Expected '99' and '77' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        output,
        result.rust_code
    );
}

// =============================================================================
// Category 12: String Operations Across Features
// =============================================================================

#[test]
fn e2e_string_concat_field_access() {
    // Bug B target: "Hello " + person's name should use format!()
    assert_output(
        r#"## A Person has:
    A name: Text.

## Main
Let p be a new Person with name "Alice".
Show "Hello " + p's name.
"#,
        "Hello Alice",
    );
}

#[test]
fn e2e_string_concat_function_return() {
    // Bug B target: "Hello " + getName() should use format!()
    assert_output(
        r#"## To getName -> Text:
    Return "Bob".

## Main
Show "Hello " + getName().
"#,
        "Hello Bob",
    );
}

#[test]
fn e2e_string_concat_index_result() {
    // Bug B target: "Winner: " + item 1 of names should use format!()
    assert_output(
        r#"## Main
Let names be ["Alice", "Bob"].
Show "Winner: " + item 1 of names.
"#,
        "Winner: Alice",
    );
}

#[test]
fn e2e_string_contains_on_field() {
    // Contains check on a struct's text field
    assert_output(
        r#"## A Doc has:
    A body: Text.

## Main
Let d be a new Doc with body "hello world".
If d's body contains "world":
    Show "found".
Otherwise:
    Show "missing".
"#,
        "found",
    );
}

#[test]
fn e2e_string_length_of_text() {
    // Length of a text string
    assert_output(
        r#"## Main
Let s be "hello".
Show length of s.
"#,
        "5",
    );
}

// =============================================================================
// Category 13: Enum Interplay
// =============================================================================

#[test]
fn e2e_enum_returned_from_function() {
    // Function returns enum, caller inspects it
    assert_output(
        r#"## A Result is one of:
    A Success with value Int.
    A Failure with message Text.

## To compute (x: Int) -> Result:
    If x is greater than 0:
        Return a new Success with value x * x.
    Return a new Failure with message "negative".

## Main
Let r be compute(3).
Inspect r:
    When Success (v): Show v - 6.
    When Failure (m): Show m.
"#,
        "3",
    );
}

#[test]
fn e2e_enum_passed_to_function() {
    // Function receives enum, inspects it
    assert_output(
        r#"## A Shape is one of:
    A Circle with radius Int.
    A Square with side Int.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle (r): Return r * r.
        When Square (side): Return side * side.

## Main
Let c be a new Square with side 5.
Show area(c).
"#,
        "25",
    );
}

#[test]
fn e2e_seq_of_enums_iterate_inspect() {
    // Collection of enums, iterate and inspect each
    assert_output(
        r#"## A Shape is one of:
    A Circle with radius Int.
    A Square with side Int.

## Main
Let shapes be a new Seq of Shape.
Push a new Circle with radius 5 to shapes.
Push a new Square with side 5 to shapes.
Let sum be 0.
Repeat for s in shapes:
    Inspect s:
        When Circle (r): Set sum to sum + r * r.
        When Square (side): Set sum to sum + side * side.
Show sum.
"#,
        "50",
    );
}

#[test]
fn e2e_struct_enum_field_inspect() {
    // Struct with enum field, access and inspect
    // Tests that CalendarUnit tokens (Year) work as enum variant names
    assert_output(
        r#"## A Period is one of:
    A Year with value Int.
    A Unknown.

## A Event has:
    A period: Period.

## Main
Let e be a new Event with period a new Year with value 2024.
Inspect e's period:
    When Year (v): Show v.
    When Unknown: Show "unknown".
"#,
        "2024",
    );
}

#[test]
fn e2e_nested_inspect_boxed() {
    // Nested inspect on recursive boxed enum
    assert_output(
        r#"## A Expr is one of:
    A Num with value Int.
    A Add with left Expr and right Expr.

## To eval (e: Expr) -> Int:
    Inspect e:
        When Num (v): Return v.
        When Add (l, r): Return eval(l) + eval(r).

## Main
Let a be a new Num with value 3.
Let b be a new Num with value 7.
Let sum be a new Add with left a and right b.
Show eval(sum).
"#,
        "10",
    );
}

#[test]
fn e2e_enum_func_return_to_collection() {
    // Function returns enum, push to collection, iterate
    assert_output(
        r#"## A Wrapped is one of:
    A Num with n Int.
    A Nothing.

## To wrap (x: Int) -> Wrapped:
    If x is greater than 0:
        Return a new Num with n x.
    Return a new Nothing.

## Main
Let items be a new Seq of Wrapped.
Push wrap(40) to items.
Push wrap(42) to items.
Push wrap(0) to items.
Let sum be 0.
Repeat for v in items:
    Inspect v:
        When Num (n): Set sum to sum + n.
        When Nothing: Set sum to sum + 0.
Show sum.
"#,
        "82",
    );
}

// =============================================================================
// Category 14: Complex Mutations
// =============================================================================

#[test]
fn e2e_set_index_swap_pattern() {
    // Swap two elements using a temp variable
    assert_output(
        r#"## Main
Let mutable items be [10, 20, 30, 40, 50].
Let temp be item 2 of items.
Set item 2 of items to item 4 of items.
Set item 4 of items to temp.
Show items.
"#,
        "[10, 40, 30, 20, 50]",
    );
}

#[test]
fn e2e_set_index_computed() {
    // Set index at a computed position
    assert_output(
        r#"## Main
Let mutable items be [1, 2, 3, 4, 5].
Let pos be 3.
Set item pos of items to 99.
Show items.
"#,
        "[1, 2, 99, 4, 5]",
    );
}

#[test]
fn e2e_set_field_increment() {
    // Read a struct field, compute, write back
    assert_output(
        r#"## A Counter has:
    A count: Int.

## Main
Let mutable c be a new Counter with count 0.
Set c's count to c's count + 1.
Set c's count to c's count + 1.
Set c's count to c's count + 1.
Show c's count.
"#,
        "3",
    );
}

#[test]
fn e2e_nested_field_chain_3_levels() {
    // 3-level struct nesting: set and get deep field
    assert_output(
        r#"## A Inner has:
    A value: Int.

## A Middle has:
    A inner: Inner.

## A Outer has:
    A middle: Middle.

## Main
Let i be a new Inner with value 42.
Let m be a new Middle with inner i.
Let o be a new Outer with middle m.
Show o's middle's inner's value.
"#,
        "42",
    );
}

// =============================================================================
// Category 15: Function Composition
// =============================================================================

#[test]
fn e2e_function_calling_function() {
    // Function calls another function
    assert_output(
        r#"## To double (x: Int) -> Int:
    Return x * 2.

## To quadruple (x: Int) -> Int:
    Return double(double(x)).

## Main
Show quadruple(5).
"#,
        "20",
    );
}

#[test]
fn e2e_func_returns_struct_with_collection() {
    // Function builds and returns a struct containing a collection
    let source = r#"
## A Report has:
    A totals: Seq of Int.

## To buildReport -> Report:
    Let mutable r be a new Report.
    Push 10 to r's totals.
    Push 20 to r's totals.
    Push 30 to r's totals.
    Return r.

## Main
Let report be buildReport().
Let sum be 0.
Repeat for t in report's totals:
    Set sum to sum + t.
Show sum.
Show length of report's totals.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    let output = result.stdout.trim();
    assert!(
        output.contains("60") && output.contains("3"),
        "Expected '60' and '3' in output.\nGot: '{}'\n\nGenerated Rust:\n{}",
        output,
        result.rust_code
    );
}

#[test]
fn e2e_multi_param_mixed_types() {
    // Function taking multiple params of different types
    assert_output(
        r#"## To compute (name: Text) and (base: Int) and (multiplier: Int) -> Int:
    Return base * multiplier.

## Main
Show compute("test", 50, 10).
"#,
        "500",
    );
}

#[test]
fn e2e_function_chain_struct_transform() {
    // Pass struct through a chain of functions
    assert_output(
        r#"## A Box has:
    A value: Int.

## To addTen (b: Box) -> Box:
    Return a new Box with value b's value + 10.

## To triple (b: Box) -> Box:
    Return a new Box with value b's value * 3.

## Main
Let b be a new Box with value 10.
Let b2 be addTen(b).
Let b3 be triple(b2).
Show b3's value.
"#,
        "60",
    );
}

// =============================================================================
// Category 16: Collection Ops in Complex Contexts
// =============================================================================

#[test]
fn e2e_show_slice_result() {
    // Bug C target: Show + Slice creates double-reference show(&&items[...])
    assert_output(
        r#"## Main
Let items be [10, 20, 30, 40, 50].
Show items 2 through 4.
"#,
        "[20, 30, 40]",
    );
}

#[test]
fn e2e_length_in_condition() {
    // Use length of collection in an If condition
    assert_output(
        r#"## Main
Let items be [1, 2, 3, 4, 5].
If length of items is greater than 3:
    Show "long".
Otherwise:
    Show "short".
"#,
        "long",
    );
}

#[test]
fn e2e_contains_set_conditional() {
    // Contains check on Set in If condition
    assert_output(
        r#"## Main
Let roles be a new Set of Text.
Add "admin" to roles.
Add "user" to roles.
If roles contains "admin":
    Show "granted".
Otherwise:
    Show "denied".
"#,
        "granted",
    );
}

#[test]
fn e2e_while_set_accumulator() {
    // Build a set in a while loop until it has enough elements
    assert_output(
        r#"## Main
Let mutable s be a new Set of Int.
Let i be 1.
While i is at most 10:
    If i / 2 * 2 equals i:
        Add i to s.
    Set i to i + 1.
Show length of s.
"#,
        "5",
    );
}
