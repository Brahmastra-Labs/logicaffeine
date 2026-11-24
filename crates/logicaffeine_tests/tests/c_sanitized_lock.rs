//! ★ LOCK — the C a program compiles to is defined C. ★
//!
//! A program built from either C emitter (from the AST, and from the bytecode) runs under UndefinedBehaviorSanitizer,
//! recovery off, to its end and prints what the tree-walker prints. An optimizing build can hide undefined behavior that happens to compute the
//! reference's answer on one machine — a shift by 64 on x86, whose shift instruction masks the count — and the next
//! compiler or target need not; the sanitizer stops at it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_sanitized_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::time::Duration;

use logicaffeine_compile::compile::{compile_program_full, compile_to_c, compile_to_c_via_bytecode, compile_to_rust, tw_lines};

/// The sanitizer build: every undefined behavior UBSan detects stops the program with its report.
const SANITIZED: &[&str] = &[
    "-std=gnu17",
    "-O1",
    "-g",
    "-fsanitize=undefined",
    "-fno-sanitize-recover=all",
    "-fno-omit-frame-pointer",
];

/// How long a sanitized program may run.
const LIMIT: Duration = Duration::from_secs(120);

/// The two C emitters.
const EMITTERS: [(&str, fn(&str) -> Result<String, logicaffeine_compile::ParseError>); 2] =
    [("AST C", compile_to_c), ("bytecode C", compile_to_c_via_bytecode)];

/// `c`, built sanitized: what it prints, or why it stopped.
fn sanitized(c: &str, src: &str) -> common::TimedRun {
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = std::process::Command::new(common::c_compiler())
        .args(SANITIZED)
        .arg(&c_path)
        .arg("-o")
        .arg(&bin)
        .arg("-lm")
        .output()
        .expect("run the C compiler");
    assert!(
        built.status.success(),
        "the sanitized C does not build:\n{}\nfor:\n{src}",
        String::from_utf8_lossy(&built.stderr)
    );
    common::run_within(&bin, &[], LIMIT)
}

/// `src`, through each C emitter, runs as defined C and prints what the tree-walker prints.
fn runs_defined(src: &str) {
    runs_defined_by(&EMITTERS, src);
}

