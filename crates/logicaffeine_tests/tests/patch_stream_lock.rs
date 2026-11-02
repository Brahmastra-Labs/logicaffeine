//! ★ LOCK — the DOM patch stream is a BYTE FORMAT, and it round-trips. ★
//!
//! Tristen's constraint: the browser runs compiled LOGOS, and JavaScript is a host SHIM, not a
//! layer of the framework. The way that becomes true for the DOM is a bulk patch stream — the
//! program BUILDS a buffer of operations and the host applies it in a `for` loop, deciding
//! nothing. **The stream is the interface.** Every decision that used to live in `runtime.js`
//! has to move to the side that emits it.
//!
//! So the format needs to be a real, tested artifact rather than an implicit agreement between an
//! emitter and a decoder written in different languages. A wire format that only its own author
//! can read is how two implementations drift while both look correct.
//!
//! ROBUSTNESS IS THE POINT HERE: the decoder's input arrives from linear memory that a compiled
//! program wrote, so a truncated, over-long or unknown record is a BUG SOMEWHERE, and the decoder
//! must say so rather than read past the end or guess.

use logicaffeine_compile::semantics::dom::DomOp;
use logicaffeine_compile::semantics::patch;

/// Every op kind, so the round-trip covers the whole surface rather than the easy half.
fn every_op() -> Vec<DomOp> {
    vec![
        DomOp::CreateElement {
            node: 1,
            tag: "div".into(),
        },
        DomOp::CreateTextNode {
            node: 2,
            text: "hello".into(),
        },
        DomOp::SetText {
            node: 2,
            text: "bye".into(),
        },
        DomOp::SetHtml {
            node: 1,
            html: "<b>x</b>".into(),
        },
        DomOp::ClearChildren { node: 0 },
        DomOp::SetAttribute {
            node: 1,
            name: "class".into(),
            value: "card".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
        DomOp::SetValue {
            node: 3,
            value: "typed".into(),
        },
        DomOp::SetChecked {
            node: 4,
            checked: true,
        },
        DomOp::SetChecked {
            node: 4,
            checked: false,
        },
        DomOp::AddEventListener {
            node: 1,
            event: "click".into(),
        },
        DomOp::SetStream {
            node: 5,
            stream: -7,
        },
        // ★ THE ONE THIS CORPUS WAS MISSING, and the reason `patch_opcode_parity_lock` exists:
        // this function's own doc comment says "every op kind … rather than the easy half", and
        // `PreventDefault` was encoded, decoded by Rust, and never once round-tripped — while the
        // browser's decoder did not know the opcode at all.
        DomOp::PreventDefault {
            node: 1,
            event: "submit".into(),
        },
        // Wave 7.4's retained-tree primitives — `vdom.lg`'s own opcodes.
        DomOp::RemoveNode { node: 2 },
        DomOp::InsertBefore {
            parent: 0,
            child: 1,
            before: 2,
        },
        DomOp::InsertBefore {
            parent: 0,
            child: 1,
            before: u32::MAX,
        },
        DomOp::RemoveAttribute {
            node: 1,
            name: "class".into(),
        },
        DomOp::Adopt {
            parent: 0,
            node: 7,
            tag: "li".into(),
        },
        DomOp::AdoptText {
            parent: 7,
            node: 8,
            text: "row".into(),
        },
        DomOp::AdoptText {
            parent: 7,
            node: 9,
            text: "".into(),
        },
        DomOp::Listen {
            node: 1,
            event: "submit".into(),
            flags: 0b0001_1111,
        },
        DomOp::Unlisten {
            node: 1,
            event: "submit".into(),
        },
        DomOp::FreeNode { node: 2 },
        // Wave 7.6 — the hydration a stream opens before its claims.
        DomOp::Hydrate { root: 0 },
    ]
}

#[test]
fn every_operation_survives_the_round_trip() {
    let ops = every_op();
    let bytes = patch::encode(&ops);
    assert_eq!(patch::decode(&bytes).expect("it must decode"), ops);
}

#[test]
fn an_empty_stream_is_a_valid_stream() {
    // A paint that changed nothing is a normal thing to send, and a decoder that treated an empty
    // buffer as an error would make "nothing happened" indistinguishable from a bug.
    let bytes = patch::encode(&[]);
    assert_eq!(
        patch::decode(&bytes).expect("it must decode"),
        Vec::<DomOp>::new()
    );
}

#[test]
fn encoding_is_deterministic() {
    // The same ops must produce the same bytes, or every byte-comparison lock built on the stream
    // becomes a coin flip — the same reason the JSON derive sorts its records.
    assert_eq!(patch::encode(&every_op()), patch::encode(&every_op()));
}

#[test]
fn multibyte_text_survives() {
    // Lengths are BYTES, not characters. A decoder that counted characters would split a
    // codepoint and hand the host mojibake — or read into the next record.
    let ops = vec![DomOp::SetText {
        node: 1,
        text: "héllo → 世界 🌍".into(),
    }];
    let bytes = patch::encode(&ops);
    assert_eq!(patch::decode(&bytes).expect("it must decode"), ops);
}

#[test]
fn text_that_could_break_a_delimiter_based_format_survives() {
    // The format is LENGTH-PREFIXED precisely so no byte is special. A NUL, a newline and a quote
    // are ordinary content.
    let ops = vec![DomOp::SetAttribute {
        node: 1,
        name: "title".into(),
        value: "a\0b\nc\"d\\e".into(),
    }];
    let bytes = patch::encode(&ops);
    assert_eq!(patch::decode(&bytes).expect("it must decode"), ops);
}

#[test]
fn an_empty_string_is_not_a_missing_string() {
    // `setAttribute(el, "disabled", "")` is how a boolean attribute is written, so an empty value
    // has to survive as an empty value rather than as nothing.
    let ops = vec![DomOp::SetAttribute {
        node: 1,
        name: "disabled".into(),
        value: String::new(),
    }];
    let bytes = patch::encode(&ops);
    assert_eq!(patch::decode(&bytes).expect("it must decode"), ops);
}

#[test]
fn the_largest_node_id_survives() {
    // `u32::MAX` is the "no such element" sentinel the mount path already uses, so it must ride
    // the wire intact rather than wrapping into a real node.
    let ops = vec![DomOp::AppendChild {
        parent: u32::MAX,
        child: u32::MAX - 1,
    }];
    let bytes = patch::encode(&ops);
    assert_eq!(patch::decode(&bytes).expect("it must decode"), ops);
}

// ---- Robustness: the buffer came from a compiled program's memory. ----

#[test]
fn a_truncated_stream_is_refused_rather_than_read_past() {
    // Every prefix of a valid stream that is not itself a whole number of records must be
    // REFUSED. Reading past the end is the bug class this format exists to make impossible.
    let bytes = patch::encode(&every_op());
    for cut in 1..bytes.len() {
        let head = &bytes[..cut];
        if let Ok(ops) = patch::decode(head) {
            // A prefix may legitimately be a shorter VALID stream — but never longer than the
            // whole, and never containing a record the truncation cut in half.
            assert!(
                ops.len() < every_op().len(),
                "a truncated stream decoded to the whole thing at cut {cut}"
            );
        }
    }
}

#[test]
fn an_unknown_opcode_is_refused_by_name() {
    let mut bytes = patch::encode(&[DomOp::ClearChildren { node: 0 }]);
    bytes[0] = 0xEE;
    let err = patch::decode(&bytes).expect_err("an unknown opcode must be refused");
    assert!(
        err.contains("238") || err.contains("0xEE") || err.contains("ee"),
        "names it: {err}"
    );
}

#[test]
fn a_length_that_overruns_the_buffer_is_refused() {
    // The hostile case: a length field claiming more bytes than exist. A decoder that trusted it
    // would read whatever followed in memory.
    let mut bytes = patch::encode(&[DomOp::SetText {
        node: 1,
        text: "hi".into(),
    }]);
    let n = bytes.len();
    bytes[n - 6..n - 2].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(
        patch::decode(&bytes).is_err(),
        "an overrunning length must be refused"
    );
}

#[test]
fn invalid_utf8_is_refused_rather_than_lossily_converted() {
    // Text on the wire is UTF-8. Replacing a bad byte with U+FFFD would put a character on the
    // page that nobody wrote, silently.
    let mut bytes = patch::encode(&[DomOp::SetText {
        node: 1,
        text: "ab".into(),
    }]);
    let n = bytes.len();
    bytes[n - 1] = 0xFF;
    assert!(
        patch::decode(&bytes).is_err(),
        "invalid UTF-8 must be refused"
    );
}

#[test]
fn a_retired_opcode_is_refused_by_name_in_both_formats() {
    for &(code, why) in patch::RETIRED_OPCODES {
        let name = patch::OPCODES[code as usize];
        let bytes = patch::decode(&[code]).expect_err("the byte format refuses a retired opcode");
        let words =
            patch::decode_words(&[code as i64], &[]).expect_err("the word format refuses it too");
        for (format, refusal) in [("bytes", &bytes), ("words", &words)] {
            assert!(
                refusal.contains(&format!("patch opcode {code} ({name})")) && refusal.contains(why),
                "{format}: a retired opcode is refused by its name and why, never as an unknown code: {refusal}"
            );
        }
        let mut stream = patch::encode(&[DomOp::ClearChildren { node: 0 }]);
        stream.insert(0, code);
        assert!(
            patch::decode(&stream).is_err(),
            "a stream carrying it is refused whole"
        );
    }
}

#[test]
fn the_retired_paint_boundary_is_named_where_it_stands() {
    // The whole sentence, and the position it gives: the record's own offset, first in the stream
    // or behind a whole record, in either format.
    let refusal = |at: &str| {
        format!(
            "patch opcode 0 (BeginPaint) at {at} is retired: a paint's boundary is the commit that \
             emits the stream, and no build since the retained tree writes it."
        )
    };
    assert_eq!(patch::decode(&[0]), Err(refusal("byte 0")));
    assert_eq!(patch::decode_words(&[0], &[]), Err(refusal("word 0")));
    let mut bytes = patch::encode(&[DomOp::ClearChildren { node: 0 }]);
    bytes.push(0);
    assert_eq!(patch::decode(&bytes), Err(refusal("byte 5")));
    let (mut words, strings) = patch::encode_words(&[DomOp::ClearChildren { node: 0 }]);
    words.push(0);
    assert_eq!(
        patch::decode_words(&words, &strings),
        Err(refusal("word 2"))
    );
}

#[test]
fn the_retired_child_move_is_named_where_it_stands() {
    // A `{children}` slot's nodes are a range of the tape the commit places before it emits the
    // stream, so a record reparenting a holder's children is refused by its name wherever it
    // stands, whole with its operands, in either format.
    let refusal = |at: &str| {
        format!(
            "patch opcode 13 (MoveChildren) at {at} is retired: a slot's children are placed by the \
             commit that emits the stream, and no build since the retained tree writes it."
        )
    };
    let record: [u8; 9] = [13, 6, 0, 0, 0, 1, 0, 0, 0];
    assert_eq!(patch::decode(&record), Err(refusal("byte 0")));
    assert_eq!(
        patch::decode_words(&[13, 6, 1], &[]),
        Err(refusal("word 0"))
    );
    let mut bytes = patch::encode(&[DomOp::ClearChildren { node: 0 }]);
    bytes.extend_from_slice(&record);
    assert_eq!(patch::decode(&bytes), Err(refusal("byte 5")));
    let (mut words, strings) = patch::encode_words(&[DomOp::ClearChildren { node: 0 }]);
    words.extend_from_slice(&[13, 6, 1]);
    assert_eq!(
        patch::decode_words(&words, &strings),
        Err(refusal("word 2"))
    );
}

#[test]
fn the_opcode_table_is_append_only() {
    // The emitter is compiled into a wasm module and the decoder ships in a host. They are
    // updated at DIFFERENT TIMES, so renumbering an opcode silently reinterprets every stream a
    // previously-built module emits. New ops go on the END.
    assert_eq!(
        patch::OPCODES,
        &[
            "BeginPaint",
            "CreateElement",
            "CreateTextNode",
            "SetText",
            "SetHtml",
            "ClearChildren",
            "SetAttribute",
            "AppendChild",
            "SetValue",
            "SetChecked",
            "AddEventListener",
            "SetStream",
            // APPENDED for `on-submit.prevent`: a listener that cancels the browser's own action.
            // At the END, never inserted — the emitter is compiled into a wasm module and the
            // decoder ships in a host, updated at different times, so renumbering would silently
            // reinterpret every stream a previously-built module emits.
            "PreventDefault",
            // APPENDED for the `{children}` slot's holder, which the tape's ranges retired
            // (`patch::RETIRED_OPCODES`): it keeps its number. Same rule — at the END, because the
            // emitter and the decoder ship separately.
            "MoveChildren",
            // APPENDED for Wave 7.4's retained-tree reconciler (`vdom.lg`): the primitives an
            // immediate-mode stream never needed — removing/repositioning one node, deleting an
            // attribute, hydration's two bind ops, a listener registration carrying its
            // behavioural modifiers, and releasing a node's slot. Same rule — at the END.
            "RemoveNode",
            "InsertBefore",
            "RemoveAttribute",
            "Adopt",
            "AdoptText",
            "Listen",
            "Unlisten",
            "FreeNode",
            // APPENDED for Wave 7.6's hydration: the op that opens one. Same rule — at the END.
            "Hydrate",
        ],
        "an opcode changed position: a stream a previously-built module emits would be \
         reinterpreted. Add new operations to the END of the table."
    );
}

#[test]
fn a_long_stream_round_trips() {
    // A real paint of a thousand-row list is thousands of records; the format must not have a
    // small ceiling hiding in it.
    let ops: Vec<DomOp> = (0..5000)
        .map(|i| DomOp::CreateElement {
            node: i,
            tag: format!("tag{i}"),
        })
        .collect();
    let bytes = patch::encode(&ops);
    assert_eq!(patch::decode(&bytes).expect("it must decode"), ops);
}

// ---- Fuzzing: the decoder's input is memory a compiled program wrote. ----

/// A tiny deterministic PRNG, so a failure is reproducible from its seed alone.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 ^ (self.0 >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn random_text(rng: &mut Rng) -> String {
    // Deliberately hostile: the characters a delimiter-based format would choke on, plus
    // multi-byte codepoints, plus the empty string.
    const ALPHABET: &[&str] = &[
        "a", "\"", "\\", "\n", "\0", "<", "&", "é", "→", "世", "🌍", " ", "=",
    ];
    let n = rng.below(6);
    (0..n)
        .map(|_| ALPHABET[rng.below(ALPHABET.len() as u64) as usize])
        .collect()
}

