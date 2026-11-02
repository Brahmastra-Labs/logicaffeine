//! The DOM patch stream: the byte format a compiled program writes and a host applies.
//!
//! Tristen's constraint is that the browser runs compiled LOGOS and JavaScript is a host SHIM,
//! not a layer of the framework. For the DOM that means a BULK stream: the program builds a
//! buffer of operations and the host applies it in a `for` loop, deciding nothing. **The stream
//! is the interface** — every decision that used to live in `runtime.js` belongs to the side that
//! emits it.
//!
//! # The format
//!
//! Little-endian, byte-oriented, self-delimiting. One record is a 1-byte OPCODE followed by its
//! operands, each of which is one of:
//!
//! - a NODE, `u32` — the creation ordinal every tier already agrees on, so `u32::MAX` (the
//!   "no such element" sentinel) rides through intact;
//! - a FLAG, one byte, `0` or `1`;
//! - an `i64`, 8 bytes — an opaque host handle;
//! - TEXT, a `u32` byte length then that many UTF-8 bytes. LENGTH-PREFIXED, never delimited, so
//!   no byte is special: a NUL, a newline and a quote are ordinary content, and an EMPTY string
//!   is a string rather than an absence (`setAttribute(el, "disabled", "")` is how a boolean
//!   attribute is written).
//!
//! # Why the decoder is strict
//!
//! Its input comes from linear memory a compiled program wrote. A truncated record, a length that
//! overruns the buffer, an unknown opcode or invalid UTF-8 all mean A BUG SOMEWHERE — so the
//! decoder REFUSES rather than reading past the end, guessing, or lossily converting. A U+FFFD
//! substituted for a bad byte would put a character on the page that nobody wrote.
//!
//! # Why the opcode table is append-only
//!
//! The emitter is compiled into a wasm module; the decoder ships in a host. They are updated at
//! DIFFERENT TIMES, so renumbering an opcode silently reinterprets every stream a
//! previously-built module emits. New operations go on the END, and `OPCODES` is pinned by
//! `patch_stream_lock` so that has to be deliberate.

use super::dom::DomOp;

/// The opcode table, in wire order. **Append-only** — see the module note.
pub const OPCODES: &[&str] = &[
    // Reserved ([`RETIRED_OPCODES`]): the boundary of a host-side staging pass, which the retained
    // tree retired.
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
    // APPENDED, never inserted: the emitter is compiled into a wasm module and the decoder ships
    // in a host, and they are updated at different times — renumbering silently reinterprets
    // every stream a previously-built module emits.
    "PreventDefault",
    // Reserved ([`RETIRED_OPCODES`]): a holding node's children reparented into a `{children}`
    // slot, which the tape's ranges retired.
    "MoveChildren",
    // Wave 7.4 — the retained-tree primitives `vdom.lg` needs that the immediate-mode stream
    // never did: removing/repositioning a single node, deleting an attribute, hydration's two
    // bind ops, a generalised listener registration carrying its behavioural modifiers, and
    // releasing a node's slot.
    "RemoveNode",
    "InsertBefore",
    "RemoveAttribute",
    "Adopt",
    "AdoptText",
    "Listen",
    "Unlisten",
    "FreeNode",
    // Wave 7.6 — opens a hydration: the stream's `Adopt`/`AdoptText` claim the markup the root
    // already holds.
    "Hydrate",
];

const OP_BEGIN_PAINT: u8 = 0;

