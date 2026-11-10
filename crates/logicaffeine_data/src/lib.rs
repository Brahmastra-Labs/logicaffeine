#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

#[cfg(any(feature = "c-abi", test))]
#[cfg_attr(docsrs, doc(cfg(feature = "c-abi")))]
pub mod abi;
pub mod backpressure;
pub mod crdt;
pub mod exact;
pub mod failure;
pub mod fmt;
pub mod indexing;
pub mod ops;
pub mod opt;
pub mod temporal;
pub mod types;
pub mod wire;

// Re-export commonly used types
pub use crdt::{
    generate_replica_id, AddWins, DeltaBuffer, DeltaCrdt, Dot, DotContext, GCounter, LWWRegister,
    MVRegister, Merge, ORMap, ORSet, PNCounter, RemoveWins, ReplicaId, SetBias, VClock, RGA, YATA,
};
pub use failure::{contract_reason, ContractReason};
pub use indexing::{
    logos_offset, logos_run_before, logos_run_through, logos_slice_range, logos_text_run_before,
    logos_text_run_through, resolve_logos_index, LogosGetChar, LogosIndex, LogosIndexMut,
    LogosItems,
};
pub use ops::{
    logos_add_exact, logos_add_i64, logos_approx_eq, logos_band_exact, logos_cmp_i64_f64,
    logos_cold_path, logos_div_exact, logos_div_i128, logos_div_i64, logos_floordiv_exact,
    logos_i64_eq_f64, logos_i64_key_of_f64, logos_mul_exact, logos_mul_i64, logos_narrow_i128,
    logos_pow_exact, logos_rem_exact, logos_rem_i128, logos_rem_i64, logos_sub_exact,
    logos_sub_i64, logos_truthy, logos_word_operand, LogosStrPushInt, Truthy,
};
/// The language's `min` and `max` of two reals ([`logicaffeine_base::numeric::float_min`]), which a
/// compiled program calls where the interpreter's builtins call them.
pub use logicaffeine_base::numeric::{float_max as logos_max_f64, float_min as logos_min_f64};
pub use rustc_hash::{FxHashMap, FxHashSet};
pub use types::{
    decimal_of, set_rate, set_rates, text_bytes, text_from_bytes, to_currency, Bool, Boxable, Byte,
    Char, DecimalArg, DeepCopy, DeepCopyByClone, DeepCopyByFill, FillClone, FxIndexMap, FxIndexSet,
    Int, IntoRate, LogosBits, LogosBoxed,
    LogosComplex, LogosComplexF, LogosContains, LogosDecimal, LogosDenseI64Map,
    LogosDenseI64MapNoPresence, LogosDenseI64Set, LogosDivU64, LogosI32Map, LogosI32Set,
    LogosI64Map, LogosI64Set, LogosInt, LogosMap, LogosModular, LogosMoney, LogosQuantity,
    LogosRational, LogosSeq, LogosTuple, LogosUuid, Map, Nat, Real, Seq, Set, Text, ToRationals, Tuple,
    Unit, Value, VecCowNoop,
};
pub use exact::LogosValue;
pub use opt::LogosOpt;