/// One op of any kind the table still carries, with hostile operands — so the fuzz rows round-trip
/// and mutate every live opcode. A retired opcode has no op to draw.
fn random_op(rng: &mut Rng) -> DomOp {
    let node = rng.next() as u32;
    match rng.below(21) {
        0 => DomOp::CreateElement {
            node,
            tag: random_text(rng),
        },
        1 => DomOp::CreateTextNode {
            node,
            text: random_text(rng),
        },
        2 => DomOp::SetText {
            node,
            text: random_text(rng),
        },
        3 => DomOp::SetHtml {
            node,
            html: random_text(rng),
        },
        4 => DomOp::ClearChildren { node },
        5 => DomOp::SetAttribute {
            node,
            name: random_text(rng),
            value: random_text(rng),
        },
        6 => DomOp::AppendChild {
            parent: node,
            child: rng.next() as u32,
        },
        7 => DomOp::SetValue {
            node,
            value: random_text(rng),
        },
        8 => DomOp::SetChecked {
            node,
            checked: rng.below(2) == 1,
        },
        9 => DomOp::AddEventListener {
            node,
            event: random_text(rng),
        },
        10 => DomOp::SetStream {
            node,
            stream: rng.next() as i64,
        },
        11 => DomOp::PreventDefault {
            node,
            event: random_text(rng),
        },
        12 => DomOp::RemoveNode { node },
        13 => DomOp::InsertBefore {
            parent: node,
            child: rng.next() as u32,
            before: rng.next() as u32,
        },
        14 => DomOp::RemoveAttribute {
            node,
            name: random_text(rng),
        },
        15 => DomOp::Adopt {
            parent: node,
            node: rng.next() as u32,
            tag: random_text(rng),
        },
        16 => DomOp::AdoptText {
            parent: node,
            node: rng.next() as u32,
            text: random_text(rng),
        },
        17 => DomOp::Listen {
            node,
            event: random_text(rng),
            flags: rng.next() as u8,
        },
        18 => DomOp::Unlisten {
            node,
            event: random_text(rng),
        },
        19 => DomOp::FreeNode { node },
        _ => DomOp::Hydrate { root: node },
    }
}

