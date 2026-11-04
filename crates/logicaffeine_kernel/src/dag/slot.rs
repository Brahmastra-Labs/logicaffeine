//! Reading the dag core's stores by the ids they hand out.
//!
//! Every id the core reads a store with was created by that store, as the index of the entry it
//! pushed, and stores only grow until the end of the scope their ids live in: an expression's
//! region (persistent, or one declaration's scratch), a value engine session. A hash table's probe
//! position is its hash's top bits or a successor masked to its power-of-two capacity. So every
//! read is in range. With the `unchecked-access` feature, which the Lean Kernel Arena entry builds
//! with because the Arena ranks checkers by the instructions they execute, a read skips the bounds
//! check; every other build keeps it, and debug builds assert the bound either way.

/// `v[i]`, for an index `v`'s store handed out or a probe position of its table.
#[inline(always)]
pub(crate) fn at<T>(v: &[T], i: usize) -> &T {
    debug_assert!(i < v.len(), "id {i} read from a store of {}", v.len());
    #[cfg(feature = "unchecked-access")]
    // SAFETY: `i` is an id this store handed out or a probe position of its table, in range (above).
    unsafe {
        v.get_unchecked(i)
    }
    #[cfg(not(feature = "unchecked-access"))]
    &v[i]
}

/// `&mut v[i]`, for an index `v`'s store handed out or a probe position of its table.
#[inline(always)]
pub(crate) fn at_mut<T>(v: &mut [T], i: usize) -> &mut T {
    debug_assert!(i < v.len(), "id {i} written to a store of {}", v.len());
    #[cfg(feature = "unchecked-access")]
    // SAFETY: as for `at`.
    unsafe {
        v.get_unchecked_mut(i)
    }
    #[cfg(not(feature = "unchecked-access"))]
    &mut v[i]
}

/// `&v[start..start + len]`, for a run of entries `v`'s store handed out.
#[inline(always)]
pub(crate) fn run<T>(v: &[T], start: usize, len: usize) -> &[T] {
    debug_assert!(
        start + len <= v.len(),
        "run {start}+{len} of a store of {}",
        v.len()
    );
    #[cfg(feature = "unchecked-access")]
    // SAFETY: the run was pushed whole by this store, so it lies inside it (as for `at`).
    unsafe {
        v.get_unchecked(start..start + len)
    }
    #[cfg(not(feature = "unchecked-access"))]
    &v[start..start + len]
}
