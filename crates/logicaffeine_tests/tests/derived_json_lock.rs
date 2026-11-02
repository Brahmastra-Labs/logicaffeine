//! ★ LOCK — every record gets a `toJson_<T>` written for it. ★
//!
//! A route that answers `Seq of Todo`, a typed fetch that sends one, and a form post that
//! receives one all need the same thing: a record turned into JSON, the SAME way everywhere. The
//! plan is explicit that this is ONE derive pass emitting ordinary LOGOS functions rather than a
//! per-backend serializer, because the moment two tiers spell a record differently the wire
//! stops being a contract — `http_differential` already demands byte-identical wording across
//! tiers, and a hand-written encoder per backend cannot give that.
//!
//! Emitting LOGOS (rather than Rust) is also what makes it testable: `toJson_Todo` is an ordinary
//! function every tier runs and every ordinary lock can call.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Both tiers, same answer.
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

const TODO: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n    \
    a done, which is Bool.\n\n";

#[test]
fn a_record_becomes_json_with_its_fields_in_declaration_order() {
    let src = format!(
        "{TODO}## Main\nLet t be a new Todo with id 1 and text \"milk\" and done false.\n\
         Show toJson_Todo(t).\n"
    );
    both(&src, &[r#"{"id":1,"text":"milk","done":false}"#]);
}

#[test]
fn a_true_bool_and_a_negative_int_survive_the_trip() {
    let src = format!(
        "{TODO}## Main\nLet t be a new Todo with id (0 - 7) and text \"x\" and done true.\n\
         Show toJson_Todo(t).\n"
    );
    both(&src, &[r#"{"id":-7,"text":"x","done":true}"#]);
}

#[test]
fn text_that_would_break_the_document_is_escaped() {
    // The security floor of a serializer: a value containing a quote or a backslash must not be
    // able to end the string it lives in. Getting this wrong does not look like a bug — it looks
    // like valid JSON that says something the author did not write.
    let src = format!(
        "{TODO}## Main\nLet t be a new Todo with id 1 and text \"a\\\"b\\\\c\" and done false.\n\
         Show toJson_Todo(t).\n"
    );
    both(&src, &[r#"{"id":1,"text":"a\"b\\c","done":false}"#]);
}

#[test]
fn an_empty_text_field_is_an_empty_string_and_not_a_missing_key() {
    // `""` and absent are different things to every JSON reader on earth.
    let src = format!(
        "{TODO}## Main\nLet t be a new Todo with id 0 and text \"\" and done false.\n\
         Show toJson_Todo(t).\n"
    );
    both(&src, &[r#"{"id":0,"text":"","done":false}"#]);
}

#[test]
fn every_declared_record_gets_its_own_encoder() {
    // The derive is over the whole file, not over whatever the first route happened to mention.
    let src = "## A Point has:\n    an x, which is Int.\n    a y, which is Int.\n\n\
        ## A Tag has:\n    a name, which is Text.\n\n\
        ## Main\n\
        Show toJson_Point(a new Point with x 1 and y 2).\n\
        Show toJson_Tag(a new Tag with name \"red\").\n";
    both(src, &[r#"{"x":1,"y":2}"#, r#"{"name":"red"}"#]);
}

#[test]
fn a_record_field_holding_another_record_nests() {
    let src = "## A Inner has:\n    a v, which is Int.\n\n\
        ## A Outer has:\n    a label, which is Text.\n    an inner, which is Inner.\n\n\
        ## Main\n\
        Show toJson_Outer(a new Outer with label \"o\" and inner (a new Inner with v 3)).\n";
    both(src, &[r#"{"label":"o","inner":{"v":3}}"#]);
}

// ---- Reachability: an encoder is derived for a record that is ENCODED, not merely declared. ----

/// Every function the compiled program carries, by name.
fn emitted_functions(src: &str) -> Vec<String> {
    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("the program must parse");
        let program =
            logicaffeine_compile::vm::Compiler::compile_with_types(stmts, interner, Some(types))
                .expect("the program must compile");
        let mut names: Vec<String> = program
            .fn_index
            .keys()
            .map(|s| interner.resolve(*s).to_string())
            .collect();
        names.sort();
        names
    })
}

/// ★ A REGRESSION THIS FILE CAUSED, and the test that stops it coming back.
///
/// The derive emitted `toJson_<T>` for every DECLARED record. The plan's own wording is every
/// REACHABLE one, and the difference is not cosmetic: a program that declares a record and never
/// serialises it then carries a function nobody calls, and a function nobody calls has no call
/// site to infer its parameter's type from. The C bytecode walker types a function's parameters
/// from its callers, so it saw `GetField` on a value of unknown kind and REFUSED THE WHOLE
/// PROGRAM — taking out every C, CUDA and RISC-V lock whose fixture happens to declare a record.
///
/// The lesson generalises past JSON: a derive that fires on DECLARATION taxes every program that
/// merely names the shape. It must fire on USE.
#[test]
fn a_record_that_is_never_encoded_gets_no_encoder() {
    let src = "## Definition\nA Tally has:\n    n: an Int.\n\
        ## Main\n    Let t be a new Tally with n 3.\n    Show t's n.\n";
    both(src, &["3"]);
    let fns = emitted_functions(src);
    assert!(
        !fns.iter().any(|f| f.starts_with("toJson_")),
        "no encoder is derived for a record nobody encodes: {fns:?}"
    );
}

#[test]
fn a_record_declaring_program_still_compiles_on_the_c_bytecode_walker() {
    // The shape that broke: declare a record, never serialise it, lower through the bytecode
    // Op-walker. This is the tier that refused, so this is the tier the test has to stand on.
    let src = "## Definition\nA Tally has:\n    n: an Int.\n\
        ## Main\n    Let t be a new Tally with n 3.\n    Show t's n.\n";
    let program = logicaffeine_compile::compile::compile_to_program_optimized(src)
        .unwrap_or_else(|e| panic!("the program must compile: {e:?}"));
    logicaffeine_compile::codegen_c::from_program::compile_program_to_c(&program)
        .unwrap_or_else(|e| panic!("the Op-walker must lower it: {e}"));
}

#[test]
fn a_nested_records_encoder_is_reachable_through_its_parent() {
    // Only `toJson_Outer` is named in the source; `toJson_Inner` is reachable only because Outer's
    // encoder calls it. A reachability rule that stopped at the source text would emit a call to a
    // function it never wrote.
    let src = "## Definition\nAn Inner has:\n    v: an Int.\n\
        ## Definition\nAn Outer has:\n    label: Text.\n    inner: an Inner.\n\
        ## Main\n    Show toJson_Outer(a new Outer with label \"o\" and inner (a new Inner with v 3)).\n";
    let fns = emitted_functions(src);
    assert!(
        fns.contains(&"toJson_Outer".to_string()),
        "the named encoder: {fns:?}"
    );
    assert!(
        fns.contains(&"toJson_Inner".to_string()),
        "and the one it calls: {fns:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// ★ A LIST FIELD IS A JSON ARRAY, and a list of TEXT is a list of QUOTED strings. ★
// ---------------------------------------------------------------------------------------------

#[test]
fn a_list_of_text_encodes_as_quoted_json_strings() {
    // ★ IT WAS EMITTING INVALID JSON. A `Seq` field fell through to LOGOS's own list display, so a
    // `Seq of Text` went out as `"tags":[a, b]` — unquoted, which every JSON parser on earth
    // rejects. A route answering that record sent a body no client could read, and the derive
    // claimed to have encoded it. Silent, and only visible to whoever was receiving it.
    both(
        "## A Bag has:\n\
        \x20   a tags, which is Seq of Text.\n\
         \n\
         ## Main\n\
         Let b be a new Bag with tags [\"a\", \"b\"].\n\
         Show toJson_Bag(b).\n",
        &["{\"tags\":[\"a\",\"b\"]}"],
    );
}

#[test]
fn a_list_of_numbers_encodes_as_a_json_array() {
    both(
        "## A Bag has:\n\
        \x20   a nums, which is Seq of Int.\n\
         \n\
         ## Main\n\
         Let b be a new Bag with nums [1, 2, 3].\n\
         Show toJson_Bag(b).\n",
        &["{\"nums\":[1,2,3]}"],
    );
}

#[test]
fn an_empty_list_is_an_empty_array_and_not_a_missing_key() {
    // `[]` and "no key at all" are different answers, and a client that told them apart would be
    // right to. An empty list is a value the program HAS.
    both(
        "## A Bag has:\n\
        \x20   a tags, which is Seq of Text.\n\
         \n\
         ## Main\n\
         Let b be a new Bag with tags (a new Seq of Text).\n\
         Show toJson_Bag(b).\n",
        &["{\"tags\":[]}"],
    );
}

#[test]
fn hostile_text_inside_a_list_is_escaped_like_any_other_string() {
    // The escaping must reach INSIDE the array. A quote in an element ends the string it lives in
    // and everything after becomes structure the program never wrote — the same injection the
    // scalar case is escaped against, one level down, where it is easy to forget.
    both(
        "## A Bag has:\n\
        \x20   a tags, which is Seq of Text.\n\
         \n\
         ## Main\n\
         Let b be a new Bag with tags [text_from_bytes([34, 92, 10])].\n\
         Show toJson_Bag(b).\n",
        // The three escapes JSON requires, INSIDE the array: `\"`, `\\` and `\n`.
        &["{\"tags\":[\"\\\"\\\\\\n\"]}"],
    );
}

#[test]
fn a_list_field_sits_beside_the_scalars_in_declaration_order() {
    // The array must not disturb the shape around it — a comma dropped or doubled next to it
    // breaks the whole document rather than one field.
    both(
        "## A Bag has:\n\
        \x20   a id, which is Int.\n\
        \x20   a tags, which is Seq of Text.\n\
        \x20   a ok, which is Bool.\n\
         \n\
         ## Main\n\
         Let b be a new Bag with id 7 and tags [\"x\"] and ok true.\n\
         Show toJson_Bag(b).\n",
        &["{\"id\":7,\"tags\":[\"x\"],\"ok\":true}"],
    );
}

// ---------------------------------------------------------------------------------------------
// ★ AN OPTION IS `null` OR THE VALUE — and anything the derive cannot encode gets NO encoder. ★
// ---------------------------------------------------------------------------------------------

#[test]
fn an_optional_text_field_is_quoted_when_present() {
    // ★ IT USED TO GO OUT UNQUOTED. An `Option of Text` fell through to the "print it as itself"
    // arm — the same fall-through that sent a `Seq of Text` out as `[a, b]` — so a record went on
    // the wire as `{"tag":x}`, which every JSON parser on earth rejects. Silent, and visible only
    // to whoever was trying to read it.
    both(
        "## A Box has:\n\
        \x20   a tag, which is Option of Text.\n\
         \n\
         ## Main\n\
         Show toJson_Box(a new Box with tag some(\"hi\")).\n",
        &["{\"tag\":\"hi\"}"],
    );
}

#[test]
fn an_absent_optional_field_is_null_and_not_a_missing_key() {
    // `null` is what JSON has for "the sender has nothing here", and it is a VALUE — a reader can
    // tell it apart from a key that was never sent, which is a different fact about a different
    // bug. Dropping the key would make an absent value indistinguishable from an older sender
    // that did not know the field existed.
    both(
        "## A Box has:\n\
        \x20   a tag, which is Option of Text.\n\
         \n\
         ## Main\n\
         Show toJson_Box(a new Box with tag none).\n",
        &["{\"tag\":null}"],
    );
}

#[test]
fn an_optional_text_field_is_escaped_like_any_other_text() {
    // The quoting has to reach INSIDE the option, which is exactly the place it is easy to forget:
    // a quote in the value ends the string it lives in and everything after becomes structure
    // nobody wrote.
    both(
        "## A Box has:\n\
        \x20   a tag, which is Option of Text.\n\
         \n\
         ## Main\n\
         Show toJson_Box(a new Box with tag some(\"a\\\"b\")).\n",
        &["{\"tag\":\"a\\\"b\"}"],
    );
}

#[test]
fn an_optional_number_is_bare_when_present_and_null_when_absent() {
    // A number needs no quotes, and quoting it would change its TYPE for the reader — a client
    // decoding `"3"` where it expected 3 either fails or silently gets text.
    both(
        "## A Box has:\n\
        \x20   a n, which is Option of Int.\n\
         \n\
         ## Main\n\
         Show toJson_Box(a new Box with n some(3)).\n\
         Show toJson_Box(a new Box with n none).\n",
        &["{\"n\":3}", "{\"n\":null}"],
    );
}

#[test]
fn an_optional_record_field_nests_through_its_own_encoder() {
    // An option of a record is the shape a "maybe I have one of these" answer has, and it must
    // reach the nested encoder rather than printing the record's own display.
    both(
        "## An Inner has:\n\
        \x20   an id, which is Int.\n\
         \n\
         ## A Box has:\n\
        \x20   an inner, which is Option of Inner.\n\
         \n\
         ## Main\n\
         Show toJson_Box(a new Box with inner some(a new Inner with id 7)).\n\
         Show toJson_Box(a new Box with inner none).\n",
        &["{\"inner\":{\"id\":7}}", "{\"inner\":null}"],
    );
}

#[test]
fn a_field_the_derive_cannot_encode_means_no_encoder_at_all() {
    // ★★ THE RULE THAT MAKES ALL OF THIS SAFE, and the reason this is not a smaller fix: the
    // fall-through printed LOGOS's own display for ANY type it had no case for, so an enum field
    // went out as `{"level":Red}` — invalid JSON, produced silently, by a derive that reported
    // success. Better no encoder at all than one that is right about some fields: a missing
    // function is a loud failure at the call, and a wrong body is a bug in somebody else's parser.
    let src = "## An Alert is one of:\n\
        \x20   Red.\n\
        \x20   Blue.\n\
         \n\
         ## A Sig has:\n\
        \x20   a level, which is Alert.\n\
         \n\
         ## Main\n\
         Show toJson_Sig(a new Sig with level Red).\n";
    let o = logicaffeine_compile::compile::tw_outcome(src);
    assert!(
        o.error.is_some(),
        "a record with a field the derive cannot encode must get NO encoder, not one that emits \
         invalid JSON — got output {:?}",
        o.output
    );
}

#[test]
fn a_record_of_only_encodable_fields_still_gets_its_encoder() {
    // The refusal has to stay NARROW. A rule that dropped an encoder for a record standing next to
    // an unencodable one would take out every ordinary record in a real program.
    both(
        "## An Alert is one of:\n\
        \x20   Red.\n\
         \n\
         ## A Sig has:\n\
        \x20   a level, which is Alert.\n\
         \n\
         ## A Todo has:\n\
        \x20   an id, which is Int.\n\
        \x20   a text, which is Text.\n\
         \n\
         ## Main\n\
         Show toJson_Todo(a new Todo with id 1 and text \"hi\").\n",
        &["{\"id\":1,\"text\":\"hi\"}"],
    );
}

#[test]
fn a_numeric_and_boolean_field_still_prints_bare() {
    // The allowlist must not be so tight that it refuses the types JSON shares with LOGOS. These
    // print as themselves and always did; the change is that they do so BY NAME rather than by
    // being whatever fell through.
    both(
        "## A Row has:\n\
        \x20   a count, which is Int.\n\
        \x20   a flag, which is Bool.\n\
        \x20   a rate, which is Real.\n\
         \n\
         ## Main\n\
         Show toJson_Row(a new Row with count (0 - 4) and flag true and rate 1.5).\n",
        &["{\"count\":-4,\"flag\":true,\"rate\":1.5}"],
    );
}
