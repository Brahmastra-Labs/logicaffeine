//! Tests for bitwise operators in LOGOS.
//!
//! Tests parsing and interpretation of:
//! - `xor` / `^` — bitwise XOR
//! - `shifted left by` / `shifted right by` — bit shifts
//! - `&` / `|` — bitwise AND/OR
//! - `~` — bitwise NOT (ones complement)
//!
//! (The words `and`/`or`/`not` are LOGICAL — truthiness in, Bool out —
//! locked in correctness_truthiness.rs.)

#[cfg(not(target_arch = "wasm32"))]
use logicaffeine_compile::interpret_for_ui;

#[cfg(not(target_arch = "wasm32"))]
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    futures::executor::block_on(f)
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitwise_xor_basic() {
    let code = r#"## Main
Let a be 5.
Let b be 3.
Let c be a xor b.
Show c."#;
    let result = block_on(interpret_for_ui(code));
    assert!(result.error.is_none(), "error: {:?}", result.error);
    assert_eq!(result.lines, vec!["6"]); // 5 ^ 3 = 6
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitwise_shift_left() {
    let code = r#"## Main
Let a be 1.
Let b be a shifted left by 4.
Show b."#;
    let result = block_on(interpret_for_ui(code));
    assert!(result.error.is_none(), "error: {:?}", result.error);
    assert_eq!(result.lines, vec!["16"]); // 1 << 4 = 16
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitwise_shift_right() {
    let code = r#"## Main
Let a be 16.
Let b be a shifted right by 2.
Show b."#;
    let result = block_on(interpret_for_ui(code));
    assert!(result.error.is_none(), "error: {:?}", result.error);
    assert_eq!(result.lines, vec!["4"]); // 16 >> 2 = 4
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitwise_and_integers() {
    // `&` is the bitwise spelling; the word `and` is logical (truthiness →
    // Bool), locked in correctness_truthiness.rs.
    let code = r#"## Main
Let a be 12.
Let b be 10.
Let c be a & b.
Show c."#;
    let result = block_on(interpret_for_ui(code));
    assert!(result.error.is_none(), "error: {:?}", result.error);
    assert_eq!(result.lines, vec!["8"]); // 12 & 10 = 8
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitwise_or_integers() {
    let code = r#"## Main
Let a be 12.
Let b be 10.
Let c be a | b.
Show c."#;
    let result = block_on(interpret_for_ui(code));
    assert!(result.error.is_none(), "error: {:?}", result.error);
    assert_eq!(result.lines, vec!["14"]); // 12 | 10 = 14
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitwise_not_integer() {
    // mask = (1 << 3) - 1 = 7 (0b0111)
    // a = 5 (0b0101)
    // a & ~mask = 5 & (-8 in two's complement) = 0
    let code = r#"## Main
Let a be 5.
Let mask be (1 shifted left by 3) - 1.
Let b be a & ~mask.
Show b."#;
    let result = block_on(interpret_for_ui(code));
    assert!(result.error.is_none(), "error: {:?}", result.error);
    assert_eq!(result.lines, vec!["0"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitwise_nqueens_n1() {
    let code = r#"## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Let all be (1 shifted left by n) - 1.
    Let mutable available be all & ~(cols | diag1 | diag2).
    Let mutable count be 0.
    While available is not 0:
        Let bit be available & (0 - available).
        Set available to available xor bit.
        Set count to count + solve(row + 1, cols | bit, (diag1 | bit) shifted left by 1, (diag2 | bit) shifted right by 1, n).
    Return count.

## Main
Show solve(0, 0, 0, 0, 1)."#;
    let result = block_on(interpret_for_ui(code));
    assert!(result.error.is_none(), "error: {:?}", result.error);
    assert_eq!(result.lines, vec!["1"]); // 1-queens has 1 solution
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bitwise_nqueens_n8() {
    let code = r#"## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Let all be (1 shifted left by n) - 1.
    Let mutable available be all & ~(cols | diag1 | diag2).
    Let mutable count be 0.
    While available is not 0:
        Let bit be available & (0 - available).
        Set available to available xor bit.
        Set count to count + solve(row + 1, cols | bit, (diag1 | bit) shifted left by 1, (diag2 | bit) shifted right by 1, n).
    Return count.

## Main
Show solve(0, 0, 0, 0, 8)."#;
    let result = block_on(interpret_for_ui(code));
    assert!(result.error.is_none(), "error: {:?}", result.error);
    assert_eq!(result.lines, vec!["92"]); // 8-queens has 92 solutions
}
