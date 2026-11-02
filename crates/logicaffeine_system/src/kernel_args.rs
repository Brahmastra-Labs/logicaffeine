//! The contract every argument of a standard-library native kernel meets before the kernel reads
//! it. A LOGOS program hands a kernel integers, and only some integers mean what the kernel takes:
//! an item of a byte string lies in 0..=255, a seed is exactly 32 bytes, an ML-KEM coefficient lies in
//! [0, q). A kernel refuses anything else with the reason and never folds it into a value it was not
//! given: `[256]` and `[0]` are different messages, and a 31-byte seed is not a 32-byte one. Every
//! tier reports the reason, after the kernel's name, as the program's failure.

use logicaffeine_base::Word16;
use logicaffeine_data::LogosSeq;

/// `s` as bytes, refusing the first item outside 0..=255.
pub(crate) fn bytes(what: &str, s: &[i64]) -> Result<Vec<u8>, String> {
    s.iter()
        .enumerate()
        .map(|(i, &x)| {
            u8::try_from(x)
                .map_err(|_| format!("the {what} must be bytes (0 to 255); item {} is {x}", i + 1))
        })
        .collect()
}

/// `s` as exactly `n` bytes.
pub(crate) fn exact_bytes(what: &str, s: &[i64], n: usize) -> Result<Vec<u8>, String> {
    let b = bytes(what, s)?;
    if b.len() == n {
        Ok(b)
    } else {
        Err(format!("the {what} is {} bytes; it must be {n}", b.len()))
    }
}

/// `s` as exactly `N` bytes.
pub(crate) fn byte_array<const N: usize>(what: &str, s: &[i64]) -> Result<[u8; N], String> {
    let b = exact_bytes(what, s, N)?;
    let mut a = [0u8; N];
    a.copy_from_slice(&b);
    Ok(a)
}

/// The integer `v`, which must lie in `lo..=hi`.
pub(crate) fn int_in(what: &str, v: i64, lo: i64, hi: i64) -> Result<i64, String> {
    if (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(format!("the {what} must be from {lo} to {hi}; it is {v}"))
    }
}

/// The integer `v`, which must not be negative.
pub(crate) fn non_negative(what: &str, v: i64) -> Result<usize, String> {
    usize::try_from(v).map_err(|_| format!("the {what} must not be negative; it is {v}"))
}

/// `len` items where the kernel takes exactly `n`.
pub(crate) fn count(what: &str, len: usize, n: usize) -> Result<(), String> {
    if len == n {
        Ok(())
    } else {
        Err(format!(
            "the {what} has {len} coefficients; it must have {n}"
        ))
    }
}

/// Two operands that must have the same number of coefficients.
pub(crate) fn same_count(what: &str, a: usize, b: usize) -> Result<(), String> {
    if a == b {
        Ok(())
    } else {
        Err(format!(
            "the {what} have {a} and {b} coefficients; they must match"
        ))
    }
}

/// Every item of `s` in `0..bound`.
pub(crate) fn ints_below(what: &str, s: &[i64], bound: i64) -> Result<(), String> {
    match s.iter().position(|&x| !(0..bound).contains(&x)) {
        None => Ok(()),
        Some(i) => Err(format!(
            "the {what} must hold values from 0 to {}; item {} is {}",
            bound - 1,
            i + 1,
            s[i]
        )),
    }
}

/// Every word of `s` below `bound`.
pub(crate) fn words_below(what: &str, s: &[Word16], bound: u16) -> Result<(), String> {
    match s.iter().position(|w| w.0 >= bound) {
        None => Ok(()),
        Some(i) => Err(format!(
            "the {what} must hold values from 0 to {}; item {} is {}",
            bound - 1,
            i + 1,
            s[i].0
        )),
    }
}

/// A byte string as the `Seq of Int` a LOGOS program reads.
pub(crate) fn seq_of_bytes(b: &[u8]) -> LogosSeq<i64> {
    LogosSeq::from_vec(b.iter().map(|&x| x as i64).collect())
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/kernel_args/tests.rs"]
mod tests;
