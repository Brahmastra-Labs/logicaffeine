//! A `Seq of Int` that may outgrow the word: machine words until a value past the word is stored, then, once, every
//! element an exact [`LogosInt`].
//!
//! The generated Rust gives a list whose elements can leave the word this type instead of `Vec<i64>`. While it holds
//! words it is a `Vec<i64>` behind one discriminant, so a list that never overflows runs on its words; the first store
//! of a value past the word promotes it, and from then on it holds exact integers. Reads come in three forms: the word
//! a machine word can hold (`get_word`, narrowing a promoted element loudly), the word or nothing (`get_ck`, for a
//! checked chain that leaves the word path once the list has promoted), and the exact integer (`get_int`). Indexes are
//! 0-based; the generated code turns LOGOS's 1-based positions into them as it does for `Vec<i64>`.

use crate::types::LogosInt;

/// A list of `Int`: words, or, once promoted, exact integers.
#[derive(Clone, Debug)]
pub enum IntVec {
    Words(Vec<i64>),
    Exact(Vec<LogosInt>),
}

impl Default for IntVec {
    fn default() -> Self {
        IntVec::Words(Vec::new())
    }
}

impl IntVec {
    pub fn new() -> Self {
        IntVec::Words(Vec::new())
    }

    pub fn with_capacity(n: usize) -> Self {
        IntVec::Words(Vec::with_capacity(n))
    }

    pub fn from_words(words: Vec<i64>) -> Self {
        IntVec::Words(words)
    }

