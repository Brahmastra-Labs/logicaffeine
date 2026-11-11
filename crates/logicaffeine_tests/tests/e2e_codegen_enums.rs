//! E2E Codegen Tests: Enums and Pattern Matching
//!
//! Mirrors e2e_enums.rs but compiles through the Rust codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

// === UNIT VARIANTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_unit_variant() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Red.
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
    When Blue: Show "blue".
"#,
        "red",
    );
}

// === PAYLOAD VARIANTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_payload_variant() {
    assert_exact_output(
        r#"## A Shape is one of:
    A Circle with radius Int.
    A Rectangle with width Int and height Int.

## Main
Let s be a new Circle with radius 10.
Inspect s:
    When Circle (r): Show r.
    When Rectangle (w, h): Show w.
"#,
        "10",
    );
}

// === SIMPLE PATTERN MATCHING ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_inspect_simple() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Green.
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
    When Blue: Show "blue".
"#,
        "green",
    );
}

// === PATTERN MATCHING WITH BINDING ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_inspect_with_binding() {
    assert_exact_output(
        r#"## A Shape is one of:
    A Circle with radius Int.
    A Rectangle with width Int and height Int.

## Main
Let s be a new Circle with radius 42.
Inspect s:
    When Circle (r): Show r.
    When Rectangle (w, h): Show w.
"#,
        "42",
    );
}

// === OTHERWISE CLAUSE ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_inspect_otherwise() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Blue.
Inspect c:
    When Red: Show "red".
    Otherwise: Show "other".
"#,
        "other",
    );
}

// === ENUM IN FUNCTION ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_in_function() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## To isRed (c: Color) -> Bool:
    Inspect c:
        When Red: Return true.
        Otherwise: Return false.

## Main
Let c be a new Red.
Show isRed(c).
"#,
        "true",
    );
}

// === ENUM RETURN ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_return() {
    assert_exact_output(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## To primary -> Color:
    Return a new Red.

## Main
Let c be primary().
Inspect c:
    When Red: Show "red".
    Otherwise: Show "other".
"#,
        "red",
    );
}

// === NESTED INSPECT ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_nested_inspect() {
    assert_exact_output(
        r#"## A Color is one of:
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
    Otherwise: Show "not-red".
"#,
        "red-green",
    );
}

// === RECTANGLE PAYLOAD ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_enum_rectangle_area() {
    assert_exact_output(
        r#"## A Shape is one of:
    A Circle with radius Int.
    A Rectangle with width Int and height Int.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle (r): Return r * r.
        When Rectangle (w, h): Return w * h.

## Main
Let rect be a new Rectangle with width 5 and height 3.
Show area(rect).
"#,
        "15",
    );
}

// === ARMS THAT BIND NO FIELD ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_an_arm_binding_no_field_of_a_payload_variant() {
    assert_exact_output(
        r#"## A Shape is one of:
    A Circle with radius Int.
    A Rectangle with width Int and height Int.
    A Dot.

## To name (s: Shape) -> Text:
    Inspect s:
        When Circle: Return "circle".
        When Rectangle: Return "rectangle".
        When Dot: Return "dot".

## Main
Show name(a new Rectangle with width 5 and height 3).
Show name(a new Circle with radius 2).
Show name(a new Dot).
"#,
        "rectangle\ncircle\ndot",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_an_arm_binding_no_field_of_a_recursive_variant() {
    assert_exact_output(
        r#"## A Tree is one of:
    A Leaf.
    A Node with left Tree and right Tree.

## To kind (t: Tree) -> Text:
    Inspect t:
        When Leaf: Return "leaf".
        When Node: Return "node".

## Main
Show kind(a new Node with left (a new Leaf) and right (a new Leaf)).
Show kind(a new Leaf).
"#,
        "node\nleaf",
    );
}

// === A CONSTRUCTION TAKES ITS DECLARED SHAPE WHEREVER IT IS WRITTEN ===

/// A variant that holds its own enum is boxed whole; built as an argument of a call statement, it is boxed there too.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_a_recursive_variant_built_in_a_call_statements_arguments() {
    assert_exact_output(
        r#"## A Tree is one of:
    A Leaf with value Int.
    A Node with kids Seq of Tree.

## To keep (t: Tree) and (out: mutable Seq of Tree):
    Push t to out.

## To size (t: Tree) -> Int:
    Inspect t:
        When Leaf (v):
            Return 1.
        When Node (ks):
            Let mutable n be 1.
            Repeat for k in ks:
                Set n to n + size(k).
            Return n.

## Main
Let mutable out be a new Seq of Tree.
Let mutable kids be a new Seq of Tree.
Push a new Leaf with value 3 to kids.
Push a new Leaf with value 4 to kids.
keep(a new Node with kids kids, out).
keep(a new Leaf with value 5, out).
Repeat for t in out:
    Show size(t).
"#,
        "3\n1",
    );
}

/// A list parameter the function only reads, stored in a variant that holds its own enum, is a list the variant owns.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_a_recursive_variant_holds_a_list_parameter_its_function_only_reads() {
    assert_exact_output(
        r#"## A Tree is one of:
    A Leaf with value Int.
    A Node with kids Seq of Tree.

## To wrap (ks: Seq of Tree) -> Tree:
    Return a new Node with kids ks.

## To size (t: Tree) -> Int:
    Inspect t:
        When Leaf (v):
            Return 1.
        When Node (ks):
            Let mutable n be 1.
            Repeat for k in ks:
                Set n to n + size(k).
            Return n.

## Main
Let mutable kids be a new Seq of Tree.
Push a new Leaf with value 3 to kids.
Push a new Leaf with value 4 to kids.
Let t be wrap(kids).
Show size(t).
Show length of kids.
"#,
        "3\n2",
    );
}

/// A list parameter the function only reads, stored in a variant, is a copy: the caller's later push does not reach it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_a_variant_holds_a_copy_of_a_list_parameter_its_function_only_reads() {
    assert_exact_output(
        r#"## A Bag is one of:
    A Full with items Seq of Int.
    A Empty.

## To fill (xs: Seq of Int) -> Bag:
    Return a new Full with items xs.

## To count (b: Bag) -> Int:
    Inspect b:
        When Full (its):
            Return length of its.
        When Empty:
            Return 0.

## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
Let b be fill(xs).
Push 4 to xs.
Show count(b).
Show length of xs.
"#,
        "3\n4",
    );
}
