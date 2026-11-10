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
use logicaffeine_base::describe::{
    self, consider, deserialize_gen, detect_affine, emit_best_int_column, serialize_gen,
    MAX_GEN_DEPTH, MAX_GEN_NODES,
};
pub use logicaffeine_base::describe::{gen_eval, GenCmp, GenExpr};

use crate::interpreter::{
    ClosureValue, InductiveValue, ListRepr, MapStorage, RuntimeValue, StructValue,
};
use logicaffeine_data::wire::{canonical_f64, canonical_f64_bits, put_f64, put_f64s};

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
            RtPayload::BigInt {
                negative,
                magnitude,
            }
        }
        RuntimeValue::Rational(r) => {
            let (num_negative, num_magnitude) = r.numerator().to_le_bytes();
            let (_den_sign, den_magnitude) = r.denominator().to_le_bytes();
            RtPayload::Rational {
                num_negative,
                num_magnitude,
                den_magnitude,
            }
        }
        RuntimeValue::Decimal(d) => {
            let (negative, magnitude, scale) = d.to_le_bytes();
            RtPayload::Decimal {
                negative,
                magnitude,
                scale,
            }
        }
        RuntimeValue::Money(m) => {
            let (negative, magnitude, scale) = m.amount.to_le_bytes();
            RtPayload::Money {
                negative,
                magnitude,
                scale,
                currency: m.currency.code.to_string(),
            }
        }
        RuntimeValue::Uuid(u) => RtPayload::Uuid(u.to_bytes()),
        RuntimeValue::Complex(c) => {
            let (re_negative, re_num) = c.re().numerator().to_le_bytes();
            let (_, re_den) = c.re().denominator().to_le_bytes();
            let (im_negative, im_num) = c.im().numerator().to_le_bytes();
            let (_, im_den) = c.im().denominator().to_le_bytes();
            RtPayload::Complex {
                re_negative,
                re_num,
                re_den,
                im_negative,
                im_num,
                im_den,
            }
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
        RuntimeValue::Span { months, days } => RtPayload::Span {
            months: *months,
            days: *days,
        },
        RuntimeValue::Time(n) => RtPayload::Time(*n),
        RuntimeValue::Word(w) => RtPayload::Word {
            width: w.width(),
            bits: w.to_u64(),
        },
        // A SIMD lane vector is a transient compute value (a register), not a wire type — like a
        // closure, it does not cross a task boundary. (A future increment can serialize its lanes.)
        RuntimeValue::Lanes(_) => return Err(MarshalError::NotSendable("Lanes8Word32")),
        RuntimeValue::ComplexF(c) => RtPayload::ComplexF {
            re: c.re(),
            im: c.im(),
        },
        RuntimeValue::List(items) => {
            let vals = items.borrow().to_values();
            RtPayload::List(vals.iter().map(materialize).collect::<Result<_, _>>()?)
        }
        RuntimeValue::Set(items) => RtPayload::Set(
            items
                .borrow()
                .iter()
                .map(materialize)
                .collect::<Result<_, _>>()?,
        ),
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
            RtPayload::Struct {
                type_name: s.type_name.clone(),
                fields,
            }
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
        RuntimeValue::Algebraic(_) | RuntimeValue::ExactReal(_) | RuntimeValue::Ordinal(_) => {
            let Some((kind, bytes)) = crate::semantics::exact::exact_bytes(value) else {
                return Err(MarshalError::NotSendable(logicaffeine_base::exact_real::STAYS_WITH_ITS_PROGRAM));
            };
            RtPayload::Exact { kind, bytes }
        }
        RuntimeValue::Hyperreal(_) | RuntimeValue::Hypernatural(_) => {
            let Some((kind, name, residues, bytes)) = crate::semantics::exact::scaled_bytes(value) else {
                return Err(MarshalError::NotSendable(logicaffeine_base::exact_real::STAYS_WITH_ITS_PROGRAM));
            };
            RtPayload::Scaled {
                kind,
                name,
                residues,
                bytes,
            }
        }
        RuntimeValue::Depends(d) => {
            let (name, residues) = crate::semantics::exact::scale_parts(&d.scale);
            let branches = d
                .value
                .raw_branches()
                .into_iter()
                .map(|b| b.map(|b| materialize(&b.0)).transpose())
                .collect::<Result<Vec<_>, _>>()?;
            RtPayload::Depends {
                name,
                residues,
                modulus: d.value.modulus(),
                branches,
            }
        }
        RuntimeValue::Converging(c) => RtPayload::Converging {
            value: Box::new(materialize(c.value())?),
            steps: c.steps(),
            revisions: c.revisions(),
            certificate: c.certificate().map(str::to_string),
        },
        // A view is a handle into ONE task's DOM: sending it would alias another task's page.
        RuntimeValue::View(_) => return Err(MarshalError::NotSendable("View")),
        RuntimeValue::Action(_) => return Err(MarshalError::NotSendable("Action")),
        // A live CRDT crosses the boundary as its KIND-TAGGED full convergent STATE — NOT its `Rc`
        // handle (that would alias the same replica cell). The far side rebuilds a fresh replica and
        // merges, so a redelivered payload is idempotent (per-replica MAX, never double-counts). This
        // is what makes `Sync <crdt>` marshal correctly across the resumable-VM boundary.
        RuntimeValue::Crdt(c) => RtPayload::Crdt(c.borrow().to_full_state_bytes()),
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
        RtPayload::BigInt {
            negative,
            magnitude,
        } => RuntimeValue::from_bigint(logicaffeine_base::BigInt::from_le_bytes(
            negative, &magnitude,
        )),
        RtPayload::Rational {
            num_negative,
            num_magnitude,
            den_magnitude,
        } => {
            let num = logicaffeine_base::BigInt::from_le_bytes(num_negative, &num_magnitude);
            let den = logicaffeine_base::BigInt::from_le_bytes(false, &den_magnitude);
            // A well-formed payload always has a nonzero denominator; fall back to the
            // numerator (treated as a whole number) if a corrupt one slips through.
            match logicaffeine_base::Rational::new(num.clone(), den) {
                Some(r) => RuntimeValue::from_rational(r),
                None => RuntimeValue::from_bigint(num),
            }
        }
        RtPayload::Decimal {
            negative,
            magnitude,
            scale,
        } => RuntimeValue::Decimal(Rc::new(logicaffeine_base::Decimal::from_le_bytes(
            negative, &magnitude, scale,
        ))),
        RtPayload::Money {
            negative,
            magnitude,
            scale,
            currency,
        } => {
            let amount = logicaffeine_base::Decimal::from_le_bytes(negative, &magnitude, scale);
            let currency = logicaffeine_base::money::currency::by_code(&currency).unwrap_or(
                logicaffeine_base::Currency {
                    code: "XXX",
                    scale: 0,
                },
            );
            RuntimeValue::Money(Rc::new(logicaffeine_base::Money { amount, currency }))
        }
        RtPayload::Uuid(bytes) => {
            RuntimeValue::Uuid(Rc::new(logicaffeine_base::Uuid::from_bytes(bytes)))
        }
        RtPayload::Complex {
            re_negative,
            re_num,
            re_den,
            im_negative,
            im_num,
            im_den,
        } => {
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
        RtPayload::ComplexF { re, im } => {
            RuntimeValue::ComplexF(Rc::new(logicaffeine_base::ComplexF::new(re, im)))
        }
        RtPayload::Modular { value, modulus } => {
            let v = logicaffeine_base::BigInt::from_le_bytes(false, &value);
            let n = logicaffeine_base::BigInt::from_le_bytes(false, &modulus);
            match logicaffeine_base::Modular::new(v, n) {
                Some(m) => RuntimeValue::Modular(Rc::new(m)),
                None => RuntimeValue::Nothing, // a corrupt (non-positive) modulus degrades gracefully
            }
        }
        RtPayload::Quantity {
            num_negative,
            num_magnitude,
            den_magnitude,
            dim_num,
            dim_den,
            unit_symbol,
        } => {
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
        RtPayload::Word { width, bits } => {
            match logicaffeine_base::WordVal::from_u64(width, bits) {
                Some(w) => RuntimeValue::Word(w),
                // A well-formed payload always carries width 32/64; degrade a corrupt one to its value.
                None => RuntimeValue::Int(bits as i64),
            }
        }
        RtPayload::List(items) => {
            let vals: Vec<RuntimeValue> = items.into_iter().map(rebuild).collect();
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(vals))))
        }
        RtPayload::Set(items) => RuntimeValue::Set(Rc::new(RefCell::new(
            items.into_iter().map(rebuild).collect(),
        ))),
        RtPayload::Tuple(items) => {
            RuntimeValue::Tuple(Rc::new(items.into_iter().map(rebuild).collect()))
        }
        RtPayload::Map(pairs) => {
            let m: MapStorage = pairs
                .into_iter()
                .map(|(k, v)| (rebuild(k), rebuild(v)))
                .collect();
            RuntimeValue::Map(Rc::new(RefCell::new(m)))
        }
        RtPayload::Struct { type_name, fields } => {
            let f = fields.into_iter().map(|(k, v)| (k, rebuild(v))).collect();
            RuntimeValue::Struct(Box::new(StructValue {
                type_name,
                fields: f,
            }))
        }
        RtPayload::Inductive {
            type_name,
            constructor,
            args,
        } => RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
            inductive_type: type_name,
            constructor,
            args: args.into_iter().map(rebuild).collect(),
        })),
        RtPayload::Chan(id) => RuntimeValue::Chan(id),
        RtPayload::TaskHandle(id) => RuntimeValue::TaskHandle(id),
        RtPayload::Peer(topic) => RuntimeValue::Peer(Rc::new(topic)),
        // Rebuild a fresh CRDT replica from the shipped full state (see the `materialize` arm): the
        // far side owns an independent `Rc` cell whose convergent value equals the sender's.
        RtPayload::Exact { kind, bytes } => crate::semantics::exact::from_exact_bytes(kind, &bytes)
            .expect("a materialized exact value rebuilds"),
        RtPayload::Scaled {
            kind,
            name,
            residues,
            bytes,
        } => crate::semantics::exact::from_scaled_bytes(kind, &name, &residues, &bytes)
            .expect("a materialized scaled value rebuilds"),
        RtPayload::Depends {
            name,
            residues,
            modulus,
            branches,
        } => crate::semantics::exact::depends_from_parts(
            &name,
            &residues,
            modulus,
            branches.into_iter().map(|b| b.map(rebuild)).collect(),
        )
        .expect("a materialized dependence rebuilds"),
        RtPayload::Converging {
            value,
            steps,
            revisions,
            certificate,
        } => RuntimeValue::Converging(Rc::new(match certificate {
            Some(cert) => logicaffeine_base::limit_value::Converging::settled(rebuild(*value), &cert),
            None => logicaffeine_base::limit_value::Converging::provisional(
                rebuild(*value),
                steps,
                revisions,
            ),
        })),
        RtPayload::Crdt(bytes) => RuntimeValue::Crdt(Rc::new(RefCell::new(
            crate::semantics::crdt::CrdtValue::from_full_state_bytes(&bytes),
        ))),
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
    BigInt {
        negative: bool,
        magnitude: Vec<u8>,
    },
    Rational {
        num_negative: bool,
        num_magnitude: Vec<u8>,
        den_magnitude: Vec<u8>,
    },
    Decimal {
        negative: bool,
        magnitude: Vec<u8>,
        scale: u32,
    },
    Money {
        negative: bool,
        magnitude: Vec<u8>,
        scale: u32,
        currency: std::string::String,
    },
    Uuid([u8; 16]),
    Complex {
        re_negative: bool,
        re_num: Vec<u8>,
        re_den: Vec<u8>,
        im_negative: bool,
        im_num: Vec<u8>,
        im_den: Vec<u8>,
    },
    ComplexF {
        re: f64,
        im: f64,
    },
    Modular {
        value: Vec<u8>,
        modulus: Vec<u8>,
    },
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
    Span {
        months: i32,
        days: i32,
    },
    Time(i64),
    Word {
        width: u32,
        bits: u64,
    },
    Peer(String),
    List(Vec<WireValue>),
    Tuple(Vec<WireValue>),
    Set(Vec<WireValue>),
    Map(Vec<(WireValue, WireValue)>),
    Struct {
        type_name: String,
        fields: Vec<(String, WireValue)>,
    },
    Inductive {
        type_name: String,
        constructor: String,
        args: Vec<WireValue>,
    },
    Exact {
        kind: u8,
        bytes: Vec<u8>,
    },
    Scaled {
        kind: u8,
        name: String,
        residues: Vec<u8>,
        bytes: Vec<u8>,
    },
    Depends {
        name: String,
        residues: Vec<u8>,
        modulus: u64,
        branches: Vec<Option<WireValue>>,
    },
    Converging {
        value: Box<WireValue>,
        steps: u64,
        revisions: u64,
        certificate: Option<String>,
    },
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
        RtPayload::BigInt {
            negative,
            magnitude,
        } => WireValue::BigInt {
            negative: *negative,
            magnitude: magnitude.clone(),
        },
        RtPayload::Rational {
            num_negative,
            num_magnitude,
            den_magnitude,
        } => WireValue::Rational {
            num_negative: *num_negative,
            num_magnitude: num_magnitude.clone(),
            den_magnitude: den_magnitude.clone(),
        },
        RtPayload::Decimal {
            negative,
            magnitude,
            scale,
        } => WireValue::Decimal {
            negative: *negative,
            magnitude: magnitude.clone(),
            scale: *scale,
        },
        RtPayload::Money {
            negative,
            magnitude,
            scale,
            currency,
        } => WireValue::Money {
            negative: *negative,
            magnitude: magnitude.clone(),
            scale: *scale,
            currency: currency.clone(),
        },
        RtPayload::Uuid(bytes) => WireValue::Uuid(*bytes),
        RtPayload::ComplexF { re, im } => WireValue::ComplexF {
            re: canonical_f64(*re),
            im: canonical_f64(*im),
        },
        RtPayload::Complex {
            re_negative,
            re_num,
            re_den,
            im_negative,
            im_num,
            im_den,
        } => WireValue::Complex {
            re_negative: *re_negative,
            re_num: re_num.clone(),
            re_den: re_den.clone(),
            im_negative: *im_negative,
            im_num: im_num.clone(),
            im_den: im_den.clone(),
        },
        RtPayload::Modular { value, modulus } => WireValue::Modular {
            value: value.clone(),
            modulus: modulus.clone(),
        },
        RtPayload::Quantity {
            num_negative,
            num_magnitude,
            den_magnitude,
            dim_num,
            dim_den,
            unit_symbol,
        } => WireValue::Quantity {
            num_negative: *num_negative,
            num_magnitude: num_magnitude.clone(),
            den_magnitude: den_magnitude.clone(),
            dim_num: dim_num.clone(),
            dim_den: dim_den.clone(),
            unit_symbol: unit_symbol.clone(),
        },
        RtPayload::Float(f) => WireValue::Float(canonical_f64(*f)),
        RtPayload::Bool(b) => WireValue::Bool(*b),
        RtPayload::Char(c) => WireValue::Char(*c),
        RtPayload::Text(s) => WireValue::Text(s.clone()),
        RtPayload::Duration(n) => WireValue::Duration(*n),
        RtPayload::Date(n) => WireValue::Date(*n),
        RtPayload::Moment(n) => WireValue::Moment(*n),
        RtPayload::Span { months, days } => WireValue::Span {
            months: *months,
            days: *days,
        },
        RtPayload::Time(n) => WireValue::Time(*n),
        RtPayload::Word { width, bits } => WireValue::Word {
            width: *width,
            bits: *bits,
        },
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
            WireValue::Struct {
                type_name: type_name.clone(),
                fields: wire_fields,
            }
        }
        RtPayload::Inductive {
            type_name,
            constructor,
            args,
        } => WireValue::Inductive {
            type_name: type_name.clone(),
            constructor: constructor.clone(),
            args: rt_seq_to_wire(args)?,
        },
        // A scheduler token indexes THIS process's scheduler — not portable.
        RtPayload::Chan(_) | RtPayload::TaskHandle(_) => return None,
        // A CRDT does not travel as an INLINE channel/wire message — it converges over the dedicated
        // relay `Sync`/`Merge` protocol (the state-based `crdt_to_wire`/`crdt_merge_wire` delta path).
        // As a channel payload it is a same-process move (rebuilt via `RtPayload::Crdt`), so it has no
        // portable inline `WireValue` form here.
        RtPayload::Crdt(_) => return None,
        RtPayload::Exact { kind, bytes } => WireValue::Exact {
            kind: *kind,
            bytes: bytes.clone(),
        },
        RtPayload::Scaled {
            kind,
            name,
            residues,
            bytes,
        } => WireValue::Scaled {
            kind: *kind,
            name: name.clone(),
            residues: residues.clone(),
            bytes: bytes.clone(),
        },
        RtPayload::Depends {
            name,
            residues,
            modulus,
            branches,
        } => WireValue::Depends {
            name: name.clone(),
            residues: residues.clone(),
            modulus: *modulus,
            branches: branches
                .iter()
                .map(|b| match b {
                    Some(p) => rt_to_wire(p).map(Some),
                    None => Some(None),
                })
                .collect::<Option<Vec<_>>>()?,
        },
        RtPayload::Converging {
            value,
            steps,
            revisions,
            certificate,
        } => WireValue::Converging {
            value: Box::new(rt_to_wire(value)?),
            steps: *steps,
            revisions: *revisions,
            certificate: certificate.clone(),
        },
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
        WireValue::BigInt {
            negative,
            magnitude,
        } => RtPayload::BigInt {
            negative,
            magnitude,
        },
        WireValue::Rational {
            num_negative,
            num_magnitude,
            den_magnitude,
        } => RtPayload::Rational {
            num_negative,
            num_magnitude,
            den_magnitude,
        },
        WireValue::Decimal {
            negative,
            magnitude,
            scale,
        } => RtPayload::Decimal {
            negative,
            magnitude,
            scale,
        },
        WireValue::Money {
            negative,
            magnitude,
            scale,
            currency,
        } => RtPayload::Money {
            negative,
            magnitude,
            scale,
            currency,
        },
        WireValue::Uuid(bytes) => RtPayload::Uuid(bytes),
        WireValue::ComplexF { re, im } => RtPayload::ComplexF { re, im },
        WireValue::Complex {
            re_negative,
            re_num,
            re_den,
            im_negative,
            im_num,
            im_den,
        } => RtPayload::Complex {
            re_negative,
            re_num,
            re_den,
            im_negative,
            im_num,
            im_den,
        },
        WireValue::Modular { value, modulus } => RtPayload::Modular { value, modulus },
        WireValue::Quantity {
            num_negative,
            num_magnitude,
            den_magnitude,
            dim_num,
            dim_den,
            unit_symbol,
        } => RtPayload::Quantity {
            num_negative,
            num_magnitude,
            den_magnitude,
            dim_num,
            dim_den,
            unit_symbol,
        },
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
        WireValue::Map(pairs) => RtPayload::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (wire_to_rt(k), wire_to_rt(v)))
                .collect(),
        ),
        WireValue::Struct { type_name, fields } => RtPayload::Struct {
            type_name,
            fields: fields
                .into_iter()
                .map(|(n, v)| (n, wire_to_rt(v)))
                .collect(),
        },
        WireValue::Inductive {
            type_name,
            constructor,
            args,
        } => RtPayload::Inductive {
            type_name,
            constructor,
            args: args.into_iter().map(wire_to_rt).collect(),
        },
        WireValue::Exact { kind, bytes } => RtPayload::Exact { kind, bytes },
        WireValue::Scaled {
            kind,
            name,
            residues,
            bytes,
        } => RtPayload::Scaled {
            kind,
            name,
            residues,
            bytes,
        },
        WireValue::Depends {
            name,
            residues,
            modulus,
            branches,
        } => RtPayload::Depends {
            name,
            residues,
            modulus,
            branches: branches.into_iter().map(|b| b.map(wire_to_rt)).collect(),
        },
        WireValue::Converging {
            value,
            steps,
            revisions,
            certificate,
        } => RtPayload::Converging {
            value: Box::new(wire_to_rt(*value)),
            steps,
            revisions,
            certificate,
        },
    }
}

