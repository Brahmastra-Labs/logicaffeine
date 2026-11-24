//! The runtime's word and lane functions (`logicaffeine_system::word_rt`), one row per builtin: the Rust function a
//! call compiles to, how it takes each argument, and the type it returns. The emitter writes a call
//! ([`WordCall::lower`]) and states the type of a binding of one ([`WordCall::result`]) from the same row; the unit
//! tests hold every row to the runtime's own signature.

use crate::semantics::builtins::{builtin_from_name, BuiltinId};

/// How a word function takes an argument.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Takes {
    /// By value: a word, a lane vector, an `i64`.
    Value,
    /// By reference: the list a lane vector is packed from (`&LogosSeq<…>`).
    List,
}

use Takes::{List, Value};

/// One word function of the runtime.
#[derive(Debug)]
pub(crate) struct WordCall {
    /// The builtin a program calls.
    pub(crate) builtin: BuiltinId,
    /// The `word_rt` function it compiles to.
    pub(crate) rust: &'static str,
    /// How the function takes each argument, in order.
    pub(crate) takes: &'static [Takes],
    /// The Rust type the function returns.
    pub(crate) result: &'static str,
}

const fn row(builtin: BuiltinId, rust: &'static str, takes: &'static [Takes], result: &'static str) -> WordCall {
    WordCall { builtin, rust, takes, result }
}

