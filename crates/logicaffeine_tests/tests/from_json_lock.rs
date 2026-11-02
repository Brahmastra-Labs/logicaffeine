//! ★ LOCK — every record gets a `fromJson_<T>` written for it. ★
//!
//! The decoder half of the derive. `toJson_<T>` made a record into wire text; this makes wire
//! text back into a record, so a route's answer and a page's `Get … as T` are two ends of ONE
//! contract rather than two hand-written serialisers that agree until they don't.
//!
//! It answers a `Result of T and Text`, because the text came from somewhere else: a field that
//! is missing, or holds the wrong kind of value, is a normal thing for a stranger's JSON to do
//! and must produce an answer rather than kill the program. A decoder that trapped would make
//! every server a denial of service away from a malformed body.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Both tiers, same answer — the wire is a contract, so the two tiers must decode identically.
fn both(src: &str, want: &[&str]) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it:\n{src}");
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        want,
        "tree-walker:\n{src}"
    );
    assert_eq!(vm.output.lines().collect::<Vec<_>>(), want, "VM:\n{src}");
}

/// A JSON literal as LOGOS source. `{` opens an INTERPOLATION HOLE in a LOGOS string, so a JSON
/// object cannot be written as one — the bytes are spelled out instead, which is what
/// `json_typed_decode_lock` does for the same reason.
fn lg_text(s: &str) -> String {
    let bytes: Vec<String> = s.bytes().map(|b| b.to_string()).collect();
    format!("text_from_bytes([{}])", bytes.join(", "))
}

const TODO: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n    \
    a done, which is Bool.\n\n";

/// Show the decoded record's fields, or the error.
const SHOW: &str =
    "Inspect r:\n    When Ok (v):\n        Show v's id.\n        Show v's text.\n        \
    Show v's done.\n    When Err (e):\n        Show \"err: \" + e.\n";

#[test]
fn a_record_decodes_from_the_json_its_own_encoder_writes() {
    // The round trip is the property that matters: whatever `toJson_` writes, `fromJson_` reads.
    let src = format!(
        "{TODO}## Main\nLet t be a new Todo with id 7 and text \"milk\" and done true.\n\
         Let r be fromJson_Todo(toJson_Todo(t)).\n{SHOW}"
    );
    both(&src, &["7", "milk", "true"]);
}

