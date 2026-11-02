//! JSON conformance lock — the RFC 8259 profile of the pure-LOGOS stdlib json module
//! (`crates/logicaffeine_compile/assets/std/json.lg`), held to cross-tier agreement
//! (tree-walker ≡ VM) on every case.
//!
//! THE LOCKED PROFILE (v1):
//!
//! * **Values** — `Json` is `JNull` / `JBool` / `JNum` / `JDec` / `JStr` / `JArr` / `JObj`
//!   (`JObj` carries a `Seq of JEntry`, each `JEntry` a Text `key` + Json `val`).
//! * **Numbers are exact scaled decimals, never floats.** An integer-valued number —
//!   including `1e3`, `1.0`, `1500e-2`, `-0` — parses to `JNum` (i64). A non-integer
//!   value parses to `JDec` = `mant × 10^scale` with `scale < 0` and `mant` not
//!   divisible by 10 (parser-canonical, exact). A number whose canonical mantissa
//!   magnitude exceeds i64 (9223372036854775807) is REJECTED as out of range — no
//!   silent precision loss, ever. Canonical serialization is plain decimal notation
//!   (`1.5`, `0.01`, `-0.0015`), never exponent form, never a `+` sign.
//! * **Exponents** are ACCEPTED (`e`/`E`, optional `+`/`-`) and folded exactly.
//! * **Duplicate object keys**: LAST value wins, FIRST position is kept
//!   (`{"a":1,"b":2,"a":3}` → `{"a":3,"b":2}`), matching JS `JSON.parse`.
//! * **Strings**: all eight standard escapes decode; `\uXXXX` decodes BMP code points
//!   to UTF-8; a surrogate PAIR decodes to its supplementary code point; a LONE
//!   surrogate is rejected. Raw bytes ≥ 0x20 pass through untouched (byte-transparent;
//!   raw DEL 0x7F is legal per RFC). Raw control bytes < 0x20 are rejected.
//!   Canonical serialization escapes `"` `\` and controls (`\b \f \n \r \t`, else
//!   lowercase `\u00xx`), does NOT escape `/`, and re-emits non-ASCII as raw UTF-8.
//! * **Whitespace** between tokens is exactly RFC's four: space, tab, LF, CR.
//! * **Strict rejection** with the module's error form (`jsonTryParse` → `JErr msg`,
//!   never a trap, never a wrong success): trailing input, unterminated
//!   strings/containers/escapes, bare or misspelled words, control chars in strings,
//!   leading zeros, lone minus, `[1,]`, `{"a":}`, out-of-range numbers, …
//! * **Public surface**: `jsonTryParse (Text) -> JsonResult` (`JOk v` / `JErr msg`) is
//!   the strict non-trapping API. The legacy `jsonParse (Text) -> Json` stays: it
//!   returns the value for valid input and STOPS THE PROGRAM (assertion failure) on
//!   invalid input — loud, never garbage. `jsonSerialize` is the canonical
//!   serializer; `jsonGet (obj, key) -> Json` (JNull when missing / not an object)
//!   and `jsonHas (obj, key) -> Bool` are the object accessors.

mod common;
use common::{assert_all_tiers, run_all_tiers};

/// The tree-walker's recursive descent is Rust-stack-hungry per interpreted call;
/// the deep-nesting case runs on a dedicated big-stack thread (the established
/// idiom, e.g. tco.rs) so the 30-level document exercises the PARSER's limits,
/// not the host thread's default stack.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

/// Render a JSON document as a LOGOS Text expression, byte-exactly, sidestepping
/// LOGOS string-literal escaping/interpolation entirely: the bytes go through
/// `text_from_bytes`, so quotes, braces, backslashes and raw control characters
/// in test inputs are all first-class.
fn lg_text(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".to_string();
    }
    let bytes: Vec<String> = s.bytes().map(|b| b.to_string()).collect();
    format!("text_from_bytes([{}])", bytes.join(", "))
}

/// A program that strict-parses `json` and Shows either the canonical
/// serialization (`JOk`) or `REJECTED: <msg>` (`JErr`).
fn try_parse_render_src(json: &str) -> String {
    format!(
        "## Main\n\
         Let s be {}.\n\
         Inspect jsonTryParse(s):\n\
         \x20   When JOk (v):\n\
         \x20       Show jsonSerialize(v).\n\
         \x20   When JErr (msg):\n\
         \x20       Let bad be \"REJECTED: \" combined with msg.\n\
         \x20       Show bad.\n",
        lg_text(json)
    )
}