    /// The list of these exact integers, promoted only if one is past the word.
    pub fn from_ints(ints: Vec<LogosInt>) -> Self {
        if ints.iter().all(|v| v.to_i64().is_some()) {
            IntVec::Words(ints.iter().map(|v| v.to_i64().expect("checked")).collect())
        } else {
            IntVec::Exact(ints)
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            IntVec::Words(v) => v.len(),
            IntVec::Exact(v) => v.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the list has promoted.
    #[inline]
    pub fn is_exact(&self) -> bool {
        matches!(self, IntVec::Exact(_))
    }

    /// Element `i` where a machine word holds it: the word, or a promoted element narrowed, loudly past the word.
    #[inline]
    pub fn get_word(&self, i: usize) -> i64 {
        match self {
            IntVec::Words(v) => v[i],
            IntVec::Exact(v) => v[i].expect_i64("Int"),
        }
    }

    /// Element `i`, an index the caller has proven names one: the word, or a promoted element narrowed, loudly past
    /// the word.
    ///
    /// # Safety
    /// `i < self.len()`.
    #[inline]
    pub unsafe fn get_word_unchecked(&self, i: usize) -> i64 {
        match self {
            // SAFETY: the caller proves `i < self.len()`, the length of either form.
            IntVec::Words(v) => unsafe { *v.get_unchecked(i) },
            IntVec::Exact(v) => unsafe { v.get_unchecked(i) }.expect_i64("Int"),
        }
    }

    /// Element `i` as a leaf of a checked chain: the word while the list holds words, nothing once it has promoted.
    #[inline]
    pub fn get_ck(&self, i: usize) -> Option<i64> {
        match self {
            IntVec::Words(v) => Some(v[i]),
            IntVec::Exact(v) => {
                let _ = &v[i];
                None
            }
        }
    }

    /// Element `i` as the exact integer it is.
    #[inline]
    pub fn get_int(&self, i: usize) -> LogosInt {
        match self {
            IntVec::Words(v) => LogosInt::from_i64(v[i]),
            IntVec::Exact(v) => v[i].clone(),
        }
    }

    /// The exact integers of `words`, out of line: the list's one promotion.
    #[cold]
    #[inline(never)]
    fn exact_of(words: Vec<i64>) -> Vec<LogosInt> {
        words.iter().map(|w| LogosInt::from_i64(*w)).collect()
    }

    /// The list's exact integers, promoting it first if it holds words. The words move out and the exact integers
    /// move back by value, so a caller's list is never lent to an out-of-line call: its form and buffer stay in
    /// registers through a loop whose stores might promote it.
    #[inline]
    fn promote(&mut self) -> &mut Vec<LogosInt> {
        if let IntVec::Words(v) = self {
            let words = std::mem::take(v);
            *self = IntVec::Exact(Self::exact_of(words));
        }
        match self {
            IntVec::Exact(v) => v,
            IntVec::Words(_) => unreachable!("promoted above"),
        }
    }

    #[inline]
    pub fn push_word(&mut self, w: i64) {
        match self {
            IntVec::Words(v) => v.push(w),
            IntVec::Exact(v) => v.push(LogosInt::from_i64(w)),
        }
    }

    /// Append an exact integer: a word list keeps its words while the value fits one, and promotes when it does not.
    #[inline]
    pub fn push_int(&mut self, x: LogosInt) {
        match (self.is_exact(), x.to_i64()) {
            (false, Some(w)) => self.push_word(w),
            (true, _) => self.promote().push(x),
            (false, None) => self.promote().push(x),
        }
    }

    /// Append a value a width-bounded chain computed in 128 bits: a word while it fits one, else exact.
    #[inline]
    pub fn push_i128(&mut self, x: i128) {
        match (self.is_exact(), i64::try_from(x)) {
            (false, Ok(w)) => self.push_word(w),
            _ => self.promote().push(LogosInt::from_i128(x)),
        }
    }

    #[inline]
    pub fn set_word(&mut self, i: usize, w: i64) {
        match self {
            IntVec::Words(v) => v[i] = w,
            IntVec::Exact(v) => v[i] = LogosInt::from_i64(w),
        }
    }

    /// Replace element `i`, an index the caller has proven names one, with a machine word.
    ///
    /// # Safety
    /// `i < self.len()`.
    #[inline]
    pub unsafe fn set_word_unchecked(&mut self, i: usize, w: i64) {
        match self {
            // SAFETY: the caller proves `i < self.len()`, the length of either form.
            IntVec::Words(v) => unsafe { *v.get_unchecked_mut(i) = w },
            IntVec::Exact(v) => unsafe { *v.get_unchecked_mut(i) = LogosInt::from_i64(w) },
        }
    }

    /// Replace element `i`, an index the caller has proven names one, with a value computed in 128 bits, promoting the
    /// list when it does not fit a word.
    ///
    /// # Safety
    /// `i < self.len()`.
    #[inline]
    pub unsafe fn set_i128_unchecked(&mut self, i: usize, x: i128) {
        match (self.is_exact(), i64::try_from(x)) {
            // SAFETY: the caller proves `i < self.len()`.
            (false, Ok(w)) => unsafe { self.set_word_unchecked(i, w) },
            _ => self.promote()[i] = LogosInt::from_i128(x),
        }
    }

    /// Replace element `i`, an index the caller has proven names one, with an exact integer, promoting the list when
    /// it does not fit a word.
    ///
    /// # Safety
    /// `i < self.len()`.
    #[inline]
    pub unsafe fn set_int_unchecked(&mut self, i: usize, x: LogosInt) {
        match (self.is_exact(), x.to_i64()) {
            // SAFETY: the caller proves `i < self.len()`.
            (false, Some(w)) => unsafe { self.set_word_unchecked(i, w) },
            _ => self.promote()[i] = x,
        }
    }

    /// Replace element `i` with a value a width-bounded chain computed in 128 bits, promoting the list when it does not
    /// fit a word.
    #[inline]
    pub fn set_i128(&mut self, i: usize, x: i128) {
        match (self.is_exact(), i64::try_from(x)) {
            (false, Ok(w)) => self.set_word(i, w),
            _ => self.promote()[i] = LogosInt::from_i128(x),
        }
    }

    /// Replace element `i` with an exact integer, promoting the list when the value does not fit a word.
    #[inline]
    pub fn set_int(&mut self, i: usize, x: LogosInt) {
        match (self.is_exact(), x.to_i64()) {
            (false, Some(w)) => self.set_word(i, w),
            _ => {
                let v = self.promote();
                v[i] = x;
            }
        }
    }

    /// `item index of` the list as a machine word: 1-based from the front, counted from the back when negative, failing
    /// with every tier's sentence when it names no item.
    #[inline]
    pub fn item_word(&self, index: i64) -> i64 {
        self.get_word(crate::indexing::resolve_logos_index(index, self.len()))
    }

    /// The word `item index of` the list holds, to update in place, while the list holds words; `None` once it has
    /// promoted. The index resolves as [`IntVec::item_word`]'s does, failing with every tier's sentence where it names
    /// no item.
    #[inline]
    pub fn item_word_mut(&mut self, index: i64) -> Option<&mut i64> {
        let at = crate::indexing::resolve_logos_index(index, self.len());
        match self {
            IntVec::Words(v) => Some(&mut v[at]),
            IntVec::Exact(_) => None,
        }
    }

    /// The list's words, to read, while it holds words; `None` once it has promoted.
    #[inline]
    pub fn words(&self) -> Option<&Vec<i64>> {
        match self {
            IntVec::Words(v) => Some(v),
            IntVec::Exact(_) => None,
        }
    }

    /// The list's words, to read, write and push in place, while it holds words; `None` once it has promoted. One
    /// check of the form covers every access through the view, where each method call makes its own.
    #[inline]
    pub fn words_mut(&mut self) -> Option<&mut Vec<i64>> {
        match self {
            IntVec::Words(v) => Some(v),
            IntVec::Exact(_) => None,
        }
    }

    /// The word element `i` holds, to update in place, while the list holds words; `None` once it has promoted.
    ///
    /// # Safety
    /// `i < self.len()`.
    #[inline]
    pub unsafe fn word_mut_unchecked(&mut self, i: usize) -> Option<&mut i64> {
        match self {
            // SAFETY: the caller proves `i < self.len()`.
            IntVec::Words(v) => Some(unsafe { v.get_unchecked_mut(i) }),
            IntVec::Exact(_) => None,
        }
    }

    /// `item index of` the list as a leaf of a checked chain ([`IntVec::get_ck`]).
    #[inline]
    pub fn item_ck(&self, index: i64) -> Option<i64> {
        self.get_ck(crate::indexing::resolve_logos_index(index, self.len()))
    }

    /// `item index of` the list as the exact integer it is.
    #[inline]
    pub fn item_int(&self, index: i64) -> LogosInt {
        self.get_int(crate::indexing::resolve_logos_index(index, self.len()))
    }

    #[inline]
    pub fn set_item_word(&mut self, index: i64, w: i64) {
        let at = crate::indexing::resolve_logos_index(index, self.len());
        self.set_word(at, w);
    }

    #[inline]
    pub fn set_item_int(&mut self, index: i64, x: LogosInt) {
        let at = crate::indexing::resolve_logos_index(index, self.len());
        self.set_int(at, x);
    }

    /// Replace `item index of` the list with a value computed in 128 bits, promoting the list when it does not fit a
    /// word.
    #[inline]
    pub fn set_item_i128(&mut self, index: i64, x: i128) {
        let at = crate::indexing::resolve_logos_index(index, self.len());
        match (self.is_exact(), i64::try_from(x)) {
            (false, Ok(w)) => self.set_word(at, w),
            _ => self.promote()[at] = LogosInt::from_i128(x),
        }
    }

    /// Whether the list holds the machine word `w`.
    #[inline]
    pub fn contains_word(&self, w: i64) -> bool {
        match self {
            IntVec::Words(v) => v.contains(&w),
            IntVec::Exact(v) => v.iter().any(|x| x.to_i64() == Some(w)),
        }
    }

    /// Every element as the machine word it is, stopping loudly at an element past the word.
    #[inline]
    pub fn iter_word(&self) -> impl Iterator<Item = i64> + '_ {
        (0..self.len()).map(move |i| self.get_word(i))
    }

    /// Whether the list holds the exact integer `x`.
    pub fn contains_int(&self, x: &LogosInt) -> bool {
        match self {
            IntVec::Words(v) => x.to_i64().is_some_and(|w| v.contains(&w)),
            IntVec::Exact(v) => v.contains(x),
        }
    }

    /// Every element as the exact integer it is.
    pub fn iter_int(&self) -> impl Iterator<Item = LogosInt> + '_ {
        (0..self.len()).map(move |i| self.get_int(i))
    }
}

impl PartialEq for IntVec {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (IntVec::Words(a), IntVec::Words(b)) => a == b,
            _ => self.len() == other.len() && self.iter_int().eq(other.iter_int()),
        }
    }
}

impl Eq for IntVec {}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/int_vec/tests.rs"]
mod tests;