/// The opcodes no emitter writes any more, each with why. Each keeps its number, so no later opcode
/// is renumbered, and both decoders refuse it by name (`retired_opcode`) rather than as an unknown
/// code.
pub const RETIRED_OPCODES: &[(u8, &str)] = &[
    (
        OP_BEGIN_PAINT,
        "a paint's boundary is the commit that emits the stream, and no build since the retained tree writes it",
    ),
    (
        OP_MOVE_CHILDREN,
        "a slot's children are placed by the commit that emits the stream, and no build since the retained tree writes it",
    ),
];
const OP_CREATE_ELEMENT: u8 = 1;
const OP_CREATE_TEXT_NODE: u8 = 2;
const OP_SET_TEXT: u8 = 3;
const OP_SET_HTML: u8 = 4;
const OP_CLEAR_CHILDREN: u8 = 5;
const OP_SET_ATTRIBUTE: u8 = 6;
const OP_APPEND_CHILD: u8 = 7;
const OP_SET_VALUE: u8 = 8;
const OP_SET_CHECKED: u8 = 9;
const OP_ADD_EVENT_LISTENER: u8 = 10;
const OP_SET_STREAM: u8 = 11;
const OP_PREVENT_DEFAULT: u8 = 12;
const OP_MOVE_CHILDREN: u8 = 13;
const OP_REMOVE_NODE: u8 = 14;
const OP_INSERT_BEFORE: u8 = 15;
const OP_REMOVE_ATTRIBUTE: u8 = 16;
const OP_ADOPT: u8 = 17;
const OP_ADOPT_TEXT: u8 = 18;
const OP_LISTEN: u8 = 19;
const OP_UNLISTEN: u8 = 20;
const OP_FREE_NODE: u8 = 21;
const OP_HYDRATE: u8 = 22;

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_text(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

/// Encode `ops` as a patch stream.
///
/// Deterministic: the same operations always produce the same bytes, so a byte comparison over
/// the stream is a real assertion rather than a coin flip.
pub fn encode(ops: &[DomOp]) -> Vec<u8> {
    let mut out = Vec::new();
    for op in ops {
        match op {
            DomOp::CreateElement { node, tag } => {
                out.push(OP_CREATE_ELEMENT);
                put_u32(&mut out, *node);
                put_text(&mut out, tag);
            }
            DomOp::CreateTextNode { node, text } => {
                out.push(OP_CREATE_TEXT_NODE);
                put_u32(&mut out, *node);
                put_text(&mut out, text);
            }
            DomOp::SetText { node, text } => {
                out.push(OP_SET_TEXT);
                put_u32(&mut out, *node);
                put_text(&mut out, text);
            }
            DomOp::SetHtml { node, html } => {
                out.push(OP_SET_HTML);
                put_u32(&mut out, *node);
                put_text(&mut out, html);
            }
            DomOp::ClearChildren { node } => {
                out.push(OP_CLEAR_CHILDREN);
                put_u32(&mut out, *node);
            }
            DomOp::SetAttribute { node, name, value } => {
                out.push(OP_SET_ATTRIBUTE);
                put_u32(&mut out, *node);
                put_text(&mut out, name);
                put_text(&mut out, value);
            }
            DomOp::AppendChild { parent, child } => {
                out.push(OP_APPEND_CHILD);
                put_u32(&mut out, *parent);
                put_u32(&mut out, *child);
            }
            DomOp::SetValue { node, value } => {
                out.push(OP_SET_VALUE);
                put_u32(&mut out, *node);
                put_text(&mut out, value);
            }
            DomOp::SetChecked { node, checked } => {
                out.push(OP_SET_CHECKED);
                put_u32(&mut out, *node);
                out.push(u8::from(*checked));
            }
            DomOp::AddEventListener { node, event } => {
                out.push(OP_ADD_EVENT_LISTENER);
                put_u32(&mut out, *node);
                put_text(&mut out, event);
            }
            DomOp::SetStream { node, stream } => {
                out.push(OP_SET_STREAM);
                put_u32(&mut out, *node);
                out.extend_from_slice(&stream.to_le_bytes());
            }
            DomOp::PreventDefault { node, event } => {
                out.push(OP_PREVENT_DEFAULT);
                put_u32(&mut out, *node);
                put_text(&mut out, event);
            }
            DomOp::RemoveNode { node } => {
                out.push(OP_REMOVE_NODE);
                put_u32(&mut out, *node);
            }
            DomOp::InsertBefore {
                parent,
                child,
                before,
            } => {
                out.push(OP_INSERT_BEFORE);
                put_u32(&mut out, *parent);
                put_u32(&mut out, *child);
                put_u32(&mut out, *before);
            }
            DomOp::RemoveAttribute { node, name } => {
                out.push(OP_REMOVE_ATTRIBUTE);
                put_u32(&mut out, *node);
                put_text(&mut out, name);
            }
            DomOp::Adopt { parent, node, tag } => {
                out.push(OP_ADOPT);
                put_u32(&mut out, *parent);
                put_u32(&mut out, *node);
                put_text(&mut out, tag);
            }
            DomOp::AdoptText { parent, node, text } => {
                out.push(OP_ADOPT_TEXT);
                put_u32(&mut out, *parent);
                put_u32(&mut out, *node);
                put_text(&mut out, text);
            }
            DomOp::Hydrate { root } => {
                out.push(OP_HYDRATE);
                put_u32(&mut out, *root);
            }
            DomOp::Listen { node, event, flags } => {
                out.push(OP_LISTEN);
                put_u32(&mut out, *node);
                put_text(&mut out, event);
                out.push(*flags);
            }
            DomOp::Unlisten { node, event } => {
                out.push(OP_UNLISTEN);
                put_u32(&mut out, *node);
                put_text(&mut out, event);
            }
            DomOp::FreeNode { node } => {
                out.push(OP_FREE_NODE);
                put_u32(&mut out, *node);
            }
            // The WebRTC handshake is a peer-connection protocol, not a document mutation: it
            // mints opaque host handles and exchanges SDP. It rides its own imports rather than
            // the DOM patch stream, and encoding it here would put a second, unrelated protocol
            // in a format whose whole job is "what to do to the document".
            DomOp::RtcPeerConnection { .. }
            | DomOp::RtcDataChannel { .. }
            | DomOp::RtcCreateOffer { .. }
            | DomOp::RtcCreateAnswer { .. }
            | DomOp::RtcSetLocal { .. }
            | DomOp::RtcSetRemote { .. }
            | DomOp::RtcAddIce { .. }
            | DomOp::RtcSend { .. } => {}
        }
    }
    out
}

/// Read a patch buffer out of a LOGOS value — a `Seq of Int` whose every element is a byte.
///
/// A value OUTSIDE a byte's range is refused rather than truncated to its low 8 bits: a wrong
/// number silently becoming a different, valid-looking operation is exactly the class of bug a
/// binary format invites, and it would show up as a mangled page rather than as an error.
pub fn bytes_from_value(value: &crate::interpreter::RuntimeValue) -> Result<Vec<u8>, String> {
    use crate::interpreter::RuntimeValue;
    let RuntimeValue::List(list) = value else {
        return Err(format!(
            "`applyPatches` wants a patch buffer — a Seq of Int, each element one byte — and got \
             {}.",
            value.type_name()
        ));
    };
    let mut out = Vec::new();
    for (i, item) in list.borrow().to_values().into_iter().enumerate() {
        let RuntimeValue::Int(n) = item else {
            return Err(format!(
                "a patch buffer holds bytes; element {} is {}.",
                i + 1,
                item.type_name()
            ));
        };
        let byte = u8::try_from(n).map_err(|_| {
            format!(
                "a patch buffer holds bytes (0 to 255); element {} is {n}.",
                i + 1
            )
        })?;
        out.push(byte);
    }
    Ok(out)
}

/// Apply a decoded stream to the recorder — the DUMB SHIM, in Rust.
///
/// It is a `for` loop with no diffing, no reconciliation and no decisions of its own: every
/// choice was made by whoever emitted the stream. That is the whole point of the format, and it
/// is what a browser host has to be too — a host that decided anything would be a second
/// implementation of the framework, in a language the framework does not compile to.
///
/// The EMITTER is the id authority: a created node takes the id the stream names for it, and every
/// later record addresses it by that id. A stream names ids in the range [`dom::STREAM_ID_BASE`]
/// opens, which the recorder never mints; below it, a stream can name only the ordinal the
/// recorder would mint next — how a recorded op log replays into a fresh document.
///
/// All or nothing: a stream the host cannot honour — a create naming a live id, or an id the
/// recorder mints itself out of turn — leaves the document exactly as it was.
///
/// A stream that opens a hydration (`Hydrate`) ends it here: answers how many of its element claims
/// found no element of their tag — `0` for a stream that hydrated nothing. The emitter decides what
/// a mismatch means.
///
/// [`dom::STREAM_ID_BASE`]: super::dom::STREAM_ID_BASE
pub fn apply(ops: &[DomOp]) -> Result<i64, String> {
    super::dom::atomically(|| {
        ops.iter().try_for_each(apply_one)?;
        super::dom::end_hydration()
    })
}

fn apply_one(op: &DomOp) -> Result<(), String> {
    match op {
        DomOp::CreateElement { node, tag } => {
            return super::dom::create_element_as(*node as i64, tag)
        }
        DomOp::CreateTextNode { node, text } => {
            return super::dom::create_text_node_as(*node as i64, text)
        }
        DomOp::SetText { node, text } => super::dom::set_text(*node as i64, text),
        DomOp::SetHtml { node, html } => super::dom::set_html(*node as i64, html),
        DomOp::ClearChildren { node } => super::dom::clear_children(*node as i64),
        DomOp::SetAttribute { node, name, value } => {
            super::dom::set_attribute(*node as i64, name, value)
        }
        DomOp::AppendChild { parent, child } => {
            super::dom::append_child(*parent as i64, *child as i64)
        }
        DomOp::SetValue { node, value } => super::dom::set_value(*node as i64, value),
        DomOp::SetChecked { node, checked } => super::dom::set_checked(*node as i64, *checked),
        // A listener's HANDLER cannot ride the stream — it is a closure in the emitting
        // program, not data. The stream carries the REGISTRATION, which is what a browser
        // needs to attach; a replay into the recorder records the same fact.
        DomOp::AddEventListener { node, event } => {
            super::dom::note_event_listener(*node as i64, event)
        }
        DomOp::PreventDefault { node, event } => super::dom::prevent_default(*node as i64, event),
        DomOp::SetStream { node, stream } => super::dom::set_stream(*node as i64, *stream),
        DomOp::RemoveNode { node } => super::dom::remove_node(*node as i64),
        DomOp::InsertBefore {
            parent,
            child,
            before,
        } => super::dom::insert_before(*parent as i64, *child as i64, *before as i64),
        DomOp::RemoveAttribute { node, name } => super::dom::remove_attribute(*node as i64, name),
        DomOp::Hydrate { root } => return super::dom::hydrate(*root as i64),
        DomOp::Adopt { parent, node, tag } => {
            return super::dom::adopt(*parent as i64, *node as i64, tag)
        }
        DomOp::AdoptText { parent, node, text } => {
            return super::dom::adopt_text(*parent as i64, *node as i64, text)
        }
        DomOp::Listen { node, event, flags } => super::dom::listen(*node as i64, event, *flags),
        DomOp::Unlisten { node, event } => super::dom::unlisten(*node as i64, event),
        DomOp::FreeNode { node } => super::dom::free_node(*node as i64),
        DomOp::RtcPeerConnection { .. }
        | DomOp::RtcDataChannel { .. }
        | DomOp::RtcCreateOffer { .. }
        | DomOp::RtcCreateAnswer { .. }
        | DomOp::RtcSetLocal { .. }
        | DomOp::RtcSetRemote { .. }
        | DomOp::RtcAddIce { .. }
        | DomOp::RtcSend { .. } => {}
    }
    Ok(())
}

/// One cursor over the buffer, refusing anything that would read past its end.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or_else(|| {
            format!(
                "a patch record at byte {} claims a length that overflows",
                self.at
            )
        })?;
        if end > self.bytes.len() {
            return Err(format!(
                "a patch record at byte {} needs {n} more bytes and the stream has {}",
                self.at,
                self.bytes.len() - self.at
            ));
        }
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i64(&mut self) -> Result<i64, String> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn text(&mut self) -> Result<String, String> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        // Never lossy: a replacement character would put text on the page nobody wrote.
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|e| format!("a patch record carries text that is not UTF-8: {e}"))
    }

    fn flag(&mut self) -> Result<bool, String> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(format!("a patch flag must be 0 or 1, got {other}")),
        }
    }
}

