#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod algebraic;
pub mod arena;
pub mod closure;
pub mod cm;
pub mod complex_algebraic;
pub mod describe;
pub mod dimension;
pub mod elimination;
pub mod entropy;
pub mod error;
pub mod exact_codec;
pub mod exact_real;
pub mod expansion;
pub mod extension;
pub mod fec;
pub mod germ;
pub mod gf256;
pub mod hash;
pub mod hybrid;
pub mod hypernatural;
pub mod hyperreal;
pub mod intern;
mod interval;
pub mod limit_value;
pub mod matrix_exp;
pub mod money;
pub mod multi;
pub mod nested;
pub mod nested_flow;
pub mod number;
pub mod numeric;
pub mod ode;
pub mod ordered_field;
pub mod ordinal;
pub mod poly;
pub mod puiseux;
pub mod quantity;
pub mod sha_ops;
pub mod si;
pub mod span;
pub mod supertask;
pub mod temporal;
pub mod tower;
pub mod union_find;
pub mod uuid;
pub mod word;

pub use arena::Arena;
pub use dimension::{BaseDim, Dimension, Exp};
pub use error::{Result, SpannedError};
pub use fec::ReedSolomon;
pub use intern::{Interner, Symbol, SymbolEq};
pub use money::{currency, Currency, Money, RateTable};
pub use numeric::{BigInt, Complex, ComplexF, Decimal, Modular, Rational, RoundingMode};
pub use quantity::{Quantity, Unit};
pub use span::Span;
pub use uuid::{Uuid, Variant};
pub use word::{
    Lanes16Word16, Lanes16Word8, Lanes4Word32, Lanes4Word64, Lanes8Word32, LanesVal, Word16,
    Word32, Word64, Word8, WordVal,
};