/// Whether every exact value inside a payload from a peer decodes canonically, so that
/// [`rebuild`] (which trusts payloads materialized in this process) never meets a forged one.
fn exact_payloads_valid(p: &RtPayload) -> bool {
    use crate::semantics::exact;
    match p {
        RtPayload::Exact { kind, bytes } => exact::from_exact_bytes(*kind, bytes).is_some(),
        RtPayload::Scaled {
            kind,
            name,
            residues,
            bytes,
        } => exact::from_scaled_bytes(*kind, name, residues, bytes).is_some(),
        RtPayload::Depends {
            name,
            residues,
            modulus,
            branches,
        } => {
            branches.iter().flatten().all(exact_payloads_valid)
                && exact::depends_from_parts(
                    name,
                    residues,
                    *modulus,
                    branches
                        .iter()
                        .map(|b| b.as_ref().map(|_| RuntimeValue::Nothing))
                        .collect(),
                )
                .is_some()
        }
        RtPayload::Converging { value, .. } => exact_payloads_valid(value),
        RtPayload::List(items) | RtPayload::Tuple(items) | RtPayload::Set(items) => {
            items.iter().all(exact_payloads_valid)
        }
        RtPayload::Map(pairs) => pairs
            .iter()
            .all(|(k, v)| exact_payloads_valid(k) && exact_payloads_valid(v)),
        RtPayload::Struct { fields, .. } => fields.iter().all(|(_, v)| exact_payloads_valid(v)),
        RtPayload::Inductive { args, .. } => args.iter().all(exact_payloads_valid),
        _ => true,
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
/// integrity (`default_integrity`). Closures, and channel/task handles (local to
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
pub fn message_to_wire_best(
    from: &str,
    value: &RuntimeValue,
    goal: WireGoal,
) -> Result<Vec<u8>, String> {
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
            &[
                WireCompression::None,
                WireCompression::Deflate,
                WireCompression::Lz4,
                WireCompression::Zstd,
                WireCompression::Cm,
            ],
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
    let integrity = current_integrity();
    let mut best: Option<Vec<u8>> = None;
    let mut keep = |bytes: Vec<u8>| {
        if best.as_ref().map_or(true, |b| bytes.len() < b.len()) {
            best = Some(bytes);
        }
    };
    // Structure-sharing dimension: if the SAME subtree is reached more than once, Rc-dedup ships it
    // ONCE + backrefs. It competes as just another bake-off candidate (each is a single dedup-scoped
    // encode, so the per-encode id table never leaks across passes), so it wins ONLY when it actually
    // beats the backref-tag overhead. The gather runs once HERE; no sharing → no dedup pass, no cost.
    let dedup_opts: &[bool] = if value_has_sharing(value) {
        &[false, true]
    } else {
        &[false]
    };
    // Each shape (numerics × structure × float coding × dedup × effort level) is encoded ONCE and every
    // allowed compressor is tried on that one body; dials a value never reaches (numerics on a float
    // column) give identical bodies, which are compressed only once.
    let mut tried: std::collections::HashSet<(usize, u64, bool)> = std::collections::HashSet::new();
    // Bodies for the context-mixing codec, with what zstd made of each: it runs after every other candidate
    // is known (see below).
    let mut cm_candidates: Vec<(usize, Vec<u8>)> = Vec::new();
    for level in [WireCompressionLevel::Balanced, WireCompressionLevel::Max] {
        for num in [
            WireNumerics::Varint,
            WireNumerics::Fixed,
            WireNumerics::GroupVarint,
        ] {
            for st in [
                WireStructure::Off,
                WireStructure::Affine,
                WireStructure::Auto,
            ] {
                // The `Auto` column menu chooses every int column's form itself and never reads the numeric
                // dial, so under `Auto` the other numerics would re-run the costliest encode for an identical
                // body.
                if st == WireStructure::Auto && num != WireNumerics::Varint {
                    continue;
                }
                for fl in [WireFloats::Memcpy, WireFloats::XorDelta] {
                    for &dedup in dedup_opts {
                        let body = with_compression_level(level, || {
                            with_dedup(dedup, || {
                                with_numerics(num, || {
                                    with_structure(st, || with_floats(fl, || native_body(from, value)))
                                })
                            })
                        })?;
                        if !tried.insert((body.len(), fnv1a_64(&body), level == WireCompressionLevel::Max)) {
                            continue;
                        }
                        // The uncompressed body is always a candidate: it is what any compressor that
                        // fails to shrink the body falls back to.
                        keep(frame(WireCodec::Native, integrity, WireCompression::None, body.clone()));
                        let mut zstd_len = None;
                        for &comp in compressions {
                            if comp == WireCompression::Cm {
                                continue;
                            }
                            if let Some((used, z)) =
                                with_compression_level(level, || compress_body(comp, &body))
                            {
                                if used == WireCompression::Zstd {
                                    zstd_len = Some(z.len());
                                }
                                if z.len() < body.len() {
                                    keep(frame(WireCodec::Native, integrity, used, z));
                                }
                            }
                        }
                        if level == WireCompressionLevel::Max && compressions.contains(&WireCompression::Cm) {
                            cm_candidates.push((zstd_len.unwrap_or(body.len()), body));
                        }
                    }
                }
            }
        }
    }
    // Context mixing, the Max-effort compressor, runs last and only on bodies that can still win: it gains a
    // fraction over zstd on the same body, so a body whose zstd form is more than a quarter larger than the
    // best candidate so far cannot become the smallest.
    for (zstd_len, body) in cm_candidates {
        let best_len = best.as_ref().map_or(usize::MAX, Vec::len);
        if zstd_len.saturating_mul(4) > best_len.saturating_mul(5) {
            continue;
        }
        if let Some((used, z)) = compress_body(WireCompression::Cm, &body) {
            let framed = frame(WireCodec::Native, integrity, used, z);
            if framed.len() < best_len {
                best = Some(framed);
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
    let best = if neg.use_type_id {
        with_type_registry(registry, search)
    } else {
        search()
    }?;
    Ok(if best.len() < default.len() {
        best
    } else {
        default
    })
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
    Ok(if best.len() < default.len() {
        best
    } else {
        default
    })
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
/// `MAX_ENCODE_DEPTH` on purpose — the recursive DECODER's stack frame is heavier than the encoder's
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
/// The peer decodes the context-mixing codec ([`WireCompression::Cm`]).
pub const FEAT_CM: u32 = 1 << 6;

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
            features: FEAT_DEFLATE | FEAT_LZ4 | FEAT_ZSTD | FEAT_TYPE_ID | FEAT_COMPUTED | FEAT_FEC | FEAT_CM,
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
        Self {
            limits: DEFAULT_RECEIVE_LIMITS,
            registry_epoch: 0,
            features: 0,
        }
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
        limits: ReceiveLimits {
            max_bytes,
            max_depth,
            max_elements,
            max_string_bytes,
            accept_computed,
        },
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
    /// The strongest compression BOTH peers understand (`None` if they share none). Never the
    /// context-mixing codec: a negotiated send is the everyday path, and CM spends CPU for size.
    pub compression: WireCompression,
    /// Both peers decode the context-mixing codec, so a sender asked for its smallest form may use it.
    pub may_use_cm: bool,
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
        may_use_cm: both(FEAT_CM),
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
        WireCodec::Native => native_body(from, value)?,
        // JSON goes through the serde mirror (interop, not the speed path).
        WireCodec::Json => {
            let payload = materialize(value).map_err(|MarshalError::NotSendable(t)| {
                format!("a {t} cannot be sent over the network")
            })?;
            let msg = rt_to_wire(&payload).ok_or_else(|| {
                "a channel or task handle cannot be sent over the network".to_string()
            })?;
            serde_json::to_vec(&WireMessage {
                from: from.to_string(),
                msg,
            })
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

/// The native message body — the sender, then the value — encoded under the dials in scope. The column
/// menu follows the compression level: `Max` (spend CPU for size) adds the entropy-coded column form, any
/// other level keeps to the structural menu.
fn native_body(from: &str, value: &RuntimeValue) -> Result<Vec<u8>, String> {
    // A small base capacity covers the envelope + a scalar/short message without a realloc; the
    // packed-array arms reserve their own bulk.
    let mut out = Vec::with_capacity(from.len() + 32);
    write_str(from, &mut out);
    let menu = if compression_level() == WireCompressionLevel::Max {
        describe::Menu::Full
    } else {
        describe::Menu::Structural
    };
    describe::with_menu(menu, || native_encode(value, &mut out))?;
    Ok(out)
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
            let payload = wire_to_rt(msg);
            exact_payloads_valid(&payload).then(|| (from, rebuild(payload)))
        }
    }
}

/// Every column encoding the codec can select, by tag, with its plain-words name — the codec's own dial
/// vocabulary, for surfacing *which* encoding actually won (benchmarks, docs, debug). One table: the name
/// a message reports and the vocabulary a reader checks it against are the same list.
pub const COLUMN_TAG_NAMES: &[(u8, &str)] = &[
    (T_INTS, "varint"),
    (T_INTS_FIXED, "fixed (memcpy)"),
    (T_INTS_GV, "group-varint"),
    (T_INTS_ALIGNED, "fixed-aligned"),
    (T_INTS_AFFINE, "affine (base,stride,n)"),
    (T_INTS_DELTA, "delta"),
    (T_INTS_DOD, "delta-of-delta"),
    (T_INTS_FOR, "FOR bit-pack"),
    (T_INTS_RLE, "run-length"),
    (T_INTS_DICT, "dictionary"),
    (T_INTS_POLY, "polynomial"),
    (T_INTS_GEOMETRIC, "geometric"),
    (T_INTS_PERIODIC, "periodic"),
    (T_INTS_SPARSE, "sparse"),
    (T_GEN, "generator"),
    (T_BYTES, "byte column"),
    (describe::T_INTS_LRECUR, "linear-recurrence"),
    (describe::T_INTS_LFSR, "LFSR"),
    (describe::T_INTS_FCSR, "FCSR"),
    (describe::T_INTS_MULT, "common-multiple"),
    (describe::T_INTS_CM, "entropy-coded"),
    (describe::T_FLOATS_DECIMAL, "decimal floats"),
    (describe::T_FLOATS_BITS, "float bits"),
    (describe::T_INTS_CXOR, "conditional-xor"),
    (T_FLOATS, "memcpy floats"),
    (T_FLOATS_XOR, "xor-delta floats"),
    (T_FLOATS_CONST, "constant floats"),
    (T_FLOATS_AFFINE, "affine floats"),
    (T_FLOATS_SPARSE, "sparse floats"),
    (T_FLOATS_PERIODIC, "periodic floats"),
    (T_FLOATS_GEOMETRIC, "geometric floats"),
    (T_FLOATS_ALIGNED, "aligned floats"),
    (T_BOOLS, "bit-packed bools"),
    (T_BOOLS_PERIODIC, "periodic bools"),
    (T_BOOLS_RLE, "run-length bools"),
    (T_STRINGS, "flat strings"),
    (T_STRINGS_TEMPLATE, "templated strings"),
    (T_STRINGS_FRONT, "front-coded strings"),
    (T_STRINGS_AFFIX, "affix strings"),
    (T_STRINGS_DICT, "dictionary strings"),
    (T_SET_INTS, "int set (column menu)"),
    (T_SET_STRINGS, "string set (front-coded)"),
    (T_MAP_INTKEY, "int-keyed map (columnar)"),
];

/// The plain-words name of the structural encoding a column tag selects (see [`COLUMN_TAG_NAMES`]). A
/// non-column or unknown tag reads as the generic `"value"`.
fn column_tag_name(tag: u8) -> &'static str {
    COLUMN_TAG_NAMES.iter().find(|&&(t, _)| t == tag).map_or("value", |&(_, name)| name)
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
pub fn message_from_wire_cached(
    bytes: &[u8],
    cache: &mut WireSchemaCache,
) -> Option<(String, RuntimeValue)> {
    let _scope = CacheScope::enter(cache);
    message_from_wire(bytes)
}

/// Encode side: decide how to transmit a struct schema, recording state as needed.
fn schema_send(type_name: &str, field_names: &[String]) -> SchemaEmit {
    SCHEMA_CACHE.with(|c| {
        let mut g = c.borrow_mut();
        let Some(cache) = g.as_mut() else {
            return SchemaEmit::Inline;
        };
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
    SCHEMA_CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .and_then(|cache| cache.recv_seq.get(id as usize).cloned())
    })
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
    SCHEMA_CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .and_then(|cache| cache.recv_ca.get(&fp).cloned())
    })
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
        Self {
            by_id: canon,
            by_fp,
            ..Self::default()
        }
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
        self.by_fp
            .get(&schema_fingerprint(type_name, field_names))
            .copied()
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
    TYPE_REGISTRY.with(|c| {
        c.borrow()
            .as_ref()
            .and_then(|r| r.id_of(type_name, field_names))
    })
}

/// Resolve a registry id back to its `(type_name, field_names)` schema.
fn type_registry_schema(id: u32) -> Option<(String, Vec<String>)> {
    TYPE_REGISTRY.with(|c| c.borrow().as_ref().and_then(|r| r.schema_of(id)))
}

/// The active registry's `(enum_id, constructor_index)` for an enum value, if known.
fn type_registry_enum_id(type_name: &str, constructor: &str) -> Option<(u32, u32)> {
    TYPE_REGISTRY.with(|c| {
        c.borrow()
            .as_ref()
            .and_then(|r| r.enum_id_of(type_name, constructor))
    })
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
    let Some(p) = shareable_ptr(v) else {
        return false;
    };
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

/// Decompression-bomb budget for DECODE paths where a length prefix is NOT gated by
/// per-element input bytes (e.g. an RLE/dict run that repeats one element `n` times in zero
/// bytes). A crafted frame could otherwise request billions of copies from a tiny input and
/// OOM the peer. 256 MiB is far beyond any real peer message yet bounds the blast radius
/// (Invariant X). Length-gated paths are already bounded by the remaining buffer.
const MAX_DECODE_BYTES: usize = 1 << 28;

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
    let deferrable = view.structs_schema().is_some()
        || matches!(view.tag(), Some(T_INTS_ALIGNED) | Some(T_FLOATS_ALIGNED));
    if !deferrable {
        return None;
    }
    let (_, _, body) = unframe_with(bytes, false)?;
    let mut p = 0;
    read_str(body, &mut p)
}

pub fn message_from_wire_view(bytes: &[u8]) -> Option<(String, RuntimeValue)> {
    // Only a self-describing native record-list view is lazily wrappable.
    if view_message(bytes)
        .and_then(|v| v.structs_schema())
        .is_some()
    {
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
    Some(WireView {
        val: body.get(pos..)?,
    })
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
        Some(WireView {
            val: self.val.get(values_start.checked_add(offset)?..)?,
        })
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
        let row_off = u32::from_le_bytes(
            self.val
                .get(row_off_at..row_off_at.checked_add(4)?)?
                .try_into()
                .ok()?,
        ) as usize;
        let field_table_pos = rows_start.checked_add(row_off)?;
        let values_start = field_table_pos.checked_add(f.checked_mul(4)?)?;
        let field_off_at = field_table_pos.checked_add(fi.checked_mul(4)?)?;
        let field_off = u32::from_le_bytes(
            self.val
                .get(field_off_at..field_off_at.checked_add(4)?)?
                .try_into()
                .ok()?,
        ) as usize;
        Some(WireView {
            val: self.val.get(values_start.checked_add(field_off)?..)?,
        })
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
                let cell_pos = rows_start
                    .checked_add(row.checked_mul(stride)?)?
                    .checked_add(offsets[fi])?;
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
        // Cap the pre-allocation by remaining input: an untrusted field count can't exceed
        // the bytes left, so a hostile `f` can't trigger a giant OOM allocation (Invariant X).
        let mut field_names = Vec::with_capacity(f.min(self.val.len().saturating_sub(p)));
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
        // Cap the pre-allocation by remaining input (untrusted field count ⇒ OOM guard, Invariant X).
        let mut field_names = Vec::with_capacity(f.min(val.len().saturating_sub(p)));
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
                kind: CursorKind::Fixed {
                    field_kinds,
                    field_offsets,
                    stride,
                    rows_start,
                    blob,
                },
            })
        } else {
            let n = read_uvarint(val, &mut p)? as usize;
            let row_table_pos = p;
            let rows_start = row_table_pos.checked_add(n.checked_mul(4)?)?;
            Some(WireStructsCursor {
                val,
                field_names,
                n,
                kind: CursorKind::Variable {
                    row_table_pos,
                    rows_start,
                },
            })
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
    Variable {
        row_table_pos: usize,
        rows_start: usize,
    },
    Fixed {
        field_kinds: &'a [u8],
        field_offsets: Vec<usize>,
        stride: usize,
        rows_start: usize,
        blob: &'a [u8],
    },
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
            CursorKind::Fixed {
                field_offsets,
                stride,
                rows_start,
                ..
            } => {
                let pos = rows_start
                    .checked_add(row.checked_mul(*stride)?)?
                    .checked_add(field_offsets[fi])?;
                self.val.get(pos..)
            }
            CursorKind::Variable {
                row_table_pos,
                rows_start,
            } => {
                let f = self.field_names.len();
                let row_off_at = row_table_pos.checked_add(row.checked_mul(4)?)?;
                let row_off = u32::from_le_bytes(
                    self.val
                        .get(row_off_at..row_off_at.checked_add(4)?)?
                        .try_into()
                        .ok()?,
                ) as usize;
                let field_table_pos = rows_start.checked_add(row_off)?;
                let values_start = field_table_pos.checked_add(f.checked_mul(4)?)?;
                let field_off_at = field_table_pos.checked_add(fi.checked_mul(4)?)?;
                let field_off = u32::from_le_bytes(
                    self.val
                        .get(field_off_at..field_off_at.checked_add(4)?)?
                        .try_into()
                        .ok()?,
                ) as usize;
                self.val.get(values_start.checked_add(field_off)?..)
            }
        }
    }

    /// The `(row, field)` value as an owned `RuntimeValue` — O(1), no header re-scan.
    pub fn value(&self, row: usize, fi: usize) -> Option<RuntimeValue> {
        match &self.kind {
            CursorKind::Fixed {
                field_kinds, blob, ..
            } => fview_read_cell(*field_kinds.get(fi)?, self.cell_slice(row, fi)?, blob),
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
        let CursorKind::Fixed {
            field_kinds,
            field_offsets,
            stride,
            rows_start,
            ..
        } = &self.kind
        else {
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
const T_EXACT: u8 = 86; // algebraic (0) / computable real (1) / ordinal (2): kind + len + canonical bytes
const T_SCALED: u8 = 87; // hyperreal (0) / infinite integer (1): kind + name + len + residues + len + bytes
const T_DEPENDS: u8 = 88; // name + len + residues + modulus + per class (0 | 1 + value)
const T_CONVERGING: u8 = 89; // value + cases + revisions + (0 | 1 + certificate)
const T_COMPLEXF: u8 = 85; // floating-point complex (LAPACK Z): re then im, each 8-byte little-endian f64. Tag 85 (NOT 82/83/84 — the shared column-tag namespace reserves those for describe::T_INTS_{LRECUR,LFSR,FCSR}, which native_decode dispatches by the same byte)
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
        FK_INT => Some(RuntimeValue::Int(i64::from_le_bytes(
            cell.get(0..8)?.try_into().ok()?,
        ))),
        FK_FLOAT => Some(RuntimeValue::Float(f64::from_le_bytes(
            cell.get(0..8)?.try_into().ok()?,
        ))),
        FK_BOOL => Some(RuntimeValue::Bool(*cell.first()? != 0)),
        FK_TEXT => {
            let off = u32::from_le_bytes(cell.get(0..4)?.try_into().ok()?) as usize;
            let len = u32::from_le_bytes(cell.get(4..8)?.try_into().ok()?) as usize;
            let s = blob.get(off..off.checked_add(len)?)?;
            Some(RuntimeValue::Text(Rc::new(
                String::from_utf8(s.to_vec()).ok()?,
            )))
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
/// previous and varint-codes the result — lossless and bit-exact (it operates on the wire
/// bits, so Inf/±0/subnormals are preserved and a NaN is the canonical NaN, as everywhere on
/// the wire), and far smaller for slowly-varying
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
pub(crate) fn lower_expr_to_genexpr(
    e: &crate::ast::stmt::Expr<'_>,
    param: logicaffeine_base::Symbol,
) -> Option<GenExpr> {
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

/// If every element is BIT-IDENTICAL (compared by wire bits, so `-0.0` and `+0.0` differ and every
/// NaN is the one canonical NaN, not `==`), return its bit pattern — the column ships as one f64 + a
/// count (constant readings, padding,
/// defaults). `None` for an empty or non-constant column.
fn detect_float_const(v: &[f64]) -> Option<u64> {
    let bits = canonical_f64_bits(*v.first()?);
    v.iter().all(|x| canonical_f64_bits(*x) == bits).then_some(bits)
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
        if canonical_f64_bits(base + (i as f64) * stride) != canonical_f64_bits(x) {
            return None;
        }
    }
    Some((base, stride))
}

/// The float twin of [`detect_sparse`]: if ONE f64 (by bit pattern) dominates ≥ ¾ of the column,
/// return its bits and the sorted `(index, value)` exceptions — a mostly-default/constant-with-
/// outliers float column (sparse telemetry, a mostly-zero signal). Boyer–Moore over the wire bits, so a
/// column with no dominant value pays only the O(1)-memory pass.
fn detect_float_sparse(v: &[f64]) -> Option<(u64, Vec<(usize, u64)>)> {
    if v.len() < 8 {
        return None;
    }
    let mut cand = canonical_f64_bits(v[0]);
    let mut count: i64 = 0;
    for x in v {
        let b = canonical_f64_bits(*x);
        if count == 0 {
            cand = b;
            count = 1;
        } else if b == cand {
            count += 1;
        } else {
            count -= 1;
        }
    }
    let occ = v.iter().filter(|x| canonical_f64_bits(**x) == cand).count();
    if v.len() - occ > v.len() / 4 {
        return None;
    }
    let exceptions: Vec<(usize, u64)> = v
        .iter()
        .enumerate()
        .filter(|(_, x)| canonical_f64_bits(**x) != cand)
        .map(|(i, x)| (i, canonical_f64_bits(*x)))
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
            if canonical_f64_bits(v[i]) != canonical_f64_bits(v[i - p]) {
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
        if canonical_f64_bits(cur) != canonical_f64_bits(x) {
            return None;
        }
        cur *= ratio;
    }
    Some((base, ratio))
}

/// XOR-delta encode a float column: count, then the LEB128 varint of each value's
/// wire bits XOR the previous value's. Lossless and bit-exact, a NaN canonical.
fn floats_xor_encode(out: &mut Vec<u8>, v: &[f64]) {
    write_uvarint(v.len() as u64, out);
    let mut prev = 0u64;
    for &f in v {
        let bits = canonical_f64_bits(f);
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
    /// Context mixing (`logicaffeine_base::cm` at its fast level) — a smaller body than any of the above on
    /// text and structured bytes, at a few hundred kilobytes a second each way. Pure Rust, so every target
    /// encodes and decodes it; bodies are kept within [`MAX_CM_DECODE_BYTES`].
    Cm,
}

/// The largest body the context-mixing codec carries, and the most bytes its model may code. Its decode costs
/// several microseconds a byte, so the cap — not the general decompression budget — bounds what a small frame
/// can make a receiver spend: a few seconds at most.
pub const MAX_CM_DECODE_BYTES: usize = 1 << 20;

/// The 2-bit on-wire id for a codec (header bits 2-3). `None`/`Deflate` share id 0;
/// `None` is distinguished by `H_COMPRESSED` being unset.
fn compression_id(c: WireCompression) -> u8 {
    match c {
        WireCompression::None | WireCompression::Deflate => 0,
        WireCompression::Lz4 => 1,
        WireCompression::Zstd => 2,
        WireCompression::Cm => 3,
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
        WireCompression::Deflate => Some((
            codec,
            miniz_oxide::deflate::compress_to_vec(body, deflate_level()),
        )),
        WireCompression::Lz4 => Some((codec, lz4_flex::compress_prepend_size(body))),
        WireCompression::Zstd => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                zstd::encode_all(body, zstd_level())
                    .ok()
                    .map(|z| (WireCompression::Zstd, z))
            }
            #[cfg(target_arch = "wasm32")]
            {
                // No C encoder in the browser — fall back to lz4 (still universally
                // decodable). The header will record lz4, not zstd.
                Some((WireCompression::Lz4, lz4_flex::compress_prepend_size(body)))
            }
        }
        WireCompression::Cm => (body.len() <= MAX_CM_DECODE_BYTES)
            .then(|| (WireCompression::Cm, logicaffeine_base::cm::compress_with(body, logicaffeine_base::cm::Level::Fast, MAX_CM_DECODE_BYTES))),
    }
}

/// The smallest `body` compresses to across the built-in compressors (deflate / lz4 / zstd), or its
/// raw length when none helps — the "fair fight" size for an arbitrary byte string. This is the same
/// shop-every-compressor rule [`message_to_wire_best`]'s `Smallest` goal applies to the LOGOS wire,
/// exposed so a benchmark can grant a COMPETITOR codec the identical compression opportunity: then a
/// size comparison is compressed-vs-compressed (fair), not compressed-LOGOS-vs-raw-competitor.
pub fn best_compressed_len(body: &[u8]) -> usize {
    [
        WireCompression::Deflate,
        WireCompression::Lz4,
        WireCompression::Zstd,
        WireCompression::Cm,
    ]
    .into_iter()
    .filter_map(|c| compress_body(c, body).map(|(_, z)| z.len()))
    .fold(body.len(), usize::min)
}

/// Compress `body` with the SMALLEST of the built-in codecs (deflate / lz4 / zstd), self-describing:
/// byte 0 is the codec tag (0=deflate, 1=lz4, 2=zstd, 3=stored raw), the rest is the body. When
/// nothing shrinks the input it is stored raw under tag 3 — so this NEVER grows a payload by more
/// than the single tag byte (the discipline for already-encoded / already-encrypted data, which does
/// not compress). Compress-BEFORE-encrypt is the intended use: ciphertext is incompressible, so the
/// squeeze must happen on the plaintext frame. The inverse is [`decompress_any`].
pub fn compress_best(body: &[u8]) -> Vec<u8> {
    let mut best_tag = 3u8; // stored (raw)
    let mut best_body: Vec<u8> = body.to_vec();
    for codec in [
        WireCompression::Deflate,
        WireCompression::Lz4,
        WireCompression::Zstd,
    ] {
        if let Some((used, z)) = compress_body(codec, body) {
            if z.len() < best_body.len() {
                best_tag = compression_id(used); // records the codec ACTUALLY used (wasm zstd→lz4)
                best_body = z;
            }
        }
    }
    let mut out = Vec::with_capacity(best_body.len() + 1);
    out.push(best_tag);
    out.extend_from_slice(&best_body);
    out
}

/// Inverse of [`compress_best`]: read the codec tag and inflate. `None` on a malformed or oversized
/// frame — the decompression-bomb bound (`MAX_DECODE_BYTES`) is inherited from `decompress_body`.
pub fn decompress_any(frame: &[u8]) -> Option<Vec<u8>> {
    let (&tag, body) = frame.split_first()?;
    let codec = match tag {
        0 => WireCompression::Deflate,
        1 => WireCompression::Lz4,
        2 => WireCompression::Zstd,
        3 => return Some(body.to_vec()), // stored raw
        _ => return None,
    };
    decompress_body(codec, body)
}

/// Inflate `body` that was compressed with `codec`. `None` on any malformed input.
fn decompress_body(codec: WireCompression, body: &[u8]) -> Option<Vec<u8>> {
    match codec {
        WireCompression::None => Some(body.to_vec()),
        // Decompression-bomb guards (Invariant X): a tiny compressed frame must never inflate to
        // gigabytes and OOM the peer. Bound EVERY codec's output at MAX_DECODE_BYTES.
        WireCompression::Deflate => {
            miniz_oxide::inflate::decompress_to_vec_with_limit(body, MAX_DECODE_BYTES).ok()
        }
        WireCompression::Lz4 => {
            // lz4's 4-byte little-endian size prefix is untrusted — reject a huge claim BEFORE
            // `decompress_size_prepended` allocates it.
            if body.len() < 4 {
                return None;
            }
            let claimed = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
            if claimed > MAX_DECODE_BYTES {
                return None;
            }
            lz4_flex::decompress_size_prepended(body).ok()
        }
        WireCompression::Zstd => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                use std::io::Read;
                // zstd carries no trusted output size, so bound the read: take one byte past the
                // budget and reject if the stream would exceed it.
                let mut out = Vec::new();
                let dec = zstd::stream::read::Decoder::new(body).ok()?;
                dec.take(MAX_DECODE_BYTES as u64 + 1)
                    .read_to_end(&mut out)
                    .ok()?;
                if out.len() > MAX_DECODE_BYTES {
                    return None;
                }
                Some(out)
            }
            #[cfg(target_arch = "wasm32")]
            {
                zstd_decode_ruzstd(body)
            }
        }
        // The length prefix is checked against the cap before any model is built or bit decoded.
        WireCompression::Cm => logicaffeine_base::cm::decompress_within(body, MAX_CM_DECODE_BYTES, MAX_CM_DECODE_BYTES),
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
        let bytes = unsafe {
            std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v))
        };
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
            let mask = if width == 8 {
                u64::MAX
            } else {
                (1u64 << (width * 8)) - 1
            };
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
    let iw = if dict.len() <= 1 {
        0
    } else {
        (64 - ((dict.len() - 1) as u64).leading_zeros()) as u8
    };
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
            out[field_table_pos + i * 4..field_table_pos + i * 4 + 4]
                .copy_from_slice(&off.to_le_bytes());
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
            out[field_table_pos + i * 4..field_table_pos + i * 4 + 4]
                .copy_from_slice(&off.to_le_bytes());
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
            put_f64(out, v[row]);
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
        other => native_encode(
            &other
                .get(row)
                .ok_or("struct-view column row out of bounds")?,
            out,
        )?,
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
                ListRepr::Floats(v) => put_f64(out, v[r]),
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
    put_f64s(out, v);
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
pub fn build_columnar_record(
    from: &str,
    type_name: &str,
    fields: &[(&str, WireColumn)],
) -> Vec<u8> {
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
    frame(
        WireCodec::Native,
        current_integrity(),
        WireCompression::None,
        out,
    )
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
    // A packed `Words32` list ALSO takes this fully-tagged path: it is a forge-only pin optimization of a
    // `Seq of Word32` (previously `Boxed`), so encoding it as tagged `Word` values keeps the wire bytes
    // byte-identical to the old boxed encoding — no new columnar tag, no decoder change.
    if flat_lists() || matches!(repr, ListRepr::Words32(_)) {
        let values = repr.to_values();
        out.push(T_LIST);
        write_uvarint(values.len() as u64, out);
        for x in &values {
            native_encode(x, out)?;
        }
        return Ok(());
    }
    match repr {
        // A `Words32` list always took the fully-tagged flat path above (byte-identical to the old boxed
        // `Seq of Word32` encoding), so it never reaches this columnar dispatch.
        ListRepr::Words32(_) => unreachable!("Words32 encodes via the flat/tagged path"),
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
                    put_f64(out, base);
                    put_f64(out, stride);
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
                    put_f64(&mut c, base);
                    put_f64(&mut c, ratio);
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
                    put_f64s(&mut c, &v[..p]);
                    c
                })
            } else {
                None
            };
            // A column of decimals (`k / 10^e`) describes as its integer column `k`; any other column can
            // still describe as its bit patterns. Both go through the full integer menu, so both see the
            // number-theoretic and entropy-coded forms. They read the column with every NaN made the
            // canonical one, as every other float encoding writes it.
            let (decimal, bits) = if st == WireStructure::Auto {
                let canon: Vec<f64> = v.iter().map(|&x| canonical_f64(x)).collect();
                (describe::emit_decimal_float_column(&canon), describe::emit_float_bits_column(&canon))
            } else {
                (None, None)
            };
            if let Some(c) = [sparse, xor, geometric, periodic, decimal, bits]
                .into_iter()
                .flatten()
                .min_by_key(Vec::len)
            {
                // The memcpy column is `1` (tag) + body; keep the candidate only if it is smaller.
                if c.len() < floats_memcpy_body_len(v.len()) + 1 {
                    out.extend_from_slice(&c);
                    return Ok(());
                }
            }
            out.push(T_FLOATS);
            write_uvarint(v.len() as u64, out);
            // One pass over the column: each element's little-endian bytes, a NaN written canonically.
            put_f64s(out, v);
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
        ListRepr::Structs {
            type_name,
            field_names,
            columns,
        } => {
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
        ListRepr::Inductives {
            inductive_type,
            ctor_dict,
            ctors,
            ranks: _,
            arg_cols,
        } => {
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
                            RuntimeValue::Struct(sv) => {
                                sv.fields.get(&field_names[fi]).cloned().unwrap()
                            }
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
    let currency =
        logicaffeine_base::money::currency::by_code(code).unwrap_or(logicaffeine_base::Currency {
            code: "XXX",
            scale: 0,
        });
    Some(RuntimeValue::Money(Rc::new(logicaffeine_base::Money {
        amount,
        currency,
    })))
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
    Some(RuntimeValue::Uuid(Rc::new(
        logicaffeine_base::Uuid::from_bytes(arr),
    )))
}

/// Write a value as tagged varint bytes. `Err` for a non-portable value (with the
/// same messages the JSON path produces), caught at the exact offending node.
/// Why the wire refuses a value holding a real number a function of the program defines.
fn never_sent() -> String {
    format!(
        "{} stays with its program and is never sent",
        logicaffeine_base::exact_real::STAYS_WITH_ITS_PROGRAM
    )
}

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
            return Err(
                "a SIMD lane vector is a transient compute value, not a wire type".to_string(),
            );
        }
        RuntimeValue::ComplexF(c) => {
            out.push(T_COMPLEXF);
            put_f64(out, c.re());
            put_f64(out, c.im());
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
            put_f64(out, *f);
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
                    .map(|x| {
                        if let RuntimeValue::Int(n) = x {
                            *n
                        } else {
                            unreachable!()
                        }
                    })
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
                    .map(|x| {
                        if let RuntimeValue::Text(t) = x {
                            (**t).clone()
                        } else {
                            unreachable!()
                        }
                    })
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
                    let vlist = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                        vals.clone(),
                    ))));
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
                    out[table_pos + i * 4..table_pos + i * 4 + 4]
                        .copy_from_slice(&off.to_le_bytes());
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
            if let Some((enum_id, ctor_idx)) =
                type_registry_enum_id(&ind.inductive_type, &ind.constructor)
            {
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
        RuntimeValue::Algebraic(_) | RuntimeValue::ExactReal(_) | RuntimeValue::Ordinal(_) => {
            let Some((kind, bytes)) = crate::semantics::exact::exact_bytes(v) else {
                return Err(never_sent());
            };
            out.push(T_EXACT);
            out.push(kind);
            write_uvarint(bytes.len() as u64, out);
            out.extend_from_slice(&bytes);
        }
        RuntimeValue::Hyperreal(_) | RuntimeValue::Hypernatural(_) => {
            let Some((kind, name, residues, bytes)) = crate::semantics::exact::scaled_bytes(v) else {
                return Err(never_sent());
            };
            out.push(T_SCALED);
            out.push(kind);
            write_str(&name, out);
            write_uvarint(residues.len() as u64, out);
            out.extend_from_slice(&residues);
            write_uvarint(bytes.len() as u64, out);
            out.extend_from_slice(&bytes);
        }
        RuntimeValue::Depends(d) => {
            let (name, residues) = crate::semantics::exact::scale_parts(&d.scale);
            out.push(T_DEPENDS);
            write_str(&name, out);
            write_uvarint(residues.len() as u64, out);
            out.extend_from_slice(&residues);
            write_uvarint(d.value.modulus(), out);
            for branch in d.value.raw_branches() {
                match branch {
                    None => out.push(0),
                    Some(b) => {
                        out.push(1);
                        native_encode(&b.0, out)?;
                    }
                }
            }
        }
        RuntimeValue::Converging(c) => {
            out.push(T_CONVERGING);
            native_encode(c.value(), out)?;
            write_uvarint(c.steps(), out);
            write_uvarint(c.revisions(), out);
            match c.certificate() {
                None => out.push(0),
                Some(cert) => {
                    out.push(1);
                    write_str(cert, out);
                }
            }
        }
        // A view belongs to the page that rendered it and never crosses the wire.
        RuntimeValue::View(_) | RuntimeValue::Action(_) => {
            return Err(
                "a view or callback belongs to the page that made it and cannot be sent"
                    .to_string(),
            );
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
        T_EXACT => {
            let kind = *buf.get(*pos)?;
            *pos += 1;
            let n = read_uvarint(buf, pos)? as usize;
            let bytes = buf.get(*pos..pos.checked_add(n)?)?;
            *pos += n;
            crate::semantics::exact::from_exact_bytes(kind, bytes)?
        }
        T_SCALED => {
            let kind = *buf.get(*pos)?;
            *pos += 1;
            let name = read_str(buf, pos)?;
            let rn = read_uvarint(buf, pos)? as usize;
            let residues = buf.get(*pos..pos.checked_add(rn)?)?.to_vec();
            *pos += rn;
            let n = read_uvarint(buf, pos)? as usize;
            let bytes = buf.get(*pos..pos.checked_add(n)?)?;
            *pos += n;
            crate::semantics::exact::from_scaled_bytes(kind, &name, &residues, bytes)?
        }
        T_DEPENDS => {
            let name = read_str(buf, pos)?;
            let rn = read_uvarint(buf, pos)? as usize;
            let residues = buf.get(*pos..pos.checked_add(rn)?)?.to_vec();
            *pos += rn;
            let modulus = read_uvarint(buf, pos)?;
            if modulus == 0 || modulus > logicaffeine_base::exact_codec::MAX_WIRE_LEN as u64 {
                return None;
            }
            let mut branches = Vec::with_capacity(modulus as usize);
            for _ in 0..modulus {
                branches.push(match *buf.get(*pos)? {
                    0 => {
                        *pos += 1;
                        None
                    }
                    1 => {
                        *pos += 1;
                        Some(native_decode(buf, pos)?)
                    }
                    _ => return None,
                });
            }
            crate::semantics::exact::depends_from_parts(&name, &residues, modulus, branches)?
        }
        T_CONVERGING => {
            let value = native_decode(buf, pos)?;
            if matches!(value, RuntimeValue::Converging(_)) {
                return None;
            }
            let steps = read_uvarint(buf, pos)?;
            let revisions = read_uvarint(buf, pos)?;
            let converging = match *buf.get(*pos)? {
                0 => {
                    *pos += 1;
                    logicaffeine_base::limit_value::Converging::provisional(value, steps, revisions)
                }
                1 => {
                    *pos += 1;
                    if steps != 0 || revisions != 0 {
                        return None;
                    }
                    let cert = read_str(buf, pos)?;
                    logicaffeine_base::limit_value::Converging::settled(value, &cert)
                }
                _ => return None,
            };
            RuntimeValue::Converging(Rc::new(converging))
        }
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
            let re = logicaffeine_base::Rational::new(
                re_num,
                logicaffeine_base::BigInt::from_le_bytes(false, re_db),
            )?;
            let im_neg = *buf.get(*pos)? != 0;
            *pos += 1;
            let im_nlen = read_uvarint(buf, pos)? as usize;
            let im_nb = buf.get(*pos..pos.checked_add(im_nlen)?)?;
            *pos += im_nlen;
            let im_num = logicaffeine_base::BigInt::from_le_bytes(im_neg, im_nb);
            let im_dlen = read_uvarint(buf, pos)? as usize;
            let im_db = buf.get(*pos..pos.checked_add(im_dlen)?)?;
            *pos += im_dlen;
            let im = logicaffeine_base::Rational::new(
                im_num,
                logicaffeine_base::BigInt::from_le_bytes(false, im_db),
            )?;
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
            let magnitude = logicaffeine_base::Rational::new(
                num,
                logicaffeine_base::BigInt::from_le_bytes(false, db),
            )?;
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
        T_COMPLEXF => {
            let re: [u8; 8] = buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?;
            *pos += 8;
            let im: [u8; 8] = buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?;
            *pos += 8;
            RuntimeValue::ComplexF(Rc::new(logicaffeine_base::ComplexF::new(
                f64::from_le_bytes(re),
                f64::from_le_bytes(im),
            )))
        }
        T_CHAR => RuntimeValue::Char(char::from_u32(
            u32::try_from(read_uvarint(buf, pos)?).ok()?,
        )?),
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
        | describe::T_INTS_FCSR
        | describe::T_INTS_MULT
        | describe::T_INTS_CM
        | describe::T_INTS_CXOR => {
            // The Auto column menu decodes through the shared engine (single source of the format);
            // the receiver's element budget is threaded in so the DoS gate is unchanged.
            let v =
                describe::decode_int_column_body(tag, buf, pos, receive_limits().max_elements, 0)?;
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(v))))
        }
        T_INTS_GV => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(gv_decode_dispatch(
            buf, pos,
        )?)))),
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
            let param_names: Vec<logicaffeine_base::Symbol> = (0..arity)
                .map(logicaffeine_base::Symbol::from_index)
                .collect();
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
                    std::ptr::copy_nonoverlapping(
                        raw.as_ptr(),
                        v.as_mut_ptr().cast::<u8>(),
                        nbytes,
                    );
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
                    std::ptr::copy_nonoverlapping(
                        raw.as_ptr(),
                        v.as_mut_ptr().cast::<u8>(),
                        nbytes,
                    );
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
                    std::ptr::copy_nonoverlapping(
                        raw.as_ptr(),
                        v.as_mut_ptr().cast::<u8>(),
                        nbytes,
                    );
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
            let dom = f64::from_bits(u64::from_le_bytes(
                buf.get(*pos..pos.checked_add(8)?)?.try_into().ok()?,
            ));
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
        // Decimal and bit-pattern float columns: an integer column underneath, decoded by the shared
        // engine under the receiver's element budget.
        describe::T_FLOATS_DECIMAL | describe::T_FLOATS_BITS => {
            let v = describe::decode_float_column_body(tag, buf, pos, receive_limits().max_elements, 0)?;
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
                    std::ptr::copy_nonoverlapping(
                        raw.as_ptr(),
                        v.as_mut_ptr().cast::<u8>(),
                        nbytes,
                    );
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
            let push = |s: &[u8], out_data: &mut Vec<u8>, out_ends: &mut Vec<u32>| {
                out_data.extend_from_slice(s);
                out_ends.push(out_data.len() as u32);
            };
            if iw == 0 {
                if n > 0 {
                    let s = *dict.first()?;
                    // Decompression-bomb guard (Invariant X): iw==0 encodes n identical
                    // strings in ZERO index bytes, so `n` is ungated by input size. Reject if
                    // the expansion (data bytes + 4-byte end offsets) would exceed the budget.
                    if n.checked_mul(s.len().max(4))? > MAX_DECODE_BYTES {
                        return None;
                    }
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
            // A length prefix can never exceed the bytes left — every entry needs at least
            // one wire byte — so cap the pre-allocation by the remaining input. A lying `n`
            // from a corrupt or hostile frame can no longer trigger a giant OOM allocation;
            // the loop below then fails cleanly when the buffer runs out. Invariant X.
            let mut items = Vec::with_capacity(n.min(buf.len().saturating_sub(*pos)));
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
            let mut fields =
                std::collections::HashMap::with_capacity((n as usize).min(PREALLOC_CAP));
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
                rows.push(decode_struct_values(
                    buf,
                    pos,
                    type_name.clone(),
                    field_names.clone(),
                )?);
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
                            v.push(i64::from_le_bytes(
                                rows_bytes
                                    .get(base..base.checked_add(8)?)?
                                    .try_into()
                                    .ok()?,
                            ));
                        }
                        ListRepr::Ints(v)
                    }
                    FK_FLOAT => {
                        let mut v = Vec::with_capacity(n.min(PREALLOC_CAP));
                        for r in 0..n {
                            let base = r.checked_mul(stride)?.checked_add(off)?;
                            v.push(f64::from_le_bytes(
                                rows_bytes
                                    .get(base..base.checked_add(8)?)?
                                    .try_into()
                                    .ok()?,
                            ));
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
                            let toff = u32::from_le_bytes(
                                rows_bytes
                                    .get(base..base.checked_add(4)?)?
                                    .try_into()
                                    .ok()?,
                            ) as usize;
                            let tlen = u32::from_le_bytes(
                                rows_bytes
                                    .get(base.checked_add(4)?..base.checked_add(8)?)?
                                    .try_into()
                                    .ok()?,
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
            RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Structs {
                type_name,
                field_names,
                columns,
            })))
        }
        T_INDUCTIVE => {
            let inductive_type = read_str(buf, pos)?;
            let constructor = read_str(buf, pos)?;
            let args = read_seq(buf, pos)?;
            RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
                inductive_type,
                constructor,
                args,
            }))
        }
        // Type-id elided enum: resolve the enum id + constructor index against the shared
        // registry, then read the args. Unknown id / out-of-range index → None (clean).
        T_INDUCTIVE_TID => {
            let enum_id = read_uvarint(buf, pos)? as u32;
            let ctor_idx = read_uvarint(buf, pos)? as usize;
            let (inductive_type, ctors) = type_registry_enum_schema(enum_id)?;
            let constructor = ctors.get(ctor_idx)?.clone();
            let args = read_seq(buf, pos)?;
            RuntimeValue::Inductive(std::rc::Rc::new(InductiveValue {
                inductive_type,
                constructor,
                args,
            }))
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
                RuntimeValue::List(l) => Rc::try_unwrap(l)
                    .map(RefCell::into_inner)
                    .unwrap_or_else(|rc| rc.borrow().clone()),
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
                        RuntimeValue::List(l) => Rc::try_unwrap(l)
                            .map(RefCell::into_inner)
                            .unwrap_or_else(|rc| rc.borrow().clone()),
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
                if arg_cols[c]
                    .iter()
                    .any(|col| col.len() != counts[c] as usize)
                {
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
    Some(RuntimeValue::Struct(Box::new(StructValue {
        type_name,
        fields,
    })))
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
            RuntimeValue::List(l) => Rc::try_unwrap(l)
                .map(RefCell::into_inner)
                .unwrap_or_else(|rc| rc.borrow().clone()),
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
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Structs {
            type_name,
            field_names,
            columns,
        })))
    })
}