/// Why `code` is retired, when it is ([`RETIRED_OPCODES`]).
fn retired(code: u8) -> Option<&'static str> {
    RETIRED_OPCODES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, why)| *why)
}

/// The refusal of a retired opcode, found at `at`.
fn retired_opcode(code: u8, at: &str) -> String {
    format!(
        "patch opcode {code} ({}) at {at} is retired: {}.",
        OPCODES[code as usize],
        retired(code).unwrap_or_default()
    )
}

/// Decode a patch stream, or say exactly what is wrong with it.
pub fn decode(bytes: &[u8]) -> Result<Vec<DomOp>, String> {
    let mut r = Reader { bytes, at: 0 };
    let mut ops = Vec::new();
    while r.at < bytes.len() {
        let code = r.u8()?;
        let op = match code {
            code if retired(code).is_some() => {
                return Err(retired_opcode(code, &format!("byte {}", r.at - 1)))
            }
            OP_CREATE_ELEMENT => DomOp::CreateElement {
                node: r.u32()?,
                tag: r.text()?,
            },
            OP_CREATE_TEXT_NODE => DomOp::CreateTextNode {
                node: r.u32()?,
                text: r.text()?,
            },
            OP_SET_TEXT => DomOp::SetText {
                node: r.u32()?,
                text: r.text()?,
            },
            OP_SET_HTML => DomOp::SetHtml {
                node: r.u32()?,
                html: r.text()?,
            },
            OP_CLEAR_CHILDREN => DomOp::ClearChildren { node: r.u32()? },
            OP_SET_ATTRIBUTE => DomOp::SetAttribute {
                node: r.u32()?,
                name: r.text()?,
                value: r.text()?,
            },
            OP_APPEND_CHILD => DomOp::AppendChild {
                parent: r.u32()?,
                child: r.u32()?,
            },
            OP_SET_VALUE => DomOp::SetValue {
                node: r.u32()?,
                value: r.text()?,
            },
            OP_SET_CHECKED => DomOp::SetChecked {
                node: r.u32()?,
                checked: r.flag()?,
            },
            OP_ADD_EVENT_LISTENER => DomOp::AddEventListener {
                node: r.u32()?,
                event: r.text()?,
            },
            OP_SET_STREAM => DomOp::SetStream {
                node: r.u32()?,
                stream: r.i64()?,
            },
            OP_PREVENT_DEFAULT => DomOp::PreventDefault {
                node: r.u32()?,
                event: r.text()?,
            },
            OP_REMOVE_NODE => DomOp::RemoveNode { node: r.u32()? },
            OP_INSERT_BEFORE => DomOp::InsertBefore {
                parent: r.u32()?,
                child: r.u32()?,
                before: r.u32()?,
            },
            OP_REMOVE_ATTRIBUTE => DomOp::RemoveAttribute {
                node: r.u32()?,
                name: r.text()?,
            },
            OP_ADOPT => DomOp::Adopt {
                parent: r.u32()?,
                node: r.u32()?,
                tag: r.text()?,
            },
            OP_ADOPT_TEXT => DomOp::AdoptText {
                parent: r.u32()?,
                node: r.u32()?,
                text: r.text()?,
            },
            OP_LISTEN => DomOp::Listen {
                node: r.u32()?,
                event: r.text()?,
                flags: r.u8()?,
            },
            OP_UNLISTEN => DomOp::Unlisten {
                node: r.u32()?,
                event: r.text()?,
            },
            OP_FREE_NODE => DomOp::FreeNode { node: r.u32()? },
            OP_HYDRATE => DomOp::Hydrate { root: r.u32()? },
            other => {
                return Err(format!(
                    "unknown patch opcode {other} (0x{other:02x}) at byte {}. The opcode table is \
                     append-only ({} operations today); a stream carrying a code this host does \
                     not know was emitted by a newer build.",
                    r.at - 1,
                    OPCODES.len()
                ))
            }
        };
        ops.push(op);
    }
    Ok(ops)
}

