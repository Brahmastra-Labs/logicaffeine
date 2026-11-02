//! A remainder by a constant of a value bounded below twice it (`Opt::ModWrap`).
//!
//! For `0 <= x < 2m`, `x % m` is `x - m` when `x >= m` and `x` otherwise, and for `0 <= x < m` it is `x`: a compare
//! and a subtract, or nothing, in place of the multiply-shift a remainder by a constant compiles to. The rule is
//! decided here, once, from the range proved for `x`: the Rust and C emitters each spell the result, and the VM
//! compiler marks the remainder it emits (`Op::MagicDivU`'s `wrap`) for the bytecode tiers.

/// How `x % m` reduces, given the range of `x`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModWrap {
    /// `0 <= x < m`: the remainder is `x`.
    Identity,
    /// `0 <= x < 2m`: the remainder is `x - m` when `x >= m`, else `x`.
    Once,
}

/// The reduction of `x % divisor` for an `x` within `range` (inclusive), or `None` when the remainder must divide:
/// a divisor that is not positive, a range that is unknown, reaches below zero, or reaches `2 * divisor`.
pub(crate) fn mod_wrap(divisor: i64, range: Option<(i128, i128)>) -> Option<ModWrap> {
    let (lo, hi) = range?;
    if divisor <= 0 || lo < 0 {
        return None;
    }
    let m = i128::from(divisor);
    if hi < m {
        Some(ModWrap::Identity)
    } else if hi < 2 * m {
        Some(ModWrap::Once)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/modwrap/tests.rs"]
mod tests;