#[test]
fn random_streams_round_trip() {
    let mut rng = Rng(0x5EED);
    for case in 0..2000 {
        let n = rng.below(40) as usize;
        let ops: Vec<DomOp> = (0..n).map(|_| random_op(&mut rng)).collect();
        let bytes = patch::encode(&ops);
        match patch::decode(&bytes) {
            Ok(back) => assert_eq!(back, ops, "case {case} did not round-trip"),
            Err(e) => panic!("case {case}: a stream we ENCODED failed to decode: {e}"),
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    // The decoder's whole contract under garbage: ANSWER — with a value or an error — and never
    // panic, never hang, never read past the end. This is the property that makes it safe to
    // point at a compiled program's linear memory.
    let mut rng = Rng(0xC0FFEE);
    for _ in 0..5000 {
        let n = rng.below(64) as usize;
        let bytes: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
        let _ = patch::decode(&bytes);
    }
}

#[test]
fn every_mutation_of_a_valid_stream_is_answered() {
    // Bit-flips and truncations of REAL streams — the shapes an off-by-one in an emitter
    // actually produces, which random bytes rarely reach.
    let mut rng = Rng(0xBEEF);
    for _ in 0..2000 {
        let n = rng.below(8) as usize;
        let ops: Vec<DomOp> = (0..n).map(|_| random_op(&mut rng)).collect();
        let mut bytes = patch::encode(&ops);
        if bytes.is_empty() {
            continue;
        }
        match rng.below(3) {
            0 => {
                let at = rng.below(bytes.len() as u64) as usize;
                bytes[at] ^= 1 << rng.below(8);
            }
            1 => {
                let keep = rng.below(bytes.len() as u64) as usize;
                bytes.truncate(keep);
            }
            _ => bytes.push(rng.next() as u8),
        }
        let _ = patch::decode(&bytes);
    }
}

// ---- The stream against a REAL page: the format has to carry what a paint actually does. ----

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

/// A page exercising the constructs a real paint emits.
const PAGE: &str = "\
## Component App
State:
    Let name be \"ada\".
    Let on be true.
    Let xs be [\"a\", \"b\"].
Actions:
    To go:
        Set name to \"bob\".
View:
    <div class=\"card\">
      <h1 id=\"who\">Hi {name}</h1>
      <input id=\"c\" type=\"checkbox\" bind={on}>
      <ul>{for each x in xs:}<li>{x}</li>{end}</ul>
      <button id=\"b\" on-click={go}>go</button>
    </div>

## Main
Mount App.
";

#[test]
fn a_real_pages_op_log_round_trips_through_the_stream() {
    // Not a synthetic list of ops: whatever a mount actually emits — the paint boundary, the
    // clear, the elements, the attributes, the checkbox presence, the listener registration.
    dom::reset();
    assert_eq!(tw_outcome(PAGE).error, None);
    let ops = dom::op_log();
    assert!(
        ops.len() > 10,
        "the page should emit a real paint: {} ops",
        ops.len()
    );
    let bytes = patch::encode(&ops);
    assert_eq!(patch::decode(&bytes).expect("it must decode"), ops);
}

#[test]
fn applying_the_stream_rebuilds_the_same_document() {
    // ★ THE PROPERTY THAT MATTERS. If a host can rebuild the page from the stream alone, then the
    // stream really is the interface and the host really can be a `for` loop — which is the whole
    // claim. If it could not, something would still have to live on the host side, deciding.
    dom::reset();
    assert_eq!(tw_outcome(PAGE).error, None);
    let ops = dom::op_log();
    let rendered = dom::render();

    let bytes = patch::encode(&ops);
    let decoded = patch::decode(&bytes).expect("it must decode");
    dom::reset();
    patch::apply(&decoded).expect("the host honours every id the stream names");
    assert_eq!(
        dom::render(),
        rendered,
        "the replayed stream built a different document"
    );
}

#[test]
fn a_turn_streams_and_replays_too() {
    // A repaint is where the stream earns its keep, so it is not enough that the first paint
    // survives: the ops a TURN emits have to as well.
    dom::reset();
    let src = format!("{PAGE}Click \"#b\".\n");
    assert_eq!(tw_outcome(&src).error, None);
    let ops = dom::op_log();
    let rendered = dom::render();
    assert!(rendered.contains("bob"), "the turn ran: {rendered}");

    let decoded = patch::decode(&patch::encode(&ops)).expect("it must decode");
    dom::reset();
    patch::apply(&decoded).expect("the host honours every id the stream names");
    assert_eq!(
        dom::render(),
        rendered,
        "the replayed stream built a different document"
    );
}

#[test]
fn the_replayed_stream_emits_the_same_ops_again() {
    // Idempotence of the FORMAT, not of the DOM: replaying a stream into a fresh recorder must
    // produce that same stream. Anything else means the applier is adding or dropping a record,
    // and the two sides of the wire would disagree the first time a host echoed one back.
    dom::reset();
    assert_eq!(tw_outcome(PAGE).error, None);
    let ops = dom::op_log();

    dom::reset();
    patch::apply(&ops).expect("the host honours every id the stream names");
    assert_eq!(dom::op_log(), ops, "replaying changed the stream");
}

// ---- Wave 7.4: the new primitives change the DOCUMENT MODEL, not just the op-log. A recorder
// function that only pushed its op would make `render()` lie about what the tree looks like, and
// the whole point of `applying_the_stream_rebuilds_the_same_document` above is that it cannot. ----

use logicaffeine_compile::interpreter::RuntimeValue;

#[test]
fn remove_node_detaches_it_from_render_and_leaves_its_sibling() {
    dom::reset();
    let ul = dom::create_element("ul");
    dom::append_child(0, ul);
    let a = dom::create_element("li");
    dom::set_text(a, "a");
    dom::append_child(ul, a);
    let b = dom::create_element("li");
    dom::set_text(b, "b");
    dom::append_child(ul, b);
    let before = dom::render();
    assert!(before.contains(">a<") && before.contains(">b<"), "{before}");

    dom::remove_node(a);
    let after = dom::render();
    assert!(
        !after.contains(">a<"),
        "removed row still rendered: {after}"
    );
    assert!(after.contains(">b<"), "surviving sibling lost: {after}");
}

#[test]
fn insert_before_reorders_the_rendered_children() {
    dom::reset();
    let ul = dom::create_element("ul");
    dom::append_child(0, ul);
    let a = dom::create_element("li");
    dom::set_text(a, "a");
    dom::append_child(ul, a);
    let b = dom::create_element("li");
    dom::set_text(b, "b");
    dom::append_child(ul, b);
    let c = dom::create_element("li");
    dom::set_text(c, "c");
    dom::append_child(ul, c);

    // a, b, c -> c, a, b: move c before a.
    dom::insert_before(ul, c, a);
    let rendered = dom::render();
    let (pos_a, pos_b, pos_c) = (
        rendered.find(">a<").unwrap(),
        rendered.find(">b<").unwrap(),
        rendered.find(">c<").unwrap(),
    );
    assert!(
        pos_c < pos_a && pos_a < pos_b,
        "expected c, a, b order: {rendered}"
    );
}

#[test]
fn insert_before_the_no_such_element_sentinel_appends_last() {
    dom::reset();
    let ul = dom::create_element("ul");
    dom::append_child(0, ul);
    let a = dom::create_element("li");
    dom::set_text(a, "a");
    dom::append_child(ul, a);
    let b = dom::create_element("li");
    dom::set_text(b, "b");
    dom::append_child(ul, b);

    dom::insert_before(ul, a, u32::MAX as i64);
    let rendered = dom::render();
    assert!(
        rendered.find(">b<").unwrap() < rendered.find(">a<").unwrap(),
        "expected b before a: {rendered}"
    );
}

#[test]
fn remove_attribute_removes_it_and_keeps_the_others() {
    dom::reset();
    let el = dom::create_element("input");
    dom::append_child(0, el);
    dom::set_attribute(el, "class", "card");
    dom::set_attribute(el, "id", "draft");
    let before = dom::render();
    assert!(
        before.contains(r#"class="card""#) && before.contains(r#"id="draft""#),
        "{before}"
    );

    dom::remove_attribute(el, "class");
    let after = dom::render();
    assert!(
        !after.contains("class"),
        "removed attribute still rendered: {after}"
    );
    assert!(
        after.contains(r#"id="draft""#),
        "surviving attribute lost: {after}"
    );
}

#[test]
fn free_node_clears_its_content_detaches_it_and_drops_its_listener() {
    dom::reset();
    let ul = dom::create_element("ul");
    dom::append_child(0, ul);
    let a = dom::create_element("li");
    dom::set_text(a, "gone");
    dom::append_child(ul, a);
    dom::add_event_listener(a, "click", RuntimeValue::Int(1));
    assert!(
        dom::begin_dispatch_keyed(a, "click", "", "").is_some(),
        "listener should be attached"
    );

    dom::free_node(a);
    let rendered = dom::render();
    assert!(
        !rendered.contains("gone"),
        "freed node's content still rendered: {rendered}"
    );
    assert!(
        dom::begin_dispatch_keyed(a, "click", "", "").is_none(),
        "a freed node's listener must be dropped"
    );
}

#[test]
fn free_node_is_one_op_that_releases_every_descendant_too() {
    // `FreeNode(root)` frees root AND every descendant in one call — a 1,000-row list clear is
    // 1,000 ops, not one per node in the whole tree. The host (this recorder, or runtime.js) does
    // the walk; a descendant is never freed by an op of its own.
    dom::reset();
    let ul = dom::create_element("ul");
    dom::append_child(0, ul);
    let li = dom::create_element("li");
    dom::append_child(ul, li);
    let span = dom::create_element("span");
    dom::set_attribute(span, "class", "tag");
    dom::append_child(li, span);
    let text = dom::create_text_node("deep");
    dom::append_child(span, text);
    dom::add_event_listener(span, "click", RuntimeValue::Int(7));
    assert!(dom::begin_dispatch_keyed(span, "click", "", "").is_some());

    dom::free_node(li);
    let rendered = dom::render();
    assert!(
        !rendered.contains("deep"),
        "a grandchild's text survived one FreeNode: {rendered}"
    );
    assert!(
        !rendered.contains("tag"),
        "a child's attribute survived one FreeNode: {rendered}"
    );
    assert!(
        dom::begin_dispatch_keyed(span, "click", "", "").is_none(),
        "a descendant's listener must be dropped by its ancestor's FreeNode, not its own"
    );
}

#[test]
fn listen_with_the_prevent_bit_marks_prevents_default_exactly_like_the_dedicated_op() {
    dom::reset();
    let by_flag = dom::create_element("form");
    let by_op = dom::create_element("form");
    dom::append_child(0, by_flag);
    dom::append_child(0, by_op);
    dom::listen(by_op, "submit", 0);
    assert!(!dom::dispatch_path(by_op, "submit").prevented);
    assert!(!dom::dispatch_path(by_flag, "submit").prevented);

    dom::prevent_default(by_op, "submit");
    dom::listen(by_flag, "submit", dom::LISTEN_PREVENT);
    assert!(dom::dispatch_path(by_op, "submit").prevented);
    assert!(dom::dispatch_path(by_flag, "submit").prevented);
}

#[test]
fn listen_without_the_prevent_bit_does_not_mark_prevents_default() {
    dom::reset();
    let el = dom::create_element("form");
    dom::append_child(0, el);

    dom::listen(
        el,
        "submit",
        dom::LISTEN_STOP | dom::LISTEN_ONCE | dom::LISTEN_CAPTURE | dom::LISTEN_PASSIVE,
    );
    assert!(!dom::dispatch_path(el, "submit").prevented);
    dom::listen(
        el,
        "submit",
        dom::LISTEN_STOP | dom::LISTEN_ONCE | dom::LISTEN_CAPTURE,
    );
    assert!(!dom::dispatch_path(el, "submit").prevented);
}

// ---- The emitter is the id authority: a host binds the id a stream names, from its own range. ----

/// The id a stream names for its `n`-th node — the range `vdom.lg`'s `vdWire` writes.
fn streamed(n: u32) -> u32 {
    dom::STREAM_ID_BASE + n
}

#[test]
fn a_created_node_takes_the_id_the_stream_names() {
    dom::reset();
    let p = streamed(7);
    patch::apply(&[
        DomOp::CreateElement {
            node: p,
            tag: "p".into(),
        },
        DomOp::SetAttribute {
            node: p,
            name: "id".into(),
            value: "seven".into(),
        },
        DomOp::CreateTextNode {
            node: streamed(8),
            text: "hello".into(),
        },
        DomOp::AppendChild {
            parent: p,
            child: streamed(8),
        },
        DomOp::AppendChild {
            parent: 0,
            child: p,
        },
    ])
    .expect("a stream id no node holds is the stream's to name");
    assert_eq!(dom::render(), "<body><p id=\"seven\">hello</p></body>");
    assert_eq!(
        dom::query_selector("#seven"),
        p as i64,
        "the page finds the node by the id the stream gave it"
    );
}

#[test]
fn a_freed_id_is_rebound_by_the_next_create() {
    dom::reset();
    let li = streamed(3);
    patch::apply(&[
        DomOp::CreateElement {
            node: li,
            tag: "li".into(),
        },
        DomOp::CreateTextNode {
            node: streamed(4),
            text: "old".into(),
        },
        DomOp::AppendChild {
            parent: li,
            child: streamed(4),
        },
        DomOp::AppendChild {
            parent: 0,
            child: li,
        },
        DomOp::FreeNode { node: li },
    ])
    .expect("create then free");
    assert_eq!(dom::render(), "<body></body>");
    patch::apply(&[
        DomOp::CreateElement {
            node: streamed(4),
            tag: "em".into(),
        },
        DomOp::CreateElement {
            node: li,
            tag: "b".into(),
        },
        DomOp::AppendChild {
            parent: li,
            child: streamed(4),
        },
        DomOp::AppendChild {
            parent: 0,
            child: li,
        },
    ])
    .expect("a freed subtree's ids are free to name again, the root's and every descendant's");
    assert_eq!(dom::render(), "<body><b><em></em></b></body>");
}

#[test]
fn a_host_minted_query_id_never_collides_with_a_stream_id() {
    // The page's own imperative element and the stream's first node exist side by side: the
    // recorder mints below the stream range and the stream names only inside it.
    dom::reset();
    let root = dom::create_element("div");
    dom::set_attribute(root, "id", "root");
    dom::append_child(0, root);
    patch::apply(&[
        DomOp::CreateElement {
            node: streamed(1),
            tag: "span".into(),
        },
        DomOp::AppendChild {
            parent: root as u32,
            child: streamed(1),
        },
    ])
    .expect("the stream renders into a host-made root");
    let own = dom::create_element("i");
    dom::append_child(0, own);
    assert!((own as u32) < dom::STREAM_ID_BASE && root != own);
    assert_eq!(dom::query_selector("#root"), root);
    assert_eq!(
        dom::render(),
        "<body><div id=\"root\"><span></span></div><i></i></body>"
    );
}

#[test]
fn a_stream_naming_a_live_id_is_refused_and_leaves_the_document_untouched() {
    dom::reset();
    patch::apply(&[
        DomOp::CreateElement {
            node: streamed(1),
            tag: "p".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: streamed(1),
        },
    ])
    .expect("first paint");
    let (render, log) = (dom::render(), dom::op_log());
    let err = patch::apply(&[
        DomOp::SetAttribute {
            node: streamed(1),
            name: "class".into(),
            value: "half".into(),
        },
        DomOp::CreateElement {
            node: streamed(1),
            tag: "div".into(),
        },
    ])
    .expect_err("an id still live cannot be created again");
    assert!(
        err.contains(&streamed(1).to_string()),
        "the refusal names the id: {err}"
    );
    assert_eq!(
        dom::render(),
        render,
        "the record before the refused create did not land"
    );
    assert_eq!(
        dom::op_log(),
        log,
        "a refused stream leaves the op log as it was"
    );
}

#[test]
fn a_host_range_id_out_of_turn_is_refused() {
    // Below the stream range a stream can name only the ordinal this recorder mints next — the
    // shape of a recorded op log replayed into a fresh document, and nothing else.
    dom::reset();
    let err = patch::apply(&[DomOp::CreateElement {
        node: 5,
        tag: "p".into(),
    }])
    .expect_err("a host-range id the recorder would not mint next is not the stream's to name");
    assert!(
        err.contains("the next is 1"),
        "the refusal says which id the host would mint: {err}"
    );
    assert_eq!(dom::render(), "<body></body>");
    patch::apply(&[DomOp::CreateElement {
        node: 1,
        tag: "p".into(),
    }])
    .expect("the next ordinal replays");
}

// ---- Hydration: an adopt never mints a lookalike node. A claim with nothing to bind counts a
// mismatch, binds no id, and leaves the document as it was. ----

#[test]
fn an_adopt_with_no_markup_to_claim_binds_no_node_and_counts_a_mismatch() {
    dom::reset();
    let before = dom::render();
    let mismatches = patch::apply(&[
        DomOp::Hydrate { root: 0 },
        DomOp::Adopt {
            parent: 0,
            node: streamed(1),
            tag: "li".into(),
        },
    ])
    .expect("a hydration with nothing to adopt is still a stream the host honours");
    assert_eq!(mismatches, 1, "the claim that found no `<li>` is counted");
    assert_eq!(dom::render(), before, "and nothing was minted in its place");
    patch::apply(&[DomOp::CreateElement {
        node: streamed(1),
        tag: "li".into(),
    }])
    .expect("the claimed id is still free: no lookalike took it");
}

#[test]
fn an_adopt_text_under_a_parent_no_claim_bound_binds_no_node_and_counts_a_mismatch() {
    dom::reset();
    let before = dom::render();
    let mismatches = patch::apply(&[
        DomOp::Hydrate { root: 0 },
        DomOp::AdoptText {
            parent: streamed(9),
            node: streamed(2),
            text: "row".into(),
        },
    ])
    .expect("a hydration is a stream the host honours");
    assert_eq!(
        mismatches, 1,
        "a text claim under a node the markup never gave is counted"
    );
    assert_eq!(dom::render(), before, "and nothing was minted in its place");
    patch::apply(&[DomOp::CreateTextNode {
        node: streamed(2),
        text: "row".into(),
    }])
    .expect("the claimed id is still free: no lookalike took it");
}

#[test]
fn an_adopt_outside_a_hydration_is_refused_whole() {
    dom::reset();
    let before = (dom::render(), dom::op_log());
    let err = patch::apply(&[
        DomOp::CreateElement {
            node: streamed(1),
            tag: "p".into(),
        },
        DomOp::Adopt {
            parent: 0,
            node: streamed(2),
            tag: "li".into(),
        },
    ])
    .expect_err("a claim no `Hydrate` opened is a malformed stream");
    assert!(
        err.contains("outside a hydration"),
        "the refusal says why: {err}"
    );
    assert_eq!(
        (dom::render(), dom::op_log()),
        before,
        "the refused stream left the document as it was"
    );
}

// ---- Wire format v2: words + a string table, `vdom.lg`'s own emission shape. ----

#[test]
fn every_operation_survives_the_word_round_trip() {
    let ops = every_op();
    let (words, strings) = patch::encode_words(&ops);
    assert_eq!(
        patch::decode_words(&words, &strings).expect("it must decode"),
        ops
    );
}

#[test]
fn word_encoding_is_deterministic() {
    assert_eq!(
        patch::encode_words(&every_op()),
        patch::encode_words(&every_op())
    );
}

#[test]
fn word_string_indices_are_one_based_in_encounter_order() {
    let ops = vec![
        DomOp::CreateElement {
            node: 1,
            tag: "div".into(),
        },
        DomOp::SetAttribute {
            node: 1,
            name: "class".into(),
            value: "card".into(),
        },
    ];
    let (words, strings) = patch::encode_words(&ops);
    assert_eq!(
        strings,
        vec!["div".to_string(), "class".to_string(), "card".to_string()]
    );
    // CreateElement(1, div): opcode, node, string index 1 (div).
    assert_eq!(&words[0..3], &[1, 1, 1]);
    // SetAttribute(1, class, card): opcode, node, string index 2 (class), string index 3 (card).
    assert_eq!(&words[3..7], &[6, 1, 2, 3]);
}

#[test]
fn a_word_stream_never_dedupes_strings_matching_a_naive_emitter() {
    // `vdom.lg` interns every text it sees with no dictionary lookup; a decoder that expected
    // dedup would silently disagree with what the real emitter produces the first time two
    // elements shared a tag.
    let ops = vec![
        DomOp::CreateElement {
            node: 1,
            tag: "li".into(),
        },
        DomOp::CreateElement {
            node: 2,
            tag: "li".into(),
        },
    ];
    let (_, strings) = patch::encode_words(&ops);
    assert_eq!(strings, vec!["li".to_string(), "li".to_string()]);
}

#[test]
fn an_out_of_range_string_index_is_refused_by_words() {
    // opcode 1 (CreateElement), node 1, string index 99 — no such string.
    let err = patch::decode_words(&[1, 1, 99], &["div".to_string()])
        .expect_err("an out-of-range string index must be refused");
    assert!(err.contains("99"), "{err}");
}

#[test]
fn a_truncated_word_stream_is_refused_rather_than_read_past() {
    // opcode 7 (AppendChild) wants two more words and gets one.
    let err = patch::decode_words(&[7, 1], &[]).expect_err("truncated word stream must be refused");
    assert!(err.contains("word"), "{err}");
}

#[test]
fn an_unknown_word_opcode_is_refused_by_number() {
    let err = patch::decode_words(&[255], &[]).expect_err("unknown word opcode must be refused");
    assert!(err.contains("255"), "{err}");
}