/// Word-oriented wire format v2: the same opcodes as [`encode`]/[`decode`], but every operand is
/// an `i64` WORD and every string is a 1-based index into a separate STRING TABLE — the shape
/// `vdom.lg`'s own emitter naturally produces (`Seq of Int` for the words, `Seq of Text` for the
/// strings), with no byte-level length-prefixing to hand-roll in LOGOS. `words`/`strings` travel
/// as a pair; a decoder given one without its matching other cannot make sense of either.
///
/// Unlike [`encode`], no interning: every text operand is appended to the table in the order it
/// is encountered, exactly as a straightforward LOGOS emitter that has no dictionary to consult
/// would build one, so a byte-for-byte comparison against `vdom.lg`'s own output stays meaningful.
pub fn encode_words(ops: &[DomOp]) -> (Vec<i64>, Vec<String>) {
    let mut words = Vec::new();
    let mut strings: Vec<String> = Vec::new();
    fn intern(strings: &mut Vec<String>, s: &str) -> i64 {
        strings.push(s.to_string());
        strings.len() as i64
    }
    for op in ops {
        match op {
            DomOp::CreateElement { node, tag } => {
                words.push(OP_CREATE_ELEMENT as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, tag));
            }
            DomOp::CreateTextNode { node, text } => {
                words.push(OP_CREATE_TEXT_NODE as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, text));
            }
            DomOp::SetText { node, text } => {
                words.push(OP_SET_TEXT as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, text));
            }
            DomOp::SetHtml { node, html } => {
                words.push(OP_SET_HTML as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, html));
            }
            DomOp::ClearChildren { node } => {
                words.push(OP_CLEAR_CHILDREN as i64);
                words.push(*node as i64);
            }
            DomOp::SetAttribute { node, name, value } => {
                words.push(OP_SET_ATTRIBUTE as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, name));
                words.push(intern(&mut strings, value));
            }
            DomOp::AppendChild { parent, child } => {
                words.push(OP_APPEND_CHILD as i64);
                words.push(*parent as i64);
                words.push(*child as i64);
            }
            DomOp::SetValue { node, value } => {
                words.push(OP_SET_VALUE as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, value));
            }
            DomOp::SetChecked { node, checked } => {
                words.push(OP_SET_CHECKED as i64);
                words.push(*node as i64);
                words.push(i64::from(*checked));
            }
            DomOp::AddEventListener { node, event } => {
                words.push(OP_ADD_EVENT_LISTENER as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, event));
            }
            DomOp::SetStream { node, stream } => {
                words.push(OP_SET_STREAM as i64);
                words.push(*node as i64);
                words.push(*stream);
            }
            DomOp::PreventDefault { node, event } => {
                words.push(OP_PREVENT_DEFAULT as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, event));
            }
            DomOp::RemoveNode { node } => {
                words.push(OP_REMOVE_NODE as i64);
                words.push(*node as i64);
            }
            DomOp::InsertBefore {
                parent,
                child,
                before,
            } => {
                words.push(OP_INSERT_BEFORE as i64);
                words.push(*parent as i64);
                words.push(*child as i64);
                words.push(*before as i64);
            }
            DomOp::RemoveAttribute { node, name } => {
                words.push(OP_REMOVE_ATTRIBUTE as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, name));
            }
            DomOp::Adopt { parent, node, tag } => {
                words.push(OP_ADOPT as i64);
                words.push(*parent as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, tag));
            }
            DomOp::AdoptText { parent, node, text } => {
                words.push(OP_ADOPT_TEXT as i64);
                words.push(*parent as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, text));
            }
            DomOp::Hydrate { root } => {
                words.push(OP_HYDRATE as i64);
                words.push(*root as i64);
            }
            DomOp::Listen { node, event, flags } => {
                words.push(OP_LISTEN as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, event));
                words.push(*flags as i64);
            }
            DomOp::Unlisten { node, event } => {
                words.push(OP_UNLISTEN as i64);
                words.push(*node as i64);
                words.push(intern(&mut strings, event));
            }
            DomOp::FreeNode { node } => {
                words.push(OP_FREE_NODE as i64);
                words.push(*node as i64);
            }
            // Same exclusion as `encode`: a separate protocol, not a document mutation.
            DomOp::RtcPeerConnection { .. }
            | DomOp::RtcDataChannel { .. }
            | DomOp::RtcCreateOffer { .. }
            | DomOp::RtcCreateAnswer { .. }
            | DomOp::RtcSetLocal { .. }
            | DomOp::RtcSetRemote { .. }
            | DomOp::RtcAddIce { .. }
            | DomOp::RtcSend { .. } => {}
        }
    }
    (words, strings)
}

