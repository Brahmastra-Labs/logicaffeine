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
    BigInt {
        negative: bool,
        magnitude: Vec<u8>,
    },
    /// An exact rational: the signed numerator and the (always positive) denominator,
    /// each as little-endian magnitude bytes — a dependency-free, `Send` form of
    /// `Rational` (reconstructed with `Rational::new` on the far side). `1/3` survives
    /// here exactly, where a JSON `0.333…` would round.
    Rational {
        num_negative: bool,
        num_magnitude: Vec<u8>,
        den_magnitude: Vec<u8>,
    },
    /// An exact base-10 fixed-point number (money): its signed coefficient as
    /// little-endian magnitude bytes plus the base-10 `scale` (decimal places) — a
    /// dependency-free, `Send` form of `Decimal` (reconstructed with
    /// `Decimal::from_le_bytes` on the far side). `19.99` survives exactly, scale and all.
    Decimal {
        negative: bool,
        magnitude: Vec<u8>,
        scale: u32,
    },
    /// An exact monetary amount: its `Decimal` amount (sign + LE magnitude + base-10 scale) plus the
    /// ISO-4217 currency code. Reconstructed as `Money { Decimal::from_le_bytes, currency::by_code }`.
    Money {
        negative: bool,
        magnitude: Vec<u8>,
        scale: u32,
        currency: String,
    },
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
    /// A floating-point complex (LAPACK `Z`): the real and imaginary parts as plain `f64`s. The
    /// inexact dual of `Complex` — it is what the sqrt-based complex decompositions run on.
    ComplexF {
        re: f64,
        im: f64,
    },
    /// An element of ℤ/nℤ: its (non-negative) residue and modulus as little-endian magnitude
    /// bytes. Reconstructed with `Modular::new` on the far side; the ring is preserved.
    Modular {
        value: Vec<u8>,
        modulus: Vec<u8>,
    },
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
    Span {
        months: i32,
        days: i32,
    },
    /// A time-of-day.
    Time(i64),
    /// A fixed-width wrapping integer (`Word32`/`Word64`): its bit width (32 or 64) and the
    /// value zero-extended to `u64`. Reconstructed via `WordVal::from_u64` on the far side.
    Word {
        width: u32,
        bits: u64,
    },
    /// A channel handle (a `Pipe`) — an opaque scheduler token. `Send` (just an
    /// id), so a channel can be passed as a spawn argument across worker threads;
    /// the receiving task resolves it against the one shared scheduler.
    Chan(crate::channel::ChanId),
    /// A spawned-task handle — likewise an opaque `Send` scheduler token.
    TaskHandle(crate::task::TaskId),
    /// A remote-peer handle — its canonical relay topic. A `String` is trivially
    /// `Send`, so a peer can be passed as a spawn argument across worker threads.
    Peer(String),
    /// A CRDT value carried as its KIND-TAGGED full convergent STATE (the bytes
    /// `CrdtValue::delta_since_bytes(&VClock::new())` produces — a valid state-based delta). Unlike
    /// the other variants, a CRDT does NOT move its `Rc`/`Arc` handle across the boundary (that would
    /// alias one replica's live cell); it ships its STATE, and the far side rebuilds a fresh replica
    /// and merges. State-based ⇒ re-applying the same bytes is IDEMPOTENT (per-replica MAX), so a
    /// redelivered payload never double-counts — the property `Sync`/`Merge` over a lossy relay needs.
    Crdt(Vec<u8>),
    /// A real algebraic number (`kind` 0), a computable real (1) or an ordinal (2), as its
    /// canonical `logicaffeine_base::exact_codec` bytes.
    Exact { kind: u8, bytes: Vec<u8> },
    /// A hyperreal (`kind` 0) or an infinite integer (1) with the infinitesimal it is written in:
    /// the infinitesimal's name, the residue facts about its reciprocal, and the value, each as
    /// canonical bytes.
    Scaled {
        kind: u8,
        name: String,
        residues: Vec<u8>,
        bytes: Vec<u8>,
    },
    /// A value that depends on the residue of the infinite generator: the infinitesimal's name, the
    /// residue facts, the modulus, and each class's value (`None` for a class the facts rule out).
    Depends {
        name: String,
        residues: Vec<u8>,
        modulus: u64,
        branches: Vec<Option<RtPayload>>,
    },
    /// The answer of a search over every natural: its value, the cases it checked, how often it
    /// changed, and the certificate that settled it when one did.
    Converging {
        value: Box<RtPayload>,
        steps: u64,
        revisions: u64,
        certificate: Option<String>,
    },
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_runtime/payload/tests.rs"]
mod tests;
