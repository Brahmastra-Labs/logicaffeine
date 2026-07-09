#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod arena;
pub mod describe;
pub mod dimension;
pub mod hash;
pub mod intern;
pub mod money;
pub mod numeric;
pub mod quantity;
pub mod sha_ops;
pub mod span;
pub mod temporal;
pub mod uuid;
pub mod word;
pub mod error;
pub mod union_find;

pub use arena::Arena;
pub use dimension::{BaseDim, Dimension, Exp};
pub use quantity::{Quantity, Unit};
pub use intern::{Interner, Symbol, SymbolEq};
pub use money::{currency, Currency, Money, RateTable};
pub use numeric::{BigInt, Complex, Decimal, Modular, Rational, RoundingMode};
pub use span::Span;
pub use uuid::{Uuid, Variant};
pub use word::{
    Lanes16Word16, Lanes16Word8, Lanes4Word32, Lanes4Word64, Lanes8Word32, LanesVal, Word16, Word32,
    Word64, Word8, WordVal,
};
pub use error::{SpannedError, Result};