/// One cursor over a word buffer, refusing anything past its end — the word-format twin of
/// [`Reader`].
struct WordReader<'a> {
    words: &'a [i64],
    strings: &'a [String],
    at: usize,
}

impl<'a> WordReader<'a> {
    fn word(&mut self) -> Result<i64, String> {
        let w = *self.words.get(self.at).ok_or_else(|| {
            format!(
                "a word patch record at word {} needs one more word and the stream has none",
                self.at
            )
        })?;
        self.at += 1;
        Ok(w)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let w = self.word()?;
        u32::try_from(w).map_err(|_| {
            format!(
                "a word patch record at word {} carries {w}, which does not fit a node id (u32)",
                self.at - 1
            )
        })
    }

    fn text(&mut self) -> Result<String, String> {
        let idx = self.word()?;
        let i = usize::try_from(idx)
            .ok()
            .filter(|&i| i >= 1 && i <= self.strings.len())
            .ok_or_else(|| {
                format!(
                    "a word patch record's string index {idx} is out of range (1..={}) at word {}",
                    self.strings.len(),
                    self.at - 1
                )
            })?;
        Ok(self.strings[i - 1].clone())
    }

    fn flag(&mut self) -> Result<bool, String> {
        match self.word()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(format!(
                "a word patch flag must be 0 or 1, got {other} at word {}",
                self.at - 1
            )),
        }
    }

    fn u8(&mut self) -> Result<u8, String> {
        let w = self.word()?;
        u8::try_from(w).map_err(|_| {
            format!(
                "a word patch record at word {} carries {w}, which does not fit a flags byte",
                self.at - 1
            )
        })
    }
}

