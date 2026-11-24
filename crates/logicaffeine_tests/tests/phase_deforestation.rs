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

// =============================================================================
// Fusion legality: the fused loop runs consumer iteration k before producer iterations k+1..n, so the two
// loops fuse only when those actions commute. Each `no_fuse` row is a program whose loops do not, the last a
// producer with no effects that still fuses; each runs on the tree-walker (the oracle), the VM as resolved, the
// optimized VM with and without the native tier, and compiled Rust.
// =============================================================================

#[test]
fn deforest_no_fuse_when_the_consumer_pushes_to_another_producer_target() {
    // The consumer pushes to `first`, which the producer also fills: fused, the small items would land between the
    // large ones.
    let source = r#"## To order (xs: Seq of Int) -> Seq of Int:
    Let mutable first be a new Seq of Int.
    Let mutable rest be a new Seq of Int.
    Repeat for x in xs:
        If x is greater than 2:
            Push x to first.
        Otherwise:
            Push x to rest.
    Repeat for r in rest:
        Push r to first.
    Return first.

## Main
Show order([1, 2, 3, 4]).
"#;
    common::assert_all_tiers(source, "[3, 4, 1, 2]");
    common::assert_exact_output(source, "[3, 4, 1, 2]");
}

#[test]
fn deforest_no_fuse_when_the_producer_reads_its_intermediate() {
    // The producer tests `seen contains x`, so `seen` must exist while the producer runs.
    let source = r#"## To uniq (xs: Seq of Int) -> Int:
    Let mutable seen be a new Seq of Int.
    Repeat for x in xs:
        If not (seen contains x):
            Push x to seen.
    Let mutable total be 0.
    Repeat for s in seen:
        Set total to total + s.
    Return total.

## Main
Show uniq([1, 2, 2, 3, 3, 3]).
"#;
    common::assert_all_tiers(source, "6");
    common::assert_exact_output(source, "6");
}

#[test]
fn deforest_no_fuse_when_the_producer_measures_its_intermediate() {
    // The producer stops filling at three items by measuring the intermediate it fills.
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 5 to items.
Push 6 to items.
Push 7 to items.
Push 8 to items.
Push 9 to items.
Let mutable firstThree be a new Seq of Int.
Repeat for x in items:
    If length of firstThree is less than 3:
        Push x to firstThree.
Let mutable total be 0.
Repeat for y in firstThree:
    Set total to total + y.
Show total.
"#;
    common::assert_all_tiers(source, "18");
    common::assert_exact_output(source, "18");
}

#[test]
fn deforest_no_fuse_when_both_loops_show() {
    // Both loops print: every producer line comes before every consumer line.
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable copied be a new Seq of Int.
Repeat for x in items:
    Show x.
    Push x to copied.
Let mutable total be 0.
Repeat for y in copied:
    Show y * 10.
    Set total to total + y.
Show total.
"#;
    common::assert_all_tiers(source, "1\n2\n3\n10\n20\n30\n6");
    common::assert_exact_output(source, "1\n2\n3\n10\n20\n30\n6");
}

#[test]
fn deforest_no_fuse_when_the_consumer_reads_what_the_producer_counts() {
    // The consumer reads `count` after the producer has counted every item.
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable count be 0.
Let mutable copied be a new Seq of Int.
Repeat for x in items:
    Set count to count + 1.
    Push x to copied.
Repeat for y in copied:
    Show y + count.
"#;
    common::assert_all_tiers(source, "4\n5\n6");
    common::assert_exact_output(source, "4\n5\n6");
}

#[test]
fn deforest_no_fuse_when_the_consumer_changes_what_the_producer_reads() {
    // The producer reads `offset` before the consumer changes it.
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable offset be 0.
Let mutable shifted be a new Seq of Int.
Repeat for x in items:
    Push x + offset to shifted.
Repeat for y in shifted:
    Set offset to offset + 100.
    Show y.
Show offset.
"#;
    common::assert_all_tiers(source, "1\n2\n3\n300");
    common::assert_exact_output(source, "1\n2\n3\n300");
}

#[test]
fn deforest_no_fuse_when_the_consumer_breaks_before_the_producer_finishes() {
    // The consumer breaks after its first item, but the producer has already counted all three.
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable seen be 0.
Let mutable copied be a new Seq of Int.
Repeat for x in items:
    Set seen to seen + 1.
    Push x to copied.
Repeat for y in copied:
    Show y.
    Break.
Show seen.
"#;
    common::assert_all_tiers(source, "1\n3");
    common::assert_exact_output(source, "1\n3");
}

#[test]
fn deforest_no_fuse_when_the_consumer_reads_its_intermediate() {
    // The consumer measures the intermediate it walks.
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable copied be a new Seq of Int.
Repeat for x in items:
    Push x to copied.
Repeat for y in copied:
    Show y + length of copied.
"#;
    common::assert_all_tiers(source, "4\n5\n6");
    common::assert_exact_output(source, "4\n5\n6");
}

#[test]
fn deforest_no_fuse_when_the_producer_fails_after_the_consumer_would_show() {
    // The producer divides by zero on its third item, before the consumer has shown anything.
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 0 to items.
Let mutable shares be a new Seq of Int.
Repeat for x in items:
    Push 12 / x to shares.
Repeat for y in shares:
    Show y.
"#;
    let agreed = common::run_all_tiers(source);
    assert_eq!(agreed.output.trim(), "", "nothing is shown before the failure");
    assert!(
        agreed.error.as_deref().is_some_and(|e| e.contains("Division by zero")),
        "the run fails dividing by zero: {:?}",
        agreed.error
    );
    let native = common::run_logos(source);
    assert!(!native.success, "the compiled program fails too.\nGenerated Rust:\n{}", native.rust_code);
    assert_eq!(native.stdout.trim(), "", "nothing is shown before the failure.\nGenerated Rust:\n{}", native.rust_code);
}

#[test]
fn deforest_no_fuse_when_the_consumer_grows_the_producer_source() {
    // The consumer pushes to `items`, the list the producer walks.
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let mutable copied be a new Seq of Int.
Repeat for x in items:
    Push x to copied.
Let mutable total be 0.
Repeat for y in copied:
    Set total to total + y.
    Push y to items.
Show total.
Show length of items.
"#;
    common::assert_all_tiers(source, "6\n6");
    common::assert_exact_output(source, "6\n6");
}

#[test]
fn deforest_fuses_a_producer_with_no_effects_into_a_consumer_that_shows() {
    // The producer only computes and pushes, so the consumer may print, write and test between its items.
    let source = r#"## Main
Let mutable items be a new Seq of Int.
Push 3 to items.
Push 1 to items.
Push 2 to items.
Let mutable doubled be a new Seq of Int.
Repeat for x in items:
    Push x * 2 to doubled.
Let mutable shown be 0.
Repeat for y in doubled:
    If y is greater than 3:
        Show y.
        Set shown to shown + 1.
Show shown.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("let mut doubled"),
        "Intermediate 'doubled' should be eliminated by deforestation. Got:\n{}",
        rust
    );
    common::assert_all_tiers(source, "6\n4\n2");
    common::assert_exact_output(source, "6\n4\n2");
}
