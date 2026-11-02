//! `parse_constructor_fields` (`crates/logicaffeine_language/src/parser/mod.rs`) parses a
//! constructor's `with field1 value1 and field2 value2 ...` clause. Before this lock, it had no
//! idea which type it was building: it simply consumed every `and <identifier> <value>` pair it
//! could find, so a NESTED, unparenthesized `a new …` used as one field's value greedily read
//! PAST its own closing boundary and stole a field the ENCLOSING constructor still needed
//! (`gauntlet.md`'s `tree_depth` case: the inner `Node` swallowed both `right` clauses, leaving
//! the outer `Node` with none).
//!
//! The fix threads the constructor's own type name through, looks up its declared field names
//! from the registry, and at each `and` only continues when the next word names a field THIS
//! constructor still needs — otherwise it stops and leaves the `and` for whichever constructor
//! encloses it. This boundary rule applies to a struct and a variant alike. Two further checks
//! ride on the same field list, but only one of them applies to both: a DUPLICATE field is always
//! refused (never silently overwritten), while a MISSING field is refused only for a VARIANT's
//! payload (which has no default to fall back to) — a plain struct's omitted field is a
//! documented, working feature (`LOGOS_QUICKGUIDE.md`'s `a new Point`, `http.lg`'s `Response`)
//! that defaults from its declared type exactly as it always has.
//!
//! Every positive case here is checked with `assert_all_tiers`, which requires the tree-walker
//! and the bytecode VM to agree — the corpus fixtures at the bottom of the file are the exact
//! reproductions gathered while diagnosing the bug, kept verbatim rather than re-minimized.

mod common;
use common::assert_all_tiers;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// A construction is refused, IDENTICALLY, on both the tree-walker and the VM (they share one
/// front-end parse, so a refused program never reaches either tier) — and the shared refusal
/// text contains every fragment `expect`ed.
fn assert_refused_on_both_tiers(source: &str, expect: &[&str]) {
    let tw = tw_outcome(source);
    let vm = vm_outcome(source);
    assert_eq!(
        tw.error, vm.error,
        "a parse-time refusal must be identical on both tiers (one shared front end)\nSource:\n{source}"
    );
    let message = tw.error.clone().unwrap_or_else(|| {
        panic!(
            "expected a refusal, but the tree-walker ran cleanly (output {:?})\nSource:\n{source}",
            tw.output
        )
    });
    for fragment in expect {
        assert!(
            message.contains(fragment),
            "refusal message missing {fragment:?}.\nGot: {message}\nSource:\n{source}"
        );
    }
}

// ── Census: variant, nested left (the reported bug's exact shape) ──────────────────────────────

const TREE: &str = "\
## A Tree is either:
    A Leaf.
    A Node with value Int and left Tree and right Tree.

## To depth (t: Tree) -> Int:
    Inspect t:
        When Leaf: Return 0.
        When Node (v, l, r):
            Return 1 + max(depth(l), depth(r)).
";

#[test]
fn nested_left_constructor_leaves_the_enclosing_right_field_alone() {
    let source = format!(
        "{TREE}\n## Main\n\
        Let leaf be a new Leaf.\n\
        Let t be a new Node with value 1 and left a new Node with value 2 and left leaf and right leaf and right leaf.\n\
        Show depth(t).\n"
    );
    assert_all_tiers(&source, "2");
}

#[test]
fn let_bound_inner_constructor_gives_the_same_depth() {
    // The pre-existing workaround (bind the inner node first, place it by name) never triggered
    // the bug — a control confirming the fix does not disturb this shape.
    let source = format!(
        "{TREE}\n## Main\n\
        Let leaf be a new Leaf.\n\
        Let inner be a new Node with value 2 and left leaf and right leaf.\n\
        Let t be a new Node with value 1 and left inner and right leaf.\n\
        Show depth(t).\n"
    );
    assert_all_tiers(&source, "2");
}

#[test]
fn nested_right_constructor_in_the_last_field_still_works() {
    // Nesting in the LAST field never had a trailing `and` to misattribute — a census row
    // confirming that shape keeps working, not a regression test for the fix itself.
    let source = format!(
        "{TREE}\n## Main\n\
        Let leaf be a new Leaf.\n\
        Let t be a new Node with value 1 and left leaf and right a new Node with value 2 and left leaf and right leaf.\n\
        Show depth(t).\n"
    );
    assert_all_tiers(&source, "2");
}

