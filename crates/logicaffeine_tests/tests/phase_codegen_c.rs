mod common;

use logicaffeine_compile::compile::compile_to_c;

// =============================================================================
// Unit Tests — Verify Generated C Code Structure
// =============================================================================

#[test]
fn codegen_c_hello_world() {
    let source = "## Main\nShow 42.";
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("#include"), "Should have C includes, got:\n{}", c_code);
    assert!(c_code.contains("int main("), "Should have main function, got:\n{}", c_code);
    assert!(c_code.contains("42"), "Should contain literal 42, got:\n{}", c_code);
}

#[test]
fn codegen_c_let_and_show() {
    let source = "## Main\nLet x be 5.\nShow x.";
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("int64_t x = 5"), "Should declare int64_t variable, got:\n{}", c_code);
}

#[test]
fn codegen_c_arithmetic() {
    let source = "## Main\nLet x be 2 + 3.\nShow x.";
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("int64_t x = 5"), "Should fold 2+3 to 5, got:\n{}", c_code);
}

#[test]
fn codegen_c_function_def() {
    let source = r#"## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10)."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("int64_t fib(int64_t n)"), "Should have fib function, got:\n{}", c_code);
    assert!(c_code.contains("return"), "Should have return statement, got:\n{}", c_code);
}

#[test]
fn codegen_c_seq_operations() {
    let source = r#"## Main
Let items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Show length of items."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("Seq_i64"), "Should use Seq_i64 type, got:\n{}", c_code);
    assert!(c_code.contains("seq_i64_push"), "Should use push helper, got:\n{}", c_code);
}

#[test]
fn codegen_c_while_loop() {
    let source = r#"## Main
Let mutable i be 0.
While i is less than 5:
    Set i to i + 1.
Show i."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("while"), "Should have while loop, got:\n{}", c_code);
}

#[test]
fn codegen_c_if_else() {
    let source = r#"## Main
Let x be 5.
If x is less than 10:
    Show 1.
Otherwise:
    Show 0."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("if ("), "Should have if statement, got:\n{}", c_code);
    assert!(c_code.contains("else"), "Should have else clause, got:\n{}", c_code);
}

#[test]
fn codegen_c_map_operations() {
    let source = r#"## Main
Let m be a new Map of Int to Int.
Set item 1 of m to 42.
Show item 1 of m."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("Map_i64_i64"), "Should use Map_i64_i64 type, got:\n{}", c_code);
}

#[test]
fn codegen_c_native_functions() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show n."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("logos_args"), "Should use logos_args, got:\n{}", c_code);
    assert!(c_code.contains("logos_parseInt"), "Should use logos_parseInt, got:\n{}", c_code);
}

#[test]
fn codegen_c_string_concat() {
    let source = r#"## Main
Let s be "hello" + " " + "world".
Show s."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("str_concat"), "Should use str_concat helper, got:\n{}", c_code);
}

// =============================================================================
// Unit Tests — C Keyword Escaping
// =============================================================================

#[test]
fn codegen_c_keyword_escape_double() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(5)."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("logos_double"), "C keyword 'double' should be escaped to 'logos_double', got:\n{}", c_code);
    assert!(!c_code.contains("int64_t double("), "Should NOT emit raw 'double' as function name, got:\n{}", c_code);
}

#[test]
fn codegen_c_keyword_escape_int_var() {
    let source = "## Main\nLet int be 5.\nShow int.";
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("logos_int"), "C keyword 'int' should be escaped to 'logos_int', got:\n{}", c_code);
}

#[test]
fn codegen_c_keyword_escape_float_function() {
    let source = r#"## To float (n: Int) -> Int:
    Return n.

## Main
Show float(42)."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("logos_float"), "C keyword 'float' should be escaped to 'logos_float', got:\n{}", c_code);
}

#[test]
fn codegen_c_keyword_escape_char_var() {
    let source = "## Main\nLet char be 65.\nShow char.";
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("logos_char"), "C keyword 'char' should be escaped to 'logos_char', got:\n{}", c_code);
}

#[test]
fn codegen_c_no_escape_normal_names() {
    let source = r#"## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10)."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("int64_t fib("), "Normal names should not be escaped, got:\n{}", c_code);
    assert!(!c_code.contains("logos_fib"), "Normal names should not be prefixed, got:\n{}", c_code);
}

#[test]
fn codegen_c_not_equals_codegen() {
    let source = r#"## Main
Let x be 5.
If x is not 3:
    Show 1.
Otherwise:
    Show 0."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("!="), "Not-equals should produce != operator, got:\n{}", c_code);
}

// =============================================================================
// Benchmark Compilation — Each benchmark should produce valid C
// =============================================================================

#[test]
fn codegen_c_benchmark_fib() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show fib(n)."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("int64_t fib("), "Should have fib function");
    assert!(c_code.contains("int main("), "Should have main");
}

#[test]
fn codegen_c_benchmark_ackermann() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To ackermann (m: Int) and (n: Int) -> Int:
    If m equals 0:
        Return n + 1.
    If n equals 0:
        Return ackermann(m - 1, 1).
    Return ackermann(m - 1, ackermann(m, n - 1)).

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show ackermann(3, n)."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("int64_t ackermann("), "Should have ackermann function");
}

#[test]
fn codegen_c_benchmark_sieve() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To sieve (limit: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is at most limit:
        Push false to flags.
        Set i to i + 1.
    Let mutable count be 0.
    Set i to 2.
    While i is at most limit:
        If item (i + 1) of flags equals false:
            Set count to count + 1.
            Let mutable j be i * i.
            While j is at most limit:
                Set item (j + 1) of flags to true.
                Set j to j + i.
        Set i to i + 1.
    Return count.

## Main
Let arguments be args().
Let limit be parseInt(item 2 of arguments).
Show sieve(limit)."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("int64_t sieve("), "Should have sieve function");
    assert!(c_code.contains("Seq_bool"), "Should use Seq_bool");
}

#[test]
fn codegen_c_benchmark_bubble_sort() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable arr be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 4294967296.
    Push (seed / 65536) % 32768 to arr.
    Set i to i + 1.
Set i to 0.
While i is less than n - 1:
    Let mutable j be 1.
    While j is at most n - 1 - i:
        Let a be item j of arr.
        Let b be item (j + 1) of arr.
        If a is greater than b:
            Set item j of arr to b.
            Set item (j + 1) of arr to a.
        Set j to j + 1.
    Set i to i + 1.
Show item 1 of arr."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("Seq_i64"), "Should use Seq_i64 for array");
    assert!(c_code.contains("int main("), "Should have main");
}

#[test]
fn codegen_c_benchmark_collect() {
    let source = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable m be a new Map of Int to Int with capacity n.
Let mutable i be 1.
While i is less than n + 1:
    Set item i of m to i * 2.
    Set i to i + 1.
Let mutable found be 0.
Set i to 1.
While i is less than n + 1:
    If item i of m equals i * 2:
        Set found to found + 1.
    Set i to i + 1.
Show found."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("Map_i64_i64"), "Should use Map_i64_i64");
    assert!(c_code.contains("int main("), "Should have main");
}

