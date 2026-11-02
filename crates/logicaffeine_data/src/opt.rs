//! A value or `nothing`: what a compiled program keeps in a place the VM can store `nothing` in.
//!
//! The VM holds any value in any place. A compiled tier gives each place one type for its whole life, so a place a
//! program can store `nothing` in — a binding, an item, a field, a parameter, a result — holds a [`LogosOpt`]: the
//! value the place would hold anyway, or `nothing`. Reading it where a value is needed stops the program in the
//! words the VM stops with for that operation on `nothing`, which the compiler passes in.

/// A value of `T`, or `nothing`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LogosOpt<T>(pub Option<T>);

impl<T> LogosOpt<T> {
    /// `nothing`.
    pub const NOTHING: Self = LogosOpt(None);

    /// Whether it holds `nothing`.
    #[inline]
    pub fn is_nothing(&self) -> bool {
        self.0.is_none()
    }

    /// The value it holds. Holding `nothing`, the program stops with `failure`.
    #[inline]
    pub fn held(&self, failure: &str) -> &T {
        match &self.0 {
            Some(value) => value,
            None => stopped(failure),
        }
    }

    /// The value it holds, to change in place. Holding `nothing`, the program stops with `failure`.
    #[inline]
    pub fn held_mut(&mut self, failure: &str) -> &mut T {
        match &mut self.0 {
            Some(value) => value,
            None => stopped(failure),
        }
    }

    /// The value it holds, taken. Holding `nothing`, the program stops with `failure`.
    #[inline]
    pub fn into_held(self, failure: &str) -> T {
        match self.0 {
            Some(value) => value,
            None => stopped(failure),
        }
    }

    /// The values two operands hold, for an operation on both, each evaluated before either is read. Either holding
    /// `nothing`, the program stops with `failures`: the words for the left one alone, the right one alone, and both.
    #[inline]
    pub fn pair<U>(left: LogosOpt<T>, right: LogosOpt<U>, failures: [&str; 3]) -> (T, U) {
        match (left.0, right.0) {
            (Some(l), Some(r)) => (l, r),
            (None, Some(_)) => stopped(failures[0]),
            (Some(_), None) => stopped(failures[1]),
            (None, None) => stopped(failures[2]),
        }
    }

    /// Whether `self` and `other` are one value: both `nothing`, or two values `same` finds equal.
    #[inline]
    pub fn equals_with<U>(&self, other: &LogosOpt<U>, same: impl FnOnce(&T, &U) -> bool) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => same(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

/// A place starts holding its type's own first value, as the VM's does.
impl<T: Default> Default for LogosOpt<T> {
    #[inline]
    fn default() -> Self {
        LogosOpt(Some(T::default()))
    }
}

impl<T> From<T> for LogosOpt<T> {
    #[inline]
    fn from(value: T) -> Self {
        LogosOpt(Some(value))
    }
}

/// `nothing` is false where a condition reads it, and a value is as true as it is.
impl<T: crate::ops::Truthy> crate::ops::Truthy for LogosOpt<T> {
    #[inline]
    fn truthy(&self) -> bool {
        self.0.as_ref().is_some_and(crate::ops::Truthy::truthy)
    }
}

#[cold]
#[inline(never)]
fn stopped(failure: &str) -> ! {
    panic!("{failure}")
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/opt/tests.rs"]
mod tests;
