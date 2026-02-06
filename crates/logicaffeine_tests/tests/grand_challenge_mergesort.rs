//! Grand Challenge: Compile and run Merge Sort
//!
//! This test validates Phase 43 by compiling a complete recursive algorithm
//! that uses collections, ownership, and control flow.

use logicaffeine_compile::compile::compile_to_rust;

// =============================================================================
// Step 1: Test Comparison Parsing
// =============================================================================

#[test]
fn comparison_less_than_parses() {
    let source = r#"## Main
Let x be 5.
If x is less than 10:
    Return true.
Return false."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse 'is less than': {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("(x < 10)"), "Should generate < comparison: {}", rust);
}

#[test]
fn comparison_greater_than_parses() {
    let source = r#"## Main
Let x be 5.
If x is greater than 3:
    Return true.
Return false."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse 'is greater than': {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("(x > 3)"), "Should generate > comparison: {}", rust);
}

#[test]
fn comparison_at_most_parses() {
    let source = r#"## Main
Let i be 1.
While i is at most 5:
    Set i to i + 1.
Return i."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse 'is at most': {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("(i <= 5)"), "Should generate <= comparison: {}", rust);
}

#[test]
fn comparison_at_least_parses() {
    let source = r#"## Main
Let i be 10.
While i is at least 1:
    Set i to i - 1.
Return i."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse 'is at least': {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("(i >= 1)"), "Should generate >= comparison: {}", rust);
}

#[test]
fn comparison_symbol_lt() {
    let source = r#"## Main
Let x be 5.
If x < 10:
    Return true.
Return false."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse '<' symbol: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("(x < 10)"), "Should generate < comparison: {}", rust);
}

#[test]
fn comparison_symbol_lteq() {
    let source = r#"## Main
Let x be 5.
While x <= 10:
    Set x to x + 1.
Return x."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse '<=' symbol: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("(x <= 10)"), "Should generate <= comparison: {}", rust);
}

// =============================================================================
// Step 2: Test Compound Conditions with "and"
// =============================================================================

#[test]
fn compound_condition_and() {
    let source = r#"## Main
Let i be 1.
Let j be 1.
While i is at most 5 and j is at most 5:
    Set i to i + 1.
    Set j to j + 1.
Return i."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse compound condition with 'and': {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("&&"), "Should generate && for 'and': {}", rust);
}

#[test]
fn compound_condition_multiple_and() {
    let source = r#"## Main
Let a be 1.
Let b be 2.
Let c be 3.
If a is less than 5 and b is less than 5 and c is less than 5:
    Return true.
Return false."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should parse multiple 'and' conditions: {:?}", result);
    let rust = result.unwrap();
    // Should have at least 2 && operators
    assert!(rust.matches("&&").count() >= 2, "Should have multiple && operators: {}", rust);
}

// =============================================================================
// Step 3: Test Collection Operations in Loops
// =============================================================================

#[test]
fn loop_with_index_and_condition() {
    let source = r#"## Main
Let items be [1, 2, 3, 4, 5].
Let i be 1.
Let n be length of items.
Let result be 0.
While i is at most n:
    Let v be item i of items.
    Set result to result + v.
    Set i to i + 1.
Return result."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should compile loop with index access: {:?}", result);
}

// =============================================================================
// Step 4: Simplified Merge Sort (Main Challenge)
// =============================================================================

#[test]
fn merge_function_compiles() {
    let source = r#"## To Merge (left: Seq of Int) and (right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let i be 1.
    Let j be 1.
    Let n_left be length of left.
    Let n_right be length of right.

    While i is at most n_left and j is at most n_right:
        Let l_val be item i of left.
        Let r_val be item j of right.

        If l_val is less than r_val:
            Push l_val to result.
            Set i to i + 1.
        Otherwise:
            Push r_val to result.
            Set j to j + 1.

    While i is at most n_left:
        Let v be item i of left.
        Push v to result.
        Set i to i + 1.

    While j is at most n_right:
        Let v be item j of right.
        Push v to result.
        Set j to j + 1.

    Return result.

## Main
    Let x be 1."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Merge function should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("fn Merge"), "Should generate Merge function: {}", rust);
}

#[test]
fn mergesort_function_compiles() {
    let source = r#"## To MergeSort (items: Seq of Int) -> Seq of Int:
    Let n be length of items.
    If n is less than 2:
        Return copy of items.

    Let mid be n / 2.
    Let left_slice be items 1 through mid.
    Let right_slice be items (mid + 1) through n.

    Let sorted_left be MergeSort(copy of left_slice).
    Let sorted_right be MergeSort(copy of right_slice).

    Return Merge(sorted_left, sorted_right).

## To Merge (left: Seq of Int) and (right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Return result.

## Main
    Let x be 1."#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "MergeSort function should compile: {:?}", result);
    let rust = result.unwrap();
    assert!(rust.contains("fn MergeSort"), "Should generate MergeSort function: {}", rust);
    assert!(rust.contains(".clone()"), "Should generate .clone() for copy: {}", rust);
}