#[test]
fn nested_both_children_constructed_inline() {
    // Both children nested and unparenthesized: the LEFT nested constructor's own `right` field
    // must stop at ITS boundary so the outer's own `right` clause (itself a further nested
    // constructor) is not swallowed either.
    let source = format!(
        "{TREE}\n## Main\n\
        Let leaf be a new Leaf.\n\
        Let t be a new Node with value 1 and left a new Node with value 2 and left leaf and right leaf and right a new Node with value 3 and left leaf and right leaf.\n\
        Show depth(t).\n"
    );
    assert_all_tiers(&source, "2");
}

#[test]
fn generic_tree_constructor_field_boundary() {
    // The exact `generics_corpus.in` `tree_depth` shape, generic over `[T]` — the registry keys
    // a type's declared fields by its base name, independent of how it is parameterized.
    let source = "\
## A Tree of [T] is either:
    A Leaf.
    A Node with value T and left Tree of T and right Tree of T.

## To depth of [T] (t: Tree of T) -> Int:
    Inspect t:
        When Leaf: Return 0.
        When Node (v, l, r):
            Return 1 + max(depth(l), depth(r)).

## Main
Let leaf be a new Leaf.
Let t be a new Node with value 1 and left a new Node with value 2 and left leaf and right leaf and right leaf.
Show depth(t).
";
    assert_all_tiers(source, "2");
}

#[test]
fn without_max_the_same_nested_shape_still_computes_the_right_depth() {
    // A structurally different consumer of the same `Node` value (no `max()` builtin call) —
    // confirms the fix is a PARSE-level property, not something that only shows up through one
    // particular way of reading the fields back out.
    let source = "\
## A Tree is either:
    A Leaf.
    A Node with value Int and left Tree and right Tree.

## To depth (t: Tree) -> Int:
    Inspect t:
        When Leaf: Return 0.
        When Node (v, l, r):
            Let a be depth(l).
            Let b be depth(r).
            If a is greater than b:
                Return 1 + a.
            Return 1 + b.

## Main
Let leaf be a new Leaf.
Let t be a new Node with value 1 and left a new Node with value 2 and left leaf and right leaf and right leaf.
Show depth(t).
";
    assert_all_tiers(source, "2");
}

// ── Census: struct (non-variant record), nested non-last field ─────────────────────────────────

#[test]
fn plain_struct_nested_constructor_stops_at_its_own_fields() {
    let source = "\
## A Point has:
    An x: Int.
    A y: Int.

## A Circle has:
    A center: Point.
    A radius: Int.
    A label: Text.

## Main
Let c be a new Circle with center a new Point with x 1 and y 2 and radius 5 and label \"c1\".
Show c's radius.
Show c's label.
Show c's center's x.
";
    assert_all_tiers(source, "5\nc1\n1");
}

// ── Census: a three-field variant with no nesting at all (sanity: unaffected by the fix) ───────

#[test]
fn three_field_variant_with_no_nesting_still_reads_all_three_fields() {
    // Field names `p`/`q`/`r`, not `a`/`b`/`c`: a field literally named `a` collides with the
    // discovery pass's own "skip a leading article" step in its natural field-list syntax — a
    // separate, pre-existing gap (not this lock's concern).
    let source = "\
## A Tri is either:
    A Z.
    A T3 with p Int and q Int and r Int.

## Main
Let x be a new T3 with p 1 and q 2 and r 3.
Inspect x:
    When Z: Show 0.
    When T3 (p, q, r):
        Show p + q + r.
";
    assert_all_tiers(source, "6");
}

// ── Census: a missing field is refused at compile time, naming the field ───────────────────────

#[test]
fn a_missing_field_is_refused_by_name() {
    let source = "\
## A Tree is either:
    A Leaf.
    A Node with value Int and left Tree and right Tree.

## Main
Let leaf be a new Leaf.
Let t be a new Node with value 1 and left leaf.
Inspect t:
    When Leaf: Show 0.
    When Node (v, l, r): Show v.
";
    assert_refused_on_both_tiers(source, &["Node", "right"]);
}

// ── Census: a duplicate field is an error at the statement, not last-wins ──────────────────────

#[test]
fn a_duplicate_field_is_refused_not_last_wins() {
    let source = "\
## A P has:
    An x: Int.
    A y: Int.

## Main
Let p be a new P with x 1 and y 2 and y 3.
Show p.
";
    assert_refused_on_both_tiers(source, &["P", "y", "twice"]);
}

// ── Census: a field shared with the enclosing type — the ambiguous case the missing-field ──────
// ── hint exists for, and the parenthesized fix that resolves it. Both types here are VARIANTS: ─
// ── a struct's same-named field would simply default (see the struct census below), so this ────
// ── ambiguity can only ever surface a refusal for a variant's required payload. ─────────────────

const SHARED_FIELD_TYPES: &str = "\
## A Widget is either:
    A Bare.
    A Decorated with value Int and label Text.