/// `json` must be ACCEPTED and serialize canonically to `canonical`, on all tiers.
fn accepts(json: &str, canonical: &str) {
    let src = try_parse_render_src(json);
    let agreed = run_all_tiers(&src);
    assert!(
        agreed.error.is_none(),
        "JSON accept case {:?} must run cleanly, got error: {:?}",
        json,
        agreed.error
    );
    let out = agreed.output.trim().to_string();
    assert_eq!(
        out, canonical,
        "JSON accept case {:?}: canonical serialization mismatch",
        json
    );
}

/// `json` must be REJECTED via the module's error form — `JErr` — with no runtime
/// trap and no wrong success.
fn rejects(json: &str) {
    let src = try_parse_render_src(json);
    let agreed = run_all_tiers(&src);
    assert!(
        agreed.error.is_none(),
        "JSON reject case {:?} must produce JErr (module error form), NOT a runtime trap: {:?}",
        json,
        agreed.error
    );
    let out = agreed.output.trim().to_string();
    assert!(
        out.starts_with("REJECTED: "),
        "JSON reject case {:?} was wrongly ACCEPTED as: {}",
        json,
        out
    );
}

/// Like [`rejects`], but pins the exact error message — the diagnostic contract.
fn rejects_with(json: &str, msg: &str) {
    let src = try_parse_render_src(json);
    let agreed = run_all_tiers(&src);
    assert!(
        agreed.error.is_none(),
        "JSON reject case {:?} must produce JErr, NOT a runtime trap: {:?}",
        json,
        agreed.error
    );
    let out = agreed.output.trim().to_string();
    assert_eq!(
        out,
        format!("REJECTED: {}", msg),
        "JSON reject case {:?}: error message contract mismatch",
        json
    );
}