/// Decode a `(words, strings)` pair — [`encode_words`]'s inverse, and what a native tier applies
/// `vdom.lg`'s own `vdCommit` output through.
pub fn decode_words(words: &[i64], strings: &[String]) -> Result<Vec<DomOp>, String> {
    let mut r = WordReader {
        words,
        strings,
        at: 0,
    };
    let mut ops = Vec::new();
    while r.at < words.len() {
        let code = r.word()?;
        let code = u8::try_from(code).map_err(|_| {
            format!(
                "a word patch opcode {code} at word {} does not fit a byte",
                r.at - 1
            )
        })?;
        let op = match code {
            code if retired(code).is_some() => {
                return Err(retired_opcode(code, &format!("word {}", r.at - 1)))
            }
            OP_CREATE_ELEMENT => DomOp::CreateElement {
                node: r.u32()?,
                tag: r.text()?,
            },
            OP_CREATE_TEXT_NODE => DomOp::CreateTextNode {
                node: r.u32()?,
                text: r.text()?,
            },
            OP_SET_TEXT => DomOp::SetText {
                node: r.u32()?,
                text: r.text()?,
            },
            OP_SET_HTML => DomOp::SetHtml {
                node: r.u32()?,
                html: r.text()?,
            },
            OP_CLEAR_CHILDREN => DomOp::ClearChildren { node: r.u32()? },
            OP_SET_ATTRIBUTE => DomOp::SetAttribute {
                node: r.u32()?,
                name: r.text()?,
                value: r.text()?,
            },
            OP_APPEND_CHILD => DomOp::AppendChild {
                parent: r.u32()?,
                child: r.u32()?,
            },
            OP_SET_VALUE => DomOp::SetValue {
                node: r.u32()?,
                value: r.text()?,
            },
            OP_SET_CHECKED => DomOp::SetChecked {
                node: r.u32()?,
                checked: r.flag()?,
            },
            OP_ADD_EVENT_LISTENER => DomOp::AddEventListener {
                node: r.u32()?,
                event: r.text()?,
            },
            OP_SET_STREAM => DomOp::SetStream {
                node: r.u32()?,
                stream: r.word()?,
            },
            OP_PREVENT_DEFAULT => DomOp::PreventDefault {
                node: r.u32()?,
                event: r.text()?,
            },
            OP_REMOVE_NODE => DomOp::RemoveNode { node: r.u32()? },
            OP_INSERT_BEFORE => DomOp::InsertBefore {
                parent: r.u32()?,
                child: r.u32()?,
                before: r.u32()?,
            },
            OP_REMOVE_ATTRIBUTE => DomOp::RemoveAttribute {
                node: r.u32()?,
                name: r.text()?,
            },
            OP_ADOPT => DomOp::Adopt {
                parent: r.u32()?,
                node: r.u32()?,
                tag: r.text()?,
            },
            OP_ADOPT_TEXT => DomOp::AdoptText {
                parent: r.u32()?,
                node: r.u32()?,
                text: r.text()?,
            },
            OP_LISTEN => DomOp::Listen {
                node: r.u32()?,
                event: r.text()?,
                flags: r.u8()?,
            },
            OP_UNLISTEN => DomOp::Unlisten {
                node: r.u32()?,
                event: r.text()?,
            },
            OP_FREE_NODE => DomOp::FreeNode { node: r.u32()? },
            OP_HYDRATE => DomOp::Hydrate { root: r.u32()? },
            other => {
                return Err(format!(
                    "unknown word patch opcode {other} at word {}. The opcode table is \
                     append-only ({} operations today); a stream carrying a code this host does \
                     not know was emitted by a newer build.",
                    r.at - 1,
                    OPCODES.len()
                ))
            }
        };
        ops.push(op);
    }
    Ok(ops)
}