/// Every word function a call compiles to directly. `word32`/`word64` (whose literals fold) and the functions generic
/// over the word (`rotl`, `word_and`, …) are written elsewhere.
pub(crate) static WORD_CALLS: &[WordCall] = &[
    // Scalar words: build, read back, shift and mask at a fixed width.
    row(BuiltinId::Word8Make, "word8", &[Value], "Word8"),
    row(BuiltinId::Word16Make, "word16", &[Value], "Word16"),
    row(BuiltinId::IntOfWord8, "int_of_word8", &[Value], "i64"),
    row(BuiltinId::IntOfWord16, "int_of_word16", &[Value], "i64"),
    row(BuiltinId::IntOfWord32, "int_of_word32", &[Value], "i64"),
    row(BuiltinId::IntOfWord64, "int_of_word64", &[Value], "i64"),
    row(BuiltinId::Word32Shr, "word32_shr", &[Value, Value], "Word32"),
    row(BuiltinId::Word64Shl, "word64_shl", &[Value, Value], "Word64"),
    row(BuiltinId::Word64Shr, "word64_shr", &[Value, Value], "Word64"),
    row(BuiltinId::Word64And, "word64_and", &[Value, Value], "Word64"),
    // SHA-1 SHA-NI lane (`Lanes4Word32` = one `__m128i`): pack/unpack and the four SHA ops, which lower to the
    // `sha1rnds4`/`sha1msg1/2`/`sha1nexte` hardware instructions.
    row(BuiltinId::Lanes4Word32Make, "lanes4_word32", &[List], "Lanes4Word32"),
    row(BuiltinId::Lanes4Of, "lanes4_of", &[Value, Value, Value, Value], "Lanes4Word32"),
    row(BuiltinId::SeqOfLanes4W32, "seq_of_lanes4w32", &[Value], "LogosSeq<Word32>"),
    row(BuiltinId::Sha1Rnds4, "sha1rnds4", &[Value, Value, Value], "Lanes4Word32"),
    row(BuiltinId::Sha1Msg1, "sha1msg1", &[Value, Value], "Lanes4Word32"),
    row(BuiltinId::Sha1Msg2, "sha1msg2", &[Value, Value], "Lanes4Word32"),
    row(BuiltinId::Sha1Nexte, "sha1nexte", &[Value, Value], "Lanes4Word32"),
    // Byte-shuffle lane (`Lanes16Word8`): pshufb, per-byte shift and the interleaves the SIMD hex codec written in
    // LOGOS lowers to.
    row(BuiltinId::Lanes16Word8Make, "lanes16_word8", &[List], "Lanes16Word8"),
    row(BuiltinId::SeqOfLanes16W8, "seq_of_lanes16w8", &[Value], "LogosSeq<i64>"),
    row(BuiltinId::Splat16Word8, "splat16_word8", &[Value], "Lanes16Word8"),
    row(BuiltinId::Shuffle16, "shuffle16", &[Value, Value], "Lanes16Word8"),
    row(BuiltinId::ShrBytes16, "shr_bytes16", &[Value, Value], "Lanes16Word8"),
    row(BuiltinId::InterleaveLo16, "interleave_lo16", &[Value, Value], "Lanes16Word8"),
    row(BuiltinId::InterleaveHi16, "interleave_hi16", &[Value, Value], "Lanes16Word8"),
    row(BuiltinId::ByteAdd16, "byte_add16", &[Value, Value], "Lanes16Word8"),
    row(BuiltinId::Maddubs16, "maddubs16", &[Value, Value], "Lanes16Word8"),
    row(BuiltinId::Packus16, "packus16", &[Value, Value], "Lanes16Word8"),
    // Eight `Word32` lanes in one `__m256i`: pack a list, read its lanes back, splat, and the ML-DSA NTT's signed
    // Montgomery multiply.
    row(BuiltinId::Lanes8Word32, "lanes8_word32", &[List], "Lanes8Word32"),
    row(BuiltinId::SeqOfLanes8, "seq_of_lanes8", &[Value], "LogosSeq<Word32>"),
    row(BuiltinId::Splat8Word32, "splat8_word32", &[Value], "Lanes8Word32"),
    row(BuiltinId::Montmul32, "montmul32", &[Value, Value, Value, Value], "Lanes8Word32"),
    // Four `Word64` lanes in one `__m256i`.
    row(BuiltinId::Lanes4Word64, "lanes4_word64", &[List], "Lanes4Word64"),
    row(BuiltinId::SeqOfLanes4, "seq_of_lanes4", &[Value], "LogosSeq<i64>"),
    row(BuiltinId::Mul32x32To64, "mul32x32to64", &[Value, Value], "Lanes4Word64"),
    row(BuiltinId::HsumLanes4, "hsum_lanes4", &[Value], "i64"),
    row(BuiltinId::Splat4Word64, "splat4_word64", &[Value], "Lanes4Word64"),
    row(BuiltinId::AndNot4, "and_not4", &[Value, Value], "Lanes4Word64"),
    // Sixteen `Word16` lanes in one `__m256i`, the vector of the ML-KEM NTT.
    row(BuiltinId::Lanes16Word16, "lanes16_word16", &[List], "Lanes16Word16"),
    row(BuiltinId::SeqOfLanes16, "seq_of_lanes16", &[Value], "LogosSeq<i64>"),
    row(BuiltinId::Splat16Word16, "splat16_word16", &[Value], "Lanes16Word16"),
    row(BuiltinId::Mulhi16, "mulhi16", &[Value, Value], "Lanes16Word16"),
];

impl WordCall {
    /// The call over `args`, each already emitted: a list it packs is lent (`&(…)`), every other argument passed.
    pub(crate) fn lower(&self, args: &[String]) -> String {
        let args: Vec<String> = args
            .iter()
            .zip(self.takes)
            .map(|(arg, takes)| match takes {
                Value => arg.clone(),
                List => format!("&({arg})"),
            })
            .collect();
        format!("{}({})", self.rust, args.join(", "))
    }
}

/// The row of `builtin`.
pub(crate) fn of_builtin(builtin: BuiltinId) -> Option<&'static WordCall> {
    WORD_CALLS.iter().find(|call| call.builtin == builtin)
}

/// The row of the builtin a program calls as `name` with `arity` arguments.
pub(crate) fn by_name(name: &str, arity: usize) -> Option<&'static WordCall> {
    of_builtin(builtin_from_name(name)?).filter(|call| call.takes.len() == arity)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/word_calls/tests.rs"]
mod tests;
