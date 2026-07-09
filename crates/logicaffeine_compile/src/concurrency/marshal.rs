//! Marshalling between the interpreter's `RuntimeValue` and the `Send`-able
//! [`RtPayload`] that crosses task/thread boundaries through channels.
//!
//! [`materialize`] moves a value OUT of a task's `Rc`-based heap into an owned,
//! self-contained `RtPayload`; [`rebuild`] reconstructs a fresh `Rc`-based value
//! in the receiving task's heap. The pair mirrors [`RuntimeValue::deep_clone`]
//! but crosses the `Send` boundary. Values that cannot cross (closures) yield a
//! [`MarshalError`]; the Send/escape analysis (Phase 4) rejects them statically,
//! so this is a defensive backstop, not the primary gate.

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_runtime::RtPayload;

// The integer-sequence description-length codec (the Auto column menu) lives in the leaf crate so
// the wire layer and the proof layer share one implementation of the format. `describe::` is used
// for the encode/decode entry points; the generator IR and shared helpers are re-imported so their
// existing call sites (and downstream `marshal::GenExpr` references) resolve unchanged.
pub use logicaffeine_base::describe::{gen_eval, GenCmp, GenExpr};
use logicaffeine_base::describe::{
    self, consider, deserialize_gen, detect_affine, emit_best_int_column, serialize_gen,
    MAX_GEN_DEPTH, MAX_GEN_NODES,
};

use crate::interpreter::{ClosureValue, InductiveValue, ListRepr, MapStorage, RuntimeValue, StructValue};

/// Why a value could not be marshalled across a task boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarshalError {
    /// A value type that cannot cross a task boundary (e.g. a closure, whose
    /// captured environment would alias another task's heap).
    NotSendable(&'static str),
}

/// Move a value out of a task's heap into a `Send`-able payload.
pub fn materialize(value: &RuntimeValue) -> Result<RtPayload, MarshalError> {
    Ok(match value {
        RuntimeValue::Int(n) => RtPayload::Int(*n),
        RuntimeValue::BigInt(b) => {
            let (negative, magnitude) = b.to_le_bytes();
            RtPayload::BigInt { negative, magnitude }
        }
        RuntimeValue::Rational(r) => {
            let (num_negative, num_magnitude) = r.numerator().to_le_bytes();
            let (_den_sign, den_magnitude) = r.denominator().to_le_bytes();
            RtPayload::Rational { num_negative, num_magnitude, den_magnitude }
        }
        RuntimeValue::Decimal(d) => {
            let (negative, magnitude, scale) = d.to_le_bytes();
            RtPayload::Decimal { negative, magnitude, scale }
        }
        RuntimeValue::Money(m) => {
            let (negative, magnitude, scale) = m.amount.to_le_bytes();
            RtPayload::Money { negative, magnitude, scale, currency: m.currency.code.to_string() }
        }
        RuntimeValue::Uuid(u) => RtPayload::Uuid(u.to_bytes()),
        RuntimeValue::Complex(c) => {
            let (re_negative, re_num) = c.re().numerator().to_le_bytes();
            let (_, re_den) = c.re().denominator().to_le_bytes();
            let (im_negative, im_num) = c.im().numerator().to_le_bytes();
            let (_, im_den) = c.im().denominator().to_le_bytes();
            RtPayload::Complex { re_negative, re_num, re_den, im_negative, im_num, im_den }
        }
        RuntimeValue::Modular(m) => {
            let (_, value) = m.value().to_le_bytes();
            let (_, modulus) = m.modulus().to_le_bytes();
            RtPayload::Modular { value, modulus }
        }
        RuntimeValue::Float(f) => RtPayload::Float(*f),
        RuntimeValue::Bool(b) => RtPayload::Bool(*b),
        RuntimeValue::Char(c) => RtPayload::Char(*c),
        RuntimeValue::Text(s) => RtPayload::Text((**s).clone()),
        RuntimeValue::Nothing => RtPayload::Nothing,
        RuntimeValue::Duration(n) => RtPayload::Duration(*n),
        RuntimeValue::Date(n) => RtPayload::Date(*n),
        RuntimeValue::Moment(n) => RtPayload::Moment(*n),
        RuntimeValue::Span { months, days } => RtPayload::Span { months: *months, days: *days },
        RuntimeValue::Time(n) => RtPayload::Time(*n),
        RuntimeValue::Word(w) => RtPayload::Word { width: w.width(), bits: w.to_u64() },
        // A SIMD lane vector is a transient compute value (a register), not a wire type — like a
        // closure, it does not cross a task boundary. (A future increment can serialize its lanes.)
        RuntimeValue::Lanes(_) => return Err(MarshalError::NotSendable("Lanes8Word32")),
        RuntimeValue::List(items) => {
            let vals = items.borrow().to_values();
            RtPayload::List(vals.iter().map(materialize).collect::<Result<_, _>>()?)
        }
        RuntimeValue::Set(items) => {
            RtPayload::Set(items.borrow().iter().map(materialize).collect::<Result<_, _>>()?)
        }
        RuntimeValue::Tuple(items) => {
            RtPayload::Tuple(items.iter().map(materialize).collect::<Result<_, _>>()?)
        }
        RuntimeValue::Map(m) => {
            let mut pairs = Vec::new();
            for (k, v) in m.borrow().iter() {
                pairs.push((materialize(k)?, materialize(v)?));
            }
            RtPayload::Map(pairs)
        }
        RuntimeValue::Struct(s) => {
            let mut fields = Vec::new();
            for (name, v) in s.fields.iter() {
                fields.push((name.clone(), materialize(v)?));
            }
            RtPayload::Struct { type_name: s.type_name.clone(), fields }
        }
        RuntimeValue::Inductive(ind) => RtPayload::Inductive {
            type_name: ind.inductive_type.clone(),
            constructor: ind.constructor.clone(),
            args: ind.args.iter().map(materialize).collect::<Result<_, _>>()?,
        },
        // A channel/task handle is an opaque `Send` scheduler id, so it CAN cross
        // a task (and worker-thread) boundary — e.g. passed as a spawn argument.
        // It resolves against the one shared scheduler on the other side.
        RuntimeValue::Chan(id) => RtPayload::Chan(*id),
        RuntimeValue::TaskHandle(id) => RtPayload::TaskHandle(*id),
        // A peer handle is just its topic string — trivially `Send`.
        RuntimeValue::Peer(topic) => RtPayload::Peer((**topic).clone()),
        RuntimeValue::Function(_) => return Err(MarshalError::NotSendable("Function")),
        // A live CRDT shares convergent state via Merge/Sync (the relay wire), not by
        // moving its handle across a task heap — that would alias the same replica.
        RuntimeValue::Crdt(_) => return Err(MarshalError::NotSendable("Crdt")),
        // A Quantity travels as its exact SI magnitude (a rational), its dimension (the exponent
        // vector), and the display unit's symbol — reconstructed losslessly on the far side.
        RuntimeValue::Quantity(qv) => {
            let (num_negative, num_magnitude) = qv.q.magnitude_si().numerator().to_le_bytes();
            let (_den_sign, den_magnitude) = qv.q.magnitude_si().denominator().to_le_bytes();
            let dim = qv.q.dimension();
            let mut dim_num = Vec::with_capacity(logicaffeine_base::BaseDim::COUNT);
            let mut dim_den = Vec::with_capacity(logicaffeine_base::BaseDim::COUNT);
            for d in logicaffeine_base::BaseDim::ALL {
                let e = dim.exponent(d);
                dim_num.push(e.numerator());
                dim_den.push(e.denominator());
            }
            RtPayload::Quantity {
                num_negative,
                num_magnitude,
                den_magnitude,
                dim_num,
                dim_den,
                unit_symbol: qv.unit.symbol.to_string(),
            }
        }
    })
}

/// Reconstruct a fresh `Rc`-based value in the receiving task's heap.
pub fn rebuild(payload: RtPayload) -> RuntimeValue {
    match payload {
        RtPayload::Nothing => RuntimeValue::Nothing,
        RtPayload::Int(n) => RuntimeValue::Int(n),
        RtPayload::BigInt { negative, magnitude } => {
            RuntimeValue::from_bigint(logicaffeine_base::BigInt::from_le_bytes(negative, &magnitude))
        }
        RtPayload::Rational { num_negative, num_magnitude, den_magnitude } => {
            let num = logicaffeine_base::BigInt::from_le_bytes(num_negative, &num_magnitude);
            let den = logicaffeine_base::BigInt::from_le_bytes(false, &den_magnitude);
            // A well-formed payload always has a nonzero denominator; fall back to the
            // numerator (treated as a whole number) if a corrupt one slips through.
            match logicaffeine_base::Rational::new(num.clone(), den) {
                Some(r) => RuntimeValue::from_rational(r),
                None => RuntimeValue::from_bigint(num),
            }
        }
        RtPayload::Decimal { negative, magnitude, scale } => RuntimeValue::Decimal(Rc::new(
            logicaffeine_base::Decimal::from_le_bytes(negative, &magnitude, scale),
        )),
        RtPayload::Money { negative, magnitude, scale, currency } => {
            let amount = logicaffeine_base::Decimal::from_le_bytes(negative, &magnitude, scale);
            let currency = logicaffeine_base::money::currency::by_code(&currency)
                .unwrap_or(logicaffeine_base::Currency { code: "XXX", scale: 0 });
            RuntimeValue::Money(Rc::new(logicaffeine_base::Money { amount, currency }))
        }
        RtPayload::Uuid(bytes) => {
            RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::from_bytes(bytes)))
        }
        RtPayload::Complex { re_negative, re_num, re_den, im_negative, im_num, im_den } => {
            let mk = |neg: bool, num: &[u8], den: &[u8]| {
                logicaffeine_base::Rational::new(
                    logicaffeine_base::BigInt::from_le_bytes(neg, num),
                    logicaffeine_base::BigInt::from_le_bytes(false, den),
                )
                .unwrap_or_else(logicaffeine_base::Rational::zero)
            };
            let re = mk(re_negative, &re_num, &re_den);
            let im = mk(im_negative, &im_num, &im_den);
            RuntimeValue::Complex(Rc::new(logicaffeine_base::Complex::new(re, im)))
        }
        RtPayload::Modular { value, modulus } => {
            let v = logicaffeine_base::BigInt::from_le_bytes(false, &value);
            let n = logicaffeine_base::BigInt::from_le_bytes(false, &modulus);
            match logicaffeine_base::Modular::new(v, n) {
                Some(m) => RuntimeValue::Modular(Rc::new(m)),
                None => RuntimeValue::Nothing, // a corrupt (non-positive) modulus degrades gracefully
            }
        }
        RtPayload::Quantity { num_negative, num_magnitude, den_magnitude, dim_num, dim_den, unit_symbol } => {
            let num = logicaffeine_base::BigInt::from_le_bytes(num_negative, &num_magnitude);
            let den = logicaffeine_base::BigInt::from_le_bytes(false, &den_magnitude);
            let magnitude = logicaffeine_base::Rational::new(num, den)
                .unwrap_or_else(logicaffeine_base::Rational::zero);
            // Rebuild the dimension from its exponent vector (BaseDim::ALL order).
            let mut exps = [logicaffeine_base::Exp::ZERO; logicaffeine_base::BaseDim::COUNT];
            for (i, slot) in exps.iter_mut().enumerate() {
                let n = dim_num.get(i).copied().unwrap_or(0);
                let d = dim_den.get(i).copied().unwrap_or(1);
                *slot = logicaffeine_base::Exp::new(n, if d == 0 { 1 } else { d });
            }
            let dim = logicaffeine_base::Dimension::from_exps(exps);
            // Resolve the display unit by its symbol; a compound or unknown symbol falls back to the
            // SI/dimension display (a synthetic empty-symbol unit at the SI base).
            let unit = logicaffeine_base::quantity::units::by_name(&unit_symbol)
                .filter(|u| u.dimension == dim)
                .unwrap_or_else(|| {
                    logicaffeine_base::Unit::linear("", dim, logicaffeine_base::Rational::one())
                });
            let q = logicaffeine_base::Quantity::si(magnitude, dim);
            RuntimeValue::Quantity(Rc::new(crate::interpreter::QuantityValue { q, unit }))
        }
        RtPayload::Float(f) => RuntimeValue::Float(f),
        RtPayload::Bool(b) => RuntimeValue::Bool(b),
        RtPayload::Char(c) => RuntimeValue::Char(c),
        RtPayload::Text(s) => RuntimeValue::Text(Rc::new(s)),
        RtPayload::Duration(n) => RuntimeValue::Duration(n),
        RtPayload::Date(n) => RuntimeValue::Date(n),
        RtPayload::Moment(n) => RuntimeValue::Moment(n),
        RtPayload::Span { months, days } => RuntimeValue::Span { months, days },
        RtPayload::Time(n) => RuntimeValue::Time(n),
        RtPayload::Word { width, bits } => match logicaffeine_base::WordVal::from_u64(width, bits) {
            Some(w) => RuntimeValue::Word(w),
            // A well-formed payload always carries width 32/64; degrade a corrupt one to its value.
            None => RuntimeValue::Int(bits as i64),
        },
        RtPayload::List(items) => {
            let vals: Vec<RuntimeValue> = items.into_iter().map(rebuild).collect();
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vals))))
        }
        RtPayload::Set(items) => {
            RuntimeValue::Set(Rc::new(RefCell::new(items.into_iter().map(rebuild).collect())))
        }
        RtPayload::Tuple(items) => {
            RuntimeValue::Tuple(Rc::new(items.into_iter().map(rebuild).collect()))
        }
        RtPayload::Map(pairs) => {
            let m: MapStorage = pairs.into_iter().map(|(k, v)| (rebuild(k), rebuild(v))).collect();
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }
        RtPayload::Struct { type_name, fields } => {
            let f = fields.into_iter().map(|(k, v)| (k, rebuild(v))).collect();
            RuntimeValue::Struct(Box::new(StructValue { type_name, fields: f }))
        }
        RtPayload::Inductive { type_name, constructor, args } => {
            RuntimeValue::Inductive(Box::new(InductiveValue {
                inductive_type: type_name,
                constructor,
                args: args.into_iter().map(rebuild).collect(),
            }))
        }
        RtPayload::Chan(id) => RuntimeValue::Chan(id),
        RtPayload::TaskHandle(id) => RuntimeValue::TaskHandle(id),
        RtPayload::Peer(topic) => RuntimeValue::Peer(Rc::new(topic)),
    }
}

// =============================================================================
// Wire codec — a message is any language value, sent over the relay
// =============================================================================
//
// A peer message is just a value, and both ends speak the same language, so the
// wire form IS the value — carrying its type. [`WireValue`] is the network-
// portable shape of a value, mirroring the `Send`-able [`RtPayload`] minus the
// pieces that have no meaning off-machine (a `Function` cannot be marshalled at
// all; a `Chan`/`TaskHandle` is an index into THIS process's scheduler). Its
// `struct`/`inductive` variants carry the type name (and constructor), so a sent
// `Point` is reconstructed as a `Point`, not a bare map — the type rides with the
// value. Encoding is `bincode`: compact binary, no text parsing, and `RtPayload`
// is already the materialized form, so on the far side `rebuild` is the only cost
// left. (Runtime is serde-free by charter, so this serde mirror lives here.)
// Both peers are the same Logos binary, so the non-self-describing encoding is
// sound; swapping `bincode` for a self-describing codec (CBOR) is a one-liner if
// cross-version peers ever matter.

/// A value as it travels the wire — the network-portable projection of
/// [`RtPayload`], type tags and all.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum WireValue {
    Nothing,
    Int(i64),
    BigInt { negative: bool, magnitude: Vec<u8> },
    Rational { num_negative: bool, num_magnitude: Vec<u8>, den_magnitude: Vec<u8> },
    Decimal { negative: bool, magnitude: Vec<u8>, scale: u32 },
    Money { negative: bool, magnitude: Vec<u8>, scale: u32, currency: std::string::String },
    Uuid([u8; 16]),
    Complex {
        re_negative: bool,
        re_num: Vec<u8>,
        re_den: Vec<u8>,
        im_negative: bool,
        im_num: Vec<u8>,
        im_den: Vec<u8>,
    },
    Modular { value: Vec<u8>, modulus: Vec<u8> },
    Quantity {
        num_negative: bool,
        num_magnitude: Vec<u8>,
        den_magnitude: Vec<u8>,
        dim_num: Vec<i32>,
        dim_den: Vec<i32>,
        unit_symbol: String,
    },
    Float(f64),
    Bool(bool),
    Char(char),
    Text(String),
    Duration(i64),
    Date(i32),
    Moment(i64),
    Span { months: i32, days: i32 },
    Time(i64),
    Word { width: u32, bits: u64 },
    Peer(String),
    List(Vec<WireValue>),
    Tuple(Vec<WireValue>),
    Set(Vec<WireValue>),
    Map(Vec<(WireValue, WireValue)>),
    Struct { type_name: String, fields: Vec<(String, WireValue)> },
    Inductive { type_name: String, constructor: String, args: Vec<WireValue> },
}

/// The wire envelope: the sender's inbox topic plus the typed value.
#[derive(serde::Serialize, serde::Deserialize)]
struct WireMessage {
    from: String,
    msg: WireValue,
}

/// One `RtPayload` → its portable [`WireValue`]. `None` if it carries a value
/// with no meaning on another machine — a local channel or task handle.
fn rt_to_wire(p: &RtPayload) -> Option<WireValue> {
    Some(match p {
        RtPayload::Nothing => WireValue::Nothing,
        RtPayload::Int(n) => WireValue::Int(*n),
        RtPayload::BigInt { negative, magnitude } => {
            WireValue::BigInt { negative: *negative, magnitude: magnitude.clone() }
        }
        RtPayload::Rational { num_negative, num_magnitude, den_magnitude } => WireValue::Rational {
            num_negative: *num_negative,
            num_magnitude: num_magnitude.clone(),
            den_magnitude: den_magnitude.clone(),
        },
        RtPayload::Decimal { negative, magnitude, scale } => {
            WireValue::Decimal { negative: *negative, magnitude: magnitude.clone(), scale: *scale }
        }
        RtPayload::Money { negative, magnitude, scale, currency } => WireValue::Money {
            negative: *negative,
            magnitude: magnitude.clone(),
            scale: *scale,
            currency: currency.clone(),
        },
        RtPayload::Uuid(bytes) => WireValue::Uuid(*bytes),
        RtPayload::Complex { re_negative, re_num, re_den, im_negative, im_num, im_den } => {
            WireValue::Complex {
                re_negative: *re_negative,
                re_num: re_num.clone(),
                re_den: re_den.clone(),
                im_negative: *im_negative,
                im_num: im_num.clone(),
                im_den: im_den.clone(),
            }
        }
        RtPayload::Modular { value, modulus } => {
            WireValue::Modular { value: value.clone(), modulus: modulus.clone() }
        }
        RtPayload::Quantity { num_negative, num_magnitude, den_magnitude, dim_num, dim_den, unit_symbol } => {
            WireValue::Quantity {
                num_negative: *num_negative,
                num_magnitude: num_magnitude.clone(),
                den_magnitude: den_magnitude.clone(),
                dim_num: dim_num.clone(),
                dim_den: dim_den.clone(),
                unit_symbol: unit_symbol.clone(),
            }
        }
        RtPayload::Float(f) => WireValue::Float(*f),
        RtPayload::Bool(b) => WireValue::Bool(*b),
        RtPayload::Char(c) => WireValue::Char(*c),
        RtPayload::Text(s) => WireValue::Text(s.clone()),
        RtPayload::Duration(n) => WireValue::Duration(*n),
        RtPayload::Date(n) => WireValue::Date(*n),
        RtPayload::Moment(n) => WireValue::Moment(*n),
        RtPayload::Span { months, days } => WireValue::Span { months: *months, days: *days },
        RtPayload::Time(n) => WireValue::Time(*n),
        RtPayload::Word { width, bits } => WireValue::Word { width: *width, bits: *bits },
        RtPayload::Peer(topic) => WireValue::Peer(topic.clone()),
        RtPayload::List(items) => WireValue::List(rt_seq_to_wire(items)?),
        RtPayload::Tuple(items) => WireValue::Tuple(rt_seq_to_wire(items)?),
        RtPayload::Set(items) => WireValue::Set(rt_seq_to_wire(items)?),
        RtPayload::Map(pairs) => {
            let mut wire_pairs = pairs
                .iter()
                .map(|(k, v)| Some((rt_to_wire(k)?, rt_to_wire(v)?)))
                .collect::<Option<Vec<_>>>()?;
            // Canonical order: sort by the encoded key, so the wire is the same
            // bytes regardless of the source map's (hash) iteration order.
            wire_pairs.sort_by(|a, b| canon_bytes(&a.0).cmp(&canon_bytes(&b.0)));
            WireValue::Map(wire_pairs)
        }
        RtPayload::Struct { type_name, fields } => {
            let mut wire_fields = fields
                .iter()
                .map(|(n, v)| Some((n.clone(), rt_to_wire(v)?)))
                .collect::<Option<Vec<_>>>()?;
            // A struct is a record (unordered fields), so canonicalize by name.
            wire_fields.sort_by(|a, b| a.0.cmp(&b.0));
            WireValue::Struct { type_name: type_name.clone(), fields: wire_fields }
        }
        RtPayload::Inductive { type_name, constructor, args } => WireValue::Inductive {
            type_name: type_name.clone(),
            constructor: constructor.clone(),
            args: rt_seq_to_wire(args)?,
        },
        // A scheduler token indexes THIS process's scheduler — not portable.
        RtPayload::Chan(_) | RtPayload::TaskHandle(_) => return None,
    })
}

fn rt_seq_to_wire(items: &[RtPayload]) -> Option<Vec<WireValue>> {
    items.iter().map(rt_to_wire).collect()
}

/// One [`WireValue`] → its `RtPayload`. The inverse of [`rt_to_wire`]; total.
fn wire_to_rt(w: WireValue) -> RtPayload {
    match w {
        WireValue::Nothing => RtPayload::Nothing,
        WireValue::Int(n) => RtPayload::Int(n),
        WireValue::BigInt { negative, magnitude } => RtPayload::BigInt { negative, magnitude },
        WireValue::Rational { num_negative, num_magnitude, den_magnitude } => {
            RtPayload::Rational { num_negative, num_magnitude, den_magnitude }
        }
        WireValue::Decimal { negative, magnitude, scale } => {
            RtPayload::Decimal { negative, magnitude, scale }
        }
        WireValue::Money { negative, magnitude, scale, currency } => {
            RtPayload::Money { negative, magnitude, scale, currency }
        }
        WireValue::Uuid(bytes) => RtPayload::Uuid(bytes),
        WireValue::Complex { re_negative, re_num, re_den, im_negative, im_num, im_den } => {
            RtPayload::Complex { re_negative, re_num, re_den, im_negative, im_num, im_den }
        }
        WireValue::Modular { value, modulus } => RtPayload::Modular { value, modulus },
        WireValue::Quantity { num_negative, num_magnitude, den_magnitude, dim_num, dim_den, unit_symbol } => {
            RtPayload::Quantity { num_negative, num_magnitude, den_magnitude, dim_num, dim_den, unit_symbol }
        }
        WireValue::Float(f) => RtPayload::Float(f),
        WireValue::Bool(b) => RtPayload::Bool(b),
        WireValue::Char(c) => RtPayload::Char(c),
        WireValue::Text(s) => RtPayload::Text(s),
        WireValue::Duration(n) => RtPayload::Duration(n),
        WireValue::Date(n) => RtPayload::Date(n),
        WireValue::Moment(n) => RtPayload::Moment(n),
        WireValue::Span { months, days } => RtPayload::Span { months, days },
        WireValue::Time(n) => RtPayload::Time(n),
        WireValue::Word { width, bits } => RtPayload::Word { width, bits },
        WireValue::Peer(topic) => RtPayload::Peer(topic),
        WireValue::List(items) => RtPayload::List(items.into_iter().map(wire_to_rt).collect()),
        WireValue::Tuple(items) => RtPayload::Tuple(items.into_iter().map(wire_to_rt).collect()),
        WireValue::Set(items) => RtPayload::Set(items.into_iter().map(wire_to_rt).collect()),
        WireValue::Map(pairs) => {
            RtPayload::Map(pairs.into_iter().map(|(k, v)| (wire_to_rt(k), wire_to_rt(v))).collect())
        }
        WireValue::Struct { type_name, fields } => RtPayload::Struct {
            type_name,
            fields: fields.into_iter().map(|(n, v)| (n, wire_to_rt(v))).collect(),
        },
        WireValue::Inductive { type_name, constructor, args } => RtPayload::Inductive {
            type_name,
            constructor,
            args: args.into_iter().map(wire_to_rt).collect(),
        },
    }
}

/// The wire encoding for a message body.
///
/// `Native` is the default and the fast path — *our* compact tagged-varint binary
/// format, encoded and decoded in a SINGLE PASS straight to/from `RuntimeValue`
/// with no intermediate trees: the hot loop for a list of scalars allocates only
/// the output buffer. `Json` is offered for interop with non-Logos peers (or human
/// debugging) through a real parser (`serde_json`), never a hand-rolled one. Both
/// ride the same relay: every message self-describes its codec in a leading header
/// byte, so any receiver decodes either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireCodec {
    /// Our single-pass tagged-varint binary (default) — the high-throughput path.
    Native,
    /// `serde_json` text — interop / debuggable, larger and slower.
    Json,
}

/// Whether a message carries an integrity checksum.
///
/// Independent of the codec. Pay a few bytes + a hash to have the receiver reject
/// a corrupted or mis-encoded message, or go bare for raw speed. This is
/// *integrity*, not secrecy — for confidentiality run the relay over `wss://`/TLS
/// at the transport layer; a message-level signing/encryption layer is separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireIntegrity {
    /// No checksum — smallest and fastest; the receiver trusts the bytes.
    Raw,
    /// An FNV-1a checksum over the payload — the receiver rejects a corrupted or
    /// mis-encoded message (`message_from_wire` returns `None`).
    Checked,
}

// The framing header byte: bit 0 = integrity, bit 1 = compressed, bits 2-3 = the
// compression codec id (when compressed), bit 4 = payload codec; any other bit set
// is an unknown format and is rejected.
const H_CHECKED: u8 = 0x01;
const H_COMPRESSED: u8 = 0x02;
const H_CODEC: u8 = 0x0C; // bits 2-3: 0=deflate, 1=lz4, 2=zstd (only meaningful when H_COMPRESSED)
const H_JSON: u8 = 0x10;
const H_KNOWN: u8 = H_CHECKED | H_COMPRESSED | H_CODEC | H_JSON;

/// Encode a directed peer message for the relay wire: the sender's inbox topic
/// plus the FULL language value — scalars, collections, structs, inductives,
/// nested, type tags and all — as compact `bincode` under the process default
/// integrity ([`default_integrity`]). Closures, and channel/task handles (local to
/// this process), cannot travel between machines and are reported with a clear
/// error rather than silently dropped.
pub fn message_to_wire(from: &str, value: &RuntimeValue) -> Result<Vec<u8>, String> {
    message_to_wire_with(from, value, WireCodec::Native, current_integrity())
}

/// Encode ONE value as the plain, self-describing recursive wire form — no envelope, no
/// framing, no columnar/compression/dedup transforms. This is the exact format the shared
/// [`logicaffeine_data::wire`] core decodes, so a `RuntimeValue` encoded here round-trips
/// through an AOT-generated type's `wire_decode` (and vice versa). The speed-first form used
/// to hand a program AST to a compile-once native partial evaluator.
pub fn encode_value_raw(v: &RuntimeValue) -> Result<Vec<u8>, String> {
    with_flat_lists(true, || {
        with_structure(WireStructure::Off, || {
            with_dedup(false, || {
                let mut out = Vec::new();
                native_encode(v, &mut out)?;
                Ok(out)
            })
        })
    })
}

/// Decode a value produced by [`encode_value_raw`] (or by an AOT-generated `wire_encode`)
/// back into a `RuntimeValue`. `None` on any malformed or trailing-byte input.
pub fn decode_value_raw(buf: &[u8]) -> Option<RuntimeValue> {
    let mut pos = 0usize;
    let v = native_decode(buf, &mut pos)?;
    if pos == buf.len() {
        Some(v)
    } else {
        None
    }
}

/// What the [`message_to_wire_best`] auto-tuner optimizes for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WireGoal {
    /// Fewest bytes on the wire — measure every applicable encoding and ship the smallest.
    /// The bandwidth-bound choice (a network link).
    Smallest,
    /// Cheapest decode — fixed-width `memcpy` columns, no compression, no structural
    /// transform. The latency / CPU-bound choice (datacenter, shared memory, RDMA).
    Fastest,
}

/// The no-brainer encoder — "just use this." Picks the Pareto-optimal dial combination for
/// `goal` and ships it. Because every wire form is self-describing by its leading tag, this
/// is purely an ENCODE-side decision: the decoder reconstructs via `message_from_wire` with
/// no hint, so `best` interoperates with every existing peer.
///
/// `Smallest` measures the FULL cross product of the size-affecting dials (numerics ×
/// structure × float-coding × compression) and returns the minimum. Because every
/// single-dial configuration is literally one of the candidates, the result is provably
/// never larger than ANY single knob — on any workload. `Fastest` returns the fixed
/// memcpy-decode form directly. (`Smallest` pays N encode passes for the minimum bytes; it
/// is the opt-in "I am bandwidth-bound" choice, not the default.)
pub fn message_to_wire_best(from: &str, value: &RuntimeValue, goal: WireGoal) -> Result<Vec<u8>, String> {
    match goal {
        WireGoal::Fastest => with_numerics(WireNumerics::Fixed, || {
            with_structure(WireStructure::Off, || {
                with_floats(WireFloats::Memcpy, || {
                    with_compression_codec(WireCompression::None, || message_to_wire(from, value))
                })
            })
        }),
        WireGoal::Smallest => smallest_over(
            from,
            value,
            &[WireCompression::None, WireCompression::Deflate, WireCompression::Lz4, WireCompression::Zstd],
        ),
    }
}

/// The size bake-off, parameterized by the compression codecs allowed (so a negotiated send only tries
/// what the receiver can decode). Measures the FULL cross product of the size-affecting dials
/// (numerics × structure/G5 × float-coding) against each allowed compression and returns the smallest —
/// the maximal crush. Runs under whatever type registry is in scope (so name elision composes when the
/// caller enabled it). Every form is self-describing, so the decoder needs no hint.
fn smallest_over(
    from: &str,
    value: &RuntimeValue,
    compressions: &[WireCompression],
) -> Result<Vec<u8>, String> {
    let mut best: Option<Vec<u8>> = None;
    // Structure-sharing dimension: if the SAME subtree is reached more than once, Rc-dedup ships it
    // ONCE + backrefs. It competes as just another bake-off candidate (each is a single dedup-scoped
    // encode, so the per-encode id table never leaks across passes), so it wins ONLY when it actually
    // beats the backref-tag overhead. The gather runs once HERE; no sharing → no dedup pass, no cost.
    let dedup_opts: &[bool] = if value_has_sharing(value) { &[false, true] } else { &[false] };
    for num in [WireNumerics::Varint, WireNumerics::Fixed, WireNumerics::GroupVarint] {
        for st in [WireStructure::Off, WireStructure::Affine, WireStructure::Auto] {
            for fl in [WireFloats::Memcpy, WireFloats::XorDelta] {
                for &comp in compressions {
                    for &dedup in dedup_opts {
                        let bytes = with_dedup(dedup, || {
                            with_numerics(num, || {
                                with_structure(st, || {
                                    with_floats(fl, || {
                                        with_compression_codec(comp, || message_to_wire(from, value))
                                    })
                                })
                            })
                        })?;
                        if best.as_ref().map_or(true, |b| bytes.len() < b.len()) {
                            best = Some(bytes);
                        }
                    }
                }
            }
        }
    }
    best.ok_or_else(|| "no encoding produced".to_string())
}

/// Encode a message TO a peer using EVERYTHING both sides support — the negotiated maximal crush.
/// Applies all the self-describing dials (any peer decodes them) via the size bake-off, but restricts
/// the receiver-capability knobs to the negotiated surface: only compression codecs the receiver can
/// decode, and type-id NAME ELISION only when epochs matched (`neg.use_type_id`). Refuses to ship a
/// computation the receiver declined. Stays MINIMAL in cost too: a tiny message that name-elision can't
/// help ships the plain default without paying for the search. Never larger than the default; always
/// self-describing, so it round-trips on the receiver with no hint.
pub fn message_to_wire_negotiated(
    from: &str,
    value: &RuntimeValue,
    neg: &Negotiated,
    registry: WireTypeRegistry,
) -> Result<Vec<u8>, String> {
    if matches!(value, RuntimeValue::Function(_)) && !neg.may_send_computed {
        return Err("the receiver does not accept computed (shipped-function) sends".to_string());
    }
    let default = message_to_wire(from, value)?;
    // Minimal cost: a small message that type-id can't shrink skips the bake-off entirely.
    if default.len() <= AUTO_SEARCH_THRESHOLD && !neg.use_type_id {
        return Ok(default);
    }
    let codecs: Vec<WireCompression> = if neg.compression == WireCompression::None {
        vec![WireCompression::None]
    } else {
        vec![WireCompression::None, neg.compression]
    };
    let search = || smallest_over(from, value, &codecs);
    // Name elision only fires when the receiver shares our type registry (negotiated epoch match).
    let best = if neg.use_type_id { with_type_registry(registry, search) } else { search() }?;
    Ok(if best.len() < default.len() { best } else { default })
}

/// Below this default-encoding size the `Smallest` search cannot meaningfully shrink a message (the
/// envelope + a scalar / short string is already near-minimal), so [`message_to_wire_auto`] skips the
/// N-pass bake-off and ships the default. Tuned so a scalar / short message never pays for the search.
const AUTO_SEARCH_THRESHOLD: usize = 64;

/// The genuine no-brainer — "just send it." Runs the full [`WireGoal::Smallest`] bake-off ONLY when the
/// payload is large enough for it to matter, and otherwise ships the plain default (so calling this on
/// every message — including scalars and short strings — costs a single encode pass, not the N-pass
/// search). The result is NEVER larger than the default, ALWAYS self-describing (so it interoperates
/// with every peer, no hint), and round-trips exactly. This is the recommended default sender.
pub fn message_to_wire_auto(from: &str, value: &RuntimeValue) -> Result<Vec<u8>, String> {
    let default = message_to_wire(from, value)?;
    if default.len() <= AUTO_SEARCH_THRESHOLD {
        return Ok(default);
    }
    // The bake-off includes the default dial set as one candidate, so its winner is ≤ `default`.
    let best = message_to_wire_best(from, value, WireGoal::Smallest)?;
    Ok(if best.len() < default.len() { best } else { default })
}

/// A receiver's admission-control budget — the limits it will accept from a sender, so a malicious or
/// buggy peer cannot exhaust the receiver's memory, stack, or CPU. Enforced DURING decode, BEFORE the
/// offending allocation or recursion happens, so an over-budget message is refused cleanly (the decode
/// returns `None`) rather than processed. A receiver advertises these in the capability handshake so a
/// cooperative sender stays within them; an uncooperative one is still bounded by the enforcement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiveLimits {
    /// Largest message body (post-decompress framing aside) the receiver will decode, in bytes.
    pub max_bytes: usize,
    /// Maximum container-nesting depth. Bounds decode RECURSION, so a deeply-nested but byte-small
    /// message cannot overflow the receiver's stack (a remote crash).
    pub max_depth: usize,
    /// Maximum element count for any single decoded collection (list / map / set / struct-list /
    /// numeric column). A claimed count above this is refused before the elements are read.
    pub max_elements: usize,
    /// Maximum byte length of any single decoded string.
    pub max_string_bytes: usize,
    /// Whether to accept a SHIPPED computation (`T_FUNC`) at all. When `false`, a computed send is
    /// refused at decode — independent of (and prior to) the C2 acceptance contract that gates whether
    /// an accepted computation may be EVALUATED.
    pub accept_computed: bool,
}

/// Generous-but-finite defaults: every real message passes (genuine nesting is almost always < 10
/// deep); only the pathological / adversarial ones are refused. `max_depth` sits BELOW
/// [`MAX_ENCODE_DEPTH`] on purpose — the recursive DECODER's stack frame is heavier than the encoder's
/// (one giant `match`), so the depth that is safe to recurse on a small (wasm ~1 MiB) stack is lower
/// than what we allow ourselves to encode. A deployment tightens these per peer through the handshake.
pub const DEFAULT_RECEIVE_LIMITS: ReceiveLimits = ReceiveLimits {
    max_bytes: 64 << 20,
    max_depth: 64,
    max_elements: 1 << 24,
    max_string_bytes: 1 << 24,
    accept_computed: true,
};

impl Default for ReceiveLimits {
    fn default() -> Self {
        DEFAULT_RECEIVE_LIMITS
    }
}

thread_local! {
    static RECEIVE_LIMITS: std::cell::Cell<ReceiveLimits> =
        const { std::cell::Cell::new(DEFAULT_RECEIVE_LIMITS) };
    static DECODE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Decode under `limits` for the duration of `f` — the receiver's admission gate. Restores the prior
/// limits afterward (so it nests). Pair with [`message_from_wire`].
pub fn with_receive_limits<T>(limits: ReceiveLimits, f: impl FnOnce() -> T) -> T {
    let prev = RECEIVE_LIMITS.with(|c| c.replace(limits));
    let r = f();
    RECEIVE_LIMITS.with(|c| c.set(prev));
    r
}

/// The limits in force on this thread (default [`DEFAULT_RECEIVE_LIMITS`] outside a
/// [`with_receive_limits`] scope).
fn receive_limits() -> ReceiveLimits {
    RECEIVE_LIMITS.with(std::cell::Cell::get)
}

// ── Optional-capability feature bits a peer advertises in its [`PeerProfile`]. Unknown bits are
//    ignored on receipt, so the set grows forward/backward-compatibly. ──
/// The peer understands DEFLATE-compressed frames.
pub const FEAT_DEFLATE: u32 = 1 << 0;
/// The peer understands LZ4-compressed frames.
pub const FEAT_LZ4: u32 = 1 << 1;
/// The peer understands Zstd-compressed frames.
pub const FEAT_ZSTD: u32 = 1 << 2;
/// The peer can resolve type-id name elision against a shared type registry.
pub const FEAT_TYPE_ID: u32 = 1 << 3;
/// The peer is willing to receive a shipped computation (subject to its acceptance contracts).
pub const FEAT_COMPUTED: u32 = 1 << 4;
/// The peer can reconstruct FEC / erasure-coded redundant frames.
pub const FEAT_FEC: u32 = 1 << 5;

/// A peer's PUBLISHED acceptance surface — the single declarative object it EXPOSES to the other side
/// AND that its own decoder ENFORCES. Declare it once: it is both advertised (in the handshake) and the
/// budget the decode path checks, so the two can never drift. Carries the resource budget
/// ([`ReceiveLimits`]), the peer's type-registry epoch (a content hash of its type table — when both
/// peers' epochs match, struct/enum NAMES need not travel, since both derive the same ids), and a
/// feature bitset (`FEAT_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerProfile {
    /// What this peer will accept — enforced on decode, advertised so a cooperative sender stays within it.
    pub limits: ReceiveLimits,
    /// Content hash of this peer's type table (0 = none). Equal non-zero epochs on both sides unlock
    /// name elision.
    pub registry_epoch: u64,
    /// `FEAT_*` capability bits this peer understands.
    pub features: u32,
}

impl Default for PeerProfile {
    fn default() -> Self {
        Self {
            limits: ReceiveLimits::default(),
            registry_epoch: 0,
            features: FEAT_DEFLATE | FEAT_LZ4 | FEAT_ZSTD | FEAT_TYPE_ID | FEAT_COMPUTED | FEAT_FEC,
        }
    }
}

impl PeerProfile {
    /// The profile to ASSUME for a peer we have NOT yet heard from — NO optional capabilities and NO
    /// shared type registry. Negotiating against it yields a plain, uncompressed, self-describing send
    /// that ANY receiver (even a non-Logos relay consumer) can decode. A capability turns on only once
    /// the peer ADVERTISES it in its handshake. (`Default` is the opposite — the FULL profile a Logos
    /// node advertises for ITSELF.)
    pub const fn conservative() -> Self {
        Self { limits: DEFAULT_RECEIVE_LIMITS, registry_epoch: 0, features: 0 }
    }
}

const PEER_PROFILE_VERSION: u8 = 1;

/// Serialize a [`PeerProfile`] for the handshake — a version byte (so a future layout is recognized,
/// not mis-parsed) followed by the budget, epoch, and feature bits.
pub fn encode_peer_profile(p: &PeerProfile) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.push(PEER_PROFILE_VERSION);
    write_uvarint(p.limits.max_bytes as u64, &mut out);
    write_uvarint(p.limits.max_depth as u64, &mut out);
    write_uvarint(p.limits.max_elements as u64, &mut out);
    write_uvarint(p.limits.max_string_bytes as u64, &mut out);
    out.push(p.limits.accept_computed as u8);
    write_uvarint(p.registry_epoch, &mut out);
    write_uvarint(p.features as u64, &mut out);
    out
}

/// Parse a peer's advertised [`PeerProfile`]. A version this build does not understand, or a truncated
/// blob, yields `None` — the caller then falls back to the conservative defaults (never mis-decodes).
pub fn decode_peer_profile(buf: &[u8]) -> Option<PeerProfile> {
    let mut pos = 0;
    let version = *buf.get(pos)?;
    pos += 1;
    if version != PEER_PROFILE_VERSION {
        return None;
    }
    let max_bytes = read_uvarint(buf, &mut pos)? as usize;
    let max_depth = read_uvarint(buf, &mut pos)? as usize;
    let max_elements = read_uvarint(buf, &mut pos)? as usize;
    let max_string_bytes = read_uvarint(buf, &mut pos)? as usize;
    let accept_computed = *buf.get(pos)? != 0;
    pos += 1;
    let registry_epoch = read_uvarint(buf, &mut pos)?;
    let features = read_uvarint(buf, &mut pos)? as u32;
    Some(PeerProfile {
        limits: ReceiveLimits { max_bytes, max_depth, max_elements, max_string_bytes, accept_computed },
        registry_epoch,
        features,
    })
}

/// The encoding choices negotiated for sending TO a peer, from my profile and the peer's advertised one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Negotiated {
    /// Elide type NAMES from the wire — only when both sides speak type-id AND share the same non-zero
    /// registry epoch (so the receiver can resolve the ids).
    pub use_type_id: bool,
    /// Ship a computed function — only when the receiver both accepts computed sends and advertises the
    /// capability. (The receiver still gates INVOCATION through its acceptance contracts.)
    pub may_send_computed: bool,
    /// The strongest compression BOTH peers understand (`None` if they share none).
    pub compression: WireCompression,
    /// The receiver's byte budget — keep each message under this so it is not refused.
    pub peer_max_bytes: usize,
}

/// Negotiate how to send TO a peer from my profile + the peer's advertised one. CONSERVATIVE: a
/// capability is used only when BOTH sides expose it. This is where "expose properly" pays off — the
/// sender automatically restricts itself to exactly the surface the receiver published, so it never
/// ships a form the receiver can't decode, won't run code the receiver declined, and stays under the
/// receiver's size budget.
pub fn negotiate(mine: &PeerProfile, theirs: &PeerProfile) -> Negotiated {
    let both = |bit: u32| mine.features & bit != 0 && theirs.features & bit != 0;
    let compression = if both(FEAT_ZSTD) {
        WireCompression::Zstd
    } else if both(FEAT_LZ4) {
        WireCompression::Lz4
    } else if both(FEAT_DEFLATE) {
        WireCompression::Deflate
    } else {
        WireCompression::None
    };
    Negotiated {
        use_type_id: both(FEAT_TYPE_ID)
            && mine.registry_epoch != 0
            && mine.registry_epoch == theirs.registry_epoch,
        may_send_computed: theirs.limits.accept_computed && both(FEAT_COMPUTED),
        compression,
        peer_max_bytes: theirs.limits.max_bytes,
    }
}

/// Magic prefix that marks a frame as a capability HANDSHAKE rather than a data message. A data frame
/// begins with a 1-byte header in the small `H_KNOWN` range, so this ASCII prefix can never collide
/// with one — the receiver tells them apart unambiguously.
const HANDSHAKE_MAGIC: &[u8; 4] = b"LCHS";

/// Build a handshake frame advertising `from`'s [`PeerProfile`]: the magic prefix, the sender identity,
/// then the serialized profile. Published like any message but recognized + absorbed (not delivered as
/// data) by the receiver.
pub fn make_handshake_frame(from: &str, profile: &PeerProfile) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + from.len() + 16);
    out.extend_from_slice(HANDSHAKE_MAGIC);
    write_str(from, &mut out);
    out.extend_from_slice(&encode_peer_profile(profile));
    out
}

/// Parse a handshake frame → `(sender, advertised profile)`. `None` when `data` is not a handshake (no
/// magic) or is malformed — so a data message is never mistaken for one, and an unknown/garbage profile
/// is ignored rather than mis-applied.
pub fn parse_handshake_frame(data: &[u8]) -> Option<(String, PeerProfile)> {
    let rest = data.strip_prefix(HANDSHAKE_MAGIC.as_slice())?;
    let mut pos = 0;
    let from = read_str(rest, &mut pos)?;
    let profile = decode_peer_profile(rest.get(pos..)?)?;
    Some((from, profile))
}

/// As [`message_to_wire`], with an explicit codec and integrity mode.
pub fn message_to_wire_with(
    from: &str,
    value: &RuntimeValue,
    codec: WireCodec,
    integrity: WireIntegrity,
) -> Result<Vec<u8>, String> {
    let mut body = match codec {
        // Single pass straight from the live value — no intermediate trees.
        WireCodec::Native => {
            // A small base capacity covers the envelope + a scalar/short message
            // without a realloc; the packed-array arms reserve their own bulk.
            let mut out = Vec::with_capacity(from.len() + 32);
            write_str(from, &mut out);
            native_encode(value, &mut out)?;
            out
        }
        // JSON goes through the serde mirror (interop, not the speed path).
        WireCodec::Json => {
            let payload = materialize(value)
                .map_err(|MarshalError::NotSendable(t)| format!("a {t} cannot be sent over the network"))?;
            let msg = rt_to_wire(&payload)
                .ok_or_else(|| "a channel or task handle cannot be sent over the network".to_string())?;
            serde_json::to_vec(&WireMessage { from: from.to_string(), msg })
                .map_err(|e| format!("message encode failed: {e}"))?
        }
    };
    // Optional compression — but only KEEP it if it actually shrank the body, so a
    // small/incompressible message is never made bigger or slower.
    let mut compression = WireCompression::None;
    if let Some((used, z)) = compress_body(compression_codec(), &body) {
        if z.len() < body.len() {
            body = z;
            compression = used;
        }
    }
    Ok(frame(codec, integrity, compression, body))
}

/// Decode a wire message (from [`message_to_wire`]) into `(sender, value)`,
/// rebuilding the typed value in the local heap. Auto-detects the codec,
/// integrity, and compression; a checksum mismatch, an unknown header, a bad
/// inflate, trailing bytes, or any malformed input all return `None` — never a
/// panic, never a half-rebuilt value.
pub fn message_from_wire(bytes: &[u8]) -> Option<(String, RuntimeValue)> {
    // Admission gate: refuse a message larger than the receiver's byte budget BEFORE decompressing or
    // decoding it — a hostile peer cannot make the receiver spend memory/CPU on an over-budget frame.
    if bytes.len() > receive_limits().max_bytes {
        return None;
    }
    // Fresh dedup memo per top-level message (ids are message-local; never leak across messages).
    DECODE_MEMO.with(|c| c.borrow_mut().clear());
    let (codec, compression, framed) = unframe(bytes)?;
    // Inflate first if needed (the checksum, already verified, covered the
    // compressed bytes — so we never spend CPU inflating a corrupt message). The
    // codec is read off the header, so any peer decodes any sender's choice.
    let inflated;
    let body: &[u8] = if compression == WireCompression::None {
        framed
    } else {
        inflated = decompress_body(compression, framed)?;
        &inflated
    };
    match codec {
        WireCodec::Native => {
            let mut pos = 0;
            let from = read_str(body, &mut pos)?;
            let value = native_decode(body, &mut pos)?;
            (pos == body.len()).then_some((from, value)) // reject trailing bytes
        }
        WireCodec::Json => {
            let WireMessage { from, msg } = serde_json::from_slice(body).ok()?;
            Some((from, rebuild(wire_to_rt(msg))))
        }
    }
}

/// The plain-words name of the structural encoding a column tag selects — the codec's own
/// dial vocabulary, for surfacing *which* encoding actually won (benchmarks, docs, debug).
/// A non-column or unknown tag reads as the generic `"value"`.
fn column_tag_name(tag: u8) -> &'static str {
    match tag {
        T_INTS => "varint",
        T_INTS_FIXED => "fixed (memcpy)",
        T_INTS_GV => "group-varint",
        T_INTS_ALIGNED => "fixed-aligned",
        T_INTS_AFFINE => "affine (base,stride,n)",
        T_INTS_DELTA => "delta",
        T_INTS_DOD => "delta-of-delta",
        T_INTS_FOR => "FOR bit-pack",
        T_INTS_RLE => "run-length",
        T_INTS_DICT => "dictionary",
        T_INTS_POLY => "polynomial",
        T_INTS_GEOMETRIC => "geometric",
        T_INTS_PERIODIC => "periodic",
        T_INTS_SPARSE => "sparse",
        T_GEN => "generator",
        T_BYTES => "byte column",
        describe::T_INTS_LRECUR => "linear-recurrence",
        describe::T_INTS_LFSR => "LFSR",
        describe::T_INTS_FCSR => "FCSR",
        T_FLOATS => "memcpy floats",
        T_FLOATS_XOR => "xor-delta floats",
        T_FLOATS_CONST => "constant floats",
        T_FLOATS_AFFINE => "affine floats",
        T_FLOATS_SPARSE => "sparse floats",
        T_FLOATS_PERIODIC => "periodic floats",
        T_FLOATS_GEOMETRIC => "geometric floats",
        T_FLOATS_ALIGNED => "aligned floats",
        T_BOOLS => "bit-packed bools",
        T_BOOLS_PERIODIC => "periodic bools",
        T_BOOLS_RLE => "run-length bools",
        T_STRINGS => "flat strings",
        T_STRINGS_TEMPLATE => "templated strings",
        T_STRINGS_FRONT => "front-coded strings",
        T_STRINGS_AFFIX => "affix strings",
        T_STRINGS_DICT => "dictionary strings",
        T_SET_INTS => "int set (column menu)",
        T_SET_STRINGS => "string set (front-coded)",
        T_MAP_INTKEY => "int-keyed map (columnar)",
        _ => "value",
    }
}

/// Name the structural encoding of each column in a native wire message — the codec
/// explaining its own output, so "which dial won" is legible to a human. A single-column
/// message (int / float / string / bool list, a set, an int-keyed map) yields one name; a
/// record list (`T_STRUCTS`) yields one `"field: encoding"` per field. Empty for a shape it
/// does not model (compressed, JSON, a bare scalar, or a malformed frame) — the caller then
/// shows a generic label. It reuses the real decode dispatch to skip each column body, so it
/// can never drift from the format it reports on.
pub fn describe_columns(bytes: &[u8]) -> Vec<String> {
    describe_columns_inner(bytes).unwrap_or_default()
}

fn describe_columns_inner(bytes: &[u8]) -> Option<Vec<String>> {
    let (codec, compression, body) = unframe(bytes)?;
    if !matches!(codec, WireCodec::Native) || compression != WireCompression::None {
        return None; // callers inspect the uncompressed native bytes
    }
    DECODE_MEMO.with(|c| c.borrow_mut().clear());
    let mut pos = 0;
    let _from = read_str(body, &mut pos)?;
    let tag = *body.get(pos)?;
    if tag == T_STRUCTS {
        let mut p = pos + 1;
        let (_type_name, field_names) = read_struct_schema(body, &mut p)?;
        let _rows = read_uvarint(body, &mut p)?;
        let mut out = Vec::with_capacity(field_names.len());
        for name in &field_names {
            let col_tag = *body.get(p)?;
            native_decode(body, &mut p)?; // advance past the column via the real decoder
            out.push(format!("{name}: {}", column_tag_name(col_tag)));
        }
        return Some(out);
    }
    Some(vec![column_tag_name(tag).to_string()])
}

/// How a connection-scoped schema dictionary identifies a struct schema on the wire.
/// All modes are corruption-free; they trade size against robustness.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WireSchemaMode {
    /// No dictionary — the schema is always inline (`T_STRUCTS`). Always safe.
    #[default]
    Off,
    /// Position ids: a 1-byte counter. The smallest, but the id only means anything
    /// relative to ONE sender's ordered stream — use only for a reliable
    /// point-to-point connection (one sender per receiver cache).
    Sequential,
    /// Content-addressed: the id is a 64-bit fingerprint of the schema itself, so it
    /// is sender-independent and order-independent. Multiple senders, reordering, and
    /// loss are all safe (a reference to an unknown fingerprint resolves to `None`,
    /// never to the wrong schema; a definition whose fingerprint conflicts with a
    /// different cached schema is rejected). The footgun-free default for a mesh.
    ContentAddressed,
}

/// A connection-scoped schema dictionary (one per direction per peer). A struct
/// schema (type name + field names) is transmitted ONCE and referenced thereafter —
/// the RPC-grade win for streams of same-shaped struct messages. The mode picks the
/// id scheme (see [`WireSchemaMode`]); every mode is corruption-free — a decoder that
/// cannot resolve a reference returns `None` rather than mis-decoding. An optional
/// keyframe interval re-emits a definition every *k* references so a late or lossy
/// receiver self-heals.
#[derive(Debug)]
pub struct WireSchemaCache {
    mode: WireSchemaMode,
    // Sequential (position) state.
    send_seq: std::collections::HashMap<(String, Vec<String>), u32>,
    recv_seq: Vec<(String, Vec<String>)>,
    next: u32,
    // Content-addressed (fingerprint) state.
    send_ca: std::collections::HashSet<u64>,
    recv_ca: std::collections::HashMap<u64, (String, Vec<String>)>,
    // Self-healing: re-emit a definition every `keyframe` references (content mode).
    keyframe: Option<u32>,
    refs_since_def: std::collections::HashMap<u64, u32>,
}

impl Default for WireSchemaCache {
    /// The footgun-free default: content-addressed.
    fn default() -> Self {
        Self::with_mode(WireSchemaMode::ContentAddressed)
    }
}

impl WireSchemaCache {
    fn with_mode(mode: WireSchemaMode) -> Self {
        Self {
            mode,
            send_seq: std::collections::HashMap::new(),
            recv_seq: Vec::new(),
            next: 0,
            send_ca: std::collections::HashSet::new(),
            recv_ca: std::collections::HashMap::new(),
            keyframe: None,
            refs_since_def: std::collections::HashMap::new(),
        }
    }
    /// Content-addressed (footgun-free): safe for multiple senders, reordering, loss.
    pub fn content_addressed() -> Self {
        Self::with_mode(WireSchemaMode::ContentAddressed)
    }
    /// Position ids (smallest): for a single reliable ordered point-to-point stream.
    pub fn sequential() -> Self {
        Self::with_mode(WireSchemaMode::Sequential)
    }
    /// Re-emit a schema definition every `k` references, so a late/lossy receiver
    /// self-heals (content-addressed mode).
    pub fn with_keyframe(mut self, k: u32) -> Self {
        self.keyframe = Some(k);
        self
    }
}

/// The 64-bit content fingerprint of a struct schema — a sender-independent identity.
/// Collisions are negligible for realistic schema counts, and a fingerprint clash
/// with a *different* cached schema is rejected on definition, so it cannot corrupt.
fn schema_fingerprint(type_name: &str, field_names: &[String]) -> u64 {
    let mut bytes = Vec::with_capacity(type_name.len() + 8);
    bytes.extend_from_slice(type_name.as_bytes());
    for f in field_names {
        bytes.push(0);
        bytes.extend_from_slice(f.as_bytes());
    }
    fnv1a_64(&bytes)
}

/// What the encoder should emit for a struct list's schema, per the active cache.
enum SchemaEmit {
    Inline,
    SeqDef(u32),
    SeqRef(u32),
    CaDef,
    CaRef(u64),
}

thread_local! {
    // The cache in force for the current cached encode/decode. Swapped in by the
    // `*_cached` entry points for the duration of the call, then swapped back out.
    static SCHEMA_CACHE: RefCell<Option<WireSchemaCache>> = const { RefCell::new(None) };
}

/// RAII: install `cache` into the thread-local for the duration of a cached
/// encode/decode and ALWAYS swap it back out — on normal return AND on a panic
/// unwind — so a panic mid-codec can never strand or poison the schema state.
struct CacheScope<'a> {
    cache: &'a mut WireSchemaCache,
}
impl<'a> CacheScope<'a> {
    fn enter(cache: &'a mut WireSchemaCache) -> Self {
        SCHEMA_CACHE.with(|c| *c.borrow_mut() = Some(std::mem::take(cache)));
        Self { cache }
    }
}
impl Drop for CacheScope<'_> {
    fn drop(&mut self) {
        SCHEMA_CACHE.with(|c| *self.cache = c.borrow_mut().take().unwrap_or_default());
    }
}

/// As [`message_to_wire_with`], but a known struct schema is sent by reference
/// instead of inline (per the cache's [`WireSchemaMode`]).
pub fn message_to_wire_cached(
    from: &str,
    value: &RuntimeValue,
    codec: WireCodec,
    integrity: WireIntegrity,
    cache: &mut WireSchemaCache,
) -> Result<Vec<u8>, String> {
    let _scope = CacheScope::enter(cache);
    message_to_wire_with(from, value, codec, integrity)
}

/// As [`message_from_wire`], but resolves schema references against `cache` and
/// records schema definitions into it.
pub fn message_from_wire_cached(bytes: &[u8], cache: &mut WireSchemaCache) -> Option<(String, RuntimeValue)> {
    let _scope = CacheScope::enter(cache);
    message_from_wire(bytes)
}

/// Encode side: decide how to transmit a struct schema, recording state as needed.
fn schema_send(type_name: &str, field_names: &[String]) -> SchemaEmit {
    SCHEMA_CACHE.with(|c| {
        let mut g = c.borrow_mut();
        let Some(cache) = g.as_mut() else { return SchemaEmit::Inline };
        match cache.mode {
            WireSchemaMode::Off => SchemaEmit::Inline,
            WireSchemaMode::Sequential => {
                let key = (type_name.to_string(), field_names.to_vec());
                if let Some(&id) = cache.send_seq.get(&key) {
                    SchemaEmit::SeqRef(id)
                } else {
                    let id = cache.next;
                    cache.next += 1;
                    cache.send_seq.insert(key, id);
                    SchemaEmit::SeqDef(id)
                }
            }
            WireSchemaMode::ContentAddressed => {
                let fp = schema_fingerprint(type_name, field_names);
                let known = cache.send_ca.contains(&fp);
                let count = cache.refs_since_def.entry(fp).or_insert(0);
                let keyframe_due = matches!(cache.keyframe, Some(k) if known && *count >= k);
                if known && !keyframe_due {
                    *count += 1;
                    SchemaEmit::CaRef(fp)
                } else {
                    *count = 0;
                    cache.send_ca.insert(fp);
                    SchemaEmit::CaDef
                }
            }
        }
    })
}

/// Decode side (sequential): record a definition at `id`. Ids must arrive in order;
/// a matching re-definition is tolerated, a gap or a conflict is rejected. `true`
/// when there is no cache (a definition is self-decodable inline regardless).
fn schema_recv_register_seq(id: u32, type_name: &str, field_names: &[String]) -> bool {
    SCHEMA_CACHE.with(|c| {
        let mut g = c.borrow_mut();
        let Some(cache) = g.as_mut() else { return true };
        let entry = (type_name.to_string(), field_names.to_vec());
        let idx = id as usize;
        if idx == cache.recv_seq.len() {
            cache.recv_seq.push(entry);
            true
        } else if idx < cache.recv_seq.len() {
            cache.recv_seq[idx] == entry
        } else {
            false
        }
    })
}

/// Decode side (sequential): resolve a reference; `None` if unknown.
fn schema_recv_lookup_seq(id: u32) -> Option<(String, Vec<String>)> {
    SCHEMA_CACHE.with(|c| c.borrow().as_ref().and_then(|cache| cache.recv_seq.get(id as usize).cloned()))
}

/// Decode side (content-addressed): record a definition under its fingerprint. A
/// fingerprint that already maps to a DIFFERENT schema (a collision) is rejected —
/// so a clash can never corrupt, only fail. `true` when there is no cache.
fn schema_recv_register_ca(type_name: &str, field_names: &[String]) -> bool {
    SCHEMA_CACHE.with(|c| {
        let mut g = c.borrow_mut();
        let Some(cache) = g.as_mut() else { return true };
        let fp = schema_fingerprint(type_name, field_names);
        let entry = (type_name.to_string(), field_names.to_vec());
        match cache.recv_ca.get(&fp) {
            Some(existing) => *existing == entry, // collision with a different schema → reject
            None => {
                cache.recv_ca.insert(fp, entry);
                true
            }
        }
    })
}

/// Decode side (content-addressed): resolve a reference by fingerprint; `None` if the
/// definition was never seen (reordering / loss / stale decode).
fn schema_recv_lookup_ca(fp: u64) -> Option<(String, Vec<String>)> {
    SCHEMA_CACHE.with(|c| c.borrow().as_ref().and_then(|cache| cache.recv_ca.get(&fp).cloned()))
}

/// A program-derived registry of every struct/enum schema, shared by both ends of a
/// Logos↔Logos link (each side builds it from the SAME program type definitions). Every
/// type gets a stable small id — canonical by fingerprint, so declaration order is
/// irrelevant and sender + receiver always agree. The codec ships the id instead of the
/// type/field NAMES, and the receiver — running the same program — resolves it. This is
/// the "duh, you use that" default that drops names off the wire entirely.
#[derive(Debug, Default, Clone)]
pub struct WireTypeRegistry {
    by_id: Vec<(String, Vec<String>)>,
    by_fp: std::collections::HashMap<u64, u32>,
    // Enums: id → (type_name, ordered constructor list). The constructor ORDER is part of
    // the type def (we ship a constructor *index*), so it is preserved, not sorted.
    enums_by_id: Vec<(String, Vec<String>)>,
    enums_by_name: std::collections::HashMap<String, u32>,
}

impl WireTypeRegistry {
    /// Build from `(type_name, field_names)` struct schemas. Field names are sorted (the
    /// codec's canonical order), duplicates collapsed, and the set ordered by fingerprint
    /// so two peers that declared the same types in any order assign identical ids.
    pub fn new(schemas: Vec<(String, Vec<String>)>) -> Self {
        let mut canon: Vec<(String, Vec<String>)> = schemas
            .into_iter()
            .map(|(n, mut f)| {
                f.sort();
                (n, f)
            })
            .collect();
        canon.sort_by_key(|(n, f)| schema_fingerprint(n, f));
        canon.dedup_by_key(|(n, f)| schema_fingerprint(n, f));
        let by_fp = canon
            .iter()
            .enumerate()
            .map(|(i, (n, f))| (schema_fingerprint(n, f), i as u32))
            .collect();
        Self { by_id: canon, by_fp, ..Self::default() }
    }

    /// Add enum types `(type_name, ordered_constructors)`. Constructor order is preserved
    /// (the wire ships a constructor index); the enum set is ordered by fingerprint so
    /// both peers assign identical enum ids regardless of declaration order.
    pub fn with_enums(mut self, enums: Vec<(String, Vec<String>)>) -> Self {
        let mut canon = enums;
        canon.sort_by_key(|(n, c)| schema_fingerprint(n, c));
        canon.dedup_by_key(|(n, c)| schema_fingerprint(n, c));
        self.enums_by_name = canon
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (n.clone(), i as u32))
            .collect();
        self.enums_by_id = canon;
        self
    }

    /// A content hash of this registry's WHOLE type set — the registry EPOCH advertised in the
    /// handshake. Two peers that declared the SAME struct + enum types (in any order) compute the same
    /// epoch, so when their epochs MATCH they may elide type NAMES from the wire (type-id). `0` for an
    /// empty registry — no shared types, so never elide. Deterministic: folds the per-type
    /// fingerprints, which are already in canonical fingerprint order.
    pub fn epoch(&self) -> u64 {
        if self.by_id.is_empty() && self.enums_by_id.is_empty() {
            return 0;
        }
        let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
        for (n, f) in &self.by_id {
            acc = acc.rotate_left(5) ^ schema_fingerprint(n, f);
        }
        for (n, c) in &self.enums_by_id {
            acc = acc.rotate_left(7) ^ schema_fingerprint(n, c).wrapping_mul(0x0000_0100_0000_01b3);
        }
        acc.max(1) // a non-empty registry is never epoch 0 (0 means "no registry")
    }

    fn id_of(&self, type_name: &str, field_names: &[String]) -> Option<u32> {
        self.by_fp.get(&schema_fingerprint(type_name, field_names)).copied()
    }
    fn schema_of(&self, id: u32) -> Option<(String, Vec<String>)> {
        self.by_id.get(id as usize).cloned()
    }
    /// The enum id + the constructor's index, when this enum type is registered.
    fn enum_id_of(&self, type_name: &str, constructor: &str) -> Option<(u32, u32)> {
        let id = *self.enums_by_name.get(type_name)?;
        let (_, ctors) = self.enums_by_id.get(id as usize)?;
        let idx = ctors.iter().position(|c| c == constructor)? as u32;
        Some((id, idx))
    }
    /// The `(type_name, ordered constructors)` for an enum id.
    fn enum_schema_of(&self, id: u32) -> Option<(String, Vec<String>)> {
        self.enums_by_id.get(id as usize).cloned()
    }
}

thread_local! {
    static TYPE_REGISTRY: RefCell<Option<WireTypeRegistry>> = const { RefCell::new(None) };
}

/// Install a shared type registry for the duration of `f` (consulted by BOTH encode and
/// decode). Restores the previous registry on return or panic.
pub fn with_type_registry<T>(reg: WireTypeRegistry, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<WireTypeRegistry>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TYPE_REGISTRY.with(|c| *c.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(TYPE_REGISTRY.with(|c| c.borrow_mut().replace(reg)));
    f()
}

/// The id the active registry assigns this struct/enum schema, if it knows it.
fn type_registry_id(type_name: &str, field_names: &[String]) -> Option<u32> {
    TYPE_REGISTRY.with(|c| c.borrow().as_ref().and_then(|r| r.id_of(type_name, field_names)))
}

/// Resolve a registry id back to its `(type_name, field_names)` schema.
fn type_registry_schema(id: u32) -> Option<(String, Vec<String>)> {
    TYPE_REGISTRY.with(|c| c.borrow().as_ref().and_then(|r| r.schema_of(id)))
}

/// The active registry's `(enum_id, constructor_index)` for an enum value, if known.
fn type_registry_enum_id(type_name: &str, constructor: &str) -> Option<(u32, u32)> {
    TYPE_REGISTRY.with(|c| c.borrow().as_ref().and_then(|r| r.enum_id_of(type_name, constructor)))
}

/// Resolve an enum id back to its `(type_name, ordered constructors)`.
fn type_registry_enum_schema(id: u32) -> Option<(String, Vec<String>)> {
    TYPE_REGISTRY.with(|c| c.borrow().as_ref().and_then(|r| r.enum_schema_of(id)))
}

thread_local! {
    // Rc-DEDUP (G8): when enabled, a subtree that the SAME `Rc` reaches more than once on the value
    // graph ships ONCE (`T_SHARED_DEF id + value`) and every later occurrence ships a tiny backref
    // (`T_SHARED_REF id`) — and the decoder rebuilds the SHARING (one `Rc`, aliased), not N copies.
    // Off by default, so every existing byte-stream is untouched; a value with no actual sharing is
    // byte-identical even with the knob on (nothing is in `ENCODE_SHARED`, so no tag is emitted).
    static DEDUP_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    // The set of `Rc` pointers that occur ≥2× — computed once at the root encode. `None` until the
    // root gathers it (the lazy-init signal).
    static ENCODE_SHARED: RefCell<Option<std::collections::HashSet<usize>>> = const { RefCell::new(None) };
    // Shared pointer → the id assigned at its first occurrence (so later occurrences reference it).
    static ENCODE_WRITTEN: RefCell<std::collections::HashMap<usize, u64>> =
        RefCell::new(std::collections::HashMap::new());
    // Decode side: id → the value first decoded under it, so a `T_SHARED_REF` resolves to the SAME
    // `Rc` (sharing preserved). Reset at the top of every top-level `message_from_wire`.
    static DECODE_MEMO: RefCell<std::collections::HashMap<u64, RuntimeValue>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Encode `f`'s value with Rc-dedup ON: shared subtrees ship once + backrefs. Self-describing by tag,
/// so the receiver rebuilds the sharing with no knob of its own. The default (OFF) is byte-unchanged.
pub fn with_dedup<T>(enabled: bool, f: impl FnOnce() -> T) -> T {
    let prev = DEDUP_ENABLED.with(|c| c.replace(enabled));
    ENCODE_SHARED.with(|c| *c.borrow_mut() = None);
    ENCODE_WRITTEN.with(|c| c.borrow_mut().clear());
    let r = f();
    DEDUP_ENABLED.with(|c| c.set(prev));
    ENCODE_SHARED.with(|c| *c.borrow_mut() = None);
    ENCODE_WRITTEN.with(|c| c.borrow_mut().clear());
    r
}

/// The `Rc`-backed value types whose pointer identity can be shared — the dedup candidates. A
/// `usize` address uniquely keys an allocation (two `Rc`s of one allocation share it; distinct
/// allocations never collide, even across types).
fn shareable_ptr(v: &RuntimeValue) -> Option<usize> {
    match v {
        RuntimeValue::List(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Tuple(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Set(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Map(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Text(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::BigInt(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Rational(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Decimal(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Money(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Complex(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Modular(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        RuntimeValue::Quantity(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        _ => None,
    }
}

/// Walk the value graph counting `Rc` occurrences; any pointer seen ≥2× goes in `shared`. Cycle-safe
/// (a pointer is descended into only on its FIRST sighting; the 2nd marks it shared and stops) and
/// depth-bounded (deep non-cyclic nesting just stops gathering — missing a share only costs a little
/// size, never correctness).
fn gather_shared(
    v: &RuntimeValue,
    seen: &mut std::collections::HashMap<usize, u32>,
    shared: &mut std::collections::HashSet<usize>,
    depth: u32,
) {
    if depth >= MAX_ENCODE_DEPTH {
        return;
    }
    if let Some(p) = shareable_ptr(v) {
        let c = seen.entry(p).or_insert(0);
        *c += 1;
        if *c >= 2 {
            shared.insert(p);
            return; // already descended on the first sighting — stop (this is the cycle guard too)
        }
    }
    match v {
        RuntimeValue::List(rc) => {
            if let ListRepr::Boxed(items) = &*rc.borrow() {
                for x in items {
                    gather_shared(x, seen, shared, depth + 1);
                }
            }
        }
        RuntimeValue::Tuple(rc) => {
            for x in rc.iter() {
                gather_shared(x, seen, shared, depth + 1);
            }
        }
        RuntimeValue::Set(rc) => {
            for x in rc.borrow().iter() {
                gather_shared(x, seen, shared, depth + 1);
            }
        }
        RuntimeValue::Map(rc) => {
            for (k, val) in rc.borrow().iter() {
                gather_shared(k, seen, shared, depth + 1);
                gather_shared(val, seen, shared, depth + 1);
            }
        }
        RuntimeValue::Struct(b) => {
            for val in b.fields.values() {
                gather_shared(val, seen, shared, depth + 1);
            }
        }
        RuntimeValue::Inductive(b) => {
            for x in &b.args {
                gather_shared(x, seen, shared, depth + 1);
            }
        }
        _ => {}
    }
}

/// Does `v` actually alias a subtree — the same `Rc` reached more than once? The auto-tuner asks this
/// (one cheap graph walk) to decide whether the dedup candidate is even worth trying; a tree-shaped
/// value answers `false` and pays nothing more.
fn value_has_sharing(v: &RuntimeValue) -> bool {
    let mut seen = std::collections::HashMap::new();
    let mut shared = std::collections::HashSet::new();
    gather_shared(v, &mut seen, &mut shared, 0);
    !shared.is_empty()
}

/// At the encoder's entry for `v`: if dedup is on, lazily gather the shared set at the root, then —
/// for a shared value — emit a backref (and signal `caller returns`) or stamp a fresh def id (and
/// let the caller fall through to encode the value normally). Returns `Some(true)` = "I wrote a
/// backref, return now", `Some(false)`/`None` = "keep encoding".
fn dedup_encode_prefix(v: &RuntimeValue, out: &mut Vec<u8>) -> bool {
    if !DEDUP_ENABLED.with(|c| c.get()) {
        return false;
    }
    if ENCODE_SHARED.with(|c| c.borrow().is_none()) {
        let mut seen = std::collections::HashMap::new();
        let mut shared = std::collections::HashSet::new();
        gather_shared(v, &mut seen, &mut shared, 0);
        ENCODE_SHARED.with(|c| *c.borrow_mut() = Some(shared));
        ENCODE_WRITTEN.with(|c| c.borrow_mut().clear());
    }
    let Some(p) = shareable_ptr(v) else { return false };
    let is_shared = ENCODE_SHARED.with(|c| c.borrow().as_ref().is_some_and(|s| s.contains(&p)));
    if !is_shared {
        return false;
    }
    if let Some(id) = ENCODE_WRITTEN.with(|c| c.borrow().get(&p).copied()) {
        out.push(T_SHARED_REF);
        write_uvarint(id, out);
        return true; // a backref — caller returns
    }
    let id = ENCODE_WRITTEN.with(|c| {
        let mut m = c.borrow_mut();
        let id = m.len() as u64;
        m.insert(p, id);
        id
    });
    out.push(T_SHARED_DEF);
    write_uvarint(id, out);
    false // first occurrence — caller encodes the value normally after this def header
}

thread_local! {
    static STRUCT_VIEW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    // Current value-recursion depth of the encoder — bounds nesting so a cyclic value
    // (only constructible via the `Rc<RefCell<…>>` a List wraps) returns a clean Err
    // instead of overflowing the stack. Reset to 0 by the guard as the recursion unwinds.
    static ENCODE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

// Bounds value-recursion depth (NESTING, not breadth — a million-element list is depth 2).
// 128 levels is far beyond any real payload yet safe on the small stacks this runs on: an
// unoptimized `native_encode`/`encode_list_repr` frame is several KiB in debug, so the cap
// must stay well under a 2 MiB worker/test or ~1 MiB wasm stack — 128 leaves a wide margin.
const MAX_ENCODE_DEPTH: u32 = 128;

/// RAII depth counter for the recursive encoder. `enter()` fails (rather than recursing
/// into a stack overflow) once nesting passes [`MAX_ENCODE_DEPTH`]; `Drop` unwinds it.
struct DepthGuard;
impl DepthGuard {
    fn enter() -> Result<DepthGuard, String> {
        ENCODE_DEPTH.with(|d| {
            let n = d.get();
            if n >= MAX_ENCODE_DEPTH {
                return Err(format!(
                    "value nested deeper than {MAX_ENCODE_DEPTH} (cyclic or pathological) — not encodable"
                ));
            }
            d.set(n + 1);
            Ok(DepthGuard)
        })
    }
}
impl Drop for DepthGuard {
    fn drop(&mut self) {
        ENCODE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// RAII depth counter for the recursive DECODER — the admission gate against a stack-smashing message.
/// `enter()` returns `None` (so `native_decode` rejects cleanly via `?`) once nesting reaches the
/// receiver's [`ReceiveLimits::max_depth`], instead of recursing into a stack overflow on a crafted
/// deeply-nested payload. `Drop` unwinds the count on every path — normal return, `?`-`None`, and panic
/// unwind — so a fresh top-level decode always starts at zero.
struct DecodeDepthGuard;
impl DecodeDepthGuard {
    fn enter() -> Option<DecodeDepthGuard> {
        DECODE_DEPTH.with(|d| {
            let n = d.get();
            if n >= receive_limits().max_depth {
                return None;
            }
            d.set(n + 1);
            Some(DecodeDepthGuard)
        })
    }
}
impl Drop for DecodeDepthGuard {
    fn drop(&mut self) {
        DECODE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// Encode structs in the offset-table `T_STRUCT_VIEW` layout for the duration of `f`, so a
/// `WireView` reads any single field in O(1) (the Cap'n Proto-beating random-access mode).
/// Larger than the packed forms — it is the speed end of the size↔speed dial.
pub fn with_struct_view<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let prev = STRUCT_VIEW.with(|c| c.replace(on));
    let out = f();
    STRUCT_VIEW.with(|c| c.set(prev));
    out
}

fn struct_view_on() -> bool {
    STRUCT_VIEW.with(std::cell::Cell::get)
}

// ---- The native single-pass codec: RuntimeValue <-> tagged-varint bytes -----
//
// One byte of type tag, then a varint/utf8 payload. Signed integers are
// zig-zag + LEB128 (small magnitudes cost a byte); lengths are LEB128. Structs
// and maps are written in canonical order (fields by name, entries by key bytes)
// so the encoding is deterministic. Non-portable values (closures, scheduler
// handles) are rejected here, on the spot.

// =====================================================================================
// Zero-copy WireView — read ONE field/element without decoding the whole message.
// Borrows the wire bytes (`&'a [u8]`); a fixed-width array element is read in O(1) at its
// byte offset with ZERO allocation. Matches Cap'n Proto / Arrow random-access (O(1), no
// parse) while staying varint-small — `Send fast` (the fixed layout) is the zero-copy one.
// =====================================================================================

/// Advance `pos` past a length-prefixed string without allocating it.
fn skip_str(buf: &[u8], pos: &mut usize) -> Option<()> {
    let n = read_uvarint(buf, pos)? as usize;
    let end = pos.checked_add(n)?;
    if end > buf.len() {
        return None;
    }
    *pos = end;
    Some(())
}

/// Decode a received message LAZILY when its top-level value is a self-describing record-list view
/// (`T_STRUCTS_VIEW`): returns `(sender, List(WireStructs))` holding the raw frame, so NO row is
/// decoded until a field is touched — the production zero-copy receive ("no decode in production",
/// Cap'n Proto's home). Any other shape (scalars, maps, single structs, cached/compressed bodies)
/// falls back to a full [`message_from_wire`] decode, so every message still round-trips. The
/// receiver opts in with the `view` knob; without it, the eager path is used exactly as before.
/// Peek a frame: if its top-level value is a self-describing DEFERRABLE view — a record list
/// (`T_STRUCTS_VIEW`) or an aligned numeric column (`T_INTS_ALIGNED`/`T_FLOATS_ALIGNED`), all of
/// which have no schema-cache dependency — return the sender so its decode can be deferred to
/// `Await` (lazy under `view`, eager otherwise). `None` for anything else (scalars, single structs,
/// maps, cached, or compressed bodies), which must decode eagerly in arrival order. The drain loop
/// uses this to split deferrable views from order-sensitive messages.
pub fn peek_deferrable_sender(bytes: &[u8]) -> Option<String> {
    let view = view_message(bytes)?;
    let deferrable =
        view.structs_schema().is_some() || matches!(view.tag(), Some(T_INTS_ALIGNED) | Some(T_FLOATS_ALIGNED));
    if !deferrable {
        return None;
    }
    let (_, _, body) = unframe_with(bytes, false)?;
    let mut p = 0;
    read_str(body, &mut p)
}

pub fn message_from_wire_view(bytes: &[u8]) -> Option<(String, RuntimeValue)> {
    // Only a self-describing native record-list view is lazily wrappable.
    if view_message(bytes).and_then(|v| v.structs_schema()).is_some() {
        let (_, _, body) = unframe_with(bytes, false)?;
        let mut p = 0;
        let sender = read_str(body, &mut p)?; // the sender prefix at the body's head
        let lazy = crate::interpreter::ListRepr::from_record_list_view(Rc::new(bytes.to_vec()))?;
        return Some((sender, RuntimeValue::List(Rc::new(RefCell::new(lazy)))));
    }
    message_from_wire(bytes)
}

/// The leading marker of a batch STREAM message — distinct from any normal frame header (those use
/// only the low bits 0x01/0x02/0x10 + a 2-bit compression id) and from a FEC shard (0xFE), so the
/// drain loop tells the three apart by the first byte alone.
const STREAM_MAGIC: u8 = 0xFD;

/// Frame a sequence of values as one batch STREAM message: `[magic][sender][framed value-message]*`,
/// each value length-delimited (via [`crate::concurrency::stream::frame_for_stream`]) so the
/// receiver deframes them incrementally and reads each in place. ONE relay publish ships the whole
/// batch — Kafka-style streaming that amortizes per-message overhead — and `Await stream` reassembles
/// the list. Each value is encoded self-describingly so it round-trips without the type registry.
pub fn frame_stream_message(from: &str, values: &[RuntimeValue]) -> Result<Vec<u8>, String> {
    let mut out = vec![STREAM_MAGIC];
    write_str(from, &mut out);
    for v in values {
        let elem = message_to_wire("", v)?;
        crate::concurrency::stream::frame_for_stream(&elem, &mut out);
    }
    Ok(out)
}

/// Is `bytes` a batch stream message? If so, return its sender (so `Await stream … from <peer>`
/// matches it). `None` for a normal message / FEC shard / anything else.
pub fn peek_stream_sender(bytes: &[u8]) -> Option<String> {
    if bytes.first() != Some(&STREAM_MAGIC) {
        return None;
    }
    let mut p = 1;
    read_str(bytes, &mut p)
}

/// Deframe a batch stream message into its values, in order. `None` if `bytes` is not a stream
/// message; a frame that fails to decode is skipped (never a panic).
pub fn deframe_stream_message(bytes: &[u8]) -> Option<Vec<RuntimeValue>> {
    if bytes.first() != Some(&STREAM_MAGIC) {
        return None;
    }
    let mut p = 1;
    read_str(bytes, &mut p)?;
    let mut deframer = crate::concurrency::stream::StreamDeframer::new();
    deframer.push(bytes.get(p..)?);
    let mut values = Vec::new();
    deframer.drain_frames(|frame| {
        if let Some((_, v)) = message_from_wire(frame) {
            values.push(v);
        }
    });
    Some(values)
}

/// A borrowed view over one wire message's top-level value. Holds no owned data and never
/// decodes the rest of the message; reads are in place. Open it with [`view_message`].
#[derive(Clone, Copy)]
pub struct WireView<'a> {
    /// Slice starting at the top-level value's tag byte.
    val: &'a [u8],
}

/// Open a borrowed, zero-alloc view over `bytes`. `None` for a compressed or JSON message
/// (those must be inflated/decoded first — the view is over raw native bytes) or a
/// malformed frame. Reads any single field in place afterward.
pub fn view_message(bytes: &[u8]) -> Option<WireView<'_>> {
    // `verify = false`: opening a view is O(1) even on a checksummed message — validating
    // the FNV sum would re-hash the whole body, defeating zero-copy random access. The view
    // trusts the bytes (Cap'n Proto / Arrow have no checksum at all); callers wanting
    // integrity use a full decode, which validates.
    let (codec, compression, body) = unframe_with(bytes, false)?;
    if !matches!(codec, WireCodec::Native) || compression != WireCompression::None {
        return None;
    }
    let mut pos = 0;
    skip_str(body, &mut pos)?; // skip the sender prefix
    Some(WireView { val: body.get(pos..)? })
}

impl<'a> WireView<'a> {
    fn tag(&self) -> Option<u8> {
        self.val.first().copied()
    }

    /// The top-level value as an integer (`T_INT`).
    pub fn as_int(&self) -> Option<i64> {
        if self.tag()? != T_INT {
            return None;
        }
        let mut p = 1;
        Some(unzigzag(read_uvarint(self.val, &mut p)?))
    }

    /// The top-level value as a float (`T_FLOAT`).
    pub fn as_float(&self) -> Option<f64> {
        if self.tag()? != T_FLOAT {
            return None;
        }
        let b = self.val.get(1..9)?;
        Some(f64::from_le_bytes(b.try_into().ok()?))
    }

    /// Read ONE field of an offset-table struct view (`T_STRUCT_VIEW`): scan the small
    /// name table, then jump to the field's value via the offset table — WITHOUT parsing
    /// any other field, however large. The Cap'n Proto-class random-access read; returns
    /// a sub-view you read with `as_int`/`as_float`/etc. `None` if not a view or no field.
    pub fn struct_field(&self, name: &str) -> Option<WireView<'a>> {
        if self.tag()? != T_STRUCT_VIEW {
            return None;
        }
        let mut p = 1;
        skip_str(self.val, &mut p)?; // type_name
        let count = read_uvarint(self.val, &mut p)? as usize;
        let mut idx = None;
        for i in 0..count {
            let nlen = read_uvarint(self.val, &mut p)? as usize;
            let nbytes = self.val.get(p..p.checked_add(nlen)?)?;
            if nbytes == name.as_bytes() {
                idx = Some(i);
            }
            p += nlen;
        }
        let idx = idx?;
        let table_pos = p;
        let off_at = table_pos.checked_add(idx.checked_mul(4)?)?;
        let off_bytes = self.val.get(off_at..off_at.checked_add(4)?)?;
        let offset = u32::from_le_bytes(off_bytes.try_into().ok()?) as usize;
        let values_start = table_pos.checked_add(count.checked_mul(4)?)?;
        Some(WireView { val: self.val.get(values_start.checked_add(offset)?..)? })
    }

    /// Read an 8-byte-aligned i64 column (`T_INTS_ALIGNED`) as `&[i64]` with ZERO copy —
    /// the in-place column read (the kernel-bypass / RDMA path: no per-element decode, no
    /// `memcpy`). `None` if it is not an aligned column or the bytes are not 8-aligned in
    /// this buffer (then the caller decodes/copies instead, still one `memcpy`).
    pub fn as_i64_slice(&self) -> Option<&'a [i64]> {
        if self.tag()? != T_INTS_ALIGNED {
            return None;
        }
        let mut p = 1;
        let n = read_uvarint(self.val, &mut p)? as usize;
        let pad = *self.val.get(p)? as usize;
        p += 1 + pad;
        let nbytes = n.checked_mul(8)?;
        let blob = self.val.get(p..p.checked_add(nbytes)?)?;
        if blob.as_ptr() as usize % 8 != 0 {
            return None; // not 8-aligned in this buffer → caller copies
        }
        // SAFETY: `blob` is exactly `n*8` bytes, 8-byte aligned, borrowed for `'a`; every
        // bit pattern is a valid `i64`.
        Some(unsafe { std::slice::from_raw_parts(blob.as_ptr().cast::<i64>(), n) })
    }

    /// Read an 8-byte-aligned f64 column (`T_FLOATS_ALIGNED`) as `&[f64]` with ZERO copy —
    /// the float twin of [`as_i64_slice`](Self::as_i64_slice). `None` if it is not an
    /// aligned float column or the bytes are not 8-aligned in this buffer (caller copies).
    pub fn as_f64_slice(&self) -> Option<&'a [f64]> {
        if self.tag()? != T_FLOATS_ALIGNED {
            return None;
        }
        let mut p = 1;
        let n = read_uvarint(self.val, &mut p)? as usize;
        let pad = *self.val.get(p)? as usize;
        p += 1 + pad;
        let nbytes = n.checked_mul(8)?;
        let blob = self.val.get(p..p.checked_add(nbytes)?)?;
        if blob.as_ptr() as usize % 8 != 0 {
            return None; // not 8-aligned in this buffer → caller copies
        }
        // SAFETY: `blob` is exactly `n*8` bytes, 8-byte aligned, borrowed for `'a`; every
        // bit pattern is a valid `f64` (NaN/Inf/subnormal included — all read verbatim).
        Some(unsafe { std::slice::from_raw_parts(blob.as_ptr().cast::<f64>(), n) })
    }

    /// Read field `fi` of a COLUMNAR fixed struct list (`T_STRUCTS` whose every column is a
    /// `T_INTS_FIXED` blob) as its contiguous little-endian `i64` bytes — zero-copy, no per-cell
    /// navigation and no materialization. Because the layout is columnar, ALL of field `fi` is
    /// adjacent, so summing/scanning it is one CACHE-FRIENDLY contiguous pass — the columnar-
    /// analytics win over a row-major reader's strided walk (Cap'n Proto interleaves the fields).
    /// `None` if the value is not a columnar all-fixed struct list, or `fi` is out of range.
    pub fn structs_fixed_i64_col(&self, fi: usize) -> Option<&'a [u8]> {
        if self.tag()? != T_STRUCTS {
            return None;
        }
        let mut p = 1;
        skip_str(self.val, &mut p)?; // type_name
        let k = read_uvarint(self.val, &mut p)? as usize;
        if fi >= k {
            return None;
        }
        for _ in 0..k {
            skip_str(self.val, &mut p)?; // field names
        }
        let n = read_uvarint(self.val, &mut p)? as usize;
        for c in 0..=fi {
            if *self.val.get(p)? != T_INTS_FIXED {
                return None; // a non-fixed column — this fast path needs all columns fixed-width
            }
            p += 1;
            let cnt = read_uvarint(self.val, &mut p)? as usize;
            if cnt != n {
                return None;
            }
            let nbytes = n.checked_mul(8)?;
            let blob = self.val.get(p..p.checked_add(nbytes)?)?;
            if c == fi {
                return Some(blob);
            }
            p += nbytes;
        }
        None
    }

    /// Read a byte column (`T_BYTES`) as `&[u8]` with ZERO copy — binary data (hashes, file
    /// chunks, crypto) read in place: no decode, no allocation, no `i64` expansion, and
    /// (unlike the i64/f64 columns) no alignment requirement, since a `u8` slice is always
    /// 1-aligned. The first-class `bytes`/`Data` read that bit-packing can never offer.
    /// `None` if this is not a byte column.
    pub fn as_byte_slice(&self) -> Option<&'a [u8]> {
        if self.tag()? != T_BYTES {
            return None;
        }
        let mut p = 1;
        let n = read_uvarint(self.val, &mut p)? as usize;
        self.val.get(p..p.checked_add(n)?)
    }

    /// Row count of a record-list view (variable `T_STRUCTS_VIEW` or fixed `T_STRUCTS_FVIEW`),
    /// or `None` if not one.
    pub fn structs_len(&self) -> Option<usize> {
        let tag = self.tag()?;
        if tag != T_STRUCTS_VIEW && tag != T_STRUCTS_FVIEW {
            return None;
        }
        let mut p = 1;
        skip_str(self.val, &mut p)?; // type_name
        let f = read_uvarint(self.val, &mut p)? as usize;
        for _ in 0..f {
            let nlen = read_uvarint(self.val, &mut p)? as usize;
            p = p.checked_add(nlen)?;
        }
        if tag == T_STRUCTS_FVIEW {
            p = p.checked_add(f)?; // skip the F kind bytes
        }
        Some(read_uvarint(self.val, &mut p)? as usize)
    }

    /// Read field `name` of row `row` in a record-list view (`T_STRUCTS_VIEW`) in O(1):
    /// scan the shared name table once for the field index, jump via the row-offset table
    /// to the row block, then via that row's field-offset table to the value — NEVER parsing
    /// the other rows or fields, however large the list. The Cap'n Proto-class random access
    /// into a record list. Returns a sub-view (`as_int`/`as_float`/…). `None` if not a record
    /// view, the row is out of range, or no such field.
    pub fn structs_row_field(&self, row: usize, name: &str) -> Option<WireView<'a>> {
        if self.tag()? != T_STRUCTS_VIEW {
            return None;
        }
        let mut p = 1;
        skip_str(self.val, &mut p)?; // type_name
        let f = read_uvarint(self.val, &mut p)? as usize;
        let mut field_idx = None;
        for i in 0..f {
            let nlen = read_uvarint(self.val, &mut p)? as usize;
            let nbytes = self.val.get(p..p.checked_add(nlen)?)?;
            if nbytes == name.as_bytes() {
                field_idx = Some(i);
            }
            p += nlen;
        }
        let fi = field_idx?;
        let n = read_uvarint(self.val, &mut p)? as usize;
        if row >= n {
            return None;
        }
        let row_table_pos = p;
        let rows_start = row_table_pos.checked_add(n.checked_mul(4)?)?;
        let row_off_at = row_table_pos.checked_add(row.checked_mul(4)?)?;
        let row_off =
            u32::from_le_bytes(self.val.get(row_off_at..row_off_at.checked_add(4)?)?.try_into().ok()?) as usize;
        let field_table_pos = rows_start.checked_add(row_off)?;
        let values_start = field_table_pos.checked_add(f.checked_mul(4)?)?;
        let field_off_at = field_table_pos.checked_add(fi.checked_mul(4)?)?;
        let field_off =
            u32::from_le_bytes(self.val.get(field_off_at..field_off_at.checked_add(4)?)?.try_into().ok()?) as usize;
        Some(WireView { val: self.val.get(values_start.checked_add(field_off)?..)? })
    }

    /// Read field `name` of row `row` as an owned value for EITHER record-list view: the variable
    /// offset-table view (`T_STRUCTS_VIEW`) or the fixed-stride view (`T_STRUCTS_FVIEW`). Both are
    /// O(1) random access — the fixed view by pure arithmetic (no offset tables). Numeric/bool
    /// reads allocate nothing; a text read materializes the one string. The unified read the lazy
    /// `Await view` backing uses, so a peer's `Send indexed` (either layout) reads the same way.
    /// `None` if this is not a record view, the row is out of range, or there is no such field.
    pub fn structs_row_field_value(&self, row: usize, name: &str) -> Option<RuntimeValue> {
        match self.tag()? {
            T_STRUCTS_VIEW => self.structs_row_field(row, name)?.decode(),
            T_STRUCTS_FVIEW => {
                let mut p = 1;
                skip_str(self.val, &mut p)?; // type_name
                let f = read_uvarint(self.val, &mut p)? as usize;
                let mut field_idx = None;
                for i in 0..f {
                    let nlen = read_uvarint(self.val, &mut p)? as usize;
                    let nbytes = self.val.get(p..p.checked_add(nlen)?)?;
                    if nbytes == name.as_bytes() {
                        field_idx = Some(i);
                    }
                    p += nlen;
                }
                let fi = field_idx?;
                let kinds = self.val.get(p..p.checked_add(f)?)?;
                p += f;
                let n = read_uvarint(self.val, &mut p)? as usize;
                if row >= n {
                    return None;
                }
                let (offsets, stride) = fview_layout(kinds);
                let rows_start = p;
                let cell_pos = rows_start.checked_add(row.checked_mul(stride)?)?.checked_add(offsets[fi])?;
                // The string blob follows the fixed rows; its length varint sits right after them.
                let mut bp = rows_start.checked_add(n.checked_mul(stride)?)?;
                let blob_len = read_uvarint(self.val, &mut bp)? as usize;
                let blob = self.val.get(bp..bp.checked_add(blob_len)?)?;
                fview_read_cell(kinds[fi], self.val.get(cell_pos..)?, blob)
            }
            _ => None,
        }
    }

    /// Fully decode the ONE value this view points at (a cell / field / element) into an owned
    /// `RuntimeValue` — the materialize-on-touch step a lazy reader runs after locating a field in
    /// place. Decodes only this value, never the rest of the message; the bytes outside it stay
    /// untouched. (Uses the ambient type registry, so it round-trips name-elided cells too.)
    pub fn decode(&self) -> Option<RuntimeValue> {
        let mut p = 0;
        native_decode(self.val, &mut p)
    }

    /// The schema of a record-list view (`T_STRUCTS_VIEW`): `(type_name, field_names, row_count)`,
    /// read from the shared header WITHOUT decoding a single row — so a lazy backing can carry the
    /// schema + length while the row bytes stay un-decoded until a field is touched. `None` if this
    /// is not a record-list view.
    pub fn structs_schema(&self) -> Option<(String, Vec<String>, usize)> {
        let tag = self.tag()?;
        if tag != T_STRUCTS_VIEW && tag != T_STRUCTS_FVIEW {
            return None;
        }
        let mut p = 1;
        let type_name = read_str(self.val, &mut p)?;
        let f = read_uvarint(self.val, &mut p)? as usize;
        let mut field_names = Vec::with_capacity(f);
        for _ in 0..f {
            field_names.push(read_str(self.val, &mut p)?);
        }
        if tag == T_STRUCTS_FVIEW {
            p = p.checked_add(f)?; // skip the F kind bytes
        }
        let n = read_uvarint(self.val, &mut p)? as usize;
        Some((type_name, field_names, n))
    }

    /// The top-level value as a bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self.tag()? {
            T_TRUE => Some(true),
            T_FALSE => Some(false),
            _ => None,
        }
    }

    /// Element count of a homogeneous int list (`T_INTS` varint or `T_INTS_FIXED`).
    pub fn int_list_len(&self) -> Option<usize> {
        let mut p = 1;
        match self.tag()? {
            T_INTS_FIXED => Some(read_uvarint(self.val, &mut p)? as usize),
            T_INTS => Some((read_uvarint(self.val, &mut p)? >> 1) as usize),
            _ => None,
        }
    }

    /// Element `i` of an int list — O(1) + ZERO ALLOC for the fixed layout (seek to the
    /// byte offset, read 8 bytes); O(i) scan for the varint layout, still no full decode.
    pub fn int_list_get(&self, i: usize) -> Option<i64> {
        match self.tag()? {
            T_INTS_FIXED => {
                let mut p = 1;
                let n = read_uvarint(self.val, &mut p)? as usize;
                if i >= n {
                    return None;
                }
                let off = p + i * 8; // O(1): direct seek to element i
                let b = self.val.get(off..off + 8)?;
                Some(i64::from_le_bytes(b.try_into().ok()?))
            }
            T_INTS => {
                let mut p = 1;
                let header = read_uvarint(self.val, &mut p)?;
                let signed = header & 1 == 1;
                let n = (header >> 1) as usize;
                if i >= n {
                    return None;
                }
                for _ in 0..i {
                    read_uvarint(self.val, &mut p)?;
                }
                let u = read_uvarint(self.val, &mut p)?;
                Some(if signed { unzigzag(u) } else { u as i64 })
            }
            _ => None,
        }
    }

    /// Element count of a memcpy float list (`T_FLOATS`).
    pub fn float_list_len(&self) -> Option<usize> {
        if self.tag()? != T_FLOATS {
            return None;
        }
        let mut p = 1;
        Some(read_uvarint(self.val, &mut p)? as usize)
    }

    /// Element `i` of a memcpy float list — O(1), zero alloc.
    pub fn float_list_get(&self, i: usize) -> Option<f64> {
        if self.tag()? != T_FLOATS {
            return None;
        }
        let mut p = 1;
        let n = read_uvarint(self.val, &mut p)? as usize;
        if i >= n {
            return None;
        }
        let off = p + i * 8;
        let b = self.val.get(off..off + 8)?;
        Some(f64::from_le_bytes(b.try_into().ok()?))
    }

    /// Open a parse-ONCE bulk cursor over a record-list view (either layout). `None` if this is not
    /// a record-list view. Use it to read a WHOLE list as fast as Cap'n Proto's lazy reader:
    /// `structs_row_field_value` re-parses the header on every call (fine for one read, O(n·f) for a
    /// full scan), whereas the cursor parses the schema/tables once and every access is O(1).
    pub fn structs_cursor(&self) -> Option<WireStructsCursor<'a>> {
        let tag = self.tag()?;
        if tag != T_STRUCTS_VIEW && tag != T_STRUCTS_FVIEW {
            return None;
        }
        let val = self.val;
        let mut p = 1;
        let tn_len = read_uvarint(val, &mut p)? as usize; // type_name (skipped)
        p = p.checked_add(tn_len)?;
        let f = read_uvarint(val, &mut p)? as usize;
        let mut field_names = Vec::with_capacity(f);
        for _ in 0..f {
            let nlen = read_uvarint(val, &mut p)? as usize;
            field_names.push(val.get(p..p.checked_add(nlen)?)?);
            p += nlen;
        }
        if tag == T_STRUCTS_FVIEW {
            let field_kinds = val.get(p..p.checked_add(f)?)?;
            p += f;
            let n = read_uvarint(val, &mut p)? as usize;
            let (field_offsets, stride) = fview_layout(field_kinds);
            let rows_start = p;
            let mut bp = rows_start.checked_add(n.checked_mul(stride)?)?;
            let blob_len = read_uvarint(val, &mut bp)? as usize;
            let blob = val.get(bp..bp.checked_add(blob_len)?)?;
            Some(WireStructsCursor {
                val,
                field_names,
                n,
                kind: CursorKind::Fixed { field_kinds, field_offsets, stride, rows_start, blob },
            })
        } else {
            let n = read_uvarint(val, &mut p)? as usize;
            let row_table_pos = p;
            let rows_start = row_table_pos.checked_add(n.checked_mul(4)?)?;
            Some(WireStructsCursor { val, field_names, n, kind: CursorKind::Variable { row_table_pos, rows_start } })
        }
    }
}

/// A parse-once cursor over a record-list view (`T_STRUCTS_VIEW` / `T_STRUCTS_FVIEW`): the schema
/// and tables are read ONCE at open, then every `(row, field)` access is O(1) — pure arithmetic for
/// the fixed-stride view, a two-`u32` offset jump for the variable view — with no per-call re-scan.
pub struct WireStructsCursor<'a> {
    val: &'a [u8],
    field_names: Vec<&'a [u8]>,
    n: usize,
    kind: CursorKind<'a>,
}

enum CursorKind<'a> {
    Variable { row_table_pos: usize, rows_start: usize },
    Fixed { field_kinds: &'a [u8], field_offsets: Vec<usize>, stride: usize, rows_start: usize, blob: &'a [u8] },
}

impl<'a> WireStructsCursor<'a> {
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    pub fn field_count(&self) -> usize {
        self.field_names.len()
    }
    /// Index of the field named `name`, scanned once by the caller and then reused for every row.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.field_names.iter().position(|&n| n == name.as_bytes())
    }

    /// Byte slice at the start of cell `(row, fi)` — the one arithmetic/offset step both layouts share.
    fn cell_slice(&self, row: usize, fi: usize) -> Option<&'a [u8]> {
        if row >= self.n || fi >= self.field_names.len() {
            return None;
        }
        match &self.kind {
            CursorKind::Fixed { field_offsets, stride, rows_start, .. } => {
                let pos = rows_start.checked_add(row.checked_mul(*stride)?)?.checked_add(field_offsets[fi])?;
                self.val.get(pos..)
            }
            CursorKind::Variable { row_table_pos, rows_start } => {
                let f = self.field_names.len();
                let row_off_at = row_table_pos.checked_add(row.checked_mul(4)?)?;
                let row_off =
                    u32::from_le_bytes(self.val.get(row_off_at..row_off_at.checked_add(4)?)?.try_into().ok()?) as usize;
                let field_table_pos = rows_start.checked_add(row_off)?;
                let values_start = field_table_pos.checked_add(f.checked_mul(4)?)?;
                let field_off_at = field_table_pos.checked_add(fi.checked_mul(4)?)?;
                let field_off =
                    u32::from_le_bytes(self.val.get(field_off_at..field_off_at.checked_add(4)?)?.try_into().ok()?) as usize;
                self.val.get(values_start.checked_add(field_off)?..)
            }
        }
    }

    /// The `(row, field)` value as an owned `RuntimeValue` — O(1), no header re-scan.
    pub fn value(&self, row: usize, fi: usize) -> Option<RuntimeValue> {
        match &self.kind {
            CursorKind::Fixed { field_kinds, blob, .. } => {
                fview_read_cell(*field_kinds.get(fi)?, self.cell_slice(row, fi)?, blob)
            }
            CursorKind::Variable { .. } => {
                let cell = self.cell_slice(row, fi)?;
                let mut q = 0;
                native_decode(cell, &mut q)
            }
        }
    }

    /// Read an ENTIRE int field of the fixed-stride view as a `Vec<i64>` in one tight pass — the
    /// Cap'n-Proto-class read-all: the cells are at a fixed `offset` every `stride` bytes, so after
    /// one bounds check on the last cell, the reads are unchecked raw 8-byte loads (no per-read
    /// bounds check, no slice indirection, no `RuntimeValue` box). `None` for the variable view
    /// (varint cells aren't strided) or a non-int field.
    pub fn i64_column(&self, fi: usize) -> Option<Vec<i64>> {
        let CursorKind::Fixed { field_kinds, field_offsets, stride, rows_start, .. } = &self.kind else {
            return None;
        };
        if *field_kinds.get(fi)? != FK_INT {
            return None;
        }
        let base = rows_start.checked_add(field_offsets[fi])?;
        let mut out = Vec::with_capacity(self.n);
        if self.n > 0 {
            // The extreme cell `base + (n-1)*stride .. +8` ⊆ val; every earlier cell is below it,
            // so this single check makes every loop read in-bounds.
            let last = base.checked_add((self.n - 1).checked_mul(*stride)?)?;
            self.val.get(last..last.checked_add(8)?)?;
            let ptr = self.val.as_ptr();
            for r in 0..self.n {
                let p = base + r * stride;
                // SAFETY: `p..p+8` ⊆ `[rows_start, rows_start + n*stride)` ⊆ `val` (checked above).
                let b = unsafe { std::slice::from_raw_parts(ptr.add(p), 8) };
                out.push(i64::from_le_bytes(b.try_into().unwrap()));
            }
        }
        Some(out)
    }

    /// Fast integer read of an int cell — the Cap'n-Proto-class random read: pure arithmetic + a
    /// raw 8-byte read (fixed view) or a tagged-varint decode (variable view), NO `RuntimeValue` box.
    /// `None` if the cell is not an integer.
    pub fn i64(&self, row: usize, fi: usize) -> Option<i64> {
        let cell = self.cell_slice(row, fi)?;
        match &self.kind {
            CursorKind::Fixed { field_kinds, .. } => match *field_kinds.get(fi)? {
                FK_INT => Some(i64::from_le_bytes(cell.get(0..8)?.try_into().ok()?)),
                _ => None,
            },
            CursorKind::Variable { .. } => {
                if *cell.first()? != T_INT {
                    return None;
                }
                let mut q = 1;
                Some(unzigzag(read_uvarint(cell, &mut q)?))
            }
        }
    }
}

const T_NOTHING: u8 = 0;
const T_FALSE: u8 = 1;
const T_TRUE: u8 = 2;
const T_INT: u8 = 3;
const T_FLOAT: u8 = 4;
const T_CHAR: u8 = 5;
const T_TEXT: u8 = 6;
const T_DURATION: u8 = 7;
const T_DATE: u8 = 8;
const T_MOMENT: u8 = 9;
const T_SPAN: u8 = 10;
const T_TIME: u8 = 11;
const T_PEER: u8 = 12;
const T_LIST: u8 = 13;
const T_TUPLE: u8 = 14;
const T_SET: u8 = 15;
const T_MAP: u8 = 16;
const T_STRUCT: u8 = 17;
const T_INDUCTIVE: u8 = 18;
// Packed homogeneous lists — one tag + count, NO per-element tag, encoded
// straight from the specialized `ListRepr` storage. The throughput path.
const T_INTS: u8 = 19; // zig-zag varint per element (covers Ints + IntsI32)
const T_FLOATS: u8 = 20; // 8-byte little-endian per element
const T_BOOLS: u8 = 21; // bit-packed, 8 booleans per byte
const T_STRINGS: u8 = 22; // flat string array: count + per-elem byte-lengths + one bytes blob
const T_INTS_FIXED: u8 = 23; // fixed-width i64 array: count + raw 8-byte-LE blob (memcpy)
const T_INTS_GV: u8 = 24; // group-varint (Stream VByte layout): control stream + data stream
// Columnar packing for homogeneous lists of compound values: the schema is written
// ONCE, then each field becomes its own packed column (reusing the array tags above).
const T_STRUCTS: u8 = 25; // homogeneous struct list: type_name + field names + one column per field
const T_INDUCTIVES: u8 = 26; // homogeneous enum list: type_name + ctor dictionary + index + arg columns
// Schema-dictionary forms of a struct list (cross-message, connection-scoped cache):
const T_STRUCTS_DEF: u8 = 27; // sequential: defines schema at `id` inline, then columns (self-decodable)
const T_STRUCTS_REF: u8 = 28; // sequential: references a previously-defined `id`, then columns
const T_STRUCTS_CDEF: u8 = 29; // content-addressed: schema inline (fingerprint derived), then columns
const T_STRUCTS_CREF: u8 = 30; // content-addressed: 8-byte schema fingerprint, then columns
const T_FLOATS_XOR: u8 = 31; // lossless XOR-delta + varint float array (Gorilla-style)
const T_INTS_AFFINE: u8 = 32; // closed-form: base + stride*i for all i (3 numbers, no data)
const T_BIGINT: u8 = 33; // exact out-of-i64 integer: sign byte + length + little-endian magnitude
const T_RATIONAL: u8 = 34; // exact fraction: signed numerator (sign+len+LE) then positive denominator (len+LE)
const T_DECIMAL: u8 = 75; // exact base-10 fixed-point (money): sign + coefficient magnitude (len+LE) + base-10 scale (uvarint)
const T_COMPLEX: u8 = 76; // exact complex re+im·i: two rationals back to back, each as sign + numerator (len+LE) + denominator (len+LE)
const T_MODULAR: u8 = 77; // ℤ/nℤ element: residue magnitude (len+LE) then modulus magnitude (len+LE), both non-negative
const T_QUANTITY: u8 = 79; // dimensioned quantity: SI magnitude (sign + num len+LE + den len+LE), 10 exponent (num,den) zigzag-varint pairs, then the unit symbol (len+UTF-8)
const T_MONEY: u8 = 80; // money: amount as Decimal (sign + coefficient len+LE + base-10 scale uvarint) then the ISO-4217 currency code (len + UTF-8)
const T_UUID: u8 = 81; // uuid: 16 big-endian bytes verbatim (fixed width, no length prefix)
// Schema-dictionary forms of a SINGLE struct (cross-message, connection-scoped cache),
// the lone-struct analog of the T_STRUCTS_* list forms: once both peers know a schema,
// a struct message ships its values in canonical field order with NO inline field-name
// strings — closing the postcard gap (a lone struct otherwise pays for "x","y",… every
// send). The DEF/CDEF forms are self-decodable (schema inline); REF/CREF carry values only.
const T_STRUCT_DEF: u8 = 35; // sequential: id + schema inline (registered), then values in field order
const T_STRUCT_REF: u8 = 36; // sequential: id resolved against the cache, then values
const T_STRUCT_CDEF: u8 = 37; // content-addressed: schema inline (fingerprint derived), then values
const T_STRUCT_CREF: u8 = 38; // content-addressed: 8-byte schema fingerprint, then values
// The per-column compression menu (WireStructure::Auto picks the smallest of these +
// the varint/affine baselines). Each is a categorical win on one data shape; the
// selector always includes the plain varint, so the chosen form is never larger.
const T_INTS_DELTA: u8 = 39; // count + first(zz) + (n-1) zig-zag deltas — monotone columns
const T_INTS_DOD: u8 = 40; // count + first(zz) + d1(zz) + (n-2) zig-zag delta-of-deltas — near-linear (timestamps)
const T_INTS_FOR: u8 = 41; // count + min(zz) + bit-width + bit-packed (v-min) residuals — clustered ints
const T_INTS_RLE: u8 = 42; // run count + (value(zz), run-length) pairs — runs of repeats
const T_INTS_DICT: u8 = 43; // dict size + distinct values(zz) + count + index-width + bit-packed indices — low cardinality
const T_INTS_POLY: u8 = 50; // degree + count + (degree+1) finite-difference seeds(zz) — SHIP THE GENERATOR for a polynomial column
const T_GEN: u8 = 51; // serialized GenExpr + count — a sandboxed pure generator over the index `i` (the general compute-shipping form)
const T_FUNC: u8 = 52; // arity + serialized GenExpr — a SHIPPED CALLABLE pure function (the receiver evaluates it in the sandbox)
const T_BYTES: u8 = 53; // count + raw 1-byte-per-element blob — a byte column; memcpy in/out and readable in place as &[u8] (zero-copy, no alignment)
const T_STRUCTS_TID: u8 = 54; // shared-registry struct LIST: type-id(varint) + N + columns — type/field NAMES elided (the struct-list analog of T_STRUCT_TID)
const T_SET_INTS: u8 = 55; // homogeneous int SET: the SORTED-canonical members shipped through the G5 int-column menu (delta/affine/RLE) — a consecutive set {1..n} collapses to base+stride+count, no data
const T_STRUCTS_FVIEW: u8 = 56; // FIXED-stride record-list view: type + F + names + F kind-bytes + N + [n×stride fixed rows] + blob_len + string blob. Random access = pure arithmetic (no offset tables): the `indexed fast` form — composes the struct-view with the fixed numeric dial.
const T_SET_STRINGS: u8 = 57; // homogeneous string SET, FRONT-CODED: members sorted to canonical order, each shipped as (shared-prefix-len-with-previous, suffix) — sorted similar strings share long prefixes so only the deltas go on the wire
const T_MAP_INTKEY: u8 = 58; // INT-KEYED map, COLUMNAR: entries sorted by numeric key → keys as a best int column (G5 menu: affine/delta/RLE) + values as a best-encoded list (reuses the full column menu). An affine int→int map {i↦2i} collapses BOTH columns to closed forms — ~no data. Canonical (insertion-order-invariant).
const T_STRINGS_DICT: u8 = 59; // DICTIONARY string column (low cardinality / categorical labels): dict-len + distinct strings (len+bytes) once + count + index-width + bit-packed per-row indices. The string twin of T_INTS_DICT — a handful of distinct labels repeated N times ships the labels once.

// Fixed-view field kinds (1 byte each in the schema): the wire width of a cell.
const FK_INT: u8 = 0; // 8 bytes, raw i64 little-endian (NOT zig-zag — memcpy, like T_INTS_FIXED)
const FK_FLOAT: u8 = 1; // 8 bytes, f64 little-endian
const FK_BOOL: u8 = 2; // 1 byte (0/1)
const FK_TEXT: u8 = 3; // 8 bytes: u32 offset + u32 length, into the trailing string blob

/// The byte width of one fixed-view cell of `kind`.
fn fview_width(kind: u8) -> usize {
    match kind {
        FK_BOOL => 1,
        _ => 8, // FK_INT / FK_FLOAT / FK_TEXT(ref)
    }
}

/// Per-field byte offsets within a fixed-view row, and the row stride (their sum).
fn fview_layout(kinds: &[u8]) -> (Vec<usize>, usize) {
    let mut offsets = Vec::with_capacity(kinds.len());
    let mut cur = 0usize;
    for &k in kinds {
        offsets.push(cur);
        cur += fview_width(k);
    }
    (offsets, cur)
}

/// The fixed-view kind for each column, or `None` if any column is not a fixed-width-encodable
/// leaf (Int/Float/Bool/Text) — then the caller keeps the variable offset-table view.
fn columns_fview_kinds(columns: &[ListRepr]) -> Option<Vec<u8>> {
    let mut kinds = Vec::with_capacity(columns.len());
    for col in columns {
        kinds.push(match col {
            ListRepr::Ints(_) | ListRepr::IntsI32(_) => FK_INT,
            ListRepr::Floats(_) => FK_FLOAT,
            ListRepr::Bools(_) => FK_BOOL,
            ListRepr::Strings { .. } => FK_TEXT,
            _ => return None,
        });
    }
    Some(kinds)
}

/// Read one fixed-view cell (`kind` bytes at `cell`) into an owned value; `blob` backs FK_TEXT.
fn fview_read_cell(kind: u8, cell: &[u8], blob: &[u8]) -> Option<RuntimeValue> {
    match kind {
        FK_INT => Some(RuntimeValue::Int(i64::from_le_bytes(cell.get(0..8)?.try_into().ok()?))),
        FK_FLOAT => Some(RuntimeValue::Float(f64::from_le_bytes(cell.get(0..8)?.try_into().ok()?))),
        FK_BOOL => Some(RuntimeValue::Bool(*cell.first()? != 0)),
        FK_TEXT => {
            let off = u32::from_le_bytes(cell.get(0..4)?.try_into().ok()?) as usize;
            let len = u32::from_le_bytes(cell.get(4..8)?.try_into().ok()?) as usize;
            let s = blob.get(off..off.checked_add(len)?)?;
            Some(RuntimeValue::Text(Rc::new(String::from_utf8(s.to_vec()).ok()?)))
        }
        _ => None,
    }
}
// Type-id elided struct: when both ends share a program type registry, ship the type's
// small registry id + the values only — type/field NAMES never go on the wire (the
// Logos↔Logos default that beats raw varint). Falls back to T_STRUCT when unknown.
const T_STRUCT_TID: u8 = 44; // registry-id(varint) + values in canonical field order
const T_WORD: u8 = 60; // fixed-width wrapping int: width byte (32|64) + uvarint value (zero-extended to u64)
const T_INTS_GEOMETRIC: u8 = 61; // closed-form: base * ratio^i for all i (3 numbers, no data) — wrapping-exact
const T_INTS_PERIODIC: u8 = 62; // cyclic: period p + count + one block of p values → pattern[i % p]
const T_SHARED_DEF: u8 = 63; // dedup: id(uvarint) + the value — registers a shared subtree at `id`
const T_SHARED_REF: u8 = 64; // dedup: id(uvarint) — a backref to an already-shipped shared subtree
const T_FLOATS_CONST: u8 = 65; // closed-form: one f64 (8 LE bytes) + count — every element identical
const T_FLOATS_AFFINE: u8 = 66; // closed-form: base + i·stride (2 f64 + count), BIT-EXACT or not used
const T_INTS_SPARSE: u8 = 67; // dominant value + count + (delta-index, value) exceptions — sparse/default columns
const T_FLOATS_SPARSE: u8 = 68; // dominant f64 + count + (delta-index, f64) exceptions — sparse float columns
const T_FLOATS_PERIODIC: u8 = 69; // cyclic: period p + count + one block of p f64 → pattern[i % p]
const T_FLOATS_GEOMETRIC: u8 = 70; // closed-form: base * ratio^i (2 f64 + count), BIT-EXACT or not used
const T_STRINGS_TEMPLATE: u8 = 71; // templated: prefix + suffix + affine(base,stride) + count → prefix+(base+i·stride)+suffix
const T_STRINGS_FRONT: u8 = 72; // front-coded COLUMN (order-preserving): each string = (shared-prefix-len-with-previous, suffix); sorted/hierarchical columns share long prefixes
const T_BOOLS_PERIODIC: u8 = 73; // cyclic bool column: period p + count + one p-bit block → block[i % p] (covers const all-true/all-false at p=1, alternating at p=2)
const T_BOOLS_RLE: u8 = 74; // run-length bool column: first value + run lengths (alternating) → big runs ([F×n, T×m] / clustered flags) collapse to a few varints
const T_STRINGS_AFFIX: u8 = 78; // common prefix + common suffix + per-row ARBITRARY middle → emails (…@host), extensions (….log), wrapped ids; the non-affine sibling of T_STRINGS_TEMPLATE
// Type-id elided enum: ship the enum's registry id + the constructor INDEX (into the
// type's ordered constructor list) + the args — type and constructor names elided.
const T_INDUCTIVE_TID: u8 = 45; // enum-id(varint) + ctor-index(varint) + arg-count + args
// Offset-table struct view (the Cap'n Proto-beating random-access layout): a per-field
// byte-offset table precedes the values, so a `WireView` jumps to ANY field in O(1)
// without parsing the others (even a huge preceding field). Decodes normally too.
const T_STRUCT_VIEW: u8 = 46; // type_name + count + names + [u32 offset]×count + values
// 8-byte-aligned i64 column: `count + pad-len + pad + raw i64 LE blob`, padded so the blob
// lands on an 8-byte boundary in the final framed buffer (header len ≡ 1 mod 8, so the
// body offset is aligned to ≡ 7 mod 8). A `WireView` reads it as `&[i64]` with ZERO copy
// (`as_i64_slice`) — the in-place column read, for kernel-bypass / RDMA on a LAN.
const T_INTS_ALIGNED: u8 = 47; // count + pad-len(1) + pad + i64 LE blob (8-byte aligned blob)
// The float twin of `T_INTS_ALIGNED`: an 8-byte-aligned `f64` blob a `WireView` reads as
// `&[f64]` with ZERO copy (`as_f64_slice`). Same padding discipline → the blob is 8-aligned
// in the framed buffer, so the cast is sound on every architecture (the float column axis).
const T_FLOATS_ALIGNED: u8 = 48; // count + pad-len(1) + pad + f64 LE blob (8-byte aligned blob)
// A record-LIST view: the shared schema once, a per-ROW offset table, then each row's own
// per-FIELD offset table + values. `WireView::structs_row_field(row, name)` jumps to ANY
// (row, field) in O(1) — Cap'n Proto-class random access into a huge struct list, without
// parsing the other rows or fields. The list analog of `T_STRUCT_VIEW`.
const T_STRUCTS_VIEW: u8 = 49; // type + F + names + N + [u32 row_off]×N + per-row([u32 field_off]×F + values)

/// How integer arrays are laid out on the wire — the sender's size↔speed dial.
/// The *decoder* always handles every variant (each has its own tag), so this is
/// purely a sender preference; mix freely on one relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireNumerics {
    /// LEB128 varint — smallest, and the best *scalar* decode. The default; the
    /// right choice for a network link (bytes are the bottleneck).
    Varint,
    /// Raw fixed-width `i64` — a `memcpy` both ways (float speed) at 4× the size.
    /// For a CPU-bound / bandwidth-rich link (datacenter, shared memory, RDMA).
    Fixed,
    /// Group-varint (Stream VByte layout) — varint-class size with the widths
    /// hoisted into a control stream, so a SIMD shuffle decodes it several ints at
    /// a time. The "small AND fast" middle ground on a SIMD-capable host.
    GroupVarint,
}

/// How float arrays are encoded. `Memcpy` is the raw 8-byte-per-element blob (the
/// memory-bandwidth ceiling, the default). `XorDelta` XORs each value's bits with the
/// previous and varint-codes the result — lossless and bit-exact (it operates on raw
/// bits, so NaN/Inf/±0/subnormals are preserved), and far smaller for slowly-varying
/// (time-series) columns. It is applied per-column only when it actually shrinks the
/// column, so it never grows a message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WireFloats {
    Memcpy,
    XorDelta,
}

/// Whether the encoder may replace a column with a *closed-form generator* when the
/// data is mathematically structured — the Futamura move on the wire: if the values
/// are described by a formula, ship the formula, not the values. Every form is
/// lossless and gated by an exact-match proof, so it can never change the decoded
/// value; the decoder always reconstructs (each form has its own tag). `Off` by
/// default (detection is an O(n) scan the speed dials skip), opt-in per send.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WireStructure {
    /// No structural analysis — integer columns encode by the numeric dial.
    Off,
    /// Detect an affine progression `v[i] = base + i·stride`; when *every* element
    /// matches exactly (wrapping i64), send `(base, stride, n)` — three numbers for
    /// the whole column — instead of the data. Falls back to the numeric dial when
    /// the data is not affine, so it never grows a message or loses a value.
    Affine,
    /// The full per-column compression menu: build every applicable encoding
    /// (varint baseline · affine · delta · delta-of-delta · frame-of-reference
    /// bit-packing · run-length · dictionary) and ship the SMALLEST. The varint
    /// baseline is always a candidate, so the result is never larger than `Off`'s
    /// varint — each encoding is a categorical win on its shape (monotone,
    /// near-linear timestamps, clustered, runs, low-cardinality) and silently loses
    /// the bake-off otherwise. The "smallest" knob.
    Auto,
}

thread_local! {
    static NUMERICS: std::cell::Cell<WireNumerics> = const { std::cell::Cell::new(WireNumerics::Varint) };
    static FLOATS: std::cell::Cell<WireFloats> = const { std::cell::Cell::new(WireFloats::Memcpy) };
    static STRUCTURE: std::cell::Cell<WireStructure> = const { std::cell::Cell::new(WireStructure::Off) };
    /// When on, EVERY list encodes as the plain, self-describing `T_LIST` (count + per-element
    /// tagged values) — no columnar string/struct/inductive packing. This is the flat, fastest-to-
    /// decode form the shared [`logicaffeine_data::wire`] core reads. Off by default so the peer
    /// codec's columnar wins are untouched; opt-in via [`with_flat_lists`] (used by [`encode_value_raw`]).
    static FLAT_LISTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with flat (columnar-free) list encoding forced on/off, restoring the prior value.
pub fn with_flat_lists<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let prev = FLAT_LISTS.with(|c| c.replace(on));
    let out = f();
    FLAT_LISTS.with(|c| c.set(prev));
    out
}

#[inline]
fn flat_lists() -> bool {
    FLAT_LISTS.with(|c| c.get())
}

/// Encode integer arrays under `n` for the duration of `f`. Scoped — never leaks.
pub fn with_numerics<T>(n: WireNumerics, f: impl FnOnce() -> T) -> T {
    let prev = NUMERICS.with(|c| c.replace(n));
    let out = f();
    NUMERICS.with(|c| c.set(prev));
    out
}

/// Convenience for [`WireNumerics::Fixed`] (back-compat).
pub fn with_fixed_numerics<T>(f: impl FnOnce() -> T) -> T {
    with_numerics(WireNumerics::Fixed, f)
}

fn numerics() -> WireNumerics {
    NUMERICS.with(std::cell::Cell::get)
}

/// Enable structural (closed-form) integer encoding under `s` for the duration of
/// `f`. Scoped — never leaks. See [`WireStructure`].
pub fn with_structure<T>(s: WireStructure, f: impl FnOnce() -> T) -> T {
    let prev = STRUCTURE.with(|c| c.replace(s));
    let out = f();
    STRUCTURE.with(|c| c.set(prev));
    out
}

fn structure() -> WireStructure {
    STRUCTURE.with(std::cell::Cell::get)
}

/// The longest repeating block the periodic detector will consider. A period beyond this is an
/// unusual "pattern" whose block barely beats storing the data, and it bounds the search cost.
const PERIOD_CAP: usize = 512;

/// Lower a pure single-parameter arithmetic expression into a [`GenExpr`] over the index —
/// the bridge that lets a user's pure function be SHIPPED as the sandboxed generator (it
/// becomes data the receiver evaluates, never code it runs). Returns `None` for anything
/// outside the provably-total arithmetic subset (calls, indexing, unknown variables, non-
/// integer literals, comparison/logical/bitwise ops), so only a safe function is shippable.
pub(crate) fn lower_expr_to_genexpr(e: &crate::ast::stmt::Expr<'_>, param: logicaffeine_base::Symbol) -> Option<GenExpr> {
    use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
    match e {
        Expr::Literal(Literal::Number(n)) => Some(GenExpr::Const(*n)),
        Expr::Identifier(s) if *s == param => Some(GenExpr::Index),
        Expr::BinaryOp { op, left, right } => {
            let l = Box::new(lower_expr_to_genexpr(left, param)?);
            let r = Box::new(lower_expr_to_genexpr(right, param)?);
            Some(match op {
                BinaryOpKind::Add => GenExpr::Add(l, r),
                BinaryOpKind::Subtract => GenExpr::Sub(l, r),
                BinaryOpKind::Multiply => GenExpr::Mul(l, r),
                BinaryOpKind::Divide => GenExpr::Div(l, r),
                BinaryOpKind::Modulo => GenExpr::Mod(l, r),
                _ => return None,
            })
        }
        _ => None,
    }
}

/// Pack `vals` LSB-first at `width` bits each (1..=64). The inverse of [`bitunpack`].
fn bitpack(vals: &[u64], width: u8) -> Vec<u8> {
    if width == 0 {
        return Vec::new();
    }
    let total_bits = vals.len().saturating_mul(width as usize);
    let mut out = vec![0u8; total_bits.div_ceil(8)];
    let mut bitpos = 0usize;
    for &val in vals {
        let mut bits = val;
        let mut remaining = width as usize;
        while remaining > 0 {
            let byte = bitpos / 8;
            let off = bitpos % 8;
            let take = remaining.min(8 - off);
            let mask = (1u64 << take) - 1;
            out[byte] |= ((bits & mask) as u8) << off;
            bits >>= take;
            bitpos += take;
            remaining -= take;
        }
    }
    out
}

/// Read `count` LSB-first `width`-bit values from `bytes`. `None` if `bytes` is too
/// short (clean failure on a corrupt length). The inverse of [`bitpack`].
fn bitunpack(bytes: &[u8], count: usize, width: u8) -> Option<Vec<u64>> {
    if width == 0 || width > 64 {
        return None;
    }
    let total_bits = count.checked_mul(width as usize)?;
    if bytes.len() < total_bits.div_ceil(8) {
        return None;
    }
    let mut out = Vec::with_capacity(count.min(PREALLOC_CAP));
    let mut bitpos = 0usize;
    for _ in 0..count {
        let mut val = 0u64;
        let mut got = 0usize;
        while got < width as usize {
            let byte = bitpos / 8;
            let off = bitpos % 8;
            let take = (width as usize - got).min(8 - off);
            let mask = (1u64 << take) - 1;
            val |= (((bytes[byte] >> off) as u64) & mask) << got;
            got += take;
            bitpos += take;
        }
        out.push(val);
    }
    Some(out)
}

/// Encode float arrays under `mode` for the duration of `f`. Scoped — never leaks.
pub fn with_floats<T>(mode: WireFloats, f: impl FnOnce() -> T) -> T {
    let prev = FLOATS.with(|c| c.replace(mode));
    let out = f();
    FLOATS.with(|c| c.set(prev));
    out
}

fn floats_mode() -> WireFloats {
    FLOATS.with(std::cell::Cell::get)
}

/// If every element is BIT-IDENTICAL (compared by `to_bits`, so `-0.0`/`+0.0`/`NaN` are exact, not
/// `==`), return its bit pattern — the column ships as one f64 + a count (constant readings, padding,
/// defaults). `None` for an empty or non-constant column.
fn detect_float_const(v: &[f64]) -> Option<u64> {
    let bits = v.first()?.to_bits();
    v.iter().all(|x| x.to_bits() == bits).then_some(bits)
}

/// If `v` is BIT-EXACTLY the closed form `base + i·stride` evaluated the SAME way the decoder will,
/// return `(base, stride)` — so it ships THREE numbers, not `n`. The bit-exact check (never `==`) means
/// it fires only when reconstruction is perfect: integer-valued float columns (ids/indices from JSON,
/// `0.0,1.0,2.0,…`), power-of-two-stride axes, exact linspace. Real noisy float data simply isn't
/// recognized and falls through to XOR-delta / memcpy — so the generator is a pure, lossless win.
fn detect_float_affine(v: &[f64]) -> Option<(f64, f64)> {
    if v.len() < 3 {
        return None;
    }
    let base = v[0];
    let stride = v[1] - v[0];
    for (i, &x) in v.iter().enumerate() {
        if (base + (i as f64) * stride).to_bits() != x.to_bits() {
            return None;
        }
    }
    Some((base, stride))
}

/// The float twin of [`detect_sparse`]: if ONE f64 (by bit pattern) dominates ≥ ¾ of the column,
/// return its bits and the sorted `(index, value)` exceptions — a mostly-default/constant-with-
/// outliers float column (sparse telemetry, a mostly-zero signal). Boyer–Moore over `to_bits`, so a
/// column with no dominant value pays only the O(1)-memory pass.
fn detect_float_sparse(v: &[f64]) -> Option<(u64, Vec<(usize, u64)>)> {
    if v.len() < 8 {
        return None;
    }
    let mut cand = v[0].to_bits();
    let mut count: i64 = 0;
    for x in v {
        let b = x.to_bits();
        if count == 0 {
            cand = b;
            count = 1;
        } else if b == cand {
            count += 1;
        } else {
            count -= 1;
        }
    }
    let occ = v.iter().filter(|x| x.to_bits() == cand).count();
    if v.len() - occ > v.len() / 4 {
        return None;
    }
    let exceptions: Vec<(usize, u64)> = v
        .iter()
        .enumerate()
        .filter(|(_, x)| x.to_bits() != cand)
        .map(|(i, x)| (i, x.to_bits()))
        .collect();
    Some((cand, exceptions))
}

/// The float twin of [`detect_period`]: the minimal period `2 ≤ p ≤ min(len/2, PERIOD_CAP)` such that
/// `v[i]` BIT-equals `v[i % p]` (a cyclic waveform / repeated frame). Pure bit-equality, so always
/// exact — ship one block of `p` f64 + the count, not all `n`.
fn detect_float_period(v: &[f64]) -> Option<usize> {
    let n = v.len();
    if n < 4 {
        return None;
    }
    let cap = (n / 2).min(PERIOD_CAP);
    'p: for p in 2..=cap {
        for i in p..n {
            if v[i].to_bits() != v[i - p].to_bits() {
                continue 'p;
            }
        }
        return Some(p);
    }
    None
}

/// The float twin of [`detect_geometric`]: if `v` is BIT-EXACTLY `base · ratio^i` replayed by the same
/// `cur *= ratio` accumulation the decoder uses, return `(base, ratio)`. Float multiply rounds, so this
/// fires only when reconstruction is perfect — power-of-two ratios (doubling, halving / exponential
/// decay), which ARE exact in f64. Everything else falls through, so it is a pure lossless win.
fn detect_float_geometric(v: &[f64]) -> Option<(f64, f64)> {
    if v.len() < 3 {
        return None;
    }
    let base = v[0];
    if base == 0.0 || !base.is_finite() {
        return None;
    }
    let ratio = v[1] / base;
    if !ratio.is_finite() || ratio == 1.0 {
        return None; // ratio 1 → constant, handled by `detect_float_const`
    }
    let mut cur = base;
    for &x in v {
        if cur.to_bits() != x.to_bits() {
            return None;
        }
        cur *= ratio;
    }
    Some((base, ratio))
}

/// XOR-delta encode a float column: count, then the LEB128 varint of each value's
/// bits XOR the previous value's bits. Lossless and bit-exact (raw-bit operation).
fn floats_xor_encode(out: &mut Vec<u8>, v: &[f64]) {
    write_uvarint(v.len() as u64, out);
    let mut prev = 0u64;
    for &f in v {
        let bits = f.to_bits();
        write_uvarint(bits ^ prev, out);
        prev = bits;
    }
}

/// Bytes a `write_uvarint` of `x` occupies (LEB128, 1–10 bytes).
fn uvarint_byte_len(x: u64) -> usize {
    (((64 - x.leading_zeros()).max(1) + 6) / 7) as usize
}

/// The body size of the memcpy float encoding (`T_FLOATS`): the count varint + 8
/// bytes per element. Used to keep the XOR-delta column ONLY when it actually shrinks.
fn floats_memcpy_body_len(n: usize) -> usize {
    uvarint_byte_len(n as u64) + n * 8
}

/// The compression codec for an encoded body — the sender's dial. The wire is
/// self-describing (the header carries the codec), so this is purely a sender
/// preference; any peer decodes any codec. Each is kept only if it actually shrank
/// the body (see [`message_to_wire_with`]), so compression never hurts the fast path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WireCompression {
    /// No compression. The default — our binary is already compact, so compression
    /// is for large/redundant payloads on size-constrained links.
    None,
    /// DEFLATE (`miniz_oxide`). The balanced middle; what `Send compressed` selects.
    Deflate,
    /// LZ4 (`lz4_flex`, pure-Rust) — near-memcpy speed, lighter ratio. Ships on every
    /// target (native + browser).
    Lz4,
    /// Zstandard — the best ratio. Native uses the C encoder; the browser cannot
    /// encode it (falls back to lz4) but decodes it via the pure-Rust `ruzstd`.
    Zstd,
}

/// The 2-bit on-wire id for a codec (header bits 2-3). `None`/`Deflate` share id 0;
/// `None` is distinguished by `H_COMPRESSED` being unset.
fn compression_id(c: WireCompression) -> u8 {
    match c {
        WireCompression::None | WireCompression::Deflate => 0,
        WireCompression::Lz4 => 1,
        WireCompression::Zstd => 2,
    }
}

/// The compression effort dial — a sender-only preference (the codec output is
/// self-describing, so the decoder needs no knowledge of the level). `Fast` favors
/// throughput, `Max` favors ratio, `Balanced` is the default middle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WireCompressionLevel {
    Fast,
    Balanced,
    Max,
}

thread_local! {
    static COMPRESSION_CODEC: std::cell::Cell<WireCompression> =
        const { std::cell::Cell::new(WireCompression::None) };
    static COMPRESSION_LEVEL: std::cell::Cell<WireCompressionLevel> =
        const { std::cell::Cell::new(WireCompressionLevel::Balanced) };
}

/// Compress encoded bodies with `codec` (kept only if smaller) for the duration of
/// `f`. Scoped so it never leaks.
pub fn with_compression_codec<T>(codec: WireCompression, f: impl FnOnce() -> T) -> T {
    let prev = COMPRESSION_CODEC.with(|c| c.replace(codec));
    let out = f();
    COMPRESSION_CODEC.with(|c| c.set(prev));
    out
}

/// Set the compression effort for the duration of `f`. Scoped so it never leaks.
pub fn with_compression_level<T>(level: WireCompressionLevel, f: impl FnOnce() -> T) -> T {
    let prev = COMPRESSION_LEVEL.with(|c| c.replace(level));
    let out = f();
    COMPRESSION_LEVEL.with(|c| c.set(prev));
    out
}

fn compression_level() -> WireCompressionLevel {
    COMPRESSION_LEVEL.with(std::cell::Cell::get)
}

/// DEFLATE effort: miniz_oxide levels 1 (fast) / 6 (balanced) / 9 (max).
fn deflate_level() -> u8 {
    match compression_level() {
        WireCompressionLevel::Fast => 1,
        WireCompressionLevel::Balanced => 6,
        WireCompressionLevel::Max => 9,
    }
}

/// zstd effort: levels 1 (fast) / 9 (balanced) / 19 (max). Decode speed is
/// level-independent in zstd, so `Max` costs only encode time.
#[cfg(not(target_arch = "wasm32"))]
fn zstd_level() -> i32 {
    match compression_level() {
        WireCompressionLevel::Fast => 1,
        WireCompressionLevel::Balanced => 9,
        WireCompressionLevel::Max => 19,
    }
}

/// Back-compat convenience: compress with DEFLATE (what the bare `Send compressed`
/// keyword selects).
pub fn with_compression<T>(f: impl FnOnce() -> T) -> T {
    with_compression_codec(WireCompression::Deflate, f)
}

fn compression_codec() -> WireCompression {
    COMPRESSION_CODEC.with(std::cell::Cell::get)
}

/// Compress `body` with `codec`, returning the codec actually used (it may differ
/// from the request: a wasm `Zstd` encode falls back to `Lz4`) and the bytes. The
/// caller keeps the result only if it shrank.
fn compress_body(codec: WireCompression, body: &[u8]) -> Option<(WireCompression, Vec<u8>)> {
    match codec {
        WireCompression::None => None,
        WireCompression::Deflate => Some((codec, miniz_oxide::deflate::compress_to_vec(body, deflate_level()))),
        WireCompression::Lz4 => Some((codec, lz4_flex::compress_prepend_size(body))),
        WireCompression::Zstd => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                zstd::encode_all(body, zstd_level()).ok().map(|z| (WireCompression::Zstd, z))
            }
            #[cfg(target_arch = "wasm32")]
            {
                // No C encoder in the browser — fall back to lz4 (still universally
                // decodable). The header will record lz4, not zstd.
                Some((WireCompression::Lz4, lz4_flex::compress_prepend_size(body)))
            }
        }
    }
}

/// The smallest `body` compresses to across the built-in compressors (deflate / lz4 / zstd), or its
/// raw length when none helps — the "fair fight" size for an arbitrary byte string. This is the same
/// shop-every-compressor rule [`message_to_wire_best`]'s `Smallest` goal applies to the LOGOS wire,
/// exposed so a benchmark can grant a COMPETITOR codec the identical compression opportunity: then a
/// size comparison is compressed-vs-compressed (fair), not compressed-LOGOS-vs-raw-competitor.
pub fn best_compressed_len(body: &[u8]) -> usize {
    [WireCompression::Deflate, WireCompression::Lz4, WireCompression::Zstd]
        .into_iter()
        .filter_map(|c| compress_body(c, body).map(|(_, z)| z.len()))
        .fold(body.len(), usize::min)
}

/// Inflate `body` that was compressed with `codec`. `None` on any malformed input.
fn decompress_body(codec: WireCompression, body: &[u8]) -> Option<Vec<u8>> {
    match codec {
        WireCompression::None => Some(body.to_vec()),
        WireCompression::Deflate => miniz_oxide::inflate::decompress_to_vec(body).ok(),
        WireCompression::Lz4 => lz4_flex::decompress_size_prepended(body).ok(),
        WireCompression::Zstd => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                zstd::decode_all(body).ok()
            }
            #[cfg(target_arch = "wasm32")]
            {
                zstd_decode_ruzstd(body)
            }
        }
    }
}

/// Pure-Rust zstd decode (the browser's decode path; also the native C-vs-ruzstd
/// parity oracle). A standard zstd frame in, the inflated bytes out.
fn zstd_decode_ruzstd(body: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut dec = ruzstd::StreamingDecoder::new(body).ok()?;
    let mut out = Vec::new();
    dec.read_to_end(&mut out).ok()?;
    Some(out)
}

// ---- Group varint (Stream VByte layout) for int arrays ------------------------
//
// Each int (zig-zag → u64) is stored at the NARROWEST of {1,2,4,8} bytes; a 2-bit
// width code per int packs four codes into one control byte. The control stream is
// written BEFORE the data stream, so widths are known up front — DECODE reads one
// WIDE unaligned word per int and masks it (no per-byte continuation branch, no
// per-element zeroing/copy), and the layout is what a SIMD shuffle consumes.

#[inline]
fn gv_code(zz: u64) -> u8 {
    if zz <= 0xFF {
        0
    } else if zz <= 0xFFFF {
        1
    } else if zz <= 0xFFFF_FFFF {
        2
    } else {
        3
    }
}

/// LEB128 varint array (`T_INTS`): a header, then one varint per element. The smallest
/// layout and the best *scalar* decode — the default.
///
/// ADAPTIVE SIGN MODE (zero-overhead): the header is `(count << 1) | signed`, where
/// `signed` is set iff the column holds a negative value. A non-negative column ships as
/// PLAIN LEB128 — one byte for every value `< 128`, where zig-zag would spend two (it
/// doubles the magnitude, halving the one-byte range to `< 64`). Non-negative data (ids,
/// counts, sizes, timestamps) is then up to half the bytes, matching protobuf's `int64`;
/// a column with any negative keeps zig-zag (protobuf's `sint64`). The mode rides the
/// count's low bit, so it costs ZERO extra bytes.
fn leb128_encode<I: Iterator<Item = i64> + Clone>(out: &mut Vec<u8>, vals: I, n: usize) {
    let signed = vals.clone().any(|x| x < 0);
    write_uvarint(((n as u64) << 1) | signed as u64, out);
    out.reserve(n * 2);
    if signed {
        for x in vals {
            write_uvarint(zigzag(x), out);
        }
    } else {
        for x in vals {
            write_uvarint(x as u64, out);
        }
    }
}

/// Fixed-width array (`T_INTS_FIXED`): the `i64` buffer's little-endian bytes ARE
/// the wire bytes — one `memcpy` (same trick as floats).
fn fixed_encode_i64(out: &mut Vec<u8>, v: &[i64]) {
    write_uvarint(v.len() as u64, out);
    #[cfg(target_endian = "little")]
    {
        // SAFETY: reading `&[i64]` as `&[u8]` of the same byte length.
        let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) };
        out.extend_from_slice(bytes);
    }
    #[cfg(target_endian = "big")]
    {
        out.reserve(v.len() * 8);
        for &n in v {
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}

fn gv_encode<I: Iterator<Item = i64> + Clone>(out: &mut Vec<u8>, vals: I, n: usize) {
    write_uvarint(n as u64, out);
    let control_at = out.len();
    out.resize(control_at + n.div_ceil(4), 0);
    out.reserve(n * 2);
    for (i, x) in vals.enumerate() {
        let zz = zigzag(x);
        let code = gv_code(zz);
        out[control_at + (i >> 2)] |= code << ((i & 3) * 2);
        out.extend_from_slice(&zz.to_le_bytes()[..1usize << code]);
    }
}

fn gv_decode(buf: &[u8], pos: &mut usize) -> Option<Vec<i64>> {
    let n = read_uvarint(buf, pos)? as usize;
    let control_len = n.div_ceil(4);
    let control = buf.get(*pos..pos.checked_add(control_len)?)?;
    let mut dpos = *pos + control_len;
    let len = buf.len();
    let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
    for i in 0..n {
        let code = (control[i >> 2] >> ((i & 3) * 2)) & 0x3;
        let width = 1usize << code;
        let zz = if dpos + 8 <= len {
            // Fast path: one wide load, mask off the high `8 - width` bytes. The
            // `+ 8 <= len` guard makes the fixed-size read in-bounds.
            let word = u64::from_le_bytes(buf[dpos..dpos + 8].try_into().unwrap());
            let mask = if width == 8 { u64::MAX } else { (1u64 << (width * 8)) - 1 };
            word & mask
        } else {
            // Safe tail near the buffer end: an exact-width read.
            let raw = buf.get(dpos..dpos.checked_add(width)?)?;
            let mut b = [0u8; 8];
            b[..width].copy_from_slice(raw);
            u64::from_le_bytes(b)
        };
        dpos += width;
        v.push(unzigzag(zz));
    }
    *pos = dpos;
    Some(v)
}

/// Decode a group-varint (`T_INTS_GV`) block, taking the SSSE3 shuffle fast path
/// when the CPU has it and falling back to [`gv_decode`] otherwise. Both produce
/// bit-identical output — `gv_decode` is the oracle the SIMD path is fuzzed against.
fn gv_decode_dispatch(buf: &[u8], pos: &mut usize) -> Option<Vec<i64>> {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("ssse3") {
            // SAFETY: guarded by the runtime SSSE3 feature check.
            return unsafe { gv_decode_ssse3(buf, pos) };
        }
    }
    gv_decode(buf, pos)
}

/// The 16 PSHUFB control masks, indexed by `(code_a << 2) | code_b` where each
/// code ∈ {0,1,2,3} selects a width of {1,2,4,8} bytes. Lane 0 gathers int A's
/// `width_a` low bytes (rest zeroed); lane 1 gathers int B's `width_b` bytes from
/// offset `width_a`. A `0x80` index makes PSHUFB write a zero byte.
#[cfg(target_arch = "x86_64")]
fn gv_shuffle_masks() -> &'static [[u8; 16]; 16] {
    use std::sync::OnceLock;
    static MASKS: OnceLock<[[u8; 16]; 16]> = OnceLock::new();
    MASKS.get_or_init(|| {
        let mut m = [[0x80u8; 16]; 16];
        for ca in 0..4usize {
            for cb in 0..4usize {
                let (wa, wb) = (1usize << ca, 1usize << cb);
                let entry = &mut m[(ca << 2) | cb];
                for j in 0..wa {
                    entry[j] = j as u8;
                }
                for k in 0..wb {
                    entry[8 + k] = (wa + k) as u8;
                }
            }
        }
        m
    })
}

/// SSSE3 group-varint decode: two ints per PSHUFB. Each 16-byte data load holds
/// both ints' little-endian bytes back-to-back; one shuffle splats them into the
/// two 8-byte lanes of an XMM register, then we read the lanes as `u64`s. The
/// `dpos + 16 <= len` guard keeps the wide load in-bounds; the odd/near-end tail
/// is finished with the exact-width scalar reader.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn gv_decode_ssse3(buf: &[u8], pos: &mut usize) -> Option<Vec<i64>> {
    use std::arch::x86_64::*;
    let n = read_uvarint(buf, pos)? as usize;
    let control_len = n.div_ceil(4);
    let control = buf.get(*pos..pos.checked_add(control_len)?)?;
    let mut dpos = *pos + control_len;
    let len = buf.len();
    let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
    let masks = gv_shuffle_masks();
    let mut i = 0;
    while i + 2 <= n && dpos + 16 <= len {
        let ctrl = control[i >> 2];
        let ca = ((ctrl >> ((i & 3) * 2)) & 0x3) as usize;
        let cb = ((ctrl >> (((i + 1) & 3) * 2)) & 0x3) as usize;
        let data = _mm_loadu_si128(buf.as_ptr().add(dpos).cast());
        let mask = _mm_loadu_si128(masks[(ca << 2) | cb].as_ptr().cast());
        let out = _mm_shuffle_epi8(data, mask);
        let mut tmp = [0u8; 16];
        _mm_storeu_si128(tmp.as_mut_ptr().cast(), out);
        v.push(unzigzag(u64::from_le_bytes(tmp[0..8].try_into().unwrap())));
        v.push(unzigzag(u64::from_le_bytes(tmp[8..16].try_into().unwrap())));
        dpos += (1usize << ca) + (1usize << cb);
        i += 2;
    }
    while i < n {
        let code = (control[i >> 2] >> ((i & 3) * 2)) & 0x3;
        let width = 1usize << code;
        let raw = buf.get(dpos..dpos.checked_add(width)?)?;
        let mut b = [0u8; 8];
        b[..width].copy_from_slice(raw);
        v.push(unzigzag(u64::from_le_bytes(b)));
        dpos += width;
        i += 1;
    }
    *pos = dpos;
    Some(v)
}

#[inline]
fn write_uvarint(mut x: u64, out: &mut Vec<u8>) {
    while x >= 0x80 {
        out.push((x as u8) | 0x80);
        x >>= 7;
    }
    out.push(x as u8);
}

#[inline]
fn read_uvarint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let b = *buf.get(*pos)?;
        *pos += 1;
        if shift >= 64 {
            return None; // overlong / overflow
        }
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

#[inline]
fn zigzag(x: i64) -> u64 {
    ((x << 1) ^ (x >> 63)) as u64
}

#[inline]
fn unzigzag(x: u64) -> i64 {
    ((x >> 1) as i64) ^ -((x & 1) as i64)
}

#[inline]
fn write_str(s: &str, out: &mut Vec<u8>) {
    write_uvarint(s.len() as u64, out);
    out.extend_from_slice(s.as_bytes());
}

#[inline]
fn read_str(buf: &[u8], pos: &mut usize) -> Option<String> {
    let n = read_uvarint(buf, pos)? as usize;
    let bytes = buf.get(*pos..pos.checked_add(n)?)?;
    *pos += n;
    String::from_utf8(bytes.to_vec()).ok()
}

/// Write a flat string array: count, each element's byte length (varint, derived
/// from the cumulative `ends`), then the whole bytes blob in one copy.
/// Dictionary-encode a string column (`T_STRINGS_DICT`): each distinct string is shipped once,
/// then a bit-packed per-row index into that dictionary — the string twin of [`dict_encode`].
/// A win exactly when cardinality is low (categorical labels); `emit_best_string_column` keeps
/// it only if it beats the plain flat array, so it is never larger.
fn dict_encode_strings(data: &[u8], ends: &[u32]) -> Vec<u8> {
    let n = ends.len();
    let mut dict: Vec<&[u8]> = Vec::new();
    let mut index_of: std::collections::HashMap<&[u8], u64> = std::collections::HashMap::new();
    let mut indices: Vec<u64> = Vec::with_capacity(n);
    let mut prev = 0u32;
    for &e in ends {
        let s = &data[prev as usize..e as usize];
        prev = e;
        let idx = *index_of.entry(s).or_insert_with(|| {
            dict.push(s);
            (dict.len() - 1) as u64
        });
        indices.push(idx);
    }
    let mut out = vec![T_STRINGS_DICT];
    write_uvarint(dict.len() as u64, &mut out);
    for d in &dict {
        write_uvarint(d.len() as u64, &mut out);
        out.extend_from_slice(d);
    }
    write_uvarint(n as u64, &mut out);
    let iw = if dict.len() <= 1 { 0 } else { (64 - ((dict.len() - 1) as u64).leading_zeros()) as u8 };
    out.push(iw);
    if iw > 0 {
        out.extend_from_slice(&bitpack(&indices, iw));
    }
    out
}

/// Emit the smaller of the plain flat string array and the dictionary form. The flat array is
/// always a candidate, so the result is never larger than `T_STRINGS`.
/// The `(start, end)` byte range of each string in a flat `data`/`ends` column.
fn string_slices<'a>(data: &'a [u8], ends: &[u32]) -> Vec<&'a [u8]> {
    let mut out = Vec::with_capacity(ends.len());
    let mut prev = 0usize;
    for &e in ends {
        out.push(&data[prev..e as usize]);
        prev = e as usize;
    }
    out
}

/// If every string is `<prefix><n><suffix>` for a common `prefix`/`suffix` and an AFFINE sequence of
/// integers `n = base + i·stride` whose EXACT decimal spelling is the middle (so reconstruction is
/// byte-perfect — no zero-padding / `+` quirks), return the templated encoding: the two affixes once +
/// `(base, stride, count)`. Sequential-id URLs / paths / labels (`item_0…item_999`, `…/items/0…`)
/// collapse from O(n) to a handful of bytes — the string twin of the affine int generator.
fn try_template_encode(data: &[u8], ends: &[u32]) -> Option<Vec<u8>> {
    let n = ends.len();
    if n < 3 {
        return None;
    }
    let strs = string_slices(data, ends);
    let first = strs[0];
    // Common prefix (byte-wise).
    let mut prefix_len = first.len();
    for s in &strs[1..] {
        let lim = prefix_len.min(s.len());
        let mut i = 0;
        while i < lim && first[i] == s[i] {
            i += 1;
        }
        prefix_len = i;
    }
    // Common suffix of the PREFIX-STRIPPED remainders (so prefix and suffix never overlap).
    let mut suffix_len = first.len() - prefix_len;
    for s in &strs[1..] {
        let lim = suffix_len.min(s.len() - prefix_len);
        let mut i = 0;
        while i < lim && first[first.len() - 1 - i] == s[s.len() - 1 - i] {
            i += 1;
        }
        suffix_len = i;
    }
    // Parse each middle as an i64 whose canonical decimal spelling is exactly the middle bytes.
    let mut nums = Vec::with_capacity(n);
    for s in &strs {
        let mid = &s[prefix_len..s.len() - suffix_len];
        let mid_str = std::str::from_utf8(mid).ok()?;
        let num: i64 = mid_str.parse().ok()?;
        if num.to_string().as_bytes() != mid {
            return None;
        }
        nums.push(num);
    }
    let (base, stride) = detect_affine(&nums)?;
    let mut c = vec![T_STRINGS_TEMPLATE];
    write_uvarint(prefix_len as u64, &mut c);
    c.extend_from_slice(&first[..prefix_len]);
    write_uvarint(suffix_len as u64, &mut c);
    c.extend_from_slice(&first[first.len() - suffix_len..]);
    write_uvarint(zigzag(base), &mut c);
    write_uvarint(zigzag(stride), &mut c);
    write_uvarint(n as u64, &mut c);
    Some(c)
}

/// Front-code the column IN ORDER: each string ships `(shared-prefix-len-with-the-previous, suffix)`.
/// Sorted or hierarchical columns — file paths, object-store keys, zero-padded ids, sorted labels —
/// have adjacent strings that share long prefixes, so only the per-row delta goes on the wire. Order
/// is preserved (a column, not a set), so it round-trips a column the dictionary can't crush (all
/// strings distinct) and the template can't (non-affine / zero-padded middles). When no two adjacent
/// strings share a prefix it costs ~1 extra byte per row over flat — `emit_best_string_column`'s
/// `consider` then keeps the flat form, so this is never a loss.
fn front_code_strings(data: &[u8], ends: &[u32]) -> Vec<u8> {
    let strs = string_slices(data, ends);
    let mut out = vec![T_STRINGS_FRONT];
    write_uvarint(strs.len() as u64, &mut out);
    let mut prev: &[u8] = &[];
    for s in &strs {
        let lim = prev.len().min(s.len());
        let mut common = 0usize;
        while common < lim && prev[common] == s[common] {
            common += 1;
        }
        // Back off to a UTF-8 char boundary so the shipped suffix is itself valid UTF-8.
        while common > 0 && common < s.len() && (s[common] & 0xC0) == 0x80 {
            common -= 1;
        }
        write_uvarint(common as u64, &mut out);
        write_uvarint((s.len() - common) as u64, &mut out);
        out.extend_from_slice(&s[common..]);
        prev = s;
    }
    out
}

/// The bit-packed bool baseline: `T_BOOLS` + count + 8 booleans per byte (LSB-first).
fn bool_bitpack(v: &[bool], out: &mut Vec<u8>) {
    out.push(T_BOOLS);
    write_uvarint(v.len() as u64, out);
    let mut cur = 0u8;
    let mut nbits = 0u8;
    for &b in v {
        cur |= u8::from(b) << nbits;
        nbits += 1;
        if nbits == 8 {
            out.push(cur);
            cur = 0;
            nbits = 0;
        }
    }
    if nbits > 0 {
        out.push(cur);
    }
}

/// Bound on the cyclic period a bool column is probed for — small, since real bool cycles are tiny
/// (1 = constant, 2 = alternating, 7 = a weekly flag); a 256-bit block is still only 32 bytes.
const BOOL_PERIOD_CAP: usize = 256;

/// The MINIMAL period `1 ≤ p ≤ n/2` such that `v[i] == v[i-p]` everywhere (so the column is exactly
/// `block[i % p]`). `p == 1` is a constant column (all-true / all-false); `p == 2` is alternating.
fn detect_bool_period(v: &[bool]) -> Option<usize> {
    let n = v.len();
    let cap = (n / 2).min(BOOL_PERIOD_CAP);
    'p: for p in 1..=cap {
        for i in p..n {
            if v[i] != v[i - p] {
                continue 'p;
            }
        }
        return Some(p);
    }
    None
}

/// Ship `period p + count + one p-bit block`; the decoder replays `block[i % p]`.
fn bool_periodic_encode(v: &[bool], p: usize) -> Vec<u8> {
    let mut c = vec![T_BOOLS_PERIODIC];
    write_uvarint(p as u64, &mut c);
    write_uvarint(v.len() as u64, &mut c);
    let mut cur = 0u8;
    let mut nbits = 0u8;
    for &b in &v[..p] {
        cur |= u8::from(b) << nbits;
        nbits += 1;
        if nbits == 8 {
            c.push(cur);
            cur = 0;
            nbits = 0;
        }
    }
    if nbits > 0 {
        c.push(cur);
    }
    c
}

/// Ship `first value + the run lengths` (runs alternate). One big run, or a handful of clustered
/// flips, collapses to a few varints; a high-flip column makes this larger than bit-pack, and
/// `emit_best_bool_column`'s `consider` then keeps the bit-packed form.
fn bool_rle_encode(v: &[bool]) -> Vec<u8> {
    let mut c = vec![T_BOOLS_RLE];
    write_uvarint(v.len() as u64, &mut c);
    if v.is_empty() {
        c.push(0); // `first` placeholder so the frame layout matches the decoder (bit-pack wins anyway)
        write_uvarint(0, &mut c);
        return c;
    }
    c.push(v[0] as u8);
    let mut runs: Vec<u64> = Vec::new();
    let mut cur = v[0];
    let mut len = 0u64;
    for &b in v {
        if b == cur {
            len += 1;
        } else {
            runs.push(len);
            cur = b;
            len = 1;
        }
    }
    runs.push(len);
    write_uvarint(runs.len() as u64, &mut c);
    for r in runs {
        write_uvarint(r, &mut c);
    }
    c
}

fn emit_best_bool_column(v: &[bool], out: &mut Vec<u8>) {
    let mut best = Vec::new();
    bool_bitpack(v, &mut best);
    if let Some(p) = detect_bool_period(v) {
        consider(&mut best, bool_periodic_encode(v, p));
    }
    consider(&mut best, bool_rle_encode(v));
    out.extend_from_slice(&best);
}

/// The longest common BYTE prefix and the longest common BYTE suffix (of the prefix-stripped
/// remainders, so they never overlap) shared by EVERY string, each clamped back to a UTF-8 char
/// boundary. Shared by the affix and template encoders.
fn common_affix_lens(strs: &[&[u8]]) -> (usize, usize) {
    let first = strs[0];
    let mut prefix_len = first.len();
    for s in &strs[1..] {
        let lim = prefix_len.min(s.len());
        let mut i = 0;
        while i < lim && first[i] == s[i] {
            i += 1;
        }
        prefix_len = i;
    }
    while prefix_len > 0 && prefix_len < first.len() && (first[prefix_len] & 0xC0) == 0x80 {
        prefix_len -= 1;
    }
    let mut suffix_len = first.len() - prefix_len;
    for s in &strs[1..] {
        let lim = suffix_len.min(s.len() - prefix_len);
        let mut i = 0;
        while i < lim && first[first.len() - 1 - i] == s[s.len() - 1 - i] {
            i += 1;
        }
        suffix_len = i;
    }
    while suffix_len > 0 && (first[first.len() - suffix_len] & 0xC0) == 0x80 {
        suffix_len -= 1;
    }
    (prefix_len, suffix_len)
}

/// If every string is `<prefix><middle><suffix>` for a common `prefix`/`suffix` (ARBITRARY middles —
/// no affine constraint), ship the two affixes ONCE + each middle. Catches the column the dictionary
/// can't (all distinct), the template can't (non-affine middles), and front-coding can't (the shared
/// part is a SUFFIX): emails `…@example.com`, files `…​.log`, wrapped ids `v…​.json`. Worthwhile only
/// when there's a shared affix; `consider` keeps the flat form otherwise.
fn try_affix_encode(data: &[u8], ends: &[u32]) -> Option<Vec<u8>> {
    let n = ends.len();
    if n < 2 {
        return None;
    }
    let strs = string_slices(data, ends);
    let (prefix_len, suffix_len) = common_affix_lens(&strs);
    if prefix_len + suffix_len == 0 {
        return None;
    }
    let first = strs[0];
    let mut c = vec![T_STRINGS_AFFIX];
    write_uvarint(prefix_len as u64, &mut c);
    c.extend_from_slice(&first[..prefix_len]);
    write_uvarint(suffix_len as u64, &mut c);
    c.extend_from_slice(&first[first.len() - suffix_len..]);
    write_uvarint(n as u64, &mut c);
    for s in &strs {
        let mid = &s[prefix_len..s.len() - suffix_len];
        write_uvarint(mid.len() as u64, &mut c);
        c.extend_from_slice(mid);
    }
    Some(c)
}

fn emit_best_string_column(data: &[u8], ends: &[u32], out: &mut Vec<u8>) {
    let mut best = Vec::new();
    write_string_array_from_ends(&mut best, data, ends);
    consider(&mut best, dict_encode_strings(data, ends));
    consider(&mut best, front_code_strings(data, ends));
    if let Some(tpl) = try_template_encode(data, ends) {
        consider(&mut best, tpl);
    }
    if let Some(affix) = try_affix_encode(data, ends) {
        consider(&mut best, affix);
    }
    out.extend_from_slice(&best);
}

fn write_string_array_from_ends(out: &mut Vec<u8>, data: &[u8], ends: &[u32]) {
    out.push(T_STRINGS);
    write_uvarint(ends.len() as u64, out);
    let mut prev = 0u32;
    for &e in ends {
        write_uvarint(u64::from(e - prev), out);
        prev = e;
    }
    out.extend_from_slice(data);
}

/// If `v` is a non-empty run of structs that all share one `type_name` and the
/// same field-name set, return `(type_name, sorted_field_names)` — the schema for a
/// columnar [`T_STRUCTS`] encoding. `None` otherwise (the list stays boxed). The
/// field order is canonical (sorted), matching the per-struct [`T_STRUCT`] path, so
/// a round-trip is byte-stable.
fn struct_schema(v: &[RuntimeValue]) -> Option<(String, Vec<String>)> {
    let first = match v.first()? {
        RuntimeValue::Struct(s) => s,
        _ => return None,
    };
    let mut names: Vec<String> = first.fields.keys().cloned().collect();
    names.sort();
    // A columnar store needs ≥1 column to carry the row count — a zero-field struct
    // list stays boxed (encodes as the generic per-element list).
    if names.is_empty() {
        return None;
    }
    for item in v {
        match item {
            RuntimeValue::Struct(s)
                if s.type_name == first.type_name
                    && s.fields.len() == names.len()
                    && names.iter().all(|n| s.fields.contains_key(n)) => {}
            _ => return None,
        }
    }
    Some((first.type_name.clone(), names))
}

/// Emit a homogeneous struct list: the schema (type name + field names) followed by
/// `encode_columns`. With no schema cache active it is the self-describing `T_STRUCTS`
/// (schema inline). With a cache, the first occurrence of a schema is a `T_STRUCTS_DEF`
/// (schema inline + registered under an id, still self-decodable) and later ones are a
/// `T_STRUCTS_REF` (just the id) — the cross-message win. The columns are identical in
/// all three forms, so the only difference is whether the schema strings are present.
/// Write a struct schema (type name + field names) — the exact byte layout
/// [`read_struct_schema`] consumes. Shared by the struct-list and single-struct
/// schema-dictionary paths so their `def` forms can never drift.
fn write_struct_schema(type_name: &str, field_names: &[String], out: &mut Vec<u8>) {
    write_str(type_name, out);
    write_uvarint(field_names.len() as u64, out);
    for f in field_names {
        write_str(f, out);
    }
}

fn emit_struct_list(
    type_name: &str,
    field_names: &[String],
    n: usize,
    out: &mut Vec<u8>,
    encode_columns: impl FnOnce(&mut Vec<u8>) -> Result<(), String>,
) -> Result<(), String> {
    let write_schema = |out: &mut Vec<u8>| write_struct_schema(type_name, field_names, out);
    if let Some(id) = type_registry_id(type_name, field_names) {
        // Both ends share this type's definition (the program-derived registry): ship the
        // small id and the columns only — type/field NAMES never go on the wire. The
        // struct-list analog of `T_STRUCT_TID`; takes precedence over the schema cache,
        // mirroring the single-struct path.
        out.push(T_STRUCTS_TID);
        write_uvarint(id as u64, out);
        write_uvarint(n as u64, out);
        return encode_columns(out);
    }
    match schema_send(type_name, field_names) {
        SchemaEmit::Inline => {
            out.push(T_STRUCTS);
            write_schema(out);
        }
        SchemaEmit::SeqDef(id) => {
            out.push(T_STRUCTS_DEF);
            write_uvarint(id as u64, out);
            write_schema(out);
        }
        SchemaEmit::SeqRef(id) => {
            out.push(T_STRUCTS_REF);
            write_uvarint(id as u64, out);
        }
        SchemaEmit::CaDef => {
            out.push(T_STRUCTS_CDEF);
            write_schema(out);
        }
        SchemaEmit::CaRef(fp) => {
            out.push(T_STRUCTS_CREF);
            out.extend_from_slice(&fp.to_le_bytes());
        }
    }
    write_uvarint(n as u64, out);
    encode_columns(out)
}

/// Encode a homogeneous struct list in the random-access `T_STRUCTS_VIEW` layout: the
/// shared schema (type + sorted field names) once, then a per-ROW byte-offset table, then
/// each row's own per-FIELD byte-offset table followed by its values. A `WireView` reaches
/// ANY (row, field) in O(1) via the two offset tables — the record-list analog of the
/// single-struct view, beating Cap'n Proto's `items.get(i).get_field()` on open + access.
/// `field_names` MUST be canonically sorted and `get(row, field_index)` returns that row's
/// value for `field_names[field_index]`, so the Boxed and columnar `Structs` sources emit
/// byte-identical output for the same logical data.
fn emit_structs_view(
    type_name: &str,
    field_names: &[String],
    n: usize,
    out: &mut Vec<u8>,
    mut get: impl FnMut(usize, usize) -> RuntimeValue,
) -> Result<(), String> {
    let f = field_names.len();
    out.push(T_STRUCTS_VIEW);
    write_str(type_name, out);
    write_uvarint(f as u64, out);
    for name in field_names {
        write_str(name, out);
    }
    write_uvarint(n as u64, out);
    let row_table_pos = out.len();
    out.resize(row_table_pos + n * 4, 0);
    let rows_start = out.len();
    let mut row_offsets: Vec<u32> = Vec::with_capacity(n);
    for r in 0..n {
        row_offsets.push((out.len() - rows_start) as u32);
        let field_table_pos = out.len();
        out.resize(field_table_pos + f * 4, 0);
        let values_start = out.len();
        let mut field_offsets: Vec<u32> = Vec::with_capacity(f);
        for fi in 0..f {
            field_offsets.push((out.len() - values_start) as u32);
            native_encode(&get(r, fi), out)?;
        }
        for (i, off) in field_offsets.iter().enumerate() {
            out[field_table_pos + i * 4..field_table_pos + i * 4 + 4].copy_from_slice(&off.to_le_bytes());
        }
    }
    for (i, off) in row_offsets.iter().enumerate() {
        out[row_table_pos + i * 4..row_table_pos + i * 4 + 4].copy_from_slice(&off.to_le_bytes());
    }
    Ok(())
}

/// The columnar fast path for [`emit_structs_view`]: writes each (row, field) cell straight
/// from the typed column instead of materializing a `RuntimeValue` per cell (the boxed `get`
/// allocates a fresh `Rc<String>` for every text cell — the dominant cost on record lists).
/// Byte-identical to `emit_structs_view` for the same logical data.
fn emit_structs_view_columnar(
    type_name: &str,
    field_names: &[String],
    columns: &[ListRepr],
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let f = field_names.len();
    let n = columns.first().map_or(0, |c| c.len());
    out.push(T_STRUCTS_VIEW);
    write_str(type_name, out);
    write_uvarint(f as u64, out);
    for name in field_names {
        write_str(name, out);
    }
    write_uvarint(n as u64, out);
    let row_table_pos = out.len();
    out.resize(row_table_pos + n * 4, 0);
    let rows_start = out.len();
    let mut row_offsets: Vec<u32> = Vec::with_capacity(n);
    let mut field_offsets: Vec<u32> = Vec::with_capacity(f);
    for r in 0..n {
        row_offsets.push((out.len() - rows_start) as u32);
        let field_table_pos = out.len();
        out.resize(field_table_pos + f * 4, 0);
        let values_start = out.len();
        field_offsets.clear();
        for col in columns {
            field_offsets.push((out.len() - values_start) as u32);
            write_view_cell(col, r, out)?;
        }
        for (i, off) in field_offsets.iter().enumerate() {
            out[field_table_pos + i * 4..field_table_pos + i * 4 + 4].copy_from_slice(&off.to_le_bytes());
        }
    }
    for (i, off) in row_offsets.iter().enumerate() {
        out[row_table_pos + i * 4..row_table_pos + i * 4 + 4].copy_from_slice(&off.to_le_bytes());
    }
    Ok(())
}

/// Write one struct-view cell straight from its typed column, matching `native_encode`'s
/// tagged bytes exactly. Uncommon column types fall back to materialize-then-encode.
fn write_view_cell(col: &ListRepr, row: usize, out: &mut Vec<u8>) -> Result<(), String> {
    match col {
        ListRepr::Ints(v) => {
            out.push(T_INT);
            write_uvarint(zigzag(v[row]), out);
        }
        ListRepr::IntsI32(v) => {
            out.push(T_INT);
            write_uvarint(zigzag(v[row] as i64), out);
        }
        ListRepr::Floats(v) => {
            out.push(T_FLOAT);
            out.extend_from_slice(&v[row].to_le_bytes());
        }
        ListRepr::Bools(v) => out.push(if v[row] { T_TRUE } else { T_FALSE }),
        ListRepr::Strings { data, ends, .. } => {
            let start = if row == 0 { 0 } else { ends[row - 1] as usize };
            let end = ends[row] as usize;
            let s = &data[start..end];
            out.push(T_TEXT);
            write_uvarint(s.len() as u64, out);
            out.extend_from_slice(s);
        }
        other => native_encode(&other.get(row).ok_or("struct-view column row out of bounds")?, out)?,
    }
    Ok(())
}

/// Encode a record list as a FIXED-stride view (`T_STRUCTS_FVIEW`): every row is the same width
/// (Int/Float = 8 B raw, Bool = 1 B, Text = an 8 B (offset,len) into a trailing string blob), so a
/// reader reaches any (row, field) by ARITHMETIC — no row/field offset tables at all. Smaller than
/// the variable view and a near-memcpy encode, while still O(1) random-access. The `indexed fast`
/// form: the struct-view composed with the fixed numeric dial. `None` (out untouched) if any column
/// is not a fixed-width leaf, so the caller falls back to the variable offset-table view.
fn emit_structs_view_fixed(
    type_name: &str,
    field_names: &[String],
    columns: &[ListRepr],
    out: &mut Vec<u8>,
) -> Option<()> {
    let kinds = columns_fview_kinds(columns)?; // checked BEFORE any write — out stays clean on None
    let f = field_names.len();
    let n = columns.first().map_or(0, |c| c.len());
    let (_, stride) = fview_layout(&kinds);
    out.push(T_STRUCTS_FVIEW);
    write_str(type_name, out);
    write_uvarint(f as u64, out);
    for name in field_names {
        write_str(name, out);
    }
    out.extend_from_slice(&kinds);
    write_uvarint(n as u64, out);
    out.reserve(n.saturating_mul(stride));
    let mut blob: Vec<u8> = Vec::new();
    for r in 0..n {
        for col in columns {
            match col {
                ListRepr::Ints(v) => out.extend_from_slice(&v[r].to_le_bytes()),
                ListRepr::IntsI32(v) => out.extend_from_slice(&(v[r] as i64).to_le_bytes()),
                ListRepr::Floats(v) => out.extend_from_slice(&v[r].to_le_bytes()),
                ListRepr::Bools(v) => out.push(v[r] as u8),
                ListRepr::Strings { data, ends, .. } => {
                    let start = if r == 0 { 0 } else { ends[r - 1] as usize };
                    let end = ends[r] as usize;
                    let off = blob.len() as u32;
                    let len = (end - start) as u32;
                    blob.extend_from_slice(&data[start..end]);
                    out.extend_from_slice(&off.to_le_bytes());
                    out.extend_from_slice(&len.to_le_bytes());
                }
                // `columns_fview_kinds` already proved every column is one of the above.
                _ => unreachable!("non-fixed-viewable column passed the kind check"),
            }
        }
    }
    write_uvarint(blob.len() as u64, out);
    out.extend_from_slice(&blob);
    Some(())
}

/// Emit an 8-byte-aligned i64 column (`T_INTS_ALIGNED`) the receiver reads as `&[i64]` with no
/// copy. The pad lands the blob's body offset ≡ 7 mod 8 → ≡ 0 mod 8 once the (≡ 1 mod 8) frame
/// header is prepended, so the slice cast is sound. Shared by the `RuntimeValue` encode path and
/// the build-in-place [`build_columnar_record`] API so both produce byte-identical aligned columns.
fn emit_aligned_i64(v: &[i64], out: &mut Vec<u8>) {
    out.push(T_INTS_ALIGNED);
    write_uvarint(v.len() as u64, out);
    let after_count = out.len();
    let pad = (14 - after_count % 8) % 8;
    out.push(pad as u8);
    out.resize(out.len() + pad, 0);
    #[cfg(target_endian = "little")]
    {
        // SAFETY: an `&[i64]` reinterpreted as `&[u8]` of the same byte length.
        let raw = unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 8) };
        out.extend_from_slice(raw);
    }
    #[cfg(target_endian = "big")]
    {
        out.reserve(v.len() * 8);
        for &n in v {
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}

/// Emit an 8-byte-aligned f64 column (`T_FLOATS_ALIGNED`), the float twin of [`emit_aligned_i64`].
fn emit_aligned_f64(v: &[f64], out: &mut Vec<u8>) {
    out.push(T_FLOATS_ALIGNED);
    write_uvarint(v.len() as u64, out);
    let after_count = out.len();
    let pad = (14 - after_count % 8) % 8;
    out.push(pad as u8);
    out.resize(out.len() + pad, 0);
    #[cfg(target_endian = "little")]
    {
        // SAFETY: an `&[f64]` reinterpreted as `&[u8]` of the same byte length.
        let raw = unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 8) };
        out.extend_from_slice(raw);
    }
    #[cfg(target_endian = "big")]
    {
        out.reserve(v.len() * 8);
        for &x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
}

/// One column of a [`build_columnar_record`] message — a borrowed slice that lands in the wire's
/// zero-copy aligned layout with no intermediate `RuntimeValue`.
#[derive(Clone, Copy)]
pub enum WireColumn<'a> {
    /// An i64 column → `T_INTS_ALIGNED`, read back as `&[i64]`.
    Ints(&'a [i64]),
    /// An f64 column → `T_FLOATS_ALIGNED`, read back as `&[f64]`.
    Floats(&'a [f64]),
}

/// Build a columnar record message **in place** — Cap'n Proto's home turf. The named columns are
/// written DIRECTLY into the offset-table `T_STRUCT_VIEW` + `T_*_ALIGNED` wire layout from borrowed
/// slices: no intermediate `RuntimeValue`, no second serialize pass over the data (each column is a
/// single `memcpy`). The receiver opens it with [`view_message`] and reads ANY column in O(1) and
/// zero-copy via [`WireView::struct_field`] + [`WireView::as_i64_slice`]/[`WireView::as_f64_slice`].
///
/// This is the encode side of the dual zero-encode / zero-decode story: where capnp builds the
/// message in its wire buffer and reads it in place, this writes the same aligned layout in one pass
/// and reads it back with no decode — while staying name-elided and 24–34 % smaller on the wire.
pub fn build_columnar_record(from: &str, type_name: &str, fields: &[(&str, WireColumn)]) -> Vec<u8> {
    // Canonical field order (by name) — byte-identical to the `RuntimeValue` struct-view path and
    // deterministic regardless of the caller's insertion order.
    let mut fields: Vec<(&str, WireColumn)> = fields.to_vec();
    fields.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = Vec::with_capacity(from.len() + type_name.len() + 32 + fields.len() * 64);
    write_str(from, &mut out);
    out.push(T_STRUCT_VIEW);
    write_str(type_name, &mut out);
    write_uvarint(fields.len() as u64, &mut out);
    for (name, _) in &fields {
        write_str(name, &mut out);
    }
    let table_pos = out.len();
    out.resize(table_pos + fields.len() * 4, 0);
    let values_start = out.len();
    let mut offsets: Vec<u32> = Vec::with_capacity(fields.len());
    for (_, col) in fields {
        offsets.push((out.len() - values_start) as u32);
        match col {
            WireColumn::Ints(d) => emit_aligned_i64(d, &mut out),
            WireColumn::Floats(d) => emit_aligned_f64(d, &mut out),
        }
    }
    for (i, off) in offsets.iter().enumerate() {
        out[table_pos + i * 4..table_pos + i * 4 + 4].copy_from_slice(&off.to_le_bytes());
    }
    frame(WireCodec::Native, current_integrity(), WireCompression::None, out)
}

/// Encode a `ListRepr` to the wire — the packed/columnar path. Homogeneous arrays
/// go out as one tag + the typed buffer (no per-element tag, no boxing); a `Structs`
/// repr streams its in-memory columns straight out (a near-memcpy). Shared by the
/// `RuntimeValue::List` arm and, recursively, by struct columns.
fn encode_list_repr(repr: &ListRepr, out: &mut Vec<u8>) -> Result<(), String> {
    let _depth = DepthGuard::enter()?;
    // Flat mode: one uniform, columnar-free `T_LIST` regardless of the in-memory repr — the
    // format the shared wire core decodes. Every element is a fully tagged value, so a native
    // decoder needs no columnar/string/struct/inductive special cases.
    if flat_lists() {
        let values = repr.to_values();
        out.push(T_LIST);
        write_uvarint(values.len() as u64, out);
        for x in &values {
            native_encode(x, out)?;
        }
        return Ok(());
    }
    match repr {
        // Integer arrays dispatch on the sender's numeric strategy; each lands on its
        // own tag, so the decoder reconstructs it regardless. First, if structural
        // encoding is on and the whole column is an exact affine progression, ship
        // the generating formula `(base, stride, n)` instead of the data.
        ListRepr::Ints(v) => {
            if struct_view_on() {
                emit_aligned_i64(v, out);
                return Ok(());
            }
            if structure() == WireStructure::Affine {
                if let Some((base, stride)) = detect_affine(v) {
                    out.push(T_INTS_AFFINE);
                    write_uvarint(zigzag(base), out);
                    write_uvarint(zigzag(stride), out);
                    write_uvarint(v.len() as u64, out);
                    return Ok(());
                }
            }
            if structure() == WireStructure::Auto {
                emit_best_int_column(v, out);
                return Ok(());
            }
            match numerics() {
                WireNumerics::Varint => {
                    out.push(T_INTS);
                    leb128_encode(out, v.iter().copied(), v.len());
                }
                WireNumerics::Fixed => {
                    out.push(T_INTS_FIXED);
                    fixed_encode_i64(out, v);
                }
                WireNumerics::GroupVarint => {
                    out.push(T_INTS_GV);
                    gv_encode(out, v.iter().copied(), v.len());
                }
            }
        }
        ListRepr::IntsI32(v) => {
          if structure() == WireStructure::Auto {
            let widened: Vec<i64> = v.iter().map(|&n| n as i64).collect();
            emit_best_int_column(&widened, out);
            return Ok(());
          }
          match numerics() {
            WireNumerics::Varint => {
                out.push(T_INTS);
                leb128_encode(out, v.iter().map(|&n| n as i64), v.len());
            }
            WireNumerics::Fixed => {
                out.push(T_INTS_FIXED);
                write_uvarint(v.len() as u64, out);
                out.reserve(v.len() * 8);
                for &n in v {
                    out.extend_from_slice(&(n as i64).to_le_bytes());
                }
            }
            WireNumerics::GroupVarint => {
                out.push(T_INTS_GV);
                gv_encode(out, v.iter().map(|&n| n as i64), v.len());
            }
          }
        }
        ListRepr::Floats(v) => {
            if struct_view_on() {
                emit_aligned_f64(v, out);
                return Ok(());
            }
            // The float dial is decided AHEAD OF TIME: under `Off` this path is a pure memcpy and
            // never inspects the data (the lightning-quick, know-the-shape-beforehand hot path). The
            // closed-form generators and the shrinking menu below run ONLY when the caller opts into
            // structural analysis (`Affine`/`Auto`) — mirroring the integer contract exactly.
            let st = structure();
            // Ship the GENERATOR, not the data — the float twin of the int closed-forms. A constant
            // column is one f64 + count; a bit-exact `base + i·stride` column is three numbers. Both
            // are LOSSLESS (constant by `to_bits`, affine verified bit-exact), so they only ever fire
            // when reconstruction is perfect, and they are always smaller than the n×8 raw form.
            if matches!(st, WireStructure::Affine | WireStructure::Auto) {
                if let Some(bits) = detect_float_const(v) {
                    out.push(T_FLOATS_CONST);
                    out.extend_from_slice(&bits.to_le_bytes());
                    write_uvarint(v.len() as u64, out);
                    return Ok(());
                }
                if let Some((base, stride)) = detect_float_affine(v) {
                    out.push(T_FLOATS_AFFINE);
                    out.extend_from_slice(&base.to_le_bytes());
                    out.extend_from_slice(&stride.to_le_bytes());
                    write_uvarint(v.len() as u64, out);
                    return Ok(());
                }
            }
            // The `Auto` size-shrinking menu: a sparse / mostly-one-value column ships its dominant
            // value + outliers; a `base·ratioⁱ` or a cyclic `pattern[i % p]` column ships the
            // generator, not the samples. Each is an O(n) scan, so it runs ONLY under `Auto` — the
            // default path never pays for it. Every candidate is kept only when genuinely smaller
            // than the raw `memcpy` floor. XOR-delta is the separate `WireFloats::XorDelta` DIAL
            // (independent of structure), so it is tried whenever that dial is selected.
            let sparse: Option<Vec<u8>> = if st == WireStructure::Auto {
                detect_float_sparse(v).map(|(dom, exc)| {
                    let mut c = vec![T_FLOATS_SPARSE];
                    c.extend_from_slice(&dom.to_le_bytes());
                    write_uvarint(v.len() as u64, &mut c);
                    write_uvarint(exc.len() as u64, &mut c);
                    let mut prev = 0usize;
                    for (i, bits) in &exc {
                        write_uvarint((i - prev) as u64, &mut c);
                        prev = *i;
                        c.extend_from_slice(&bits.to_le_bytes());
                    }
                    c
                })
            } else {
                None
            };
            let xor: Option<Vec<u8>> = if floats_mode() == WireFloats::XorDelta {
                let mut body = Vec::with_capacity(v.len() + 2);
                floats_xor_encode(&mut body, v);
                if body.len() < floats_memcpy_body_len(v.len()) {
                    let mut c = vec![T_FLOATS_XOR];
                    c.extend_from_slice(&body);
                    Some(c)
                } else {
                    None
                }
            } else {
                None
            };
            let geometric: Option<Vec<u8>> = if st == WireStructure::Auto {
                detect_float_geometric(v).map(|(base, ratio)| {
                    let mut c = vec![T_FLOATS_GEOMETRIC];
                    c.extend_from_slice(&base.to_le_bytes());
                    c.extend_from_slice(&ratio.to_le_bytes());
                    write_uvarint(v.len() as u64, &mut c);
                    c
                })
            } else {
                None
            };
            let periodic: Option<Vec<u8>> = if st == WireStructure::Auto {
                detect_float_period(v).map(|p| {
                    let mut c = vec![T_FLOATS_PERIODIC];
                    write_uvarint(p as u64, &mut c);
                    write_uvarint(v.len() as u64, &mut c);
                    for &x in &v[..p] {
                        c.extend_from_slice(&x.to_le_bytes());
                    }
                    c
                })
            } else {
                None
            };
            if let Some(c) = [sparse, xor, geometric, periodic].into_iter().flatten().min_by_key(Vec::len) {
                // The memcpy column is `1` (tag) + body; keep the candidate only if it is smaller.
                if c.len() < floats_memcpy_body_len(v.len()) + 1 {
                    out.extend_from_slice(&c);
                    return Ok(());
                }
            }
            out.push(T_FLOATS);
            write_uvarint(v.len() as u64, out);
            // Direct memory transfer: a float buffer's little-endian bytes ARE the wire
            // bytes, so the whole array is one `memcpy`. (Big-endian byte-swaps.)
            #[cfg(target_endian = "little")]
            {
                // SAFETY: reading an `&[f64]` as `&[u8]` of the same byte length.
                let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(&v[..])) };
                out.extend_from_slice(bytes);
            }
            #[cfg(target_endian = "big")]
            {
                out.reserve(v.len() * 8);
                for &f in v {
                    out.extend_from_slice(&f.to_le_bytes());
                }
            }
        }
        ListRepr::Bools(v) => {
            // Under the `Auto` per-column menu, run the bool generator bake-off (periodic / RLE / bit-
            // pack, smallest wins); otherwise the plain bit-packed array, byte-identical to before.
            if structure() == WireStructure::Auto {
                emit_best_bool_column(v, out);
            } else {
                bool_bitpack(v, out);
            }
        }
        ListRepr::Strings { data, ends, .. } => {
            // Under the `Auto` per-column menu, try the dictionary form (categorical labels) and
            // ship the smaller; otherwise the plain flat array.
            if structure() == WireStructure::Auto {
                emit_best_string_column(data, ends, out);
            } else {
                write_string_array_from_ends(out, data, ends);
            }
        }
        // A columnar struct store: the schema (inline, or by reference when a schema
        // cache is active), then each in-memory column streamed straight out.
        ListRepr::Structs { type_name, field_names, columns } => {
            let n = columns.first().map_or(0, |c| c.len());
            if struct_view_on() {
                // Random-access record-list view: O(1) (row, field) reads. `field_names` is
                // canonically sorted (from `struct_schema`) and `columns[fi]` is its column.
                // With the fixed numeric dial (`indexed fast`) over all-fixed-width columns, the
                // FIXED-stride view drops the offset tables (smaller + pure-arithmetic O(1));
                // otherwise the columnar offset-table view, written straight from the typed
                // columns (no per-cell `RuntimeValue`), byte-identical to the boxed `get` path.
                if numerics() == WireNumerics::Fixed
                    && emit_structs_view_fixed(type_name, field_names, columns, out).is_some()
                {
                    return Ok(());
                }
                return emit_structs_view_columnar(type_name, field_names, columns, out);
            }
            emit_struct_list(type_name, field_names, n, out, |out| {
                for col in columns {
                    encode_list_repr(col, out)?;
                }
                Ok(())
            })?;
        }
        // A columnar enum store (tagged union): the type once, a constructor
        // dictionary with arities, the per-row constructor-index column, then the
        // dense per-constructor argument columns. Nullary enums emit just the
        // dictionary + index column. `ranks` are recomputed on decode (not sent).
        ListRepr::Inductives { inductive_type, ctor_dict, ctors, ranks: _, arg_cols } => {
            out.push(T_INDUCTIVES);
            write_str(inductive_type, out);
            write_uvarint(ctor_dict.len() as u64, out);
            for (c, name) in ctor_dict.iter().enumerate() {
                write_str(name, out);
                write_uvarint(arg_cols[c].len() as u64, out); // arity
            }
            let idx: Vec<i64> = ctors.iter().map(|&c| c as i64).collect();
            encode_list_repr(&ListRepr::Ints(idx), out)?;
            for cols in arg_cols {
                for col in cols {
                    encode_list_repr(col, out)?;
                }
            }
        }
        // A received lazy view being re-sent: materialize its rows/elements once, then encode
        // through the normal path (re-columnarizes / re-views per the active dial).
        ListRepr::WireStructs { .. } | ListRepr::WireColumn { .. } => {
            let materialized = ListRepr::from_values(repr.to_values());
            return encode_list_repr(&materialized, out);
        }
        ListRepr::Boxed(v) => {
            if !v.is_empty() && v.iter().all(|x| matches!(x, RuntimeValue::Text(_))) {
                // ALL strings (e.g. a string-literal list). Under the `Auto` per-column menu, run the
                // FULL string-column menu — flat / dictionary / template — the same one the columnar
                // `Strings` repr gets (so categorical labels dictionary and `item_0…item_n` collapses
                // to a template); otherwise the plain flat array, byte-identical to the boxed path.
                let mut sdata = Vec::new();
                let mut sends = Vec::with_capacity(v.len());
                for x in v {
                    if let RuntimeValue::Text(s) = x {
                        sdata.extend_from_slice(s.as_bytes());
                        sends.push(sdata.len() as u32);
                    }
                }
                if structure() == WireStructure::Auto {
                    emit_best_string_column(&sdata, &sends, out);
                } else {
                    write_string_array_from_ends(out, &sdata, &sends);
                }
            } else if let Some((type_name, field_names)) = struct_schema(v) {
                if struct_view_on() {
                    // Random-access record-list view: O(1) (row, field) reads. `struct_schema`
                    // returns sorted field names, matching the columnar `Structs` repr above.
                    return emit_structs_view(&type_name, &field_names, v.len(), out, |row, fi| {
                        match &v[row] {
                            RuntimeValue::Struct(sv) => sv.fields.get(&field_names[fi]).cloned().unwrap(),
                            _ => unreachable!("struct_schema guaranteed all-struct"),
                        }
                    });
                }
                // A homogeneous struct list (stored boxed) packs COLUMNAR via the same
                // schema-inline-or-by-reference path as the in-memory `Structs` repr.
                emit_struct_list(&type_name, &field_names, v.len(), out, |out| {
                    for fname in &field_names {
                        let column: Vec<RuntimeValue> = v
                            .iter()
                            .map(|s| match s {
                                RuntimeValue::Struct(sv) => sv.fields.get(fname).cloned().unwrap(),
                                _ => unreachable!("struct_schema guaranteed all-struct"),
                            })
                            .collect();
                        encode_list_repr(&ListRepr::from_values(column), out)?;
                    }
                    Ok(())
                })?;
            } else if let Some(ind) = ListRepr::build_inductives(v) {
                // A homogeneous enum list (stored boxed) packs columnar via the same
                // tagged-union encoding as the in-memory `Inductives` repr.
                encode_list_repr(&ind, out)?;
            } else {
                // A mixed list keeps per-element tags.
                out.push(T_LIST);
                write_uvarint(v.len() as u64, out);
                for x in v {
                    native_encode(x, out)?;
                }
            }
        }
    }
    Ok(())
}

/// Encode a `Money` to the wire: its Decimal amount (sign + LE coefficient + scale) then the
/// ISO-4217 code. `#[inline(never)]` so its locals stay out of the recursive `native_encode` frame.
#[inline(never)]
fn encode_money(m: &logicaffeine_base::Money, out: &mut Vec<u8>) {
    out.push(T_MONEY);
    let (negative, magnitude, scale) = m.amount.to_le_bytes();
    out.push(negative as u8);
    write_uvarint(magnitude.len() as u64, out);
    out.extend_from_slice(&magnitude);
    write_uvarint(scale as u64, out);
    let code = m.currency.code.as_bytes();
    write_uvarint(code.len() as u64, out);
    out.extend_from_slice(code);
}

/// Decode a `Money` from the wire (inverse of [`encode_money`]). `#[inline(never)]` so its locals
/// stay out of the recursive `native_decode` frame.
#[inline(never)]
fn decode_money(buf: &[u8], pos: &mut usize) -> Option<RuntimeValue> {
    let negative = *buf.get(*pos)? != 0;
    *pos += 1;
    let len = read_uvarint(buf, pos)? as usize;
    let bytes = buf.get(*pos..pos.checked_add(len)?)?;
    *pos += len;
    let scale = u32::try_from(read_uvarint(buf, pos)?).ok()?;
    let amount = logicaffeine_base::Decimal::from_le_bytes(negative, bytes, scale);
    let clen = read_uvarint(buf, pos)? as usize;
    let cbytes = buf.get(*pos..pos.checked_add(clen)?)?;
    *pos += clen;
    let code = std::str::from_utf8(cbytes).ok()?;
    let currency = logicaffeine_base::money::currency::by_code(code)
        .unwrap_or(logicaffeine_base::Currency { code: "XXX", scale: 0 });
    Some(RuntimeValue::Money(Rc::new(logicaffeine_base::Money { amount, currency })))
}

/// A UUID is a fixed 16 big-endian bytes — no length prefix needed.
fn encode_uuid(u: &logicaffeine_base::Uuid, out: &mut Vec<u8>) {
    out.push(T_UUID);
    out.extend_from_slice(u.as_bytes());
}

/// Decode a `Uuid` from the wire (inverse of [`encode_uuid`]): the next 16 bytes verbatim.
fn decode_uuid(buf: &[u8], pos: &mut usize) -> Option<RuntimeValue> {
    let bytes = buf.get(*pos..pos.checked_add(16)?)?;
    *pos += 16;
    let mut arr = [0u8; 16];
    arr.copy_from_slice(bytes);
    Some(RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::from_bytes(arr))))
}

/// Write a value as tagged varint bytes. `Err` for a non-portable value (with the
/// same messages the JSON path produces), caught at the exact offending node.
fn native_encode(v: &RuntimeValue, out: &mut Vec<u8>) -> Result<(), String> {
    let _depth = DepthGuard::enter()?;
    // Rc-dedup (G8): a shared subtree's repeat occurrences ship as a tiny backref; its first writes a
    // `T_SHARED_DEF` header here and then falls through to encode the value normally. No-op when the
    // dedup knob is off or the value isn't shared, so the default wire is byte-unchanged.
    if dedup_encode_prefix(v, out) {
        return Ok(());
    }
    match v {
        RuntimeValue::Nothing => out.push(T_NOTHING),
        RuntimeValue::Bool(false) => out.push(T_FALSE),
        RuntimeValue::Bool(true) => out.push(T_TRUE),
        RuntimeValue::Int(n) => {
            out.push(T_INT);
            write_uvarint(zigzag(*n), out);
        }
        RuntimeValue::Word(w) => {
            out.push(T_WORD);
            out.push(w.width() as u8);
            write_uvarint(w.to_u64(), out);
        }
        RuntimeValue::Lanes(_) => {
            return Err("a SIMD lane vector is a transient compute value, not a wire type".to_string());
        }
        // An out-of-i64 integer: ship sign + length + little-endian magnitude bytes —
        // exact (no base conversion), the typed alternative to a lossy JSON number.
        RuntimeValue::BigInt(b) => {
            out.push(T_BIGINT);
            let (negative, magnitude) = b.to_le_bytes();
            out.push(negative as u8);
            write_uvarint(magnitude.len() as u64, out);
            out.extend_from_slice(&magnitude);
        }
        // An exact fraction: signed numerator then positive denominator, each as
        // sign?+length+little-endian magnitude — `1/3` survives where a JSON number rounds.
        RuntimeValue::Rational(r) => {
            out.push(T_RATIONAL);
            let (num_negative, num_magnitude) = r.numerator().to_le_bytes();
            out.push(num_negative as u8);
            write_uvarint(num_magnitude.len() as u64, out);
            out.extend_from_slice(&num_magnitude);
            let (_den_sign, den_magnitude) = r.denominator().to_le_bytes();
            write_uvarint(den_magnitude.len() as u64, out);
            out.extend_from_slice(&den_magnitude);
        }
        // An exact base-10 fixed-point (money): sign + coefficient magnitude + base-10
        // scale — `19.99` survives bit-exact, scale and all (no lossy f64 round-trip).
        RuntimeValue::Decimal(d) => {
            out.push(T_DECIMAL);
            let (negative, magnitude, scale) = d.to_le_bytes();
            out.push(negative as u8);
            write_uvarint(magnitude.len() as u64, out);
            out.extend_from_slice(&magnitude);
            write_uvarint(scale as u64, out);
        }
        // Money: its Decimal amount, then the ISO-4217 currency code. Body lives in an
        // `#[inline(never)]` helper so its locals do not enlarge this RECURSIVE encoder's stack
        // frame (the codec recurses per nesting level; a fat frame overflows deep-but-finite values).
        RuntimeValue::Money(m) => encode_money(m, out),
        RuntimeValue::Uuid(u) => encode_uuid(u, out),
        // An exact complex number: its real then imaginary part, each shipped as a rational
        // (sign + numerator len+LE + denominator len+LE). `i·i = −1` survives bit-exact.
        RuntimeValue::Complex(c) => {
            out.push(T_COMPLEX);
            for part in [c.re(), c.im()] {
                let (neg, num) = part.numerator().to_le_bytes();
                out.push(neg as u8);
                write_uvarint(num.len() as u64, out);
                out.extend_from_slice(&num);
                let (_den_sign, den) = part.denominator().to_le_bytes();
                write_uvarint(den.len() as u64, out);
                out.extend_from_slice(&den);
            }
        }
        // A ℤ/nℤ element: its non-negative residue then its modulus, each len-prefixed LE.
        RuntimeValue::Modular(m) => {
            out.push(T_MODULAR);
            let (_, value) = m.value().to_le_bytes();
            write_uvarint(value.len() as u64, out);
            out.extend_from_slice(&value);
            let (_, modulus) = m.modulus().to_le_bytes();
            write_uvarint(modulus.len() as u64, out);
            out.extend_from_slice(&modulus);
        }
        RuntimeValue::Float(f) => {
            out.push(T_FLOAT);
            out.extend_from_slice(&f.to_le_bytes());
        }
        RuntimeValue::Char(c) => {
            out.push(T_CHAR);
            write_uvarint(*c as u64, out);
        }
        RuntimeValue::Text(s) => {
            out.push(T_TEXT);
            write_str(s, out);
        }
        RuntimeValue::Duration(n) => {
            out.push(T_DURATION);
            write_uvarint(zigzag(*n), out);
        }
        RuntimeValue::Date(n) => {
            out.push(T_DATE);
            write_uvarint(zigzag(*n as i64), out);
        }
        RuntimeValue::Moment(n) => {
            out.push(T_MOMENT);
            write_uvarint(zigzag(*n), out);
        }
        RuntimeValue::Span { months, days } => {
            out.push(T_SPAN);
            write_uvarint(zigzag(*months as i64), out);
            write_uvarint(zigzag(*days as i64), out);
        }
        RuntimeValue::Time(n) => {
            out.push(T_TIME);
            write_uvarint(zigzag(*n), out);
        }
        RuntimeValue::Peer(topic) => {
            out.push(T_PEER);
            write_str(topic, out);
        }
        RuntimeValue::List(items) => encode_list_repr(&items.borrow(), out)?,
        RuntimeValue::Tuple(items) => {
            out.push(T_TUPLE);
            write_uvarint(items.len() as u64, out);
            for x in items.iter() {
                native_encode(x, out)?;
            }
        }
        RuntimeValue::Set(items) => {
            let b = items.borrow();
            if !b.is_empty() && b.iter().all(|x| matches!(x, RuntimeValue::Int(_))) {
                // EXTREME SYMMETRY BREAKING: a homogeneous int set, sorted to its canonical
                // representative, is a strictly-MONOTONE column — the ideal input to the G5 int menu.
                // A consecutive set {1..n} collapses to T_INTS_AFFINE (base+stride+count, NO data); a
                // clustered set to delta+RLE. Same members in any order → byte-identical wire.
                let mut ints: Vec<i64> = b
                    .iter()
                    .map(|x| if let RuntimeValue::Int(n) = x { *n } else { unreachable!() })
                    .collect();
                ints.sort_unstable();
                ints.dedup();
                out.push(T_SET_INTS);
                emit_best_int_column(&ints, out);
            } else if !b.is_empty() && b.iter().all(|x| matches!(x, RuntimeValue::Text(_))) {
                // EXTREME SYMMETRY BREAKING (strings): sort to canonical order, then FRONT-CODE each
                // member as (shared-prefix-length-with-the-previous, suffix). Sorted similar strings
                // share long prefixes — "apple"/"apply"/"apricot" or "user_1".."user_999" or URL paths
                // — so only the deltas go on the wire. Same set in any order → byte-identical.
                let mut strs: Vec<String> = b
                    .iter()
                    .map(|x| if let RuntimeValue::Text(t) = x { (**t).clone() } else { unreachable!() })
                    .collect();
                strs.sort_unstable();
                strs.dedup();
                out.push(T_SET_STRINGS);
                write_uvarint(strs.len() as u64, out);
                let mut prev: &str = "";
                for s in &strs {
                    // common char-prefix length in BYTES (char-boundary-safe by construction).
                    let common: usize = s
                        .chars()
                        .zip(prev.chars())
                        .take_while(|(a, b)| a == b)
                        .map(|(a, _)| a.len_utf8())
                        .sum();
                    write_uvarint(common as u64, out);
                    write_str(&s[common..], out);
                    prev = s;
                }
            } else {
                // SYMMETRY BREAKING ON THE WIRE (general case). A Set is ORDER-INVARIANT — the decoder
                // rebuilds a Set, so element order carries NO information. Encode each element, then
                // sort by encoded bytes to ship the CANONICAL representative. Same set in any insertion
                // order → BYTE-IDENTICAL wire (content-addressing / dedup / cached keying / FEC hold).
                // Mirrors the canonical `T_MAP` (sort-by-encoded-key) / `T_STRUCT` (sort-by-field-name).
                out.push(T_SET);
                let mut encoded: Vec<Vec<u8>> = Vec::with_capacity(b.len());
                for x in b.iter() {
                    let mut xb = Vec::new();
                    native_encode(x, &mut xb)?;
                    encoded.push(xb);
                }
                encoded.sort_unstable();
                write_uvarint(encoded.len() as u64, out);
                for xb in &encoded {
                    out.extend_from_slice(xb);
                }
            }
        }
        RuntimeValue::Map(m) => {
            let b = m.borrow();
            if !b.is_empty() && b.keys().all(|k| matches!(k, RuntimeValue::Int(_))) {
                // SYMMETRY BREAKING (int-keyed map → struct-of-arrays). Sort entries by NUMERIC key
                // (canonical — insertion order carries nothing), then ship TWO columns: the keys as a
                // best int column (G5 menu: a {0..n} key range collapses to affine base+stride+count,
                // NO per-key data), and the values as a best-encoded LIST (composing the entire column
                // menu — int/float/string/struct columns). An affine int→int map crushes BOTH sides.
                let mut pairs: Vec<(i64, RuntimeValue)> = b
                    .iter()
                    .map(|(k, v)| match k {
                        RuntimeValue::Int(n) => (*n, v.clone()),
                        _ => unreachable!("all keys verified Int"),
                    })
                    .collect();
                pairs.sort_by_key(|(k, _)| *k);
                let keys: Vec<i64> = pairs.iter().map(|(k, _)| *k).collect();
                let vals: Vec<RuntimeValue> = pairs.into_iter().map(|(_, v)| v).collect();
                out.push(T_MAP_INTKEY);
                emit_best_int_column(&keys, out);
                // The value column crushes UNCONDITIONALLY (like the keys + the int-set), independent
                // of the structure dial: a 1-byte value-kind selects an int column (the int→int crush)
                // or the general per-value fallback. (kind 2+ — string/struct value columns — is a
                // future additive refinement, never a re-encoding of existing kinds.)
                if vals.iter().all(|v| matches!(v, RuntimeValue::Int(_))) {
                    out.push(1u8);
                    let int_vals: Vec<i64> = vals
                        .iter()
                        .map(|v| match v {
                            RuntimeValue::Int(n) => *n,
                            _ => unreachable!("all values verified Int"),
                        })
                        .collect();
                    emit_best_int_column(&int_vals, out);
                } else if vals.iter().all(|v| matches!(v, RuntimeValue::Text(_))) {
                    // String value column, FRONT-CODED in key order: each value ships as the shared-
                    // prefix length with the PREVIOUS value + the suffix. Sequential keys → sequential
                    // values (URLs / paths / ids — the database-table case) share long prefixes, so only
                    // the deltas go on the wire. Never worse than kind 0 (an unshared value just ships
                    // common=0 + the string). Char-boundary-safe by construction.
                    out.push(2u8);
                    write_uvarint(vals.len() as u64, out);
                    let mut prev: &str = "";
                    for v in &vals {
                        let s = match v {
                            RuntimeValue::Text(t) => t.as_str(),
                            _ => unreachable!("all values verified Text"),
                        };
                        let common: usize = s
                            .chars()
                            .zip(prev.chars())
                            .take_while(|(a, b)| a == b)
                            .map(|(a, _)| a.len_utf8())
                            .sum();
                        write_uvarint(common as u64, out);
                        write_str(&s[common..], out);
                        prev = s;
                    }
                } else if vals.iter().all(|v| matches!(v, RuntimeValue::Struct(_))) {
                    // Struct value column → the database-table crush. A homogeneous struct list packs
                    // COLUMNAR (the schema / field names ship ONCE, then one best-encoded column per
                    // field — the id column compresses, a bool column bit-packs) via the existing
                    // struct-list encoder; heterogeneous structs degrade to a tagged list. Either way it
                    // decodes back to the same value sequence.
                    out.push(3u8);
                    let vlist = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vals.clone()))));
                    native_encode(&vlist, out)?;
                } else {
                    out.push(0u8);
                    write_uvarint(vals.len() as u64, out);
                    for v in &vals {
                        native_encode(v, out)?;
                    }
                }
            } else {
                out.push(T_MAP);
                // Canonical (general): encode each entry, then sort by encoded key.
                let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(b.len());
                for (k, val) in b.iter() {
                    let mut kb = Vec::new();
                    native_encode(k, &mut kb)?;
                    let mut vb = Vec::new();
                    native_encode(val, &mut vb)?;
                    entries.push((kb, vb));
                }
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                write_uvarint(entries.len() as u64, out);
                for (kb, vb) in entries {
                    out.extend_from_slice(&kb);
                    out.extend_from_slice(&vb);
                }
            }
        }
        RuntimeValue::Struct(s) => {
            // Canonical field order (by name) is both the schema identity and the order
            // values are written in. With no cache this stays the self-describing
            // `T_STRUCT` (field names inline — byte-identical to before). With a cache
            // the schema is sent once and later structs of the same shape ship values
            // only (no field-name strings) — the cross-message win for lone structs.
            let mut fields: Vec<(&String, &RuntimeValue)> = s.fields.iter().collect();
            fields.sort_by(|a, b| a.0.cmp(b.0));
            let field_names: Vec<String> = fields.iter().map(|(n, _)| (*n).clone()).collect();
            if struct_view_on() {
                // Offset-table view: a per-field byte-offset table precedes the values so a
                // `WireView` jumps to ANY field in O(1) (Cap'n Proto-class random access),
                // never parsing the others. Offsets are backpatched after the values land.
                out.push(T_STRUCT_VIEW);
                write_str(&s.type_name, out);
                write_uvarint(fields.len() as u64, out);
                for (name, _) in &fields {
                    write_str(name, out);
                }
                let table_pos = out.len();
                out.resize(table_pos + fields.len() * 4, 0);
                let values_start = out.len();
                let mut offsets: Vec<u32> = Vec::with_capacity(fields.len());
                for (_, val) in &fields {
                    offsets.push((out.len() - values_start) as u32);
                    native_encode(val, out)?;
                }
                for (i, off) in offsets.iter().enumerate() {
                    out[table_pos + i * 4..table_pos + i * 4 + 4].copy_from_slice(&off.to_le_bytes());
                }
            } else if let Some(id) = type_registry_id(&s.type_name, &field_names) {
                // Both ends share this type's definition (the program-derived registry):
                // ship its small id and the values only — the type/field NAMES never go on
                // the wire. The default-on win that beats raw varint on Logos↔Logos.
                out.push(T_STRUCT_TID);
                write_uvarint(id as u64, out);
                for (_, val) in &fields {
                    native_encode(val, out)?;
                }
            } else {
            match schema_send(&s.type_name, &field_names) {
                SchemaEmit::Inline => {
                    out.push(T_STRUCT);
                    write_str(&s.type_name, out);
                    write_uvarint(fields.len() as u64, out);
                    for (name, val) in &fields {
                        write_str(name, out);
                        native_encode(val, out)?;
                    }
                }
                SchemaEmit::SeqDef(id) => {
                    out.push(T_STRUCT_DEF);
                    write_uvarint(id as u64, out);
                    write_struct_schema(&s.type_name, &field_names, out);
                    for (_, val) in &fields {
                        native_encode(val, out)?;
                    }
                }
                SchemaEmit::SeqRef(id) => {
                    out.push(T_STRUCT_REF);
                    write_uvarint(id as u64, out);
                    for (_, val) in &fields {
                        native_encode(val, out)?;
                    }
                }
                SchemaEmit::CaDef => {
                    out.push(T_STRUCT_CDEF);
                    write_struct_schema(&s.type_name, &field_names, out);
                    for (_, val) in &fields {
                        native_encode(val, out)?;
                    }
                }
                SchemaEmit::CaRef(fp) => {
                    out.push(T_STRUCT_CREF);
                    out.extend_from_slice(&fp.to_le_bytes());
                    for (_, val) in &fields {
                        native_encode(val, out)?;
                    }
                }
            }
            }
        }
        RuntimeValue::Inductive(ind) => {
            if let Some((enum_id, ctor_idx)) = type_registry_enum_id(&ind.inductive_type, &ind.constructor) {
                // Shared registry knows this enum: ship its id + the constructor index,
                // names elided (the receiver's ordered constructor list resolves it).
                out.push(T_INDUCTIVE_TID);
                write_uvarint(enum_id as u64, out);
                write_uvarint(ctor_idx as u64, out);
                write_uvarint(ind.args.len() as u64, out);
                for a in &ind.args {
                    native_encode(a, out)?;
                }
            } else {
                out.push(T_INDUCTIVE);
                write_str(&ind.inductive_type, out);
                write_str(&ind.constructor, out);
                write_uvarint(ind.args.len() as u64, out);
                for a in &ind.args {
                    native_encode(a, out)?;
                }
            }
        }
        RuntimeValue::Chan(_) | RuntimeValue::TaskHandle(_) => {
            return Err("a channel or task handle cannot be sent over the network".to_string());
        }
        RuntimeValue::Crdt(_) => {
            return Err("a CRDT value is shared via Merge/Sync, not sent inline".to_string());
        }
        RuntimeValue::Function(f) => {
            // Only a SHIPPED pure function (lowered to a sandboxed generator) crosses the
            // wire — an ordinary closure (an arena AST body) still cannot, since the receiver
            // never compiled it. A `generated` function ships its arity + the generator tree.
            match &f.generated {
                Some(expr) => {
                    out.push(T_FUNC);
                    write_uvarint(f.param_names.len() as u64, out);
                    serialize_gen(expr, out);
                }
                None => return Err("a Function cannot be sent over the network".to_string()),
            }
        }
        // A dimensioned quantity: its SI magnitude (rational), its dimension (10 exponent pairs as
        // zigzag varints), then the display unit symbol (len + UTF-8) — reconstructed exactly.
        RuntimeValue::Quantity(qv) => {
            out.push(T_QUANTITY);
            let (num_negative, num_magnitude) = qv.q.magnitude_si().numerator().to_le_bytes();
            out.push(num_negative as u8);
            write_uvarint(num_magnitude.len() as u64, out);
            out.extend_from_slice(&num_magnitude);
            let (_den_sign, den_magnitude) = qv.q.magnitude_si().denominator().to_le_bytes();
            write_uvarint(den_magnitude.len() as u64, out);
            out.extend_from_slice(&den_magnitude);
            let dim = qv.q.dimension();
            for d in logicaffeine_base::BaseDim::ALL {
                let e = dim.exponent(d);
                write_uvarint(zigzag(e.numerator() as i64), out);
                write_uvarint(zigzag(e.denominator() as i64), out);
            }
            let sym = qv.unit.symbol.as_bytes();
            write_uvarint(sym.len() as u64, out);
            out.extend_from_slice(sym);
        }
    }
    Ok(())
}

/// A cap on a length prefix's pre-allocation, so a corrupt huge count can't ask
/// for gigabytes up front; the actual reads still bound-check every element.
const PREALLOC_CAP: usize = 4096;

/// Reject a decoded element count that exceeds the receiver's `max_elements` budget BEFORE any
/// `count`-sized materialization. This is the gate the byte budget cannot provide: a generator column
/// (`T_INTS_AFFINE` / `T_INTS_POLY` / `T_GEN`) is a ~10-byte descriptor that expands to `count`
/// elements, so a crafted `count = 10^9` is a few bytes on the wire but gigabytes in memory — the
/// small-message-huge-output DoS. Returns the count as `usize` when within budget, else `None`.
fn bounded_count(n: u64) -> Option<usize> {
    let n = n as usize;
    (n <= receive_limits().max_elements).then_some(n)
}

fn native_decode(buf: &[u8], pos: &mut usize) -> Option<RuntimeValue> {
    // Bound recursion depth FIRST — a crafted deeply-nested message is refused here (clean `None`),
    // never recursed into a stack overflow. Unwinds via `Drop` as the `?`s propagate back up.
    let _depth = DecodeDepthGuard::enter()?;
    let tag = *buf.get(*pos)?;
    *pos += 1;
    // Rc-dedup (G8): a backref resolves to the EXACT `Rc` first decoded under that id (sharing
    // preserved); a def decodes the value once and registers it. A dangling ref → clean `None`.
    if tag == T_SHARED_REF {
        let id = read_uvarint(buf, pos)?;
        return DECODE_MEMO.with(|c| c.borrow().get(&id).cloned());
    }
    if tag == T_SHARED_DEF {
        let id = read_uvarint(buf, pos)?;
        let v = native_decode(buf, pos)?;
        DECODE_MEMO.with(|c| c.borrow_mut().insert(id, v.clone()));
        return Some(v);
    }
    Some(match tag {
        T_NOTHING => RuntimeValue::Nothing,
        T_FALSE => RuntimeValue::Bool(false),
        T_TRUE => RuntimeValue::Bool(true),
        T_INT => RuntimeValue::Int(unzigzag(read_uvarint(buf, pos)?)),
        T_WORD => {
            let width = *buf.get(*pos)? as u32;
            *pos += 1;
            let bits = read_uvarint(buf, pos)?;
            RuntimeValue::Word(logicaffeine_base::WordVal::from_u64(width, bits)?)
        }
        T_BIGINT => {
            let negative = *buf.get(*pos)? != 0;
            *pos += 1;
            let len = read_uvarint(buf, pos)? as usize;
            let bytes = buf.get(*pos..pos.checked_add(len)?)?;
            *pos += len;
            RuntimeValue::from_bigint(logicaffeine_base::BigInt::from_le_bytes(negative, bytes))
        }
        T_RATIONAL => {
            let num_negative = *buf.get(*pos)? != 0;
            *pos += 1;
            let num_len = read_uvarint(buf, pos)? as usize;
            let num_bytes = buf.get(*pos..pos.checked_add(num_len)?)?;
            *pos += num_len;
            let num = logicaffeine_base::BigInt::from_le_bytes(num_negative, num_bytes);
            let den_len = read_uvarint(buf, pos)? as usize;
            let den_bytes = buf.get(*pos..pos.checked_add(den_len)?)?;
            *pos += den_len;
            let den = logicaffeine_base::BigInt::from_le_bytes(false, den_bytes);
            // A zero/garbage denominator is rejected (None) rather than panicking.
            RuntimeValue::from_rational(logicaffeine_base::Rational::new(num, den)?)
        }
        T_DECIMAL => {
            let negative = *buf.get(*pos)? != 0;
            *pos += 1;
            let len = read_uvarint(buf, pos)? as usize;
            let bytes = buf.get(*pos..pos.checked_add(len)?)?;
            *pos += len;
            let scale = u32::try_from(read_uvarint(buf, pos)?).ok()?;
            // Decimal does NOT downsize on a whole value (the scale is meaning), so build
            // the variant directly rather than through a downsizing chokepoint.
            RuntimeValue::Decimal(Rc::new(logicaffeine_base::Decimal::from_le_bytes(
                negative, bytes, scale,
            )))
        }
        // Body in an `#[inline(never)]` helper to keep this recursive decoder's frame small.
        T_MONEY => decode_money(buf, pos)?,
        T_UUID => decode_uuid(buf, pos)?,
        T_COMPLEX => {
            // Read the real part (a rational), then the imaginary part (a rational).
            let re_neg = *buf.get(*pos)? != 0;
            *pos += 1;
            let re_nlen = read_uvarint(buf, pos)? as usize;
            let re_nb = buf.get(*pos..pos.checked_add(re_nlen)?)?;
            *pos += re_nlen;
            let re_num = logicaffeine_base::BigInt::from_le_bytes(re_neg, re_nb);
            let re_dlen = read_uvarint(buf, pos)? as usize;
            let re_db = buf.get(*pos..pos.checked_add(re_dlen)?)?;
            *pos += re_dlen;
            let re = logicaffeine_base::Rational::new(re_num, logicaffeine_base::BigInt::from_le_bytes(false, re_db))?;
            let im_neg = *buf.get(*pos)? != 0;
            *pos += 1;
            let im_nlen = read_uvarint(buf, pos)? as usize;
            let im_nb = buf.get(*pos..pos.checked_add(im_nlen)?)?;
            *pos += im_nlen;
            let im_num = logicaffeine_base::BigInt::from_le_bytes(im_neg, im_nb);
            let im_dlen = read_uvarint(buf, pos)? as usize;
            let im_db = buf.get(*pos..pos.checked_add(im_dlen)?)?;
            *pos += im_dlen;
            let im = logicaffeine_base::Rational::new(im_num, logicaffeine_base::BigInt::from_le_bytes(false, im_db))?;
            RuntimeValue::Complex(Rc::new(logicaffeine_base::Complex::new(re, im)))
        }
        T_MODULAR => {
            let vlen = read_uvarint(buf, pos)? as usize;
            let vb = buf.get(*pos..pos.checked_add(vlen)?)?;
            *pos += vlen;
            let v = logicaffeine_base::BigInt::from_le_bytes(false, vb);
            let nlen = read_uvarint(buf, pos)? as usize;
            let nb = buf.get(*pos..pos.checked_add(nlen)?)?;
            *pos += nlen;
            let n = logicaffeine_base::BigInt::from_le_bytes(false, nb);
            RuntimeValue::Modular(Rc::new(logicaffeine_base::Modular::new(v, n)?))
        }
        T_QUANTITY => {
            // SI magnitude (rational): sign + numerator (len+LE) + denominator (len+LE).
            let num_neg = *buf.get(*pos)? != 0;
            *pos += 1;
            let nlen = read_uvarint(buf, pos)? as usize;
            let nb = buf.get(*pos..pos.checked_add(nlen)?)?;
            *pos += nlen;
            let num = logicaffeine_base::BigInt::from_le_bytes(num_neg, nb);
            let dlen = read_uvarint(buf, pos)? as usize;
            let db = buf.get(*pos..pos.checked_add(dlen)?)?;
            *pos += dlen;
            let magnitude =
                logicaffeine_base::Rational::new(num, logicaffeine_base::BigInt::from_le_bytes(false, db))?;
            // Dimension: 10 exponent (numerator, denominator) pairs as zig-zag varints.
            let mut exps = [logicaffeine_base::Exp::ZERO; logicaffeine_base::BaseDim::COUNT];
            for slot in exps.iter_mut() {
                let en = unzigzag(read_uvarint(buf, pos)?) as i32;
                let ed = unzigzag(read_uvarint(buf, pos)?) as i32;
                *slot = logicaffeine_base::Exp::new(en, if ed == 0 { 1 } else { ed });
            }
            let dim = logicaffeine_base::Dimension::from_exps(exps);
            // Display unit symbol — resolve by name, else fall back to the SI/dimension display.
            let sym = read_str(buf, pos)?;
            let unit = logicaffeine_base::quantity::units::by_name(&sym)
                .filter(|u| u.dimension == dim)
                .unwrap_or_else(|| {
                    logicaffeine_base::Unit::linear("", dim, logicaffeine_base::Rational::one())
                });
            RuntimeValue::Quantity(Rc::new(crate::interpreter::QuantityValue {
                q: logicaffeine_base::Quantity::si(magnitude, dim),
                unit,
            }))
        }
        T_FLOAT => {
            let b: [u8; 8] = buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?;
            *pos += 8;
            RuntimeValue::Float(f64::from_le_bytes(b))
        }
        T_CHAR => RuntimeValue::Char(char::from_u32(u32::try_from(read_uvarint(buf, pos)?).ok()?)?),
        T_TEXT => RuntimeValue::Text(Rc::new(read_str(buf, pos)?)),
        T_DURATION => RuntimeValue::Duration(unzigzag(read_uvarint(buf, pos)?)),
        T_DATE => RuntimeValue::Date(i32::try_from(unzigzag(read_uvarint(buf, pos)?)).ok()?),
        T_MOMENT => RuntimeValue::Moment(unzigzag(read_uvarint(buf, pos)?)),
        T_SPAN => RuntimeValue::Span {
            months: i32::try_from(unzigzag(read_uvarint(buf, pos)?)).ok()?,
            days: i32::try_from(unzigzag(read_uvarint(buf, pos)?)).ok()?,
        },
        T_TIME => RuntimeValue::Time(unzigzag(read_uvarint(buf, pos)?)),
        T_PEER => RuntimeValue::Peer(Rc::new(read_str(buf, pos)?)),
        // A mixed list rebuilds as `Boxed` directly — never re-specialized, so a
        // round-trip is byte-stable (only genuinely-homogeneous lists are packed).
        T_LIST => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(read_seq(buf, pos)?)))),
        T_INTS
        | T_INTS_AFFINE
        | T_INTS_GEOMETRIC
        | T_INTS_PERIODIC
        | T_INTS_SPARSE
        | T_INTS_POLY
        | T_GEN
        | T_BYTES
        | T_INTS_DELTA
        | T_INTS_DOD
        | T_INTS_FOR
        | T_INTS_RLE
        | T_INTS_DICT
        | describe::T_INTS_LRECUR
        | describe::T_INTS_LFSR
        | describe::T_INTS_FCSR => {
            // The Auto column menu decodes through the shared engine (single source of the format);
            // the receiver's element budget is threaded in so the DoS gate is unchanged.
            let v = describe::decode_int_column_body(tag, buf, pos, receive_limits().max_elements, 0)?;
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))))
        }
        T_INTS_GV => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(gv_decode_dispatch(buf, pos)?)))),
        // A shipped callable function: read the arity + the bounded generator tree, rebuild a
        // self-contained closure the receiver can invoke (the body is the sandboxed generator).
        T_FUNC => {
            if !receive_limits().accept_computed {
                return None;
            }
            let arity = read_uvarint(buf, pos)? as usize;
            if arity > 16 {
                return None;
            }
            let mut budget = MAX_GEN_NODES;
            let expr = deserialize_gen(buf, pos, &mut budget, 0)?;
            let param_names: Vec<logicaffeine_base::Symbol> =
                (0..arity).map(logicaffeine_base::Symbol::from_index).collect();
            RuntimeValue::Function(Box::new(ClosureValue {
                body_index: usize::MAX,
                captured_env: std::collections::HashMap::default(),
                param_names,
                generated: Some(Rc::new(expr)),
            }))
        }
        T_INTS_FIXED => {
            let n = read_uvarint(buf, pos)? as usize;
            let nbytes = n.checked_mul(8)?;
            let raw = buf.get(*pos..pos.checked_add(nbytes)?)?;
            *pos += nbytes;
            // Direct memory transfer: copy the little-endian bytes straight into a
            // fresh `Vec<i64>` (one `memcpy`), then take ownership.
            #[cfg(target_endian = "little")]
            let v: Vec<i64> = {
                let mut v = Vec::<i64>::with_capacity(n);
                // SAFETY: `raw` is exactly `n * 8` bytes; copy into the capacity of
                // a properly-aligned `Vec<i64>`, then set its length.
                unsafe {
                    std::ptr::copy_nonoverlapping(raw.as_ptr(), v.as_mut_ptr().cast::<u8>(), nbytes);
                    v.set_len(n);
                }
                v
            };
            #[cfg(target_endian = "big")]
            let v: Vec<i64> = raw
                .chunks_exact(8)
                .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
                .collect();
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))))
        }
        // 8-byte-aligned i64 blob: skip the pad, then the same memcpy as T_INTS_FIXED.
        T_INTS_ALIGNED => {
            let n = read_uvarint(buf, pos)? as usize;
            let pad = *buf.get(*pos)? as usize;
            *pos += 1 + pad;
            let nbytes = n.checked_mul(8)?;
            let raw = buf.get(*pos..pos.checked_add(nbytes)?)?;
            *pos += nbytes;
            #[cfg(target_endian = "little")]
            let v: Vec<i64> = {
                let mut v = Vec::<i64>::with_capacity(n);
                // SAFETY: `raw` is exactly `n * 8` bytes; copy into a properly-aligned
                // `Vec<i64>`'s capacity, then set its length.
                unsafe {
                    std::ptr::copy_nonoverlapping(raw.as_ptr(), v.as_mut_ptr().cast::<u8>(), nbytes);
                    v.set_len(n);
                }
                v
            };
            #[cfg(target_endian = "big")]
            let v: Vec<i64> = raw.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap())).collect();
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))))
        }
        T_FLOATS => {
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let nbytes = n.checked_mul(8)?;
            let raw = buf.get(*pos..pos.checked_add(nbytes)?)?;
            *pos += nbytes;
            // Direct memory transfer: copy the little-endian bytes straight into a
            // fresh `Vec<f64>` (one `memcpy`), then take ownership — no per-element
            // decode. Bounds were just checked, so the copy reads exactly `nbytes`.
            #[cfg(target_endian = "little")]
            let v: Vec<f64> = {
                let mut v = Vec::<f64>::with_capacity(n);
                // SAFETY: `raw` has exactly `n * 8` bytes; we copy them into the
                // capacity of a properly-aligned `Vec<f64>`, then set its length.
                unsafe {
                    std::ptr::copy_nonoverlapping(raw.as_ptr(), v.as_mut_ptr().cast::<u8>(), nbytes);
                    v.set_len(n);
                }
                v
            };
            #[cfg(target_endian = "big")]
            let v: Vec<f64> = raw
                .chunks_exact(8)
                .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                .collect();
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
        }
        // Constant float column: one f64 + count → fill.
        T_FLOATS_CONST => {
            let bits = u64::from_le_bytes(buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?);
            *pos += 8;
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            v.resize(n, f64::from_bits(bits));
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
        }
        // Affine float column: reconstruct `base + i·stride` with the SAME f64 ops the encoder
        // verified bit-exact against, so it round-trips perfectly.
        T_FLOATS_AFFINE => {
            let base = f64::from_le_bytes(buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?);
            *pos += 8;
            let stride = f64::from_le_bytes(buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?);
            *pos += 8;
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            for i in 0..n {
                v.push(base + (i as f64) * stride);
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
        }
        // Sparse float column: fill with the dominant value, then patch the delta-indexed outliers.
        T_FLOATS_SPARSE => {
            let dom = f64::from_bits(u64::from_le_bytes(buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?));
            *pos += 8;
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let num_exc = bounded_count(read_uvarint(buf, pos)?)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            v.resize(n, dom);
            let mut idx = 0usize;
            for _ in 0..num_exc {
                idx = idx.checked_add(read_uvarint(buf, pos)? as usize)?;
                let bits = u64::from_le_bytes(buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?);
                *pos += 8;
                *v.get_mut(idx)? = f64::from_bits(bits);
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
        }
        // Cyclic float column: read the period block, then emit `block[i % p]` for `i in 0..n`.
        T_FLOATS_PERIODIC => {
            let p = bounded_count(read_uvarint(buf, pos)?)?;
            if p == 0 {
                return None;
            }
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let mut block = Vec::with_capacity(p.min(PREALLOC_CAP));
            for _ in 0..p {
                let bits = u64::from_le_bytes(buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?);
                *pos += 8;
                block.push(f64::from_bits(bits));
            }
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            for i in 0..n {
                v.push(block[i % p]);
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
        }
        // Geometric float column: replay `base · ratio^i` with the SAME `cur *= ratio` accumulation
        // the encoder verified bit-exact against.
        T_FLOATS_GEOMETRIC => {
            let base = f64::from_le_bytes(buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?);
            *pos += 8;
            let ratio = f64::from_le_bytes(buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?);
            *pos += 8;
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            let mut cur = base;
            for _ in 0..n {
                v.push(cur);
                cur *= ratio;
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
        }
        // 8-byte-aligned f64 blob: skip the pad, then the same memcpy as T_FLOATS.
        T_FLOATS_ALIGNED => {
            let n = read_uvarint(buf, pos)? as usize;
            let pad = *buf.get(*pos)? as usize;
            *pos += 1 + pad;
            let nbytes = n.checked_mul(8)?;
            let raw = buf.get(*pos..pos.checked_add(nbytes)?)?;
            *pos += nbytes;
            #[cfg(target_endian = "little")]
            let v: Vec<f64> = {
                let mut v = Vec::<f64>::with_capacity(n);
                // SAFETY: `raw` is exactly `n * 8` bytes; copy into a properly-aligned
                // `Vec<f64>`'s capacity, then set its length.
                unsafe {
                    std::ptr::copy_nonoverlapping(raw.as_ptr(), v.as_mut_ptr().cast::<u8>(), nbytes);
                    v.set_len(n);
                }
                v
            };
            #[cfg(target_endian = "big")]
            let v: Vec<f64> = raw.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect();
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
        }
        // Lossless XOR-delta float array: undo the running XOR and rebuild each f64
        // from its exact bits (NaN/Inf/±0/subnormals preserved).
        T_FLOATS_XOR => {
            let n = read_uvarint(buf, pos)? as usize;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            let mut prev = 0u64;
            for _ in 0..n {
                let bits = read_uvarint(buf, pos)? ^ prev;
                prev = bits;
                v.push(f64::from_bits(bits));
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
        }
        T_BOOLS => {
            let n = read_uvarint(buf, pos)? as usize;
            let nbytes = n.div_ceil(8);
            let bits = buf.get(*pos..pos.checked_add(nbytes)?)?;
            *pos += nbytes;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            for i in 0..n {
                v.push((bits[i / 8] >> (i % 8)) & 1 == 1);
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools(v))))
        }
        // Cyclic bool column: read the p-bit block, replay `block[i % p]` for `i in 0..n`.
        T_BOOLS_PERIODIC => {
            let p = bounded_count(read_uvarint(buf, pos)?)?;
            if p == 0 {
                return None;
            }
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let block_bytes = p.div_ceil(8);
            let raw = buf.get(*pos..pos.checked_add(block_bytes)?)?;
            *pos += block_bytes;
            let block: Vec<bool> = (0..p).map(|i| (raw[i / 8] >> (i % 8)) & 1 == 1).collect();
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            for i in 0..n {
                v.push(block[i % p]);
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools(v))))
        }
        // Run-length bool column: alternate from the first value, emitting each run length in turn.
        T_BOOLS_RLE => {
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let first = *buf.get(*pos)? != 0;
            *pos += 1;
            let nruns = bounded_count(read_uvarint(buf, pos)?)?;
            let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
            let mut cur = first;
            for _ in 0..nruns {
                let runlen = bounded_count(read_uvarint(buf, pos)?)?;
                // The runs must reconstruct EXACTLY `n` elements — a corrupt over-run is refused.
                if v.len().checked_add(runlen)? > n {
                    return None;
                }
                v.resize(v.len() + runlen, cur);
                cur = !cur;
            }
            if v.len() != n {
                return None;
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools(v))))
        }
        T_STRINGS => {
            let n = read_uvarint(buf, pos)? as usize;
            let mut ends = Vec::with_capacity(n.min(PREALLOC_CAP));
            let mut total: u64 = 0;
            for _ in 0..n {
                total = total.checked_add(read_uvarint(buf, pos)?)?;
                ends.push(u32::try_from(total).ok()?);
            }
            let total = usize::try_from(total).ok()?;
            let raw = buf.get(*pos..pos.checked_add(total)?)?;
            *pos += total;
            // The concatenation of valid UTF-8 strings is itself valid UTF-8;
            // validate the whole blob once (SIMD-fast in std) so element access can
            // trust it, and reject corrupt data here. One bulk copy into the buffer.
            if std::str::from_utf8(raw).is_err() {
                return None;
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::strings(raw.to_vec(), ends))))
        }
        // Templated string column: rebuild `prefix + (base + i·stride) + suffix` for `i in 0..n`.
        T_STRINGS_TEMPLATE => {
            let plen = bounded_count(read_uvarint(buf, pos)?)?;
            let prefix = buf.get(*pos..pos.checked_add(plen)?)?.to_vec();
            *pos += plen;
            let slen = bounded_count(read_uvarint(buf, pos)?)?;
            let suffix = buf.get(*pos..pos.checked_add(slen)?)?.to_vec();
            *pos += slen;
            let base = unzigzag(read_uvarint(buf, pos)?);
            let stride = unzigzag(read_uvarint(buf, pos)?);
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            // The affixes came from valid strings; a corrupt frame that isn't UTF-8 is refused.
            if std::str::from_utf8(&prefix).is_err() || std::str::from_utf8(&suffix).is_err() {
                return None;
            }
            let mut data = Vec::new();
            let mut ends = Vec::with_capacity(n.min(PREALLOC_CAP));
            for i in 0..n {
                let num = base.wrapping_add((i as i64).wrapping_mul(stride));
                data.extend_from_slice(&prefix);
                data.extend_from_slice(num.to_string().as_bytes());
                data.extend_from_slice(&suffix);
                ends.push(u32::try_from(data.len()).ok()?);
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::strings(data, ends))))
        }
        // Front-coded string column: rebuild each string as `prev[..common] + suffix`.
        T_STRINGS_FRONT => {
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let mut data: Vec<u8> = Vec::new();
            let mut ends = Vec::with_capacity(n.min(PREALLOC_CAP));
            let mut prev: Vec<u8> = Vec::new();
            for _ in 0..n {
                let common = bounded_count(read_uvarint(buf, pos)?)?;
                let suffix_len = bounded_count(read_uvarint(buf, pos)?)?;
                // Can't share more prefix than the previous string has — a corrupt frame is refused.
                if common > prev.len() {
                    return None;
                }
                let suffix = buf.get(*pos..pos.checked_add(suffix_len)?)?;
                *pos += suffix_len;
                let mut s = Vec::with_capacity(common.checked_add(suffix_len)?);
                s.extend_from_slice(&prev[..common]);
                s.extend_from_slice(suffix);
                data.extend_from_slice(&s);
                ends.push(u32::try_from(data.len()).ok()?);
                prev = s;
            }
            // The concatenation must be valid UTF-8 (front-coding cuts on char boundaries; a corrupt
            // `common`/suffix that lands mid-char is rejected here rather than producing bad strings).
            if std::str::from_utf8(&data).is_err() {
                return None;
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::strings(data, ends))))
        }
        // Affixed string column: rebuild each string as `prefix + middle_i + suffix`.
        T_STRINGS_AFFIX => {
            let plen = bounded_count(read_uvarint(buf, pos)?)?;
            let prefix = buf.get(*pos..pos.checked_add(plen)?)?.to_vec();
            *pos += plen;
            let slen = bounded_count(read_uvarint(buf, pos)?)?;
            let suffix = buf.get(*pos..pos.checked_add(slen)?)?.to_vec();
            *pos += slen;
            let n = bounded_count(read_uvarint(buf, pos)?)?;
            let mut data: Vec<u8> = Vec::new();
            let mut ends = Vec::with_capacity(n.min(PREALLOC_CAP));
            for _ in 0..n {
                let mid_len = bounded_count(read_uvarint(buf, pos)?)?;
                let mid = buf.get(*pos..pos.checked_add(mid_len)?)?;
                *pos += mid_len;
                data.extend_from_slice(&prefix);
                data.extend_from_slice(mid);
                data.extend_from_slice(&suffix);
                ends.push(u32::try_from(data.len()).ok()?);
            }
            // Affixes + middles all came from valid strings; reject a corrupt non-UTF-8 frame.
            if std::str::from_utf8(&data).is_err() {
                return None;
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::strings(data, ends))))
        }
        // Dictionary string column: the distinct strings once, then a bit-packed index per row.
        // Reconstruct the flat `Strings` buffer by replaying each row's dictionary entry.
        T_STRINGS_DICT => {
            let d = read_uvarint(buf, pos)? as usize;
            let mut dict: Vec<&[u8]> = Vec::with_capacity(d.min(PREALLOC_CAP));
            for _ in 0..d {
                let len = read_uvarint(buf, pos)? as usize;
                let s = buf.get(*pos..pos.checked_add(len)?)?;
                if std::str::from_utf8(s).is_err() {
                    return None;
                }
                dict.push(s);
                *pos += len;
            }
            let n = read_uvarint(buf, pos)? as usize;
            let iw = *buf.get(*pos)?;
            *pos += 1;
            if iw > 64 {
                return None;
            }
            let mut out_data: Vec<u8> = Vec::new();
            let mut out_ends: Vec<u32> = Vec::with_capacity(n.min(PREALLOC_CAP));
            let mut push = |s: &[u8], out_data: &mut Vec<u8>, out_ends: &mut Vec<u32>| {
                out_data.extend_from_slice(s);
                out_ends.push(out_data.len() as u32);
            };
            if iw == 0 {
                if n > 0 {
                    let s = *dict.first()?;
                    for _ in 0..n {
                        push(s, &mut out_data, &mut out_ends);
                    }
                }
            } else {
                let nbytes = n.checked_mul(iw as usize)?.div_ceil(8);
                let bytes = buf.get(*pos..pos.checked_add(nbytes)?)?;
                *pos += nbytes;
                for ix in bitunpack(bytes, n, iw)? {
                    push(*dict.get(ix as usize)?, &mut out_data, &mut out_ends);
                }
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::strings(out_data, out_ends))))
        }
        T_TUPLE => RuntimeValue::Tuple(Rc::new(read_seq(buf, pos)?)),
        T_SET => RuntimeValue::Set(Rc::new(RefCell::new(read_seq(buf, pos)?))),
        T_SET_INTS => {
            // The body is a best-encoded int column (any `T_INTS_*` form); decode it as a list —
            // the recovered ints ARE the set's (already canonical-sorted) members.
            let ints = match native_decode(buf, pos)? {
                RuntimeValue::List(l) => l.borrow().to_values(),
                _ => return None,
            };
            RuntimeValue::Set(Rc::new(RefCell::new(ints)))
        }
        T_SET_STRINGS => {
            // Front-coded: reconstruct each member from `common` bytes of the previous + the suffix.
            let n = read_uvarint(buf, pos)? as usize;
            let mut items = Vec::with_capacity(n);
            let mut prev = String::new();
            for _ in 0..n {
                let common = read_uvarint(buf, pos)? as usize;
                if common > prev.len() || !prev.is_char_boundary(common) {
                    return None;
                }
                let suffix = read_str(buf, pos)?;
                let s = format!("{}{}", &prev[..common], suffix);
                items.push(RuntimeValue::Text(Rc::new(s.clone())));
                prev = s;
            }
            RuntimeValue::Set(Rc::new(RefCell::new(items)))
        }
        T_MAP => {
            let n = read_uvarint(buf, pos)?;
            let mut m = MapStorage::default();
            for _ in 0..n {
                let k = native_decode(buf, pos)?;
                let v = native_decode(buf, pos)?;
                m.insert(k, v);
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }
        T_MAP_INTKEY => {
            // The int key column, a value-kind byte, then the values. Re-pair positionally — keys were
            // written in numeric order, values in the corresponding order.
            let keys = match native_decode(buf, pos)? {
                RuntimeValue::List(l) => l.borrow().to_values(),
                _ => return None,
            };
            let kind = *buf.get(*pos)?;
            *pos += 1;
            let vals: Vec<RuntimeValue> = match kind {
                1 => match native_decode(buf, pos)? {
                    RuntimeValue::List(l) => l.borrow().to_values(),
                    _ => return None,
                },
                2 => {
                    // Front-coded string column: reconstruct each value from `common` bytes of the
                    // previous + the suffix (mirrors T_SET_STRINGS).
                    let n = read_uvarint(buf, pos)? as usize;
                    let mut vs = Vec::with_capacity(n.min(keys.len().saturating_add(1)));
                    let mut prev = String::new();
                    for _ in 0..n {
                        let common = read_uvarint(buf, pos)? as usize;
                        if common > prev.len() || !prev.is_char_boundary(common) {
                            return None;
                        }
                        let suffix = read_str(buf, pos)?;
                        let s = format!("{}{}", &prev[..common], suffix);
                        vs.push(RuntimeValue::Text(Rc::new(s.clone())));
                        prev = s;
                    }
                    vs
                }
                3 => match native_decode(buf, pos)? {
                    // Columnar struct value list (or a tagged list for heterogeneous structs).
                    RuntimeValue::List(l) => l.borrow().to_values(),
                    _ => return None,
                },
                0 => {
                    let n = read_uvarint(buf, pos)? as usize;
                    let mut vs = Vec::with_capacity(n.min(keys.len().saturating_add(1)));
                    for _ in 0..n {
                        vs.push(native_decode(buf, pos)?);
                    }
                    vs
                }
                _ => return None,
            };
            if keys.len() != vals.len() {
                return None;
            }
            let mut m = MapStorage::default();
            for (k, v) in keys.into_iter().zip(vals.into_iter()) {
                m.insert(k, v);
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }
        T_STRUCT => {
            let type_name = read_str(buf, pos)?;
            let n = read_uvarint(buf, pos)?;
            let mut fields = std::collections::HashMap::with_capacity((n as usize).min(PREALLOC_CAP));
            for _ in 0..n {
                let name = read_str(buf, pos)?;
                let val = native_decode(buf, pos)?;
                fields.insert(name, val);
            }
            RuntimeValue::Struct(Box::new(StructValue { type_name, fields }))
        }
        // Single-struct schema DEFINITION (sequential): id + schema inline (registered),
        // then values in field order. Self-decodable even without a cache.
        T_STRUCT_DEF => {
            let id = read_uvarint(buf, pos)? as u32;
            let (type_name, field_names) = read_struct_schema(buf, pos)?;
            if !schema_recv_register_seq(id, &type_name, &field_names) {
                return None; // out-of-order / conflicting schema definition
            }
            decode_struct_values(buf, pos, type_name, field_names)?
        }
        // Single-struct schema REFERENCE (sequential): id resolved against the cache,
        // then values only. `None` (clean) if the schema was never defined here.
        T_STRUCT_REF => {
            let id = read_uvarint(buf, pos)? as u32;
            let (type_name, field_names) = schema_recv_lookup_seq(id)?;
            decode_struct_values(buf, pos, type_name, field_names)?
        }
        // Single-struct schema DEFINITION (content-addressed): schema inline (its
        // fingerprint derived + registered), then values. A fingerprint that conflicts
        // with a different cached schema is rejected.
        T_STRUCT_CDEF => {
            let (type_name, field_names) = read_struct_schema(buf, pos)?;
            if !schema_recv_register_ca(&type_name, &field_names) {
                return None; // fingerprint collision with a different schema
            }
            decode_struct_values(buf, pos, type_name, field_names)?
        }
        // Single-struct schema REFERENCE (content-addressed): an 8-byte fingerprint,
        // then values. `None` (clean) if no definition for it was seen (reorder/loss).
        T_STRUCT_CREF => {
            let raw = buf.get(*pos..pos.checked_add(8)?)?;
            let fp = u64::from_le_bytes(raw.try_into().ok()?);
            *pos += 8;
            let (type_name, field_names) = schema_recv_lookup_ca(fp)?;
            decode_struct_values(buf, pos, type_name, field_names)?
        }
        // Type-id elided struct: resolve the id against the shared registry (the receiver
        // runs the same program), then read the values. Unknown id → None (clean fail).
        T_STRUCT_TID => {
            let id = read_uvarint(buf, pos)? as u32;
            let (type_name, field_names) = type_registry_schema(id)?;
            decode_struct_values(buf, pos, type_name, field_names)?
        }
        // Offset-table view struct: read the schema, SKIP the offset table (a full decode
        // reads the values sequentially; the table is only for `WireView` random access).
        T_STRUCT_VIEW => {
            let type_name = read_str(buf, pos)?;
            let n = read_uvarint(buf, pos)? as usize;
            let mut field_names = Vec::with_capacity(n.min(PREALLOC_CAP));
            for _ in 0..n {
                field_names.push(read_str(buf, pos)?);
            }
            *pos = pos.checked_add(n.checked_mul(4)?)?; // skip the offset table
            if *pos > buf.len() {
                return None;
            }
            decode_struct_values(buf, pos, type_name, field_names)?
        }
        // Random-access record-list view: shared schema, then the row table (skipped), then
        // each row's field table (skipped) + values. We zip the rows back into structs and
        // re-columnarize via `from_values`, so re-encoding is byte-stable with the original.
        T_STRUCTS_VIEW => {
            let type_name = read_str(buf, pos)?;
            let f = read_uvarint(buf, pos)? as usize;
            let mut field_names = Vec::with_capacity(f.min(PREALLOC_CAP));
            for _ in 0..f {
                field_names.push(read_str(buf, pos)?);
            }
            let n = read_uvarint(buf, pos)? as usize;
            *pos = pos.checked_add(n.checked_mul(4)?)?; // skip the row offset table
            if *pos > buf.len() {
                return None;
            }
            let mut rows = Vec::with_capacity(n.min(PREALLOC_CAP));
            for _ in 0..n {
                *pos = pos.checked_add(f.checked_mul(4)?)?; // skip this row's field offset table
                if *pos > buf.len() {
                    return None;
                }
                rows.push(decode_struct_values(buf, pos, type_name.clone(), field_names.clone())?);
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))))
        }
        // FIXED-stride record-list view: read the shared schema + the F kind bytes, then read
        // each row's cells by arithmetic (no offset tables), resolving FK_TEXT against the blob.
        T_STRUCTS_FVIEW => {
            let type_name = read_str(buf, pos)?;
            let f = read_uvarint(buf, pos)? as usize;
            let mut field_names = Vec::with_capacity(f.min(PREALLOC_CAP));
            for _ in 0..f {
                field_names.push(read_str(buf, pos)?);
            }
            let kinds = buf.get(*pos..pos.checked_add(f)?)?.to_vec();
            *pos += f;
            let n = read_uvarint(buf, pos)? as usize;
            let (offsets, stride) = fview_layout(&kinds);
            let rows_start = *pos;
            let rows_len = n.checked_mul(stride)?;
            let rows_bytes = buf.get(rows_start..rows_start.checked_add(rows_len)?)?;
            *pos = rows_start.checked_add(rows_len)?;
            let blob_len = read_uvarint(buf, pos)? as usize;
            let blob = buf.get(*pos..pos.checked_add(blob_len)?)?;
            *pos = pos.checked_add(blob_len)?;
            // Decode each field STRAIGHT into a typed column (no per-row HashMap, no `from_values`
            // re-scan) — the decode twin of the columnar encode. `field_names` is already the
            // canonical sorted order, so this is the same `Structs` repr `from_values` would build.
            let mut columns: Vec<ListRepr> = Vec::with_capacity(f);
            for j in 0..f {
                let off = offsets[j];
                let col = match kinds[j] {
                    FK_INT => {
                        let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
                        for r in 0..n {
                            let base = r.checked_mul(stride)?.checked_add(off)?;
                            v.push(i64::from_le_bytes(rows_bytes.get(base..base.checked_add(8)?)?.try_into().ok()?));
                        }
                        ListRepr::Ints(v)
                    }
                    FK_FLOAT => {
                        let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
                        for r in 0..n {
                            let base = r.checked_mul(stride)?.checked_add(off)?;
                            v.push(f64::from_le_bytes(rows_bytes.get(base..base.checked_add(8)?)?.try_into().ok()?));
                        }
                        ListRepr::Floats(v)
                    }
                    FK_BOOL => {
                        let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
                        for r in 0..n {
                            let base = r.checked_mul(stride)?.checked_add(off)?;
                            v.push(*rows_bytes.get(base)? != 0);
                        }
                        ListRepr::Bools(v)
                    }
                    FK_TEXT => {
                        let mut data: Vec<u8> = Vec::new();
                        let mut ends: Vec<u32> = Vec::with_capacity(n.min(PREALLOC_CAP));
                        for r in 0..n {
                            let base = r.checked_mul(stride)?.checked_add(off)?;
                            let toff =
                                u32::from_le_bytes(rows_bytes.get(base..base.checked_add(4)?)?.try_into().ok()?) as usize;
                            let tlen = u32::from_le_bytes(
                                rows_bytes.get(base.checked_add(4)?..base.checked_add(8)?)?.try_into().ok()?,
                            ) as usize;
                            data.extend_from_slice(blob.get(toff..toff.checked_add(tlen)?)?);
                            ends.push(data.len() as u32);
                        }
                        ListRepr::strings(data, ends)
                    }
                    _ => return None,
                };
                columns.push(col);
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Structs { type_name, field_names, columns })))
        }
        T_INDUCTIVE => {
            let inductive_type = read_str(buf, pos)?;
            let constructor = read_str(buf, pos)?;
            let args = read_seq(buf, pos)?;
            RuntimeValue::Inductive(Box::new(InductiveValue { inductive_type, constructor, args }))
        }
        // Type-id elided enum: resolve the enum id + constructor index against the shared
        // registry, then read the args. Unknown id / out-of-range index → None (clean).
        T_INDUCTIVE_TID => {
            let enum_id = read_uvarint(buf, pos)? as u32;
            let ctor_idx = read_uvarint(buf, pos)? as usize;
            let (inductive_type, ctors) = type_registry_enum_schema(enum_id)?;
            let constructor = ctors.get(ctor_idx)?.clone();
            let args = read_seq(buf, pos)?;
            RuntimeValue::Inductive(Box::new(InductiveValue { inductive_type, constructor, args }))
        }
        // Columnar struct list: schema once, then one self-describing packed column
        // per field; we read the columns and zip them back into N structs. Decoding
        // to `Boxed` keeps re-encoding byte-stable (the schema re-derives identically).
        // Self-describing struct list: schema inline, then the columns.
        T_STRUCTS => {
            let (type_name, field_names) = read_struct_schema(buf, pos)?;
            decode_struct_columns(buf, pos, type_name, field_names)?
        }
        // Shared-registry struct list: resolve the type id (names elided), then columns.
        // `None` (clean) if the id can't be resolved against this decoder's registry.
        T_STRUCTS_TID => {
            let id = read_uvarint(buf, pos)? as u32;
            let (type_name, field_names) = type_registry_schema(id)?;
            decode_struct_columns(buf, pos, type_name, field_names)?
        }
        // Sequential schema DEFINITION: id + schema inline (registered), then columns.
        // Self-decodable even without a cache.
        T_STRUCTS_DEF => {
            let id = read_uvarint(buf, pos)? as u32;
            let (type_name, field_names) = read_struct_schema(buf, pos)?;
            if !schema_recv_register_seq(id, &type_name, &field_names) {
                return None; // out-of-order / conflicting schema definition
            }
            decode_struct_columns(buf, pos, type_name, field_names)?
        }
        // Sequential schema REFERENCE: id resolved against the cache, then columns.
        // `None` (clean) if the schema was never defined to this decoder.
        T_STRUCTS_REF => {
            let id = read_uvarint(buf, pos)? as u32;
            let (type_name, field_names) = schema_recv_lookup_seq(id)?;
            decode_struct_columns(buf, pos, type_name, field_names)?
        }
        // Content-addressed schema DEFINITION: schema inline (the fingerprint is
        // derived from it), registered under its fingerprint, then columns. A
        // fingerprint that conflicts with a different cached schema is rejected.
        T_STRUCTS_CDEF => {
            let (type_name, field_names) = read_struct_schema(buf, pos)?;
            if !schema_recv_register_ca(&type_name, &field_names) {
                return None; // fingerprint collision with a different schema
            }
            decode_struct_columns(buf, pos, type_name, field_names)?
        }
        // Content-addressed schema REFERENCE: an 8-byte fingerprint, then columns.
        // `None` (clean) if no definition for that fingerprint was seen (reorder/loss).
        T_STRUCTS_CREF => {
            let raw = buf.get(*pos..pos.checked_add(8)?)?;
            let fp = u64::from_le_bytes(raw.try_into().ok()?);
            *pos += 8;
            let (type_name, field_names) = schema_recv_lookup_ca(fp)?;
            decode_struct_columns(buf, pos, type_name, field_names)?
        }
        // Columnar enum list (tagged union): type once + constructor dictionary with
        // arities + the per-row index column + dense per-constructor arg columns.
        // Decodes STRAIGHT into the columnar `Inductives` repr (no per-row rebuild);
        // `ranks` are recomputed here. Nullary enums have all-zero arities.
        T_INDUCTIVES => {
            let inductive_type = read_str(buf, pos)?;
            let d = read_uvarint(buf, pos)? as usize;
            let mut ctor_dict = Vec::with_capacity(d.min(PREALLOC_CAP));
            let mut arities = Vec::with_capacity(d.min(PREALLOC_CAP));
            for _ in 0..d {
                ctor_dict.push(read_str(buf, pos)?);
                arities.push(read_uvarint(buf, pos)? as usize);
            }
            // The constructor-index column → `ctors: Vec<u32>` (each index < d).
            let idx = match native_decode(buf, pos)? {
                RuntimeValue::List(l) => Rc::try_unwrap(l).map(RefCell::into_inner).unwrap_or_else(|rc| rc.borrow().clone()),
                _ => return None,
            };
            let mut ctors: Vec<u32> = Vec::with_capacity(idx.len().min(PREALLOC_CAP));
            for v in idx.to_values() {
                match v {
                    RuntimeValue::Int(i) if i >= 0 && (i as usize) < d => ctors.push(i as u32),
                    _ => return None,
                }
            }
            // The dense per-constructor argument columns.
            let mut arg_cols: Vec<Vec<ListRepr>> = Vec::with_capacity(d.min(PREALLOC_CAP));
            for &arity in &arities {
                let mut cols = Vec::with_capacity(arity.min(PREALLOC_CAP));
                for _ in 0..arity {
                    let col = match native_decode(buf, pos)? {
                        RuntimeValue::List(l) => Rc::try_unwrap(l).map(RefCell::into_inner).unwrap_or_else(|rc| rc.borrow().clone()),
                        _ => return None,
                    };
                    cols.push(col);
                }
                arg_cols.push(cols);
            }
            // Recompute ranks and validate each constructor's column lengths.
            let mut counts = vec![0u32; d];
            let mut ranks = Vec::with_capacity(ctors.len());
            for &c in &ctors {
                ranks.push(counts[c as usize]);
                counts[c as usize] += 1;
            }
            for c in 0..d {
                if arg_cols[c].iter().any(|col| col.len() != counts[c] as usize) {
                    return None; // a column whose length disagrees with the constructor count
                }
            }
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Inductives {
                inductive_type,
                ctor_dict,
                ctors,
                ranks,
                arg_cols,
            })))
        }
        _ => return None,
    })
}

fn read_seq(buf: &[u8], pos: &mut usize) -> Option<Vec<RuntimeValue>> {
    let n = bounded_count(read_uvarint(buf, pos)?)?;
    let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
    for _ in 0..n {
        v.push(native_decode(buf, pos)?);
    }
    Some(v)
}

/// Read a struct schema (type name + field names) from the wire.
fn read_struct_schema(buf: &[u8], pos: &mut usize) -> Option<(String, Vec<String>)> {
    let type_name = read_str(buf, pos)?;
    let k = read_uvarint(buf, pos)? as usize;
    let mut field_names = Vec::with_capacity(k.min(PREALLOC_CAP));
    for _ in 0..k {
        field_names.push(read_str(buf, pos)?);
    }
    Some((type_name, field_names))
}

/// Rebuild a single struct from a known schema (the schema-dictionary forms): one
/// value per field, in the schema's canonical field order, zipped back by name.
fn decode_struct_values(
    buf: &[u8],
    pos: &mut usize,
    type_name: String,
    field_names: Vec<String>,
) -> Option<RuntimeValue> {
    let mut fields = std::collections::HashMap::with_capacity(field_names.len().min(PREALLOC_CAP));
    for name in field_names {
        let val = native_decode(buf, pos)?;
        fields.insert(name, val);
    }
    Some(RuntimeValue::Struct(Box::new(StructValue { type_name, fields })))
}

/// Read a struct list's body (the row count + one self-describing column per field)
/// given its schema, decoding STRAIGHT into the columnar `Structs` repr (no per-row
/// rebuild). A zero-field schema — which our encoder never emits — falls back to
/// boxed empty structs so the row count survives.
fn decode_struct_columns(
    buf: &[u8],
    pos: &mut usize,
    type_name: String,
    field_names: Vec<String>,
) -> Option<RuntimeValue> {
    let k = field_names.len();
    let n = read_uvarint(buf, pos)? as usize;
    let mut columns: Vec<ListRepr> = Vec::with_capacity(k.min(PREALLOC_CAP));
    for _ in 0..k {
        // Keep each decoded column AS its packed `ListRepr` (no `to_values`):
        // `native_decode` just minted this `Rc`, so `try_unwrap` takes the inner
        // buffer without cloning.
        let col = match native_decode(buf, pos)? {
            RuntimeValue::List(l) => Rc::try_unwrap(l).map(RefCell::into_inner).unwrap_or_else(|rc| rc.borrow().clone()),
            _ => return None,
        };
        if col.len() != n {
            return None; // a column whose length disagrees with the row count
        }
        columns.push(col);
    }
    Some(if columns.is_empty() {
        let rows = (0..n)
            .map(|_| {
                RuntimeValue::Struct(Box::new(StructValue {
                    type_name: type_name.clone(),
                    fields: std::collections::HashMap::new(),
                }))
            })
            .collect();
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(rows))))
    } else {
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Structs { type_name, field_names, columns })))
    })
}

fn header(codec: WireCodec, integrity: WireIntegrity, compression: WireCompression) -> u8 {
    let c = if matches!(codec, WireCodec::Json) { H_JSON } else { 0 };
    let i = if matches!(integrity, WireIntegrity::Checked) { H_CHECKED } else { 0 };
    let z = if compression == WireCompression::None { 0 } else { H_COMPRESSED | (compression_id(compression) << 2) };
    c | i | z
}

/// Wrap a body in its frame: the header byte, then (for `Checked`) an 8-byte
/// FNV-1a checksum over the (possibly compressed) body, then the body.
fn frame(codec: WireCodec, integrity: WireIntegrity, compression: WireCompression, body: Vec<u8>) -> Vec<u8> {
    let h = header(codec, integrity, compression);
    match integrity {
        WireIntegrity::Raw => {
            let mut out = Vec::with_capacity(body.len() + 1);
            out.push(h);
            out.extend_from_slice(&body);
            out
        }
        WireIntegrity::Checked => {
            let mut out = Vec::with_capacity(body.len() + 9);
            out.push(h);
            out.extend_from_slice(&fnv1a_64(&body).to_le_bytes());
            out.extend_from_slice(&body);
            out
        }
    }
}

/// Strip the frame: return `(codec, compressed, body)`, verifying the checksum in
/// `Checked` mode. `None` on an unknown header, a short frame, or a checksum
/// mismatch. The checksum is verified BEFORE the caller inflates, so a corrupt
/// message never reaches the decompressor.
/// Decode the frame header into `(codec, compression, body)`. When `verify` is set and the
/// message carries a checksum, the body is FNV-validated (O(body)) — corruption → `None`.
/// A zero-copy view passes `verify = false` so opening a message is always O(1) (a checksum
/// hash would defeat random access; the view trusts the bytes, like Cap'n Proto / Arrow).
fn unframe_with(bytes: &[u8], verify: bool) -> Option<(WireCodec, WireCompression, &[u8])> {
    let (&h, rest) = bytes.split_first()?;
    if h & !H_KNOWN != 0 {
        return None; // an unknown format bit is set
    }
    let codec = if h & H_JSON != 0 { WireCodec::Json } else { WireCodec::Native };
    let compression = if h & H_COMPRESSED == 0 {
        WireCompression::None
    } else {
        match (h & H_CODEC) >> 2 {
            0 => WireCompression::Deflate,
            1 => WireCompression::Lz4,
            2 => WireCompression::Zstd,
            _ => return None, // a reserved codec id
        }
    };
    let body = if h & H_CHECKED != 0 {
        if rest.len() < 8 {
            return None;
        }
        let (sum, body) = rest.split_at(8);
        if verify {
            let expected = u64::from_le_bytes(sum.try_into().ok()?);
            if fnv1a_64(body) != expected {
                return None;
            }
        }
        body
    } else {
        rest
    };
    Some((codec, compression, body))
}

/// Decode the frame, validating the integrity checksum if present (the full-decode path).
fn unframe(bytes: &[u8]) -> Option<(WireCodec, WireCompression, &[u8])> {
    unframe_with(bytes, true)
}

/// FNV-1a, 64-bit — a small, fast, dependency-free checksum. Not cryptographic
/// (it detects corruption, not a forged message); a signing layer is separate. The
/// constants are part of the wire, so they must never change.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The process integrity default — `Checked`, unless `LOGOS_WIRE=raw` opts into
/// the bare fast path. Read once.
pub(crate) fn default_integrity() -> WireIntegrity {
    static MODE: std::sync::OnceLock<WireIntegrity> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("LOGOS_WIRE").ok().as_deref() {
        Some("raw") => WireIntegrity::Raw,
        _ => WireIntegrity::Checked,
    })
}

thread_local! {
    static INTEGRITY_OVERRIDE: std::cell::Cell<Option<WireIntegrity>> = const { std::cell::Cell::new(None) };
}

/// The latency↔safety dial: run `f` with the checksum on (`Checked`) or off (`Raw`),
/// overriding the process default for the duration. Scoped — never leaks. `Raw` is
/// the fastest path (the FNV checksum is the bulk of small-message encode cost);
/// `Checked` detects corruption. Pairs with `with_numerics`/`with_compression_codec`.
pub fn with_integrity<T>(integrity: WireIntegrity, f: impl FnOnce() -> T) -> T {
    let prev = INTEGRITY_OVERRIDE.with(|c| c.replace(Some(integrity)));
    let out = f();
    INTEGRITY_OVERRIDE.with(|c| c.set(prev));
    out
}

/// The integrity in force for a plain `message_to_wire`: a scoped [`with_integrity`]
/// override if set, else the process default.
fn current_integrity() -> WireIntegrity {
    INTEGRITY_OVERRIDE.with(std::cell::Cell::get).unwrap_or_else(default_integrity)
}

/// Varint-encoded bincode: small ints and lengths cost a byte or two, so an array
/// of small numbers is genuinely compact (vs. the fixed 8-byte ints of the default
/// config). Both peers run this same code, so the encoding is shared by construction.
fn wire_options() -> impl bincode::Options {
    bincode::DefaultOptions::new()
}

/// The canonical bytes of a wire value — used only to order map entries so the
/// encoding is independent of the source map's hash iteration order.
fn canon_bytes(w: &WireValue) -> Vec<u8> {
    use bincode::Options;
    wire_options().serialize(w).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::ClosureValue;
    use std::collections::HashMap;

    // ─────────────────────────────────────────────────────────────────────────────
    // describe_columns — the codec naming its own output (the "which dial won" surface).
    // ─────────────────────────────────────────────────────────────────────────────

    fn ints_list(v: Vec<i64>) -> RuntimeValue {
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))))
    }

    #[test]
    fn describe_columns_names_the_numeric_dials() {
        let ints = ints_list((0i64..64).map(|i| 3 + i * 7).collect());
        let enc = |num: WireNumerics| {
            with_structure(WireStructure::Off, || {
                with_numerics(num, || {
                    message_to_wire_with("", &ints, WireCodec::Native, WireIntegrity::Raw).unwrap()
                })
            })
        };
        assert_eq!(describe_columns(&enc(WireNumerics::Varint)), vec!["varint"]);
        assert_eq!(describe_columns(&enc(WireNumerics::Fixed)), vec!["fixed (memcpy)"]);
        assert_eq!(describe_columns(&enc(WireNumerics::GroupVarint)), vec!["group-varint"]);
    }

    #[test]
    fn describe_columns_names_the_affine_structure() {
        let ints = ints_list((0i64..64).map(|i| 5 + i * 3).collect());
        let bytes = with_structure(WireStructure::Affine, || {
            with_numerics(WireNumerics::Varint, || {
                message_to_wire_with("", &ints, WireCodec::Native, WireIntegrity::Raw).unwrap()
            })
        });
        assert_eq!(describe_columns(&bytes), vec!["affine (base,stride,n)"]);
    }

    #[test]
    fn describe_columns_names_the_float_dials() {
        // Slowly-varying floats: memcpy stays raw; xor-delta shrinks (the dial applies).
        let floats = floats_list((0..256).map(|i| 20.0 + i as f64 * 0.01).collect());
        let enc = |fl: WireFloats| {
            with_structure(WireStructure::Off, || {
                with_floats(fl, || {
                    message_to_wire_with("", &floats, WireCodec::Native, WireIntegrity::Raw).unwrap()
                })
            })
        };
        assert_eq!(describe_columns(&enc(WireFloats::Memcpy)), vec!["memcpy floats"]);
        assert_eq!(describe_columns(&enc(WireFloats::XorDelta)), vec!["xor-delta floats"]);
    }

    #[test]
    fn describe_columns_names_strings_and_bools() {
        let strings = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
            (0..24).map(|i| RuntimeValue::Text(Rc::new(format!("host-{i}-{}", i * 31 % 7)))).collect(),
        ))));
        let bools = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
            (0..40).map(|i| RuntimeValue::Bool(i * 5 % 3 == 0)).collect(),
        ))));
        let enc = |rv: &RuntimeValue| {
            with_structure(WireStructure::Off, || {
                message_to_wire_with("", rv, WireCodec::Native, WireIntegrity::Raw).unwrap()
            })
        };
        assert_eq!(describe_columns(&enc(&strings)), vec!["flat strings"]);
        assert_eq!(describe_columns(&enc(&bools)), vec!["bit-packed bools"]);
    }

    #[test]
    fn describe_columns_names_each_record_field() {
        let mut rows = Vec::new();
        for i in 0..32i64 {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(i * 3 + 1));
            f.insert("name".to_string(), RuntimeValue::Text(Rc::new(format!("node-{i}"))));
            f.insert("active".to_string(), RuntimeValue::Bool(i % 2 == 0));
            rows.push(RuntimeValue::Struct(Box::new(StructValue { type_name: "Record".to_string(), fields: f })));
        }
        let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
        let bytes = with_structure(WireStructure::Off, || {
            with_numerics(WireNumerics::Varint, || {
                message_to_wire_with("", &rv, WireCodec::Native, WireIntegrity::Raw).unwrap()
            })
        });
        // Field order is schema-defined; compare as a set so the test is order-robust.
        let cols = describe_columns(&bytes);
        let got: std::collections::BTreeSet<&str> = cols.iter().map(String::as_str).collect();
        let want: std::collections::BTreeSet<&str> =
            ["active: bit-packed bools", "id: varint", "name: flat strings"].into_iter().collect();
        assert_eq!(got, want);
    }

    #[test]
    fn column_tag_name_covers_the_structural_vocabulary() {
        // Every dial the codec can select must carry a plain-words name — none may ship as the
        // generic "value" fallback, or the benchmark card would hide which encoding actually won.
        for (tag, want) in [
            (T_INTS, "varint"),
            (T_INTS_FIXED, "fixed (memcpy)"),
            (T_INTS_GV, "group-varint"),
            (T_INTS_AFFINE, "affine (base,stride,n)"),
            (T_INTS_DELTA, "delta"),
            (T_INTS_DOD, "delta-of-delta"),
            (T_INTS_FOR, "FOR bit-pack"),
            (T_INTS_RLE, "run-length"),
            (T_INTS_DICT, "dictionary"),
            (T_FLOATS, "memcpy floats"),
            (T_FLOATS_XOR, "xor-delta floats"),
            (T_BOOLS, "bit-packed bools"),
            (T_STRINGS, "flat strings"),
        ] {
            assert_eq!(column_tag_name(tag), want, "tag {tag}");
            assert_ne!(column_tag_name(tag), "value", "tag {tag} must not be the generic fallback");
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Build-in-place columnar records — Cap'n Proto's home turf (zero-encode + zero-decode).
    // ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn build_in_place_record_reads_back_zero_copy() {
        // Build a 1000-row record straight into the wire layout from borrowed slices (no
        // RuntimeValue), then read any column in O(1) and ZERO-COPY — `Some(slice)`, never the
        // copy fallback. The dual zero-encode/zero-decode story end to end.
        let ids: Vec<i64> = (0..1000).collect();
        let xs: Vec<f64> = (0..1000).map(|i| i as f64 * 0.5).collect();
        let bytes = build_columnar_record(
            "node",
            "Sensor",
            &[("id", WireColumn::Ints(&ids)), ("x", WireColumn::Floats(&xs))],
        );
        let view = view_message(&bytes).expect("the built record opens as a view");
        let id_slice = view.struct_field("id").expect("id field").as_i64_slice().expect("zero-copy i64");
        let x_slice = view.struct_field("x").expect("x field").as_f64_slice().expect("zero-copy f64");
        assert_eq!(id_slice, &ids[..], "id column round-trips bit-exact, zero-copy");
        assert_eq!(x_slice, &xs[..], "x column round-trips bit-exact, zero-copy");
    }

    #[test]
    fn build_in_place_is_byte_identical_to_the_runtimevalue_path() {
        // The builder emits EXACTLY the canonical struct-view bytes the audited `RuntimeValue`
        // encode path produces — so it inherits every correctness property of that path for free,
        // while skipping the value materialization + second serialize pass.
        let a: Vec<i64> = vec![10, 20, 30, 40];
        let b: Vec<f64> = vec![1.5, 2.5, 3.5];
        let built = build_columnar_record(
            "p",
            "Rec",
            &[("alpha", WireColumn::Ints(&a)), ("beta", WireColumn::Floats(&b))],
        );

        let mut fields = HashMap::new();
        fields.insert("alpha".to_string(), RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(a.clone())))));
        fields.insert("beta".to_string(), RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(b.clone())))));
        let sv = RuntimeValue::Struct(Box::new(StructValue { type_name: "Rec".to_string(), fields }));
        let canonical = with_struct_view(true, || {
            message_to_wire_with("p", &sv, WireCodec::Native, current_integrity()).unwrap()
        });
        assert_eq!(built, canonical, "build-in-place must equal the canonical struct-view encode byte-for-byte");
    }

    #[test]
    fn build_in_place_record_full_decode_interop() {
        // A non-view receiver (a full `message_from_wire` decode) reconstructs the record too —
        // the build-in-place form is ordinary wire bytes, not a view-only dialect.
        let a: Vec<i64> = vec![7, 8, 9];
        let bytes = build_columnar_record("p", "R", &[("c", WireColumn::Ints(&a))]);
        let (from, val) = message_from_wire(&bytes).expect("full decode");
        assert_eq!(from, "p");
        match val {
            RuntimeValue::Struct(s) => {
                assert_eq!(s.type_name, "R");
                match s.fields.get("c").unwrap() {
                    RuntimeValue::List(l) => match &*l.borrow() {
                        ListRepr::Ints(v) => assert_eq!(v, &a),
                        other => panic!("expected Ints, got {other:?}"),
                    },
                    other => panic!("expected List, got {other:?}"),
                }
            }
            other => panic!("expected Struct, got {other:?}"),
        }
    }

    #[test]
    #[ignore = "build-in-place encode-parity measurement — run with --ignored --nocapture"]
    fn build_in_place_encode_is_at_capnp_parity() {
        // Honest measurement: Logos's column encode was ALREADY memcpy-fast (the aligned column is
        // one `extend_from_slice`), so build-in-place does NOT dramatically beat the existing path —
        // it MATCHES it (capnp parity on encode) while needing no intermediate `RuntimeValue`. The
        // comparison is fair: the value is pre-built once (the realistic "you already hold it" case),
        // so neither side pays a clone. The proven capnp *win* is the read side, not encode.
        use std::time::Instant;
        const ITERS: usize = 4000;
        let cols: Vec<Vec<i64>> = (0..8).map(|c| (0..256).map(|i| (c * 256 + i) as i64).collect()).collect();
        let names: Vec<String> = (0..8).map(|c| format!("col{c}")).collect();

        // Pre-build the RuntimeValue ONCE — the existing path then only serializes (no clone).
        let mut fields_map = HashMap::new();
        for (n, c) in names.iter().zip(&cols) {
            fields_map.insert(n.clone(), RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(c.clone())))));
        }
        let sv = RuntimeValue::Struct(Box::new(StructValue { type_name: "Batch".to_string(), fields: fields_map }));

        let t = Instant::now();
        for _ in 0..ITERS {
            let fields: Vec<(&str, WireColumn)> =
                names.iter().zip(&cols).map(|(n, c)| (n.as_str(), WireColumn::Ints(c))).collect();
            std::hint::black_box(build_columnar_record("p", "Batch", &fields));
        }
        let in_place = t.elapsed();

        let t = Instant::now();
        for _ in 0..ITERS {
            let bytes = with_struct_view(true, || {
                message_to_wire_with("p", &sv, WireCodec::Native, current_integrity()).unwrap()
            });
            std::hint::black_box(bytes);
        }
        let serialize_existing = t.elapsed();

        eprintln!(
            "encode 8×256 i64 record: build-in-place={in_place:?} serialize-existing={serialize_existing:?} ratio={:.2}x",
            serialize_existing.as_secs_f64() / in_place.as_secs_f64().max(f64::MIN_POSITIVE)
        );
        // Parity, not a fabricated win: build-in-place must be within a small band of the existing
        // memcpy-fast path (never meaningfully slower) — it ships the SAME bytes with no RuntimeValue.
        assert!(
            in_place.as_secs_f64() <= serialize_existing.as_secs_f64() * 1.5,
            "build-in-place must be at parity with the existing column encode: in_place={in_place:?} existing={serialize_existing:?}"
        );
    }

    #[test]
    fn columnar_record_is_position_independent_mmap_and_ipc_ready() {
        // Cap'n Proto's FLAGSHIP: messages are position-independent (offset-based, never pointer-
        // based), so you can mmap a file or share a buffer across processes and read fields IN PLACE
        // — the OS pages in only what you touch, two processes share one segment with no kernel pipe.
        // `T_STRUCT_VIEW` uses RELATIVE offsets, so the SAME holds for Logos: the bytes read zero-copy
        // from ANY backing store at ANY base, with no relocation/fixup. This locks that property.
        let ids: Vec<i64> = (0..4096).collect();
        let xs: Vec<f64> = (0..4096).map(|i| i as f64 * 1.25).collect();
        let msg = build_columnar_record(
            "p",
            "Batch",
            &[("id", WireColumn::Ints(&ids)), ("x", WireColumn::Floats(&xs))],
        );

        // Relocate the message to a different base address (an mmap maps at a page boundary; a shared
        // segment lands at its own offset). An 8-aligned shift keeps the columns zero-copy at the new
        // base; the read stays CORRECT at any base (position independence) — verified both ways.
        for &shift in &[0usize, 8, 16, 4096, 65536] {
            let mut arena = vec![0u8; shift];
            arena.extend_from_slice(&msg);
            let relocated = &arena[shift..]; // a fresh borrow at base + `shift`
            let view = view_message(relocated).expect("position-independent open at any base");
            let id_slice = view.struct_field("id").unwrap().as_i64_slice().expect("zero-copy at aligned base");
            let x_slice = view.struct_field("x").unwrap().as_f64_slice().expect("zero-copy at aligned base");
            assert_eq!(id_slice, &ids[..], "id column read in place at base+{shift}");
            assert_eq!(x_slice, &xs[..], "x column read in place at base+{shift}");
        }

        // A NON-aligned base: the slice cast declines (alignment guard) — but the message is still
        // read correctly via the copy path, so correctness is base-independent, only the zero-copy
        // fast path needs alignment (which mmap/page boundaries always provide).
        let mut arena = vec![0u8; 3];
        arena.extend_from_slice(&msg);
        let view = view_message(&arena[3..]).expect("opens at an unaligned base too");
        let (_, val) = message_from_wire(&arena[3..]).expect("full decode at unaligned base");
        match val {
            RuntimeValue::Struct(s) => match s.fields.get("id").unwrap() {
                RuntimeValue::List(l) => match &*l.borrow() {
                    ListRepr::Ints(v) => assert_eq!(v, &ids, "correct at an unaligned base via the copy path"),
                    o => panic!("expected Ints, got {o:?}"),
                },
                o => panic!("expected List, got {o:?}"),
            },
            o => panic!("expected Struct, got {o:?}"),
        }
        // The field view still resolves at the unaligned base (offsets are relative); only the
        // zero-copy slice cast is alignment-gated.
        assert!(view.struct_field("id").is_some(), "field still locatable at an unaligned base");
    }

    #[test]
    fn columnar_record_mmaps_a_column_zero_copy_from_disk() {
        // The visceral crush of Cap'n Proto's headline: write the columnar record to a FILE, mmap
        // it, and read one column ZERO-COPY straight from the mapped pages — no parse, no decode, no
        // per-element copy. mmap pages start at a page boundary (4 KiB-aligned ⇒ 8-aligned), so the
        // aligned columns cast soundly. The OS pages in only what we touch. Smaller file than capnp
        // for the same data (name elision), and read in place all the same.
        use std::io::Write;
        let ids: Vec<i64> = (0..50_000).collect();
        let xs: Vec<f64> = (0..50_000).map(|i| (i as f64).sqrt()).collect();
        let msg = build_columnar_record(
            "p",
            "Telemetry",
            &[("id", WireColumn::Ints(&ids)), ("x", WireColumn::Floats(&xs))],
        );

        let mut tmp = tempfile::NamedTempFile::new().expect("temp file");
        tmp.write_all(&msg).expect("write the wire message to disk");
        tmp.flush().expect("flush");
        let file = tmp.reopen().expect("reopen for mapping");
        // SAFETY: the file is not mutated while mapped (single-test, exclusive temp file).
        let mmap = unsafe { memmap2::Mmap::map(&file).expect("mmap the message file") };

        let view = view_message(&mmap[..]).expect("the mmap'd message opens in place");
        let id_slice = view.struct_field("id").unwrap().as_i64_slice().expect("zero-copy i64 from mmap");
        let x_slice = view.struct_field("x").unwrap().as_f64_slice().expect("zero-copy f64 from mmap");
        // The slices point INTO the mmap — no allocation, no decode. Verify against the source.
        assert_eq!(id_slice, &ids[..], "id column read zero-copy directly from the mmap");
        assert_eq!(x_slice, &xs[..], "x column read zero-copy directly from the mmap");
        // The slice genuinely aliases the mapped pages (zero-copy), not a decoded heap copy.
        let map_base = mmap.as_ptr() as usize;
        let slice_base = id_slice.as_ptr() as usize;
        assert!(
            slice_base >= map_base && slice_base < map_base + mmap.len(),
            "the i64 slice must alias the mapped pages, not a copy"
        );
    }

    #[test]
    fn wireview_decode_and_schema_read_a_record_list_in_place() {
        // ZC1: a record-list view exposes its schema (type, fields, row count) and decodes any ONE
        // (row, field) cell in place — the primitives a lazy zero-copy receive backing reads through,
        // never decoding the rest of the list.
        let mk = |id: i64, x: f64| {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(id));
            f.insert("x".to_string(), RuntimeValue::Float(x));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "Rec".to_string(), fields: f }))
        };
        let rows = vec![mk(10, 1.5), mk(20, 2.5), mk(30, 3.5)];
        let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
        let bytes = with_struct_view(true, || {
            message_to_wire_with("p", &list, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let view = view_message(&bytes).expect("record list opens as a view");

        let (ty, fields, n) = view.structs_schema().expect("record-list schema in place");
        assert_eq!(ty, "Rec");
        assert_eq!(fields, vec!["id".to_string(), "x".to_string()], "sorted field schema");
        assert_eq!(n, 3, "row count from the header, no rows decoded");

        assert_eq!(view.structs_row_field(1, "id").unwrap().decode(), Some(RuntimeValue::Int(20)));
        assert_eq!(view.structs_row_field(2, "x").unwrap().decode(), Some(RuntimeValue::Float(3.5)));
        assert_eq!(view.structs_row_field(0, "id").unwrap().decode(), Some(RuntimeValue::Int(10)));
        assert!(view.structs_row_field(3, "id").is_none(), "out-of-range row is None");
        assert!(view.structs_row_field(0, "nope").is_none(), "missing field is None");
    }

    #[test]
    fn lazy_wirestructs_reads_records_without_eager_decode() {
        // ZC2: a received record-list held as RAW BYTES (ListRepr::WireStructs) reads any (row,
        // field) in place — `len` is O(1) with zero rows decoded, `get_field` touches one cell, and
        // full materialization matches the eager decode value-for-value.
        use crate::interpreter::{ListRepr, StructValue};
        let mk = |id: i64, x: f64, tag: &str| {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(id));
            f.insert("x".to_string(), RuntimeValue::Float(x));
            f.insert("tag".to_string(), RuntimeValue::Text(Rc::new(tag.to_string())));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "Rec".to_string(), fields: f }))
        };
        let rows: Vec<RuntimeValue> = (0..1000).map(|i| mk(i, i as f64 * 0.5, &format!("t{i}"))).collect();
        let eager = ListRepr::from_values(rows);
        let list = RuntimeValue::List(Rc::new(RefCell::new(eager.clone())));
        let bytes = with_struct_view(true, || {
            message_to_wire_with("p", &list, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });

        let lazy = ListRepr::from_record_list_view(Rc::new(bytes)).expect("wraps as a lazy view");
        assert_eq!(lazy.len(), 1000, "len is O(1) from the header — no rows decoded");

        // Single-cell reads, located + decoded in place (never touching the other rows).
        assert_eq!(lazy.get_field(0, "id"), Some(RuntimeValue::Int(0)));
        assert_eq!(lazy.get_field(999, "id"), Some(RuntimeValue::Int(999)));
        assert_eq!(lazy.get_field(500, "x"), Some(RuntimeValue::Float(250.0)));
        match lazy.get_field(7, "tag") {
            Some(RuntimeValue::Text(s)) => assert_eq!(&*s, "t7"),
            o => panic!("expected tag text, got {o:?}"),
        }
        assert_eq!(lazy.get_field(0, "missing"), None, "missing field is None");

        // Whole-row reconstruction on demand.
        match lazy.get(3) {
            Some(RuntimeValue::Struct(s)) => {
                assert_eq!(s.type_name, "Rec");
                assert_eq!(s.fields.get("id"), Some(&RuntimeValue::Int(3)));
                assert_eq!(s.fields.get("x"), Some(&RuntimeValue::Float(1.5)));
            }
            o => panic!("expected struct row, got {o:?}"),
        }

        // Full materialization equals the eager decode, value-for-value.
        // Structural struct equality (the kernel's `values_equal` is reference-semantic for structs,
        // so compare type + every field by value).
        fn struct_eq(a: &RuntimeValue, b: &RuntimeValue) -> bool {
            match (a, b) {
                (RuntimeValue::Struct(x), RuntimeValue::Struct(y)) => {
                    x.type_name == y.type_name
                        && x.fields.len() == y.fields.len()
                        && x.fields.iter().all(|(k, v)| {
                            y.fields.get(k).is_some_and(|w| crate::semantics::compare::values_equal(v, w))
                        })
                }
                _ => crate::semantics::compare::values_equal(a, b),
            }
        }
        let lazy_vals = lazy.to_values();
        let eager_vals = eager.to_values();
        assert_eq!(lazy_vals.len(), eager_vals.len());
        for (idx, (a, b)) in lazy_vals.iter().zip(&eager_vals).enumerate() {
            assert!(struct_eq(a, b), "row {idx} differs:\n  lazy={a:?}\n eager={b:?}");
        }
    }

    #[test]
    fn message_from_wire_view_is_lazy_for_record_lists_eager_otherwise() {
        // ZC3: the lazy receive entry point holds a record list as raw bytes (WireStructs, NO rows
        // decoded) while any other shape full-decodes exactly as before. Sender is preserved either
        // way; the receiver opts in via the `view` knob (ZC4).
        use crate::interpreter::{ListRepr, StructValue};
        let mk = |id: i64| {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(id));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "R".to_string(), fields: f }))
        };
        let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values((0..100).map(mk).collect()))));
        let bytes =
            with_struct_view(true, || message_to_wire_with("alice", &list, WireCodec::Native, WireIntegrity::Raw).unwrap());

        let (from, val) = message_from_wire_view(&bytes).expect("lazy view decode");
        assert_eq!(from, "alice", "sender preserved on the lazy path");
        match &val {
            RuntimeValue::List(rc) => {
                assert!(
                    matches!(&*rc.borrow(), ListRepr::WireStructs { .. }),
                    "a record list must be held LAZILY (no rows decoded), got {:?}",
                    rc.borrow()
                );
                assert_eq!(rc.borrow().len(), 100, "O(1) len, no decode");
                assert_eq!(rc.borrow().get_field(42, "id"), Some(RuntimeValue::Int(42)), "in-place field read");
            }
            o => panic!("expected a lazy list, got {o:?}"),
        }

        // A scalar message has no record-list view → full decode, identical to `message_from_wire`.
        let sbytes = message_to_wire_with("bob", &RuntimeValue::Int(7), WireCodec::Native, WireIntegrity::Raw).unwrap();
        let (sfrom, sval) = message_from_wire_view(&sbytes).expect("scalar decode");
        assert_eq!(sfrom, "bob");
        assert_eq!(sval, RuntimeValue::Int(7), "non-record shape falls back to full decode");
    }

    #[test]
    fn production_receive_path_defers_then_reads_record_list_lazily() {
        // ZC5: mirror EXACTLY the interpreter's receive decisions for a record list:
        //   drain      → `peek_deferrable_sender` says "DEFER" (buffer raw, decode NOTHING)
        //   Await view  → `from_record_list_view` (LAZY — no rows decoded until touched)
        //   Await       → `message_from_wire` (eager) — same values
        // and prove a scalar is NOT deferrable (decoded in order at drain). This is the
        // "no decode in production" path, verified at the exact functions it calls.
        use crate::interpreter::{ListRepr, StructValue};
        let mk = |id: i64, name: &str| {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(id));
            f.insert("name".to_string(), RuntimeValue::Text(Rc::new(name.to_string())));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "User".to_string(), fields: f }))
        };
        let rows: Vec<RuntimeValue> = (0..500).map(|i| mk(i, &format!("u{i}"))).collect();
        let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
        let frame =
            with_struct_view(true, || message_to_wire_with("alice", &list, WireCodec::Native, WireIntegrity::Raw).unwrap());

        // 1) Drain: a record-list view is a DEFERRABLE message — peek yields the sender, no decode.
        assert_eq!(peek_deferrable_sender(&frame).as_deref(), Some("alice"), "record list defers at drain");

        // 2) `Await view`: wrap the buffered frame LAZILY — a WireStructs, zero rows decoded.
        let lazy = ListRepr::from_record_list_view(Rc::new(frame.clone())).expect("lazy wrap");
        assert!(matches!(lazy, ListRepr::WireStructs { .. }), "Await view holds the list lazily");
        assert_eq!(lazy.len(), 500, "O(1) len, nothing decoded");
        assert_eq!(lazy.get_field(123, "id"), Some(RuntimeValue::Int(123)), "in-place cell read");

        // 3) `Await` (no view): the SAME buffered frame decodes eagerly to the same values.
        let (efrom, eager_val) = message_from_wire(&frame).expect("eager decode");
        assert_eq!(efrom, "alice");
        let eager_rows = match &eager_val {
            RuntimeValue::List(rc) => rc.borrow().to_values(),
            o => panic!("expected list, got {o:?}"),
        };
        let lazy_rows = lazy.to_values();
        assert_eq!(lazy_rows.len(), eager_rows.len());
        for (idx, (a, b)) in lazy_rows.iter().zip(&eager_rows).enumerate() {
            let eq = match (a, b) {
                (RuntimeValue::Struct(x), RuntimeValue::Struct(y)) => {
                    x.type_name == y.type_name
                        && x.fields.iter().all(|(k, v)| {
                            y.fields.get(k).is_some_and(|w| crate::semantics::compare::values_equal(v, w))
                        })
                }
                _ => false,
            };
            assert!(eq, "lazy and eager receive must agree at row {idx}");
        }

        // 4) A scalar message is NOT a deferrable record list → decoded eagerly in arrival order.
        let sframe = message_to_wire_with("bob", &RuntimeValue::Int(7), WireCodec::Native, WireIntegrity::Raw).unwrap();
        assert_eq!(peek_deferrable_sender(&sframe), None, "a scalar is not deferred");
    }

    #[test]
    fn lazy_wirecolumn_reads_received_numeric_columns_zero_copy() {
        // EXTEND: a received aligned NUMERIC column (`Seq of Int`/`Seq of Float` sent fast/aligned)
        // is deferred at drain and read ZERO-COPY out of the borrowed `&[i64]`/`&[f64]` — capnp's
        // `List<i64>` read-in-place. `len` O(1), `get(i)` reads one element, no eager decode.
        use crate::interpreter::ListRepr;

        let ints: Vec<i64> = (0..2000).collect();
        let il = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(ints.clone()))));
        let ibytes =
            with_struct_view(true, || message_to_wire_with("alice", &il, WireCodec::Native, WireIntegrity::Raw).unwrap());
        assert_eq!(peek_deferrable_sender(&ibytes).as_deref(), Some("alice"), "aligned int column defers");
        let lazy = ListRepr::from_received_view(Rc::new(ibytes)).expect("lazy int column");
        assert!(matches!(lazy, ListRepr::WireColumn { floats: false, .. }), "held as a lazy int column");
        assert_eq!(lazy.len(), 2000, "O(1) len, no decode");
        assert_eq!(lazy.get(0), Some(RuntimeValue::Int(0)), "zero-copy element read");
        assert_eq!(lazy.get(1999), Some(RuntimeValue::Int(1999)));
        assert_eq!(lazy.get(2000), None, "out of range");
        let vals = lazy.to_values();
        assert_eq!(vals.len(), 2000);
        assert_eq!(vals[500], RuntimeValue::Int(500));

        let floats: Vec<f64> = (0..1000).map(|i| i as f64 * 0.25).collect();
        let fl = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(floats.clone()))));
        let fbytes =
            with_struct_view(true, || message_to_wire_with("bob", &fl, WireCodec::Native, WireIntegrity::Raw).unwrap());
        assert_eq!(peek_deferrable_sender(&fbytes).as_deref(), Some("bob"), "aligned float column defers");
        let lazyf = ListRepr::from_received_view(Rc::new(fbytes)).expect("lazy float column");
        assert!(matches!(lazyf, ListRepr::WireColumn { floats: true, .. }), "held as a lazy float column");
        assert_eq!(lazyf.len(), 1000);
        assert_eq!(lazyf.get(4), Some(RuntimeValue::Float(1.0)), "zero-copy float read (0.25*4)");
        assert_eq!(lazyf.to_values()[8], RuntimeValue::Float(2.0));
    }

    #[test]
    fn batch_stream_message_round_trips() {
        // The `Stream`/`Await stream` substrate: many values batched into ONE framed blob, deframed
        // back in order, with the sender peekable and a normal message never mistaken for a stream.
        let values = vec![
            RuntimeValue::Int(1),
            RuntimeValue::Int(-42),
            RuntimeValue::Text(Rc::new("hi".to_string())),
            RuntimeValue::Bool(true),
        ];
        let blob = frame_stream_message("alice", &values).expect("frames a stream");
        assert_eq!(peek_stream_sender(&blob).as_deref(), Some("alice"), "sender peekable at drain");

        let got = deframe_stream_message(&blob).expect("deframes the stream");
        assert_eq!(got.len(), 4);
        assert_eq!(got[0], RuntimeValue::Int(1));
        assert_eq!(got[1], RuntimeValue::Int(-42));
        match &got[2] {
            RuntimeValue::Text(t) => assert_eq!(&**t, "hi"),
            o => panic!("expected text, got {o:?}"),
        }
        assert_eq!(got[3], RuntimeValue::Bool(true));

        // A normal message is NOT a stream (the 0xFD marker disambiguates).
        let normal = message_to_wire("p", &RuntimeValue::Int(5)).unwrap();
        assert_eq!(peek_stream_sender(&normal), None, "a normal message is not a stream");
        assert_eq!(deframe_stream_message(&normal), None);

        // An empty stream is valid (zero values).
        let empty = frame_stream_message("bob", &[]).unwrap();
        assert_eq!(peek_stream_sender(&empty).as_deref(), Some("bob"));
        assert_eq!(deframe_stream_message(&empty), Some(vec![]));
    }

    #[test]
    fn build_in_place_edge_cases() {
        // Edge cases: an EMPTY column (still 8-aligned, reads back as `&[]`), a single element, and
        // a missing field (None, never a panic). The padding aligns even the zero-length blob.
        let empty: Vec<i64> = vec![];
        let one: Vec<i64> = vec![42];
        let bytes = build_columnar_record(
            "",
            "E",
            &[("z", WireColumn::Ints(&empty)), ("o", WireColumn::Ints(&one))],
        );
        let view = view_message(&bytes).unwrap();
        assert_eq!(view.struct_field("z").unwrap().as_i64_slice().expect("empty is still aligned"), &[] as &[i64]);
        assert_eq!(view.struct_field("o").unwrap().as_i64_slice().expect("singleton zero-copy"), &[42]);
        assert!(view.struct_field("missing").is_none(), "a missing field is None, not a panic");
    }

    /// A value round-trips iff materialize∘rebuild∘materialize is the identity on
    /// the payload. We compare through `RtPayload` (which has structural
    /// equality), because `RuntimeValue`'s `PartialEq` returns false for
    /// collections/structs (reference semantics).
    fn assert_roundtrips(v: &RuntimeValue) -> RtPayload {
        let p = materialize(v).expect("materialize");
        let back = rebuild(p.clone());
        let p2 = materialize(&back).expect("re-materialize");
        assert_eq!(p, p2, "marshalling round-trip changed the value");
        p
    }

    /// Encode `v` as an `Ints` list, decode it back, and return the recovered
    /// integers — the round-trip oracle for the affine math hack.
    fn affine_roundtrip(v: &[i64], s: WireStructure) -> (Vec<u8>, Vec<i64>) {
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v.to_vec()))));
        let bytes =
            with_structure(s, || message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap());
        let (_, back) = message_from_wire(&bytes).expect("decode");
        let got = match back {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Ints(g) => g.clone(),
                _ => panic!("expected an Ints list back"),
            },
            _ => panic!("expected a List back"),
        };
        (bytes, got)
    }

    #[test]
    fn affine_int_column_elides_to_a_formula_and_round_trips() {
        let v: Vec<i64> = (0..1000).collect();
        let (bytes, got) = affine_roundtrip(&v, WireStructure::Affine);
        // The whole 1000-element column becomes (base, stride, n) — a handful of bytes.
        assert!(bytes.len() < 40, "a 1000-element affine column should elide to O(1) bytes; got {}", bytes.len());
        assert_eq!(got, v, "affine round-trip must be exact");
    }

    #[test]
    fn non_affine_column_is_not_elided_but_still_round_trips() {
        let mut v: Vec<i64> = (0..1000).collect();
        v[500] = 999_999; // break the progression — must NOT be mis-detected as affine
        let (bytes, got) = affine_roundtrip(&v, WireStructure::Affine);
        assert!(bytes.len() > 500, "a non-affine column must fall back to a real encoding; got {}", bytes.len());
        assert_eq!(got, v, "fallback round-trip must be exact");
    }

    #[test]
    fn affine_is_bijective_across_i64_overflow() {
        // A progression that wraps past i64::MAX — the wrapping match must reproduce it.
        let base = i64::MAX - 3;
        let stride = 5i64;
        let v: Vec<i64> = (0..100).map(|i| base.wrapping_add((i as i64).wrapping_mul(stride))).collect();
        let (_, got) = affine_roundtrip(&v, WireStructure::Affine);
        assert_eq!(got, v, "wrapping-affine round-trip must be exact");
    }

    // ---- G5: the per-column compression menu (WireStructure::Auto) -----------------

    #[test]
    fn wire_auto_delta_wins_on_monotone() {
        // A monotone column with small steps → delta makes the deltas one byte each.
        let v: Vec<i64> = (0..200i64).scan(1000i64, |s, i| { *s += 1 + (i % 3); Some(*s) }).collect();
        let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
        let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
        assert_eq!(got, v, "delta round-trips bit-exact");
        assert!(auto.len() < varint.len(), "Auto ({}) must beat varint ({}) on a monotone column", auto.len(), varint.len());
    }

    #[test]
    fn wire_auto_dod_wins_on_timestamps() {
        // Near-linear timestamps (large base + i·step + tiny jitter) → delta-of-delta ≈ 0.
        let v: Vec<i64> = (0..300i64).map(|i| 1_700_000_000 + i * 1000 + (i % 5)).collect();
        let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
        let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
        assert_eq!(got, v, "delta-of-delta round-trips bit-exact");
        assert!(auto.len() < varint.len(), "Auto ({}) must beat varint ({}) on timestamps", auto.len(), varint.len());
    }

    #[test]
    fn wire_auto_for_wins_on_clustered() {
        // A tight cluster around a large base → frame-of-reference bit-packs the residuals.
        let mut rng = SplitMix64 { state: 0x0000_F00D };
        let v: Vec<i64> = (0..400).map(|_| 1_000_000 + (rng.next() % 16) as i64).collect();
        let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
        let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
        assert_eq!(got, v, "frame-of-reference round-trips bit-exact");
        assert!(auto.len() < varint.len(), "Auto ({}) must beat varint ({}) on a clustered column", auto.len(), varint.len());
    }

    #[test]
    fn wire_auto_polynomial_ships_the_generator_not_the_data() {
        // A degree-2 polynomial column ships a tiny GENERATOR (degree + a few seeds + n) —
        // "ship the computation, not the data" — and reconstructs bit-exact. The frontier
        // nobody else has: protobuf/capnp/arrow all ship the n raw values.
        let v: Vec<i64> = (0..10_000i64).map(|i| 3 * i * i - 5 * i + 7).collect();
        let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
        assert_eq!(got, v, "the polynomial column reconstructs bit-exact");
        let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
        eprintln!(
            "polynomial generator: {} values → ours {} B vs raw varint {} B ({}× smaller)",
            v.len(),
            auto.len(),
            varint.len(),
            varint.len() / auto.len().max(1)
        );
        assert!(
            auto.len() < varint.len() / 100,
            "the generator ({} B) must ship ≪ the data ({} B)",
            auto.len(),
            varint.len()
        );
    }

    #[test]
    fn wire_auto_polynomial_handles_cubic_and_negative_and_overflow() {
        // Degree-3 round-trips; a column whose differences overflow i64 falls back to the
        // menu (never mis-encodes); a non-polynomial column is left to the other candidates.
        let cubic: Vec<i64> = (0..500i64).map(|i| 2 * i * i * i - i * i + 11).collect();
        let (_, got) = affine_roundtrip(&cubic, WireStructure::Auto);
        assert_eq!(got, cubic, "cubic reconstructs bit-exact");

        let overflowing: Vec<i64> = vec![i64::MIN, i64::MAX, i64::MIN, i64::MAX];
        let (_, got) = affine_roundtrip(&overflowing, WireStructure::Auto);
        assert_eq!(got, overflowing, "an overflowing column still round-trips (via the menu)");

        let mut rng = SplitMix64 { state: 0xBADC_0FFE };
        let noise: Vec<i64> = (0..500).map(|_| rng.next() as i64).collect();
        let (_, got) = affine_roundtrip(&noise, WireStructure::Auto);
        assert_eq!(got, noise, "random noise round-trips (no false polynomial detection)");
    }

    #[test]
    fn wire_auto_geometric_ships_the_generator_not_the_data() {
        // A geometric column `base * ratio^i` is NOT a polynomial (finite differences never
        // settle) and NOT affine, so without a dedicated detector it costs ~1 varint PER
        // element — the magnitude doubles every step. The generator ships THREE numbers
        // (base, ratio, count) regardless of length, and reconstructs bit-exact.
        let doubling: Vec<i64> = (0..40).map(|i| 3i64 * (1i64 << i)).collect();
        let (bytes, got) = affine_roundtrip(&doubling, WireStructure::Auto);
        assert_eq!(got, doubling, "a geometric column reconstructs bit-exact");
        assert!(
            bytes.len() < 20,
            "the geometric GENERATOR ships ~3 numbers, not 40 growing values: {} bytes",
            bytes.len()
        );

        // Negative ratio (alternating sign, growing magnitude) is geometric too.
        let alternating: Vec<i64> = {
            let mut c = 1i64;
            (0..40)
                .map(|_| {
                    let v = c;
                    c = c.wrapping_mul(-2);
                    v
                })
                .collect()
        };
        let (bytes, got) = affine_roundtrip(&alternating, WireStructure::Auto);
        assert_eq!(got, alternating, "a negative-ratio geometric column reconstructs bit-exact");
        assert!(bytes.len() < 20, "negative-ratio geometric also ships the generator: {} bytes", bytes.len());

        // SOUNDNESS under overflow: a doubling sequence that runs PAST i64 wraps deterministically
        // — the detector verifies reproduction under the SAME `wrapping_mul` the decoder uses, so it
        // is still recognized AND round-trips bit-exact across the wrap (2^63 → i64::MIN → 0 → 0…).
        let wrapping: Vec<i64> = {
            let mut c = 1i64;
            (0..70)
                .map(|_| {
                    let v = c;
                    c = c.wrapping_mul(2);
                    v
                })
                .collect()
        };
        let (_, got) = affine_roundtrip(&wrapping, WireStructure::Auto);
        assert_eq!(got, wrapping, "an overflowing geometric column still round-trips exactly");

        // NO false positives: random noise, a near-geometric sequence with one perturbed element,
        // and an affine column are NOT mis-encoded as geometric — each round-trips exactly.
        let mut rng = SplitMix64 { state: 0x6E0_47E7 };
        let noise: Vec<i64> = (0..500).map(|_| rng.next() as i64).collect();
        let (_, got) = affine_roundtrip(&noise, WireStructure::Auto);
        assert_eq!(got, noise, "random noise round-trips (no false geometric detection)");

        let mut perturbed: Vec<i64> = (0..30).map(|i| 5i64 * (1i64 << i)).collect();
        perturbed[17] += 1; // breaks the geometric law at one point
        let (_, got) = affine_roundtrip(&perturbed, WireStructure::Auto);
        assert_eq!(got, perturbed, "a perturbed near-geometric column round-trips (verification rejects it)");

        let affine: Vec<i64> = (0..40).map(|i| 7 + 3 * i).collect();
        let (_, got) = affine_roundtrip(&affine, WireStructure::Auto);
        assert_eq!(got, affine, "an affine column round-trips (geometric does not steal it)");
    }

    #[test]
    fn wire_auto_periodic_ships_the_repeating_block() {
        // A cyclic column `pattern[i % p]` (weekly schedules, repeating categories, sawtooth
        // bytes) is none of affine/geometric/polynomial — but it is fully described by ONE
        // period's worth of values plus the count. Ship the block, not the 500 repeats.
        let block = [10i64, 20, 30, 40, 50];
        let cyclic: Vec<i64> = (0..500).map(|i| block[i % block.len()]).collect();
        let (bytes, got) = affine_roundtrip(&cyclic, WireStructure::Auto);
        assert_eq!(got, cyclic, "a periodic column reconstructs bit-exact");
        assert!(
            bytes.len() < 30,
            "the periodic GENERATOR ships ONE period ({} values), not 500: {} bytes",
            block.len(),
            bytes.len()
        );

        // Negative / mixed-magnitude period, and a non-trivial period length.
        let block2 = [-7i64, 0, 1000000, -3, 42, 42, -1];
        let cyclic2: Vec<i64> = (0..1001).map(|i| block2[i % block2.len()]).collect();
        let (bytes, got) = affine_roundtrip(&cyclic2, WireStructure::Auto);
        assert_eq!(got, cyclic2, "a mixed-magnitude periodic column reconstructs bit-exact");
        assert!(bytes.len() < 40, "still ships ONE period, not 1001 values: {} bytes", bytes.len());

        // Minimal period wins: a column that is ALSO period-10 (because it is period-5) ships the
        // SMALLER period-5 block — and round-trips.
        let p5: Vec<i64> = (0..200).map(|i| block[i % 5]).collect();
        let (_, got) = affine_roundtrip(&p5, WireStructure::Auto);
        assert_eq!(got, p5, "the minimal period round-trips");

        // NO false positives: random noise and an aperiodic column (period == length, no repeat)
        // are NOT mis-encoded as periodic — each round-trips exactly.
        let mut rng = SplitMix64 { state: 0x9E15_AB0 };
        let noise: Vec<i64> = (0..500).map(|_| rng.next() as i64).collect();
        let (_, got) = affine_roundtrip(&noise, WireStructure::Auto);
        assert_eq!(got, noise, "random noise round-trips (no false periodic detection)");

        let aperiodic: Vec<i64> = (0..50).map(|i| i * i + 1).collect();
        let (_, got) = affine_roundtrip(&aperiodic, WireStructure::Auto);
        assert_eq!(got, aperiodic, "an aperiodic column round-trips");
    }

    #[test]
    fn wire_float_default_dial_is_pure_memcpy() {
        // The lightning-quick contract: under the DEFAULT (`Off`) dial the float encoder never
        // inspects the data — even a constant / affine / periodic column ships as the raw n×8 memcpy
        // (`T_FLOATS`). Structural shrinking is opt-in via `Auto`/`Affine`. This mirrors the integer
        // contract (Off = straight varint, no detection) and is what keeps the hot encode path fast:
        // you pick the dial ahead of time, the encoder does not search on every send.
        let shapes: Vec<(&str, Vec<f64>)> = vec![
            ("constant", vec![3.14159f64; 1000]),
            ("affine", (0..1000).map(|i| i as f64).collect()),
            ("periodic", (0..1000).map(|i| [1.5f64, -2.25, 3.0, 0.0, 99.99][i % 5]).collect()),
        ];
        for (name, v) in &shapes {
            let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v.clone()))));
            let bytes = message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap();
            // The tag is the plain memcpy form and the body carries all n×8 raw bytes — NOT a tiny
            // generator. (Framing adds a few bytes on top, so `>=` n×8 is the memcpy signature.)
            assert!(
                bytes.len() >= v.len() * 8,
                "[{name}] the Off dial must ship the raw memcpy (≥ {} B), got {} B (detection leaked into the hot path)",
                v.len() * 8,
                bytes.len()
            );
            let got = match message_from_wire(&bytes).unwrap().1 {
                RuntimeValue::List(l) => match &*l.borrow() {
                    ListRepr::Floats(g) => g.clone(),
                    _ => panic!("expected Floats"),
                },
                _ => panic!("expected List"),
            };
            assert_eq!(&got, v, "[{name}] memcpy float column round-trips bit-exact");
        }
        // The opt-in still works: the SAME constant column under `Auto` ships the tiny generator.
        let c = vec![3.14159f64; 1000];
        let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(c))));
        let small = with_structure(WireStructure::Auto, || {
            message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        assert!(small.len() < 30, "Auto still ships the generator (one f64 + count): {} B", small.len());
    }

    #[test]
    fn wire_float_const_and_affine_ship_the_generator() {
        // Ship the GENERATOR for floats too: a constant column = one f64 + count; a bit-exact
        // `base + i·stride` column = three numbers. Both lossless; real noisy data falls through.
        fn roundtrip(v: Vec<f64>) -> (Vec<u8>, Vec<f64>) {
            let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))));
            // Structural float forms are `Auto`/`Affine`-only now (the default dial is a pure memcpy —
            // see `wire_float_default_dial_is_pure_memcpy`); opt into the menu to exercise them.
            let bytes = with_structure(WireStructure::Auto, || {
                message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            let got = match message_from_wire(&bytes).unwrap().1 {
                RuntimeValue::List(l) => match &*l.borrow() {
                    ListRepr::Floats(g) => g.clone(),
                    _ => panic!("expected Floats"),
                },
                _ => panic!("expected List"),
            };
            (bytes, got)
        }

        // Constant — 1000 identical f64 ship in ~11 bytes.
        let c = vec![3.14159f64; 1000];
        let (bytes, got) = roundtrip(c.clone());
        assert_eq!(got, c, "constant float column reconstructs");
        assert!(bytes.len() < 30, "constant ships one f64 + count, not 8000 B: {} B", bytes.len());

        // Affine — integer-valued floats (exact in f64) ship as base+stride+count.
        let ints: Vec<f64> = (0..1000).map(|i| i as f64).collect();
        let (bytes, got) = roundtrip(ints.clone());
        assert_eq!(got, ints, "integer-valued float column reconstructs bit-exact");
        assert!(bytes.len() < 40, "affine ships 3 numbers, not 8000 B: {} B", bytes.len());

        // Affine with a power-of-two stride — `i·0.5` is bit-exact.
        let half: Vec<f64> = (0..500).map(|i| (i as f64) * 0.5).collect();
        let (bytes, got) = roundtrip(half.clone());
        assert_eq!(got, half, "power-of-two-stride affine reconstructs bit-exact");
        assert!(bytes.len() < 40, "still 3 numbers: {} B", bytes.len());

        // NO false positives: noisy finite floats and a perturbed near-affine column round-trip
        // exactly via the raw/XOR path (the bit-exact check refuses anything that isn't perfect).
        let mut rng = SplitMix64 { state: 0xF10A7_C0DE };
        let noise: Vec<f64> = (0..500).map(|_| (rng.next() % 10_000_000) as f64 / 13.0).collect();
        let (_, got) = roundtrip(noise.clone());
        assert_eq!(got, noise, "noisy floats round-trip (no false generator detection)");

        let mut perturbed: Vec<f64> = (0..50).map(|i| i as f64).collect();
        perturbed[37] = 36.9999999;
        let (_, got) = roundtrip(perturbed.clone());
        assert_eq!(got, perturbed, "a perturbed near-affine column round-trips");
    }

    #[test]
    fn wire_auto_sparse_column_ships_dominant_plus_exceptions() {
        // A mostly-one-value column with a handful of DIVERSE-valued exceptions ships the dominant
        // value + a short (delta-index, value) list — beating dict (which would bit-pack 1000 indices
        // over 11 distinct values) and RLE (two run-entries per isolated exception).
        let mut v = vec![0i64; 1000];
        for k in 0..10 {
            v[k * 97] = (k as i64 + 1) * 12345;
        }
        let (bytes, got) = affine_roundtrip(&v, WireStructure::Auto);
        assert_eq!(got, v, "sparse column reconstructs exactly");
        assert!(bytes.len() < 80, "sparse ships ~10 exceptions, not 1000 values: {} bytes", bytes.len());

        // A non-zero dominant value works too.
        let mut v2 = vec![42i64; 500];
        for k in 0..5 {
            v2[k * 80 + 3] = -(k as i64 + 1) * 7_000_003;
        }
        let (bytes, got) = affine_roundtrip(&v2, WireStructure::Auto);
        assert_eq!(got, v2, "non-zero-dominant sparse column reconstructs exactly");
        assert!(bytes.len() < 80, "still ~5 exceptions: {} bytes", bytes.len());

        // NO false positives: a column with NO dominant value (random) round-trips via the menu,
        // not mis-encoded as sparse.
        let mut rng = SplitMix64 { state: 0x5A95E_C0DE };
        let noise: Vec<i64> = (0..500).map(|_| rng.next() as i64).collect();
        let (_, got) = affine_roundtrip(&noise, WireStructure::Auto);
        assert_eq!(got, noise, "random column round-trips (no false sparse detection)");
    }

    #[test]
    fn wire_float_sparse_column_ships_dominant_plus_exceptions() {
        // The float twin of sparse: a mostly-one-value f64 column (sparse telemetry, a mostly-zero
        // signal with a few spikes) ships the dominant value + the outliers, not all n×8 bytes.
        fn roundtrip(v: Vec<f64>) -> (Vec<u8>, Vec<f64>) {
            let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))));
            // Structural float forms are `Auto`/`Affine`-only now (the default dial is a pure memcpy —
            // see `wire_float_default_dial_is_pure_memcpy`); opt into the menu to exercise them.
            let bytes = with_structure(WireStructure::Auto, || {
                message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            let got = match message_from_wire(&bytes).unwrap().1 {
                RuntimeValue::List(l) => match &*l.borrow() {
                    ListRepr::Floats(g) => g.clone(),
                    _ => panic!("expected Floats"),
                },
                _ => panic!("expected List"),
            };
            (bytes, got)
        }

        let mut v = vec![0.0f64; 1000];
        for k in 0..10 {
            v[k * 97] = (k as f64 + 1.0) * 1234.5;
        }
        let (bytes, got) = roundtrip(v.clone());
        assert_eq!(got, v, "sparse float column reconstructs exactly");
        assert!(bytes.len() < 160, "sparse ships ~10 outliers, not 8000 B: {} B", bytes.len());

        // Non-zero dominant, with negative outliers.
        let mut v2 = vec![3.5f64; 500];
        for k in 0..5 {
            v2[k * 80 + 3] = -(k as f64 + 1.0) * 99.0;
        }
        let (_, got) = roundtrip(v2.clone());
        assert_eq!(got, v2, "non-zero-dominant sparse float column reconstructs exactly");

        // No dominant value (random finite floats) → not mis-encoded; round-trips via memcpy.
        let mut rng = SplitMix64 { state: 0xF10A7_5A95E };
        let noise: Vec<f64> = (0..500).map(|_| (rng.next() % 10_000_000) as f64 / 13.0).collect();
        let (_, got) = roundtrip(noise.clone());
        assert_eq!(got, noise, "random float column round-trips (no false sparse detection)");
    }

    #[test]
    fn wire_float_periodic_and_geometric_ship_the_generator() {
        fn roundtrip(v: Vec<f64>) -> (Vec<u8>, Vec<f64>) {
            let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))));
            // Structural float forms are `Auto`/`Affine`-only now (the default dial is a pure memcpy —
            // see `wire_float_default_dial_is_pure_memcpy`); opt into the menu to exercise them.
            let bytes = with_structure(WireStructure::Auto, || {
                message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            let got = match message_from_wire(&bytes).unwrap().1 {
                RuntimeValue::List(l) => match &*l.borrow() {
                    ListRepr::Floats(g) => g.clone(),
                    _ => panic!("expected Floats"),
                },
                _ => panic!("expected List"),
            };
            (bytes, got)
        }

        // Periodic — a repeating 5-float waveform ships ONE block, not 1000 samples.
        let block = [1.5f64, -2.25, 3.0, 0.0, 99.99];
        let cyclic: Vec<f64> = (0..1000).map(|i| block[i % 5]).collect();
        let (bytes, got) = roundtrip(cyclic.clone());
        assert_eq!(got, cyclic, "periodic float column reconstructs");
        assert!(bytes.len() < 80, "ships one 5-float block, not 1000: {} B", bytes.len());

        // Geometric — doubling and halving (power-of-two ratios) are bit-exact in f64.
        let doubling: Vec<f64> = {
            let mut c = 1.0f64;
            (0..50).map(|_| { let x = c; c *= 2.0; x }).collect()
        };
        let (bytes, got) = roundtrip(doubling.clone());
        assert_eq!(got, doubling, "doubling float column reconstructs bit-exact");
        assert!(bytes.len() < 40, "geometric ships base+ratio+count: {} B", bytes.len());

        let halving: Vec<f64> = {
            let mut c = 1024.0f64;
            (0..40).map(|_| { let x = c; c *= 0.5; x }).collect()
        };
        let (_, got) = roundtrip(halving.clone());
        assert_eq!(got, halving, "halving (exponential decay) reconstructs bit-exact");

        // NO false positives: random finite floats are neither periodic nor geometric — round-trip exact.
        let mut rng = SplitMix64 { state: 0xC0FFEE_F10A7 };
        let noise: Vec<f64> = (0..500).map(|_| (rng.next() % 9_999_991) as f64 / 7.0 - 1234.5).collect();
        let (_, got) = roundtrip(noise.clone());
        assert_eq!(got, noise, "random floats round-trip (no false periodic/geometric detection)");
    }

    #[test]
    fn wire_string_template_ships_prefix_suffix_and_affine_index() {
        // Sequential-id strings — REST URLs, file paths, generated labels — are `prefix + (base +
        // i·stride) + suffix`. Ship the two affixes once + the affine index, not all n strings.
        fn roundtrip(strings: &[String]) -> (Vec<u8>, Vec<String>) {
            let items: Vec<RuntimeValue> =
                strings.iter().map(|s| RuntimeValue::Text(Rc::new(s.clone()))).collect();
            let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
            let bytes = with_structure(WireStructure::Auto, || {
                message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            let got = match message_from_wire(&bytes).unwrap().1 {
                RuntimeValue::List(l) => {
                    let b = l.borrow();
                    (0..b.len())
                        .map(|i| match b.get(i).unwrap() {
                            RuntimeValue::Text(s) => (*s).clone(),
                            other => panic!("expected Text, got {other:?}"),
                        })
                        .collect()
                }
                _ => panic!("expected List"),
            };
            (bytes, got)
        }

        // Sequential-id URLs (long shared prefix) → ~40 bytes, not 37 KB.
        let urls: Vec<String> = (0..1000).map(|i| format!("https://api.example.com/v1/items/{i}")).collect();
        let (bytes, got) = roundtrip(&urls);
        assert_eq!(got, urls, "templated URLs reconstruct exactly");
        assert!(bytes.len() < 80, "ships prefix + affine index, not 1000 URLs: {} bytes", bytes.len());

        // Non-unit stride, and a prefix+suffix template (`file_<i>.txt`).
        let stepped: Vec<String> = (0..500).map(|i| format!("row_{}", i * 2)).collect();
        let (_, got) = roundtrip(&stepped);
        assert_eq!(got, stepped, "stride-2 templated labels reconstruct exactly");

        let files: Vec<String> = (0..300).map(|i| format!("file_{i}.txt")).collect();
        let (bytes, got) = roundtrip(&files);
        assert_eq!(got, files, "prefix+suffix template reconstructs exactly");
        assert!(bytes.len() < 60, "prefix+suffix template stays tiny: {} bytes", bytes.len());

        // NO false positives: non-affine ids and ZERO-PADDED ids (exact-decimal check) round-trip
        // via the flat/dictionary path, never a wrong template.
        let mut rng = SplitMix64 { state: 0x57117_C0DE };
        let scattered: Vec<String> = (0..200).map(|_| format!("k{}", rng.next() % 1_000_000)).collect();
        let (_, got) = roundtrip(&scattered);
        assert_eq!(got, scattered, "non-affine ids round-trip (no false template)");

        let padded: Vec<String> = (0..50).map(|i| format!("id_{i:03}")).collect();
        let (_, got) = roundtrip(&padded);
        assert_eq!(got, padded, "zero-padded ids round-trip (not templated — exact-decimal guard)");
    }

    #[test]
    fn wire_string_front_coding_crushes_sorted_shared_prefix_columns() {
        // The structural string compressor the dictionary (all-distinct → no win) and template
        // (non-affine / zero-padded → bails) can't touch: a sorted or hierarchical column whose
        // adjacent strings share long prefixes ships each as (shared-prefix-len, suffix).
        fn roundtrip(strings: &[String]) -> (Vec<u8>, Vec<String>) {
            let items: Vec<RuntimeValue> =
                strings.iter().map(|s| RuntimeValue::Text(Rc::new(s.clone()))).collect();
            let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
            let bytes = with_structure(WireStructure::Auto, || {
                message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            let got = match message_from_wire(&bytes).unwrap().1 {
                RuntimeValue::List(l) => {
                    let b = l.borrow();
                    (0..b.len())
                        .map(|i| match b.get(i).unwrap() {
                            RuntimeValue::Text(s) => (*s).clone(),
                            other => panic!("expected Text, got {other:?}"),
                        })
                        .collect()
                }
                _ => panic!("expected List"),
            };
            (bytes, got)
        }
        let flat_len = |v: &[String]| -> usize { v.iter().map(|s| s.len()).sum() };

        // Zero-padded log paths (long shared prefix AND suffix; template bails on the zero-padding,
        // the dictionary can't help 500 distinct strings — front-coding crushes them).
        let paths: Vec<String> = (0..500).map(|i| format!("/var/log/app/2026/06/service-{i:04}.log")).collect();
        let (bytes, got) = roundtrip(&paths);
        assert_eq!(got, paths, "front-coded log paths reconstruct exactly");
        assert!(
            bytes.len() * 3 < flat_len(&paths),
            "front-coding crushes the shared path prefix: {} vs flat {}",
            bytes.len(),
            flat_len(&paths)
        );

        // Sorted hierarchical object-store keys — non-affine, all distinct, deep shared prefixes.
        let mut keys: Vec<String> = Vec::new();
        for user in ["alice", "bob", "carol", "dave"] {
            for kind in ["profile", "settings", "avatar", "session"] {
                keys.push(format!("users/{user}/{kind}/data.json"));
            }
        }
        keys.sort();
        let (bytes, got) = roundtrip(&keys);
        assert_eq!(got, keys, "front-coded hierarchical keys reconstruct exactly");
        assert!(bytes.len() < flat_len(&keys), "front-coding beats flat on hierarchical keys");

        // MULTI-BYTE UTF-8 shared prefix: the `café` prefix (é = 2 bytes) must be cut on a char
        // boundary so the suffix stays valid UTF-8 — round-trips bit-exact.
        let unicode: Vec<String> = (0..100).map(|i| format!("café/naïve/Москва/key-{i:03}")).collect();
        let (_, got) = roundtrip(&unicode);
        assert_eq!(got, unicode, "front-coding cuts on UTF-8 char boundaries (no corruption)");

        // NO false win / NO corruption on a column with NO shared prefixes (high-entropy, unsorted):
        // `consider` keeps flat, and it still round-trips.
        let mut rng = SplitMix64 { state: 0xF20D_C0DE };
        let scattered: Vec<String> = (0..200)
            .map(|_| {
                let len = 6 + (rng.next() % 10) as usize;
                (0..len).map(|_| (b'a' + (rng.next() % 26) as u8) as char).collect()
            })
            .collect();
        let (_, got) = roundtrip(&scattered);
        assert_eq!(got, scattered, "prefix-free random strings round-trip (front-coding not falsely applied)");
    }

    #[test]
    fn wire_bool_generators_ship_constant_alternating_and_run_columns() {
        // The generator theme completed for the LAST column type: a constant / alternating / cyclic
        // bool column ships its tiny period, a run-structured one ships its runs — none of which the
        // 1-bit bit-pack (≈ n/8 bytes) can touch — while a random column correctly falls back.
        fn roundtrip(v: &[bool]) -> (Vec<u8>, Vec<bool>) {
            let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools(v.to_vec()))));
            let bytes = with_structure(WireStructure::Auto, || {
                message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            let got = match message_from_wire(&bytes).unwrap().1 {
                RuntimeValue::List(l) => match &*l.borrow() {
                    ListRepr::Bools(b) => b.clone(),
                    other => panic!("expected Bools, got {other:?}"),
                },
                _ => panic!("expected List"),
            };
            (bytes, got)
        }

        // Constant columns (all-true / all-false) → period-1 generator, not 125 packed bytes.
        for constant in [true, false] {
            let col = vec![constant; 1000];
            let (bytes, got) = roundtrip(&col);
            assert_eq!(got, col, "constant bool column reconstructs exactly");
            assert!(bytes.len() < 20, "constant={constant} ships a period-1 generator: {} bytes", bytes.len());
        }

        // Alternating → period-2; a weekly (period-7) flag → period-7.
        let alt: Vec<bool> = (0..1000).map(|i| i % 2 == 0).collect();
        let (bytes, got) = roundtrip(&alt);
        assert_eq!(got, alt, "alternating reconstructs exactly");
        assert!(bytes.len() < 20, "alternating ships a period-2 generator: {} bytes", bytes.len());

        let weekly: Vec<bool> = (0..700).map(|i| [true, true, true, true, true, false, false][i % 7]).collect();
        let (bytes, got) = roundtrip(&weekly);
        assert_eq!(got, weekly, "weekly flag reconstructs exactly");
        assert!(bytes.len() < 20, "period-7 weekly flag ships as periodic: {} bytes", bytes.len());

        // Two big runs ([F×500, T×500]) have NO short period → RLE crushes them.
        let runs: Vec<bool> = (0..1000).map(|i| i >= 500).collect();
        let (bytes, got) = roundtrip(&runs);
        assert_eq!(got, runs, "two big runs reconstruct exactly");
        assert!(bytes.len() < 20, "two big runs ship as RLE: {} bytes", bytes.len());

        // Mostly-true with a few scattered flips → RLE beats the 125-byte bit-pack.
        let mut mostly = vec![true; 1000];
        for k in [37, 199, 450, 777, 988] {
            mostly[k] = false;
        }
        let (bytes, got) = roundtrip(&mostly);
        assert_eq!(got, mostly, "mostly-true-with-flips reconstructs exactly");
        assert!(bytes.len() < 60, "a handful of flips ships as RLE, not 125 packed bytes: {} bytes", bytes.len());

        // Random → NO false generator: falls back to the bit-pack (~125 B) and round-trips.
        let mut rng = SplitMix64 { state: 0xB001_C0DE_F00D };
        let random: Vec<bool> = (0..1000).map(|_| rng.next() & 1 == 0).collect();
        let (bytes, got) = roundtrip(&random);
        assert_eq!(got, random, "random bools round-trip (no false generator detection)");
        assert!(bytes.len() >= 100, "random bools fall back to the bit-pack, not a false generator: {} bytes", bytes.len());

        // Edges: empty + single element round-trip cleanly.
        for edge in [vec![], vec![true], vec![false], vec![true, false], vec![false, false, false]] {
            let (_, got) = roundtrip(&edge);
            assert_eq!(got, edge, "bool edge case {edge:?} round-trips");
        }
    }

    #[test]
    fn wire_string_affix_ships_common_prefix_and_suffix_with_arbitrary_middles() {
        // The last string-column gap: a column sharing a common PREFIX and/or SUFFIX with ARBITRARY
        // middles — what the dictionary (all distinct), the template (non-affine middles), and front-
        // coding (shares only a PREFIX, pairwise) all miss.
        fn roundtrip(strings: &[String]) -> (Vec<u8>, Vec<String>) {
            let items: Vec<RuntimeValue> =
                strings.iter().map(|s| RuntimeValue::Text(Rc::new(s.clone()))).collect();
            let val = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
            let bytes = with_structure(WireStructure::Auto, || {
                message_to_wire_with("", &val, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            let got = match message_from_wire(&bytes).unwrap().1 {
                RuntimeValue::List(l) => {
                    let b = l.borrow();
                    (0..b.len())
                        .map(|i| match b.get(i).unwrap() {
                            RuntimeValue::Text(s) => (*s).clone(),
                            other => panic!("expected Text, got {other:?}"),
                        })
                        .collect()
                }
                _ => panic!("expected List"),
            };
            (bytes, got)
        }
        let flat_len = |v: &[String]| -> usize { v.iter().map(|s| s.len()).sum() };

        // Emails — common SUFFIX `@example.com`, arbitrary names (front-coding shares only prefixes,
        // so it can't crush the repeated suffix; affix ships the suffix ONCE).
        let names = ["alice", "bob", "charlie", "dave", "erin", "frank", "grace", "heidi"];
        let emails: Vec<String> = (0..400)
            .map(|i| format!("{}{}@example.com", names[i % names.len()], i))
            .collect();
        let (bytes, got) = roundtrip(&emails);
        assert_eq!(got, emails, "affixed emails reconstruct exactly");
        assert!(
            bytes.len() * 2 < flat_len(&emails),
            "affix ships the shared @example.com suffix once: {} vs flat {}",
            bytes.len(),
            flat_len(&emails)
        );

        // Common extension `.log` (suffix only, arbitrary stems that don't sort-share prefixes).
        let stems = ["app", "db", "auth", "cache", "queue", "web", "cron", "mail"];
        let files: Vec<String> = (0..400).map(|i| format!("{}-{}.log", stems[i % stems.len()], i * 7)).collect();
        let (bytes, got) = roundtrip(&files);
        assert_eq!(got, files, "affixed log files reconstruct exactly");
        assert!(bytes.len() < flat_len(&files), "affix beats flat on a shared extension");

        // Both prefix AND suffix, arbitrary (non-affine) middle → wrapped versioned ids.
        let mut rng = SplitMix64 { state: 0xAFF1_C0DE };
        let wrapped: Vec<String> = (0..300)
            .map(|_| format!("https://cdn.example.com/v2/{}/asset.json", rng.next() % 1_000_000))
            .collect();
        let (bytes, got) = roundtrip(&wrapped);
        assert_eq!(got, wrapped, "prefix+suffix wrapped ids reconstruct exactly");
        assert!(bytes.len() * 2 < flat_len(&wrapped), "affix crushes the shared URL wrapper");

        // NO false win: a column with NO shared affix round-trips via flat/dict (affix returns None
        // when prefix+suffix is empty).
        let mut rng2 = SplitMix64 { state: 0x0FF1_DEAD };
        let scattered: Vec<String> = (0..200)
            .map(|_| {
                let len = 5 + (rng2.next() % 8) as usize;
                (0..len).map(|_| (b'a' + (rng2.next() % 26) as u8) as char).collect()
            })
            .collect();
        let (_, got) = roundtrip(&scattered);
        assert_eq!(got, scattered, "affix-free random strings round-trip (no false affix)");
    }

    #[test]
    fn wire_gen_expr_substrate_round_trips_and_evaluates() {
        // `(i % 2 == 0) ? i*10 : i*10 + 5` — a piecewise column. The sandbox evaluates it
        // bit-exact, the tree round-trips through serialize/deserialize, and a T_GEN value
        // decodes to the evaluated column. This is the substrate a pure user function lowers
        // into — the receiver runs only this bounded evaluator, never arbitrary code.
        let expr = GenExpr::Select {
            op: GenCmp::Eq,
            lhs: Box::new(GenExpr::Mod(Box::new(GenExpr::Index), Box::new(GenExpr::Const(2)))),
            rhs: Box::new(GenExpr::Const(0)),
            then: Box::new(GenExpr::Mul(Box::new(GenExpr::Index), Box::new(GenExpr::Const(10)))),
            els: Box::new(GenExpr::Add(
                Box::new(GenExpr::Mul(Box::new(GenExpr::Index), Box::new(GenExpr::Const(10)))),
                Box::new(GenExpr::Const(5)),
            )),
        };
        let expected: Vec<i64> = (0..256i64).map(|i| if i % 2 == 0 { i * 10 } else { i * 10 + 5 }).collect();
        for (i, &e) in expected.iter().enumerate() {
            assert_eq!(gen_eval(&expr, i as i64), e, "sandbox eval matches at {i}");
        }
        let mut sbytes = Vec::new();
        serialize_gen(&expr, &mut sbytes);
        let mut p = 0;
        let mut budget = MAX_GEN_NODES;
        assert_eq!(deserialize_gen(&sbytes, &mut p, &mut budget, 0), Some(expr.clone()), "GenExpr round-trips");
        assert_eq!(p, sbytes.len(), "the tree is self-delimiting (no trailing bytes)");

        let mut bytes = vec![T_GEN];
        serialize_gen(&expr, &mut bytes);
        write_uvarint(expected.len() as u64, &mut bytes);
        let mut p = 0;
        match native_decode(&bytes, &mut p).expect("T_GEN decodes") {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Ints(got) => assert_eq!(got, &expected, "T_GEN evaluates to the column"),
                _ => panic!("expected Ints"),
            },
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn wire_lower_pure_function_to_genexpr() {
        // The bridge for "ship a user function": a pure single-param arithmetic function
        // lowers to the sandboxed GenExpr (so the receiver evaluates data, never runs code).
        // `f(i) = 3*i*i - 5*i + 7` lowers and evaluates identically; anything outside the
        // safe arithmetic subset (unknown var, a call) is refused — never shippable.
        use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
        use logicaffeine_base::{Arena, Symbol};
        fn num<'a>(a: &'a Arena<Expr<'a>>, v: i64) -> &'a Expr<'a> {
            a.alloc(Expr::Literal(Literal::Number(v)))
        }
        fn bin<'a>(a: &'a Arena<Expr<'a>>, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
            a.alloc(Expr::BinaryOp { op, left: l, right: r })
        }
        let a: Arena<Expr> = Arena::new();
        let i = Symbol::from_index(0);
        let idx: &Expr = a.alloc(Expr::Identifier(i));
        // 3*i*i - 5*i + 7
        let body = bin(
            &a,
            BinaryOpKind::Add,
            bin(
                &a,
                BinaryOpKind::Subtract,
                bin(&a, BinaryOpKind::Multiply, bin(&a, BinaryOpKind::Multiply, num(&a, 3), idx), idx),
                bin(&a, BinaryOpKind::Multiply, num(&a, 5), idx),
            ),
            num(&a, 7),
        );
        let g = lower_expr_to_genexpr(body, i).expect("pure arithmetic lowers");
        for x in -50..50i64 {
            assert_eq!(gen_eval(&g, x), 3 * x * x - 5 * x + 7, "lowered generator matches f at {x}");
        }
        let other: &Expr = a.alloc(Expr::Identifier(Symbol::from_index(1)));
        assert!(lower_expr_to_genexpr(other, i).is_none(), "unknown variable → not shippable");
        let call: &Expr = a.alloc(Expr::Call { function: Symbol::from_index(2), args: vec![] });
        assert!(lower_expr_to_genexpr(call, i).is_none(), "a function call → not shippable");
    }

    #[test]
    fn wire_computed_function_ships_callable_and_round_trips() {
        // A pure single-arg function lowered to a generator ships as T_FUNC, round-trips, and
        // the decoded function carries a generator that evaluates f(x) — a CALLABLE on a peer
        // that never compiled it. An ordinary closure (no generator) stays unsendable.
        use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
        use logicaffeine_base::{Arena, Symbol};
        let a: Arena<Expr> = Arena::new();
        let i = Symbol::from_index(0);
        let idx: &Expr = a.alloc(Expr::Identifier(i));
        let sq: &Expr = a.alloc(Expr::BinaryOp { op: BinaryOpKind::Multiply, left: idx, right: idx });
        let one: &Expr = a.alloc(Expr::Literal(Literal::Number(1)));
        let body: &Expr = a.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: sq, right: one });
        let expr = lower_expr_to_genexpr(body, i).expect("f(i)=i*i+1 lowers");

        let f = RuntimeValue::Function(Box::new(ClosureValue {
            body_index: usize::MAX,
            captured_env: std::collections::HashMap::default(),
            param_names: vec![i],
            generated: Some(Rc::new(expr.clone())),
        }));
        let bytes = message_to_wire("p", &f).expect("a generated function is sendable");
        match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::Function(c) => {
                let g = c.generated.expect("decoded function carries its generator");
                assert_eq!(&*g, &expr, "the generator round-trips exactly");
                assert_eq!(c.param_names.len(), 1, "arity preserved");
                for x in -20..20i64 {
                    assert_eq!(gen_eval(&g, x), x * x + 1, "the shipped function evaluates f(x) on the receiver");
                }
            }
            _ => panic!("expected a Function back"),
        }

        let plain = RuntimeValue::Function(Box::new(ClosureValue {
            body_index: 0,
            captured_env: std::collections::HashMap::default(),
            param_names: vec![],
            generated: None,
        }));
        assert!(message_to_wire("p", &plain).is_err(), "an un-lowered closure is still not sendable");
    }

    #[test]
    fn receiver_refuses_a_shipped_computation_when_it_declines_code() {
        // THE EXECUTABLE-PAYLOAD GATE. The ONLY "code" the wire can carry is a bounded GenExpr sandbox
        // (never arbitrary bytecode — un-lowered closures are unsendable, proven above). A receiver that
        // declares `accept_computed: false` REFUSES even that, at decode — the first of three gates
        // against running code you didn't ask for (the C2 acceptance contract gates INVOCATION; the
        // sandbox bounds what an accepted call can do).
        use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
        use logicaffeine_base::{Arena, Symbol};
        let a: Arena<Expr> = Arena::new();
        let i = Symbol::from_index(0);
        let idx: &Expr = a.alloc(Expr::Identifier(i));
        let one: &Expr = a.alloc(Expr::Literal(Literal::Number(1)));
        let body: &Expr = a.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: idx, right: one });
        let expr = lower_expr_to_genexpr(body, i).expect("f(i)=i+1 lowers");
        let f = RuntimeValue::Function(Box::new(ClosureValue {
            body_index: usize::MAX,
            captured_env: std::collections::HashMap::default(),
            param_names: vec![i],
            generated: Some(Rc::new(expr)),
        }));
        let bytes = message_to_wire("p", &f).expect("a generated function is sendable");
        // Default: the computation decodes (it stays INERT until invoked through an acceptance contract).
        assert!(
            message_from_wire(&bytes).is_some(),
            "by default a shipped computation decodes (inert data until a contract invokes it)"
        );
        // A receiver that declines code refuses to decode it at all.
        let no_code = ReceiveLimits { accept_computed: false, ..Default::default() };
        assert!(
            with_receive_limits(no_code, || message_from_wire(&bytes)).is_none(),
            "a receiver with accept_computed=false must REFUSE a shipped computation at decode"
        );
    }

    #[test]
    fn wire_auto_modular_affine_ships_a_generator() {
        // A sawtooth column `a + b·(i mod p)` ships a tiny GenExpr, not the values, bit-exact.
        let v: Vec<i64> = (0..5000i64).map(|i| 1000 + 7 * (i % 12)).collect();
        let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
        assert_eq!(got, v, "the modular-affine column reconstructs bit-exact");
        let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
        assert!(
            auto.len() < varint.len() / 50,
            "the generator ({} B) must ship ≪ the data ({} B)",
            auto.len(),
            varint.len()
        );
    }

    #[test]
    fn wire_auto_byte_column_ships_a_tight_zero_copy_blob() {
        // Wide-range, STRUCTURE-FREE bytes (deterministic random, no affine/geometric/periodic
        // law to exploit) ship as a tight 1-byte-per-element blob AND read back in place as
        // `&[u8]` with zero copy — the first-class binary type (capnp `Data`, protobuf `bytes`)
        // that FOR's bit-packing can't offer. Beats varint (2 bytes on every ≥128). The data is
        // random precisely so the GENERATORS (which legitimately win on structured columns) don't
        // preempt the blob — arbitrary binary is exactly when the zero-copy blob is the best form.
        let mut rng = SplitMix64 { state: 0x6B70_B10B_CAFE_F00D };
        let data: Vec<i64> = (0..1000).map(|_| (rng.next() % 256) as i64).collect();
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(data.clone()))));
        let bytes = with_structure(WireStructure::Auto, || message_to_wire("p", &v).unwrap());

        match &message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Ints(g) => assert_eq!(g, &data, "byte column round-trips bit-exact"),
                _ => panic!("expected Ints"),
            },
            _ => panic!("expected List"),
        }
        assert!(bytes.len() <= data.len() + 32, "tight blob: {} bytes for {} values", bytes.len(), data.len());

        let view = view_message(&bytes).unwrap();
        let slice = view.as_byte_slice().expect("a byte column reads zero-copy as &[u8]");
        assert_eq!(slice.len(), data.len(), "all bytes present");
        for (i, &b) in slice.iter().enumerate() {
            assert_eq!(b as i64, data[i], "byte {i} matches");
        }
        let base = bytes.as_ptr() as usize;
        let lo = slice.as_ptr() as usize;
        assert!(lo >= base && lo < base + bytes.len(), "the &[u8] borrows the message buffer (zero-copy)");

        let varint = with_structure(WireStructure::Off, || message_to_wire("p", &v).unwrap());
        assert!(bytes.len() < varint.len(), "byte blob ({}) beats varint ({})", bytes.len(), varint.len());
    }

    #[test]
    fn wire_narrow_byte_column_prefers_for_when_smaller() {
        // Narrow-range bytes (0..16) bit-pack SMALLER via FOR (≈4 bits each) than a 1-byte
        // blob, so the menu correctly chooses FOR — `T_BYTES` is only selected when it is
        // genuinely the best, never as a redundant default.
        let data: Vec<i64> = (0..1000).map(|i: i64| (i * 7).rem_euclid(16)).collect();
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(data.clone()))));
        let bytes = with_structure(WireStructure::Auto, || message_to_wire("p", &v).unwrap());

        match &message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Ints(g) => assert_eq!(g, &data, "narrow byte column round-trips"),
                _ => panic!("expected Ints"),
            },
            _ => panic!("expected List"),
        }
        assert!(bytes.len() < data.len(), "narrow bytes pack below 1 byte/element ({} for {})", bytes.len(), data.len());
        assert!(
            view_message(&bytes).unwrap().as_byte_slice().is_none(),
            "the narrow column uses FOR/dict, not a T_BYTES blob"
        );
    }

    #[test]
    fn wire_gen_decode_rejects_hostile_trees() {
        // Sandbox safety: an over-deep tree is rejected (not stack-overflowed); garbage and
        // truncated trees return None; a T_GEN value with a hostile body decodes to None —
        // never a panic, never unbounded work.
        let mut deep = GenExpr::Index;
        for _ in 0..(MAX_GEN_DEPTH + 5) {
            deep = GenExpr::Add(Box::new(deep), Box::new(GenExpr::Const(1)));
        }
        let mut sbytes = Vec::new();
        serialize_gen(&deep, &mut sbytes);
        let mut p = 0;
        let mut budget = MAX_GEN_NODES;
        assert!(deserialize_gen(&sbytes, &mut p, &mut budget, 0).is_none(), "over-deep tree rejected");

        let mut p = 0;
        let mut budget = MAX_GEN_NODES;
        assert!(deserialize_gen(&[99u8], &mut p, &mut budget, 0).is_none(), "garbage node tag → None");

        let mut p = 0;
        let mut budget = MAX_GEN_NODES;
        assert!(deserialize_gen(&[2u8], &mut p, &mut budget, 0).is_none(), "truncated binary op → None");

        let mut p = 0;
        assert!(native_decode(&[T_GEN, 99u8, 5u8], &mut p).is_none(), "garbage T_GEN body → None, no panic");
    }

    #[test]
    fn wire_matrix_blast_every_knob_combo_composes() {
        // MATRIX BLAST: the full Cartesian product of every codec knob, over representative
        // payloads. The risk in chaining knobs is INTERFERENCE — one silently corrupting
        // another. For each (payload × numerics × structure × floats × compression × integrity
        // × struct-view) we (1) encode, (2) decode without panic, and (3) prove the decoded
        // value is canonically IDENTICAL to the original by re-normalizing both to one fixed
        // baseline encoding and comparing bytes. So every combination must round-trip exactly.
        // Adding a knob = adding one loop dimension here; nothing else can quietly break.
        fn li(v: Vec<i64>) -> RuntimeValue {
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))))
        }
        fn lf(v: Vec<f64>) -> RuntimeValue {
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))))
        }
        fn rec(id: i64, name: &str, active: bool) -> RuntimeValue {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(id));
            f.insert("name".to_string(), RuntimeValue::Text(Rc::new(name.to_string())));
            f.insert("active".to_string(), RuntimeValue::Bool(active));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "Rec".to_string(), fields: f }))
        }
        let names = ["a", "bb", "ccc", "dddd"];
        let payloads: Vec<(&str, RuntimeValue)> = vec![
            ("scalar int", RuntimeValue::Int(-123456789)),
            ("bigint", RuntimeValue::Int(i64::MIN)),
            ("text", RuntimeValue::Text(Rc::new("hello, wire".to_string()))),
            ("bool", RuntimeValue::Bool(true)),
            ("nothing", RuntimeValue::Nothing),
            ("random ints", li((0..64).map(|i: i64| i.wrapping_mul(2_654_435_761)).collect())),
            ("monotone ints", li((0..64i64).scan(1000i64, |s, i| { *s += 1 + (i % 3); Some(*s) }).collect())),
            ("poly ints", li((0..64).map(|i: i64| 3 * i * i - 5 * i + 7).collect())),
            ("sawtooth ints", li((0..64).map(|i: i64| 100 + 5 * (i % 9)).collect())),
            ("clustered ints", li((0..64).map(|_| 1_000_000).collect())),
            ("wide bytes", li((0..64).map(|i: i64| (i * 37 + 128).rem_euclid(256)).collect())),
            ("narrow bytes", li((0..64).map(|i: i64| (i * 7).rem_euclid(16)).collect())),
            ("floats", lf((0..64).map(|i| i as f64 * 1.5 - 7.0).collect())),
            ("special floats", lf(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, f64::MIN_POSITIVE])),
            ("bools", RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..64).map(|i| RuntimeValue::Bool(i % 3 == 0)).collect(),
            ))))),
            ("strings", RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..64).map(|i| RuntimeValue::Text(Rc::new(names[i % 4].to_string()))).collect(),
            ))))),
            ("struct", rec(7, "lone", false)),
            ("struct list", RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..16).map(|i| rec(i, names[i as usize % 4], i % 2 == 0)).collect(),
            ))))),
            ("empty list", li(vec![])),
            ("int→int map", {
                let mut m = MapStorage::default();
                for k in 0..32i64 { m.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * k)); }
                RuntimeValue::Map(Rc::new(RefCell::new(m)))
            }),
            ("int→string map", {
                let mut m = MapStorage::default();
                for k in 0..16i64 { m.insert(RuntimeValue::Int(k * 10), RuntimeValue::Text(Rc::new(format!("row_{k}")))); }
                RuntimeValue::Map(Rc::new(RefCell::new(m)))
            }),
            ("int→struct map", {
                // kind-0 values respond to the dials (struct_view etc.) — composes the map crush with
                // the struct encoders across every combo.
                let mut m = MapStorage::default();
                for k in 0..8i64 { m.insert(RuntimeValue::Int(k), rec(k, names[k as usize % 4], k % 2 == 0)); }
                RuntimeValue::Map(Rc::new(RefCell::new(m)))
            }),
            ("text→int map", {
                let mut m = MapStorage::default();
                for k in 0..8i64 { m.insert(RuntimeValue::Text(Rc::new(format!("k{k}"))), RuntimeValue::Int(k)); }
                RuntimeValue::Map(Rc::new(RefCell::new(m)))
            }),
        ];

        // The fixed normalizer: encode under one canonical knob set so two values that are
        // canonically equal produce identical bytes regardless of how they were transmitted.
        let canon = |val: &RuntimeValue| -> Vec<u8> {
            with_numerics(WireNumerics::Varint, || {
                with_structure(WireStructure::Off, || {
                    with_floats(WireFloats::Memcpy, || {
                        message_to_wire_with("c", val, WireCodec::Native, WireIntegrity::Raw).unwrap()
                    })
                })
            })
        };

        let mut combos = 0u64;
        for (name, v) in &payloads {
            let want = canon(v);
            for num in [WireNumerics::Varint, WireNumerics::Fixed, WireNumerics::GroupVarint] {
                for st in [WireStructure::Off, WireStructure::Affine, WireStructure::Auto] {
                    for fl in [WireFloats::Memcpy, WireFloats::XorDelta] {
                        for comp in
                            [WireCompression::None, WireCompression::Deflate, WireCompression::Lz4, WireCompression::Zstd]
                        {
                            for integ in [WireIntegrity::Raw, WireIntegrity::Checked] {
                                for sv in [false, true] {
                                    let bytes = with_numerics(num, || {
                                        with_structure(st, || {
                                            with_floats(fl, || {
                                                with_compression_codec(comp, || {
                                                    with_struct_view(sv, || {
                                                        message_to_wire_with("p", v, WireCodec::Native, integ).unwrap()
                                                    })
                                                })
                                            })
                                        })
                                    });
                                    let back = message_from_wire(&bytes).unwrap_or_else(|| {
                                        panic!(
                                            "{name} failed to decode under num={num:?} st={st:?} fl={fl:?} \
                                             comp={comp:?} integ={integ:?} sv={sv}"
                                        )
                                    });
                                    assert_eq!(
                                        canon(&back.1),
                                        want,
                                        "{name} corrupted under num={num:?} st={st:?} fl={fl:?} \
                                         comp={comp:?} integ={integ:?} sv={sv}"
                                    );
                                    combos += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(combos >= 4000, "matrix should blast thousands of knob combos, ran {combos}");
    }

    #[test]
    fn wire_shared_type_id_composes_with_every_knob() {
        // Type-id elision (`shared`) drops struct names to a small id. This focuses on STRUCT
        // payloads — the only shapes type-id touches — and verifies the elided form COMPOSES
        // with every other knob: encode + decode under a registry, across all dials, must
        // decode canonically identical to the self-describing form. Kept small (structs only)
        // so it stays quick while still chaining the registry knob against everything else.
        fn rec(id: i64, name: &str, active: bool) -> RuntimeValue {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(id));
            f.insert("name".to_string(), RuntimeValue::Text(Rc::new(name.to_string())));
            f.insert("active".to_string(), RuntimeValue::Bool(active));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "Rec".to_string(), fields: f }))
        }
        let names = ["a", "bb", "ccc"];
        let payloads = vec![
            rec(7, "lone", false),
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..12).map(|i| rec(i, names[i as usize % 3], i % 2 == 0)).collect(),
            )))),
        ];
        let mk_reg = || {
            WireTypeRegistry::new(vec![(
                "Rec".to_string(),
                vec!["active".to_string(), "id".to_string(), "name".to_string()],
            )])
        };
        let canon = |val: &RuntimeValue| -> Vec<u8> {
            with_numerics(WireNumerics::Varint, || {
                with_structure(WireStructure::Off, || {
                    message_to_wire_with("c", val, WireCodec::Native, WireIntegrity::Raw).unwrap()
                })
            })
        };
        let mut combos = 0u32;
        for v in &payloads {
            let want = canon(v);
            for num in [WireNumerics::Varint, WireNumerics::Fixed, WireNumerics::GroupVarint] {
                for st in [WireStructure::Off, WireStructure::Affine, WireStructure::Auto] {
                    for comp in [WireCompression::None, WireCompression::Zstd] {
                        for integ in [WireIntegrity::Raw, WireIntegrity::Checked] {
                            for sv in [false, true] {
                                let enc = || {
                                    with_numerics(num, || {
                                        with_structure(st, || {
                                            with_compression_codec(comp, || {
                                                with_struct_view(sv, || {
                                                    message_to_wire_with("p", v, WireCodec::Native, integ).unwrap()
                                                })
                                            })
                                        })
                                    })
                                };
                                // Type-id active for BOTH encode and decode (the id must resolve).
                                let bytes = with_type_registry(mk_reg(), enc);
                                let back = with_type_registry(mk_reg(), || message_from_wire(&bytes))
                                    .expect("a type-id-elided struct decodes with the registry");
                                assert_eq!(
                                    canon(&back.1),
                                    want,
                                    "shared struct corrupted under num={num:?} st={st:?} comp={comp:?} integ={integ:?} sv={sv}"
                                );
                                combos += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(combos >= 100, "shared-knob matrix ran {combos} combos");
    }

    #[test]
    fn wire_auto_rle_wins_on_runs() {
        // 20 runs of 30 identical values → run-length collapses each run to one pair.
        let mut v = Vec::new();
        for k in 0..20i64 {
            for _ in 0..30 {
                v.push(k);
            }
        }
        let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
        let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
        assert_eq!(got, v, "run-length round-trips bit-exact");
        assert!(auto.len() < varint.len(), "Auto ({}) must beat varint ({}) on runs", auto.len(), varint.len());
    }

    #[test]
    fn wire_auto_dict_wins_on_low_cardinality() {
        // Five scattered distinct values (no runs, so RLE can't win) → dictionary + narrow
        // bit-packed indices.
        let mut rng = SplitMix64 { state: 0x0000_BEEF };
        let palette = [7i64, 42, 1000, -5, 999_999];
        let v: Vec<i64> = (0..500).map(|_| palette[(rng.next() % 5) as usize]).collect();
        let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
        let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
        assert_eq!(got, v, "dictionary round-trips bit-exact");
        assert!(auto.len() < varint.len(), "Auto ({}) must beat varint ({}) on low cardinality", auto.len(), varint.len());
    }

    #[test]
    fn wire_auto_never_worse_than_varint_on_random() {
        // Full-range random columns have no structure: the selector always includes the
        // plain-varint baseline, so Auto is never larger — and it still round-trips exactly.
        let mut rng = SplitMix64 { state: 0x00C0_FFEE };
        for _ in 0..50 {
            let n = (rng.next() % 200) as usize;
            let v: Vec<i64> = (0..n).map(|_| rng.next() as i64).collect();
            let (auto, got) = affine_roundtrip(&v, WireStructure::Auto);
            let (varint, _) = affine_roundtrip(&v, WireStructure::Off);
            assert_eq!(got, v, "Auto round-trips bit-exact on random data");
            assert!(auto.len() <= varint.len(), "Auto ({}) must never exceed varint ({})", auto.len(), varint.len());
        }
    }

    #[test]
    fn wire_auto_decoder_never_panics_on_mutated_menu_messages() {
        // Every compression-menu tag, byte-mutated, must fail cleanly (None) — never panic
        // or over-allocate (the RLE/bit-pack lengths are attacker-controlled here).
        let mut rng = SplitMix64 { state: 0x00AB_1234 };
        let shapes: Vec<Vec<i64>> = vec![
            (0..50i64).collect(),
            vec![5i64; 80],
            (0..60i64).map(|i| 1_000_000 + (i % 8)).collect(),
            (0..70i64).map(|i| 1_700_000_000 + i * 7).collect(),
        ];
        for shape in &shapes {
            let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(shape.clone()))));
            let base = with_structure(WireStructure::Auto, || {
                message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            for _ in 0..2000 {
                let mut m = base.clone();
                let i = (rng.next() as usize) % m.len();
                m[i] ^= (rng.next() & 0xFF) as u8;
                let _ = message_from_wire(&m); // must not panic
            }
        }
    }

    #[test]
    fn wire_intkey_map_decoder_never_panics_on_mutation() {
        // The T_MAP_INTKEY two-column form (int key column + value-kind byte + value column / per-value
        // list) is fully attacker-controlled on decode: the kind byte, both column lengths, and the
        // kind-0 value count can each be flipped. Byte-mutating both an int→int (kind 1) and an
        // int→text (kind 0) map message must fail cleanly — never panic, never over-allocate.
        let mut rng = SplitMix64 { state: 0x00CD_5678 };
        let mk_int = {
            let mut m = MapStorage::default();
            for k in 0..40i64 {
                m.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * 3));
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        };
        let mk_text = {
            let mut m = MapStorage::default();
            for k in 0..12i64 {
                m.insert(RuntimeValue::Int(k * 100), RuntimeValue::Text(Rc::new(format!("v{k}"))));
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        };
        let mk_struct = {
            let mut m = MapStorage::default();
            for k in 0..10i64 {
                let mut f = HashMap::new();
                f.insert("id".to_string(), RuntimeValue::Int(k));
                f.insert("ok".to_string(), RuntimeValue::Bool(k % 2 == 0));
                m.insert(
                    RuntimeValue::Int(k),
                    RuntimeValue::Struct(Box::new(StructValue { type_name: "R".to_string(), fields: f })),
                );
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        };
        for value in [mk_int, mk_text, mk_struct] {
            let base =
                message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap();
            for _ in 0..3000 {
                let mut b = base.clone();
                let i = (rng.next() as usize) % b.len();
                b[i] ^= (rng.next() & 0xFF) as u8;
                let _ = message_from_wire(&b); // must not panic / OOM
            }
        }
    }

    /// Encode an `Ints` list through the LEB128 varint path (structure off, the default
    /// `Varint` numerics), decode it, and return (bytes, recovered) — the oracle for the
    /// adaptive sign-mode encoding.
    fn varint_roundtrip(v: &[i64]) -> (Vec<u8>, Vec<i64>) {
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v.to_vec()))));
        let bytes = with_structure(WireStructure::Off, || {
            message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let (_, back) = message_from_wire(&bytes).expect("decode");
        let got = match back {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Ints(g) => g.clone(),
                _ => panic!("expected an Ints list back"),
            },
            _ => panic!("expected a List back"),
        };
        (bytes, got)
    }

    #[test]
    fn a_non_negative_int_column_skips_zigzag_for_half_the_size() {
        // 100 copies of 64: plain LEB128 is one byte each (64 < 128); zigzag would spend
        // two (zigzag(64) = 128). The non-negative column must skip zigzag — matching
        // protobuf's `int64` instead of paying its `sint64` doubling penalty.
        let v = vec![64i64; 100];
        let (bytes, got) = varint_roundtrip(&v);
        assert_eq!(got, v, "non-negative round-trip must be exact");
        assert!(
            bytes.len() < 150,
            "a non-negative column must use plain LEB128 (≈100B), not zigzag (≈200B); got {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn a_column_with_any_negative_uses_zigzag_and_round_trips() {
        let v = vec![-1i64, -64, 5, -100, 0, 127];
        let (_, got) = varint_roundtrip(&v);
        assert_eq!(got, v, "mixed-sign round-trip must be exact");
    }

    #[test]
    fn adaptive_sign_mode_round_trips_random_columns() {
        // Fill with RANDOM data (a deterministic xorshift, so the test is reproducible)
        // to be honest about the sign-mode decision: ~half the columns are forced
        // all-non-negative (exercising plain LEB128), the rest are full-range i64
        // (exercising zig-zag). Every column must round-trip exactly.
        let mut rng = 0x1234_5678_9ABC_DEF0u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for _ in 0..3000 {
            let n = (next() % 80) as usize;
            let force_nonneg = next() & 1 == 0;
            let v: Vec<i64> = (0..n)
                .map(|_| {
                    let r = next() as i64;
                    if force_nonneg {
                        r & i64::MAX // clear the sign bit → non-negative
                    } else {
                        r
                    }
                })
                .collect();
            let (_, got) = varint_roundtrip(&v);
            assert_eq!(got, v, "random adaptive sign-mode round-trip failed for {v:?}");
        }
    }

    #[test]
    fn adaptive_sign_mode_round_trips_every_boundary() {
        let cases: Vec<Vec<i64>> = vec![
            vec![],
            vec![0i64; 10],
            vec![1, 2, 3, 63, 64, 65, 127, 128, 255, 256],
            vec![-1, -2, -63, -64, -128, -129],
            vec![i64::MIN, i64::MAX, 0, -1, 1],
            vec![i64::MAX, i64::MAX - 1], // all non-negative, max magnitude
        ];
        for v in cases {
            let (_, got) = varint_roundtrip(&v);
            assert_eq!(got, v, "adaptive sign-mode round-trip failed for {v:?}");
        }
    }

    #[test]
    fn wireview_reads_any_element_in_place_without_decoding() {
        // A million-element fixed-width int column. The view reads element N directly at
        // its byte offset — no full decode, no allocation, any index.
        let v: Vec<i64> = (0..1_000_000).map(|i| i as i64 * 3 - 7).collect();
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v.clone()))));
        let bytes = with_numerics(WireNumerics::Fixed, || {
            message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let view = view_message(&bytes).expect("view opens over the fixed-layout message");
        assert_eq!(view.int_list_len(), Some(1_000_000));
        for &i in &[0usize, 1, 12_345, 678_901, 999_999] {
            assert_eq!(view.int_list_get(i), Some(v[i]), "random-access element {i}");
        }
        assert_eq!(view.int_list_get(1_000_000), None, "out-of-bounds is None, not a panic");
    }

    #[test]
    fn wireview_varint_and_float_and_scalar_views_agree_with_decode() {
        // The view must read varint-layout ints, memcpy floats, and scalars in agreement
        // with a full decode — every layout, not just fixed.
        let ints: Vec<i64> = vec![-5, 0, 7, 200, -3000, i64::MAX, i64::MIN];
        let iv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(ints.clone()))));
        let ib = with_numerics(WireNumerics::Varint, || {
            message_to_wire_with("", &iv, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let view = view_message(&ib).unwrap();
        assert_eq!(view.int_list_len(), Some(ints.len()));
        for (i, &x) in ints.iter().enumerate() {
            assert_eq!(view.int_list_get(i), Some(x), "varint element {i}");
        }
        let flts = vec![1.5f64, -2.25, 3.0e10, f64::MIN, f64::MAX];
        let fv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(flts.clone()))));
        let fb = message_to_wire_with("", &fv, WireCodec::Native, WireIntegrity::Raw).unwrap();
        let fview = view_message(&fb).unwrap();
        assert_eq!(fview.float_list_len(), Some(flts.len()));
        for (i, &x) in flts.iter().enumerate() {
            assert_eq!(fview.float_list_get(i), Some(x), "float element {i}");
        }
        let sb = message_to_wire_with("", &RuntimeValue::Int(-42), WireCodec::Native, WireIntegrity::Raw).unwrap();
        assert_eq!(view_message(&sb).unwrap().as_int(), Some(-42));
    }

    #[test]
    fn wireview_single_field_read_is_far_cheaper_than_full_decode() {
        // The zero-copy win: reading ONE element of a 1M-element message — even a THOUSAND
        // of them — must be far cheaper than ONE full decode (O(1) access, no bulk copy).
        // A same-run comparison, so it is load-invariant.
        let v: Vec<i64> = (0..1_000_000).map(|i| i as i64).collect();
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))));
        let bytes = with_numerics(WireNumerics::Fixed, || {
            message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        let reads = {
            let t = std::time::Instant::now();
            for _ in 0..1000 {
                let view = view_message(&bytes).unwrap();
                std::hint::black_box(view.int_list_get(999_999));
            }
            t.elapsed().as_nanos()
        };
        let full = {
            let t = std::time::Instant::now();
            std::hint::black_box(message_from_wire(&bytes).unwrap());
            t.elapsed().as_nanos()
        };
        assert!(
            reads < full,
            "1000 zero-copy field reads ({reads}ns) must be cheaper than ONE full decode ({full}ns)"
        );
    }

    #[test]
    fn wireview_open_is_o1_even_with_a_checksum() {
        // "Raw for certain": `view_message` never re-hashes the body, so opening a CHECKED
        // message (the default) and reading one element is O(1) — exactly as for a Raw one.
        // (Cap'n Proto / Arrow carry no checksum at all; our zero-copy view matches their
        // cost, while the FULL decode path still validates.) This would FAIL if the view
        // verified the 8 MB body's FNV sum on open.
        let v: Vec<i64> = (0..1_000_000).map(|i| i as i64).collect();
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))));
        let bytes = with_numerics(WireNumerics::Fixed, || message_to_wire("", &value).unwrap());
        assert!(bytes[0] & H_CHECKED != 0, "the default message carries a checksum");
        let reads = {
            let t = std::time::Instant::now();
            for _ in 0..1000 {
                let view = view_message(&bytes).unwrap();
                std::hint::black_box(view.int_list_get(999_999));
            }
            t.elapsed().as_nanos()
        };
        let full = {
            let t = std::time::Instant::now();
            std::hint::black_box(message_from_wire(&bytes).unwrap());
            t.elapsed().as_nanos()
        };
        assert!(
            reads < full,
            "1000 view reads of a checksummed message ({reads}ns) must beat one full decode ({full}ns)"
        );
    }

    #[test]
    fn wireview_rejects_compressed_and_malformed() {
        // The view is over raw native bytes; a compressed message returns None (it must be
        // inflated first), and a truncated/garbage frame never panics. Use REPETITIVE data
        // so zstd actually shrinks it (the codec keeps compression only when it helps).
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(
            (0..4000).map(|i| (i % 4) as i64 * 1000).collect(),
        ))));
        let compressed = with_compression_codec(WireCompression::Zstd, || {
            message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap()
        });
        assert!(compressed[0] & 0x02 != 0, "the test payload must actually be compressed");
        assert!(view_message(&compressed).is_none(), "a compressed message has no in-place view");
        assert!(view_message(&[]).is_none(), "empty");
        assert!(view_message(&[0xFF, 0x00, 0x01]).is_none(), "garbage header");
    }

    #[test]
    fn structure_off_is_the_default_and_leaves_bytes_unchanged() {
        let v: Vec<i64> = (0..50).collect();
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))));
        let default = message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap();
        let (off, _) = {
            let b = with_structure(WireStructure::Off, || {
                message_to_wire_with("", &value, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            (b, ())
        };
        assert_eq!(default, off, "Off must equal the default path byte-for-byte (no regression)");
    }

    /// A BigInt beyond i64 — built straight from a decimal so the test does not depend
    /// on the arithmetic layer.
    fn big(decimal: &str) -> RuntimeValue {
        RuntimeValue::from_bigint(logicaffeine_base::BigInt::parse_decimal(decimal).unwrap())
    }

    #[test]
    fn bigint_round_trips_through_the_binary_wire_exactly() {
        let v = big("123456789012345678901234567890");
        let bytes = message_to_wire("", &v).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert_eq!(back.to_display_string(), "123456789012345678901234567890");
        assert!(matches!(back, RuntimeValue::BigInt(_)), "stays a BigInt across the wire");
    }

    #[test]
    fn negative_bigint_round_trips_through_the_wire() {
        let v = big("-99999999999999999999999999999");
        let bytes = message_to_wire("", &v).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert_eq!(back.to_display_string(), "-99999999999999999999999999999");
    }

    #[test]
    fn bigint_round_trips_through_cross_task_materialize_rebuild() {
        let v = big("340282366920938463463374607431768211456"); // 2^128
        let back = rebuild(materialize(&v).unwrap());
        assert_eq!(back.to_display_string(), "340282366920938463463374607431768211456");
    }

    /// An exact base-10 fixed-point, built straight from its literal so the test does
    /// not depend on the arithmetic layer.
    fn dec_lit(s: &str) -> RuntimeValue {
        RuntimeValue::Decimal(Rc::new(logicaffeine_base::Decimal::parse(s).unwrap()))
    }

    #[test]
    fn decimal_round_trips_through_the_binary_wire_preserving_scale() {
        // Money must survive the wire bit-exact, SCALE INCLUDED: `100.00` comes back
        // `100.00`, not `100`, and `0.10` keeps its trailing zero.
        for s in ["19.99", "-0.005", "100.00", "0", "0.10", "123456789.000001"] {
            let bytes = message_to_wire("", &dec_lit(s)).unwrap();
            let (_, back) = message_from_wire(&bytes).unwrap();
            assert!(matches!(back, RuntimeValue::Decimal(_)), "stays a Decimal across the wire: {s}");
            assert_eq!(back.to_display_string(), s, "scale + value preserved exactly: {s}");
        }
    }

    #[test]
    fn decimal_round_trips_through_cross_task_materialize_rebuild() {
        let v = dec_lit("19.99");
        let back = rebuild(materialize(&v).unwrap());
        assert!(matches!(back, RuntimeValue::Decimal(_)));
        assert_eq!(back.to_display_string(), "19.99");
    }

    #[test]
    fn decimal_money_survives_wire_where_a_json_float_would_drift() {
        // `0.30` crosses the wire bit-exact, unlike the 0.30000000000000004 a JSON/f64
        // consumer would carry — the "JSON numbers ruin lives" footgun, removed for money.
        let bytes = message_to_wire("", &dec_lit("0.30")).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert_eq!(back.to_display_string(), "0.30");
        assert_ne!(0.1_f64 + 0.2, 0.3);
    }

    /// An exact complex from rational parts (`re_n/re_d + im_n/im_d · i`).
    fn cplx(re: (i64, i64), im: (i64, i64)) -> RuntimeValue {
        RuntimeValue::Complex(Rc::new(logicaffeine_base::Complex::new(
            logicaffeine_base::Rational::from_ratio_i64(re.0, re.1).unwrap(),
            logicaffeine_base::Rational::from_ratio_i64(im.0, im.1).unwrap(),
        )))
    }

    #[test]
    fn complex_round_trips_through_the_binary_wire_exactly() {
        // Both rational parts (and `i·i = −1` exactness) survive the wire bit-for-bit.
        for z in [
            cplx((3, 1), (4, 1)),   // 3+4i
            cplx((0, 1), (1, 1)),   // i
            cplx((0, 1), (-1, 1)),  // -i
            cplx((-1, 2), (3, 4)),  // -1/2 + 3/4 i
            cplx((7, 1), (0, 1)),   // a real (7+0i)
        ] {
            let bytes = message_to_wire("", &z).unwrap();
            let (_, back) = message_from_wire(&bytes).unwrap();
            assert!(matches!(back, RuntimeValue::Complex(_)), "stays Complex on the wire");
            assert_eq!(back, z, "exact complex survives the wire");
        }
    }

    #[test]
    fn complex_round_trips_through_cross_task_materialize_rebuild() {
        let z = cplx((3, 1), (-4, 1));
        let back = rebuild(materialize(&z).unwrap());
        assert!(matches!(back, RuntimeValue::Complex(_)));
        assert_eq!(back, z);
    }

    fn modu(v: i64, n: i64) -> RuntimeValue {
        RuntimeValue::Modular(Rc::new(logicaffeine_base::Modular::from_i64(v, n).unwrap()))
    }

    #[test]
    fn modular_round_trips_through_the_binary_wire_preserving_the_ring() {
        // Both the residue AND the modulus survive the wire — the ℤ/nℤ ring is preserved.
        for z in [modu(3, 7), modu(0, 13), modu(10, 7), modu(123456, 1_000_003)] {
            let bytes = message_to_wire("", &z).unwrap();
            let (_, back) = message_from_wire(&bytes).unwrap();
            assert!(matches!(back, RuntimeValue::Modular(_)), "stays Modular on the wire");
            assert_eq!(back, z, "exact residue + modulus survive the wire");
        }
    }

    #[test]
    fn modular_round_trips_through_cross_task_materialize_rebuild() {
        let z = modu(42, 97);
        let back = rebuild(materialize(&z).unwrap());
        assert!(matches!(back, RuntimeValue::Modular(_)));
        assert_eq!(back, z);
    }

    /// A quantity built straight from a magnitude + unit, independent of the arithmetic layer.
    fn qty(num: i64, den: i64, unit: &str) -> RuntimeValue {
        let unit = logicaffeine_base::quantity::units::by_name(unit).unwrap();
        let mag = logicaffeine_base::Rational::new(
            logicaffeine_base::BigInt::from_i64(num),
            logicaffeine_base::BigInt::from_i64(den),
        )
        .unwrap();
        RuntimeValue::Quantity(Rc::new(crate::interpreter::QuantityValue {
            q: logicaffeine_base::Quantity::of(mag, &unit),
            unit,
        }))
    }

    #[test]
    fn quantity_round_trips_through_the_binary_wire_exactly() {
        // Value, dimension, AND display unit all survive — including a fractional magnitude
        // (the golden 42/127 ft) and an affine unit (°C, whose offset rides in the rebuilt unit).
        for (q, shown) in [
            (qty(2, 1, "inch"), "2 in"),
            (qty(20, 1, "celsius"), "20 °C"),
            (qty(5, 1, "kilogram"), "5 kg"),
            (qty(42, 127, "foot"), "42/127 ft"),
        ] {
            let bytes = message_to_wire("", &q).unwrap();
            let (_, back) = message_from_wire(&bytes).unwrap();
            assert!(matches!(back, RuntimeValue::Quantity(_)), "stays a Quantity on the wire: {shown}");
            assert_eq!(back.to_display_string(), shown, "value + unit preserved exactly: {shown}");
            assert_eq!(back, q, "physical equality preserved: {shown}");
        }
    }

    #[test]
    fn quantity_round_trips_through_cross_task_materialize_rebuild() {
        let q = qty(42, 127, "foot");
        let back = rebuild(materialize(&q).unwrap());
        assert!(matches!(back, RuntimeValue::Quantity(_)));
        assert_eq!(back.to_display_string(), "42/127 ft");
        assert_eq!(back, q);
    }

    // ===================================================================================
    // TYPE CONFORMANCE / LOCK-IN HARNESS
    //
    // The contract every first-class value type must honor, and the lock that makes it
    // impossible to add a new `RuntimeValue` variant without deciding its conformance.
    // ===================================================================================

    /// Conformance class of a runtime value.
    #[derive(PartialEq, Eq, Debug)]
    enum ConfClass {
        /// A first-class value: must pass FULL conformance — `type_name`, `to_display_string`,
        /// reflexive equality, and lossless wire round-trip (both cross-task and binary).
        Value,
        /// A container of other values (its own round-trip is tested elsewhere; here we only
        /// require name/display/eq so the harness stays representation-agnostic).
        Container,
        /// An opaque handle or reference with no value-semantics wire identity.
        Opaque,
    }

    /// THE LOCK. This match is exhaustive, so a NEW `RuntimeValue` variant will not compile until it
    /// is classified here — forcing a deliberate decision about its conformance. (Completeness
    /// Doctrine: no type enters the language without passing through the conformance harness.)
    fn conformance_class(v: &RuntimeValue) -> ConfClass {
        match v {
            RuntimeValue::Int(_)
            | RuntimeValue::BigInt(_)
            | RuntimeValue::Rational(_)
            | RuntimeValue::Decimal(_)
            | RuntimeValue::Complex(_)
            | RuntimeValue::Modular(_)
            | RuntimeValue::Float(_)
            | RuntimeValue::Bool(_)
            | RuntimeValue::Text(_)
            | RuntimeValue::Char(_)
            | RuntimeValue::Nothing
            | RuntimeValue::Duration(_)
            | RuntimeValue::Date(_)
            | RuntimeValue::Moment(_)
            | RuntimeValue::Span { .. }
            | RuntimeValue::Time(_)
            | RuntimeValue::Word(_)
            | RuntimeValue::Quantity(_)
            | RuntimeValue::Money(_)
            | RuntimeValue::Uuid(_)
            | RuntimeValue::Peer(_) => ConfClass::Value,
            RuntimeValue::List(_)
            | RuntimeValue::Tuple(_)
            | RuntimeValue::Set(_)
            | RuntimeValue::Map(_)
            | RuntimeValue::Struct(_)
            | RuntimeValue::Inductive(_) => ConfClass::Container,
            RuntimeValue::Function(_)
            | RuntimeValue::Chan(_)
            | RuntimeValue::TaskHandle(_)
            | RuntimeValue::Crdt(_)
            // A SIMD lane vector is a transient compute register: first-class at the AOT tier (the
            // crypto kernels compute over it), but NOT a wire type — you serialize the underlying
            // `Seq`, never the register. So it is not wire-round-tripped (hence Opaque here), yet it
            // IS AOT-wired to a concrete Rust type (declared in `aot_wiring` below).
            | RuntimeValue::Lanes(_) => ConfClass::Opaque,
        }
    }

    /// One representative of every `ConfClass::Value` variant. If you add a value type, the lock above
    /// forces you to classify it; add its representative here so the conformance test exercises it.
    fn value_representatives() -> Vec<RuntimeValue> {
        vec![
            RuntimeValue::Int(7),
            // A genuine BigInt (2^64, beyond i64) so it does not narrow back to Int.
            RuntimeValue::BigInt(Rc::new(logicaffeine_base::BigInt::from_le_bytes(
                false,
                &[0, 0, 0, 0, 0, 0, 0, 0, 1],
            ))),
            // A non-integer rational (1/2) so it stays Rational.
            RuntimeValue::Rational(Rc::new(
                logicaffeine_base::Rational::from_ratio_i64(1, 2).unwrap(),
            )),
            dec_lit("19.99"),
            cplx((3, 1), (4, 1)),
            modu(3, 7),
            RuntimeValue::Float(2.5),
            RuntimeValue::Bool(true),
            RuntimeValue::Text(Rc::new("hi".to_string())),
            RuntimeValue::Char('x'),
            RuntimeValue::Nothing,
            RuntimeValue::Duration(10),
            RuntimeValue::Date(19_000),
            RuntimeValue::Moment(123),
            RuntimeValue::Span { months: 3, days: 14 },
            RuntimeValue::Time(99),
            RuntimeValue::Word(logicaffeine_base::WordVal::from_u64(32, 42).unwrap()),
            qty(42, 127, "foot"),
            RuntimeValue::Money(Rc::new(logicaffeine_base::Money::of(
                logicaffeine_base::Decimal::parse("19.99").unwrap(),
                logicaffeine_base::money::currency::by_code("USD").unwrap(),
            ))),
            RuntimeValue::Uuid(Rc::new(
                logicaffeine_base::Uuid::parse("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            )),
            RuntimeValue::Peer(Rc::new("ws://host:9944".to_string())),
        ]
    }

    /// The compiled-tier (AOT) wiring a runtime value's type MUST have.
    #[derive(Debug)]
    enum AotWiring {
        /// First-class at the AOT tier: the codegen lowers this type's name to this concrete Rust
        /// type (a primitive like `i64`, or a `Logos*` newtype like `LogosMoney`).
        Rust(&'static str),
        /// Deliberately runtime-only: a handle/reference/container with no single AOT value type
        /// (containers lower generically by element; handles never cross into compiled value code).
        RuntimeOnly,
    }

    /// THE AOT WIRING LOCK. Exhaustive over `RuntimeValue`, so a NEW variant will not compile until
    /// its compiled-tier wiring is declared here — you cannot add a value type and forget to wire it
    /// into the AOT codegen. The test below then checks the REAL codegen (`map_type_to_rust`) agrees,
    /// so a type wired into the runtime + wire but missing from AOT FAILS. (Completeness Doctrine:
    /// all types must be wired through EVERY tier, not just the wire.)
    fn aot_wiring(v: &RuntimeValue) -> AotWiring {
        use AotWiring::{Rust, RuntimeOnly};
        match v {
            // A BigInt reports type_name "Int" (same logical type, wider repr) → i64 codegen path.
            RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Rust("i64"),
            RuntimeValue::Rational(_) => Rust("LogosRational"),
            RuntimeValue::Decimal(_) => Rust("LogosDecimal"),
            RuntimeValue::Complex(_) => Rust("LogosComplex"),
            RuntimeValue::Modular(_) => Rust("LogosModular"),
            RuntimeValue::Float(_) => Rust("f64"),
            RuntimeValue::Bool(_) => Rust("bool"),
            RuntimeValue::Text(_) => Rust("String"),
            RuntimeValue::Char(_) => Rust("char"),
            RuntimeValue::Nothing => Rust("()"),
            RuntimeValue::Duration(_) => Rust("std::time::Duration"),
            RuntimeValue::Date(_) => Rust("LogosDate"),
            RuntimeValue::Moment(_) => Rust("LogosMoment"),
            RuntimeValue::Span { .. } => Rust("LogosSpan"),
            RuntimeValue::Time(_) => Rust("LogosTime"),
            // The representative is a 32-bit word; its type_name "Word32" is in the codegen map.
            RuntimeValue::Word(_) => Rust("Word32"),
            // A lane vector is AOT-first-class (the crypto kernels' working registers) even though it
            // is not wire-round-tripped — `map_type_to_rust("Lanes8Word32")` lowers it.
            RuntimeValue::Lanes(_) => Rust("Lanes8Word32"),
            RuntimeValue::Quantity(_) => Rust("LogosQuantity"),
            RuntimeValue::Money(_) => Rust("LogosMoney"),
            RuntimeValue::Uuid(_) => Rust("LogosUuid"),
            // A peer is a runtime networking handle — it round-trips on the wire but is not a
            // compiled-tier value type.
            RuntimeValue::Peer(_) => RuntimeOnly,
            // Containers lower generically (LogosSeq<T>/LogosMap<K,V>/…) via codegen_type_expr, not by
            // a bare name; their own AOT coverage is the collection codegen tests.
            RuntimeValue::List(_)
            | RuntimeValue::Tuple(_)
            | RuntimeValue::Set(_)
            | RuntimeValue::Map(_)
            | RuntimeValue::Struct(_)
            | RuntimeValue::Inductive(_) => RuntimeOnly,
            // Opaque handles never cross into compiled value code.
            RuntimeValue::Function(_)
            | RuntimeValue::Chan(_)
            | RuntimeValue::TaskHandle(_)
            | RuntimeValue::Crdt(_) => RuntimeOnly,
        }
    }

    /// Full conformance for one value: name, display, reflexive equality, and BOTH wire round-trips.
    fn assert_value_conformance(v: &RuntimeValue) {
        let name = v.type_name();
        assert!(!name.is_empty(), "type_name must be non-empty");
        // Display must not panic.
        let _ = v.to_display_string();
        // Equality is reflexive (representatives are finite — no NaN).
        assert!(v == v, "{name} value is not equal to itself");
        // Cross-task marshalling (materialize → rebuild) is the identity.
        let back = rebuild(materialize(v).unwrap_or_else(|_| panic!("{name} did not materialize")));
        assert_eq!(&back, v, "{name} changed across cross-task marshalling");
        assert_eq!(
            conformance_class(&back),
            ConfClass::Value,
            "{name} changed variant across marshalling"
        );
        // Binary wire (encode → decode) is the identity.
        let bytes = message_to_wire("", v).unwrap_or_else(|_| panic!("{name} did not encode to wire"));
        let (_, back2) = message_from_wire(&bytes).unwrap_or_else(|| panic!("{name} did not decode"));
        assert_eq!(&back2, v, "{name} changed across the binary wire");
    }

    #[test]
    fn every_value_type_conforms_name_display_eq_and_both_wire_round_trips() {
        let reps = value_representatives();
        for v in &reps {
            assert_eq!(
                conformance_class(v),
                ConfClass::Value,
                "representative for {} must be a Value",
                v.type_name()
            );
            assert_value_conformance(v);
        }
        // Sanity: the representative set is non-trivial and covers the value types we have shipped.
        assert!(reps.len() >= 20, "expected a representative for every value type");
    }

    #[test]
    fn every_value_type_is_wired_into_the_aot_codegen_tier() {
        // A first-class value must compile to Rust, not just travel the wire. For every Value
        // representative whose AOT wiring spec names a concrete Rust type, the real codegen
        // (`map_type_to_rust`, the single source of truth for type-name → Rust) MUST lower the
        // value's `type_name()` to exactly that. Adding a value type wired into the runtime + wire
        // but missing from the AOT codegen fails here — the gap the wire conformance test alone
        // could not catch. `RuntimeOnly` is the conscious, exhaustive-match-forced exemption (e.g. a
        // networking handle); the exhaustiveness of `aot_wiring` is what makes the decision
        // unavoidable for every new type.
        let mut checked = 0usize;
        for v in &value_representatives() {
            let name = v.type_name();
            if let AotWiring::Rust(expected) = aot_wiring(v) {
                let mapped = crate::codegen::types::map_type_to_rust(name);
                assert_eq!(
                    mapped, expected,
                    "{name}: AOT codegen lowers it to `{mapped}`, but the wiring lock expects `{expected}` \
                     — wire the type into codegen/types.rs::map_type_to_rust"
                );
                // ANALYSIS tier: where the type checker knows this type name, its lowering MUST agree
                // with the codegen — the two type-lowering paths cannot drift. (A type the checker does
                // not yet model by name, e.g. the width-indexed `Word`, returns `Unknown` and is left to
                // the codegen lock above; that exemption is documented, not silent.)
                let lt = crate::analysis::types::LogosType::from_type_name(name);
                if lt != crate::analysis::types::LogosType::Unknown {
                    assert_eq!(
                        lt.to_rust_type(), expected,
                        "{name}: analysis LogosType lowers to `{}`, codegen to `{expected}` — the type checker \
                         and codegen disagree; wire them consistently",
                        lt.to_rust_type()
                    );
                }
                checked += 1;
            }
        }
        // The lock must actually be exercising the AOT tier, not silently skipping everything.
        assert!(checked >= 18, "AOT wiring lock checked only {checked} value types");
    }

    #[test]
    fn compound_dimension_quantity_survives_the_wire_as_its_signature() {
        // A dimension-combining product (Area, empty display symbol) preserves its dimension and
        // renders the signature on the far side.
        let m = logicaffeine_base::quantity::units::by_name("meter").unwrap();
        let area = logicaffeine_base::Quantity::of(logicaffeine_base::Rational::from_i64(3), &m)
            .mul(&logicaffeine_base::Quantity::of(logicaffeine_base::Rational::from_i64(4), &m));
        let area_unit =
            logicaffeine_base::Unit::linear("", area.dimension(), logicaffeine_base::Rational::one());
        let v = RuntimeValue::Quantity(Rc::new(crate::interpreter::QuantityValue { q: area, unit: area_unit }));
        let (_, back) = message_from_wire(&message_to_wire("", &v).unwrap()).unwrap();
        assert_eq!(back.to_display_string(), "12 L^2");
        assert_eq!(back, v);
    }

    #[test]
    fn our_wire_preserves_an_integer_that_json_would_corrupt() {
        // 2^53 + 1 is the smallest integer a conforming JSON (f64) consumer rounds
        // away — the canonical "JSON numbers ruin lives" value. Our typed wire keeps
        // the i64 EXACT, with no 2^53 cliff.
        let n = 9_007_199_254_740_993i64;
        let bytes = message_to_wire("", &RuntimeValue::Int(n)).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert_eq!(back, RuntimeValue::Int(n), "our wire keeps it exact");
        // The JSON number model (IEEE-754 double) loses it.
        assert_ne!(n as f64 as i64, n, "f64 round-trip corrupts 2^53+1 — the JSON footgun");
    }

    fn rat(n: i64, d: i64) -> RuntimeValue {
        RuntimeValue::from_rational(logicaffeine_base::Rational::from_ratio_i64(n, d).unwrap())
    }

    #[test]
    fn our_wire_preserves_a_fraction_that_json_would_round() {
        // 1/3 has no exact f64 / JSON-number representation — a JSON consumer stores
        // 0.3333… and can never recover 1/3. Our typed wire keeps the fraction EXACT —
        // the other half of "JSON numbers ruin lives", alongside the 2^53 cliff above.
        let bytes = message_to_wire("", &rat(1, 3)).unwrap();
        let (_, back) = message_from_wire(&bytes).unwrap();
        assert_eq!(back.to_display_string(), "1/3", "the fraction survives the wire exactly");
        assert!(matches!(back, RuntimeValue::Rational(_)), "stays a Rational across the wire");
        // The JSON/IEEE-754 model can't even add tenths exactly — the footgun this removes.
        assert_ne!(0.1_f64 + 0.2, 0.3);
    }

    #[test]
    fn rational_round_trips_through_the_wire_and_cross_task() {
        // Includes a fraction that REDUCES to a whole number (6/2 → 3): it downsizes to
        // an Int and rides the wire as one, proving the canonical-representation invariant.
        for (n, d, shown) in [(7i64, 2i64, "7/2"), (-3, 4, "-3/4"), (6, 2, "3"), (22, 7, "22/7")] {
            let v = rat(n, d);
            let bytes = message_to_wire("", &v).unwrap();
            let (_, back) = message_from_wire(&bytes).unwrap();
            assert_eq!(back.to_display_string(), shown, "{n}/{d} via the binary wire");
            let back2 = rebuild(materialize(&v).unwrap());
            assert_eq!(back2.to_display_string(), shown, "{n}/{d} via materialize/rebuild");
        }
    }

    #[test]
    fn scalars_and_temporals_roundtrip() {
        for v in [
            RuntimeValue::Int(5),
            RuntimeValue::Float(2.5),
            RuntimeValue::Bool(true),
            RuntimeValue::Char('z'),
            RuntimeValue::Nothing,
            RuntimeValue::Duration(10),
            RuntimeValue::Date(19_000),
            RuntimeValue::Moment(123),
            RuntimeValue::Span { months: 3, days: 14 },
            RuntimeValue::Time(99),
        ] {
            assert_roundtrips(&v);
        }
    }

    #[test]
    fn text_roundtrips() {
        let v = RuntimeValue::Text(Rc::new("hello".to_string()));
        let p = assert_roundtrips(&v);
        assert_eq!(p, RtPayload::Text("hello".to_string()));
    }

    #[test]
    fn peer_handle_roundtrips_exactly() {
        let v = RuntimeValue::Peer(Rc::new("ws://127.0.0.1:9944".to_string()));
        let p = assert_roundtrips(&v);
        assert_eq!(p, RtPayload::Peer("ws://127.0.0.1:9944".to_string()));
        // …and rebuilds back to a Peer (not a bare Text).
        assert!(matches!(rebuild(p), RuntimeValue::Peer(t) if t.as_str() == "ws://127.0.0.1:9944"));
    }

    #[test]
    fn int_list_roundtrips() {
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(3),
        ]))));
        let p = assert_roundtrips(&v);
        assert_eq!(p, RtPayload::List(vec![RtPayload::Int(1), RtPayload::Int(2), RtPayload::Int(3)]));
    }

    #[test]
    fn set_and_tuple_roundtrip() {
        let set = RuntimeValue::Set(Rc::new(RefCell::new(vec![RuntimeValue::Int(7), RuntimeValue::Int(8)])));
        let p = assert_roundtrips(&set);
        assert_eq!(p, RtPayload::Set(vec![RtPayload::Int(7), RtPayload::Int(8)]));

        let tup = RuntimeValue::Tuple(Rc::new(vec![RuntimeValue::Int(1), RuntimeValue::Bool(false), RuntimeValue::Char('x')]));
        let p = assert_roundtrips(&tup);
        assert_eq!(p, RtPayload::Tuple(vec![RtPayload::Int(1), RtPayload::Bool(false), RtPayload::Char('x')]));
    }

    #[test]
    fn sets_are_canonical_regardless_of_insertion_order() {
        // SYMMETRY BREAKING ON THE WIRE: a set is order-invariant, so the SAME members in ANY
        // insertion order MUST serialize to byte-identical wire — so content-addressing, dedup,
        // `Send cached` keying, and FEC all hold for sets. Mirrors canonical T_MAP / T_STRUCT.
        let mk = |order: &[i64]| {
            RuntimeValue::Set(Rc::new(RefCell::new(
                order.iter().map(|&n| RuntimeValue::Int(n)).collect::<Vec<_>>(),
            )))
        };
        let mut e1 = Vec::new();
        native_encode(&mk(&[5, 1, 3, 2, 4]), &mut e1).unwrap();
        let mut e2 = Vec::new();
        native_encode(&mk(&[4, 2, 3, 1, 5]), &mut e2).unwrap();
        let mut e3 = Vec::new();
        native_encode(&mk(&[1, 2, 3, 4, 5]), &mut e3).unwrap();
        assert_eq!(e1, e2, "same set, different insertion order → byte-identical wire");
        assert_eq!(e1, e3, "...identical to the already-sorted insertion order too");

        let mut pos = 0;
        match native_decode(&e1, &mut pos).expect("decode") {
            RuntimeValue::Set(s) => {
                let mut got: Vec<i64> = s
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        RuntimeValue::Int(n) => *n,
                        other => panic!("expected Int in set, got {other:?}"),
                    })
                    .collect();
                got.sort_unstable();
                assert_eq!(got, vec![1, 2, 3, 4, 5], "canonical bytes round-trip to the same members");
            }
            other => panic!("expected a Set, got {other:?}"),
        }
    }

    #[test]
    fn int_set_crushes_to_a_compressed_column() {
        // The extreme symmetry-break case: a CONSECUTIVE int set {1..1000} (built in reverse order).
        // Sorted, it's a perfect affine column → base+stride+count, ZERO per-element data. It must be
        // a tiny fraction of the ~1000+ bytes the naive tagged-element encoding would write, and round
        // -trip exactly. (A clustered/sparse set instead gets delta/RLE — never worse than the naive.)
        let n = 1000i64;
        let order: Vec<RuntimeValue> = (1..=n).rev().map(RuntimeValue::Int).collect();
        let set = RuntimeValue::Set(Rc::new(RefCell::new(order)));
        let mut enc = Vec::new();
        native_encode(&set, &mut enc).unwrap();
        assert!(
            enc.len() < 64,
            "a consecutive int set of {n} must collapse to a closed-form column, got {} bytes",
            enc.len()
        );
        let mut pos = 0;
        match native_decode(&enc, &mut pos).unwrap() {
            RuntimeValue::Set(s) => {
                let mut got: Vec<i64> = s
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        RuntimeValue::Int(k) => *k,
                        other => panic!("expected Int, got {other:?}"),
                    })
                    .collect();
                got.sort_unstable();
                assert_eq!(got, (1..=n).collect::<Vec<_>>(), "round-trips to the full set");
            }
            other => panic!("expected a Set, got {other:?}"),
        }
    }

    #[test]
    fn int_keyed_map_columnarizes_keys_and_values() {
        // The map analog of the int-set crush. A map with CONSECUTIVE integer keys {0..1000} mapping
        // to an affine value {i ↦ 2i} (inserted in REVERSE) ships as TWO columns: the keys collapse to
        // an affine closed form (base+stride+count, no per-key data) AND the values collapse the same
        // way (they reuse the full best-list column menu). So the whole int→int map — 1000 entries —
        // becomes a handful of bytes, vs the ~5–6 KB the per-entry `T_MAP` would write (each key AND
        // value a tagged inline varint). Must round-trip exactly and be insertion-order-invariant.
        let n = 1000i64;
        let build = |rev: bool| {
            let mut keys: Vec<i64> = (0..n).collect();
            if rev {
                keys.reverse();
            }
            let mut m = MapStorage::default();
            for k in keys {
                m.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * 2));
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        };

        let mut enc = Vec::new();
        native_encode(&build(false), &mut enc).unwrap();
        assert!(
            enc.len() < 64,
            "an affine int→int map of {n} entries must collapse BOTH columns to closed forms, got {} bytes",
            enc.len()
        );

        // Canonical: the SAME map built in the opposite insertion order is BYTE-IDENTICAL.
        let mut enc_rev = Vec::new();
        native_encode(&build(true), &mut enc_rev).unwrap();
        assert_eq!(enc, enc_rev, "int-keyed map encoding must be insertion-order-invariant");

        // Round-trips exactly.
        let mut pos = 0;
        match native_decode(&enc, &mut pos).unwrap() {
            RuntimeValue::Map(m) => {
                let b = m.borrow();
                assert_eq!(b.len(), n as usize, "all entries recovered");
                for k in 0..n {
                    let got = b.get(&RuntimeValue::Int(k)).expect("key present");
                    assert_eq!(*got, RuntimeValue::Int(k * 2), "value for key {k} survived");
                }
            }
            other => panic!("expected a Map, got {other:?}"),
        }
    }

    #[test]
    fn int_keyed_map_with_non_int_values_still_columnarizes_keys() {
        // Keys all Int but values heterogeneous-ish (Text) — the KEY column still collapses; the value
        // list takes its own best encoding (here a string column). Must round-trip + stay canonical.
        let build = |rev: bool| {
            let mut ks: Vec<i64> = vec![10, 20, 30, 40];
            if rev {
                ks.reverse();
            }
            let mut m = MapStorage::default();
            for k in ks {
                m.insert(RuntimeValue::Int(k), RuntimeValue::Text(Rc::new(format!("item_{k}"))));
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        };
        let mut a = Vec::new();
        native_encode(&build(false), &mut a).unwrap();
        let mut b = Vec::new();
        native_encode(&build(true), &mut b).unwrap();
        assert_eq!(a, b, "int-keyed map with text values must be insertion-order-invariant");
        let mut pos = 0;
        match native_decode(&a, &mut pos).unwrap() {
            RuntimeValue::Map(m) => {
                let mb = m.borrow();
                assert_eq!(mb.len(), 4);
                for k in [10i64, 20, 30, 40] {
                    let got = mb.get(&RuntimeValue::Int(k)).expect("key present");
                    assert_eq!(*got, RuntimeValue::Text(Rc::new(format!("item_{k}"))));
                }
            }
            other => panic!("expected a Map, got {other:?}"),
        }
    }

    #[test]
    fn int_keyed_map_front_codes_string_value_column() {
        // The database-table case: int primary key → string column with shared prefixes (URLs/paths/
        // ids). Keys collapse to an affine column; the VALUES front-code in key order (each shipped as
        // shared-prefix-len-with-previous + suffix), so a column of `https://…/items/<i>` ships only
        // the changing suffix. Must be far smaller than the per-value fallback, round-trip, be canonical.
        let n = 100i64;
        let build = |rev: bool| {
            let mut ks: Vec<i64> = (0..n).collect();
            if rev {
                ks.reverse();
            }
            let mut m = MapStorage::default();
            for k in ks {
                m.insert(
                    RuntimeValue::Int(k),
                    RuntimeValue::Text(Rc::new(format!("https://example.com/items/{k}"))),
                );
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        };

        let mut enc = Vec::new();
        native_encode(&build(false), &mut enc).unwrap();
        // Per-value (kind 0) would ship ~30 bytes × 100 ≈ 3 KB; front-coding the ~26-char shared prefix
        // leaves only the suffix deltas.
        assert!(
            enc.len() < 1200,
            "an int→string map with a shared value prefix must front-code its value column, got {} bytes",
            enc.len()
        );

        let mut enc_rev = Vec::new();
        native_encode(&build(true), &mut enc_rev).unwrap();
        assert_eq!(enc, enc_rev, "int→string map must be insertion-order-invariant");

        let mut pos = 0;
        match native_decode(&enc, &mut pos).unwrap() {
            RuntimeValue::Map(m) => {
                let b = m.borrow();
                assert_eq!(b.len(), n as usize);
                for k in 0..n {
                    let got = b.get(&RuntimeValue::Int(k)).expect("key present");
                    assert_eq!(
                        *got,
                        RuntimeValue::Text(Rc::new(format!("https://example.com/items/{k}"))),
                        "value for key {k} survived"
                    );
                }
            }
            other => panic!("expected a Map, got {other:?}"),
        }
    }

    #[test]
    fn int_keyed_map_columnarizes_struct_value_column() {
        // The full database-table crush: int primary key → homogeneous struct record (id/name/active).
        // The values pack COLUMNAR like a struct list — the field NAMES ship ONCE, then one column per
        // field (the id column compresses, the active column bit-packs) — instead of re-shipping the
        // whole self-describing struct per row (the per-value fallback). Must round-trip + be canonical.
        let n = 100i64;
        let rec = |id: i64| {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(id));
            f.insert("name".to_string(), RuntimeValue::Text(Rc::new(format!("user_{id}"))));
            f.insert("active".to_string(), RuntimeValue::Bool(id % 2 == 0));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "Rec".to_string(), fields: f }))
        };
        let build = |rev: bool| {
            let mut ks: Vec<i64> = (0..n).collect();
            if rev {
                ks.reverse();
            }
            let mut m = MapStorage::default();
            for k in ks {
                m.insert(RuntimeValue::Int(k), rec(k));
            }
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        };

        let mut enc = Vec::new();
        native_encode(&build(false), &mut enc).unwrap();
        // Per-value (kind 0) re-ships "Rec"+"id"+"name"+"active" per row ≈ 27 B × 100 ≈ 2.7 KB; the
        // columnar form ships the schema once.
        assert!(
            enc.len() < 1500,
            "an int→struct map must pack its value column columnarly (schema once), got {} bytes",
            enc.len()
        );

        let mut enc_rev = Vec::new();
        native_encode(&build(true), &mut enc_rev).unwrap();
        assert_eq!(enc, enc_rev, "int→struct map must be insertion-order-invariant");

        // Round-trip proof by BYTE-STABILITY (re-encode the decoded value and compare bytes), the
        // canonical method in this codec — `RuntimeValue`'s `PartialEq` does not deep-compare struct
        // values, so direct `assert_eq!` on a decoded struct is unavailable; byte-identity is strictly
        // stronger (any dropped/corrupted field would perturb the re-encoding).
        let mut pos = 0;
        let decoded = native_decode(&enc, &mut pos).unwrap();
        match &decoded {
            RuntimeValue::Map(m) => assert_eq!(m.borrow().len(), n as usize, "all rows recovered"),
            other => panic!("expected a Map, got {other:?}"),
        }
        let mut reenc = Vec::new();
        native_encode(&decoded, &mut reenc).unwrap();
        assert_eq!(reenc, enc, "decoded int→struct map must re-encode byte-identically");
    }

    #[test]
    fn string_set_front_codes_shared_prefixes() {
        // Sorted strings share prefixes → front-coding ships only the suffix deltas. A set of strings
        // with long common prefixes must be smaller than the naive concat, round-trip exactly, and be
        // canonical (insertion-order-invariant).
        let words = ["user_1000", "user_1001", "user_1002", "user_1003", "user_2000"];
        let mk = |order: &[usize]| {
            RuntimeValue::Set(Rc::new(RefCell::new(
                order
                    .iter()
                    .map(|&i| RuntimeValue::Text(Rc::new(words[i].to_string())))
                    .collect::<Vec<_>>(),
            )))
        };
        let mut e1 = Vec::new();
        native_encode(&mk(&[0, 1, 2, 3, 4]), &mut e1).unwrap();
        let mut e2 = Vec::new();
        native_encode(&mk(&[4, 2, 0, 3, 1]), &mut e2).unwrap();
        assert_eq!(e1, e2, "same string set, different insertion order → identical wire");

        let naive: usize = words.iter().map(|w| w.len()).sum();
        assert!(
            e1.len() < naive,
            "front-coding must beat the {naive}-byte naive concat (shared prefixes elided), got {}",
            e1.len()
        );

        let mut pos = 0;
        match native_decode(&e1, &mut pos).unwrap() {
            RuntimeValue::Set(s) => {
                let mut got: Vec<String> = s
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        RuntimeValue::Text(t) => (**t).clone(),
                        other => panic!("expected Text, got {other:?}"),
                    })
                    .collect();
                got.sort();
                let mut want: Vec<String> = words.iter().map(|w| w.to_string()).collect();
                want.sort();
                assert_eq!(got, want, "round-trips to the same members");
            }
            other => panic!("expected a Set, got {other:?}"),
        }
    }

    #[test]
    fn single_entry_map_roundtrips_exactly() {
        let mut m: MapStorage = MapStorage::default();
        m.insert(RuntimeValue::Int(1), RuntimeValue::Text(Rc::new("a".to_string())));
        let v = RuntimeValue::Map(Rc::new(RefCell::new(m)));
        let p = assert_roundtrips(&v);
        assert_eq!(p, RtPayload::Map(vec![(RtPayload::Int(1), RtPayload::Text("a".to_string()))]));
    }

    #[test]
    fn multi_entry_map_preserves_entries() {
        let mut m: MapStorage = MapStorage::default();
        m.insert(RuntimeValue::Int(1), RuntimeValue::Int(10));
        m.insert(RuntimeValue::Int(2), RuntimeValue::Int(20));
        let v = RuntimeValue::Map(Rc::new(RefCell::new(m)));
        let p = materialize(&v).unwrap();
        let entries = match &p {
            RtPayload::Map(e) => e,
            _ => panic!("expected map"),
        };
        assert_eq!(entries.len(), 2);
        assert!(entries.contains(&(RtPayload::Int(1), RtPayload::Int(10))));
        assert!(entries.contains(&(RtPayload::Int(2), RtPayload::Int(20))));

        // Round-trip preserves the entry set (order may differ — hashmap).
        let p2 = materialize(&rebuild(p.clone())).unwrap();
        let e2 = match &p2 {
            RtPayload::Map(e) => e,
            _ => panic!("expected map"),
        };
        assert_eq!(entries.len(), e2.len());
        for e in entries {
            assert!(e2.contains(e), "entry {e:?} lost in round-trip");
        }
    }

    #[test]
    fn single_field_struct_roundtrips_exactly() {
        let mut fields = HashMap::new();
        fields.insert("x".to_string(), RuntimeValue::Int(7));
        let v = RuntimeValue::Struct(Box::new(StructValue { type_name: "Point".to_string(), fields }));
        let p = assert_roundtrips(&v);
        assert_eq!(
            p,
            RtPayload::Struct {
                type_name: "Point".to_string(),
                fields: vec![("x".to_string(), RtPayload::Int(7))],
            }
        );
    }

    #[test]
    fn inductive_roundtrips() {
        let v = RuntimeValue::Inductive(Box::new(InductiveValue {
            inductive_type: "Option".to_string(),
            constructor: "Some".to_string(),
            args: vec![RuntimeValue::Int(42)],
        }));
        let p = assert_roundtrips(&v);
        assert_eq!(
            p,
            RtPayload::Inductive {
                type_name: "Option".to_string(),
                constructor: "Some".to_string(),
                args: vec![RtPayload::Int(42)],
            }
        );
    }

    #[test]
    fn nested_collections_roundtrip() {
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
            RuntimeValue::Tuple(Rc::new(vec![RuntimeValue::Int(1), RuntimeValue::Bool(true)])),
            RuntimeValue::Text(Rc::new("nested".to_string())),
        ]))));
        assert_roundtrips(&v);
    }

    // ── Best-of-both-worlds interop: the peer codec (RuntimeValue) and the shared wire core
    //    (an AOT-generated-style type) must produce and read the SAME bytes. This is the lock
    //    that lets a compile-once native PE receive a program over the real fast codec. ──────
    use logicaffeine_data::wire::{self, WireDecode, WireEncode};

    // A hand-written mirror of exactly what codegen emits for a `## A CE is one of:` enum.
    #[derive(Debug, Clone, PartialEq)]
    enum CE {
        CInt(i64),
        CText(String),
        CBool(bool),
        CBinOp { op: String, left: Box<CE>, right: Box<CE> },
        CList(Vec<CE>),
    }
    impl WireEncode for CE {
        fn wire_encode(&self, out: &mut Vec<u8>) {
            match self {
                CE::CInt(v) => { wire::write_inductive_header(out, "CE", "CInt", 1); v.wire_encode(out); }
                CE::CText(s) => { wire::write_inductive_header(out, "CE", "CText", 1); s.wire_encode(out); }
                CE::CBool(b) => { wire::write_inductive_header(out, "CE", "CBool", 1); b.wire_encode(out); }
                CE::CBinOp { op, left, right } => {
                    wire::write_inductive_header(out, "CE", "CBinOp", 3);
                    op.wire_encode(out);
                    left.wire_encode(out);
                    right.wire_encode(out);
                }
                CE::CList(xs) => { wire::write_inductive_header(out, "CE", "CList", 1); xs.wire_encode(out); }
            }
        }
    }
    impl WireDecode for CE {
        fn wire_decode(buf: &[u8], pos: &mut usize) -> Option<Self> {
            let (ty, ctor, _n) = wire::read_inductive_header(buf, pos)?;
            debug_assert_eq!(ty, "CE");
            Some(match ctor.as_str() {
                "CInt" => CE::CInt(i64::wire_decode(buf, pos)?),
                "CText" => CE::CText(String::wire_decode(buf, pos)?),
                "CBool" => CE::CBool(bool::wire_decode(buf, pos)?),
                "CBinOp" => CE::CBinOp {
                    op: String::wire_decode(buf, pos)?,
                    left: Box::<CE>::wire_decode(buf, pos)?,
                    right: Box::<CE>::wire_decode(buf, pos)?,
                },
                "CList" => CE::CList(Vec::<CE>::wire_decode(buf, pos)?),
                _ => return None,
            })
        }
    }

    // The same logical program in both value models.
    fn ce_tree() -> CE {
        CE::CBinOp {
            op: "+".to_string(),
            left: Box::new(CE::CInt(2)),
            right: Box::new(CE::CList(vec![CE::CInt(3), CE::CBool(true), CE::CText("hi".to_string())])),
        }
    }
    fn rt_ind(ty: &str, ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
        RuntimeValue::Inductive(Box::new(InductiveValue {
            inductive_type: ty.to_string(),
            constructor: ctor.to_string(),
            args,
        }))
    }
    fn rt_tree() -> RuntimeValue {
        rt_ind("CE", "CBinOp", vec![
            RuntimeValue::Text(Rc::new("+".to_string())),
            rt_ind("CE", "CInt", vec![RuntimeValue::Int(2)]),
            rt_ind("CE", "CList", vec![RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
                rt_ind("CE", "CInt", vec![RuntimeValue::Int(3)]),
                rt_ind("CE", "CBool", vec![RuntimeValue::Bool(true)]),
                rt_ind("CE", "CText", vec![RuntimeValue::Text(Rc::new("hi".to_string()))]),
            ]))))]),
        ])
    }

    #[test]
    fn peer_and_wire_core_produce_identical_bytes() {
        let peer_bytes = encode_value_raw(&rt_tree()).expect("peer encode");
        let mut wire_bytes = Vec::new();
        ce_tree().wire_encode(&mut wire_bytes);
        assert_eq!(
            peer_bytes, wire_bytes,
            "the shared wire core must be byte-identical to the peer codec"
        );
    }

    #[test]
    fn peer_encode_then_wire_decode_reconstructs_the_generated_type() {
        // The exact path a compile-once native PE uses: host peer-encodes, native wire-decodes.
        let bytes = encode_value_raw(&rt_tree()).expect("peer encode");
        let mut pos = 0usize;
        let decoded = CE::wire_decode(&bytes, &mut pos).expect("wire decode");
        assert_eq!(pos, bytes.len(), "must consume every byte");
        assert_eq!(decoded, ce_tree());
    }

    #[test]
    fn wire_encode_then_peer_decode_reconstructs_the_runtime_value() {
        // The reverse direction: native wire-encodes, host peer-decodes.
        let mut bytes = Vec::new();
        ce_tree().wire_encode(&mut bytes);
        let rv = decode_value_raw(&bytes).expect("peer decode");
        assert_eq!(materialize(&rv).unwrap(), materialize(&rt_tree()).unwrap());
    }

    #[test]
    fn function_is_not_sendable() {
        let v = RuntimeValue::Function(Box::new(ClosureValue {
            body_index: 0,
            captured_env: HashMap::default(),
            param_names: vec![],
            generated: None,
        }));
        assert_eq!(materialize(&v), Err(MarshalError::NotSendable("Function")));
    }

    // -------------------------------------------------------------------------
    // Wire codec — a message is any language value
    // -------------------------------------------------------------------------

    /// A value survives the wire iff its `RtPayload` is unchanged across
    /// encode→decode (we compare through `RtPayload`, which has structural eq).
    fn assert_wire_roundtrips(v: &RuntimeValue, from: &str) {
        let bytes = message_to_wire(from, v).expect("message_to_wire");
        let (got_from, back) = message_from_wire(&bytes).expect("message_from_wire");
        assert_eq!(got_from, from, "sender lost on the wire");
        assert_eq!(
            materialize(v).expect("before"),
            materialize(&back).expect("after"),
            "wire round-trip changed the value"
        );
    }

    #[test]
    fn message_wire_scalars_roundtrip() {
        for v in [
            RuntimeValue::Int(42),
            RuntimeValue::Float(2.5),
            RuntimeValue::Bool(true),
            RuntimeValue::Char('z'),
            RuntimeValue::Text(Rc::new("ping".to_string())),
            RuntimeValue::Nothing,
            RuntimeValue::Duration(1000),
        ] {
            assert_wire_roundtrips(&v, "alice");
        }
    }

    #[test]
    fn message_wire_is_compact_binary() {
        // A 100-element int list encodes to compact binary (a few bytes/element),
        // far tighter than a text encoding, and round-trips exactly.
        let items: Vec<RuntimeValue> = (0..100).map(RuntimeValue::Int).collect();
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
        let bytes = message_to_wire("", &v).unwrap();
        assert!(bytes.len() < 100 * 12, "wire should be compact, was {} bytes", bytes.len());
        assert_wire_roundtrips(&v, "");
    }

    #[test]
    fn message_wire_anonymous_sender_is_empty_from() {
        let bytes = message_to_wire("", &RuntimeValue::Int(1)).unwrap();
        let (from, _) = message_from_wire(&bytes).unwrap();
        assert_eq!(from, "");
    }

    #[test]
    fn message_wire_list_and_tuple_and_set_roundtrip() {
        let list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Text(Rc::new("two".to_string())),
            RuntimeValue::Bool(true),
        ]))));
        assert_wire_roundtrips(&list, "");

        let tup = RuntimeValue::Tuple(Rc::new(vec![RuntimeValue::Int(1), RuntimeValue::Char('q')]));
        assert_wire_roundtrips(&tup, "");

        let set = RuntimeValue::Set(Rc::new(RefCell::new(vec![RuntimeValue::Int(7), RuntimeValue::Int(8)])));
        assert_wire_roundtrips(&set, "");
    }

    #[test]
    fn message_wire_single_entry_map_roundtrips() {
        let mut m: MapStorage = MapStorage::default();
        m.insert(RuntimeValue::Text(Rc::new("k".to_string())), RuntimeValue::Int(9));
        let v = RuntimeValue::Map(Rc::new(RefCell::new(m)));
        assert_wire_roundtrips(&v, "");
    }

    #[test]
    fn message_wire_struct_roundtrips_by_field() {
        let mut fields = HashMap::new();
        fields.insert("x".to_string(), RuntimeValue::Int(1));
        fields.insert("y".to_string(), RuntimeValue::Int(2));
        let v = RuntimeValue::Struct(Box::new(StructValue { type_name: "Point".to_string(), fields }));
        let bytes = message_to_wire("alice", &v).unwrap();
        let (_from, back) = message_from_wire(&bytes).unwrap();
        match back {
            RuntimeValue::Struct(s) => {
                assert_eq!(s.type_name, "Point");
                assert_eq!(s.fields.get("x"), Some(&RuntimeValue::Int(1)));
                assert_eq!(s.fields.get("y"), Some(&RuntimeValue::Int(2)));
            }
            other => panic!("expected a struct, got {other:?}"),
        }
    }

    #[test]
    fn message_wire_inductive_roundtrips() {
        let v = RuntimeValue::Inductive(Box::new(InductiveValue {
            inductive_type: "Option".to_string(),
            constructor: "Some".to_string(),
            args: vec![RuntimeValue::Int(42)],
        }));
        assert_wire_roundtrips(&v, "");
    }

    #[test]
    fn message_wire_nested_list_of_structs_roundtrips() {
        let mut fields = HashMap::new();
        fields.insert("n".to_string(), RuntimeValue::Int(1));
        let s = RuntimeValue::Struct(Box::new(StructValue { type_name: "Item".to_string(), fields }));
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
            s,
            RuntimeValue::Tuple(Rc::new(vec![RuntimeValue::Int(3), RuntimeValue::Bool(false)])),
        ]))));
        assert_wire_roundtrips(&v, "carol");
    }

    #[test]
    fn message_wire_channel_handle_is_not_network_portable() {
        // A channel id is local to this process's scheduler — it cannot travel.
        let v = RuntimeValue::Chan(logicaffeine_runtime::ChanId(3));
        let err = message_to_wire("", &v).expect_err("a channel must not be sendable over the network");
        assert!(err.contains("channel") || err.contains("task"), "got: {err}");
    }

    #[test]
    fn message_wire_function_is_rejected_with_a_clear_error() {
        let v = RuntimeValue::Function(Box::new(ClosureValue {
            body_index: 0,
            captured_env: HashMap::default(),
            param_names: vec![],
            generated: None,
        }));
        let err = message_to_wire("", &v).expect_err("a closure must not be sendable");
        assert!(err.contains("Function"), "got: {err}");
    }

    #[test]
    fn message_wire_malformed_bytes_decode_to_none() {
        assert!(message_from_wire(b"").is_none()); // empty
        assert!(message_from_wire(b"\xff\xff\xff\xff garbage").is_none()); // not a valid frame
        // A truncated-but-plausible prefix of a real message must not panic.
        let good = message_to_wire("alice", &RuntimeValue::Text(Rc::new("hi".to_string()))).unwrap();
        assert!(message_from_wire(&good[..good.len() / 2]).is_none());
    }

    // =========================================================================
    // Codec hardening — fidelity, canonicality, integrity, speed, to the point
    // of absurdity.
    // =========================================================================

    /// A round-trip is byte-stable: encode → decode → re-encode yields identical
    /// bytes. This proves the value reconstructed EXACTLY (bit-for-bit — floats,
    /// NaN, `-0.0` and all, since we compare bytes not `PartialEq`), that encoding
    /// is deterministic, and that structs/maps are canonical (hash order can't
    /// leak in). It's the workhorse assertion for the property fuzzer.
    fn assert_wire_stable(v: &RuntimeValue) {
        let once = message_to_wire("peer", v).expect("encode");
        let (from, back) = message_from_wire(&once).expect("decode");
        assert_eq!(from, "peer", "sender lost");
        let twice = message_to_wire("peer", &back).expect("re-encode");
        assert_eq!(once, twice, "round-trip was not byte-stable for {v:?}");
    }

    #[test]
    fn wire_boundary_ints_roundtrip() {
        for n in [0i64, 1, -1, i64::MIN, i64::MAX, i64::MIN + 1, i64::MAX - 1, i32::MIN as i64, i32::MAX as i64] {
            assert_wire_stable(&RuntimeValue::Int(n));
        }
    }

    #[test]
    fn wire_special_floats_roundtrip_bit_exact() {
        for f in [
            0.0f64, -0.0, 1.0, -1.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN,
            f64::MIN, f64::MAX, f64::MIN_POSITIVE, 1e-308, -1e308, std::f64::consts::PI,
        ] {
            let bytes = message_to_wire("", &RuntimeValue::Float(f)).unwrap();
            let (_, back) = message_from_wire(&bytes).unwrap();
            let RuntimeValue::Float(g) = back else { panic!("not a float") };
            assert_eq!(f.to_bits(), g.to_bits(), "float {f} lost bits");
        }
    }

    #[test]
    fn wire_unicode_text_and_char_roundtrip() {
        for s in ["", "ascii", "héllo", "日本語", "emoji 😀🎉", "null\0byte", "tabs\tand\nnewlines"] {
            assert_wire_stable(&RuntimeValue::Text(Rc::new(s.to_string())));
        }
        for c in ['a', '\0', '😀', '\u{10FFFF}', 'λ', '\n', '\u{1}'] {
            assert_wire_stable(&RuntimeValue::Char(c));
        }
    }

    #[test]
    fn wire_empty_collections_roundtrip() {
        assert_wire_stable(&RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![])))));
        assert_wire_stable(&RuntimeValue::Tuple(Rc::new(vec![])));
        assert_wire_stable(&RuntimeValue::Set(Rc::new(RefCell::new(vec![]))));
        assert_wire_stable(&RuntimeValue::Map(Rc::new(RefCell::new(MapStorage::default()))));
        assert_wire_stable(&RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Empty".to_string(),
            fields: HashMap::new(),
        })));
        assert_wire_stable(&RuntimeValue::Inductive(Box::new(InductiveValue {
            inductive_type: "Unit".to_string(),
            constructor: "Unit".to_string(),
            args: vec![],
        })));
    }

    #[test]
    fn wire_temporal_and_misc_scalars_roundtrip() {
        for v in [
            RuntimeValue::Nothing,
            RuntimeValue::Bool(true),
            RuntimeValue::Bool(false),
            RuntimeValue::Duration(i64::MAX),
            RuntimeValue::Date(i32::MIN),
            RuntimeValue::Moment(-1),
            RuntimeValue::Span { months: i32::MAX, days: i32::MIN },
            RuntimeValue::Time(0),
            RuntimeValue::Peer(Rc::new("ws://host:9944".to_string())),
        ] {
            assert_wire_stable(&v);
        }
    }

    #[test]
    fn wire_struct_field_order_is_canonical() {
        // The same fields in a different insertion order encode to the SAME bytes.
        let mut a = HashMap::new();
        a.insert("z".to_string(), RuntimeValue::Int(1));
        a.insert("a".to_string(), RuntimeValue::Int(2));
        a.insert("m".to_string(), RuntimeValue::Int(3));
        let mut b = HashMap::new();
        b.insert("m".to_string(), RuntimeValue::Int(3));
        b.insert("z".to_string(), RuntimeValue::Int(1));
        b.insert("a".to_string(), RuntimeValue::Int(2));
        let va = RuntimeValue::Struct(Box::new(StructValue { type_name: "S".into(), fields: a }));
        let vb = RuntimeValue::Struct(Box::new(StructValue { type_name: "S".into(), fields: b }));
        assert_eq!(
            message_to_wire("x", &va).unwrap(),
            message_to_wire("x", &vb).unwrap(),
            "struct encoding must be canonical (field order independent)"
        );
    }

    #[test]
    fn wire_map_entry_order_is_canonical() {
        let mut a = MapStorage::default();
        a.insert(RuntimeValue::Int(3), RuntimeValue::Int(30));
        a.insert(RuntimeValue::Int(1), RuntimeValue::Int(10));
        a.insert(RuntimeValue::Int(2), RuntimeValue::Int(20));
        let mut b = MapStorage::default();
        b.insert(RuntimeValue::Int(1), RuntimeValue::Int(10));
        b.insert(RuntimeValue::Int(2), RuntimeValue::Int(20));
        b.insert(RuntimeValue::Int(3), RuntimeValue::Int(30));
        let va = RuntimeValue::Map(Rc::new(RefCell::new(a)));
        let vb = RuntimeValue::Map(Rc::new(RefCell::new(b)));
        assert_eq!(
            message_to_wire("x", &va).unwrap(),
            message_to_wire("x", &vb).unwrap(),
            "map encoding must be canonical (entry order independent)"
        );
    }

    #[test]
    fn wire_rejects_nonportable_buried_in_a_container() {
        // A channel buried in a list — caught, with a clear error, never dropped.
        let in_list = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Chan(logicaffeine_runtime::ChanId(5)),
        ]))));
        let err = message_to_wire("", &in_list).expect_err("buried channel must be rejected");
        assert!(err.contains("channel") || err.contains("task"), "got: {err}");

        // A task handle buried in a map value.
        let mut m = MapStorage::default();
        m.insert(RuntimeValue::Int(1), RuntimeValue::TaskHandle(logicaffeine_runtime::TaskId(7)));
        let in_map = RuntimeValue::Map(Rc::new(RefCell::new(m)));
        assert!(message_to_wire("", &in_map).is_err(), "buried task handle must be rejected");

        // A closure buried in a struct field.
        let mut fields = HashMap::new();
        fields.insert(
            "f".to_string(),
            RuntimeValue::Function(Box::new(ClosureValue {
                body_index: 0,
                captured_env: HashMap::default(),
                param_names: vec![],
                generated: None,
            })),
        );
        let in_struct = RuntimeValue::Struct(Box::new(StructValue { type_name: "Holder".into(), fields }));
        let err = message_to_wire("", &in_struct).expect_err("buried closure must be rejected");
        assert!(err.contains("Function"), "got: {err}");
    }

    #[test]
    fn wire_checked_detects_corruption() {
        let v = RuntimeValue::Text(Rc::new("important".to_string()));
        let good = message_to_wire_with("a", &v, WireCodec::Native, WireIntegrity::Checked).unwrap();
        assert!(message_from_wire(&good).is_some(), "intact checked message decodes");
        // Flip the final payload byte — the checksum must catch it.
        let mut bad = good.clone();
        *bad.last_mut().unwrap() ^= 0xFF;
        assert!(message_from_wire(&bad).is_none(), "corruption must be rejected");
        // Flip a checksum byte too.
        let mut bad2 = good;
        bad2[1] ^= 0xFF;
        assert!(message_from_wire(&bad2).is_none(), "a mangled checksum must be rejected");
    }

    #[test]
    fn wire_raw_skips_the_checksum_for_speed() {
        let v = RuntimeValue::Int(7);
        let raw = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
        let checked = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Checked).unwrap();
        assert_eq!(checked.len(), raw.len() + 8, "checked adds exactly the 8-byte checksum");
        assert_eq!(message_from_wire(&raw).unwrap().1, RuntimeValue::Int(7));
        // A flipped byte in a RAW message is NOT caught (no integrity) — it just
        // decodes to whatever it decodes to, or fails to decode; either way no panic.
        let mut tampered = raw;
        *tampered.last_mut().unwrap() ^= 0x01;
        let _ = message_from_wire(&tampered);
    }

    #[test]
    fn wire_all_codec_and_integrity_modes_interoperate() {
        let v = RuntimeValue::Text(Rc::new("hello".to_string()));
        for codec in [WireCodec::Native, WireCodec::Json] {
            for integrity in [WireIntegrity::Raw, WireIntegrity::Checked] {
                let bytes = message_to_wire_with("s", &v, codec, integrity).unwrap();
                let (from, back) = message_from_wire(&bytes).expect("decodes any mode");
                assert_eq!(from, "s");
                assert_eq!(back, v, "{codec:?}/{integrity:?} did not round-trip");
            }
        }
    }

    #[test]
    fn wire_compression_shrinks_redundant_payloads_and_roundtrips() {
        // A big redundant payload (the same string 500×) compresses hard.
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
            (0..500).map(|_| RuntimeValue::Text(Rc::new("the same repeated string".to_string()))).collect(),
        ))));
        let plain = message_to_wire("", &v).unwrap();
        let zipped = with_compression(|| message_to_wire("", &v).unwrap());
        assert!(zipped.len() * 2 < plain.len(), "redundant data should compress: {} vs {}", zipped.len(), plain.len());
        // …and inflates transparently on the way back in.
        let count = |b: &[u8]| match message_from_wire(b).unwrap().1 {
            RuntimeValue::List(l) => l.borrow().len(),
            other => panic!("expected a list, got {other:?}"),
        };
        assert_eq!(count(&zipped), 500);
        assert_eq!(count(&plain), 500);
    }

    #[test]
    fn wire_compression_never_grows_a_message() {
        // A tiny / already-compact value: compression wouldn't help, so it's shipped
        // RAW ("keep only if it shrank") — never bigger, never a panic.
        let v = RuntimeValue::Int(42);
        let plain = message_to_wire("", &v).unwrap();
        let maybe = with_compression(|| message_to_wire("", &v).unwrap());
        assert!(maybe.len() <= plain.len(), "compression must never grow a message");
        assert_eq!(message_from_wire(&maybe).unwrap().1, RuntimeValue::Int(42));
    }

    #[test]
    fn wire_compressed_message_integrity_is_checked_before_inflate() {
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
            (0..500).map(|_| RuntimeValue::Text(Rc::new("redundant".to_string()))).collect(),
        ))));
        let mut zipped =
            with_compression(|| message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Checked)).unwrap();
        assert!(message_from_wire(&zipped).is_some(), "intact compressed message decodes");
        // Corrupt a compressed byte: the checksum (over the compressed bytes) rejects
        // it BEFORE we ever try to inflate — a clean None, no decompressor panic.
        *zipped.last_mut().unwrap() ^= 0xFF;
        assert!(message_from_wire(&zipped).is_none(), "corruption of a compressed message must be rejected");
    }

    fn redundant_list(n: usize) -> RuntimeValue {
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
            (0..n).map(|_| RuntimeValue::Text(Rc::new("the same repeated string value".to_string()))).collect(),
        ))))
    }
    fn count_list(v: &RuntimeValue) -> usize {
        match v {
            RuntimeValue::List(l) => l.borrow().len(),
            other => panic!("expected a list, got {other:?}"),
        }
    }

    #[test]
    fn wire_lz4_roundtrips_and_shrinks_redundant() {
        let v = redundant_list(500);
        let plain = message_to_wire("", &v).unwrap();
        let lz = with_compression_codec(WireCompression::Lz4, || message_to_wire("", &v).unwrap());
        assert!(lz.len() < plain.len(), "lz4 should shrink redundant data: {} vs {}", lz.len(), plain.len());
        // Self-describing: the header records the codec, so decode auto-detects it.
        assert_eq!(count_list(&message_from_wire(&lz).unwrap().1), 500);
    }

    #[test]
    fn wire_compression_codec_is_self_describing() {
        // Deflate + lz4 ship everywhere (pure-Rust). Each is auto-detected on decode
        // with no out-of-band hint — the header byte carries the codec id.
        let v = redundant_list(300);
        for c in [WireCompression::Deflate, WireCompression::Lz4] {
            let bytes = with_compression_codec(c, || message_to_wire("", &v).unwrap());
            assert_eq!(count_list(&message_from_wire(&bytes).unwrap().1), 300, "codec {c:?} self-describes");
            let (_, comp, _) = unframe(&bytes).unwrap();
            assert_eq!(comp, c, "header round-trips the codec id for {c:?}");
        }
    }

    #[test]
    fn wire_old_deflate_bytes_still_decode() {
        // Back-compat: a message framed the OLD way (H_COMPRESSED set, codec bits 0 =
        // deflate) must still decode after the 2-bit codec field was added.
        let v = redundant_list(200);
        let body = {
            let mut out = Vec::new();
            write_str("", &mut out);
            native_encode(&v, &mut out).unwrap();
            miniz_oxide::deflate::compress_to_vec(&out, 6)
        };
        let mut legacy = vec![H_COMPRESSED]; // legacy header: no codec bits
        legacy.extend_from_slice(&body);
        assert_eq!(count_list(&message_from_wire(&legacy).unwrap().1), 200);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wire_zstd_roundtrips_native_and_is_self_describing() {
        let v = redundant_list(500);
        let plain = message_to_wire("", &v).unwrap();
        let z = with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap());
        assert!(z.len() < plain.len(), "zstd should shrink redundant data: {} vs {}", z.len(), plain.len());
        assert_eq!(count_list(&message_from_wire(&z).unwrap().1), 500);
        let (_, comp, _) = unframe(&z).unwrap();
        assert_eq!(comp, WireCompression::Zstd, "header records zstd");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wire_zstd_decodes_via_ruzstd_parity() {
        // The native C zstd encoder writes a standard frame; the pure-Rust ruzstd
        // decoder (the browser's decode path) must inflate it byte-identically — so a
        // native-sent zstd message is decodable by a wasm peer.
        let v = redundant_list(500);
        let z = with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap());
        let (_codec, comp, body) = unframe(&z).expect("unframe");
        assert_eq!(comp, WireCompression::Zstd);
        let via_c = zstd::decode_all(body).expect("C zstd decode");
        let via_ruzstd = zstd_decode_ruzstd(body).expect("ruzstd decode");
        assert_eq!(via_c, via_ruzstd, "ruzstd must match C zstd byte-for-byte");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wire_compression_level_dial_trades_size_and_roundtrips() {
        // A moderately-compressible payload so the effort level actually moves the
        // size. Max ≤ Balanced ≤ Fast in bytes; every level decodes exactly.
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
            (0..2000).map(|i| RuntimeValue::Text(Rc::new(format!("event-{i}-status-{}", i % 37)))).collect(),
        ))));
        let at = |lvl| {
            with_compression_level(lvl, || with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap()))
        };
        let fast = at(WireCompressionLevel::Fast);
        let bal = at(WireCompressionLevel::Balanced);
        let max = at(WireCompressionLevel::Max);
        assert!(max.len() <= bal.len() && bal.len() <= fast.len(), "max {} ≤ bal {} ≤ fast {}", max.len(), bal.len(), fast.len());
        for b in [&fast, &bal, &max] {
            assert!(message_from_wire(b).is_some(), "every level decodes");
        }
        // The level is a sender-only preference — it never leaks past the scope.
        let default = with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap());
        assert_eq!(default.len(), bal.len(), "default level is Balanced");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wire_zstd_ratio_beats_deflate_on_redundant() {
        let v = redundant_list(1000);
        let d = with_compression_codec(WireCompression::Deflate, || message_to_wire("", &v).unwrap());
        let z = with_compression_codec(WireCompression::Zstd, || message_to_wire("", &v).unwrap());
        assert!(z.len() <= d.len(), "zstd should match or beat deflate: zstd {} vs deflate {}", z.len(), d.len());
    }

    fn point(x: i64, y: i64) -> RuntimeValue {
        let mut f = HashMap::new();
        f.insert("x".to_string(), RuntimeValue::Int(x));
        f.insert("y".to_string(), RuntimeValue::Int(y));
        RuntimeValue::Struct(Box::new(StructValue { type_name: "Point".to_string(), fields: f }))
    }
    fn struct_list(n: i64) -> RuntimeValue {
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed((0..n).map(|i| point(i, i * 2)).collect()))))
    }
    fn enum_val(ty: &str, ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
        RuntimeValue::Inductive(Box::new(InductiveValue {
            inductive_type: ty.to_string(),
            constructor: ctor.to_string(),
            args,
        }))
    }

    /// Encode `v` (asserting its top tag), decode it, then re-encode the decoded
    /// value and assert byte-equality. Encoding is canonical (fields sorted), so this
    /// proves "decodes to the exact rows" deterministically — unlike comparing
    /// `materialize` output, whose struct field order is per-HashMap random.
    fn assert_columnar_roundtrip(v: &RuntimeValue, expect_tag: u8) {
        let mut buf = Vec::new();
        native_encode(v, &mut buf).unwrap();
        assert_eq!(buf[0], expect_tag, "top wire tag");
        let mut pos = 0;
        let back = native_decode(&buf, &mut pos).unwrap();
        assert_eq!(pos, buf.len(), "decode consumes the whole buffer");
        let mut buf2 = Vec::new();
        native_encode(&back, &mut buf2).unwrap();
        assert_eq!(buf2, buf, "re-encode of the decoded value is byte-identical (exact rows, canonical)");
    }

    #[test]
    fn wire_homogeneous_struct_list_packs_columnar() {
        assert_columnar_roundtrip(&struct_list(1000), T_STRUCTS);
    }

    #[test]
    fn wire_columnar_struct_list_is_far_smaller_than_boxed() {
        // The old boxed path re-emitted "Point"/"x"/"y" strings on EVERY row
        // (~16 B/row ⇒ ~16 KB for 1000). Columnar writes the schema once + two packed
        // int columns ⇒ a few KB. A threshold cleanly between the two regimes.
        let mut buf = Vec::new();
        native_encode(&struct_list(1000), &mut buf).unwrap();
        assert!(buf.len() < 6000, "columnar 1000×Point should be well under boxed's ~16 KB, was {}", buf.len());
    }

    #[test]
    fn wire_columnar_int_field_is_memcpy_under_fixed() {
        // Columnar fields honor the numeric dial: under Fixed, each int column is raw
        // 8-byte i64 (memcpy) — wider than varint. Boxed structs ignored the dial, so
        // this size difference is exactly what proves the fields pack as columns.
        let v = struct_list(200);
        let varint = {
            let mut b = Vec::new();
            native_encode(&v, &mut b).unwrap();
            b
        };
        let fixed = {
            let mut b = Vec::new();
            with_fixed_numerics(|| native_encode(&v, &mut b).unwrap());
            b
        };
        assert_eq!(varint[0], T_STRUCTS);
        assert_eq!(fixed[0], T_STRUCTS);
        assert!(fixed.len() > varint.len(), "fixed int columns are memcpy-wide: fixed {} vs varint {}", fixed.len(), varint.len());
        // The fixed encoding decodes and re-encodes (under fixed) byte-identically.
        let mut pos = 0;
        let back = native_decode(&fixed, &mut pos).unwrap();
        let mut re = Vec::new();
        with_fixed_numerics(|| native_encode(&back, &mut re).unwrap());
        assert_eq!(re, fixed, "fixed columnar decodes to the exact rows");
    }

    #[test]
    fn wire_ragged_struct_list_falls_back_to_boxed() {
        // Different field sets ⇒ not homogeneous ⇒ the generic per-element path.
        let a = point(1, 2);
        let mut bf = HashMap::new();
        bf.insert("x".to_string(), RuntimeValue::Int(3)); // no "y"
        let b = RuntimeValue::Struct(Box::new(StructValue { type_name: "Point".to_string(), fields: bf }));
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![a, b]))));
        assert_columnar_roundtrip(&v, T_LIST); // ragged struct list stays boxed
    }

    #[test]
    fn wire_columnar_struct_roundtrip_is_byte_stable() {
        assert_wire_stable(&struct_list(50));
    }

    #[test]
    fn wire_in_memory_columnar_structs_encode_identically_to_boxed() {
        // A `from_values`-built Structs repr (in-memory columns) and a hand-built
        // Boxed struct list with the same rows must encode to byte-identical wire —
        // the in-memory columns stream out exactly as the boxed path would gather them.
        let rows: Vec<RuntimeValue> = (0..100).map(|i| point(i, i * 2)).collect();
        let repr = ListRepr::from_values(rows.clone());
        assert!(matches!(repr, ListRepr::Structs { .. }), "from_values de-boxes a struct list to columns");
        let columnar = RuntimeValue::List(Rc::new(RefCell::new(repr)));
        let boxed = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(rows))));
        assert_eq!(
            message_to_wire("p", &columnar).unwrap(),
            message_to_wire("p", &boxed).unwrap(),
            "in-memory columns encode byte-identically to the boxed columnar path"
        );
    }

    #[test]
    fn wire_struct_list_decodes_to_columnar_repr() {
        // A received struct list decodes DIRECTLY into the columnar in-memory repr —
        // no per-row `StructValue` rebuild (the receive-throughput win).
        let bytes = message_to_wire("p", &struct_list(100)).unwrap();
        match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => {
                assert!(matches!(&*l.borrow(), ListRepr::Structs { .. }), "decodes to the columnar Structs repr");
                assert_eq!(l.borrow().len(), 100);
            }
            other => panic!("expected a list, got {other:?}"),
        }
    }

    #[test]
    fn wire_nullary_enum_list_packs_dictionary() {
        let names = ["Red", "Green", "Blue"];
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
            (0..900).map(|i| enum_val("Color", names[i % 3], vec![])).collect(),
        ))));
        let mut buf = Vec::new();
        native_encode(&v, &mut buf).unwrap();
        assert!(buf.len() < 1200, "dictionaried enum list should be tiny, was {}", buf.len());
        assert_columnar_roundtrip(&v, T_INDUCTIVES);
    }

    #[test]
    fn wire_arg_enum_list_packs_columnar() {
        // Arg-bearing enums now pack as a tagged union (T_INDUCTIVES) — a constructor
        // dictionary + index column + dense per-constructor arg columns — not boxed.
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
            (0..10).map(|i| enum_val("Option", "Some", vec![RuntimeValue::Int(i)])).collect(),
        ))));
        assert_columnar_roundtrip(&v, T_INDUCTIVES);
    }

    #[test]
    fn wire_mixed_arity_enum_list_packs_columnar() {
        // Some(1), None, Some(2), None — mixed constructors/arities round-trip exact.
        let rows: Vec<RuntimeValue> = (0..20)
            .map(|i| {
                if i % 2 == 0 {
                    enum_val("Option", "Some", vec![RuntimeValue::Int(i)])
                } else {
                    enum_val("Option", "None", vec![])
                }
            })
            .collect();
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(rows))));
        assert_columnar_roundtrip(&v, T_INDUCTIVES);
    }

    #[test]
    fn wire_enum_list_decodes_to_columnar_repr() {
        // A received enum list decodes DIRECTLY into the columnar Inductives repr.
        let rows: Vec<RuntimeValue> =
            (0..50).map(|i| enum_val("Option", "Some", vec![RuntimeValue::Int(i)])).collect();
        let bytes = message_to_wire("p", &RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(rows))))).unwrap();
        match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => {
                assert!(matches!(&*l.borrow(), ListRepr::Inductives { .. }), "decodes to the columnar Inductives repr");
                assert_eq!(l.borrow().len(), 50);
            }
            other => panic!("expected a list, got {other:?}"),
        }
    }

    #[test]
    fn wire_schema_dictionary_sends_schema_once_and_shrinks() {
        // With a connection-scoped cache, a struct schema (type + field names) is sent
        // ONCE; later messages of the same shape reference it by id and are smaller.
        let v = struct_list(50);
        let mut send_cache = WireSchemaCache::default();
        let mut recv_cache = WireSchemaCache::default();
        let m1 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut send_cache).unwrap();
        let m2 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut send_cache).unwrap();
        assert!(m2.len() < m1.len(), "the 2nd message references the schema and is smaller: {} vs {}", m2.len(), m1.len());
        // The receiver decodes both in order with its own cache; each reconstructs the
        // exact rows (proven by canonical stateless re-encode equality).
        let d1 = message_from_wire_cached(&m1, &mut recv_cache).unwrap().1;
        let d2 = message_from_wire_cached(&m2, &mut recv_cache).unwrap().1;
        let canon = |x: &RuntimeValue| message_to_wire("p", x).unwrap();
        assert_eq!(canon(&d1), canon(&v));
        assert_eq!(canon(&d2), canon(&v));
    }

    #[test]
    fn wire_single_struct_schema_ref_drops_field_names_and_shrinks() {
        // G4: a LONE struct (not a list) under a connection cache sends its schema
        // once; the next same-shaped struct ships values only — no inline "x"/"y" —
        // so it is strictly smaller, and both still decode to the exact struct.
        let v = point(1, 2);
        let mut sc = WireSchemaCache::default();
        let mut rc = WireSchemaCache::default();
        let m1 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
        let m2 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
        assert!(m2.len() < m1.len(), "2nd lone struct references the schema and is smaller: {} vs {}", m2.len(), m1.len());
        let canon = |x: &RuntimeValue| message_to_wire("p", x).unwrap();
        let d1 = message_from_wire_cached(&m1, &mut rc).unwrap().1;
        let d2 = message_from_wire_cached(&m2, &mut rc).unwrap().1;
        assert_eq!(canon(&d1), canon(&v));
        assert_eq!(canon(&d2), canon(&v));
    }

    #[test]
    fn wire_single_struct_sequential_ref_beats_inline_field_names() {
        // The sequential (1-byte id) ref is strictly smaller than the uncached inline
        // struct, because it replaces the type-name + every field-name string with one
        // id — the clean, uncompressed size win that closes the postcard gap.
        let v = point(7, 9);
        let inline = message_to_wire("p", &v).unwrap();
        let mut sc = WireSchemaCache::sequential();
        let _def = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
        let refmsg = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
        assert!(refmsg.len() < inline.len(), "schema-ref ({}) must beat inline field-names ({})", refmsg.len(), inline.len());
    }

    #[test]
    fn wire_single_struct_def_decodes_without_a_cache() {
        // The FIRST cached lone-struct message carries its schema inline (a "def"), so
        // a stateless decoder still reconstructs it.
        let v = point(3, 4);
        let mut sc = WireSchemaCache::default();
        let m1 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
        let d = message_from_wire(&m1).unwrap().1;
        assert_eq!(message_to_wire("p", &d).unwrap(), message_to_wire("p", &v).unwrap());
    }

    #[test]
    fn wire_single_struct_ref_without_cache_fails_cleanly() {
        // A schema-reference lone struct decoded WITHOUT the cache that defined it
        // returns None (clean) — never a mis-decode, never a panic.
        let v = point(5, 6);
        let mut sc = WireSchemaCache::default();
        let _m1 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
        let m2 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
        assert!(message_from_wire(&m2).is_none(), "a bare schema-ref must not decode without its cache");
    }

    #[test]
    fn wire_single_struct_uncached_stays_inline_tag() {
        // No cache → the lone struct still uses the self-describing T_STRUCT tag (field
        // names inline), keeping every non-cached encode byte-identical to before G4.
        let v = point(1, 2);
        let bytes = message_to_wire("p", &v).unwrap();
        let (codec, _comp, body) = unframe(&bytes).unwrap();
        assert!(matches!(codec, WireCodec::Native));
        let mut pos = 0;
        skip_str(body, &mut pos).unwrap();
        assert_eq!(body[pos], T_STRUCT, "uncached lone struct must stay the inline T_STRUCT form");
    }

    // ---- Pillar B: type-id substrate (default-on name elision, beats raw varint) ----

    #[test]
    fn wire_struct_type_id_elides_names_and_beats_inline() {
        // With a shared type registry (both ends derive it from their program's type
        // defs), a struct ships its small registry id instead of its type/field NAMES —
        // strictly smaller than the self-describing inline form, on the FIRST message.
        let v = point(1, 2);
        let schemas = vec![("Point".to_string(), vec!["x".to_string(), "y".to_string()])];
        let with_reg = with_type_registry(WireTypeRegistry::new(schemas.clone()), || {
            message_to_wire("p", &v).unwrap()
        });
        let inline = message_to_wire("p", &v).unwrap();
        assert!(
            with_reg.len() < inline.len(),
            "type-id encode ({}) must elide names vs inline ({})",
            with_reg.len(),
            inline.len()
        );
        // Decoding with the same registry reconstructs the exact struct.
        let back = with_type_registry(WireTypeRegistry::new(schemas), || {
            message_from_wire(&with_reg).unwrap().1
        });
        assert_eq!(message_to_wire("p", &back).unwrap(), inline, "type-id round-trips to the exact value");
    }

    #[test]
    fn wire_struct_type_id_falls_back_to_inline_for_unknown_type() {
        // A registry that does NOT contain the struct's type → byte-identical inline form
        // (so a cross-version / non-Logos peer is never confused).
        let v = point(1, 2);
        let other = vec![("Other".to_string(), vec!["a".to_string()])];
        let bytes = with_type_registry(WireTypeRegistry::new(other), || message_to_wire("p", &v).unwrap());
        assert_eq!(bytes, message_to_wire("p", &v).unwrap(), "unknown type falls back to byte-identical inline");
    }

    #[test]
    fn wire_struct_type_id_unknown_id_fails_cleanly() {
        // Bytes encoded against one registry, decoded against an EMPTY one: the id can't
        // resolve → None (clean), never a mis-decode.
        let v = point(1, 2);
        let schemas = vec![("Point".to_string(), vec!["x".to_string(), "y".to_string()])];
        let bytes = with_type_registry(WireTypeRegistry::new(schemas), || message_to_wire("p", &v).unwrap());
        let decoded = with_type_registry(WireTypeRegistry::new(vec![]), || message_from_wire(&bytes));
        assert!(decoded.is_none(), "an unresolvable type-id must fail cleanly, not mis-decode");
    }

    // ---- Pillar D: the no-brainer auto-tuner (`best`) ----

    #[test]
    fn wire_best_smallest_is_never_larger_than_any_single_knob() {
        // The auto-tuner's contract: `best(Smallest)` ≤ EVERY single-dial encoding, on EVERY
        // workload — and it round-trips with no decode hint (every form is self-describing).
        let il = |v: Vec<i64>| RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))));
        let fl = |v: Vec<f64>| RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(v))));
        let workloads: Vec<(&str, RuntimeValue)> = vec![
            ("sequential", il((0..256).collect())),
            ("random", il((0..256).map(|i: i64| i.wrapping_mul(2_654_435_761)).collect())),
            ("repetitive", il(vec![7i64; 256])),
            ("clustered", il((0..256).map(|i| 1000 + (i % 8)).collect())),
            ("timeseries", fl((0..256).map(|i| 100.0 + i as f64 * 0.5).collect())),
            ("structs", RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..40).map(|i| point(i, i * 2)).collect(),
            ))))),
            ("int map", {
                let mut m = MapStorage::default();
                for k in 0..128i64 { m.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * k)); }
                RuntimeValue::Map(Rc::new(RefCell::new(m)))
            }),
            ("id→row map", {
                let mut m = MapStorage::default();
                for k in 0..40i64 { m.insert(RuntimeValue::Int(k), point(k, k * 2)); }
                RuntimeValue::Map(Rc::new(RefCell::new(m)))
            }),
        ];
        for (name, v) in &workloads {
            let best = message_to_wire_best("p", v, WireGoal::Smallest).unwrap();
            for num in [WireNumerics::Varint, WireNumerics::Fixed, WireNumerics::GroupVarint] {
                let s = with_numerics(num, || message_to_wire("p", v)).unwrap();
                assert!(best.len() <= s.len(), "[{name}] best {} > numerics {:?} {}", best.len(), num, s.len());
            }
            for st in [WireStructure::Off, WireStructure::Affine, WireStructure::Auto] {
                let s = with_structure(st, || message_to_wire("p", v)).unwrap();
                assert!(best.len() <= s.len(), "[{name}] best {} > structure {:?} {}", best.len(), st, s.len());
            }
            for comp in [WireCompression::None, WireCompression::Deflate, WireCompression::Lz4, WireCompression::Zstd] {
                let s = with_compression_codec(comp, || message_to_wire("p", v)).unwrap();
                assert!(best.len() <= s.len(), "[{name}] best {} > compression {:?} {}", best.len(), comp, s.len());
            }
            for f in [WireFloats::Memcpy, WireFloats::XorDelta] {
                let s = with_floats(f, || message_to_wire("p", v)).unwrap();
                assert!(best.len() <= s.len(), "[{name}] best {} > floats {:?} {}", best.len(), f, s.len());
            }
            // Round-trips: decoding `best` then re-encoding under the default dials yields the
            // exact default encoding of the original value.
            let back = message_from_wire(&best).unwrap().1;
            assert_eq!(
                message_to_wire("p", &back).unwrap(),
                message_to_wire("p", v).unwrap(),
                "[{name}] best must round-trip to the exact value"
            );
        }
    }

    #[test]
    fn wire_best_fastest_is_the_fixed_memcpy_form_and_round_trips() {
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..100).collect()))));
        let fast = message_to_wire_best("p", &v, WireGoal::Fastest).unwrap();
        let fixed = with_numerics(WireNumerics::Fixed, || message_to_wire("p", &v)).unwrap();
        assert_eq!(fast, fixed, "Fastest must be the fixed memcpy-decode form");
        let back = message_from_wire(&fast).unwrap().1;
        assert_eq!(
            message_to_wire("p", &back).unwrap(),
            message_to_wire("p", &v).unwrap(),
            "Fastest must round-trip to the exact value"
        );
    }

    #[test]
    fn wire_auto_is_the_no_brainer_default_searching_only_when_it_pays() {
        // `auto` is the "just send it" entry point. On a tiny message it ships the plain default (no
        // search overhead); on a bulk payload it runs the full Smallest bake-off. It is NEVER larger
        // than the default and always round-trips.
        let scalar = RuntimeValue::Int(42);
        assert_eq!(
            message_to_wire_auto("p", &scalar).unwrap(),
            message_to_wire("p", &scalar).unwrap(),
            "a tiny message skips the search and ships the plain default"
        );

        let bulk = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..256).collect()))));
        let auto_bulk = message_to_wire_auto("p", &bulk).unwrap();
        assert_eq!(
            auto_bulk,
            message_to_wire_best("p", &bulk, WireGoal::Smallest).unwrap(),
            "a bulk payload gets the full Smallest search"
        );
        assert!(
            auto_bulk.len() < message_to_wire("p", &bulk).unwrap().len(),
            "and the search actually shrinks it below the default"
        );

        // Universal safety: never larger than the default, always round-trips — on every shape.
        let mut big_map = MapStorage::default();
        for k in 0..200i64 {
            big_map.insert(RuntimeValue::Int(k), RuntimeValue::Int(k * k));
        }
        for v in [scalar, bulk, RuntimeValue::Map(Rc::new(RefCell::new(big_map)))] {
            let a = message_to_wire_auto("p", &v).unwrap();
            assert!(
                a.len() <= message_to_wire("p", &v).unwrap().len(),
                "auto is never larger than the default"
            );
            let back = message_from_wire(&a).unwrap().1;
            assert_eq!(
                message_to_wire("p", &back).unwrap(),
                message_to_wire("p", &v).unwrap(),
                "auto round-trips to the exact value"
            );
        }
    }

    #[test]
    fn receiver_limits_refuse_a_too_deeply_nested_message() {
        // A 30-deep nested value encodes fine (well under MAX_ENCODE_DEPTH) and decodes by default, but
        // a receiver that declares a shallower max_depth REFUSES it — admission control enforced during
        // decode, before the recursion happens.
        let mut v = RuntimeValue::Nothing;
        for _ in 0..30 {
            v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![v]))));
        }
        let bytes = message_to_wire("p", &v).unwrap();
        assert!(message_from_wire(&bytes).is_some(), "default limits accept a 30-deep message");
        let tight = ReceiveLimits { max_depth: 10, ..Default::default() };
        assert!(
            with_receive_limits(tight, || message_from_wire(&bytes)).is_none(),
            "a receiver with max_depth=10 must refuse a 30-deep message"
        );
    }

    #[test]
    fn receiver_survives_a_crafted_pathologically_deep_message_no_stack_overflow() {
        // THE DoS: an attacker crafts a message that is tiny in bytes (2 per level) but THOUSANDS of
        // containers deep. Without a decode-depth bound this recurses until the receiver's stack
        // overflows — a remote crash. The guard must REFUSE it cleanly, never overflow. (Hand-built raw
        // frame: 0x00 header = Native/uncompressed/raw, then from="" , then N×[T_LIST, count=1], then a
        // terminal — bypassing the encoder's own depth cap exactly as a hostile peer would.)
        let mut bytes = vec![0x00u8, 0x00u8]; // frame header + from=""
        for _ in 0..8000 {
            bytes.push(T_LIST);
            bytes.push(0x01); // uvarint count = 1
        }
        bytes.push(T_NOTHING);
        // A tight depth budget makes the guard bail far below any stack pressure — proving the
        // MECHANISM stops the recursion (the conservative default `max_depth` carries the same
        // protection with margin to spare in production release builds).
        let limits = ReceiveLimits { max_depth: 16, ..Default::default() };
        assert!(
            with_receive_limits(limits, || message_from_wire(&bytes)).is_none(),
            "an 8000-deep crafted message must be refused, not crash the receiver"
        );
    }

    #[test]
    fn receiver_refuses_a_generator_bomb_tiny_descriptor_huge_array() {
        // The small-message-huge-output DoS that a byte budget CANNOT see: an affine column descriptor
        // `base,stride,count` is ~12 bytes but materializes `count` ints — a crafted billion-count is a
        // handful of bytes yet allocates gigabytes. `max_elements` gates the count before materializing.
        let affine = |count: u64| {
            let mut b = vec![0x00u8, 0x00u8, T_INTS_AFFINE]; // frame + from="" + tag
            write_uvarint(zigzag(0), &mut b); // base 0
            write_uvarint(zigzag(1), &mut b); // stride 1
            write_uvarint(count, &mut b);
            b
        };
        assert!(
            message_from_wire(&affine(1_000_000_000)).is_none(),
            "a 1e9-count affine descriptor (~12 bytes) must be refused, never allocated"
        );
        // A within-budget descriptor still decodes to exactly that many ints (the math-hack still works).
        match message_from_wire(&affine(100)).unwrap().1 {
            RuntimeValue::List(l) => assert_eq!(l.borrow().to_values().len(), 100, "affine still expands within budget"),
            other => panic!("expected a List, got {other:?}"),
        }
        // A tighter max_elements refuses a moderate count the default would allow, and admits a small one.
        let tight = ReceiveLimits { max_elements: 1000, ..Default::default() };
        assert!(with_receive_limits(tight, || message_from_wire(&affine(100_000))).is_none(), "max_elements=1000 refuses 100k");
        assert!(with_receive_limits(tight, || message_from_wire(&affine(500))).is_some(), "max_elements=1000 admits 500");
    }

    #[test]
    fn receiver_refuses_an_oversized_message_before_decoding() {
        // A well-formed but large message is refused at the door when it exceeds the receiver's byte
        // budget — the gross-size admission gate, checked before any decompress / decode work.
        let big = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..5000).collect()))));
        let bytes = message_to_wire("p", &big).unwrap();
        assert!(message_from_wire(&bytes).is_some(), "default byte budget accepts it");
        let tight = ReceiveLimits { max_bytes: bytes.len() - 1, ..Default::default() };
        assert!(
            with_receive_limits(tight, || message_from_wire(&bytes)).is_none(),
            "a message one byte over max_bytes is refused before decode"
        );
    }

    #[test]
    fn peer_profile_round_trips_and_falls_back_on_unknown() {
        let p = PeerProfile {
            limits: ReceiveLimits {
                max_bytes: 1 << 20,
                max_depth: 16,
                max_elements: 10_000,
                max_string_bytes: 4096,
                accept_computed: false,
            },
            registry_epoch: 0xDEAD_BEEF,
            features: FEAT_ZSTD | FEAT_TYPE_ID,
        };
        assert_eq!(decode_peer_profile(&encode_peer_profile(&p)), Some(p), "profile round-trips exactly");
        assert_eq!(
            decode_peer_profile(&encode_peer_profile(&PeerProfile::default())),
            Some(PeerProfile::default()),
            "the default profile round-trips"
        );
        // An unknown version is recognized and not mis-parsed.
        let mut bad = encode_peer_profile(&p);
        bad[0] = 99;
        assert!(decode_peer_profile(&bad).is_none(), "an unknown profile version → None (caller uses defaults)");
        // Truncation at any offset never panics or mis-decodes.
        let full = encode_peer_profile(&p);
        for cut in 0..full.len() {
            let _ = decode_peer_profile(&full[..cut]);
        }
        assert!(decode_peer_profile(&[]).is_none());
    }

    #[test]
    fn negotiate_restricts_the_sender_to_the_receivers_exposed_surface() {
        // Both peers share an epoch and speak type-id → names elided; receiver accepts computed + both
        // speak it → may ship code; strongest shared compression chosen; the receiver's budget surfaces.
        let me = PeerProfile {
            registry_epoch: 7,
            features: FEAT_ZSTD | FEAT_LZ4 | FEAT_TYPE_ID | FEAT_COMPUTED,
            ..Default::default()
        };
        let peer = PeerProfile {
            limits: ReceiveLimits { max_bytes: 4096, ..Default::default() },
            registry_epoch: 7,
            features: FEAT_LZ4 | FEAT_TYPE_ID | FEAT_COMPUTED,
        };
        let n = negotiate(&me, &peer);
        assert!(n.use_type_id, "matching epochs + both type-id → elide names");
        assert!(n.may_send_computed, "receiver accepts computed + both speak it → may ship a computation");
        assert_eq!(n.compression, WireCompression::Lz4, "strongest compression BOTH understand (peer lacks zstd)");
        assert_eq!(n.peer_max_bytes, 4096, "the receiver's byte budget surfaces to the sender");

        // A receiver that declines code, has a different epoch, and shares no compression: the sender
        // backs OFF on every axis — exposed surface respected.
        let strict = PeerProfile {
            limits: ReceiveLimits { accept_computed: false, ..Default::default() },
            registry_epoch: 999,
            features: FEAT_TYPE_ID, // no compression, no computed
        };
        let n2 = negotiate(&me, &strict);
        assert!(!n2.use_type_id, "different epochs → names must travel");
        assert!(!n2.may_send_computed, "receiver declines code → never ship a computation");
        assert_eq!(n2.compression, WireCompression::None, "no shared compression → send uncompressed");
    }

    #[test]
    fn negotiated_send_uses_all_knobs_within_the_receivers_surface() {
        let from = "p";
        let bulk = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..256).collect()))));
        let default = message_to_wire(from, &bulk).unwrap();
        // Round-trip proof by byte-stability (container `PartialEq` is shallow): decode, re-encode under
        // the default dials, compare to the default encoding of the original.
        let roundtrips = |enc: &[u8]| {
            let back = message_from_wire(enc).unwrap().1;
            assert_eq!(message_to_wire(from, &back).unwrap(), default, "negotiated send must round-trip exactly");
        };

        // FULL surface (both speak zstd): the bake-off crushes the bulk list below the default, and it
        // still decodes with no hint.
        let full = Negotiated {
            use_type_id: false,
            may_send_computed: true,
            compression: WireCompression::Zstd,
            peer_max_bytes: 1 << 20,
        };
        let enc = message_to_wire_negotiated(from, &bulk, &full, WireTypeRegistry::new(Vec::new())).unwrap();
        assert!(enc.len() <= default.len(), "negotiated is never larger than the default");
        assert!(enc.len() < default.len(), "the negotiated bake-off shrinks a 256-int list");
        roundtrips(&enc);

        // A receiver that shares NO compression: the send never uses a codec it can't decode, still
        // round-trips, and is still ≤ default.
        let nocomp = Negotiated { compression: WireCompression::None, ..full };
        let enc_nc = message_to_wire_negotiated(from, &bulk, &nocomp, WireTypeRegistry::new(Vec::new())).unwrap();
        assert!(enc_nc.len() <= default.len());
        roundtrips(&enc_nc);

        // TYPE-ID name elision fires ONLY when negotiated, shrinking a struct with long field names.
        use crate::interpreter::StructValue;
        let mut fields = HashMap::new();
        fields.insert("organization_identifier".to_string(), RuntimeValue::Int(1));
        fields.insert("organization_display_name".to_string(), RuntimeValue::Text(Rc::new("ACME".to_string())));
        let s = RuntimeValue::Struct(Box::new(StructValue { type_name: "Organization".to_string(), fields }));
        let names = vec!["organization_display_name".to_string(), "organization_identifier".to_string()];
        let inline = message_to_wire(from, &s).unwrap();
        let with_id = Negotiated { use_type_id: true, ..nocomp };
        let elided = message_to_wire_negotiated(
            from,
            &s,
            &with_id,
            WireTypeRegistry::new(vec![("Organization".to_string(), names.clone())]),
        )
        .unwrap();
        assert!(elided.len() < inline.len(), "type-id elides the long field NAMES from the wire");
        // It decodes when the receiver shares the same registry (the negotiated condition), to the same value.
        let back = with_type_registry(
            WireTypeRegistry::new(vec![("Organization".to_string(), names)]),
            || message_from_wire(&elided),
        )
        .unwrap()
        .1;
        assert_eq!(message_to_wire(from, &back).unwrap(), inline, "the struct round-trips through the shared registry");

        // A computation the receiver DECLINED is refused at send (defense in depth).
        use crate::ast::stmt::{BinaryOpKind, Expr, Literal};
        use logicaffeine_base::{Arena, Symbol};
        let a: Arena<Expr> = Arena::new();
        let i = Symbol::from_index(0);
        let idx: &Expr = a.alloc(Expr::Identifier(i));
        let one: &Expr = a.alloc(Expr::Literal(Literal::Number(1)));
        let fbody: &Expr = a.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: idx, right: one });
        let gen = lower_expr_to_genexpr(fbody, i).unwrap();
        let f = RuntimeValue::Function(Box::new(ClosureValue {
            body_index: usize::MAX,
            captured_env: std::collections::HashMap::default(),
            param_names: vec![i],
            generated: Some(Rc::new(gen)),
        }));
        let declined = Negotiated { may_send_computed: false, ..full };
        assert!(
            message_to_wire_negotiated(from, &f, &declined, WireTypeRegistry::new(Vec::new())).is_err(),
            "a computation the receiver declined is refused at SEND"
        );
        let accepted = Negotiated { may_send_computed: true, ..full };
        assert!(
            message_to_wire_negotiated(from, &f, &accepted, WireTypeRegistry::new(Vec::new())).is_ok(),
            "an accepted computation is sent"
        );
    }

    #[test]
    fn wire_type_registry_epoch_is_deterministic_and_distinguishes_type_sets() {
        // The registry epoch is the handshake's "do we share types?" key. Same types declared in any
        // field order → SAME epoch (so two same-program peers match and may elide names); a different
        // type set → a different epoch; empty → 0 (never elide).
        let a = WireTypeRegistry::new(vec![("Org".to_string(), vec!["id".to_string(), "name".to_string()])]);
        let b = WireTypeRegistry::new(vec![("Org".to_string(), vec!["name".to_string(), "id".to_string()])]);
        assert_eq!(a.epoch(), b.epoch(), "same types in any field order → identical epoch");
        assert_ne!(a.epoch(), 0, "a non-empty registry is never epoch 0");
        let c = WireTypeRegistry::new(vec![("User".to_string(), vec!["id".to_string()])]);
        assert_ne!(a.epoch(), c.epoch(), "a different type set → a different epoch");
        assert_eq!(WireTypeRegistry::new(vec![]).epoch(), 0, "empty registry → epoch 0 (never elide)");
        // Enums participate too: adding an enum changes the epoch, order-independently.
        let with_enum = WireTypeRegistry::new(vec![("Org".to_string(), vec!["id".to_string(), "name".to_string()])])
            .with_enums(vec![("Color".to_string(), vec!["Red".to_string(), "Green".to_string()])]);
        assert_ne!(a.epoch(), with_enum.epoch(), "adding an enum type changes the epoch");
    }

    #[test]
    fn handshake_frame_round_trips_and_is_never_confused_with_a_data_message() {
        let prof = PeerProfile {
            limits: ReceiveLimits { max_bytes: 4096, accept_computed: false, ..Default::default() },
            registry_epoch: 42,
            features: FEAT_LZ4 | FEAT_TYPE_ID,
        };
        let frame = make_handshake_frame("alice", &prof);
        assert_eq!(parse_handshake_frame(&frame), Some(("alice".to_string(), prof)), "handshake round-trips");

        // A real data message is NEVER parsed as a handshake (the magic can't collide with a frame header).
        let data = message_to_wire("alice", &RuntimeValue::Int(7)).unwrap();
        assert!(parse_handshake_frame(&data).is_none(), "a data message is not a handshake");
        // …and a handshake frame is not a decodable data message.
        assert!(message_from_wire(&frame).is_none(), "a handshake frame is not a data message");
        // Truncations never panic or mis-parse.
        for cut in 0..frame.len() {
            let _ = parse_handshake_frame(&frame[..cut]);
        }
    }

    #[test]
    fn wire_struct_list_type_id_elides_names_and_beats_inline() {
        // The BULK case: a homogeneous struct LIST. With a shared registry, the columnar
        // list ships its type id + N + columns — the type/field NAMES never go on the wire,
        // on the FIRST message. Strictly smaller than the self-describing `T_STRUCTS`.
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
            (0..50).map(|i| point(i, i * 2)).collect(),
        ))));
        let schemas = vec![("Point".to_string(), vec!["x".to_string(), "y".to_string()])];
        let with_reg = with_type_registry(WireTypeRegistry::new(schemas.clone()), || {
            message_to_wire("p", &v).unwrap()
        });
        let inline = message_to_wire("p", &v).unwrap();
        assert!(
            with_reg.len() < inline.len(),
            "struct-list type-id ({}) must elide names vs inline ({})",
            with_reg.len(),
            inline.len()
        );
        let back = with_type_registry(WireTypeRegistry::new(schemas), || {
            message_from_wire(&with_reg).unwrap().1
        });
        assert_eq!(
            message_to_wire("p", &back).unwrap(),
            inline,
            "struct-list type-id round-trips to the exact value"
        );
    }

    #[test]
    fn wire_struct_list_type_id_falls_back_for_unknown_type() {
        // A registry without this type → byte-identical self-describing `T_STRUCTS`.
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
            (0..10).map(|i| point(i, i)).collect(),
        ))));
        let other = vec![("Other".to_string(), vec!["a".to_string()])];
        let bytes = with_type_registry(WireTypeRegistry::new(other), || message_to_wire("p", &v).unwrap());
        assert_eq!(
            bytes,
            message_to_wire("p", &v).unwrap(),
            "unknown type falls back to byte-identical inline T_STRUCTS"
        );
    }

    #[test]
    fn wire_struct_list_type_id_unknown_id_fails_cleanly() {
        // Encoded against one registry, decoded against an EMPTY one: the id can't resolve
        // → None (clean), never a mis-decode.
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
            (0..10).map(|i| point(i, i)).collect(),
        ))));
        let schemas = vec![("Point".to_string(), vec!["x".to_string(), "y".to_string()])];
        let bytes = with_type_registry(WireTypeRegistry::new(schemas), || message_to_wire("p", &v).unwrap());
        let decoded = with_type_registry(WireTypeRegistry::new(vec![]), || message_from_wire(&bytes));
        assert!(decoded.is_none(), "an unresolvable struct-list type-id must fail cleanly, not mis-decode");
    }

    #[test]
    fn wire_struct_type_id_registry_order_independent() {
        // The id is content-addressed (canonical by fingerprint), so two registries that
        // declare the same types in different ORDER assign the same ids — sender and
        // receiver always agree regardless of declaration order.
        let v = point(7, 9);
        let a = vec![
            ("Point".to_string(), vec!["x".to_string(), "y".to_string()]),
            ("Other".to_string(), vec!["a".to_string()]),
        ];
        let mut b = a.clone();
        b.reverse();
        let enc_a = with_type_registry(WireTypeRegistry::new(a), || message_to_wire("p", &v).unwrap());
        let dec_b = with_type_registry(WireTypeRegistry::new(b), || message_from_wire(&enc_a).unwrap().1);
        assert_eq!(message_to_wire("p", &dec_b).unwrap(), message_to_wire("p", &v).unwrap());
    }

    #[test]
    fn wire_enum_type_id_elides_type_and_constructor_names() {
        // An enum (Inductive) under a shared registry ships its enum-id + a constructor
        // INDEX instead of the type and constructor NAMES — both ends know the ordered
        // constructor list from their shared type def. Strictly smaller than inline.
        let v = enum_val("Color", "Green", vec![]);
        let enums = vec![("Color".to_string(), vec!["Red".to_string(), "Green".to_string(), "Blue".to_string()])];
        let reg = || WireTypeRegistry::new(vec![]).with_enums(enums.clone());
        let with_reg = with_type_registry(reg(), || message_to_wire("p", &v).unwrap());
        let inline = message_to_wire("p", &v).unwrap();
        assert!(with_reg.len() < inline.len(), "enum type-id ({}) elides names vs inline ({})", with_reg.len(), inline.len());
        let back = with_type_registry(reg(), || message_from_wire(&with_reg).unwrap().1);
        assert_eq!(message_to_wire("p", &back).unwrap(), inline, "enum type-id round-trips to the exact value");
    }

    #[test]
    fn wire_enum_type_id_carries_args_and_falls_back_when_unknown() {
        // A non-nullary constructor's args ride along; an enum NOT in the registry stays
        // the byte-identical self-describing inline form.
        let some = enum_val("Option", "Some", vec![RuntimeValue::Int(7)]);
        let enums = vec![("Option".to_string(), vec!["None".to_string(), "Some".to_string()])];
        let with_reg = with_type_registry(
            WireTypeRegistry::new(vec![]).with_enums(enums.clone()),
            || message_to_wire("p", &some).unwrap(),
        );
        let back = with_type_registry(
            WireTypeRegistry::new(vec![]).with_enums(enums),
            || message_from_wire(&with_reg).unwrap().1,
        );
        assert_eq!(message_to_wire("p", &back).unwrap(), message_to_wire("p", &some).unwrap());
        // An unrelated registry (no Option) → byte-identical inline.
        let other = WireTypeRegistry::new(vec![]).with_enums(vec![("X".to_string(), vec!["A".to_string()])]);
        let bytes = with_type_registry(other, || message_to_wire("p", &some).unwrap());
        assert_eq!(bytes, message_to_wire("p", &some).unwrap(), "unknown enum falls back to inline");
    }

    // ---- Pillar A: offset-table struct view (beat Cap'n Proto random access) --------

    #[test]
    fn wire_struct_view_round_trips() {
        let mut fields = HashMap::new();
        fields.insert("a".to_string(), RuntimeValue::Int(1));
        fields.insert("b".to_string(), RuntimeValue::Int(2));
        fields.insert("c".to_string(), RuntimeValue::Int(99));
        let v = RuntimeValue::Struct(Box::new(StructValue { type_name: "Rec".to_string(), fields }));
        let bytes = with_struct_view(true, || message_to_wire("p", &v).unwrap());
        let back = message_from_wire(&bytes).unwrap().1;
        assert_eq!(
            message_to_wire("p", &back).unwrap(),
            message_to_wire("p", &v).unwrap(),
            "offset-table view round-trips to the exact struct"
        );
    }

    #[test]
    fn wire_struct_view_reads_one_field_without_parsing_the_rest() {
        // Cap'n Proto-class random access: a struct with a HUGE field and a small field.
        // Reading the small field via the offset table is O(1) — a THOUSAND such reads must
        // be cheaper than ONE full decode, proving it never parses the huge field.
        let big: Vec<i64> = (0..1_000_000).map(|i| i as i64).collect();
        let mut fields = HashMap::new();
        fields.insert("big".to_string(), RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(big)))));
        fields.insert("small".to_string(), RuntimeValue::Int(4242));
        let v = RuntimeValue::Struct(Box::new(StructValue { type_name: "Rec".to_string(), fields }));
        // The DEFAULT (checksummed) message — the view is STILL O(1): `view_message` does
        // not re-hash the body (that would defeat random access), so `Raw` is NOT required
        // for zero-copy. The offset-table field jump never touches the 1M-element field.
        let bytes = with_struct_view(true, || message_to_wire("p", &v).unwrap());

        let view = view_message(&bytes).unwrap();
        assert_eq!(
            view.struct_field("small").and_then(|f| f.as_int()),
            Some(4242),
            "the offset table reads the small field directly"
        );

        let reads = {
            let t = std::time::Instant::now();
            for _ in 0..1000 {
                let view = view_message(&bytes).unwrap();
                std::hint::black_box(view.struct_field("small").and_then(|f| f.as_int()));
            }
            t.elapsed().as_nanos()
        };
        let full = {
            let t = std::time::Instant::now();
            std::hint::black_box(message_from_wire(&bytes).unwrap());
            t.elapsed().as_nanos()
        };
        assert!(
            reads < full,
            "1000 O(1) view reads ({reads}ns) must beat ONE full decode ({full}ns) — capnp-class random access"
        );
    }

    #[test]
    fn wire_aligned_int_column_reads_zero_copy_as_slice() {
        // The in-place column read — the LAN / kernel-bypass axis Cap'n Proto owns. An
        // aligned i64 column reads back as `&[i64]` with ZERO copy: the slice BORROWS the
        // message bytes, no allocation and no per-element decode, the way an io_uring / RDMA
        // receiver reads pre-registered, alignment-guaranteed buffers in place.
        let data: Vec<i64> = (0..1000).map(|i| i * 7 - 3).collect();
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(data.clone()))));
        let bytes = with_struct_view(true, || message_to_wire("p", &value).unwrap());

        // Place the framed message in an 8-byte-aligned buffer, as a real zero-copy receiver
        // would — a `Vec<i64>` allocation is 8-aligned by `i64`'s alignment, and the encoder
        // padded the column so its final-buffer offset is ≡ 0 mod 8.
        let mut backing = vec![0i64; bytes.len() / 8 + 2];
        // SAFETY: copy the message into the aligned backing's bytes; thereafter read-only.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), backing.as_mut_ptr().cast::<u8>(), bytes.len());
        }
        let abytes: &[u8] = unsafe { std::slice::from_raw_parts(backing.as_ptr().cast::<u8>(), bytes.len()) };

        let view = view_message(abytes).unwrap();
        let slice = view.as_i64_slice().expect("an aligned column reads zero-copy as &[i64]");
        assert_eq!(slice, &data[..], "the zero-copy slice equals the column data");

        // It BORROWS the message buffer (zero allocation): the slice lives inside `abytes`
        // and is 8-byte aligned (a sound `&[i64]` cast on every architecture, not just x86).
        let base = abytes.as_ptr() as usize;
        let lo = slice.as_ptr() as usize;
        assert!(lo >= base && lo < base + abytes.len(), "the slice borrows the message bytes (zero-copy)");
        assert_eq!(lo % 8, 0, "the column blob is 8-byte aligned");

        // The same bytes still round-trip through a full owned decode (the T_INTS_ALIGNED
        // decode arm), re-encoding to the exact aligned form.
        let back = message_from_wire(abytes).unwrap().1;
        let re = with_struct_view(true, || message_to_wire("p", &back).unwrap());
        assert_eq!(re, bytes, "the aligned column also decodes + re-encodes to the exact bytes");
    }

    #[test]
    fn wire_aligned_int_column_falls_back_to_copy_when_unaligned() {
        // When the receiver's buffer is NOT 8-aligned at the column, `as_i64_slice` returns
        // None (no UB) and the caller copies via the full decode — which still round-trips.
        let data: Vec<i64> = (0..64).map(|i| i - 32).collect();
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(data.clone()))));
        let bytes = with_struct_view(true, || message_to_wire("p", &value).unwrap());

        // Force a deliberate 1-byte misalignment by prepending a byte to an aligned backing,
        // so the message body starts at an odd offset and the column can't be 8-aligned.
        let mut backing = vec![0i64; bytes.len() / 8 + 2];
        let raw = unsafe { std::slice::from_raw_parts_mut(backing.as_mut_ptr().cast::<u8>(), bytes.len() + 1) };
        raw[1..bytes.len() + 1].copy_from_slice(&bytes);
        let shifted: &[u8] = &raw[1..bytes.len() + 1];

        let view = view_message(shifted).unwrap();
        // Either the misalignment made the cast unsound (→ None, copy fallback) — the
        // important invariant is no panic and a correct owned decode regardless.
        if let Some(slice) = view.as_i64_slice() {
            assert_eq!(slice.as_ptr() as usize % 8, 0, "if it returned a slice it MUST be aligned");
        }
        let back = message_from_wire(shifted).unwrap().1;
        let re = with_struct_view(true, || message_to_wire("p", &back).unwrap());
        assert_eq!(re, bytes, "the unaligned column still decodes correctly via copy");
    }

    #[test]
    fn wire_aligned_float_column_reads_zero_copy_as_slice() {
        // The float twin of the zero-copy i64 read: an aligned f64 column reads back as
        // `&[f64]` with no copy, BIT-EXACT — including NaN / ±Inf / subnormals, which the
        // `&[f64]` cast carries verbatim (no per-element decode could change a bit).
        let mut data: Vec<f64> = (0..1000).map(|i| i as f64 * 1.5 - 7.0).collect();
        data[3] = f64::NAN;
        data[5] = f64::INFINITY;
        data[7] = f64::NEG_INFINITY;
        data[9] = f64::MIN_POSITIVE / 4.0; // a subnormal
        data[11] = -0.0;
        let value = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(data.clone()))));
        let bytes = with_struct_view(true, || message_to_wire("p", &value).unwrap());

        let mut backing = vec![0i64; bytes.len() / 8 + 2];
        // SAFETY: copy the message into the 8-aligned backing's bytes; thereafter read-only.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), backing.as_mut_ptr().cast::<u8>(), bytes.len());
        }
        let abytes: &[u8] = unsafe { std::slice::from_raw_parts(backing.as_ptr().cast::<u8>(), bytes.len()) };

        let view = view_message(abytes).unwrap();
        let slice = view.as_f64_slice().expect("an aligned float column reads zero-copy as &[f64]");
        // Bit-exact comparison (NaN != NaN under `==`, so compare the raw bits).
        let got: Vec<u64> = slice.iter().map(|x| x.to_bits()).collect();
        let want: Vec<u64> = data.iter().map(|x| x.to_bits()).collect();
        assert_eq!(got, want, "the zero-copy float slice is bit-exact (NaN/Inf/subnormal/-0 preserved)");

        // It BORROWS the message buffer (zero allocation) and is 8-byte aligned.
        let base = abytes.as_ptr() as usize;
        let lo = slice.as_ptr() as usize;
        assert!(lo >= base && lo < base + abytes.len(), "the float slice borrows the message bytes (zero-copy)");
        assert_eq!(lo % 8, 0, "the float column blob is 8-byte aligned");

        // Full owned decode round-trips the exact aligned form too.
        let back = message_from_wire(abytes).unwrap().1;
        let re = with_struct_view(true, || message_to_wire("p", &back).unwrap());
        assert_eq!(re, bytes, "the aligned float column also decodes + re-encodes to the exact bytes");
    }

    #[test]
    fn wire_structs_view_round_trips() {
        // A record LIST in the random-access view round-trips to the exact same bytes.
        let mut rows = Vec::new();
        for i in 0..50i64 {
            let mut fields = HashMap::new();
            fields.insert("id".to_string(), RuntimeValue::Int(i));
            fields.insert("score".to_string(), RuntimeValue::Int(i * 3 - 7));
            fields.insert("active".to_string(), RuntimeValue::Bool(i % 2 == 0));
            rows.push(RuntimeValue::Struct(Box::new(StructValue { type_name: "Row".to_string(), fields })));
        }
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
        let bytes = with_struct_view(true, || message_to_wire("p", &v).unwrap());
        let back = message_from_wire(&bytes).unwrap().1;
        assert_eq!(
            with_struct_view(true, || message_to_wire("p", &back).unwrap()),
            bytes,
            "record-list view round-trips to the exact bytes"
        );
    }

    #[test]
    fn wire_structs_view_reads_one_field_of_one_row_without_parsing_the_rest() {
        // O(1) random access into a record list: each row carries a big `blob` column, but
        // reading row r's small `id` must jump straight there via the row + field offset
        // tables — never materializing the blobs. The Cap'n Proto-class record-list read.
        let big: Vec<i64> = (0..1000).collect();
        let mut rows = Vec::new();
        for i in 0..200i64 {
            let mut fields = HashMap::new();
            fields.insert("id".to_string(), RuntimeValue::Int(i * 1000 + 1));
            fields.insert(
                "blob".to_string(),
                RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(big.clone())))),
            );
            rows.push(RuntimeValue::Struct(Box::new(StructValue { type_name: "Row".to_string(), fields })));
        }
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(rows))));
        let bytes = with_struct_view(true, || message_to_wire("p", &v).unwrap());
        let view = view_message(&bytes).unwrap();

        assert_eq!(view.structs_len(), Some(200), "row count read from the view head");
        assert_eq!(
            view.structs_row_field(7, "id").and_then(|f| f.as_int()),
            Some(7 * 1000 + 1),
            "O(1) read of row 7's id"
        );
        assert_eq!(
            view.structs_row_field(199, "id").and_then(|f| f.as_int()),
            Some(199 * 1000 + 1),
            "O(1) read of the last row's id"
        );
        assert!(view.structs_row_field(200, "id").is_none(), "row out of range → None");
        assert!(view.structs_row_field(0, "nope").is_none(), "unknown field → None");

        // The O(1) random reads beat ONE full decode (which materializes every blob column).
        let idxs: Vec<usize> = (0..1000).map(|k| (k * 7) % 200).collect();
        let reads = {
            let t = std::time::Instant::now();
            let mut acc = 0i64;
            for &i in &idxs {
                acc = acc.wrapping_add(view.structs_row_field(i, "id").unwrap().as_int().unwrap());
            }
            std::hint::black_box(acc);
            t.elapsed().as_nanos()
        };
        let full = {
            let t = std::time::Instant::now();
            std::hint::black_box(message_from_wire(&bytes).unwrap());
            t.elapsed().as_nanos()
        };
        assert!(
            reads < full,
            "1000 O(1) row-field reads ({reads}ns) must beat ONE full decode ({full}ns) — record-list random access"
        );
    }

    #[test]
    fn wire_cyclic_value_fails_cleanly_instead_of_overflowing() {
        // A self-referential list (constructible via the `Rc<RefCell<…>>` a List wraps) must
        // NOT stack-overflow the recursive encoder — it returns a clean Err. Completeness /
        // robustness: the codec never crashes on a value, however pathological.
        let cell = Rc::new(RefCell::new(ListRepr::Boxed(vec![])));
        let list = RuntimeValue::List(cell.clone());
        *cell.borrow_mut() = ListRepr::Boxed(vec![list.clone()]); // the list now contains itself
        let result = message_to_wire("p", &list);
        *cell.borrow_mut() = ListRepr::Boxed(vec![]); // break the cycle so the Rc doesn't leak
        assert!(result.is_err(), "a cyclic value must return Err, not overflow the stack");
    }

    #[test]
    fn wire_deeply_nested_value_round_trips_below_the_guard() {
        // A legitimately deep (but finite) nesting still round-trips — the cycle guard only
        // rejects the pathological, never real data. Build via `from_values` so the value is
        // already canonical (the codec de-boxes on decode, so a hand-built `Boxed` would not
        // compare equal — that is canonicalization, not a round-trip failure).
        let mut v = RuntimeValue::Int(7);
        for _ in 0..40 {
            v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vec![v]))));
        }
        let bytes = message_to_wire("p", &v).expect("deep-but-finite nesting encodes");
        let back = message_from_wire(&bytes).expect("deep nesting decodes").1;
        // Byte-stable round-trip (List equality is Rc-identity, so re-encode and compare the
        // canonical bytes — the idiom every round-trip lock-in in this file uses).
        assert_eq!(message_to_wire("p", &back).unwrap(), bytes, "deep nesting round-trips exactly");
    }

    #[test]
    fn wire_schema_def_decodes_without_a_cache() {
        // The FIRST cached message carries the schema inline (a "def"), so even a
        // stateless decoder handles it.
        let v = struct_list(20);
        let mut send_cache = WireSchemaCache::default();
        let m1 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut send_cache).unwrap();
        let d = message_from_wire(&m1).unwrap().1;
        assert_eq!(message_to_wire("p", &d).unwrap(), message_to_wire("p", &v).unwrap());
    }

    #[test]
    fn wire_schema_ref_without_cache_fails_cleanly() {
        // A later message is a schema "ref"; a stateless decoder has no schema for that
        // id, so it returns None — never a panic, never a wrong value.
        let v = struct_list(20);
        let mut send_cache = WireSchemaCache::default();
        let _m1 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut send_cache).unwrap();
        let m2 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut send_cache).unwrap();
        assert!(message_from_wire(&m2).is_none(), "a schema-ref without a cache decodes to None");
    }

    fn person(i: i64) -> RuntimeValue {
        let mut f = HashMap::new();
        f.insert("name".to_string(), RuntimeValue::Text(Rc::new(format!("n{i}"))));
        f.insert("score".to_string(), RuntimeValue::Int(i));
        RuntimeValue::Struct(Box::new(StructValue { type_name: "Person".to_string(), fields: f }))
    }

    #[test]
    fn wire_schema_dictionary_distinct_schemas_get_distinct_ids() {
        // Two different struct shapes each get their own schema id; interleaved sends
        // round-trip exactly through synchronized caches.
        let points = struct_list(10);
        let people = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed((0..10).map(person).collect()))));
        let mut sc = WireSchemaCache::default();
        let mut rc = WireSchemaCache::default();
        let mut enc = |x: &RuntimeValue, c: &mut WireSchemaCache| {
            message_to_wire_cached("p", x, WireCodec::Native, WireIntegrity::Raw, c).unwrap()
        };
        let seq = [enc(&points, &mut sc), enc(&people, &mut sc), enc(&points, &mut sc), enc(&people, &mut sc)];
        let originals = [&points, &people, &points, &people];
        for (bytes, orig) in seq.iter().zip(originals) {
            let d = message_from_wire_cached(bytes, &mut rc).unwrap().1;
            assert_eq!(message_to_wire("p", &d).unwrap(), message_to_wire("p", orig).unwrap());
        }
    }

    #[test]
    fn wire_schema_cache_handles_nested_struct_columns() {
        // A struct whose field is itself a list of structs — the NESTED schema is also
        // dictionaried (its own id), so the 2nd message is smaller and both round-trip.
        let inner = || RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed((0..3).map(|i| point(i, i)).collect()))));
        let outer: Vec<RuntimeValue> = (0..5)
            .map(|i| {
                let mut f = HashMap::new();
                f.insert("id".to_string(), RuntimeValue::Int(i));
                f.insert("pts".to_string(), inner());
                RuntimeValue::Struct(Box::new(StructValue { type_name: "Bag".to_string(), fields: f }))
            })
            .collect();
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(outer))));
        let mut sc = WireSchemaCache::default();
        let mut rc = WireSchemaCache::default();
        let mut enc = |c: &mut WireSchemaCache| message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, c).unwrap();
        let m1 = enc(&mut sc);
        let m2 = enc(&mut sc);
        assert!(m2.len() < m1.len(), "nested schemas reference on the 2nd message: {} vs {}", m2.len(), m1.len());
        let d1 = message_from_wire_cached(&m1, &mut rc).unwrap().1;
        let d2 = message_from_wire_cached(&m2, &mut rc).unwrap().1;
        assert_eq!(message_to_wire("p", &d1).unwrap(), message_to_wire("p", &v).unwrap());
        assert_eq!(message_to_wire("p", &d2).unwrap(), message_to_wire("p", &v).unwrap());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wire_schema_cache_composes_with_compression_and_checksum() {
        // Schema-by-reference + zstd + FNV checksum all stack: the ref message is
        // smaller and round-trips through the verified, inflated, schema-resolved path.
        let v = struct_list(300);
        let mut sc = WireSchemaCache::default();
        let mut rc = WireSchemaCache::default();
        let mut enc = |c: &mut WireSchemaCache| {
            with_compression_codec(WireCompression::Zstd, || {
                message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Checked, c).unwrap()
            })
        };
        let m1 = enc(&mut sc);
        let m2 = enc(&mut sc);
        assert!(m2.len() < m1.len(), "compressed+checked ref < def: {} vs {}", m2.len(), m1.len());
        assert!(message_from_wire_cached(&m1, &mut rc).is_some());
        let d2 = message_from_wire_cached(&m2, &mut rc).unwrap().1;
        assert_eq!(message_to_wire("p", &d2).unwrap(), message_to_wire("p", &v).unwrap());
    }

    #[test]
    fn wire_schema_cache_fuzz_never_diverges_from_stateless() {
        // The proof: over many random message sequences through synchronized caches,
        // every decoded value equals the original (canonical stateless re-encode). The
        // cached protocol must never change the MEANING — only the bytes. Schemas
        // repeat, so definitions, references, and non-struct messages all interleave.
        fn gen_msg(rng: &mut SplitMix64) -> RuntimeValue {
            match rng.below(6) {
                0 => RuntimeValue::Int(rng.next() as i64),
                1 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..rng.below(20) as i64).collect())))),
                2 => struct_list(rng.below(20) as i64),
                3 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                    (0..rng.below(20) as i64).map(person).collect(),
                )))),
                4 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                    (0..rng.below(15)).map(|i| RuntimeValue::Text(Rc::new(format!("s{i}")))).collect(),
                )))),
                _ => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                    (0..rng.below(12) as i64)
                        .map(|i| if i % 2 == 0 { enum_val("Option", "Some", vec![RuntimeValue::Int(i)]) } else { enum_val("Option", "None", vec![]) })
                        .collect(),
                )))),
            }
        }
        for seed in [1u64, 7, 42, 99, 1000, 0xDEAD_BEEF, 0x00AB_CDEF] {
            let mut rng = SplitMix64 { state: seed };
            let mut sc = WireSchemaCache::default();
            let mut rc = WireSchemaCache::default();
            for step in 0..120 {
                let v = gen_msg(&mut rng);
                let bytes = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap();
                let (_from, back) = message_from_wire_cached(&bytes, &mut rc).unwrap_or_else(|| panic!("seed {seed} step {step}: cached decode returned None"));
                assert_eq!(
                    message_to_wire("p", &back).unwrap(),
                    message_to_wire("p", &v).unwrap(),
                    "seed {seed} step {step}: cached round-trip changed the value"
                );
            }
        }
    }

    #[test]
    fn wire_schema_content_addressed_survives_multi_sender_reorder_and_loss() {
        // THE FOOTGUN PROOF. Many senders share ONE receiver cache; the stream is
        // adversarially reordered and ~20% dropped. Every decode is EITHER exactly
        // correct OR None — never a wrong value. (Sequential ids would corrupt here;
        // content-addressing cannot, because the id IS the schema's content.)
        fn gen(rng: &mut SplitMix64) -> RuntimeValue {
            match rng.below(4) {
                0 => struct_list(rng.below(15) as i64),
                1 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed((0..rng.below(15) as i64).map(person).collect())))),
                2 => RuntimeValue::Int(rng.next() as i64),
                _ => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..rng.below(15) as i64).collect())))),
            }
        }
        for seed in [3u64, 17, 71, 2024, 0xFEED, 0xBADF00D] {
            let mut rng = SplitMix64 { state: seed };
            // Three senders, each its OWN content-addressed send cache.
            let mut sends: Vec<WireSchemaCache> = (0..3).map(|_| WireSchemaCache::content_addressed()).collect();
            let mut stream: Vec<(Vec<u8>, RuntimeValue)> = Vec::new();
            for _ in 0..80 {
                let s = rng.below(3) as usize;
                let v = gen(&mut rng);
                let bytes = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sends[s]).unwrap();
                stream.push((bytes, v));
            }
            // Adversary: Fisher–Yates shuffle (reordering) ...
            for i in (1..stream.len()).rev() {
                let j = rng.below((i + 1) as u64) as usize;
                stream.swap(i, j);
            }
            // ... and decode through ONE shared receiver, dropping ~20%.
            let mut recv = WireSchemaCache::content_addressed();
            for (bytes, orig) in &stream {
                if rng.below(5) == 0 {
                    continue; // dropped before reaching the decoder (loss)
                }
                if let Some((_from, back)) = message_from_wire_cached(bytes, &mut recv) {
                    assert_eq!(
                        message_to_wire("p", &back).unwrap(),
                        message_to_wire("p", orig).unwrap(),
                        "seed {seed}: a shared cache under reorder+loss decoded the WRONG value"
                    );
                }
                // None is an acceptable outcome (a reference whose definition was
                // dropped or has not yet arrived) — it is a clean miss, not corruption.
            }
        }
    }

    #[test]
    fn wire_schema_keyframe_self_heals_after_missed_definition() {
        // With a keyframe interval, the sender re-defines the schema every k references,
        // so a receiver that joined late (missed the first definition) recovers.
        let v = struct_list(10);
        let mut send = WireSchemaCache::content_addressed().with_keyframe(2);
        let msgs: Vec<Vec<u8>> = (0..6)
            .map(|_| message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut send).unwrap())
            .collect();
        // Emission pattern is [Def, Ref, Ref, Def(keyframe), Ref, Ref].
        let mut recv = WireSchemaCache::content_addressed();
        assert!(message_from_wire_cached(&msgs[1], &mut recv).is_none(), "a reference before any definition → None");
        assert!(message_from_wire_cached(&msgs[3], &mut recv).is_some(), "the keyframe re-definition decodes and heals the cache");
        let d = message_from_wire_cached(&msgs[4], &mut recv).unwrap().1;
        assert_eq!(message_to_wire("p", &d).unwrap(), message_to_wire("p", &v).unwrap(), "references resolve after the keyframe");
    }

    #[test]
    fn wire_schema_mode_dial_trades_size() {
        // The dial: sequential (1-byte id) < content-addressed (8-byte fingerprint) <
        // inline (full schema). All three round-trip; smaller modes need more
        // discipline (sequential = one ordered sender), proven safe elsewhere.
        let v = struct_list(50);
        let ref_size = |mode_cache: fn() -> WireSchemaCache| {
            let mut c = mode_cache();
            let _def = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut c).unwrap();
            message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut c).unwrap().len()
        };
        let seq = ref_size(WireSchemaCache::sequential);
        let ca = ref_size(WireSchemaCache::content_addressed);
        let inline = message_to_wire("p", &v).unwrap().len();
        assert!(seq < ca, "sequential ref ({seq}) < content-addressed ref ({ca})");
        assert!(ca < inline, "content-addressed ref ({ca}) < inline schema ({inline})");
        // Sequential round-trips on a single ordered stream.
        let mut s = WireSchemaCache::sequential();
        let mut r = WireSchemaCache::sequential();
        let m1 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut s).unwrap();
        let m2 = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut s).unwrap();
        let d1 = message_from_wire_cached(&m1, &mut r).unwrap().1;
        let d2 = message_from_wire_cached(&m2, &mut r).unwrap().1;
        assert_eq!(message_to_wire("p", &d1).unwrap(), message_to_wire("p", &v).unwrap());
        assert_eq!(message_to_wire("p", &d2).unwrap(), message_to_wire("p", &v).unwrap());
    }

    #[test]
    fn schema_cache_survives_a_panic_in_the_codec() {
        // The CacheScope RAII guard restores the thread-local cache on a panic unwind,
        // so a panic mid-codec can't strand or reset the schema state.
        let v = struct_list(20);
        let mut cache = WireSchemaCache::content_addressed();
        let _def = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut cache).unwrap();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _scope = CacheScope::enter(&mut cache);
            panic!("boom mid-codec");
        }));
        assert!(r.is_err(), "the panic propagated");
        // The cache still remembers the schema → the next send is a (smaller) reference.
        let after = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut cache).unwrap();
        let fresh = {
            let mut f = WireSchemaCache::content_addressed();
            message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut f).unwrap()
        };
        assert!(after.len() < fresh.len(), "cache survived the panic (ref {} < fresh def {})", after.len(), fresh.len());
    }

    #[test]
    fn schema_cache_sequential_and_content_addressed_recv_are_disjoint() {
        // One receiver decoding interleaved sequential AND content-addressed messages
        // never cross-resolves — sequential uses recv_seq (ids), content uses recv_ca
        // (fingerprints), which are separate state. (Disproves the audit's "mode-mixing
        // collision" worry.)
        let v = struct_list(20);
        let mut seq = WireSchemaCache::sequential();
        let s_def = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut seq).unwrap();
        let s_ref = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut seq).unwrap();
        let mut ca = WireSchemaCache::content_addressed();
        let c_def = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut ca).unwrap();
        let c_ref = message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut ca).unwrap();
        let mut rc = WireSchemaCache::content_addressed();
        for (bytes, label) in [(&s_def, "seq def"), (&c_def, "ca def"), (&s_ref, "seq ref"), (&c_ref, "ca ref")] {
            let d = message_from_wire_cached(bytes, &mut rc).unwrap_or_else(|| panic!("{label} failed to decode")).1;
            assert_eq!(message_to_wire("p", &d).unwrap(), message_to_wire("p", &v).unwrap(), "{label} reconstructs the list");
        }
    }

    #[test]
    fn wire_integrity_dial_toggles_the_checksum() {
        // The latency↔safety dial: `Raw` drops the 8-byte FNV checksum (faster, header
        // bit 0x01 unset), `Checked` keeps it. Scoped — never leaks.
        let v = struct_list(50);
        let raw = with_integrity(WireIntegrity::Raw, || message_to_wire("p", &v).unwrap());
        let checked = with_integrity(WireIntegrity::Checked, || message_to_wire("p", &v).unwrap());
        assert_eq!(raw[0] & 0x01, 0, "Raw carries no checksum");
        assert_eq!(checked[0] & 0x01, 0x01, "Checked sets the checksum bit");
        assert_eq!(checked.len(), raw.len() + 8, "the checksum is 8 bytes");
        assert!(message_from_wire(&raw).is_some() && message_from_wire(&checked).is_some(), "both decode");
        // Scoped: outside the override the process default is restored.
        let default_bit = if default_integrity() == WireIntegrity::Checked { 0x01 } else { 0 };
        assert_eq!(message_to_wire("p", &v).unwrap()[0] & 0x01, default_bit, "the override does not leak");
    }

    #[test]
    fn uvarint_byte_len_matches_write_uvarint() {
        for x in [0u64, 1, 127, 128, 16_383, 16_384, u32::MAX as u64, u64::MAX, 0x1234_5678_9ABC_DEF0] {
            let mut buf = Vec::new();
            write_uvarint(x, &mut buf);
            assert_eq!(uvarint_byte_len(x), buf.len(), "uvarint_byte_len({x:#x}) must match write_uvarint");
        }
    }

    #[test]
    fn wire_json_codec_is_real_json() {
        let v = RuntimeValue::Text(Rc::new("hi".to_string()));
        let bytes = message_to_wire_with("alice", &v, WireCodec::Json, WireIntegrity::Raw).unwrap();
        // The body after the 1-byte header parses as JSON via a real parser.
        let json: serde_json::Value = serde_json::from_slice(&bytes[1..]).expect("valid JSON body");
        assert_eq!(json["from"], "alice");
    }

    #[test]
    fn wire_native_is_far_tighter_than_json() {
        // The throughput win: a 100-int array is a fraction of the JSON size.
        let items: Vec<RuntimeValue> = (0..100).map(RuntimeValue::Int).collect();
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items))));
        let bin = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
        let json = message_to_wire_with("", &v, WireCodec::Json, WireIntegrity::Raw).unwrap();
        assert!(
            bin.len() * 2 < json.len(),
            "native ({} bytes) should be far tighter than json ({} bytes)",
            bin.len(),
            json.len()
        );
    }

    #[test]
    fn wire_decoder_never_panics_on_arbitrary_bytes() {
        let mut rng = SplitMix64 { state: 0x1234_5678 };
        for _ in 0..5000 {
            let len = rng.below(80) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| (rng.next() & 0xFF) as u8).collect();
            let _ = message_from_wire(&bytes); // must not panic
        }
        // Every possible header byte with a short body.
        for h in 0u16..=255 {
            let _ = message_from_wire(&[h as u8, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        }
    }

    #[test]
    fn wire_decoder_never_panics_on_mutated_valid_messages() {
        // Random bytes almost never form a valid tag, so they never reach the deep
        // columnar / schema-cache paths. Here we take a VALID message of each archetype
        // and mutate it every which way — truncate at every length, flip every byte to
        // a few values, scramble bytes — and assert the decoder NEVER panics (it
        // returns None or a structurally-valid Some that itself re-encodes without
        // panicking). This is the robustness floor the whole protocol rests on.
        fn archetypes() -> Vec<RuntimeValue> {
            vec![
                RuntimeValue::Int(42),
                RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..5).collect())))),
                RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats((0..5).map(|i| i as f64 * 1.5).collect())))),
                RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools((0..5).map(|i| i % 2 == 0).collect())))),
                RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                    (0..4).map(|i| RuntimeValue::Text(Rc::new(format!("s{i}")))).collect(),
                )))),
                struct_list(4),
                RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
                    (0..6)
                        .map(|i| if i % 2 == 0 { enum_val("Option", "Some", vec![RuntimeValue::Int(i)]) } else { enum_val("Option", "None", vec![]) })
                        .collect(),
                )))),
            ]
        }
        // A mutated buffer must never panic; if it decodes, the value re-encodes.
        let check = |bytes: &[u8]| {
            if let Some((_from, v)) = message_from_wire(bytes) {
                let _ = message_to_wire("p", &v);
            }
            let mut c = WireSchemaCache::content_addressed();
            if let Some((_from, v)) = message_from_wire_cached(bytes, &mut c) {
                let _ = message_to_wire("p", &v);
            }
        };
        let mut rng = SplitMix64 { state: 0x00AB_CDEF };
        for v in archetypes() {
            // Plain, cached-def, and cached-ref encodings all get mutated.
            let mut sc = WireSchemaCache::content_addressed();
            let bases = [
                message_to_wire("p", &v).unwrap(),
                message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap(),
                message_to_wire_cached("p", &v, WireCodec::Native, WireIntegrity::Raw, &mut sc).unwrap(),
            ];
            for base in &bases {
                check(base); // the valid form decodes
                for k in 0..base.len() {
                    check(&base[..k]); // every truncation
                }
                for i in 0..base.len() {
                    for delta in [0x01u8, 0x40, 0x7F, 0x80, 0xFF] {
                        let mut m = base.clone();
                        m[i] ^= delta;
                        check(&m); // every single-byte flip
                    }
                }
                for _ in 0..30 {
                    let mut m = base.clone();
                    if !m.is_empty() {
                        let i = rng.below(m.len() as u64) as usize;
                        m[i] = (rng.next() & 0xFF) as u8;
                    }
                    check(&m); // random scramble
                }
            }
        }
    }

    #[test]
    fn wire_property_random_values_are_byte_stable() {
        for seed in [1u64, 2, 7, 42, 99, 1000, 0x00AB_CDEF, 0xDEAD_BEEF] {
            let mut rng = SplitMix64 { state: seed };
            for _ in 0..150 {
                let v = gen_value(&mut rng, 4);
                assert_wire_stable(&v);
            }
        }
    }

    #[test]
    fn wire_packed_int_array_is_tight_and_stays_packed() {
        let n = 5000i64;
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..n).collect()))));
        assert_wire_stable(&v);
        let bytes = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
        // Packed: ~2 bytes/int at these magnitudes — far under a tagged element each.
        assert!(bytes.len() < n as usize * 3, "packed ints should be tight, was {} bytes", bytes.len());
        match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => assert!(matches!(&*l.borrow(), ListRepr::Ints(_)), "decodes to a packed Ints buffer"),
            other => panic!("expected a list, got {other:?}"),
        }
    }

    #[test]
    fn wire_fixed_width_int_mode_is_memcpy_and_interops() {
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..1000).collect()))));
        let varint = message_to_wire("", &v).unwrap();
        let fixed = with_fixed_numerics(|| message_to_wire("", &v).unwrap());
        // Fixed-width is the raw i64 bytes — bigger than varint (no compression),
        // but a memcpy to load (8 bytes/int + small framing).
        assert!(fixed.len() > varint.len(), "fixed-width should be larger than varint");
        assert!(fixed.len() >= 1000 * 8, "fixed is 8 bytes/int, was {}", fixed.len());
        // Both encodings interoperate — the decoder handles either tag.
        let vals = |bytes: &[u8]| match message_from_wire(bytes).unwrap().1 {
            RuntimeValue::List(l) => l.borrow().to_values(),
            other => panic!("expected a list, got {other:?}"),
        };
        let expected: Vec<RuntimeValue> = (0..1000).map(RuntimeValue::Int).collect();
        assert_eq!(vals(&varint), expected);
        assert_eq!(vals(&fixed), expected);
        // Scoped: the mode does not leak past `with_fixed_numerics`.
        assert_eq!(message_to_wire("", &v).unwrap(), varint);
    }

    #[test]
    fn gv_simd_decode_matches_scalar_oracle_over_fuzz() {
        // The SSSE3 fast path must be bit-identical to the scalar `gv_decode`
        // oracle. We sweep magnitudes so every 2-bit width code {1,2,4,8 bytes}
        // and thus every adjacent `(code_a, code_b)` shuffle-mask is exercised,
        // and lengths so the even SIMD body, the odd tail, and the near-buffer-end
        // tail all fire. `gv_decode_dispatch` takes the SIMD path on this host.
        for seed in [1u64, 2, 7, 42, 99, 1000, 0xDEAD_BEEF, 0x00AB_CDEF] {
            let mut rng = SplitMix64 { state: seed };
            for _ in 0..200 {
                let n = (rng.below(37)) as usize; // 0, odd, and >16-int blocks
                let vals: Vec<i64> = (0..n)
                    .map(|_| {
                        let bits = rng.below(64) as u32; // span all four width buckets
                        let mask = (1u128 << bits).wrapping_sub(1) as u64;
                        let mag = (rng.next() & mask) as i64;
                        if rng.next() & 1 == 0 { mag } else { -mag }
                    })
                    .collect();
                let mut buf = Vec::new();
                gv_encode(&mut buf, vals.iter().copied(), vals.len());

                let (mut p1, mut p2) = (0usize, 0usize);
                let scalar = gv_decode(&buf, &mut p1).expect("scalar decode");
                let simd = gv_decode_dispatch(&buf, &mut p2).expect("dispatch decode");

                assert_eq!(scalar, vals, "scalar oracle lost data (seed {seed}, n {n})");
                assert_eq!(simd, vals, "simd/dispatch lost data (seed {seed}, n {n})");
                assert_eq!(p1, p2, "decoders consumed a different byte count (seed {seed})");
            }
        }
    }

    #[test]
    fn wire_group_varint_mode_interops_with_varint() {
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(
            (0..1000).map(|i| i * i - 500_000).collect(),
        ))));
        let varint = message_to_wire("", &v).unwrap();
        let gv = with_numerics(WireNumerics::GroupVarint, || message_to_wire("", &v).unwrap());
        // The numeric tag is self-describing — either encoding decodes to the same
        // values, regardless of which strategy produced it.
        let vals = |bytes: &[u8]| match message_from_wire(bytes).unwrap().1 {
            RuntimeValue::List(l) => l.borrow().to_values(),
            other => panic!("expected a list, got {other:?}"),
        };
        assert_eq!(vals(&varint), vals(&gv));
        let expected: Vec<RuntimeValue> = (0..1000).map(|i| RuntimeValue::Int(i * i - 500_000)).collect();
        assert_eq!(vals(&gv), expected);
        // Scoped: the mode does not leak past `with_numerics`.
        assert_eq!(message_to_wire("", &v).unwrap(), varint);
    }

    #[test]
    fn wire_packed_float_array_roundtrips_bit_exact() {
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(vec![
            0.0, -0.0, 1.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MIN, f64::MAX,
        ]))));
        assert_wire_stable(&v); // byte-stability ⇒ bit-exact, NaN included
        match message_from_wire(&message_to_wire("", &v).unwrap()).unwrap().1 {
            RuntimeValue::List(l) => assert!(matches!(&*l.borrow(), ListRepr::Floats(_))),
            other => panic!("expected a list, got {other:?}"),
        }
    }

    fn floats_list(vals: Vec<f64>) -> RuntimeValue {
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(vals))))
    }
    fn float_bits(v: &RuntimeValue) -> Vec<u64> {
        match v {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Floats(f) => f.iter().map(|x| x.to_bits()).collect(),
                other => panic!("not a float list: {other:?}"),
            },
            other => panic!("not a list: {other:?}"),
        }
    }

    #[test]
    fn wire_floats_xor_is_bit_exact_including_special_values() {
        // The XOR-delta float codec operates on raw bits, so it is LOSSLESS and
        // bit-exact for every f64 — NaN payloads, ±Inf, ±0.0, subnormals included.
        let vals = vec![1.0, 1.0000001, 1.0000002, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, 0.0, f64::MIN, f64::MAX, f64::MIN_POSITIVE];
        let v = floats_list(vals.clone());
        let bytes = with_floats(WireFloats::XorDelta, || message_to_wire("p", &v).unwrap());
        let back = message_from_wire(&bytes).unwrap().1;
        let orig: Vec<u64> = vals.iter().map(|x| x.to_bits()).collect();
        assert_eq!(float_bits(&back), orig, "XOR-delta float column is bit-exact");
    }

    #[test]
    fn wire_floats_xor_shrinks_slow_varying() {
        // Consecutive samples share their high bits, so XOR-delta + varint is far
        // smaller than 8 bytes/elem memcpy — the time-series win.
        let vals: Vec<f64> = (0..1000).map(|i| 100.0 + i as f64 * 1e-6).collect();
        let v = floats_list(vals);
        let memcpy = message_to_wire("p", &v).unwrap();
        let xor = with_floats(WireFloats::XorDelta, || message_to_wire("p", &v).unwrap());
        assert!(xor.len() < memcpy.len(), "XOR-delta shrinks a slow-varying column: {} vs {}", xor.len(), memcpy.len());
        assert_eq!(float_bits(&message_from_wire(&xor).unwrap().1), float_bits(&message_from_wire(&memcpy).unwrap().1));
    }

    #[test]
    fn wire_floats_xor_never_grows() {
        // A high-entropy column: XOR-delta would be larger, so the encoder falls back
        // to memcpy — never bigger than the baseline.
        let mut rng = SplitMix64 { state: 12345 };
        let vals: Vec<f64> = (0..1000).map(|_| f64::from_bits(rng.next())).collect();
        let v = floats_list(vals);
        let memcpy = message_to_wire("p", &v).unwrap();
        let xor = with_floats(WireFloats::XorDelta, || message_to_wire("p", &v).unwrap());
        assert!(xor.len() <= memcpy.len(), "XOR-delta must never grow vs memcpy: {} vs {}", xor.len(), memcpy.len());
        assert_eq!(float_bits(&message_from_wire(&xor).unwrap().1), float_bits(&message_from_wire(&memcpy).unwrap().1));
    }

    #[test]
    fn wire_floats_xor_fuzz_bit_identical() {
        // Random columns mixing slow-varying runs and arbitrary bit patterns: the
        // XOR-delta decode is bit-identical to the original, always.
        for seed in [1u64, 7, 42, 1000, 0xBEEF, 0xC0FFEE] {
            let mut rng = SplitMix64 { state: seed };
            for _ in 0..200 {
                let n = rng.below(30) as usize;
                let base = f64::from_bits(rng.next());
                let vals: Vec<f64> = (0..n)
                    .map(|i| if rng.below(2) == 0 { base + i as f64 * 1e-9 } else { f64::from_bits(rng.next()) })
                    .collect();
                let v = floats_list(vals.clone());
                let xor = with_floats(WireFloats::XorDelta, || message_to_wire("p", &v).unwrap());
                let orig: Vec<u64> = vals.iter().map(|x| x.to_bits()).collect();
                assert_eq!(float_bits(&message_from_wire(&xor).unwrap().1), orig, "seed {seed}: XOR-delta diverged");
            }
        }
    }

    #[test]
    fn wire_packed_bool_array_is_bit_packed() {
        let n = 1000usize;
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools((0..n).map(|i| i % 3 == 0).collect()))));
        assert_wire_stable(&v);
        let bytes = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
        // 8 booleans per byte → ~125 bytes for 1000, not ~1000.
        assert!(bytes.len() < n / 4, "bools must be bit-packed: {} bytes for {} bools", bytes.len(), n);
        match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => assert!(matches!(&*l.borrow(), ListRepr::Bools(_))),
            other => panic!("expected a list, got {other:?}"),
        }
    }

    #[test]
    fn wire_mixed_list_stays_generic() {
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Text(Rc::new("x".to_string())),
            RuntimeValue::Bool(true),
        ]))));
        assert_wire_stable(&v);
        match message_from_wire(&message_to_wire("", &v).unwrap()).unwrap().1 {
            RuntimeValue::List(l) => assert!(matches!(&*l.borrow(), ListRepr::Boxed(_)), "a mixed list stays Boxed"),
            other => panic!("expected a list, got {other:?}"),
        }
    }

    #[test]
    fn wire_string_array_packs_flat_and_loads_flat() {
        let strs = ["alpha", "", "héllo", "日本語", "emoji😀", "z"];
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
            strs.iter().map(|s| RuntimeValue::Text(Rc::new(s.to_string()))).collect(),
        ))));
        // Byte-stable: Boxed-of-Text → T_STRINGS → Strings → T_STRINGS (same bytes).
        assert_wire_stable(&v);
        let bytes = message_to_wire_with("", &v, WireCodec::Native, WireIntegrity::Raw).unwrap();
        match message_from_wire(&bytes).unwrap().1 {
            RuntimeValue::List(l) => {
                let b = l.borrow();
                assert!(matches!(&*b, ListRepr::Strings { .. }), "a string array loads FLAT, not per-element");
                assert_eq!(b.len(), strs.len());
                let got: Vec<String> = b
                    .to_values()
                    .into_iter()
                    .map(|x| match x {
                        RuntimeValue::Text(s) => (*s).clone(),
                        other => panic!("expected text, got {other:?}"),
                    })
                    .collect();
                assert_eq!(got, strs);
                // Indexed access (get) also materializes the right element.
                assert!(matches!(b.get(2), Some(RuntimeValue::Text(s)) if s.as_str() == "héllo"));
            }
            other => panic!("expected a list, got {other:?}"),
        }
    }

    #[test]
    fn wire_flat_strings_memoize_repeated_get() {
        // Best-of-both: load flat, but a *repeated* get of the same element returns
        // the cached `Rc` (a refcount bump) rather than re-materializing the String.
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
            RuntimeValue::Text(Rc::new("a".to_string())),
            RuntimeValue::Text(Rc::new("bb".to_string())),
            RuntimeValue::Text(Rc::new("ccc".to_string())),
        ]))));
        let back = message_from_wire(&message_to_wire("", &v).unwrap()).unwrap().1;
        let RuntimeValue::List(l) = back else { panic!("expected a list") };
        let b = l.borrow();
        let (RuntimeValue::Text(first), RuntimeValue::Text(again)) = (b.get(1).unwrap(), b.get(1).unwrap())
        else {
            panic!("expected text")
        };
        assert_eq!(first.as_str(), "bb");
        assert!(Rc::ptr_eq(&first, &again), "a repeated get must reuse the memoized Rc, not re-allocate");
    }

    #[test]
    fn wire_mixed_with_one_nonstring_stays_generic() {
        // All-string ⇒ flat; one non-string ⇒ falls back to the tagged list.
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(vec![
            RuntimeValue::Text(Rc::new("a".to_string())),
            RuntimeValue::Int(1),
            RuntimeValue::Text(Rc::new("c".to_string())),
        ]))));
        assert_wire_stable(&v);
        match message_from_wire(&message_to_wire("", &v).unwrap()).unwrap().1 {
            RuntimeValue::List(l) => assert!(matches!(&*l.borrow(), ListRepr::Boxed(_)), "mixed stays Boxed"),
            other => panic!("expected a list, got {other:?}"),
        }
    }

    #[test]
    fn wire_i32_packed_list_roundtrips_through_ints() {
        // A half-width IntsI32 buffer encodes through the same packed path; it
        // rebuilds as full-width Ints (same values, same bytes — byte-stable).
        let v = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::IntsI32(vec![1, -2, 3, -4, 100]))));
        assert_wire_stable(&v);
        let (_, back) = message_from_wire(&message_to_wire("", &v).unwrap()).unwrap();
        match back {
            RuntimeValue::List(l) => match &*l.borrow() {
                ListRepr::Ints(got) => assert_eq!(got, &vec![1i64, -2, 3, -4, 100]),
                other => panic!("expected Ints, got {other:?}"),
            },
            other => panic!("expected a list, got {other:?}"),
        }
    }

    /// A running (non-ignored) report: for every payload × codec it prints the wire
    /// size + ratios and a light throughput sample, AND asserts the deterministic
    /// invariants (round-trip, native<json, zstd≤deflate, lz4 shrinks, columnar
    /// structs are compact). Run with `--nocapture` to read the numbers; the asserts
    /// hold either way. zstd is native-only, so this is a native-target test.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wire_codec_report() {
        use std::time::Instant;

        fn enc(v: &RuntimeValue, comp: WireCompression, num: WireNumerics) -> Vec<u8> {
            with_numerics(num, || with_compression_codec(comp, || message_to_wire("p", v).unwrap()))
        }

        let ints = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..1000).collect()))));
        let floats =
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats((0..1000).map(|i| i as f64 * 1.5).collect()))));
        let bools = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools((0..1000).map(|i| i % 3 == 0).collect()))));
        let strings = redundant_list(200);
        let records = struct_list(1000);
        let payloads: [(&str, &RuntimeValue); 5] = [
            ("1000 ints", &ints),
            ("1000 floats", &floats),
            ("1000 bools", &bools),
            ("200 redundant strings", &strings),
            ("1000 structs (Point)", &records),
        ];

        println!("\n=== WIRE CODEC REPORT ===");
        for (name, v) in payloads {
            let raw = enc(v, WireCompression::None, WireNumerics::Varint);
            let json = message_to_wire_with("p", v, WireCodec::Json, WireIntegrity::Raw).unwrap();
            println!(
                "\n{name}: native {} B | json {} B  ({:.1}× tighter than json)",
                raw.len(),
                json.len(),
                json.len() as f64 / raw.len() as f64
            );
            for comp in [WireCompression::None, WireCompression::Deflate, WireCompression::Lz4, WireCompression::Zstd] {
                let b = enc(v, comp, WireNumerics::Varint);
                assert!(message_from_wire(&b).is_some(), "{name}/{comp:?} must round-trip");
                let cn = format!("{comp:?}");
                println!("  {cn:<8} {:>7} B  ({:.2}× of native raw)", b.len(), b.len() as f64 / raw.len() as f64);
            }
            assert!(raw.len() < json.len(), "{name}: native must beat json");
        }

        // Compression invariants on a hard-to-compress-no-more redundant payload.
        let red = redundant_list(1000);
        let raw = enc(&red, WireCompression::None, WireNumerics::Varint);
        let deflate = enc(&red, WireCompression::Deflate, WireNumerics::Varint);
        let lz4 = enc(&red, WireCompression::Lz4, WireNumerics::Varint);
        let zstd = enc(&red, WireCompression::Zstd, WireNumerics::Varint);
        assert!(lz4.len() < raw.len(), "lz4 shrinks redundant data ({} vs {})", lz4.len(), raw.len());
        assert!(deflate.len() < raw.len(), "deflate shrinks redundant data");
        assert!(zstd.len() <= deflate.len(), "zstd ratio ≤ deflate ({} vs {})", zstd.len(), deflate.len());

        // Columnar structs are far smaller than the old per-row-boxed form.
        assert!(records_is_compact(&records), "columnar struct list must be compact");

        // Light throughput sample (encode+decode), so the numbers are visible.
        for (name, v) in [("1000 ints", &ints), ("1000 structs", &records)] {
            let it = 2000u32;
            let t = Instant::now();
            let mut total = 0usize;
            for _ in 0..it {
                let b = message_to_wire("p", v).unwrap();
                total += message_from_wire(&b).map(|_| b.len()).unwrap();
            }
            let el = t.elapsed();
            println!(
                "  throughput {name:<13}: {:>9.0} msg/s  {:>7.1} MB/s",
                it as f64 / el.as_secs_f64(),
                total as f64 / el.as_secs_f64() / 1e6
            );
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn records_is_compact(records: &RuntimeValue) -> bool {
        let b = message_to_wire("p", records).unwrap();
        b.len() < 6000
    }

    #[test]
    #[ignore = "throughput benchmark — run with: cargo test -p logicaffeine-compile marshal::tests::bench_wire_throughput --release -- --ignored --nocapture"]
    fn bench_wire_throughput() {
        use bincode::Options;
        use std::time::Instant;

        fn bench<F: FnMut() -> usize>(label: &str, iters: u32, mut f: F) {
            for _ in 0..(iters / 10).max(1) {
                f();
            } // warm up
            let t = Instant::now();
            let mut bytes = 0usize;
            for _ in 0..iters {
                bytes += f();
            }
            let el = t.elapsed();
            println!(
                "  {label:<26} {:>11.0} msg/s  {:>9.1} MB/s  ({:>8.0?}/op)",
                iters as f64 / el.as_secs_f64(),
                bytes as f64 / el.as_secs_f64() / 1e6,
                el / iters
            );
        }

        // bincode-of-WireValue — an off-the-shelf binary format, for comparison.
        let bincode_enc = |v: &RuntimeValue| {
            let p = materialize(v).unwrap();
            let msg = rt_to_wire(&p).unwrap();
            wire_options().serialize(&WireMessage { from: "p".to_string(), msg }).unwrap()
        };

        // Payloads across the type space.
        let mk_record = |i: i64| {
            let mut f = HashMap::new();
            f.insert("id".to_string(), RuntimeValue::Int(i));
            f.insert("name".to_string(), RuntimeValue::Text(Rc::new(format!("item-{i}"))));
            f.insert("active".to_string(), RuntimeValue::Bool(i % 2 == 0));
            RuntimeValue::Struct(Box::new(StructValue { type_name: "Record".to_string(), fields: f }))
        };
        let ints = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints((0..1000).collect()))));
        let floats =
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats((0..1000).map(|i| i as f64 * 1.5).collect()))));
        let bools = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Bools((0..1000).map(|i| i % 3 == 0).collect()))));
        let strings = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed(
            (0..200).map(|i| RuntimeValue::Text(Rc::new(format!("string-value-{i}")))).collect(),
        ))));
        let records =
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Boxed((0..200).map(mk_record).collect()))));

        let payloads: [(&str, &RuntimeValue); 5] = [
            ("1000 ints", &ints),
            ("1000 floats", &floats),
            ("1000 bools", &bools),
            ("200 strings", &strings),
            ("200 records", &records),
        ];

        for (name, v) in payloads {
            let nat = message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap();
            let json = message_to_wire_with("p", v, WireCodec::Json, WireIntegrity::Raw).unwrap();
            let binc = bincode_enc(v);
            println!(
                "\n=== {name}: native {} B | bincode {} B | json {} B  ({:.1}× tighter than json) ===",
                nat.len(),
                binc.len(),
                json.len(),
                json.len() as f64 / nat.len() as f64
            );
            let it = 100_000;
            bench("native encode (raw)", it, || {
                message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap().len()
            });
            bench("native encode (checked)", it, || {
                message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Checked).unwrap().len()
            });
            bench("bincode encode", it, || bincode_enc(v).len());
            bench("json encode", it, || {
                message_to_wire_with("p", v, WireCodec::Json, WireIntegrity::Raw).unwrap().len()
            });
            // Fixed-width numeric mode (memcpy) — only differs for int arrays.
            let nat_fixed = with_fixed_numerics(|| {
                message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            if nat_fixed != nat {
                println!("  (fixed-width numerics: {} B vs varint {} B)", nat_fixed.len(), nat.len());
                bench("native encode (fixed)", it, || {
                    with_fixed_numerics(|| message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap().len())
                });
                bench("native decode (fixed)", it, || {
                    message_from_wire(&nat_fixed).unwrap();
                    nat_fixed.len()
                });
            }
            // Group-varint numeric mode (SSSE3 shuffle decode) — also int-only. The
            // decode line is the headline: does SIMD beat LEB128 at varint size?
            let nat_gv = with_numerics(WireNumerics::GroupVarint, || {
                message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap()
            });
            if nat_gv != nat {
                println!("  (group-varint numerics: {} B vs varint {} B)", nat_gv.len(), nat.len());
                bench("native encode (gv)", it, || {
                    with_numerics(WireNumerics::GroupVarint, || message_to_wire_with("p", v, WireCodec::Native, WireIntegrity::Raw).unwrap().len())
                });
                bench("native decode (gv/simd)", it, || {
                    message_from_wire(&nat_gv).unwrap();
                    nat_gv.len()
                });
            }
            bench("native decode (raw)", it, || {
                message_from_wire(&nat).unwrap();
                nat.len()
            });
            bench("bincode decode", it, || {
                let _: WireMessage = wire_options().deserialize(&binc).unwrap();
                binc.len()
            });
            bench("json decode", it, || {
                message_from_wire(&json).unwrap();
                json.len()
            });
        }
    }

    // --- A seeded generator of arbitrary (network-portable) values ----------

    struct SplitMix64 {
        state: u64,
    }
    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn gen_char(rng: &mut SplitMix64) -> char {
        loop {
            if let Some(c) = char::from_u32((rng.next() % 0x11_0000) as u32) {
                return c;
            }
        }
    }

    fn gen_string(rng: &mut SplitMix64) -> String {
        (0..rng.below(6)).map(|_| gen_char(rng)).collect()
    }

    fn gen_value(rng: &mut SplitMix64, depth: u32) -> RuntimeValue {
        // At depth 0 only scalars (indices 0..12); deeper, containers too.
        let kinds = if depth == 0 { 12 } else { 18 };
        match rng.below(kinds) {
            0 => RuntimeValue::Int(rng.next() as i64),
            1 => RuntimeValue::Float(f64::from_bits(rng.next())),
            2 => RuntimeValue::Bool(rng.next() & 1 == 0),
            3 => RuntimeValue::Char(gen_char(rng)),
            4 => RuntimeValue::Text(Rc::new(gen_string(rng))),
            5 => RuntimeValue::Nothing,
            6 => RuntimeValue::Duration(rng.next() as i64),
            7 => RuntimeValue::Date(rng.next() as i32),
            8 => RuntimeValue::Moment(rng.next() as i64),
            9 => RuntimeValue::Span { months: rng.next() as i32, days: rng.next() as i32 },
            10 => RuntimeValue::Time(rng.next() as i64),
            11 => RuntimeValue::Peer(Rc::new(gen_string(rng))),
            12 => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                (0..rng.below(4)).map(|_| gen_value(rng, depth - 1)).collect(),
            )))),
            13 => RuntimeValue::Tuple(Rc::new((0..rng.below(4)).map(|_| gen_value(rng, depth - 1)).collect())),
            14 => RuntimeValue::Set(Rc::new(RefCell::new(
                (0..rng.below(4)).map(|_| gen_value(rng, depth - 1)).collect(),
            ))),
            15 => {
                let mut m = MapStorage::default();
                for _ in 0..rng.below(4) {
                    // Keys are scalars (depth 0) so they stay hashable + simple.
                    m.insert(gen_value(rng, 0), gen_value(rng, depth - 1));
                }
                RuntimeValue::Map(Rc::new(RefCell::new(m)))
            }
            16 => {
                let mut fields = HashMap::new();
                for i in 0..rng.below(4) {
                    fields.insert(format!("f{i}"), gen_value(rng, depth - 1));
                }
                RuntimeValue::Struct(Box::new(StructValue { type_name: format!("T{}", rng.below(5)), fields }))
            }
            _ => RuntimeValue::Inductive(Box::new(InductiveValue {
                inductive_type: format!("I{}", rng.below(5)),
                constructor: format!("C{}", rng.below(5)),
                args: (0..rng.below(4)).map(|_| gen_value(rng, depth - 1)).collect(),
            })),
        }
    }
}