#[test]
fn full_mergesort_compiles() {
    let source = r#"## To Merge (left: Seq of Int) and (right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let i be 1.
    Let j be 1.
    Let n_left be length of left.
    Let n_right be length of right.

    While i is at most n_left and j is at most n_right:
        Let l_val be item i of left.
        Let r_val be item j of right.

        If l_val is less than r_val:
            Push l_val to result.
            Set i to i + 1.
        Otherwise:
            Push r_val to result.
            Set j to j + 1.

    While i is at most n_left:
        Let v be item i of left.
        Push v to result.
        Set i to i + 1.

    While j is at most n_right:
        Let v be item j of right.
        Push v to result.
        Set j to j + 1.

    Return result.

## To MergeSort (items: Seq of Int) -> Seq of Int:
    Let n be length of items.
    If n is less than 2:
        Return copy of items.

    Let mid be n / 2.
    Let left_slice be items 1 through mid.
    Let right_slice be items (mid + 1) through n.

    Let sorted_left be MergeSort(copy of left_slice).
    Let sorted_right be MergeSort(copy of right_slice).

    Return Merge(sorted_left, sorted_right).

## Main
    Let numbers be a new Seq of Int.
    Push 3 to numbers.
    Push 1 to numbers.
    Push 4 to numbers.
    Push 1 to numbers.
    Push 5 to numbers.

    Let sorted be MergeSort(numbers).
    Show sorted."#;

    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Full MergeSort should compile: {:?}", result);

    let rust = result.unwrap();

    // Verify key parts of the generated Rust
    assert!(rust.contains("fn Merge"), "Should have Merge function");
    assert!(rust.contains("fn MergeSort"), "Should have MergeSort function");
    assert!(rust.contains("fn main()"), "Should have main function");
    assert!(rust.contains("&&"), "Should have && for compound conditions");
    assert!(rust.contains(".push("), "Should have .push() for Push statements");
    assert!(rust.contains(".len()"), "Should have .len() for length of");
    assert!(rust.contains(".clone()"), "Should have .clone() for copy of");
}

// =============================================================================
// Step 5: E2E Test - Actually Run the Generated Rust
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_mergesort_runs_correctly() {
    use std::io::Write;
    use std::process::Command;

    let source = r#"## To Merge (left: Seq of Int) and (right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let i be 1.
    Let j be 1.
    Let n_left be length of left.
    Let n_right be length of right.

    While i is at most n_left and j is at most n_right:
        Let l_val be item i of left.
        Let r_val be item j of right.

        If l_val is less than r_val:
            Push l_val to result.
            Set i to i + 1.
        Otherwise:
            Push r_val to result.
            Set j to j + 1.

    While i is at most n_left:
        Let v be item i of left.
        Push v to result.
        Set i to i + 1.

    While j is at most n_right:
        Let v be item j of right.
        Push v to result.
        Set j to j + 1.

    Return result.

## To MergeSort (items: Seq of Int) -> Seq of Int:
    Let n be length of items.
    If n is less than 2:
        Return copy of items.

    Let mid be n / 2.
    Let left_slice be items 1 through mid.
    Let right_slice be items (mid + 1) through n.

    Let sorted_left be MergeSort(copy of left_slice).
    Let sorted_right be MergeSort(copy of right_slice).

    Return Merge(sorted_left, sorted_right).

## Main
    Let numbers be a new Seq of Int.
    Push 3 to numbers.
    Push 1 to numbers.
    Push 4 to numbers.
    Push 1 to numbers.
    Push 5 to numbers.
    Push 9 to numbers.
    Push 2 to numbers.
    Push 6 to numbers.

    Show numbers.
    Let sorted be MergeSort(numbers).
    Show sorted."#;

    // 1. Compile LOGOS to Rust
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should compile: {:?}", result);
    let rust_code = result.unwrap();

    // 2. Create temp project
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let project_dir = temp_dir.path();

    // Create Cargo.toml
    // Navigate up from crates/logicaffeine_tests to workspace root
    let workspace_root = std::env::current_dir()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let cargo_toml = format!(
        r#"[package]
name = "mergesort_e2e_test"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
"#,
        workspace_root.display(),
        workspace_root.display()
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();

    // 3. Build and run
    let output = Command::new("cargo")
        .args(["run", "--quiet"])
        .current_dir(project_dir)
        .output()
        .expect("Failed to run cargo");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Merge sort should run successfully.\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    // 4. Verify output
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.len() >= 2, "Should have at least 2 lines of output: {:?}", lines);

    // First line: unsorted input [3, 1, 4, 1, 5, 9, 2, 6]
    assert!(
        lines[0].contains("3") && lines[0].contains("1") && lines[0].contains("4"),
        "First line should show input array: {}",
        lines[0]
    );

    // Second line: sorted output [1, 1, 2, 3, 4, 5, 6, 9]
    let sorted_line = lines[1];
    assert!(
        sorted_line.contains("1") && sorted_line.contains("9"),
        "Second line should show sorted array: {}",
        sorted_line
    );

    // Verify correct sort order by checking that 1 appears before 9
    let first_1_pos = sorted_line.find('1').unwrap_or(999);
    let first_9_pos = sorted_line.find('9').unwrap_or(0);
    assert!(
        first_1_pos < first_9_pos,
        "Sorted array should have 1 before 9: {}",
        sorted_line
    );

    println!("E2E Merge Sort SUCCESS!");
    println!("Input:  {}", lines[0]);
    println!("Output: {}", lines[1]);
}
