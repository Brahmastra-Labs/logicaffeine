//! ★ LOCK — one `Int`, one representation, everywhere it flows. ★
//!
//! LOGOS has ONE integer type and it is arbitrary-precision. The AOT-Rust tier has TWO runtime
//! representations of it — a bare `i64` for the fast path and `LogosInt` for values that can
//! exceed it — and which one a given SLOT gets is chosen by `codegen/bigint_promote.rs`.
//!
//! That choice was made by several sequenced passes with different rules: one for function
//! returns, one for record fields, one for arguments, one for locals. Where two of them met on the
//! same value, the generated Rust said `expected i64, found LogosInt` — well-formed code rustc
//! rejects on a type, with no `compile_error!` anywhere and every other tier perfectly happy.
//!
//! ★ WHY IT KEPT COMING BACK. Each fix added a rule to the predictor. The passes are mutually
//! recursive — a promoted FIELD promotes the functions that read it, which promotes the fields
//! THEY build — and the chain ran that recursion exactly TWICE and stopped. Two iterations is
//! enough for the shapes anybody had tried and not for the next one, so the bug returned in a new
//! disguise each time. And the facts were installed from a NARROW set at struct-definition time
//! and recomputed WIDER afterwards, under a comment asserting the two were identical: a field was
//! typed `i64` while the value filling it was typed `LogosInt`, by construction.
//!
//! ★ WHAT THIS FILE ASSERTS. Not that a particular pass says a particular thing — that would pin
//! the predictor rather than the property. It asserts the PROPERTY: every one of these programs
//! BUILDS with rustc and prints what the reference tier prints. A representation disagreement
//! anywhere in the flow is a build failure, and a wrong choice of representation is a wrong
//! number.
//!
//! JSON is the shape that found it (`json.lg` parses EXACT integers, so a `JNum` payload is
//! promoted while a user's `id` field is not) but the defect was never in JSON: it is in how one
//! LOGOS type is given two machine representations.

mod common;
use common::run_logos;

/// Builds with rustc, runs, and answers what the reference tier answers.
fn e2e(name: &str, src: &str, expected: &str) {
    let tw = logicaffeine_compile::compile::tw_outcome(src);
    assert_eq!(tw.error, None, "`{name}` on the tree-walker:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        expected,
        "`{name}`: the reference tier must agree with the test:\n{src}"
    );
    let r = run_logos(src);
    assert!(
        r.success,
        "`{name}`: the generated Rust must BUILD and RUN:\n{}\n{src}",
        r.stderr
    );
    assert_eq!(
        r.stdout.trim_end(),
        expected,
        "`{name}`: AOT must answer what the tree-walker answers:\n{src}"
    );
}

const MEMO: &str = "## A Memo has:\n    an id, which is Int.\n    a title, which is Text.\n\n";

/// A JSON document as LOGOS source. LOGOS collapses `{{` to `{` in a plain string (a bare `{`
/// opens an interpolation hole), so a JSON literal doubles its braces — written once, here,
/// rather than escaped by hand at four call sites where miscounting reads as a compiler bug.
fn json_text(doc: &str) -> String {
    format!(
        "\"{}\"",
        doc.replace('{', "{{")
            .replace('}', "}}")
            .replace('"', "\\\"")
    )
}

