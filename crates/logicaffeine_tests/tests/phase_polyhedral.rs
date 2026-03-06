mod common;

use common::compile_to_rust;

// =============================================================================
// Camp 7: Polyhedral Tiling
// =============================================================================
//
// Detect triple-nested for-range loops with affine 2D array access patterns
// (i*n+j+1) and emit tiled loop nests using step_by for L1 cache locality.
// Target: matrix_mult benchmark (O(n^3) with 1D-as-2D access pattern).

#[test]
fn tile_matrix_mult_detected() {
    // Triple-nested loop with i*n+j affine access pattern should emit step_by tiling
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("8").
Let mutable a be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable i be 0.
While i is less than n * n:
    Push i % 100 to a.
    Push 0 to c.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable k be 0.
    While k is less than n:
        Let mutable j be 0.
        While j is less than n:
            Let idx be i * n + j + 1.
            Set item idx of c to (item idx of c) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of a).
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Show item 1 of c.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("step_by"),
        "Triple-nested loop with affine access should be tiled with step_by. Got:\n{}",
        rust
    );
}

#[test]
fn tile_matrix_mult_correct() {
    // Tiled matrix multiply must produce the same result as naive.
    // Uses runtime n=4 via parseInt to prevent constant propagation.
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("4").
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable i be 0.
While i is less than n * n:
    Push (i % 5) + 1 to a.
    Push ((i * 3) % 7) + 1 to b.
    Push 0 to c.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable k be 0.
    While k is less than n:
        Let mutable j be 0.
        While j is less than n:
            Let idx be i * n + j + 1.
            Set item idx of c to (item idx of c) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of b).
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Set i to 1.
While i is at most n * n:
    Set checksum to checksum + item i of c.
    Set i to i + 1.
Show checksum.
"#;
    // Pre-computed: 4x4 matrix mult with the given initialization
    // a = [1,2,3,4, 5,1,2,3, 4,5,1,2, 3,4,5,1]
    // b = [1,4,7,3, 6,2,5,1, 4,7,3,6, 2,5,1,4]
    // c[i,j] = sum_k a[i,k]*b[k,j]
    // Row 0: 1*1+2*6+3*4+4*2 = 1+12+12+8 = 33, 1*4+2*2+3*7+4*5 = 4+4+21+20 = 49, 1*7+2*5+3*3+4*1 = 7+10+9+4 = 30, 1*3+2*1+3*6+4*4 = 3+2+18+16 = 39
    // Row 1: 5*1+1*6+2*4+3*2 = 5+6+8+6 = 25, 5*4+1*2+2*7+3*5 = 20+2+14+15 = 51, 5*7+1*5+2*3+3*1 = 35+5+6+3 = 49, 5*3+1*1+2*6+3*4 = 15+1+12+12 = 40
    // Row 2: 4*1+5*6+1*4+2*2 = 4+30+4+4 = 42, 4*4+5*2+1*7+2*5 = 16+10+7+10 = 43, 4*7+5*5+1*3+2*1 = 28+25+3+2 = 58, 4*3+5*1+1*6+2*4 = 12+5+6+8 = 31
    // Row 3: 3*1+4*6+5*4+1*2 = 3+24+20+2 = 49, 3*4+4*2+5*7+1*5 = 12+8+35+5 = 60, 3*7+4*5+5*3+1*1 = 21+20+15+1 = 57, 3*3+4*1+5*6+1*4 = 9+4+30+4 = 47
    // Sum = 33+49+30+39 + 25+51+49+40 + 42+43+58+31 + 49+60+57+47 = 151+165+174+213 = 703
    common::assert_exact_output(source, "703");
}

#[test]
fn tile_no_match_single_loop() {
    // A single loop should NOT be tiled (only triple-nested qualifies)
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("10").
Let mutable sum be 0.
Let mutable i be 0.
While i is less than n:
    Set sum to sum + i.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("step_by"),
        "Single loop should NOT be tiled. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "45");
}

#[test]
fn tile_no_match_double_nested() {
    // Double-nested loop should NOT be tiled (need 3 levels)
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("4").
Let mutable a be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Let mutable j be 0.
    While j is less than n:
        Push i * n + j to a.
        Set j to j + 1.
    Set i to i + 1.
Show length of a.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("step_by"),
        "Double-nested loop should NOT be tiled. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "16");
}

#[test]
fn tile_no_match_different_bounds() {
    // Triple-nested but with different bounds should NOT be tiled
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("4").
Let m be parseInt("3").
Let mutable sum be 0.
Let mutable i be 0.
While i is less than n:
    Let mutable k be 0.
    While k is less than m:
        Let mutable j be 0.
        While j is less than n:
            Set sum to sum + 1.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Show sum.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("step_by"),
        "Triple-nested with different bounds should NOT be tiled. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "48");
}

#[test]
fn tile_remainder_handling() {
    // n=5 with tile_size=32 means n < tile_size, so tiled code should
    // still produce correct results (tile handles via min())
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("5").
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable i be 0.
While i is less than n * n:
    Push (i % 3) + 1 to a.
    Push (i % 4) + 1 to b.
    Push 0 to c.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable k be 0.
    While k is less than n:
        Let mutable j be 0.
        While j is less than n:
            Let idx be i * n + j + 1.
            Set item idx of c to (item idx of c) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of b).
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Set i to 1.
While i is at most n * n:
    Set checksum to checksum + item i of c.
    Set i to i + 1.
Show checksum.
"#;
    // Correctness test — tiled and untiled should give same result
    common::assert_exact_output(source, "600");
}
