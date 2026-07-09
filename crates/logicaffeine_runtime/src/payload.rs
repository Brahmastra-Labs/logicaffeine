//! `RtPayload` — the `Send`-able value subset that crosses task (and OS-thread)
//! boundaries via channels.
//!
//! The interpreter and VM heaps are `Rc`-based (`!Send`). A value moved through a
//! channel is *materialized* into this owned, allocation-self-contained form on
//! the sending side and *rebuilt* into the receiver's heap on the other side.
//! The marshalling between `RuntimeValue` / the VM `Value` and `RtPayload` lives
//! in `logicaffeine-compile` (which knows those representations); this crate only
//! defines the wire shape and guarantees it is `Send`, which is what makes the
//! M:N work-stealing driver sound.

/// A self-contained, `Send` value that can move between tasks and threads.
///
/// This deliberately excludes `Rc`/`RefCell`-backed and closure values: those
/// either are not `Send` or would alias another task's heap. CRDT shared cells
/// (which are `Arc`-backed and shared rather than moved) get their own variant
/// when the scheduler grows them in a later phase.
#[derive(Debug, Clone, PartialEq)]
pub enum RtPayload {
    /// The unit / absence value.
    Nothing,
    Int(i64),
    /// An exact integer that does not fit `i64`, carried as its sign + little-endian
    /// magnitude bytes — a dependency-free, `Send` form of `BigInt` (reconstructed
    /// with `BigInt::from_le_bytes` on the far side).
    BigInt { negative: bool, magnitude: Vec<u8> },
    /// An exact rational: the signed numerator and the (always positive) denominator,
    /// each as little-endian magnitude bytes — a dependency-free, `Send` form of
    /// `Rational` (reconstructed with `Rational::new` on the far side). `1/3` survives
    /// here exactly, where a JSON `0.333…` would round.
    Rational { num_negative: bool, num_magnitude: Vec<u8>, den_magnitude: Vec<u8> },
    /// An exact base-10 fixed-point number (money): its signed coefficient as
    /// little-endian magnitude bytes plus the base-10 `scale` (decimal places) — a
    /// dependency-free, `Send` form of `Decimal` (reconstructed with
    /// `Decimal::from_le_bytes` on the far side). `19.99` survives exactly, scale and all.
    Decimal { negative: bool, magnitude: Vec<u8>, scale: u32 },
    /// An exact monetary amount: its `Decimal` amount (sign + LE magnitude + base-10 scale) plus the
    /// ISO-4217 currency code. Reconstructed as `Money { Decimal::from_le_bytes, currency::by_code }`.
    Money { negative: bool, magnitude: Vec<u8>, scale: u32, currency: String },
    /// A 128-bit UUID — its 16 big-endian bytes verbatim. Reconstructed with `Uuid::from_bytes`.
    Uuid([u8; 16]),
    /// An exact complex number `re + im·i`: each part a rational (signed numerator + a
    /// positive denominator) as little-endian magnitude bytes. Reconstructed with
    /// `Complex::new` on the far side; `i·i = −1` survives exactly.
    Complex {
        re_negative: bool,
        re_num: Vec<u8>,
        re_den: Vec<u8>,
        im_negative: bool,
        im_num: Vec<u8>,
        im_den: Vec<u8>,
    },
    /// An element of ℤ/nℤ: its (non-negative) residue and modulus as little-endian magnitude
    /// bytes. Reconstructed with `Modular::new` on the far side; the ring is preserved.
    Modular { value: Vec<u8>, modulus: Vec<u8> },
    /// A dimensioned physical quantity: its SI-base magnitude as an exact rational (signed
    /// numerator + positive denominator, little-endian bytes), its dimension as the exponent
    /// vector (`dim_num[i]/dim_den[i]` over the base axes), and the display unit's symbol. The far
    /// side rebuilds the magnitude with `Rational`, the dimension with `Dimension::from_exps`, and
    /// resolves the unit by symbol (falling back to the SI/dimension display for compound units).
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
    /// An owned string (not an `Rc<str>`).
    Text(String),
    /// A fully-materialized sequence.
    List(Vec<RtPayload>),
    /// A fixed heterogeneous tuple.
    Tuple(Vec<RtPayload>),
    /// A set, materialized as its elements.
    Set(Vec<RtPayload>),
    /// A map, materialized as key/value pairs.
    Map(Vec<(RtPayload, RtPayload)>),
    /// A struct instance: its type name and named fields.
    Struct {
        type_name: String,
        fields: Vec<(String, RtPayload)>,
    },
    /// An inductive (sum-type) value: its type, constructor, and arguments.
    Inductive {
        type_name: String,
        constructor: String,
        args: Vec<RtPayload>,
    },
    /// A duration, carried in its base unit.
    Duration(i64),
    /// A calendar date.
    Date(i32),
    /// A moment in time.
    Moment(i64),
    /// A calendar span (months + days).
    Span { months: i32, days: i32 },
    /// A time-of-day.
    Time(i64),
    /// A fixed-width wrapping integer (`Word32`/`Word64`): its bit width (32 or 64) and the
    /// value zero-extended to `u64`. Reconstructed via `WordVal::from_u64` on the far side.
    Word { width: u32, bits: u64 },
    /// A channel handle (a `Pipe`) — an opaque scheduler token. `Send` (just an
    /// id), so a channel can be passed as a spawn argument across worker threads;
    /// the receiving task resolves it against the one shared scheduler.
    Chan(crate::channel::ChanId),
    /// A spawned-task handle — likewise an opaque `Send` scheduler token.
    TaskHandle(crate::task::TaskId),
    /// A remote-peer handle — its canonical relay topic. A `String` is trivially
    /// `Send`, so a peer can be passed as a spawn argument across worker threads.
    Peer(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn rtpayload_is_send() {
        // Compile-time guarantee: payloads can cross thread boundaries. This is
        // the property the M:N work-stealing driver depends on (only `RtPayload`
        // and small ids ever cross a worker boundary).
        assert_send::<RtPayload>();
    }

    #[test]
    fn rtpayload_roundtrips_structurally() {
        let v = RtPayload::Struct {
            type_name: "Point".into(),
            fields: vec![
                ("x".into(), RtPayload::Int(1)),
                (
                    "y".into(),
                    RtPayload::List(vec![RtPayload::Bool(true), RtPayload::Text("hi".into())]),
                ),
            ],
        };
        assert_eq!(v.clone(), v);
    }

    #[test]
    fn rtpayload_nested_collections() {
        let m = RtPayload::Map(vec![
            (RtPayload::Text("a".into()), RtPayload::Int(1)),
            (RtPayload::Text("b".into()), RtPayload::Set(vec![RtPayload::Char('x')])),
        ]);
        match m {
            RtPayload::Map(entries) => assert_eq!(entries.len(), 2),
            _ => panic!("expected map"),
        }
    }
}