/// The first `count` elements of `value`, a LOGOS sequence — a scratch array the emitter fills
/// only as far as its logical length. A count the sequence does not hold is refused: a stream
/// cut short or read past is a bug in the emitter, never a shorter stream.
fn prefix_of(
    value: &crate::interpreter::RuntimeValue,
    count: i64,
    what: &str,
) -> Result<Vec<crate::interpreter::RuntimeValue>, String> {
    use crate::interpreter::RuntimeValue;
    let RuntimeValue::List(list) = value else {
        return Err(format!(
            "a word patch stream's {what} is a sequence, and got {}.",
            value.type_name()
        ));
    };
    let items = list.borrow().to_values();
    let n = usize::try_from(count)
        .ok()
        .filter(|&n| n <= items.len())
        .ok_or_else(|| {
            format!(
                "a word patch stream names {count} {what}, and its sequence holds {}.",
                items.len()
            )
        })?;
    Ok(items.into_iter().take(n).collect())
}

/// The first `count` words of a `Seq of Int` — the word half of the word-oriented wire format, as
/// `vdom.lg`'s `vdApply` hands it over.
pub fn words_from_value(
    value: &crate::interpreter::RuntimeValue,
    count: i64,
) -> Result<Vec<i64>, String> {
    use crate::interpreter::RuntimeValue;
    prefix_of(value, count, "words")?
        .into_iter()
        .enumerate()
        .map(|(i, item)| match item {
            RuntimeValue::Int(n) => Ok(n),
            other => Err(format!(
                "a word patch stream holds Ints; word {} is {}.",
                i + 1,
                other.type_name()
            )),
        })
        .collect()
}

/// The first `count` strings of a `Seq of Text` — the string half of the word-oriented wire format.
pub fn strings_from_value(
    value: &crate::interpreter::RuntimeValue,
    count: i64,
) -> Result<Vec<String>, String> {
    use crate::interpreter::RuntimeValue;
    prefix_of(value, count, "strings")?
        .into_iter()
        .enumerate()
        .map(|(i, item)| match item {
            RuntimeValue::Text(s) => Ok(s.as_str().to_string()),
            other => Err(format!(
                "a word patch stream's string table holds Text; string {} is {}.",
                i + 1,
                other.type_name()
            )),
        })
        .collect()
}