#[test]
fn a_decoded_json_number_fills_a_record_field() {
    // ★ THE HEADLINE. `jsonInt` answers `Result of Int and Text` whose payload is arbitrary
    // precision; `Memo.id` is a plain `Int`. This is the shape every store-backed server has, and
    // it failed to build while `largo check` passed.
    let src = MEMO.to_string()
        + "## Main\n"
        + "Inspect fromJsonSeq_Memo("
        + &json_text(r#"[{"id":7,"title":"a"}]"#)
        + "):\n"
        + "    When Ok (rows):\n"
        + "        Repeat for m in rows:\n"
        + "            Show m's id.\n"
        + "    When Err (e):\n"
        + "        Show \"decode failed: \" + e.\n";
    e2e("decode into a record", &src, "7");
}

#[test]
fn a_decoded_field_and_a_computed_one_meet_in_the_same_record() {
    // ★ THE SECOND ATTEMPT'S FAILURE, kept as a test. Promoting the field is right and strands
    // its WRITERS: `nextId(…) -> Int` is an `i64` and no longer fits the field it fills. Both
    // directions have to move together, which is what a fixpoint does and a two-pass chain does
    // not.
    let src = MEMO.to_string()
        + "## To nextId (rows: Seq of Memo) -> Int:\n"
        + "    Let mutable top be 0.\n"
        + "    Repeat for m in rows:\n"
        + "        If m's id is greater than top:\n"
        + "            Set top to m's id.\n"
        + "    Return top + 1.\n\n"
        + "## Main\n"
        + "Inspect fromJsonSeq_Memo("
        + &json_text(r#"[{"id":41,"title":"a"}]"#)
        + "):\n"
        + "    When Ok (rows):\n"
        + "        Let fresh be a new Memo with id nextId(rows) and title \"b\".\n"
        + "        Show fresh's id.\n"
        + "    When Err (e):\n"
        + "        Show e.\n";
    e2e("decoded field meets a computed one", &src, "42");
}

#[test]
fn a_record_round_trips_through_json_and_back() {
    // Encode and decode in one program, so the encoder's reader and the decoder's writer meet on
    // the same field.
    let src = MEMO.to_string()
        + "## Main\n"
        + "Let one be a new Memo with id 3 and title \"x\".\n"
        + "Inspect fromJson_Memo(toJson_Memo(one)):\n"
        + "    When Ok (back):\n"
        + "        Show back's id.\n"
        + "    When Err (e):\n"
        + "        Show e.\n";
    e2e("round trip", &src, "3");
}

#[test]
fn a_number_past_i64_is_refused_loudly_and_never_truncated() {
    // ★ NOTHING NARROWS SILENTLY. A fix that made the build pass by narrowing would answer a WRONG
    // NUMBER here — `LogosInt::to_i64` is an `Option`, and a value past `i64` would be quietly
    // replaced by whatever the fallback chose.
    //
    // `json.lg`'s locked profile rejects a mantissa beyond `i64` rather than accepting it (see
    // `json_conformance_lock`), so the correct outcome is a POSITIONED REFUSAL — and the property
    // asserted here is that the number never comes back altered. The two tiers must agree on that
    // too: a decoder that refused on one and truncated on the other would be worse than either.
    let big = "123456789012345678901234567890";
    let src = "## A Big has:\n    a n, which is Int.\n\n".to_string()
        + "## Main\n"
        + "Inspect fromJson_Big("
        + &json_text(r#"{"n":123456789012345678901234567890}"#)
        + "):\n"
        + "    When Ok (b):\n"
        + "        Show b's n.\n"
        + "    When Err (e):\n"
        + "        Show \"refused: \" + e.\n";
    let tw = logicaffeine_compile::compile::tw_outcome(&src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    let answer = tw.output.trim_end().to_string();
    assert!(
        answer.starts_with("refused: "),
        "a number past the decoder's range is refused, not accepted: {answer}"
    );
    let r = run_logos(&src);
    assert!(
        r.success,
        "the generated Rust must BUILD and RUN:\n{}\n{src}",
        r.stderr
    );
    assert_eq!(
        r.stdout.trim_end(),
        answer,
        "and both tiers refuse it the same way:\n{src}"
    );
    assert!(
        !r.stdout.contains(&big[..18]),
        "…and neither tier ever hands back a truncated version of it: {}",
        r.stdout
    );
}

#[test]
fn a_promoted_field_flows_through_a_function_and_back_into_a_record() {
    // The mutual recursion in its shortest form: a field feeds a function, the function's answer
    // feeds a field. One iteration types the first; the second needs another.
    e2e(
        "field to function to field",
        "## A Cell has:\n    a v, which is Int.\n\n\
         ## To twice (c: Cell) -> Int:\n\
        \x20   Return c's v * 2.\n\n\
         ## Main\n\
         Let a be a new Cell with v (2 ** 70).\n\
         Let b be a new Cell with v twice(a).\n\
         Show b's v.\n",
        "2361183241434822606848",
    );
}

#[test]
fn a_plain_counter_stays_on_the_fast_path() {
    // ★ PROMOTION IS NOT FREE, and a fixpoint that promoted everything would be correct and slow.
    // A linear counter cannot exceed `i64` in any run that finishes, so it must stay an `i64` —
    // the emitted Rust says so, and this is the only assertion here that reads the code rather
    // than the answer, because "it is still fast" has no observable output.
    let src = "## To count (n: Int) -> Int:\n\
              \x20   Let mutable t be 0.\n\
              \x20   Let mutable i be 1.\n\
              \x20   While i is at most n:\n\
              \x20       Set t to t + i.\n\
              \x20       Set i to i + 1.\n\
              \x20   Return t.\n\n\
               ## Main\nShow count(10).\n";
    e2e("linear counter", src, "55");
    let rust = logicaffeine_compile::compile::compile_to_rust(src).expect("compiles");
    assert!(
        !rust.contains("LogosInt"),
        "a linear counter must stay on the i64 fast path:\n{rust}"
    );
}

#[test]
fn a_record_whose_field_never_grows_stays_an_i64() {
    let src = "## A Point has:\n    an x, which is Int.\n\n\
               ## Main\nLet p be a new Point with x 3.\nShow p's x.\n";
    e2e("small record field", src, "3");
    let rust = logicaffeine_compile::compile::compile_to_rust(src).expect("compiles");
    assert!(
        rust.contains("pub x: i64") || rust.contains("x: i64"),
        "an ordinary field keeps the machine word:\n{rust}"
    );
}

#[test]
fn a_decoded_number_flows_through_a_seq_and_a_nested_record() {
    // Depth: the decoded value crosses a nested record before it is shown, so every slot on the
    // path has to have agreed.
    let src = "## An Inner has:\n    a n, which is Int.\n\n".to_string()
        + "## An Outer has:\n    an inner, which is Inner.\n    a tag, which is Text.\n\n"
        + "## Main\n"
        + "Inspect fromJson_Outer("
        + &json_text(r#"{"inner":{"n":5},"tag":"t"}"#)
        + "):\n"
        + "    When Ok (o):\n"
        + "        Show o's inner's n.\n"
        + "    When Err (e):\n"
        + "        Show e.\n";
    e2e("nested", &src, "5");
}

#[test]
fn an_int_bound_to_a_literal_and_read_only_as_a_real_keeps_the_machine_word() {
    // The range oracle keeps `seed` below 2^31 - 1, so its multiply is emitted raw, and its only other use
    // widens it to a Real. Nothing in the generated Rust then says which integer `seed` is, and a binding
    // whose type nothing fixes is an `i32` to rustc: 48271 × 48271 does not fit, and the multiply overflows.
    e2e(
        "minstd read as a Real",
        "## Main\n\
         Let mutable seed be 1.\n\
         Let mutable total be 0.0.\n\
         Repeat for i from 1 to 5:\n\
         \x20   Set seed to (seed * 48271) % 2147483647.\n\
         \x20   Set total to total + seed.\n\
         Show total.\n",
        "5467438629",
    );
}

#[test]
fn an_int_literal_past_32_bits_bound_on_its_own_builds() {
    e2e(
        "a wide literal alone",
        "## Main\nLet big be 3000000000.\nShow big.\n",
        "3000000000",
    );
}

#[test]
fn a_range_counter_between_literals_keeps_the_machine_word() {
    // The oracle bounds `i` by the range, proves `i * i` inside i64 and emits it raw; the product's only use
    // widens it to a Real. A counter whose bounds are both literals is otherwise an `i32` to rustc, and
    // 46341 × 46341 does not fit.
    e2e(
        "squares over a literal range",
        "## To squares () -> Real:\n\
         \x20   Let mutable x be 0.0.\n\
         \x20   Repeat for i from 1 to 50000:\n\
         \x20       Set x to x + i * i.\n\
         \x20   Return x.\n\n\
         ## Main\nShow squares().\n",
        "41667916675000",
    );
}

#[test]
fn a_while_counter_bound_to_a_literal_keeps_the_machine_word() {
    e2e(
        "squares over a counted while",
        "## To squares () -> Real:\n\
         \x20   Let mutable x be 0.0.\n\
         \x20   Let mutable i be 1.\n\
         \x20   While i is at most 50000:\n\
         \x20       Set x to x + i * i.\n\
         \x20       Set i to i + 1.\n\
         \x20   Return x.\n\n\
         ## Main\nShow squares().\n",
        "41667916675000",
    );
}

#[test]
fn a_counted_loop_ending_at_the_last_i32_builds() {
    // The counted-loop lowering spells an inclusive literal limit as its successor, here 2^31: past `i32`, which
    // is what a range of two bare literals would be to rustc. The counter is read only through a Real.
    e2e(
        "a loop ending at the last i32",
        "## To f () -> Real:\n\
         \x20   Let mutable x be 0.0.\n\
         \x20   Let mutable i be 2147483645.\n\
         \x20   While i is at most 2147483647:\n\
         \x20       Set x to x + i.\n\
         \x20       Set i to i + 1.\n\
         \x20   Return x.\n\n\
         ## Main\nShow f().\n",
        "6442450938",
    );
}
