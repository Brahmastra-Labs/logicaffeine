mod common;

use common::compile_to_rust;

// =============================================================================
// Camp 6: Deforestation (Stream Fusion)
// =============================================================================
//
// Detect producer-consumer loop chains over intermediate collections and
// fuse them into a single loop, eliminating intermediate allocations.
// Producer: Let mutable temp = new Seq + loop Push to temp
// Consumer: Repeat for x in temp: <body>
// Fused: inline consumer body at each Push site, substituting pattern var

#[test]
fn deforest_map_reduce() {
    // Map to intermediate, then reduce — should fuse into single loop
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable doubled be a new Seq of Int.
Repeat for x in items:
    Push x * 2 to doubled.
Let mutable total be 0.
Repeat for y in doubled:
    Set total to total + y.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut doubled"),
        "Intermediate 'doubled' should be eliminated by deforestation. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "12");
}

#[test]
fn deforest_filter_collect() {
    // Filter to intermediate, then reduce — should fuse
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Push 4 to items.
Push 5 to items.
Let mutable big be a new Seq of Int.
Repeat for x in items:
    If x is greater than 2:
        Push x to big.
Let mutable total be 0.
Repeat for y in big:
    Set total to total + y.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut big"),
        "Intermediate 'big' should be eliminated by deforestation. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "12");
}

#[test]
fn deforest_map_filter_reduce() {
    // Three-stage pipeline: map → filter → reduce — both intermediates eliminated
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Push 4 to items.
Push 5 to items.
Let mutable doubled be a new Seq of Int.
Repeat for x in items:
    Push x * 2 to doubled.
Let mutable large be a new Seq of Int.
Repeat for y in doubled:
    If y is greater than 4:
        Push y to large.
Let mutable total be 0.
Repeat for z in large:
    Set total to total + z.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut doubled") && !rust.contains("let mut large"),
        "Both intermediates should be eliminated by deforestation. Got:\n{}",
        rust
    );
    // doubled = [2,4,6,8,10], large = [6,8,10], total = 24
    common::assert_exact_output(source, "24");
}

#[test]
fn deforest_preserves_order() {
    // Output order must be preserved after fusion
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 3 to items.
Push 1 to items.
Push 4 to items.
Push 1 to items.
Push 5 to items.
Let mutable transformed be a new Seq of Int.
Repeat for x in items:
    Push x + 10 to transformed.
Repeat for y in transformed:
    Show y.
"#;
    common::assert_exact_output(source, "13\n11\n14\n11\n15");
}

#[test]
fn deforest_empty_input() {
    // Empty source should not crash
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("0").
Let mutable items be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to items.
    Set i to i + 1.
Let mutable doubled be a new Seq of Int.
Repeat for x in items:
    Push x * 2 to doubled.
Let mutable total be 0.
Repeat for y in doubled:
    Set total to total + y.
Show total.
"#;
    common::assert_exact_output(source, "0");
}

#[test]
fn deforest_no_fuse_when_used_later() {
    // Intermediate used after consumer → NOT fused
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable doubled be a new Seq of Int.
Repeat for x in items:
    Push x * 2 to doubled.
Let mutable total be 0.
Repeat for y in doubled:
    Set total to total + y.
Show total.
Show length of doubled.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("doubled"),
        "Intermediate used after consumer should NOT be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "12\n3");
}

#[test]
fn deforest_no_fuse_when_passed_to_function() {
    // Intermediate passed to function → NOT fused
    let source = r#"## To native parseInt (s: Text) -> Int

## To sumAll (vals: Seq of Int) -> Int:
    Let mutable total be 0.
    Repeat for v in vals:
        Set total to total + v.
    Return total.

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable doubled be a new Seq of Int.
Repeat for x in items:
    Push x * 2 to doubled.
Show sumAll(doubled).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("doubled"),
        "Intermediate passed to function should NOT be eliminated. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "12");
}

#[test]
fn deforest_no_fuse_write_conflict() {
    // Consumer body writes to producer's source → NOT fused
    let source = r#"## To native parseInt (s: Text) -> Int

## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable doubled be a new Seq of Int.
Repeat for x in items:
    Push x * 2 to doubled.
Repeat for y in doubled:
    Push y to items.
Show length of items.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("doubled"),
        "Write conflict should prevent fusion. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "6");
}