#[test]
fn a_record_decodes_from_json_written_by_hand() {
    let src = format!(
        "{TODO}## Main\nLet r be fromJson_Todo({}).\n{SHOW}",
        lg_text(r#"{"id":1,"text":"a","done":false}"#)
    );
    both(&src, &["1", "a", "false"]);
}

#[test]
fn field_order_on_the_wire_does_not_matter() {
    // JSON objects are unordered. A decoder that read positionally would work against its own
    // encoder and fail against every other producer on earth.
    let src = format!(
        "{TODO}## Main\nLet r be fromJson_Todo({}).\n{SHOW}",
        lg_text(r#"{"done":true,"text":"z","id":9}"#)
    );
    both(&src, &["9", "z", "true"]);
}

#[test]
fn a_missing_field_is_an_error_and_not_a_zero() {
    // The silent-wrong-answer this lock exists to prevent: defaulting a missing field would hand
    // the program a Todo with id 0 that nobody wrote and no error anyone could see.
    let src = format!(
        "{TODO}## Main\nLet r be fromJson_Todo({}).\n{SHOW}",
        lg_text(r#"{"text":"a","done":false}"#)
    );
    let tw = tw_outcome(&src);
    assert_eq!(tw.error, None, "it must ANSWER, never trap:\n{src}");
    let out = tw.output.trim().to_string();
    assert!(
        out.starts_with("err: "),
        "a missing field is an error: {out}"
    );
    assert!(out.contains("id"), "and the error names the field: {out}");
}

#[test]
fn a_field_of_the_wrong_kind_is_an_error() {
    let src = format!(
        "{TODO}## Main\nLet r be fromJson_Todo({}).\n{SHOW}",
        lg_text(r#"{"id":"seven","text":"a","done":false}"#)
    );
    let out = tw_outcome(&src).output.trim().to_string();
    assert!(
        out.starts_with("err: "),
        "a wrong-typed field is an error: {out}"
    );
}

#[test]
fn malformed_json_is_an_error_rather_than_a_trap() {
    // The text is a stranger's. A decoder that trapped on it would make a server one bad body
    // away from going down.
    let src = format!("{TODO}## Main\nLet r be fromJson_Todo(\"not json at all\").\n{SHOW}");
    let tw = tw_outcome(&src);
    assert_eq!(tw.error, None, "it must ANSWER, never trap:\n{src}");
    assert!(tw.output.trim().starts_with("err: "), "got: {}", tw.output);
}

#[test]
fn an_empty_text_is_an_error_rather_than_an_empty_record() {
    let src = format!("{TODO}## Main\nLet r be fromJson_Todo(\"\").\n{SHOW}");
    let tw = tw_outcome(&src);
    assert_eq!(tw.error, None, "it must ANSWER, never trap:\n{src}");
    assert!(tw.output.trim().starts_with("err: "), "got: {}", tw.output);
}

#[test]
fn text_with_escapes_survives_the_round_trip() {
    // The characters that break a serialiser: a quote ends the string it lives in, a backslash
    // eats the character after it.
    let src = format!(
        "{TODO}## Main\nLet t be a new Todo with id 1 and text \"a\\\"b\\\\c\" and done false.\n\
         Let r be fromJson_Todo(toJson_Todo(t)).\n{SHOW}"
    );
    both(&src, &["1", "a\"b\\c", "false"]);
}

#[test]
fn a_nested_record_round_trips_through_both_derives() {
    let src = "## An Inner has:\n    a v, which is Int.\n\n\
        ## An Outer has:\n    a label, which is Text.\n    an inner, which is Inner.\n\n\
        ## Main\nLet o be a new Outer with label \"o\" and inner (a new Inner with v 3).\n\
        Let r be fromJson_Outer(toJson_Outer(o)).\n\
        Inspect r:\n    When Ok (v):\n        Show v's label.\n        Show v's inner's v.\n    \
        When Err (e):\n        Show \"err: \" + e.\n";
    both(src, &["o", "3"]);
}

#[test]
fn a_broken_nested_record_reports_the_error_rather_than_half_a_value() {
    let src = &format!(
        "## An Inner has:\n    a v, which is Int.\n\n\
        ## An Outer has:\n    a label, which is Text.\n    an inner, which is Inner.\n\n\
        ## Main\nLet r be fromJson_Outer({}).\n\
        Inspect r:\n    When Ok (v):\n        Show v's label.\n    When Err (e):\n        \
        Show \"err: \" + e.\n",
        lg_text(r#"{"label":"o","inner":{}}"#)
    );
    let out = tw_outcome(src).output.trim().to_string();
    assert!(
        out.starts_with("err: "),
        "a broken inner record fails the whole decode: {out}"
    );
}

#[test]
fn a_hand_written_decoder_is_never_shadowed() {
    // Declarer wins, the same rule `toJson_` follows and the same rule the stdlib prelude does.
    let src = format!(
        "{TODO}## To fromJson_Todo (t: Text) -> Result of Todo and Text:\n    \
         Return a new Err with error \"mine\".\n\n\
         ## Main\nLet r be fromJson_Todo(\"nope\").\n{SHOW}"
    );
    both(&src, &["err: mine"]);
}

#[test]
fn a_record_nobody_decodes_gets_no_decoder() {
    // The reachability rule `toJson_` learned the hard way: a derive that fires on DECLARATION
    // taxes every program that merely names the shape, and a function nobody calls has no call
    // site to type its parameter from.
    let src = "## A Tally has:\n    an n, which is Int.\n\n\
        ## Main\nLet t be a new Tally with n 3.\nShow t's n.\n";
    both(src, &["3"]);
    let program = logicaffeine_compile::compile::compile_to_program_optimized(src)
        .unwrap_or_else(|e| panic!("the program must compile: {e:?}"));
    logicaffeine_compile::codegen_c::from_program::compile_program_to_c(&program)
        .unwrap_or_else(|e| panic!("the Op-walker must lower it: {e}"));
}

// ---- Sequences: the shape a list page actually asks for. ----

/// The list decoder. `Seq of Todo` is what a page that shows a list asks a server for, so a
/// decoder that only handled single records would leave the most ordinary request in the
/// framework unserved.
#[test]
fn a_sequence_of_records_decodes() {
    let src = format!(
        "{TODO}## Main\nLet r be fromJsonSeq_Todo({}).\n\
         Inspect r:\n    When Ok (xs):\n        Show length of xs.\n        \
         Let a be item 1 of xs.\n        Show a's text.\n        \
         Let b be item 2 of xs.\n        Show b's text.\n    \
         When Err (e):\n        Show \"err: \" + e.\n",
        lg_text(r#"[{"id":1,"text":"a","done":false},{"id":2,"text":"b","done":true}]"#)
    );
    both(&src, &["2", "a", "b"]);
}

#[test]
fn an_empty_array_decodes_to_an_empty_sequence() {
    // Not an error, and not a missing answer: a server with nothing to send says `[]`.
    let src = format!(
        "{TODO}## Main\nLet r be fromJsonSeq_Todo({}).\n\
         Inspect r:\n    When Ok (xs):\n        Show length of xs.\n    \
         When Err (e):\n        Show \"err: \" + e.\n",
        lg_text("[]")
    );
    both(&src, &["0"]);
}

#[test]
fn one_bad_element_fails_the_whole_sequence() {
    // All-or-nothing. Skipping the element that would not decode would hand the page a shorter
    // list than the server sent, with nothing anywhere to say so.
    let src = format!(
        "{TODO}## Main\nLet r be fromJsonSeq_Todo({}).\n\
         Inspect r:\n    When Ok (xs):\n        Show length of xs.\n    \
         When Err (e):\n        Show \"err: \" + e.\n",
        lg_text(r#"[{"id":1,"text":"a","done":false},{"id":2}]"#)
    );
    let out = tw_outcome(&src).output.trim().to_string();
    assert!(
        out.starts_with("err: "),
        "a broken element fails the decode: {out}"
    );
}

#[test]
fn an_object_where_an_array_was_expected_is_an_error() {
    let src = format!(
        "{TODO}## Main\nLet r be fromJsonSeq_Todo({}).\n\
         Inspect r:\n    When Ok (xs):\n        Show length of xs.\n    \
         When Err (e):\n        Show \"err: \" + e.\n",
        lg_text(r#"{"id":1,"text":"a","done":false}"#)
    );
    let out = tw_outcome(&src).output.trim().to_string();
    assert!(
        out.starts_with("err: "),
        "an object is not a sequence: {out}"
    );
}

#[test]
fn a_sequence_round_trips_against_the_encoder() {
    // The property that matters, at the list level.
    let src = format!(
        "{TODO}## Main\n\
         Let a be a new Todo with id 1 and text \"x\" and done false.\n\
         Let b be a new Todo with id 2 and text \"y\" and done true.\n\
         Let both be [a, b].\n\
         Let wire be \"[\" + toJson_Todo(a) + \",\" + toJson_Todo(b) + \"]\".\n\
         Let r be fromJsonSeq_Todo(wire).\n\
         Inspect r:\n    When Ok (xs):\n        Let b be item 2 of xs.\n        \
         Show b's text.\n    When Err (e):\n        Show \"err: \" + e.\n"
    );
    both(&src, &["y"]);
}

// ---------------------------------------------------------------------------------------------
// ★ THE OPTION ROUND TRIP — what `toJson_` writes, `fromJson_` must read. ★
// ---------------------------------------------------------------------------------------------

#[test]
fn an_optional_text_survives_the_round_trip_present() {
    // The encoder learned to write `"tag":"hi"`; a decoder that could not read it would leave the
    // two halves of ONE contract disagreeing — which is the whole reason both are derived from
    // the same declaration rather than hand-written.
    both(
        "## A Box has:\n\
        \x20   a tag, which is Option of Text.\n\
         \n\
         ## Main\n\
         Inspect fromJson_Box(toJson_Box(a new Box with tag some(\"hi\"))):\n\
        \x20   When Ok (b): Show b's tag.\n\
        \x20   When Err (e): Show \"ERR \" + e.\n",
        &["hi"],
    );
}

#[test]
fn an_absent_optional_comes_back_absent_and_not_as_empty_text() {
    // `null` and `""` are different answers — a cleared field and an unset one — and a decoder
    // that folded one into the other would hand the program a value nobody sent.
    both(
        "## A Box has:\n\
        \x20   a tag, which is Option of Text.\n\
         \n\
         ## Main\n\
         Inspect fromJson_Box(toJson_Box(a new Box with tag none)):\n\
        \x20   When Ok (b):\n\
        \x20       If b's tag is none:\n\
        \x20           Show \"absent\".\n\
        \x20   When Err (e): Show \"ERR \" + e.\n",
        &["absent"],
    );
}

#[test]
fn an_optional_number_round_trips_both_ways() {
    both(
        "## A Box has:\n\
        \x20   a n, which is Option of Int.\n\
         \n\
         ## Main\n\
         Inspect fromJson_Box(toJson_Box(a new Box with n some(7))):\n\
        \x20   When Ok (b): Show b's n.\n\
        \x20   When Err (e): Show \"ERR \" + e.\n\
         Inspect fromJson_Box(toJson_Box(a new Box with n none)):\n\
        \x20   When Ok (b):\n\
        \x20       If b's n is none:\n\
        \x20           Show \"absent\".\n\
        \x20   When Err (e): Show \"ERR \" + e.\n",
        &["7", "absent"],
    );
}

#[test]
fn an_optional_field_missing_from_the_document_is_absent_rather_than_an_error() {
    // ★ THE ONE THAT DECIDES THE DESIGN. A required field that is missing is an ERROR (the sender
    // owed it), but an OPTIONAL one that is missing is exactly what optional MEANS — and it is
    // what an older sender that predates the field produces. Refusing it would make adding an
    // optional field a breaking change for every client already deployed.
    both(
        "## A Box has:\n\
        \x20   an id, which is Int.\n\
        \x20   a tag, which is Option of Text.\n\
         \n\
         ## Main\n\
         Inspect fromJson_Box(text_from_bytes([123, 34, 105, 100, 34, 58, 52, 125])):\n\
        \x20   When Ok (b):\n\
        \x20       If b's tag is none:\n\
        \x20           Show \"absent \" + b's id.\n\
        \x20   When Err (e): Show \"ERR \" + e.\n",
        &["absent 4"],
    );
}

#[test]
fn an_optional_field_of_the_wrong_kind_is_still_an_error() {
    // Optional means "may be absent", never "may be anything". A number where the declaration
    // says text is a sender bug, and reporting it is how it gets found — silently accepting it
    // would put a value of the wrong type into a typed field.
    both(
        "## A Box has:\n\
        \x20   a tag, which is Option of Text.\n\
         \n\
         ## Main\n\
         Inspect fromJson_Box(text_from_bytes([123, 34, 116, 97, 103, 34, 58, 53, 125])):\n\
        \x20   When Ok (b): Show \"decoded\".\n\
        \x20   When Err (e): Show \"ERR \" + e.\n",
        &["ERR tag: expected a JSON string"],
    );
}
