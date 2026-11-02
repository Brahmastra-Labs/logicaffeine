//! Typed kernel verdicts.
//!
//! A `Reject` is a claim: the input is not a valid Lean environment. A `Decline` claims nothing:
//! the checker ran out of a resource or met a construct it does not implement. The two are never
//! confused, because a decline reported as a rejection would be a wrong answer on a valid proof.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailKind {
    /// The input is ill-formed or ill-typed.
    Reject,
    /// Resource exhaustion or an unsupported construct; no claim about validity.
    Decline,
    /// A speculative computation ran out of the budget its caller gave it. It never escapes the
    /// caller that set the budget, which falls back to the complete path; should it escape, it
    /// reads as a decline.
    Exhausted,
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub kind: FailKind,
    pub message: String,
    pub detail: Option<String>,
    pub declaration: Option<String>,
}

/// A [`Failure`] behind one pointer. Every step of checking returns a [`KResult`], almost always
/// `Ok`; boxed, the failure keeps `KResult<T>` as small as `T` plus a pointer, so results travel
/// in registers instead of through memory on every call.
#[derive(Clone)]
pub struct KernelFailure(Box<Failure>);

pub type KResult<T> = Result<T, KernelFailure>;

const _: () = assert!(
    std::mem::size_of::<KResult<()>>() == std::mem::size_of::<usize>(),
    "a unit result is one pointer"
);
const _: () = assert!(
    std::mem::size_of::<KResult<u32>>() <= 2 * std::mem::size_of::<usize>(),
    "an id result fits in two registers"
);

impl std::ops::Deref for KernelFailure {
    type Target = Failure;

    fn deref(&self) -> &Failure {
        &self.0
    }
}

impl std::ops::DerefMut for KernelFailure {
    fn deref_mut(&mut self) -> &mut Failure {
        &mut self.0
    }
}

impl fmt::Debug for KernelFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl KernelFailure {
    #[cold]
    pub fn reject(message: impl Into<String>) -> KernelFailure {
        KernelFailure(Box::new(Failure {
            kind: FailKind::Reject,
            message: message.into(),
            detail: None,
            declaration: None,
        }))
    }

    #[cold]
    pub fn resource(message: impl Into<String>) -> KernelFailure {
        KernelFailure(Box::new(Failure {
            kind: FailKind::Decline,
            message: message.into(),
            detail: None,
            declaration: None,
        }))
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> KernelFailure {
        self.detail = Some(detail.into());
        self
    }

    pub fn in_declaration(mut self, name: impl Into<String>) -> KernelFailure {
        if self.declaration.is_none() {
            self.declaration = Some(name.into());
        }
        self
    }

    #[cold]
    pub fn exhausted() -> KernelFailure {
        KernelFailure(Box::new(Failure {
            kind: FailKind::Exhausted,
            message: "speculation budget exhausted".into(),
            detail: None,
            declaration: None,
        }))
    }

    pub fn is_exhausted(&self) -> bool {
        self.kind == FailKind::Exhausted
    }

    pub fn is_reject(&self) -> bool {
        self.kind == FailKind::Reject
    }
}

impl fmt::Display for KernelFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verdict = match self.kind {
            FailKind::Reject => "rejected",
            FailKind::Decline | FailKind::Exhausted => "declined",
        };
        match &self.declaration {
            Some(d) => write!(f, "{verdict}: declaration '{d}': {}", self.message)?,
            None => write!(f, "{verdict}: {}", self.message)?,
        }
        if let Some(detail) = &self.detail {
            write!(f, "\n  {detail}")?;
        }
        Ok(())
    }
}