fn header(codec: WireCodec, integrity: WireIntegrity, compression: WireCompression) -> u8 {
    let c = if matches!(codec, WireCodec::Json) {
        H_JSON
    } else {
        0
    };
    let i = if matches!(integrity, WireIntegrity::Checked) {
        H_CHECKED
    } else {
        0
    };
    let z = if compression == WireCompression::None {
        0
    } else {
        H_COMPRESSED | (compression_id(compression) << 2)
    };
    c | i | z
}

/// Wrap a body in its frame: the header byte, then (for `Checked`) an 8-byte
/// FNV-1a checksum over the (possibly compressed) body, then the body.
fn frame(
    codec: WireCodec,
    integrity: WireIntegrity,
    compression: WireCompression,
    body: Vec<u8>,
) -> Vec<u8> {
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
    let codec = if h & H_JSON != 0 {
        WireCodec::Json
    } else {
        WireCodec::Native
    };
    let compression = if h & H_COMPRESSED == 0 {
        WireCompression::None
    } else {
        match (h & H_CODEC) >> 2 {
            0 => WireCompression::Deflate,
            1 => WireCompression::Lz4,
            2 => WireCompression::Zstd,
            _ => WireCompression::Cm,
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
    INTEGRITY_OVERRIDE
        .with(std::cell::Cell::get)
        .unwrap_or_else(default_integrity)
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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/concurrency/marshal/tests.rs"]
mod tests;