/// `json` must be a string document whose DECODED payload is byte-for-byte
/// `expected_bytes` — verifies escape decoding, not just re-serialization.
fn decodes_string_to(json: &str, expected_bytes: &[u8]) {
    let expected_expr = if expected_bytes.is_empty() {
        "\"\"".to_string()
    } else {
        format!(
            "text_from_bytes([{}])",
            expected_bytes
                .iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let src = format!(
        "## Main\n\
         Let s be {}.\n\
         Inspect jsonTryParse(s):\n\
         \x20   When JOk (v):\n\
         \x20       Inspect v:\n\
         \x20           When JStr (t):\n\
         \x20               If t is equal to {}:\n\
         \x20                   Show \"MATCH\".\n\
         \x20               Otherwise:\n\
         \x20                   Show \"DIFF\".\n\
         \x20           Otherwise:\n\
         \x20               Show \"NOTSTR\".\n\
         \x20   When JErr (msg):\n\
         \x20       Let bad be \"ERR: \" combined with msg.\n\
         \x20       Show bad.\n",
        lg_text(json),
        expected_expr
    );
    assert_all_tiers(&src, "MATCH");
}

/// `json` must round-trip through the LEGACY strict API as a fixed point:
/// `serialize(parse(json))` equals `canonical`, and re-parsing the canonical form
/// serializes identically (determinism).
fn round_trips(json: &str, canonical: &str) {
    let src = format!(
        "## Main\n\
         Let s be {}.\n\
         Let c1 be jsonSerialize(jsonParse(s)).\n\
         Let c2 be jsonSerialize(jsonParse(c1)).\n\
         If c1 is equal to c2:\n\
         \x20   Show c1.\n\
         Otherwise:\n\
         \x20   Show \"UNSTABLE\".\n",
        lg_text(json)
    );
    assert_all_tiers(&src, canonical);
}

/// The master accept battery: (document, canonical serialization).
/// Every entry is also a round-trip fixed point (see the round-trip test).
const ACCEPT_BATTERY: &[(&str, &str)] = &[
    // Containers, empty and whitespace-padded.
    ("{}", "{}"),
    ("[]", "[]"),
    ("{ }", "{}"),
    ("[ ]", "[]"),
    (" [ 1 , 2 ] ", "[1,2]"),
    // Ordinary objects — the bread-and-butter shapes the audit found broken.
    (r#"{"a":1,"b":"x"}"#, r#"{"a":1,"b":"x"}"#),
    (r#"{"a":{"b":{"c":[]}}}"#, r#"{"a":{"b":{"c":[]}}}"#),
    (
        r#"{"a":[1,{"b":null}],"c":true}"#,
        r#"{"a":[1,{"b":null}],"c":true}"#,
    ),
    (r#"{"key with spaces":[]}"#, r#"{"key with spaces":[]}"#),
    (r#"{"":0}"#, r#"{"":0}"#),
    // Arrays and literals.
    ("[1,2,3]", "[1,2,3]"),
    ("[null,true,false]", "[null,true,false]"),
    (r#"["a","b"]"#, r#"["a","b"]"#),
    ("[[[]]]", "[[[]]]"),
    (
        r#"[[1,2],[3,[4,{"x":"y"}]],{},[]]"#,
        r#"[[1,2],[3,[4,{"x":"y"}]],{},[]]"#,
    ),
    ("true", "true"),
    ("false", "false"),
    ("null", "null"),
    // Strings.
    (r#""A""#, r#""A""#),
    (r#""""#, r#""""#),
    (r#""a\"b""#, r#""a\"b""#),
    (r#""a\\b""#, r#""a\\b""#),
    (r#""a\/b""#, r#""a/b""#),
    (r#""a\bb""#, r#""a\bb""#),
    (r#""a\fb""#, r#""a\fb""#),
    (r#""a\nb""#, r#""a\nb""#),
    (r#""a\rb""#, r#""a\rb""#),
    (r#""a\tb""#, r#""a\tb""#),
    // Unicode escapes: BMP decodes to raw UTF-8; controls canonicalize to their
    // shortest escape; quote/backslash escapes canonicalize to \" and \\; a
    // surrogate pair decodes to its supplementary code point.
    ("\"\\u0041\"", r#""A""#),
    ("\"\\u00e9\"", "\"é\""),
    ("\"\\u20AC\"", "\"€\""),
    ("\"\\u20ac\"", "\"€\""),
    ("\"\\ud83d\\ude00\"", "\"😀\""),
    ("\"\\u000a\"", "\"\\n\""),
    ("\"\\u001f\"", "\"\\u001f\""),
    ("\"\\u0022\"", "\"\\\"\""),
    ("\"\\u005c\"", "\"\\\\\""),
    // Raw non-ASCII passes through byte-transparently.
    (r#""é""#, "\"é\""),
    (r#""😀""#, "\"😀\""),
    ("\"héllo\"", "\"héllo\""),
    ("\"raw del \u{7f}\"", "\"raw del \u{7f}\""),
    // Numbers — integers, negatives, decimals, exponents (the locked profile).
    ("0", "0"),
    ("1", "1"),
    ("42", "42"),
    ("-1", "-1"),
    ("-0", "0"),
    ("9223372036854775807", "9223372036854775807"),
    ("-9223372036854775807", "-9223372036854775807"),
    ("1.5", "1.5"),
    ("-0.5", "-0.5"),
    ("0.01", "0.01"),
    ("1.50", "1.5"),
    ("1.0", "1"),
    ("123.456", "123.456"),
    ("0.000001", "0.000001"),
    ("1e3", "1000"),
    ("1E3", "1000"),
    ("1e+3", "1000"),
    ("1.5e3", "1500"),
    ("1e-2", "0.01"),
    ("15e-1", "1.5"),
    ("1500e-2", "15"),
    ("0e0", "0"),
    ("0.0", "0"),
    ("-0.0", "0"),
    ("1e18", "1000000000000000000"),
    ("2e18", "2000000000000000000"),
    ("-1.5e-3", "-0.0015"),
    ("2.5e2", "250"),
    ("1.25e1", "12.5"),
    ("[1.5,-2,3e2]", "[1.5,-2,300]"),
    (r#"{"n":-0.25}"#, r#"{"n":-0.25}"#),
    // Whitespace tolerance (all four RFC whitespace bytes).
    ("\t{\r\n \"a\" : 1 \t}\r\n", r#"{"a":1}"#),
    ("\n[\ttrue,\r false\n]\t", "[true,false]"),
];

/// The master reject battery: every document here must produce `JErr`.
const REJECT_BATTERY: &[&str] = &[
    // Emptiness and bare tokens.
    "",
    "   ",
    "\t\r\n",
    "[",
    "]",
    "{",
    "}",
    ",",
    ":",
    // Trailing input.
    "{}extra",
    "[]x",
    "1 2",
    "{} {}",
    "[1]]",
    r#"{"a":1}}"#,
    r#""ok" tail"#,
    "0x10",
    "1a",
    // Array malformations.
    "[1,]",
    "[,1]",
    "[1 2]",
    "[1,,2]",
    "[1,2",
    "[}",
    r#"["a]"#,
    // Object malformations.
    r#"{"a"}"#,
    r#"{"a":}"#,
    r#"{"a":1,}"#,
    "{,}",
    "{:1}",
    r#"{"a" 1}"#,
    r#"{"a":1"#,
    "{1:2}",
    "{true:1}",
    "{]",
    // Bare and misspelled words.
    "tru",
    "fals",
    "nul",
    "truex",
    "falsex",
    "nullx",
    "True",
    "FALSE",
    "None",
    "undefined",
    "hello",
    "'a'",
    // Number malformations.
    "01",
    "00",
    "-01",
    "007",
    "-",
    "+1",
    ".5",
    "1.",
    "1.e3",
    "1e",
    "1e+",
    "1e-",
    "--1",
    "1..5",
    "- 1",
    "NaN",
    "Infinity",
    "-Infinity",
    // Number range honesty (exact i64 mantissa or rejection — never silent loss).
    "9223372036854775808",
    "-9223372036854775808",
    "99999999999999999999",
    "1e19",
    "9.3e18",
    "1e99999",
    // String malformations.
    r#""unterminated"#,
    "\"",
    "\"\\",
    "\"\\\"",
    "\"\\x\"",
    "\"\\u\"",
    "\"\\u12\"",
    "\"\\u123\"",
    "\"\\uZZZZ\"",
    "\"\\u12G4\"",
    "\"\\ud800\"",
    "\"\\udc00\"",
    "\"\\ud800A\"",
    "\"\\ud800\\\\\"",
    "\"a\u{01}b\"",
    "\"a\u{00}b\"",
    "\"a\nb\"",
    "\"a\tb\"",
    // Exotic whitespace is NOT whitespace (RFC allows exactly four bytes).
    "\u{0b}[]",
    "\u{a0}1",
];

#[test]
fn accept_battery_is_at_least_thirty_cases() {
    assert!(
        ACCEPT_BATTERY.len() >= 30,
        "accept battery must stay property-sized: {} cases",
        ACCEPT_BATTERY.len()
    );
}

#[test]
fn reject_battery_is_at_least_thirty_cases() {
    assert!(
        REJECT_BATTERY.len() >= 30,
        "reject battery must stay property-sized: {} cases",
        REJECT_BATTERY.len()
    );
}

#[test]
fn accept_battery_parses_and_serializes_canonically() {
    for (json, canonical) in ACCEPT_BATTERY {
        accepts(json, canonical);
    }
}

#[test]
fn reject_battery_produces_the_module_error_form() {
    for json in REJECT_BATTERY {
        rejects(json);
    }
}

/// Every accepted document is a serialization fixed point through the LEGACY
/// `jsonParse` name: serialize(parse(s)) == canonical(s), and the canonical form
/// re-parses to the same canonical text — determinism, locked over the whole battery.
#[test]
fn accept_battery_round_trips_as_a_fixed_point() {
    for (json, canonical) in ACCEPT_BATTERY {
        round_trips(json, canonical);
    }
}

#[test]
fn ordinary_object_parses_with_working_accessors() {
    let src = format!(
        "## Main\n\
         Let v be jsonParse({}).\n\
         Show jsonSerialize(v).\n\
         Show jsonSerialize(jsonGet(v, \"a\")).\n\
         Show jsonSerialize(jsonGet(v, \"b\")).\n\
         Show jsonSerialize(jsonGet(v, \"missing\")).\n\
         If jsonHas(v, \"a\") is equal to true:\n\
         \x20   Show \"HAS-A\".\n\
         If jsonHas(v, \"missing\") is equal to false:\n\
         \x20   Show \"NO-MISSING\".\n",
        lg_text(r#"{"a":1,"b":"x"}"#)
    );
    assert_all_tiers(
        &src,
        "{\"a\":1,\"b\":\"x\"}\n1\n\"x\"\nnull\nHAS-A\nNO-MISSING",
    );
}

#[test]
fn accessors_are_total_on_non_objects_and_missing_keys() {
    let src = format!(
        "## Main\n\
         Let arrv be jsonParse(\"[1,2]\").\n\
         Show jsonSerialize(jsonGet(arrv, \"a\")).\n\
         If jsonHas(arrv, \"a\") is equal to false:\n\
         \x20   Show \"NONOBJ-FALSE\".\n\
         Let dup be jsonParse({}).\n\
         Show jsonSerialize(jsonGet(dup, \"a\")).\n",
        lg_text(r#"{"a":1,"b":2,"a":3}"#)
    );
    assert_all_tiers(&src, "null\nNONOBJ-FALSE\n3");
}

/// Duplicate keys: LAST value wins, FIRST position kept — locked exactly.
#[test]
fn duplicate_keys_last_value_wins_first_position_kept() {
    accepts(r#"{"a":1,"b":2,"a":3}"#, r#"{"a":3,"b":2}"#);
    accepts(r#"{"a":1,"a":2,"a":3}"#, r#"{"a":3}"#);
    accepts(r#"{"x":{"a":1,"a":null}}"#, r#"{"x":{"a":null}}"#);
}

/// Escape decoding is byte-exact — not just "re-serializes the same".
#[test]
fn string_escape_decoding_is_byte_exact() {
    decodes_string_to(r#""""#, &[]);
    decodes_string_to(r#""\n""#, &[10]);
    decodes_string_to(r#""\b\f\n\r\t""#, &[8, 12, 10, 13, 9]);
    decodes_string_to(r#""\"\\\/""#, &[34, 92, 47]);
    decodes_string_to("\"\\u0041\"", &[65]);
    decodes_string_to("\"\\u007f\"", &[127]);
    decodes_string_to("\"\\u00e9\"", &[195, 169]);
    decodes_string_to("\"\\u20ac\"", &[226, 130, 172]);
    decodes_string_to("\"\\ud83d\\ude00\"", &[240, 159, 152, 128]);
    decodes_string_to(r#""é""#, &[195, 169]);
    decodes_string_to(r#""😀""#, &[240, 159, 152, 128]);
    decodes_string_to("\"héllo\"", "héllo".as_bytes());
}

/// The exact diagnostic message contract — positions are 1-based byte positions.
#[test]
fn reject_messages_are_specific_and_positioned() {
    rejects_with("", "unexpected end of input");
    rejects_with("[1,2", "unexpected end of input");
    rejects_with("-", "unexpected end of input");
    rejects_with("tru", "invalid literal at byte 1");
    rejects_with("[null,nul]", "invalid literal at byte 7");
    rejects_with("[1,]", "unexpected character at byte 4");
    rejects_with(r#"{"a":}"#, "unexpected character at byte 6");
    rejects_with("@", "unexpected character at byte 1");
    rejects_with(r#""abc"#, "unterminated string starting at byte 1");
    rejects_with(r#"[ "x ]"#, "unterminated string starting at byte 3");
    rejects_with("{}x", "trailing characters after the JSON value at byte 3");
    rejects_with(
        "[] []",
        "trailing characters after the JSON value at byte 4",
    );
    rejects_with("01", "invalid number at byte 2");
    rejects_with("\"a\nb\"", "control character in string at byte 3");
    rejects_with("\"a\\xb\"", "invalid escape at byte 3");
    rejects_with("\"\\uZZZZ\"", "invalid unicode escape at byte 2");
    rejects_with("\"\\ud800\"", "lone surrogate at byte 2");
    rejects_with(r#"{"a" 1}"#, "expected a colon at byte 6");
    rejects_with("{1:2}", "expected an object key at byte 2");
    rejects_with("[1 2]", "expected a comma or closing bracket at byte 4");
    rejects_with(
        r#"{"a":1 "b":2}"#,
        "expected a comma or closing brace at byte 8",
    );
    rejects_with("9223372036854775808", "number out of range at byte 1");
    rejects_with("[true,1e19]", "number out of range at byte 7");
}

#[test]
fn deep_nesting_thirty_levels_parses_and_rejects_unterminated() {
    on_big_stack(|| {
        let deep_array = format!("{}1{}", "[".repeat(30), "]".repeat(30));
        accepts(&deep_array, &deep_array);
        round_trips(&deep_array, &deep_array);

        let deep_object = format!("{}1{}", r#"{"a":"#.repeat(30), "}".repeat(30));
        accepts(&deep_object, &deep_object);
        round_trips(&deep_object, &deep_object);

        let unterminated = "[".repeat(30);
        rejects(&unterminated);
    });
}

#[test]
fn long_strings_and_long_arrays_round_trip() {
    let long_plain = format!("\"{}\"", "a".repeat(1000));
    accepts(&long_plain, &long_plain);
    round_trips(&long_plain, &long_plain);

    let long_escaped = format!("\"{}\"", r"\n".repeat(200));
    accepts(&long_escaped, &long_escaped);

    let long_array = format!(
        "[{}]",
        (0..100)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    accepts(&long_array, &long_array);
    round_trips(&long_array, &long_array);
}

/// Hand-constructed values (the writer-side surface) serialize canonically and
/// reparse to the same canonical text — command envelopes are built this way.
#[test]
fn constructed_values_serialize_canonically_and_reparse() {
    let src = "## Main\n\
        Let es be a new Seq of JEntry.\n\
        Push (a new JEntry with key \"a\" and val (a new JNum with n 1)) to es.\n\
        Let inner be a new Seq of Json.\n\
        Push (a new JStr with s \"x\") to inner.\n\
        Push (a new JDec with mant 15 and scale (0 - 1)) to inner.\n\
        Push (a new JBool with b false) to inner.\n\
        Push (a new JNull) to inner.\n\
        Push (a new JEntry with key \"b\" and val (a new JArr with elems inner)) to es.\n\
        Let v be a new JObj with entries es.\n\
        Let c1 be jsonSerialize(v).\n\
        Show c1.\n\
        Let c2 be jsonSerialize(jsonParse(c1)).\n\
        If c1 is equal to c2:\n\
        \x20   Show \"REPARSE-STABLE\".\n\
        Otherwise:\n\
        \x20   Show c2.\n";
    assert_all_tiers(
        src,
        "{\"a\":1,\"b\":[\"x\",1.5,false,null]}\nREPARSE-STABLE",
    );
}

/// The serializer escapes specials and control characters (writer-side RFC duty).
#[test]
fn serializer_escapes_controls_and_specials() {
    let src = "## Main\n\
        Let raw be text_from_bytes([34, 92, 10, 9, 1, 127]).\n\
        Let node be a new JStr with s raw.\n\
        Show jsonSerialize(node).\n";
    assert_all_tiers(src, "\"\\\"\\\\\\n\\t\\u0001\u{7f}\"");
}

/// The legacy strict entry point keeps its name and signature: valid input parses,
/// invalid input STOPS THE PROGRAM identically on every tier — never silent garbage.
#[test]
fn legacy_jsonparse_stops_the_program_on_invalid_input() {
    let src = "## Main\n\
        Let v be jsonParse(\"[1,\").\n\
        Show jsonSerialize(v).\n";
    let agreed = run_all_tiers(src);
    assert!(
        agreed.error.is_some(),
        "legacy jsonParse must STOP on invalid input (tier-agreed error), got clean output: {:?}",
        agreed.output
    );
}

/// The pre-existing public shapes (parity_encoding.rs) stay byte-identical.
#[test]
fn legacy_surface_shapes_are_unchanged() {
    assert_all_tiers(
        "## Main\n\
         Show jsonSerialize(jsonParse(\"[1,2,3]\")).\n\
         Show jsonSerialize(jsonParse(\"42\")).\n\
         Show jsonSerialize(jsonParse(\"[true,null]\")).\n",
        "[1,2,3]\n42\n[true,null]",
    );
    assert_all_tiers(
        "## Main\n\
         Let arr be a new Seq of Json.\n\
         Push (a new JNum with n 1) to arr.\n\
         Push (a new JNum with n 2) to arr.\n\
         Push (a new JBool with b true) to arr.\n\
         Let j be a new JArr with elems arr.\n\
         Show jsonSerialize(j).\n",
        "[1,2,true]",
    );
}