/// `src`, through each of `emitters`, runs as defined C and prints what the tree-walker prints.
fn runs_defined_by(emitters: &[(&str, fn(&str) -> Result<String, logicaffeine_compile::ParseError>)], src: &str) {
    let tw = tw_lines(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    let expected: String = tw.lines.iter().map(|l| format!("{l}\n")).collect();
    for &(emitter, emit) in emitters {
        let c = emit(src).unwrap_or_else(|e| panic!("no {emitter} for:\n{src}\n{e:?}"));
        let run = sanitized(&c, src);
        assert!(!run.timed_out, "{emitter}: still running after {LIMIT:?}:\n{src}");
        assert!(
            run.success && !run.stderr.contains("runtime error:"),
            "{emitter} stops at undefined behavior:\n{}\nfor:\n{src}",
            run.stderr
        );
        assert_eq!(run.stdout, expected, "{emitter} prints other than the tree-walker for:\n{src}");
    }
}

/// A shift takes its count modulo 64 and shifts the bits of a negative operand as of any other (the reference's
/// `wrapping_shl`/`wrapping_shr`): no count and no operand is undefined.
#[test]
fn a_shift_by_any_count_is_defined() {
    runs_defined(
        "## Main
Let counts be [0, 1, 60, 63, 64, 65, 127, 0 - 1, 0 - 64].
Repeat for c in counts:
    Show 1 shifted left by c.
    Show (0 - 8) shifted left by c.
    Show 1024 shifted right by c.
    Show (0 - 1024) shifted right by c.
",
    );
}

/// `abs` of an `Int` is the `Int` it is, every digit kept (past 2^53 a `double` would round it), and of a `Real` the
/// `Real` it is.
#[test]
fn abs_keeps_the_kind_of_number_it_is_given() {
    runs_defined(
        "## Main
Let big be 0 - 9007199254740993.
Show abs(big).
Show abs(0 - 7).
Show abs(12).
Let r be 0 - 2.5.
Show abs(r).
Show abs(r) * 2.
",
    );
}

/// `{a, b, …}` is the set its items make: each item once, in the order first written, and it takes part in set
/// operations as any set does.
#[test]
fn a_set_literal_is_the_set_its_items_make() {
    runs_defined(
        "## Main
Let mutable seen be a new Set of Int.
Add 4 to seen.
Add 9 to seen.
Let evens be {2, 4, 6, 4}.
Show length of evens.
Show evens contains 6.
Let both be seen union evens.
Show length of both.
Let names be {\"ada\", \"bo\", \"ada\"}.
Show length of names.
Show names contains \"bo\".
",
    );
}

/// `x is none` asks which variant an `Option` holds, as `Inspect` does.
#[test]
fn an_option_compares_with_none_by_its_variant() {
    runs_defined(
        "## To find (xs: Seq of Int, target: Int) -> Option of Int:
    Repeat for i from 1 to length of xs:
        If item i of xs equals target:
            Return some i.
    Return none.

## Main
Let missing be find([4, 7, 9], 5).
If missing is none:
    Show \"absent\".
Let found be find([4, 7, 9], 7).
If found is none:
    Show \"absent too\".
Otherwise:
    Show \"found\".
",
    );
}

/// A copy of a list with nothing in it is an empty list, made without handing the C library the empty list's absent
/// buffer.
#[test]
fn copying_an_empty_list_is_defined() {
    runs_defined(
        "## Main
Let empty be a new Seq of Int.
Let copied be copy of empty.
Show length of copied.
Let flags be a new Seq of Bool.
Let flagged be copy of flags.
Show length of flagged.
Let reals be a new Seq of Real.
Let copiedReals be copy of reals.
Show length of copiedReals.
Let mutable xs be a new Seq of Int.
Push 3 to xs.
Let ys be copy of xs.
Show item 1 of ys.
",
    );
}

/// `Increase`/`Decrease` of a plain `Int` field add the amount, and `Merge` adds the incoming field (`crdt_counter_bump`, `crdt_merge_field`).
#[test]
fn a_plain_int_field_is_increased_and_merged_by_adding() {
    runs_defined(
        "## A Counter has:
    n: Int.
## Main
    Let mutable a be a new Counter with n 1.
    Let mutable b be a new Counter with n 10.
    Increase a's n by 5.
    Decrease a's n by 2.
    Merge b into a.
    Show a's n.
",
    );
}

/// `Increase` of a plain `Int` field changes the field the record holds.
#[test]
fn a_plain_int_field_is_increased_in_place() {
    runs_defined(
        "## A C has:
    a points, which is Int.

## Main
Let mutable c be a new C with points 5.
Increase c's points by 3.
Show c's points.
",
    );
}

/// `Merge` unions a set field and folds a map field key by key, a shared key's `Int`s adding; `Converge` does too, a nested `Int` adding.
#[test]
fn a_merge_folds_a_map_key_by_key_and_unions_a_set() {
    runs_defined(
        "## Definition
A Pool has:
    a tags: Set of Text.
    a counts: Map of Text to Int.

## Main
Let mutable a be a new Pool.
Let mutable b be a new Pool.
Add \"x\" to a's tags.
Add \"y\" to b's tags.
Set a's counts[\"k\"] to 1.
Set b's counts[\"k\"] to 2.
Set b's counts[\"j\"] to 7.
Merge b into a.
Show a's tags.
Show a's counts.
Converge b into a.
Show a's counts.
",
    );
}

/// A list `Merge` brings in is the target's own from then on: a push to either record shows in that record alone.
#[test]
fn a_merged_list_is_the_targets_own() {
    runs_defined(
        "## A Bag has:
    An items: Seq of Int.

## Main
Let mutable a be a new Bag with items [1].
Push 2 to a's items.
Let mutable b be a new Bag with items [5].
Push 6 to b's items.
Merge b into a.
Push 7 to a's items.
Push 8 to b's items.
Show a's items.
Show b's items.
",
    );
}

/// A record's field declared `Option of Int` holds that option: it is built with `some`/`none`, set, and inspected.
#[test]
fn a_record_field_holding_an_option_is_that_option() {
    runs_defined(
        "## A Box has:
    a n, which is Option of Int.
    a label, which is Text.

## To describe (b: Box) -> Text:
    Inspect b's n:
        When Some (v): Return b's label + \" holds \" + v.
        When None: Return b's label + \" is empty\".

## Main
Let full be a new Box with label \"left\" and n some(3).
Let empty be a new Box with label \"right\" and n none.
Show describe(full).
Show describe(empty).
Let mutable m be full.
Set m's n to some(9).
Show describe(m).
Show describe(full).
",
    );
}

/// A variant's field declared `Option of Int` holds that option.
#[test]
fn a_variant_field_holding_an_option_is_that_option() {
    runs_defined(
        "## A Shape is either:
    A Circle with a radius, which is Option of Int.
    A Dot.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle (radius):
            Inspect radius:
                When Some (r): Return r * r * 3.
                When None: Return -1.
        When Dot: Return 0.

## Main
Show area(a new Circle with radius some(2)).
Show area(a new Circle with radius none).
Show area(a new Dot).
",
    );
}

/// A field holding an option of a record, a sequence of options, a `Maybe`, or a `Result` holds what it says.
#[test]
fn a_field_holding_an_applied_option_or_result_is_that_type() {
    runs_defined(
        "## A Point has:
    An x: Int.
    A y: Int.

## A Marker has:
    a at, which is Option of Point.

## Main
Let m be a new Marker with at some(a new Point with x 3 and y 4).
Inspect m's at:
    When Some (p): Show p's x + p's y.
    When None: Show \"nowhere\".
Let e be a new Marker with at none.
Inspect e's at:
    When Some (p): Show p's x.
    When None: Show \"nowhere\".
",
    );
    runs_defined(
        "## A Shelf has:
    a slots, which is Seq of (Option of Int).
    a name, which is Text.

## Main
Let mutable xs be a new Seq of (Option of Int).
Push some(4) to xs.
Push none to xs.
Push some(6) to xs.
Let s be a new Shelf with name \"top\" and slots xs.
Let mutable total be 0.
Repeat for x in s's slots:
    Inspect x:
        When Some (v): Set total to total + v.
        When None: Set total to total + 100.
Show total.
Show length of s's slots.
",
    );
    runs_defined(
        "## A Slot has:
    a held, which is Maybe of Text.

## Main
Let s be a new Slot with held some(\"x\").
Inspect s's held:
    When Some (v): Show v.
    When None: Show \"empty\".
",
    );
    runs_defined(
        "## A Job has:
    a outcome, which is Result of Int and Text.
    a id, which is Int.

## Main
Let j be a new Job with id 4 and outcome (a new Ok with value 10).
Inspect j's outcome:
    When Ok (v): Show v + j's id.
    When Err (e): Show e.
Let k be a new Job with id 5 and outcome (a new Err with error \"bad\").
Inspect k's outcome:
    When Ok (v): Show v.
    When Err (e): Show e.
",
    );
}

/// A generic record's field holding an `Option` that names none of its parameters holds that option, and a record
/// declared without parameters holds an applied generic record as that record.
#[test]
fn a_field_holding_an_applied_generic_holds_its_instance() {
    runs_defined(
        "## A Slot of [T] has:
    a value, which is T.
    a spare, which is Option of Int.

## Main
Let s be a new Slot of Text with value \"a\" and spare some(7).
Inspect s's spare:
    When Some (v): Show s's value + \" \" + v.
    When None: Show s's value.
",
    );
    runs_defined(
        "## A Wrap of [T] has:
    a inner, which is T.

## A Holder has:
    a w, which is Wrap of Int.
    a name, which is Text.

## Main
Let h be a new Holder with w (a new Wrap of Int with inner 41) and name \"h\".
Show h's w's inner + 1.
Show h's name.
",
    );
}

/// A record holding an `Option of Int`, written as JSON with the value and without it.
const JSON_OF_A_BOX: &str = "## A Box has:
    a n, which is Option of Int.

## Main
Show toJson_Box(a new Box with n some(3)).
Show toJson_Box(a new Box with n none).
";

/// A record holding an `Option of Text`, read back from the JSON it is written as.
const JSON_OF_A_TAG: &str = "## A Tag has:
    a tag, which is Option of Text.

## Main
Inspect fromJson_Tag(toJson_Tag(a new Tag with tag some(\"hi\"))):
    When Ok (b): Show b's tag.
    When Err (e): Show \"ERR \" + e.
";

/// The JSON reader and writer every record has hold a field declared `Option of …`: `null` when it is `none`.
#[test]
fn a_record_holding_an_option_reads_and_writes_json() {
    runs_defined(JSON_OF_A_BOX);
    runs_defined(JSON_OF_A_TAG);
}

/// The same JSON reader and writer on the deploy tier: AOT Rust, the only tier that type-checks a program, writes them
/// with no `compile_error!`, builds them, and prints what the tree-walker prints.
#[test]
fn a_record_holding_an_option_reads_and_writes_json_on_the_deploy_tier() {
    for src in [JSON_OF_A_BOX, JSON_OF_A_TAG] {
        let rust = compile_to_rust(src).unwrap_or_else(|e| panic!("no Rust for:\n{src}\n{e:?}"));
        assert!(!rust.contains("compile_error!"), "the Rust refuses part of:\n{src}\n{rust}");
        let tw = tw_lines(src);
        assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
        let expected: String = tw.lines.iter().map(|l| format!("{l}\n")).collect();
        let full = compile_program_full(src).unwrap_or_else(|e| panic!("no Rust for:\n{src}\n{e:?}"));
        let built = common::build_generated(&full.rust_code, &full.dependencies, common::RustProfile::Release)
            .unwrap_or_else(|why| panic!("the Rust does not build:\n{why}\nfor:\n{src}"));
        let run = common::run_within(&built.binary, &[], LIMIT);
        assert!(run.success, "AOT Rust stops:\n{}\nfor:\n{src}", run.stderr);
        assert_eq!(run.stdout, expected, "AOT Rust prints other than the tree-walker for:\n{src}");
    }
}

/// A value that cannot be absent is never equal to `nothing`, and is still computed beside it; `nothing` is equal to
/// itself.
#[test]
fn a_value_is_never_equal_to_nothing() {
    runs_defined(
        "## To noisy () -> Int:
    Show \"computed\".
    Return 0.

## Main
Show false is equal to nothing.
Show nothing is equal to false.
Show false is not equal to nothing.
Show nothing is not equal to false.
Show 0 is equal to nothing.
Show nothing is equal to 0.
Show 0 is not equal to nothing.
Show nothing is not equal to 0.
Show nothing is equal to nothing.
Show nothing is not equal to nothing.
Let text be \"\".
Let items be a new Seq of Int.
Show text is equal to nothing.
Show nothing is equal to text.
Show items is equal to nothing.
Show nothing is equal to items.
Show noisy() is equal to nothing.
",
    );
}

/// An `Option` is equal to `nothing` when it is `none`, however it was made, and to another `Option` when both are
/// `none` or both hold equal values.
#[test]
fn an_option_is_equal_to_nothing_when_it_is_none() {
    runs_defined(
        "## Main
Let v be none.
If v is equal to nothing:
    Show \"absent\".
Otherwise:
    Show \"present\".
Let present be some 3.
Let absent be none.
Let copied be copy of absent.
Show present is equal to absent.
Show absent is equal to present.
Show copied is equal to absent.
Show copied is equal to nothing.
Show present is equal to nothing.
Show nothing is not equal to present.
",
    );
}

/// Two options, two records, two values of an enum, and two lists of records are equal when what they hold is: an
/// enum value by its variant and then its fields, a tree by its subtrees.
#[test]
fn structured_values_are_equal_when_what_they_hold_is() {
    runs_defined(
        "## A P has:
    an x: Int.
    a tag: Text.

## A Shape is either:
    A Circle with a radius, which is Int.
    A Rect with a w, which is Int, and a h, which is Int.
    A Dot.

## A Tree is either:
    A Leaf.
    A Node with a left, which is Tree, and a value, which is Int, and a right, which is Tree.

## Main
Let a be some 3.
Let b be some 3.
Let c be none.
Let d be some 4.
Show a is equal to b.
Show a is equal to c.
Show a is not equal to d.
Let s be some \"x\".
Let u be some \"x\".
Show s is equal to u.
Let p be a new P with x 1 and tag \"k\".
Let q be a new P with x 1 and tag \"k\".
Let r be a new P with x 1 and tag \"j\".
Show p is equal to q.
Show p is equal to r.
Show [p, q] is equal to [q, p].
Show [p, q] is equal to [p, r].
Let c1 be a new Circle with radius 3.
Let c2 be a new Circle with radius 3.
Let c3 be a new Circle with radius 4.
Let r1 be a new Rect with w 3 and h 3.
Let d1 be a new Dot.
Let d2 be a new Dot.
Show c1 is equal to c2.
Show c1 is equal to c3.
Show c1 is equal to r1.
Show d1 is equal to d2.
Show d1 is not equal to c1.
Let t1 be a new Node with left (a new Leaf) and value 2 and right (a new Node with left (a new Leaf) and value 5 and right (a new Leaf)).
Let t2 be a new Node with left (a new Leaf) and value 2 and right (a new Node with left (a new Leaf) and value 5 and right (a new Leaf)).
Let t3 be a new Node with left (a new Leaf) and value 2 and right (a new Leaf).
Show t1 is equal to t2.
Show t1 is equal to t3.
",
    );
}

/// A map holds records, enum values and options as what they are, under text or integer keys: each is set, read,
/// looked for, removed, shown and compared.
#[test]
fn a_map_holds_records_enum_values_and_options() {
    runs_defined(
        "## A Shape is either:
    A Circle with a radius, which is Int.
    A Dot.

## Main
Let mutable m be a new Map of Text to Shape.
Set item \"a\" of m to a new Circle with radius 3.
Set item \"b\" of m to a new Dot.
Inspect item \"a\" of m:
    When Circle (radius): Show radius.
    When Dot: Show \"dot\".
Inspect item \"b\" of m:
    When Circle (radius): Show radius.
    When Dot: Show \"dot\".
",
    );
    runs_defined(
        "## Main
Let mutable m be a new Map of Text to (Option of Int).
Set item \"a\" of m to some(5).
Set item \"b\" of m to none.
Inspect item \"a\" of m:
    When Some (v): Show v.
    When None: Show \"none\".
Inspect item \"b\" of m:
    When Some (v): Show v.
    When None: Show \"none\".
",
    );
    runs_defined(
        "## A Table has:
    a cells, which is Map of Text to (Option of Int).

## Main
Let mutable m be a new Map of Text to (Option of Int).
Set item \"a\" of m to some(5).
Set item \"b\" of m to none.
Let t be a new Table with cells m.
Inspect item \"a\" of t's cells:
    When Some (v): Show v.
    When None: Show \"none\".
Inspect item \"b\" of t's cells:
    When Some (v): Show v.
    When None: Show \"none\".
",
    );
    runs_defined(
        "## A Point has:
    An x: Int.
    A y: Int.

## Main
Let mutable m be a new Map of Int to Point.
Set item 1 of m to a new Point with x 3 and y 4.
Set item 7 of m to a new Point with x 5 and y 6.
Let far be item 7 of m.
Let near be item 1 of m.
Show far's x + near's y.
Show length of m.
Show m contains 7.
Show m contains 2.
Show m.
Remove 1 from m.
Show length of m.
Let mutable n be a new Map of Int to Point.
Set item 7 of n to a new Point with x 5 and y 6.
Show m is equal to n.
",
    );
}

/// A record holds a map of records or of lists, and a loop over the map binds each entry, key and value.
#[test]
fn a_record_holds_a_map_of_records_that_iterates_and_shows() {
    runs_defined(
        "## A Point has:
    An x: Int.
    A y: Int.

## A Atlas has:
    a places, which is Map of Text to Point.
    a name, which is Text.

## Main
Let mutable m be a new Map of Text to Point.
Set item \"home\" of m to a new Point with x 1 and y 2.
Set item \"work\" of m to a new Point with x 8 and y 9.
Let a be a new Atlas with name \"city\" and places m.
Let places be a's places.
Let work be item \"work\" of places.
Show work's x.
Repeat for k in a's places:
    Show k.
Show a's places.
Show a.
",
    );
    runs_defined(
        "## A Index has:
    a lists, which is Map of Text to Seq of Int.

## Main
Let mutable m be a new Map of Text to Seq of Int.
Set item \"a\" of m to [1, 2, 3].
Let ix be a new Index with lists m.
Show length of item \"a\" of ix's lists.
Show ix.
",
    );
}

/// An integer-keyed map holds a value type. Bytecode C leaves value types to the AST emitter (it refuses the
/// program, and the build takes the AST emitter's C), so the row holds the AST emitter's.
#[test]
fn an_integer_keyed_map_holds_a_value_type() {
    runs_defined_by(
        &EMITTERS[..1],
        "## Main
Let mutable m be a new Map of Int to Money.
Set item 1 of m to $5.25.
Set item 2 of m to $1.75.
Show item 1 of m + item 2 of m.
Show length of m.
",
    );
}

/// The lock is a check that can fail: a shift past the word in hand-written C stops under the sanitizer.
#[test]
fn the_sanitizer_stops_a_program_at_undefined_behavior() {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("ub.c");
    std::fs::write(
        &src,
        "#include <stdio.h>\nint main(int argc, char **argv) { (void)argv; long long n = 63 + argc; printf(\"%lld\\n\", 1LL << n); return 0; }\n",
    )
    .unwrap();
    let bin = dir.path().join("ub");
    let built = std::process::Command::new(common::c_compiler())
        .args(SANITIZED)
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("run the C compiler");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let run = common::run_within(&bin, &[], LIMIT);
    assert!(!run.success && run.stderr.contains("runtime error:"), "{}", run.stderr);
}
