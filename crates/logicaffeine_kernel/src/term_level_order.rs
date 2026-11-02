//! Shared level ordering for ordinary conversion and caller-budgeted construction.
//! Parameters are rigid; imax witnesses are fresh in both expressions.
use crate::recursor::bounded::{GenerationResult, RecursorBudget, RecursorGenerationError};
use crate::Universe;

struct Level {
    depth: usize,
    kind: Kind,
}
enum Kind {
    Constant(u64),
    Variable(String),
    Succ(Box<Level>),
    Max(Box<Level>, Box<Level>),
    IMax(Box<Level>, Box<Level>),
}
fn resource() -> RecursorGenerationError {
    RecursorGenerationError::ResourceLimit
}
trait Accounting {
    const TRACK_DEPTH: bool;
    fn spend(&mut self, work: u64) -> GenerationResult<()>;
    fn reserve(&mut self, bytes: u64) -> GenerationResult<()>;
    fn inventory(&mut self, entries: usize) -> GenerationResult<()>;
    fn max_depth(&self) -> usize;
}
struct Unmetered;
impl Accounting for Unmetered {
    const TRACK_DEPTH: bool = false;
    fn spend(&mut self, _: u64) -> GenerationResult<()> {
        Ok(())
    }
    fn reserve(&mut self, _: u64) -> GenerationResult<()> {
        Ok(())
    }
    fn inventory(&mut self, _: usize) -> GenerationResult<()> {
        Ok(())
    }
    fn max_depth(&self) -> usize {
        usize::MAX
    }
}
impl Accounting for &mut RecursorBudget {
    const TRACK_DEPTH: bool = true;
    fn spend(&mut self, work: u64) -> GenerationResult<()> {
        RecursorBudget::spend(self, work)
    }
    fn reserve(&mut self, bytes: u64) -> GenerationResult<()> {
        RecursorBudget::reserve(self, bytes)
    }
    fn inventory(&mut self, entries: usize) -> GenerationResult<()> {
        RecursorBudget::inventory(self, entries)
    }
    fn max_depth(&self) -> usize {
        self.max_depth
    }
}
/// Borrowed from the ordering call, so recursive mutable Meter borrows do not own the counter.
/// Unmetered specialization returns None and compiles out all active-depth accounting.
struct Frame<'a>(Option<&'a std::cell::Cell<usize>>);
impl Drop for Frame<'_> {
    fn drop(&mut self) {
        if let Some(active) = self.0 {
            active.set(active.get() - 1);
        }
    }
}
struct Meter<'a, B> {
    budget: B,
    active: &'a std::cell::Cell<usize>,
}
impl<'meter, B: Accounting> Meter<'meter, B> {
    fn enter(&mut self, depth: usize) -> GenerationResult<Frame<'meter>> {
        self.budget.spend(1)?;
        if depth > self.budget.max_depth() {
            return Err(resource());
        }
        if B::TRACK_DEPTH {
            let next = self.active.get().checked_add(1).ok_or_else(resource)?;
            if next > self.budget.max_depth() {
                return Err(resource());
            }
            self.active.set(next);
            Ok(Frame(Some(self.active)))
        } else {
            Ok(Frame(None))
        }
    }
    fn text(&mut self, s: &str) -> GenerationResult<String> {
        let _frame = self.enter(1)?;
        self.budget.spend(s.len() as u64 + 1)?;
        self.budget.reserve(s.len() as u64 + 64)?;
        Ok(s.to_owned())
    }
    fn leaf(&mut self, kind: Kind) -> GenerationResult<Level> {
        let _frame = self.enter(1)?;
        self.budget.reserve(64)?;
        Ok(Level { depth: 1, kind })
    }
    fn unary(&mut self, a: Level) -> GenerationResult<Level> {
        let depth = a.depth.checked_add(1).ok_or_else(resource)?;
        let _frame = self.enter(depth)?;
        self.budget.reserve(128)?;
        Ok(Level {
            depth,
            kind: Kind::Succ(Box::new(a)),
        })
    }
    fn binary(&mut self, a: Level, b: Level, imax: bool) -> GenerationResult<Level> {
        let depth = a.depth.max(b.depth).checked_add(1).ok_or_else(resource)?;
        let _frame = self.enter(depth)?;
        self.budget.reserve(192)?;
        Ok(Level {
            depth,
            kind: if imax {
                Kind::IMax(Box::new(a), Box::new(b))
            } else {
                Kind::Max(Box::new(a), Box::new(b))
            },
        })
    }
    fn input(&mut self, u: &Universe, depth: usize) -> GenerationResult<Level> {
        let _frame = self.enter(depth)?;
        match u {
            Universe::SProp | Universe::Prop => self.leaf(Kind::Constant(0)),
            Universe::Type(n) => self.leaf(Kind::Constant(*n as u64 + 1)),
            Universe::Var(v) => {
                let v = self.text(v)?;
                self.leaf(Kind::Variable(v))
            }
            Universe::Succ(a) => {
                let a = self.input(a, depth + 1)?;
                self.unary(a)
            }
            Universe::Max(a, b) | Universe::IMax(a, b) => {
                let x = self.input(a, depth + 1)?;
                let y = self.input(b, depth + 1)?;
                self.binary(x, y, matches!(u, Universe::IMax(..)))
            }
        }
    }
    fn copy(&mut self, a: &Level, depth: usize) -> GenerationResult<Level> {
        let _frame = self.enter(depth)?;
        match &a.kind {
            Kind::Constant(n) => self.leaf(Kind::Constant(*n)),
            Kind::Variable(v) => {
                let v = self.text(v)?;
                self.leaf(Kind::Variable(v))
            }
            Kind::Succ(x) => {
                let x = self.copy(x, depth + 1)?;
                self.unary(x)
            }
            Kind::Max(x, y) | Kind::IMax(x, y) => {
                let x = self.copy(x, depth + 1)?;
                let y = self.copy(y, depth + 1)?;
                self.binary(x, y, matches!(&a.kind, Kind::IMax(..)))
            }
        }
    }
    // Normalize imax with the existing zero/positive and distribution rules.
    fn simplify(&mut self, a: Level, depth: usize) -> GenerationResult<Level> {
        let _frame = self.enter(depth)?;
        match a.kind {
            Kind::Constant(_) | Kind::Variable(_) => Ok(a),
            Kind::Succ(x) => {
                let x = self.simplify(*x, depth + 1)?;
                self.unary(x)
            }
            Kind::Max(x, y) => {
                let x = self.simplify(*x, depth + 1)?;
                let y = self.simplify(*y, depth + 1)?;
                self.binary(x, y, false)
            }
            Kind::IMax(x, y) => {
                let x = self.simplify(*x, depth + 1)?;
                let y = self.simplify(*y, depth + 1)?;
                match y.kind {
                    Kind::Constant(0) => self.leaf(Kind::Constant(0)),
                    Kind::Constant(_) | Kind::Succ(_) => self.binary(x, y, false),
                    Kind::Variable(_) => self.binary(x, y, true),
                    Kind::Max(y1, y2) => {
                        let copy = self.copy(&x, depth + 1)?;
                        let left = self.binary(copy, *y1, true)?;
                        let right = self.binary(x, *y2, true)?;
                        let both = self.binary(left, right, false)?;
                        self.simplify(both, depth + 1)
                    }
                    Kind::IMax(y1, y2) => {
                        let left = self.binary(x, *y1, false)?;
                        let next = self.binary(left, *y2, true)?;
                        self.simplify(next, depth + 1)
                    }
                }
            }
        }
    }
    fn pivot<'a>(&mut self, a: &'a Level, depth: usize) -> GenerationResult<Option<&'a str>> {
        let _frame = self.enter(depth)?;
        match &a.kind {
            Kind::Constant(_) | Kind::Variable(_) => Ok(None),
            Kind::Succ(a) => self.pivot(a, depth + 1),
            Kind::Max(a, b) => {
                if let Some(v) = self.pivot(a, depth + 1)? {
                    Ok(Some(v))
                } else {
                    self.pivot(b, depth + 1)
                }
            }
            Kind::IMax(a, b) => match &b.kind {
                Kind::Variable(v) => Ok(Some(v)),
                _ => {
                    if let Some(v) = self.pivot(a, depth + 1)? {
                        Ok(Some(v))
                    } else {
                        self.pivot(b, depth + 1)
                    }
                }
            },
        }
    }
    fn contains(&mut self, a: &Level, name: &str, depth: usize) -> GenerationResult<bool> {
        let _frame = self.enter(depth)?;
        match &a.kind {
            Kind::Constant(_) => Ok(false),
            Kind::Variable(v) => {
                self.budget.spend(v.len().min(name.len()) as u64 + 1)?;
                Ok(v == name)
            }
            Kind::Succ(a) => self.contains(a, name, depth + 1),
            Kind::Max(a, b) | Kind::IMax(a, b) => {
                Ok(self.contains(a, name, depth + 1)? || self.contains(b, name, depth + 1)?)
            }
        }
    }
    fn fresh(&mut self, v: &str, a: &Level, b: &Level) -> GenerationResult<String> {
        let _frame = self.enter(1)?;
        let mut candidate = self.text(v)?;
        loop {
            // Reserve full reallocation, including the old bytes, BEFORE push.
            self.budget.spend(candidate.len() as u64 + 4)?;
            self.budget
                .reserve((candidate.len() as u64 + 68).saturating_mul(2))?;
            candidate.push('✦');
            if !self.contains(a, &candidate, 1)? && !self.contains(b, &candidate, 1)? {
                return Ok(candidate);
            }
        }
    }
    fn substitute(
        &mut self,
        a: &Level,
        v: &str,
        replacement: &Level,
        depth: usize,
    ) -> GenerationResult<Level> {
        let _frame = self.enter(depth)?;
        match &a.kind {
            Kind::Constant(n) => self.leaf(Kind::Constant(*n)),
            Kind::Variable(x) => {
                self.budget.spend(x.len().min(v.len()) as u64 + 1)?;
                if x == v {
                    self.copy(replacement, depth + 1)
                } else {
                    let x = self.text(x)?;
                    self.leaf(Kind::Variable(x))
                }
            }
            Kind::Succ(x) => {
                let x = self.substitute(x, v, replacement, depth + 1)?;
                self.unary(x)
            }
            Kind::Max(x, y) | Kind::IMax(x, y) => {
                let x = self.substitute(x, v, replacement, depth + 1)?;
                let y = self.substitute(y, v, replacement, depth + 1)?;
                self.binary(x, y, matches!(&a.kind, Kind::IMax(..)))
            }
        }
    }
    fn atoms<'a>(
        &mut self,
        a: &'a Level,
        offset: u64,
        out: &mut Vec<(Option<&'a str>, u64)>,
        depth: usize,
    ) -> GenerationResult<()> {
        let _frame = self.enter(depth)?;
        match &a.kind {
            Kind::Constant(n) => {
                let n = n.checked_add(offset).ok_or_else(resource)?;
                self.budget.inventory(1)?;
                out.push((None, n));
            }
            Kind::Variable(v) => {
                self.budget.inventory(1)?;
                out.push((Some(v), offset));
            }
            Kind::Succ(a) => self.atoms(
                a,
                offset.checked_add(1).ok_or_else(resource)?,
                out,
                depth + 1,
            )?,
            Kind::Max(a, b) => {
                self.atoms(a, offset, out, depth + 1)?;
                self.atoms(b, offset, out, depth + 1)?;
            }
            Kind::IMax(..) => return Err(resource()), // unreachable after pivot elimination
        }
        Ok(())
    }
    fn linear(&mut self, a: &Level, b: &Level) -> GenerationResult<bool> {
        let _frame = self.enter(1)?;
        let mut left = Vec::new();
        let mut right = Vec::new();
        self.atoms(a, 0, &mut left, 1)?;
        self.atoms(b, 0, &mut right, 1)?;
        self.budget.spend(right.len() as u64)?;
        let minimum = right.iter().map(|(_, n)| *n).max().unwrap_or(0);
        for (name, offset) in left {
            self.budget.spend(1)?;
            let Some(name) = name else {
                if offset > minimum {
                    return Ok(false);
                }
                continue;
            };
            let mut found = false;
            for (other, n) in &right {
                self.budget.spend(1)?;
                if let Some(other) = other {
                    self.budget.spend(name.len().min(other.len()) as u64 + 1)?;
                    if name == *other && *n >= offset {
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn leq(&mut self, a: &Level, b: &Level, depth: usize) -> GenerationResult<bool> {
        let _frame = self.enter(depth)?;
        let pivot = match self.pivot(a, 1)? {
            Some(v) => Some(v),
            None => self.pivot(b, 1)?,
        };
        if let Some(v) = pivot {
            let zero = self.leaf(Kind::Constant(0))?;
            let a0 = self.substitute(a, v, &zero, 1)?;
            let a0 = self.simplify(a0, 1)?;
            let b0 = self.substitute(b, v, &zero, 1)?;
            let b0 = self.simplify(b0, 1)?;
            if !self.leq(&a0, &b0, depth + 1)? {
                return Ok(false);
            }
            let fresh = self.fresh(v, a, b)?;
            let fresh = self.leaf(Kind::Variable(fresh))?;
            let positive = self.unary(fresh)?;
            let ap = self.substitute(a, v, &positive, 1)?;
            let ap = self.simplify(ap, 1)?;
            let bp = self.substitute(b, v, &positive, 1)?;
            let bp = self.simplify(bp, 1)?;
            self.leq(&ap, &bp, depth + 1)
        } else {
            self.linear(a, b)
        }
    }
}

fn ordered<B: Accounting>(a: &Universe, b: &Universe, budget: B) -> GenerationResult<bool> {
    let active = std::cell::Cell::new(0);
    let mut m = Meter {
        budget,
        active: &active,
    };
    let _frame = m.enter(1)?;
    if matches!(a, Universe::SProp) {
        return Ok(true);
    }
    if matches!(b, Universe::SProp) {
        return Ok(false);
    }
    let a = m.input(a, 1)?;
    let a = m.simplify(a, 1)?;
    let b = m.input(b, 1)?;
    let b = m.simplify(b, 1)?;
    m.leq(&a, &b, 1)
}

/// Evaluate a closed level using the same natural-number model as `Meter`.
/// Explicit continuations avoid recursively rebuilding and simplifying deep
/// successor/max trees. Variables leave the symbolic decision procedure in charge.
fn closed_level(u: &Universe) -> Option<u64> {
    match u {
        Universe::SProp | Universe::Prop => return Some(0),
        Universe::Type(n) => return Some(*n as u64 + 1),
        Universe::Var(_) => return None,
        _ => {}
    }
    enum Step<'a> {
        Visit(&'a Universe),
        Succ,
        Join(bool),
    }
    let mut pending = vec![Step::Visit(u)];
    let mut values = Vec::<u64>::new();
    while let Some(step) = pending.pop() {
        match step {
            Step::Visit(u) => match u {
                Universe::SProp | Universe::Prop => values.push(0),
                Universe::Type(n) => values.push(*n as u64 + 1),
                Universe::Var(_) => return None,
                Universe::Succ(a) => {
                    pending.push(Step::Succ);
                    pending.push(Step::Visit(a));
                }
                Universe::Max(a, b) | Universe::IMax(a, b) => {
                    pending.push(Step::Join(matches!(u, Universe::IMax(..))));
                    pending.push(Step::Visit(b));
                    pending.push(Step::Visit(a));
                }
            },
            Step::Succ => {
                let value = values.pop()?.checked_add(1)?;
                values.push(value);
            }
            Step::Join(imax) => {
                let b = values.pop()?;
                let a = values.pop()?;
                values.push(if imax && b == 0 { 0 } else { a.max(b) });
            }
        }
    }
    values.pop()
}

/// Ordinary conversion has no construction budget; its public policy is unchanged.
pub(super) fn leq(a: &Universe, b: &Universe) -> bool {
    // SProp is distinguished at the outer ordering boundary, as in `ordered`.
    if matches!(a, Universe::SProp) {
        return true;
    }
    if matches!(b, Universe::SProp) {
        return false;
    }
    if let (Some(a), Some(b)) = (closed_level(a), closed_level(b)) {
        return a <= b;
    }
    ordered(a, b, Unmetered)
        .expect("a finite in-memory universe has representable depth and offsets")
}

/// Validate the depth door before stack allocation. The existing kernel envelope reserves
/// 2 MiB + 48 KiB per admitted active rung (including room for recursive tree destruction).
/// All owned temporary levels are created and dropped inside this sized-stack closure, on
/// success, resource error, and unwind. The original input universes remain borrowed.
fn with_level_stack<T: Send>(
    budget: &mut RecursorBudget,
    f: impl FnOnce(&mut RecursorBudget) -> GenerationResult<T> + Send,
) -> GenerationResult<T> {
    if budget.max_depth > crate::type_checker::MAX_TERM_DEPTH {
        return Err(resource());
    }
    crate::term_store::grow_stack_for_depth(budget.max_depth, || f(budget))
}

/// Resource exhaustion is distinct from inequality; the caller's budget is never reset.
pub(super) fn equivalent(
    a: &Universe,
    b: &Universe,
    budget: &mut RecursorBudget,
) -> GenerationResult<bool> {
    with_level_stack(budget, |budget| {
        Ok(ordered(a, b, &mut *budget)? && ordered(b, a, &mut *budget)?)
    })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term/level_order/depth_controls.rs"]
mod depth_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term/level_order/closed_level_controls.rs"]
mod closed_level_controls;