// =============================================================================
// E2E Tests — Compile C, run with gcc, verify output
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_c_hello_world() {
    let source = "## Main\nShow 42.";
    common::assert_c_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_c_fib() {
    let source = r#"## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10)."#;
    common::assert_c_output(source, "55");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_c_ackermann() {
    let source = r#"## To ackermann (m: Int) and (n: Int) -> Int:
    If m equals 0:
        Return n + 1.
    If n equals 0:
        Return ackermann(m - 1, 1).
    Return ackermann(m - 1, ackermann(m, n - 1)).

## Main
Show ackermann(3, 4)."#;
    common::assert_c_output(source, "125");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_c_sieve() {
    let source = r#"## To sieve (limit: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is at most limit:
        Push false to flags.
        Set i to i + 1.
    Let mutable count be 0.
    Set i to 2.
    While i is at most limit:
        If item (i + 1) of flags equals false:
            Set count to count + 1.
            Let mutable j be i * i.
            While j is at most limit:
                Set item (j + 1) of flags to true.
                Set j to j + i.
        Set i to i + 1.
    Return count.

## Main
Show sieve(100)."#;
    common::assert_c_output(source, "25");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_c_bubble_sort() {
    let source = r#"## Main
Let mutable arr be a new Seq of Int.
Push 3 to arr.
Push 1 to arr.
Push 4 to arr.
Push 1 to arr.
Push 5 to arr.
Let n be length of arr.
Let mutable i be 0.
While i is less than n - 1:
    Let mutable j be 1.
    While j is at most n - 1 - i:
        Let a be item j of arr.
        Let b be item (j + 1) of arr.
        If a is greater than b:
            Set item j of arr to b.
            Set item (j + 1) of arr to a.
        Set j to j + 1.
    Set i to i + 1.
Show item 1 of arr."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_c_collect() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 2.
Set item 2 of m to 4.
Set item 3 of m to 6.
Let mutable found be 0.
Let mutable i be 1.
While i is at most 3:
    If item i of m equals i * 2:
        Set found to found + 1.
    Set i to i + 1.
Show found."#;
    common::assert_c_output(source, "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_c_seq_bool() {
    let source = r#"## Main
Let mutable flags be a new Seq of Bool.
Push true to flags.
Push false to flags.
Push true to flags.
Let mutable count be 0.
Let mutable i be 1.
While i is at most 3:
    If item i of flags equals true:
        Set count to count + 1.
    Set i to i + 1.
Show count."#;
    common::assert_c_output(source, "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_c_string_show() {
    let source = r#"## Main
Show "hello"."#;
    common::assert_c_output(source, "hello");
}

// =============================================================================
// E2E Tests — C Keyword Escaping
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_keyword_double_function() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(21)."#;
    common::assert_c_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_keyword_int_variable() {
    common::assert_c_output("## Main\nLet int be 42.\nShow int.", "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_keyword_float_variable() {
    common::assert_c_output("## Main\nLet float be 99.\nShow float.", "99");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_keyword_char_variable() {
    common::assert_c_output("## Main\nLet char be 65.\nShow char.", "65");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_keyword_long_variable() {
    common::assert_c_output("## Main\nLet long be 100.\nShow long.", "100");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_keyword_void_function() {
    let source = r#"## To void (n: Int) -> Int:
    Return n + 1.

## Main
Show void(41)."#;
    common::assert_c_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_not_equals_operator() {
    let source = r#"## Main
If 5 is not 6:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_not_equals_false() {
    let source = r#"## Main
If 5 is not 5:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "0");
}

// =============================================================================
// Comprehensive E2E — Literals & Types
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_zero() {
    common::assert_c_output("## Main\nShow 0.", "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_negative() {
    common::assert_c_output("## Main\nShow 0 - 42.", "-42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_large_int() {
    common::assert_c_output("## Main\nShow 1000000000.", "1000000000");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_bool_true() {
    common::assert_c_output("## Main\nShow true.", "true");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_bool_false() {
    common::assert_c_output("## Main\nShow false.", "false");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_string_with_spaces() {
    let source = r#"## Main
Show "hello world"."#;
    common::assert_c_output(source, "hello world");
}

// =============================================================================
// Comprehensive E2E — Arithmetic
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_addition() {
    common::assert_c_output("## Main\nShow 3 + 7.", "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_subtraction() {
    common::assert_c_output("## Main\nShow 10 - 3.", "7");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_multiplication() {
    common::assert_c_output("## Main\nShow 6 * 7.", "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_division() {
    common::assert_c_output("## Main\nShow 100 / 4.", "25");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_modulo() {
    common::assert_c_output("## Main\nShow 17 % 5.", "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_complex_arithmetic() {
    common::assert_c_output("## Main\nShow (2 + 3) * (10 - 4).", "30");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_chained_arithmetic() {
    common::assert_c_output("## Main\nLet x be 5.\nLet y be x * 2 + 3.\nShow y.", "13");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_large_arithmetic() {
    common::assert_c_output("## Main\nShow 1103515245 * 42 + 12345.", "46347652635");
}

// =============================================================================
// Comprehensive E2E — Comparisons
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_equals_true() {
    let source = "## Main\nIf 5 equals 5:\n    Show 1.\nOtherwise:\n    Show 0.";
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_equals_false() {
    let source = "## Main\nIf 5 equals 6:\n    Show 1.\nOtherwise:\n    Show 0.";
    common::assert_c_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_not_equals() {
    let source = r#"## Main
If 5 is not 6:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_less_than() {
    let source = "## Main\nIf 3 is less than 5:\n    Show 1.\nOtherwise:\n    Show 0.";
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_less_than_false() {
    let source = "## Main\nIf 5 is less than 3:\n    Show 1.\nOtherwise:\n    Show 0.";
    common::assert_c_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_greater_than() {
    let source = "## Main\nIf 10 is greater than 5:\n    Show 1.\nOtherwise:\n    Show 0.";
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_at_most() {
    let source = r#"## Main
If 5 is at most 5:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_at_least() {
    let source = r#"## Main
If 5 is at least 5:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1");
}

// =============================================================================
// Comprehensive E2E — Boolean Logic
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_and_true() {
    let source = "## Main\nIf 3 is less than 5 and 10 is greater than 2:\n    Show 1.\nOtherwise:\n    Show 0.";
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_and_false() {
    let source = "## Main\nIf 3 is less than 5 and 10 is less than 2:\n    Show 1.\nOtherwise:\n    Show 0.";
    common::assert_c_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_or_true() {
    let source = "## Main\nIf 3 is less than 5 or 10 is less than 2:\n    Show 1.\nOtherwise:\n    Show 0.";
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_or_false() {
    let source = "## Main\nIf 3 is greater than 5 or 10 is less than 2:\n    Show 1.\nOtherwise:\n    Show 0.";
    common::assert_c_output(source, "0");
}

// =============================================================================
// Comprehensive E2E — Variables & Assignment
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_let_binding() {
    common::assert_c_output("## Main\nLet x be 42.\nShow x.", "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_mutable_set() {
    let source = "## Main\nLet mutable x be 1.\nSet x to 2.\nSet x to 3.\nShow x.";
    common::assert_c_output(source, "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_multiple_variables() {
    let source = "## Main\nLet a be 10.\nLet b be 20.\nLet c be a + b.\nShow c.";
    common::assert_c_output(source, "30");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_variable_in_condition() {
    let source = r#"## Main
Let x be 5.
If x is greater than 3:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1");
}

// =============================================================================
// Comprehensive E2E — Control Flow
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_if_only() {
    let source = "## Main\nIf 1 equals 1:\n    Show 42.";
    common::assert_c_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_if_false_no_output() {
    let source = "## Main\nIf 1 equals 2:\n    Show 42.\nShow 0.";
    common::assert_c_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_nested_if() {
    let source = r#"## Main
Let x be 10.
If x is greater than 5:
    If x is less than 20:
        Show 1.
    Otherwise:
        Show 2.
Otherwise:
    Show 3."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_while_sum() {
    let source = r#"## Main
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 10:
    Set sum to sum + i.
    Set i to i + 1.
Show sum."#;
    common::assert_c_output(source, "55");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_while_countdown() {
    let source = r#"## Main
Let mutable i be 5.
While i is greater than 0:
    Set i to i - 1.
Show i."#;
    common::assert_c_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_nested_while() {
    let source = r#"## Main
Let mutable total be 0.
Let mutable i be 1.
While i is at most 3:
    Let mutable j be 1.
    While j is at most 3:
        Set total to total + 1.
        Set j to j + 1.
    Set i to i + 1.
Show total."#;
    common::assert_c_output(source, "9");
}

// =============================================================================
// Comprehensive E2E — Functions
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_simple_function() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(21)."#;
    common::assert_c_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_multi_param_function() {
    let source = r#"## To add (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main
Show add(17, 25)."#;
    common::assert_c_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_factorial() {
    let source = r#"## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Show factorial(10)."#;
    common::assert_c_output(source, "3628800");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_gcd() {
    let source = r#"## To gcd (a: Int) and (b: Int) -> Int:
    If b equals 0:
        Return a.
    Return gcd(b, a % b).

## Main
Show gcd(48, 18)."#;
    common::assert_c_output(source, "6");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_function_calls_function() {
    let source = r#"## To square (n: Int) -> Int:
    Return n * n.

## To sumOfSquares (a: Int) and (b: Int) -> Int:
    Return square(a) + square(b).

## Main
Show sumOfSquares(3, 4)."#;
    common::assert_c_output(source, "25");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_function_with_loop() {
    let source = r#"## To sumTo (n: Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 1.
    While i is at most n:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show sumTo(100)."#;
    common::assert_c_output(source, "5050");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_function_multiple_return_paths() {
    let source = r#"## To classify (n: Int) -> Int:
    If n is less than 0:
        Return 0 - 1.
    If n equals 0:
        Return 0.
    Return 1.

## Main
Show classify(5).
Show classify(0).
Show classify(0 - 3)."#;
    common::assert_c_output(source, "1\n0\n-1");
}

// =============================================================================
// Comprehensive E2E — Seq<Int> Operations
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_int_push_and_access() {
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Show item 1 of items.
Show item 2 of items.
Show item 3 of items."#;
    common::assert_c_output(source, "10\n20\n30");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_int_length() {
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Push 4 to items.
Push 5 to items.
Show length of items."#;
    common::assert_c_output(source, "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_int_set_item() {
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Set item 2 of items to 99.
Show item 2 of items."#;
    common::assert_c_output(source, "99");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_int_sum() {
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Push 4 to items.
Push 5 to items.
Let mutable total be 0.
Let mutable i be 1.
While i is at most length of items:
    Set total to total + item i of items.
    Set i to i + 1.
Show total."#;
    common::assert_c_output(source, "15");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_int_reverse() {
    let source = r#"## Main
Let mutable arr be a new Seq of Int.
Push 1 to arr.
Push 2 to arr.
Push 3 to arr.
Push 4 to arr.
Push 5 to arr.
Let n be length of arr.
Let mutable lo be 1.
Let mutable hi be n.
While lo is less than hi:
    Let tmp be item lo of arr.
    Set item lo of arr to item hi of arr.
    Set item hi of arr to tmp.
    Set lo to lo + 1.
    Set hi to hi - 1.
Show item 1 of arr.
Show item 2 of arr.
Show item 3 of arr.
Show item 4 of arr.
Show item 5 of arr."#;
    common::assert_c_output(source, "5\n4\n3\n2\n1");
}

// =============================================================================
// Comprehensive E2E — Seq<Bool> Operations
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_bool_set_item() {
    let source = r#"## Main
Let mutable flags be a new Seq of Bool.
Push true to flags.
Push true to flags.
Push true to flags.
Set item 2 of flags to false.
Let mutable count be 0.
Let mutable i be 1.
While i is at most 3:
    If item i of flags equals true:
        Set count to count + 1.
    Set i to i + 1.
Show count."#;
    common::assert_c_output(source, "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_bool_all_false() {
    let source = r#"## Main
Let mutable flags be a new Seq of Bool.
Push false to flags.
Push false to flags.
Push false to flags.
Let mutable found be 0.
Let mutable i be 1.
While i is at most 3:
    If item i of flags equals true:
        Set found to 1.
    Set i to i + 1.
Show found."#;
    common::assert_c_output(source, "0");
}

// =============================================================================
// Comprehensive E2E — Map Operations
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_basic() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 100.
Set item 2 of m to 200.
Set item 3 of m to 300.
Show item 1 of m.
Show item 2 of m.
Show item 3 of m."#;
    common::assert_c_output(source, "100\n200\n300");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_overwrite() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 100.
Set item 1 of m to 999.
Show item 1 of m."#;
    common::assert_c_output(source, "999");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_with_capacity() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int with capacity 100.
Let mutable i be 1.
While i is at most 50:
    Set item i of m to i * i.
    Set i to i + 1.
Show item 7 of m."#;
    common::assert_c_output(source, "49");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_missing_key() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 42.
Show item 999 of m."#;
    common::assert_c_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_many_entries() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Let mutable i be 1.
While i is at most 100:
    Set item i of m to i * 2.
    Set i to i + 1.
Let mutable total be 0.
Set i to 1.
While i is at most 100:
    Set total to total + item i of m.
    Set i to i + 1.
Show total."#;
    // sum of 2+4+6+...+200 = 2*(1+2+...+100) = 2*5050 = 10100
    common::assert_c_output(source, "10100");
}

// =============================================================================
// Comprehensive E2E — Strings
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_concat() {
    let source = r#"## Main
Let s be "hello" + " " + "world".
Show s."#;
    common::assert_c_output(source, "hello world");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_concat_int() {
    let source = r#"## Main
Let x be 42.
Let s be "the answer is " + x.
Show s."#;
    common::assert_c_output(source, "the answer is 42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_empty() {
    let source = r#"## Main
Show ""."#;
    common::assert_c_output(source, "");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_concat_chain() {
    let source = r#"## Main
Let mutable s be "a".
Set s to s + "b".
Set s to s + "c".
Set s to s + "d".
Show s."#;
    common::assert_c_output(source, "abcd");
}

// =============================================================================
// Comprehensive E2E — Multiple Shows
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_multiple_shows() {
    let source = "## Main\nShow 1.\nShow 2.\nShow 3.";
    common::assert_c_output(source, "1\n2\n3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_mixed_types() {
    let source = r#"## Main
Show 42.
Show "hello".
Show true."#;
    common::assert_c_output(source, "42\nhello\ntrue");
}

// =============================================================================
// Comprehensive E2E — Complex Programs
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_fibonacci_iterative() {
    let source = r#"## Main
Let mutable a be 0.
Let mutable b be 1.
Let mutable i be 0.
While i is less than 20:
    Let next be a + b.
    Set a to b.
    Set b to next.
    Set i to i + 1.
Show a."#;
    common::assert_c_output(source, "6765");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_prime_check() {
    let source = r#"## To isPrime (n: Int) -> Int:
    If n is less than 2:
        Return 0.
    Let mutable i be 2.
    While i * i is at most n:
        If n % i equals 0:
            Return 0.
        Set i to i + 1.
    Return 1.

## Main
Show isPrime(2).
Show isPrime(17).
Show isPrime(4).
Show isPrime(97)."#;
    common::assert_c_output(source, "1\n1\n0\n1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_collatz() {
    let source = r#"## To collatz (n: Int) -> Int:
    Let mutable steps be 0.
    Let mutable x be n.
    While x is not 1:
        If x % 2 equals 0:
            Set x to x / 2.
        Otherwise:
            Set x to 3 * x + 1.
        Set steps to steps + 1.
    Return steps.

## Main
Show collatz(27)."#;
    // Collatz sequence for 27 takes 111 steps
    common::assert_c_output(source, "111");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_power_function() {
    let source = r#"## To power (base: Int) and (exp: Int) -> Int:
    Let mutable result be 1.
    Let mutable i be 0.
    While i is less than exp:
        Set result to result * base.
        Set i to i + 1.
    Return result.

## Main
Show power(2, 10).
Show power(3, 5)."#;
    common::assert_c_output(source, "1024\n243");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_selection_sort() {
    let source = r#"## Main
Let mutable arr be a new Seq of Int.
Push 5 to arr.
Push 3 to arr.
Push 8 to arr.
Push 1 to arr.
Push 9 to arr.
Push 2 to arr.
Push 7 to arr.
Let n be length of arr.
Let mutable i be 1.
While i is less than n:
    Let mutable minIdx be i.
    Let mutable j be i + 1.
    While j is at most n:
        If item j of arr is less than item minIdx of arr:
            Set minIdx to j.
        Set j to j + 1.
    If minIdx is not i:
        Let tmp be item i of arr.
        Set item i of arr to item minIdx of arr.
        Set item minIdx of arr to tmp.
    Set i to i + 1.
Show item 1 of arr.
Show item 2 of arr.
Show item 3 of arr.
Show item 4 of arr.
Show item 5 of arr.
Show item 6 of arr.
Show item 7 of arr."#;
    common::assert_c_output(source, "1\n2\n3\n5\n7\n8\n9");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_frequency_count() {
    let source = r#"## Main
Let mutable data be a new Seq of Int.
Push 1 to data.
Push 2 to data.
Push 3 to data.
Push 2 to data.
Push 1 to data.
Push 2 to data.
Let mutable freq be a new Map of Int to Int.
Let mutable i be 1.
While i is at most length of data:
    Let val be item i of data.
    Set item val of freq to item val of freq + 1.
    Set i to i + 1.
Show item 1 of freq.
Show item 2 of freq.
Show item 3 of freq."#;
    common::assert_c_output(source, "2\n3\n1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_nested_function_calls() {
    let source = r#"## To square (n: Int) -> Int:
    Return n * n.

## To cube (n: Int) -> Int:
    Return n * square(n).

## To sumCubes (a: Int) and (b: Int) -> Int:
    Return cube(a) + cube(b).

## Main
Show sumCubes(2, 3)."#;
    // 8 + 27 = 35
    common::assert_c_output(source, "35");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_fibonacci_sequence_to_array() {
    let source = r#"## Main
Let mutable fibs be a new Seq of Int.
Push 0 to fibs.
Push 1 to fibs.
Let mutable i be 3.
While i is at most 10:
    Let prev1 be item (i - 1) of fibs.
    Let prev2 be item (i - 2) of fibs.
    Push prev1 + prev2 to fibs.
    Set i to i + 1.
Show item 10 of fibs."#;
    // fib(9) = 34
    common::assert_c_output(source, "34");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_sieve_small() {
    let source = r#"## To sieve (limit: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is at most limit:
        Push false to flags.
        Set i to i + 1.
    Let mutable count be 0.
    Set i to 2.
    While i is at most limit:
        If item (i + 1) of flags equals false:
            Set count to count + 1.
            Let mutable j be i * i.
            While j is at most limit:
                Set item (j + 1) of flags to true.
                Set j to j + i.
        Set i to i + 1.
    Return count.

## Main
Show sieve(1000)."#;
    common::assert_c_output(source, "168");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_int_to_string_concat() {
    let source = r#"## Main
Let mutable result be "".
Let mutable i be 1.
While i is at most 5:
    Set result to result + i + " ".
    Set i to i + 1.
Show result."#;
    common::assert_c_output(source, "1 2 3 4 5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_with_capacity() {
    let source = r#"## Main
Let mutable s be "" with capacity 100.
Set s to s + "hello".
Set s to s + " ".
Set s to s + "world".
Show s."#;
    common::assert_c_output(source, "hello world");
}

// =============================================================================
// Phase 1: Core Language Gaps
// =============================================================================

// --- 1A: Seq_f64 ---

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_f64_push_get() {
    let source = r#"## Main
Let mutable vals be a new Seq of Float.
Push 1.5 to vals.
Push 2.7 to vals.
Push 3.14 to vals.
Show item 1 of vals.
Show item 2 of vals.
Show item 3 of vals."#;
    common::assert_c_output(source, "1.5\n2.7\n3.14");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_f64_iterate() {
    let source = r#"## Main
Let mutable vals be a new Seq of Float.
Push 1.0 to vals.
Push 2.0 to vals.
Push 3.0 to vals.
Let mutable sum be 0.0.
Let mutable i be 1.
While i is at most length of vals:
    Set sum to sum + item i of vals.
    Set i to i + 1.
Show sum."#;
    common::assert_c_output(source, "6");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_f64_length() {
    let source = r#"## Main
Let mutable vals be a new Seq of Float.
Push 1.1 to vals.
Push 2.2 to vals.
Show length of vals."#;
    common::assert_c_output(source, "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_f64_set_item() {
    let source = r#"## Main
Let mutable vals be a new Seq of Float.
Push 1.0 to vals.
Push 2.0 to vals.
Set item 2 of vals to 9.9.
Show item 2 of vals."#;
    common::assert_c_output(source, "9.9");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_float_arithmetic_show() {
    common::assert_c_output("## Main\nShow 3.14.", "3.14");
}

// --- 1B: Pop ---

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_pop_basic() {
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Pop from items.
Show length of items."#;
    common::assert_c_output(source, "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_pop_into_variable() {
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
Pop from items into last.
Show last.
Show length of items."#;
    common::assert_c_output(source, "30\n2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_pop_in_while() {
    let source = r#"## Main
Let mutable stack be a new Seq of Int.
Push 1 to stack.
Push 2 to stack.
Push 3 to stack.
Let mutable total be 0.
While length of stack is greater than 0:
    Pop from stack into val.
    Set total to total + val.
Show total."#;
    common::assert_c_output(source, "6");
}

// --- 1C: Contains ---

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_contains() {
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
Push 30 to items.
If items contains 20:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_seq_not_contains() {
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 10 to items.
Push 20 to items.
If items contains 99:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_contains() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 5 of m to 100.
Set item 10 of m to 200.
If m contains 5:
    Show 1.
Otherwise:
    Show 0.
If m contains 99:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1\n0");
}

// --- 1D: Copy ---

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_copy_seq() {
    let source = r#"## Main
Let mutable original be a new Seq of Int.
Push 1 to original.
Push 2 to original.
Push 3 to original.
Let mutable cloned be copy of original.
Push 99 to cloned.
Show length of original.
Show length of cloned."#;
    common::assert_c_output(source, "3\n4");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_copy_seq_independence() {
    let source = r#"## Main
Let mutable a be a new Seq of Int.
Push 10 to a.
Push 20 to a.
Let mutable b be copy of a.
Set item 1 of b to 99.
Show item 1 of a.
Show item 1 of b."#;
    common::assert_c_output(source, "10\n99");
}

// --- 1E: String comparison and length ---

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_equality() {
    let source = r#"## Main
Let a be "hello".
Let b be "hello".
If a equals b:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_inequality() {
    let source = r#"## Main
Let a be "hello".
Let b be "world".
If a is not b:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_length() {
    let source = r#"## Main
Let s be "hello".
Show length of s."#;
    common::assert_c_output(source, "5");
}

// --- 1F: Nested control flow, early return, Stmt::Call ---

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_nested_if_deep() {
    let source = r#"## Main
Let x be 15.
If x is greater than 10:
    If x is less than 20:
        If x equals 15:
            Show 1.
        Otherwise:
            Show 2.
    Otherwise:
        Show 3.
Otherwise:
    Show 4."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_while_in_repeat() {
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 3 to items.
Push 5 to items.
Push 2 to items.
Let mutable total be 0.
Let mutable idx be 1.
While idx is at most length of items:
    Let n be item idx of items.
    Let mutable j be 0.
    While j is less than n:
        Set total to total + 1.
        Set j to j + 1.
    Set idx to idx + 1.
Show total."#;
    common::assert_c_output(source, "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_early_return() {
    let source = r#"## To findFirst (items: Seq of Int) and (target: Int) -> Int:
    Let mutable i be 1.
    While i is at most length of items:
        If item i of items equals target:
            Return i.
        Set i to i + 1.
    Return 0 - 1.

## Main
Let mutable data be a new Seq of Int.
Push 10 to data.
Push 20 to data.
Push 30 to data.
Push 40 to data.
Show findFirst(data, 30)."#;
    common::assert_c_output(source, "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_void_function_call() {
    let source = r#"## To greet (name: Text):
    Show "Hello " + name.

## Main
Call greet with "World"."#;
    common::assert_c_output(source, "Hello World");
}

// =============================================================================
// Phase 2: Map Type Matrix
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_i64_basic() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "alice" of m to 100.
Set item "bob" of m to 200.
Show item "alice" of m.
Show item "bob" of m."#;
    common::assert_c_output(source, "100\n200");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_str_basic() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Text.
Set item "name" of m to "Alice".
Set item "city" of m to "NYC".
Show item "name" of m.
Show item "city" of m."#;
    common::assert_c_output(source, "Alice\nNYC");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_i64_str_basic() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Text.
Set item 1 of m to "one".
Set item 2 of m to "two".
Set item 3 of m to "three".
Show item 1 of m.
Show item 2 of m.
Show item 3 of m."#;
    common::assert_c_output(source, "one\ntwo\nthree");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_i64_overwrite() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "x" of m to 10.
Set item "x" of m to 99.
Show item "x" of m."#;
    common::assert_c_output(source, "99");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_i64_contains() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "hello" of m to 42.
If m contains "hello":
    Show 1.
Otherwise:
    Show 0.
If m contains "missing":
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1\n0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_i64_str_contains() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Text.
Set item 5 of m to "five".
If m contains 5:
    Show 1.
Otherwise:
    Show 0.
If m contains 99:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1\n0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_i64_missing_key() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "x" of m to 42.
Show item "missing" of m."#;
    common::assert_c_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_i64_many_entries() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Int.
Let mutable names be a new Seq of Text.
Push "a" to names.
Push "b" to names.
Push "c" to names.
Push "d" to names.
Push "e" to names.
Let mutable i be 1.
While i is at most length of names:
    Set item (item i of names) of m to i * 10.
    Set i to i + 1.
Show item "a" of m.
Show item "c" of m.
Show item "e" of m."#;
    common::assert_c_output(source, "10\n30\n50");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_str_concat_values() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Text.
Set item "greeting" of m to "Hello".
Set item "greeting" of m to item "greeting" of m + " World".
Show item "greeting" of m."#;
    common::assert_c_output(source, "Hello World");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_multiple_types_one_program() {
    let source = r#"## Main
Let mutable mi be a new Map of Int to Int.
Let mutable ms be a new Map of Text to Int.
Set item 1 of mi to 100.
Set item "x" of ms to 200.
Show item 1 of mi + item "x" of ms."#;
    common::assert_c_output(source, "300");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_i64_i64_contains() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 10.
Set item 2 of m to 20.
If m contains 1:
    Show 1.
Otherwise:
    Show 0.
If m contains 99:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1\n0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_i64_str_default_value() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Text.
Set item 1 of m to "hello".
Show item 999 of m."#;
    common::assert_c_output(source, "");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_word_frequency() {
    let source = r#"## Main
Let mutable words be a new Seq of Text.
Push "hello" to words.
Push "world" to words.
Push "hello" to words.
Push "hello" to words.
Push "world" to words.
Let mutable freq be a new Map of Text to Int.
Let mutable i be 1.
While i is at most length of words:
    Let w be item i of words.
    Set item w of freq to item w of freq + 1.
    Set i to i + 1.
Show item "hello" of freq.
Show item "world" of freq."#;
    common::assert_c_output(source, "3\n2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_str_missing_key() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Text.
Set item "a" of m to "alpha".
Show item "missing" of m."#;
    common::assert_c_output(source, "");
}

// =============================================================================
// Phase 3 — Struct Definitions
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_basic_construct_show_fields() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point with x 10 and y 20.
Show p's x.
Show p's y."#;
    common::assert_c_output(source, "10\n20");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_default_init() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point.
Show p's x.
Show p's y."#;
    common::assert_c_output(source, "0\n0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_field_access() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point with x 3 and y 4.
Let sum be p's x + p's y.
Show sum."#;
    common::assert_c_output(source, "7");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_field_mutation() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let mutable p be a new Point with x 10 and y 20.
Set p's x to 100.
Show p's x.
Show p's y."#;
    common::assert_c_output(source, "100\n20");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_pass_to_function() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## To sumCoords (p: Point) -> Int:
    Return p's x + p's y.

## Main
Let p be a new Point with x 5 and y 7.
Show sumCoords(p)."#;
    common::assert_c_output(source, "12");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_return_from_function() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## To makePoint (a: Int, b: Int) -> Point:
    Return a new Point with x a and y b.

## Main
Let p be makePoint(3, 4).
Show p's x.
Show p's y."#;
    common::assert_c_output(source, "3\n4");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_multiple_types() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## A Dimensions has:
    A width: Int.
    A height: Int.

## Main
Let p be a new Point with x 1 and y 2.
Let s be a new Dimensions with width 100 and height 200.
Show p's x + s's width.
Show p's y + s's height."#;
    common::assert_c_output(source, "101\n202");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_arithmetic_on_fields() {
    let source = r#"## A Rect has:
    A width: Int.
    A height: Int.

## Main
Let r be a new Rect with width 5 and height 10.
Let area be r's width * r's height.
Show area."#;
    common::assert_c_output(source, "50");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_with_text_field() {
    let source = r#"## A Person has:
    A name: Text.
    An age: Int.

## Main
Let p be a new Person with name "Alice" and age 30.
Show p's name.
Show p's age."#;
    common::assert_c_output(source, "Alice\n30");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_with_bool_field() {
    let source = r#"## A Flag has:
    An active: Bool.
    A count: Int.

## Main
Let f be a new Flag with active true and count 5.
Show f's active.
Show f's count."#;
    common::assert_c_output(source, "true\n5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_field_comparison() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point with x 10 and y 20.
If p's x is less than p's y:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_conditional_on_field() {
    let source = r#"## A Counter has:
    A value: Int.
    A limit: Int.

## Main
Let c be a new Counter with value 5 and limit 10.
If c's value is less than c's limit:
    Show c's limit - c's value.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_copy_value_semantics() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let mutable p be a new Point with x 10 and y 20.
Let q be p.
Set p's x to 999.
Show p's x.
Show q's x."#;
    common::assert_c_output(source, "999\n10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_while_modifying() {
    let source = r#"## A Counter has:
    A value: Int.

## Main
Let mutable c be a new Counter with value 0.
While c's value is less than 5:
    Set c's value to c's value + 1.
Show c's value."#;
    common::assert_c_output(source, "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_four_fields() {
    let source = r#"## A Quad has:
    A a: Int.
    A b: Int.
    A c: Int.
    A d: Int.

## Main
Let q be a new Quad with a 1 and b 2 and c 3 and d 4.
Show q's a + q's b + q's c + q's d."#;
    common::assert_c_output(source, "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_show() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point with x 10 and y 20.
Show p."#;
    common::assert_c_output(source, "Point(x: 10, y: 20)");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_show_with_text() {
    let source = r#"## A Person has:
    A name: Text.
    An age: Int.

## Main
Let p be a new Person with name "Bob" and age 25.
Show p."#;
    common::assert_c_output(source, "Person(name: Bob, age: 25)");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_struct_nested_field() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## A Line has:
    A start: Point.
    An end: Point.

## Main
Let a be a new Point with x 1 and y 2.
Let b be a new Point with x 3 and y 4.
Let line be a new Line with start a and end b.
Show line's start's x.
Show line's end's y."#;
    common::assert_c_output(source, "1\n4");
}

// =============================================================================
// Phase 4 — Enum Definitions + Pattern Matching
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_unit_variant() {
    let source = r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Red.
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
    When Blue: Show "blue"."#;
    common::assert_c_output(source, "red");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_payload_variant() {
    let source = r#"## A Shape is one of:
    A Circle with radius Int.
    A Square with side Int.

## Main
Let s be a new Circle with radius 10.
Inspect s:
    When Circle (r): Show r.
    When Square (sd): Show sd."#;
    common::assert_c_output(source, "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_otherwise() {
    let source = r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Blue.
Inspect c:
    When Red: Show "red".
    Otherwise: Show "other"."#;
    common::assert_c_output(source, "other");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_in_function_param() {
    let source = r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## To describe (c: Color) -> Text:
    Inspect c:
        When Red: Return "red".
        When Green: Return "green".
        When Blue: Return "blue".
    Return "unknown".

## Main
Let c be a new Green.
Show describe(c)."#;
    common::assert_c_output(source, "green");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_return_from_function() {
    let source = r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## To pick (n: Int) -> Color:
    If n equals 1:
        Return a new Red.
    If n equals 2:
        Return a new Green.
    Return a new Blue.

## Main
Let c be pick(2).
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
    When Blue: Show "blue"."#;
    common::assert_c_output(source, "green");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_three_plus_variants() {
    let source = r#"## A Direction is one of:
    A North.
    A South.
    A East.
    A West.

## Main
Let d be a new West.
Inspect d:
    When North: Show "N".
    When South: Show "S".
    When East: Show "E".
    When West: Show "W"."#;
    common::assert_c_output(source, "W");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_in_loop() {
    let source = r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let mutable i be 0.
Let mutable count be 0.
While i is less than 3:
    Let c be a new Red.
    Inspect c:
        When Red: Set count to count + 1.
        Otherwise: Set count to count + 0.
    Set i to i + 1.
Show count."#;
    common::assert_c_output(source, "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_mixed_field_types() {
    let source = r#"## A Data is one of:
    A IntVal with value Int.
    A TextVal with label Text.
    A Empty.

## Main
Let d be a new IntVal with value 42.
Inspect d:
    When IntVal (v): Show v.
    When TextVal (v): Show v.
    When Empty: Show "empty"."#;
    common::assert_c_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_tag_comparison() {
    let source = r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c1 be a new Red.
Let c2 be a new Blue.
Inspect c1:
    When Red: Show "c1 is red".
    Otherwise: Show "c1 is not red".
Inspect c2:
    When Red: Show "c2 is red".
    Otherwise: Show "c2 is not red"."#;
    common::assert_c_output(source, "c1 is red\nc2 is not red");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_arithmetic_on_destructured() {
    let source = r#"## A Shape is one of:
    A Circle with radius Int.
    A Rect with width Int and height Int.

## Main
Let s be a new Rect with width 5 and height 10.
Inspect s:
    When Circle (r):
        Show r * r.
    When Rect (w, h):
        Show w * h."#;
    common::assert_c_output(source, "50");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_unit_only() {
    let source = r#"## A Answer is one of:
    A Accept.
    A Reject.

## Main
Let b be a new Accept.
Inspect b:
    When Accept: Show "accept".
    When Reject: Show "reject"."#;
    common::assert_c_output(source, "accept");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_nested_inspect() {
    let source = r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c1 be a new Red.
Let c2 be a new Green.
Inspect c1:
    When Red:
        Inspect c2:
            When Green: Show "red-green".
            Otherwise: Show "red-other".
    Otherwise: Show "not-red"."#;
    common::assert_c_output(source, "red-green");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_linked_list_basic() {
    let source = r#"## An IntList is one of:
    A Nil.
    A Cons with value Int and next IntList.

## Main
Let end be a new Nil.
Let start be a new Cons with value 42 and next end.
Inspect start:
    When Nil: Show "empty".
    When Cons (v, n): Show v."#;
    common::assert_c_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_enum_show_destructured_field() {
    let source = r#"## A Wrapper is one of:
    A Wrapped with label Text and count Int.
    A Blank.

## Main
Let w be a new Wrapped with label "hello" and count 5.
Inspect w:
    When Wrapped (l, c):
        Show l.
        Show c.
    When Blank: Show "blank"."#;
    common::assert_c_output(source, "hello\n5");
}

// =============================================================================
// Phase 5 — Recursive Types
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_three_element_list() {
    let source = r#"## An IntList is one of:
    A Nil.
    A Cons with value Int and next IntList.

## Main
Let n0 be a new Nil.
Let n1 be a new Cons with value 3 and next n0.
Let n2 be a new Cons with value 2 and next n1.
Let n3 be a new Cons with value 1 and next n2.
Inspect n3:
    When Cons (v, rest):
        Show v.
        Inspect rest:
            When Cons (v2, rest2):
                Show v2.
                Inspect rest2:
                    When Cons (v3, rest3): Show v3.
                    When Nil: Show "nil".
            When Nil: Show "nil".
    When Nil: Show "nil"."#;
    common::assert_c_output(source, "1\n2\n3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_nil_variant() {
    let source = r#"## An IntList is one of:
    A Nil.
    A Cons with value Int and next IntList.

## Main
Let empty be a new Nil.
Inspect empty:
    When Nil: Show "nil".
    When Cons (v, n): Show "cons"."#;
    common::assert_c_output(source, "nil");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_match_cons_nil() {
    let source = r#"## An IntList is one of:
    A Nil.
    A Cons with value Int and next IntList.

## Main
Let n0 be a new Nil.
Let n1 be a new Cons with value 10 and next n0.
Inspect n1:
    When Cons (v, rest):
        Show v.
        Inspect rest:
            When Nil: Show "end".
            When Cons (v2, r2): Show v2.
    When Nil: Show "empty"."#;
    common::assert_c_output(source, "10\nend");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_sum_function() {
    let source = r#"## An IntList is one of:
    A Nil.
    A Cons with value Int and next IntList.

## To sumList (lst: IntList) -> Int:
    Inspect lst:
        When Nil: Return 0.
        When Cons (v, rest): Return v + sumList(rest).
    Return 0.

## Main
Let n0 be a new Nil.
Let n1 be a new Cons with value 3 and next n0.
Let n2 be a new Cons with value 2 and next n1.
Let n3 be a new Cons with value 1 and next n2.
Show sumList(n3)."#;
    common::assert_c_output(source, "6");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_count_function() {
    let source = r#"## An IntList is one of:
    A Nil.
    A Cons with value Int and next IntList.

## To countList (lst: IntList) -> Int:
    Inspect lst:
        When Nil: Return 0.
        When Cons (v, rest): Return 1 + countList(rest).
    Return 0.

## Main
Let n0 be a new Nil.
Let n1 be a new Cons with value 10 and next n0.
Let n2 be a new Cons with value 20 and next n1.
Let n3 be a new Cons with value 30 and next n2.
Let n4 be a new Cons with value 40 and next n3.
Let n5 be a new Cons with value 50 and next n4.
Show countList(n5)."#;
    common::assert_c_output(source, "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_binary_tree() {
    let source = r#"## A Tree is one of:
    A Leaf.
    A Branch with value Int and left Tree and right Tree.

## Main
Let l be a new Leaf.
Let t be a new Branch with value 10 and left l and right l.
Inspect t:
    When Leaf: Show "leaf".
    When Branch (v, lt, rt): Show v."#;
    common::assert_c_output(source, "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_tree_sum() {
    let source = r#"## A Tree is one of:
    A Leaf.
    A Branch with value Int and left Tree and right Tree.

## To sumTree (t: Tree) -> Int:
    Inspect t:
        When Leaf: Return 0.
        When Branch (v, lt, rt): Return v + sumTree(lt) + sumTree(rt).
    Return 0.

## Main
Let l be a new Leaf.
Let right be a new Branch with value 3 and left l and right l.
Let left be a new Branch with value 2 and left l and right l.
Let root be a new Branch with value 1 and left left and right right.
Show sumTree(root)."#;
    common::assert_c_output(source, "6");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_pass_to_function() {
    let source = r#"## An IntList is one of:
    A Nil.
    A Cons with value Int and next IntList.

## To head (lst: IntList) -> Int:
    Inspect lst:
        When Cons (v, rest): Return v.
        When Nil: Return 0.
    Return 0.

## Main
Let n0 be a new Nil.
Let n1 be a new Cons with value 99 and next n0.
Show head(n1)."#;
    common::assert_c_output(source, "99");
}

// =============================================================================
// Phase 6 — Slice, Range, Set
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_slice_middle() {
    let source = r#"## Main
Let mutable s be a new Seq of Int.
Push 10 to s.
Push 20 to s.
Push 30 to s.
Push 40 to s.
Push 50 to s.
Let sub be items 2 through 4 of s.
Repeat for x in sub:
    Show x."#;
    common::assert_c_output(source, "20\n30\n40");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_slice_full() {
    let source = r#"## Main
Let mutable s be a new Seq of Int.
Push 1 to s.
Push 2 to s.
Push 3 to s.
Let sub be items 1 through 3 of s.
Show length of sub."#;
    common::assert_c_output(source, "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_slice_single() {
    let source = r#"## Main
Let mutable s be a new Seq of Int.
Push 10 to s.
Push 20 to s.
Push 30 to s.
Let sub be items 2 through 2 of s.
Show length of sub.
Show item 1 of sub."#;
    common::assert_c_output(source, "1\n20");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_range_repeat_sum() {
    let source = r#"## Main
Let mutable total be 0.
Repeat for i from 1 to 10:
    Set total to total + i.
Show total."#;
    common::assert_c_output(source, "55");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_range_repeat_show() {
    let source = r#"## Main
Repeat for i from 1 to 5:
    Show i."#;
    common::assert_c_output(source, "1\n2\n3\n4\n5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_range_nested() {
    let source = r#"## Main
Let mutable count be 0.
Repeat for i from 1 to 3:
    Repeat for j from 1 to 3:
        Set count to count + 1.
Show count."#;
    common::assert_c_output(source, "9");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_range_variable_bounds() {
    let source = r#"## Main
Let lo be 3.
Let hi be 7.
Let mutable total be 0.
Repeat for i from lo to hi:
    Set total to total + i.
Show total."#;
    common::assert_c_output(source, "25");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_add_contains() {
    let source = r#"## Main
Let mutable s be a new Set of Int.
Add 10 to s.
Add 20 to s.
Add 30 to s.
Show s contains 20.
Show s contains 99."#;
    common::assert_c_output(source, "true\nfalse");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_remove() {
    let source = r#"## Main
Let mutable s be a new Set of Int.
Add 10 to s.
Add 20 to s.
Add 30 to s.
Remove 20 from s.
Show s contains 20.
Show length of s."#;
    common::assert_c_output(source, "false\n2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_length() {
    let source = r#"## Main
Let mutable s be a new Set of Int.
Add 1 to s.
Add 2 to s.
Add 3 to s.
Show length of s."#;
    common::assert_c_output(source, "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_dedup() {
    let source = r#"## Main
Let mutable s be a new Set of Int.
Add 5 to s.
Add 5 to s.
Add 5 to s.
Show length of s."#;
    common::assert_c_output(source, "1");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_string() {
    let source = r#"## Main
Let mutable s be a new Set of Text.
Add "hello" to s.
Add "world" to s.
Show s contains "hello".
Show s contains "foo".
Show length of s."#;
    common::assert_c_output(source, "true\nfalse\n2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_empty() {
    let source = r#"## Main
Let mutable s be a new Set of Int.
Show length of s.
Show s contains 1."#;
    common::assert_c_output(source, "0\nfalse");
}

// =============================================================================
// Phase 7: Map Iteration, Tuple Destructuring, WithCapacity, Misc
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_tuple_destructure() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 10.
Set item 2 of m to 20.
Set item 3 of m to 30.
Let mutable total be 0.
Repeat for (k, v) in m:
    Set total to total + v.
Show total."#;
    common::assert_c_output(source, "60");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_sum_keys() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 5 of m to 100.
Set item 10 of m to 200.
Set item 15 of m to 300.
Let mutable keysum be 0.
Repeat for (k, v) in m:
    Set keysum to keysum + k.
Show keysum."#;
    common::assert_c_output(source, "30");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_key_iteration() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "alpha" of m to 1.
Set item "beta" of m to 2.
Set item "gamma" of m to 3.
Let mutable total be 0.
Repeat for (k, v) in m:
    Set total to total + v.
Show total."#;
    common::assert_c_output(source, "6");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_i64_str_iteration() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Text.
Set item 1 of m to "one".
Set item 2 of m to "two".
Let mutable count be 0.
Repeat for (k, v) in m:
    Set count to count + 1.
Show count."#;
    common::assert_c_output(source, "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_str_str_iteration() {
    let source = r#"## Main
Let mutable m be a new Map of Text to Text.
Set item "greeting" of m to "hello".
Set item "farewell" of m to "bye".
Let mutable count be 0.
Repeat for (k, v) in m:
    Set count to count + 1.
Show count."#;
    common::assert_c_output(source, "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_with_capacity_seq() {
    let source = r#"## Main
Let mutable items be a new Seq of Int with capacity 100.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Show length of items."#;
    common::assert_c_output(source, "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_with_capacity_seq_str() {
    let source = r#"## Main
Let mutable words be a new Seq of Text with capacity 50.
Push "hello" to words.
Push "world" to words.
Show length of words."#;
    common::assert_c_output(source, "2");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_void_function() {
    let source = r#"## To greet (name: Text):
    Show name.

## Main
greet("Alice").
greet("Bob")."#;
    common::assert_c_output(source, "Alice\nBob");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_three_param_function() {
    let source = r#"## To clamp (val: Int, lo: Int, hi: Int) -> Int:
    If val is less than lo:
        Return lo.
    If val is greater than hi:
        Return hi.
    Return val.

## Main
Show clamp(5, 1, 10).
Show clamp(-3, 0, 100).
Show clamp(999, 0, 100)."#;
    common::assert_c_output(source, "5\n0\n100");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_bool() {
    let source = r#"## Main
Show true.
Show false."#;
    common::assert_c_output(source, "true\nfalse");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_bool_and_or() {
    let source = r#"## Main
Let a be true.
Let b be false.
Show a and b.
Show a or b.
Show a and a.
Show b or b."#;
    common::assert_c_output(source, "false\ntrue\ntrue\nfalse");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_mixed_show() {
    let source = r#"## Main
Show 42.
Show 3.14.
Show true.
Show "hello"."#;
    common::assert_c_output(source, "42\n3.14\ntrue\nhello");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_recursive_multiarg() {
    let source = r#"## To gcd (a: Int, b: Int) -> Int:
    If b equals 0:
        Return a.
    Return gcd(b, a % b).

## Main
Show gcd(48, 18).
Show gcd(100, 75)."#;
    common::assert_c_output(source, "6\n25");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_give_passthrough() {
    let source = r#"## To double (n: Int) -> Int:
    Let result be n * 2.
    Return result.

## Main
Show double(21)."#;
    common::assert_c_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_call_as_statement() {
    let source = r#"## To increment (x: Int) -> Int:
    Return x + 1.

## Main
Let mutable val be 10.
Set val to increment(val).
Show val."#;
    common::assert_c_output(source, "11");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_single_entry_iteration() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 42 of m to 99.
Let mutable found be 0.
Repeat for (k, v) in m:
    Set found to k + v.
Show found."#;
    common::assert_c_output(source, "141");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_iteration_with_condition() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 10.
Set item 2 of m to 20.
Set item 3 of m to 30.
Set item 4 of m to 40.
Let mutable bigsum be 0.
Repeat for (k, v) in m:
    If v is greater than 15:
        Set bigsum to bigsum + v.
Show bigsum."#;
    common::assert_c_output(source, "90");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_overwrite_then_iterate() {
    let source = r#"## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 10.
Set item 1 of m to 99.
Let mutable total be 0.
Repeat for (k, v) in m:
    Set total to total + v.
Show total."#;
    common::assert_c_output(source, "99");
}

// =============================================================================
// C Backend String Append Optimization
// =============================================================================

#[test]
fn codegen_c_self_append_uses_str_append() {
    let source = r#"## Main
Let mutable text be "hello".
Set text to text + " world".
Show text."#;
    let c_code = compile_to_c(source).unwrap();
    assert!(c_code.contains("str_append"),
        "Self-append should use str_append, got:\n{}", c_code);
    assert!(!c_code.contains("str_concat(text"),
        "Self-append should NOT use str_concat for the target variable, got:\n{}", c_code);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_self_append_loop() {
    common::assert_c_output(
        r#"## Main
Let mutable text be "".
Let mutable i be 1.
While i is at most 100:
    Set text to text + "a".
    Set i to i + 1.
Show length of text."#,
        "100",
    );
}
