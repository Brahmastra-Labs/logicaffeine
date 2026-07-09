#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod crdt;
pub mod fmt;
pub mod indexing;
pub mod ops;
pub mod types;
pub mod wire;

// Re-export commonly used types
pub use crdt::{
    generate_replica_id, AddWins, DeltaBuffer, DeltaCrdt, Dot, DotContext, GCounter, LWWRegister,
    MVRegister, Merge, ORMap, ORSet, PNCounter, RemoveWins, ReplicaId, SetBias, VClock, RGA, YATA,
};
pub use types::{
    Bool, Byte, Char, FillClone, FxIndexMap, FxIndexSet, Int, LogosContains, LogosDenseI64Map, LogosDenseI64MapNoPresence,
    LogosDenseI64Set, LogosDivU64, LogosI32Map, LogosI32Set, LogosI64Map, LogosI64Set, LogosMap,
    LogosComplex, LogosDecimal, LogosInt, LogosModular, LogosMoney, LogosQuantity, LogosRational, Map, Nat, Real, Seq, Set, Text, Tuple,
    Unit, Value, LogosSeq, LogosUuid, IntoRate, set_rate, set_rates, to_currency,
    text_bytes, text_from_bytes,
};
pub use rustc_hash::{FxHashMap, FxHashSet};
pub use indexing::{LogosGetChar, LogosIndex, LogosIndexMut};
pub use ops::{logos_add_exact, logos_add_i64, logos_approx_eq, logos_cmp_i64_f64, logos_div_exact, logos_div_i128, logos_div_i64, logos_floordiv_exact, logos_i64_eq_f64, logos_i64_key_of_f64, logos_mul_exact, logos_mul_i64, logos_narrow_i128, logos_pow_exact, logos_rem_exact, logos_rem_i128, logos_rem_i64, logos_sub_exact, logos_sub_i64, logos_truthy, Truthy};