## A Box is either:
    A Empty.
    A Wrapped with payload Widget and value Int.
";

#[test]
fn a_field_shared_with_the_enclosing_type_names_the_inner_constructor_that_may_have_consumed_it() {
    // `Wrapped` and its nested `Decorated` both declare a field named `value`. With no
    // parentheses, the trailing `and value 1` is ambiguous — this parser attributes it to the
    // INNER constructor (still active), leaving the OUTER `Wrapped` without its own `value`. The
    // refusal must name the missing field, the enclosing variant, the nested type that may have
    // taken it, and the fix.
    let source = format!(
        "{SHARED_FIELD_TYPES}\n## Main\n\
        Let b be a new Wrapped with payload a new Decorated with value 1 and label \"x\".\n\
        Show b's value.\n"
    );
    assert_refused_on_both_tiers(
        &source,
        &[
            "Wrapped",
            "value",
            "Decorated",
            "parentheses",
            "(a new Decorated with",
        ],
    );
}

#[test]
fn parenthesizing_the_inner_constructor_resolves_the_shared_field_ambiguity() {
    // The exact fix the refusal above teaches: wrapping the inner constructor in parentheses
    // closes its own `with` clause explicitly, so the outer's own `value` clause is unambiguous.
    // A variant's own fields are read back with `Inspect`, not `'s` (direct field access is a
    // struct-only construct — a variant may not even be the shape whose field is being asked
    // for), so the extraction below matches the one legitimate way to read a `Wrapped` apart.
    // `Decorated`'s own fields are given in DECLARATION order (value, then label) rather than
    // the refusal test's `label`-first order above: `interpreter.rs::execute_inspect`'s
    // `RuntimeValue::Inductive` arm binds `When Decorated (v, l):` by POSITIONAL INDEX into the
    // constructed value's `args` — which are stored in CONSTRUCTION order, not the declared
    // order `parse_when_arm` labeled the pattern's own bindings with (unlike the sibling
    // `RuntimeValue::Struct` arm two lines above it, which correctly looks fields up BY NAME).
    // Fields out of declaration order therefore bind to the WRONG pattern variables — a real,
    // separate bug (not this lock's concern) sidestepped here by writing this test's own fields in order.
    let source = format!(
        "{SHARED_FIELD_TYPES}
## Main
Let b be a new Wrapped with payload (a new Decorated with value 1 and label \"x\") and value 2.
Inspect b:
    When Empty: Show \"empty\".
    When Wrapped (payload, value):
        Inspect payload:
            When Bare: Show \"{{value}}/bare\".
            When Decorated (v, l): Show \"{{value}}/{{v}}/{{l}}\".
"
    );
    assert_all_tiers(&source, "2/1/x");
}

// ── Census: a STRUCT's omitted fields default from their declared type — never refused ─────────
// ── (the coordinator's correction: this is a documented feature, e.g. `http.lg`'s `Response`) ──

#[test]
fn a_struct_with_omitted_fields_defaults_the_rest() {
    let source = "\
## A Reply has:
    a status, which is Int.
    a note, which is Text.
    a tags, which is Seq of Int.
    a active, which is Bool.

## Main
Let r be a new Reply with status 200.
Show r's status.
Show r's note.
Show length of r's tags.
Show r's active.
";
    assert_all_tiers(source, "200\n\n0\nfalse");
}

#[test]
fn a_bare_struct_constructor_with_no_fields_at_all_defaults_everything() {
    let source = "\
## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point.
Show p's x.
Show p's y.
";
    assert_all_tiers(source, "0\n0");
}

#[test]
fn a_struct_nested_inside_a_variant_still_defaults_its_own_omitted_fields() {
    // The boundary rule (stop at an `and` that names a field this constructor does not have or
    // has already assigned) and the missing-field refusal are independent: the struct nested
    // here still stops correctly at its own field set, but its own omission is never refused.
    // `m` is read apart with `Inspect` (a variant, not a struct); the `Point` it binds supports
    // direct `'s` field access, since a struct always has exactly the one shape. The field is
    // named `loc`, not `at` — `at` lexes as `TokenType::At` (chunk-access syntax) rather than a
    // plain identifier, a separate, pre-existing reserved-word collision unrelated to this fix.
    let source = "\
## A Point has:
    An x: Int.
    A y: Int.

## A Shape is either:
    A Dot.
    A Marker with loc Point and label Text.

## Main
Let m be a new Marker with loc a new Point with x 3 and label \"here\".
Inspect m:
    When Dot: Show \"dot\".
    When Marker (loc, label):
        Show label.
        Show loc's x.
        Show loc's y.
";
    assert_all_tiers(source, "here\n3\n0");
}
