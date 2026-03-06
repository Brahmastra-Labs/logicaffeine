#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::compile_to_rust;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn audit_mergesort_benchmark_codegen() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To mergeSort (arr: Seq of Int) -> Seq of Int:
    Let n be length of arr.
    If n is less than 2:
        Return arr.
    Let mid be n / 2.
    Let mutable left be a new Seq of Int.
    Let mutable right be a new Seq of Int.
    Let mutable i be 1.
    While i is at most mid:
        Push item i of arr to left.
        Set i to i + 1.
    While i is at most n:
        Push item i of arr to right.
        Set i to i + 1.
    Set left to mergeSort(left).
    Set right to mergeSort(right).
    Let mutable result be a new Seq of Int.
    Let mutable li be 1.
    Let mutable ri be 1.
    While li is at most length of left:
        If ri is greater than length of right:
            Push item li of left to result.
            Set li to li + 1.
        Otherwise:
            If item li of left is at most item ri of right:
                Push item li of left to result.
                Set li to li + 1.
            Otherwise:
                Push item ri of right to result.
                Set ri to ri + 1.
    While ri is at most length of right:
        Push item ri of right to result.
        Set ri to ri + 1.
    Return result.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show n.
"#;
    let rust = compile_to_rust(code).unwrap();

    // Merge capacity: result Vec should use Vec::with_capacity
    assert!(rust.contains("Vec::with_capacity"),
        "Merge result should use Vec::with_capacity, got:\n{}", rust);

    // Loop bounds hoisting: left_len and right_len should be hoisted
    assert!(rust.contains("let left_len"),
        "left length should be hoisted, got:\n{}", rust);
    assert!(rust.contains("let right_len"),
        "right length should be hoisted, got:\n{}", rust);

    // Hoisted names should appear in while condition
    assert!(rust.contains("li <= left_len"),
        "While condition should use hoisted left_len, got:\n{}", rust);

    // Hoisted names should appear in body (If condition)
    assert!(rust.contains("ri > right_len"),
        "Body condition should use hoisted right_len, got:\n{}", rust);

    // Read-only param: arr should be borrowed as &[i64]
    assert!(rust.contains("arr: &[i64]"),
        "Read-only param should be &[i64], got:\n{}", rust);

    // BUG FIX: .clone() on Copy types in hoisted while loops.
    // The |__hl: suffix on variable types broke type parsing, causing
    // from_rust_type_str("Vec<i64>|__hl:left_len") to return Unknown
    // instead of Seq(Int). This made has_copy_element_type return false,
    // emitting .clone() on every i64 array access inside hoisted loops.
    let merge_fn = rust.split("fn mergeSort").nth(1)
        .and_then(|s| s.split("\nfn ").next())
        .unwrap_or(&rust);
    assert!(!merge_fn.contains(".clone()"),
        "mergeSort should have NO .clone() calls — i64 is Copy.\n\
         The |__hl: hoisting suffix is breaking type parsing.\n\
         Generated code:\n{}", merge_fn);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn audit_hoisted_loop_no_clone_on_copy_types() {
    // Use a merge-like pattern that hoists bounds (two collections, not a simple for-range).
    let code = r#"## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let mutable result be a new Seq of Int.
    Let mutable li be 1.
    Let mutable ri be 1.
    While li is at most length of left:
        If ri is greater than length of right:
            Push item li of left to result.
            Set li to li + 1.
        Otherwise:
            If item li of left is at most item ri of right:
                Push item li of left to result.
                Set li to li + 1.
            Otherwise:
                Push item ri of right to result.
                Set ri to ri + 1.
    Return result.

## Main
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Push 1 to a. Push 3 to a.
Push 2 to b. Push 4 to b.
Show length of merge(a, b).
"#;
    let rust = compile_to_rust(code).unwrap();

    // Loop bounds should be hoisted
    assert!(rust.contains("let left_len"),
        "left length should be hoisted, got:\n{}", rust);

    // No .clone() on i64 elements — i64 is Copy
    let merge_fn = rust.split("fn merge(").nth(1)
        .and_then(|s| s.split("\nfn ").next())
        .unwrap_or(&rust);
    assert!(!merge_fn.contains(".clone()"),
        "i64 element access should NOT use .clone() when bounds are hoisted.\n\
         Generated code:\n{}", merge_fn);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn audit_for_each_vec_int_uses_copied() {
    let code = r#"## To sumAll (nums: Seq of Int) -> Int:
    Let mutable total be 0.
    Repeat for n in nums:
        Set total to total + n.
    Return total.

## Main
Let mutable nums be a new Seq of Int.
Push 10 to nums. Push 20 to nums. Push 30 to nums.
Show sumAll(nums).
"#;
    let rust = compile_to_rust(code).unwrap();

    // For-each over &[i64] should use .iter().copied(), not .iter().cloned()
    assert!(rust.contains(".iter().copied()") || rust.contains("for n in"),
        "For-each over &[i64] should use iter().copied(), got:\n{}", rust);
    assert!(!rust.contains(".iter().cloned()"),
        "For-each over &[i64] should NOT use iter().cloned() — i64 is Copy.\n\
         Generated code:\n{}", rust);
}
