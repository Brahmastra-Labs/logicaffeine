//! Affine equalities among a function's integers: Karr's domain.
//!
//! The interval domain bounds each quantity alone, and the affine prover ([`super::affine`]) decides what a set of
//! linear facts implies. Neither finds the equalities a loop keeps among the quantities it changes together. A merge
//! advances `k` with one of `li` and `ri` on every pass, so `k = li + ri - 1` holds at every visit to its loop; a
//! counted build pushes once per pass, so `length(left) = i - 1`. These are what prove the merge's store in range,
//! and no guard states them.
//!
//! A state of this domain is an affine subspace of the space of tracked quantities, holding every state the program
//! can reach at that point; every linear form constant on it is an equality the program keeps there. An assignment
//! maps the subspace affinely (or frees the assigned quantity, for a value that is not affine), a merge of two paths
//! takes the affine hull of both, and a loop's header is the hull of its entry and its passes. A hull that grows
//! raises the dimension, so a loop reaches its fixpoint within one pass per tracked quantity (Karr, "Affine
//! relationships among variables of a program", 1976).
//!
//! The quantities are named by the affine prover's variable indices, so an equality goes into a proof as it is: an
//! integer scalar by its symbol ([`super::affine::var`]) and a collection's length by its own index
//! ([`super::affine::len_var`]). A subspace is kept in generator form, one point and a basis of directions in reduced
//! row echelon form, over exact rationals. A quantity whose every value is reachable is not tracked at all: its
//! column is projected away, so a state's size is the number of quantities something relates.

use logicaffeine_base::numeric::Rational;
use logicaffeine_kernel::lia::LinearExpr;

/// The most quantities a state relates. Past it, the state forgets everything rather than grow its cubic joins:
/// soundness never depends on an equality being known.
const MAX_DIMS: usize = 64;

/// An affine subspace of the tracked quantities' values, or the empty set. Two are equal when they hold the same
/// valuations, however their columns are ordered.
#[derive(Clone, Debug)]
pub(crate) struct AffineSpace {
    /// The tracked quantities, by prover variable index: column `c` of `point` and of every direction is `dims[c]`.
    dims: Vec<i64>,
    /// One reachable valuation of `dims`; `None` when nothing reaches this point.
    point: Option<Vec<Rational>>,
    /// A basis of the directions the subspace spans, in reduced row echelon form.
    dirs: Vec<Vec<Rational>>,
}

impl AffineSpace {
    /// Every valuation: nothing is known.
    pub(crate) fn top() -> Self {
        AffineSpace {
            dims: Vec::new(),
            point: Some(Vec::new()),
            dirs: Vec::new(),
        }
    }

    /// No valuation: this point is not reached.
    pub(crate) fn bottom() -> Self {
        AffineSpace {
            dims: Vec::new(),
            point: None,
            dirs: Vec::new(),
        }
    }

    pub(crate) fn is_bottom(&self) -> bool {
        self.point.is_none()
    }

    /// The quantities this state relates.
    pub(crate) fn tracked(&self) -> &[i64] {
        &self.dims
    }

    /// `dim := value`, the value an affine expression over prover variables; `None` for a value that is not affine,
    /// after which every value of `dim` is reachable. The expression reads the quantities before the assignment, so
    /// `x := x + 1` shifts every equality through `x`.
    pub(crate) fn assign(&mut self, dim: i64, value: Option<&LinearExpr>) {
        if self.is_bottom() {
            return;
        }
        let Some(value) = value else {
            self.forget(dim);
            return;
        };
        let target = self.column(dim);
        let terms: Vec<(usize, Rational)> = value
            .coefficients
            .iter()
            .map(|(v, c)| (self.column(*v), c.clone()))
            .collect();
        let linear = |row: &[Rational]| {
            terms.iter().fold(Rational::zero(), |acc, (col, c)| {
                acc.add(&c.mul(&row[*col]))
            })
        };
        let point = self.point.as_mut().expect("not bottom");
        point[target] = value.constant.add(&linear(point));
        for dir in &mut self.dirs {
            dir[target] = linear(dir);
        }
        self.normalize();
    }

    /// Every value of `dim` becomes reachable: the equalities through it are dropped, the others kept.
    pub(crate) fn forget(&mut self, dim: i64) {
        if self.is_bottom() || !self.dims.contains(&dim) {
            return;
        }
        let col = self.column(dim);
        let mut unit = vec![Rational::zero(); self.dims.len()];
        unit[col] = Rational::one();
        self.dirs.push(unit);
        self.normalize();
    }

    /// [`Self::forget`] every tracked quantity `pred` picks.
    pub(crate) fn forget_where(&mut self, pred: impl Fn(i64) -> bool) {
        let picked: Vec<i64> = self.dims.iter().copied().filter(|d| pred(*d)).collect();
        for d in picked {
            self.forget(d);
        }
    }

    /// The valuations that also satisfy `e = 0`: what is known once a path establishes the equality (a counted loop's
    /// exit value). A space no valuation of which satisfies it becomes bottom.
    pub(crate) fn meet_eq(&mut self, e: &LinearExpr) {
        if self.is_bottom() {
            return;
        }
        let terms: Vec<(usize, Rational)> = e
            .coefficients
            .iter()
            .map(|(v, c)| (self.column(*v), c.clone()))
            .collect();
        let dot = |row: &[Rational]| {
            terms.iter().fold(Rational::zero(), |acc, (col, c)| {
                acc.add(&c.mul(&row[*col]))
            })
        };
        let point = self.point.clone().expect("not bottom");
        let miss = e.constant.add(&dot(&point));
        let Some(at) = self.dirs.iter().position(|d| !dot(d).is_zero()) else {
            if !miss.is_zero() {
                *self = AffineSpace::bottom();
            }
            self.normalize();
            return;
        };
        let pivot_dir = self.dirs.remove(at);
        let along = dot(&pivot_dir);
        let t = miss.negated().div(&along).expect("nonzero");
        self.point = Some(
            point
                .iter()
                .zip(&pivot_dir)
                .map(|(x, d)| x.add(&t.mul(d)))
                .collect(),
        );
        for dir in &mut self.dirs {
            let f = dot(dir).div(&along).expect("nonzero");
            for (x, d) in dir.iter_mut().zip(&pivot_dir) {
                *x = x.sub(&f.mul(d));
            }
        }
        self.normalize();
    }

    /// Whether `e = 0` holds at every valuation (vacuously, when there is none).
    pub(crate) fn entails_zero(&self, e: &LinearExpr) -> bool {
        let Some(point) = &self.point else {
            return true;
        };
        let at = |row: &[Rational], dim: i64| match self.dims.iter().position(|&d| d == dim) {
            Some(c) => row[c].clone(),
            None => Rational::zero(),
        };
        if e.coefficients.keys().any(|d| !self.dims.contains(d)) {
            return false;
        }
        let dot = |row: &[Rational]| {
            e.coefficients.iter().fold(Rational::zero(), |acc, (d, c)| {
                acc.add(&c.mul(&at(row, *d)))
            })
        };
        e.constant.add(&dot(point)).is_zero() && self.dirs.iter().all(|d| dot(d).is_zero())
    }

    /// The smallest affine subspace holding both: what is known after either path.
    pub(crate) fn join(&self, other: &AffineSpace) -> AffineSpace {
        if self.is_bottom() {
            return other.clone();
        }
        if other.is_bottom() {
            return self.clone();
        }
        let (mut a, b) = self.aligned_pair(other);
        let (pa, pb) = (
            a.point.clone().expect("not bottom"),
            b.point.expect("not bottom"),
        );
        a.dirs.extend(b.dirs);
        a.dirs
            .push(pb.iter().zip(&pa).map(|(x, y)| x.sub(y)).collect());
        a.normalize();
        a
    }

    /// Whether every valuation of `self` is one of `other`'s: a loop header that holds its passes is a fixpoint.
    pub(crate) fn leq(&self, other: &AffineSpace) -> bool {
        if self.is_bottom() {
            return true;
        }
        if other.is_bottom() {
            return false;
        }
        let (a, o) = self.aligned_pair(other);
        let (pa, po) = (
            a.point.as_ref().expect("not bottom"),
            o.point.as_ref().expect("not bottom"),
        );
        let offset: Vec<Rational> = pa.iter().zip(po).map(|(x, y)| x.sub(y)).collect();
        in_span(&o.dirs, &offset) && a.dirs.iter().all(|d| in_span(&o.dirs, d))
    }

    /// The equalities every valuation satisfies, each as an expression equal to zero, spanning all of them. A state
    /// nothing reaches has none: a proof never rests on unreachability.
    pub(crate) fn equalities(&self) -> Vec<LinearExpr> {
        let Some(point) = &self.point else {
            return Vec::new();
        };
        let n = self.dims.len();
        let pivots: Vec<usize> = self
            .dirs
            .iter()
            .map(|r| pivot(r).expect("no zero rows"))
            .collect();
        let mut out = Vec::new();
        for free in (0..n).filter(|c| !pivots.contains(c)) {
            let mut coeff = vec![Rational::zero(); n];
            coeff[free] = Rational::one();
            for (row, &p) in self.dirs.iter().zip(&pivots) {
                coeff[p] = row[free].negated();
            }
            let value = coeff
                .iter()
                .zip(point)
                .fold(Rational::zero(), |acc, (c, x)| acc.add(&c.mul(x)));
            let mut e = LinearExpr::constant(value.negated());
            for (c, &d) in coeff.iter().zip(&self.dims) {
                if !c.is_zero() {
                    e = e.add(&LinearExpr::var(d).scale(c));
                }
            }
            out.push(e);
        }
        out
    }

    /// [`Self::equalities`] as substitutions: each `(d, v)` says dim `d` equals `v`, an affine expression over the
    /// space's pivot dims. A `d` is a free dim, which occurs in its own equality only and in no `v`, so every
    /// substitution applies at once and together they eliminate the equalities exactly.
    pub(crate) fn solved(&self) -> Vec<(i64, LinearExpr)> {
        let Some(point) = &self.point else {
            return Vec::new();
        };
        let n = self.dims.len();
        let pivots: Vec<usize> = self
            .dirs
            .iter()
            .map(|r| pivot(r).expect("no zero rows"))
            .collect();
        (0..n)
            .filter(|c| !pivots.contains(c))
            .map(|free| {
                // x_free = point[free] + Σ_rows row[free] * (x_pivot - point[pivot]).
                let mut value = LinearExpr::constant(point[free].clone());
                for (row, &p) in self.dirs.iter().zip(&pivots) {
                    let c = &row[free];
                    if !c.is_zero() {
                        value = value
                            .add(&LinearExpr::var(self.dims[p]).scale(c))
                            .sub(&LinearExpr::constant(c.mul(&point[p])));
                    }
                }
                (self.dims[free], value)
            })
            .collect()
    }

    /// The column of `dim`, tracking it (every value reachable) when it is not yet.
    fn column(&mut self, dim: i64) -> usize {
        if let Some(c) = self.dims.iter().position(|&d| d == dim) {
            return c;
        }
        self.dims.push(dim);
        if let Some(p) = &mut self.point {
            p.push(Rational::zero());
        }
        for dir in &mut self.dirs {
            dir.push(Rational::zero());
        }
        let mut unit = vec![Rational::zero(); self.dims.len()];
        unit[self.dims.len() - 1] = Rational::one();
        self.dirs.push(unit);
        self.dims.len() - 1
    }

    /// Both spaces over the same columns in the same order: each tracks the other's quantities too, every value of
    /// them reachable. Each is the same subspace of the larger space.
    fn aligned_pair(&self, other: &AffineSpace) -> (AffineSpace, AffineSpace) {
        let a = self.aligned_to(other);
        let b = other.aligned_to(&a);
        (a, b)
    }

    /// `self` with `other`'s quantities tracked too, each with every value reachable, columns in `other`'s order
    /// first. The same subspace of a larger space.
    fn aligned_to(&self, other: &AffineSpace) -> AffineSpace {
        let mut out = self.clone();
        for &d in &other.dims {
            out.column(d);
        }
        let order: Vec<i64> = other
            .dims
            .iter()
            .copied()
            .chain(out.dims.iter().copied().filter(|d| !other.dims.contains(d)))
            .collect();
        let perm: Vec<usize> = order
            .iter()
            .map(|d| out.dims.iter().position(|x| x == d).expect("tracked"))
            .collect();
        let permute =
            |row: &Vec<Rational>| perm.iter().map(|&c| row[c].clone()).collect::<Vec<_>>();
        out.point = out.point.as_ref().map(permute);
        out.dirs = out.dirs.iter().map(permute).collect();
        out.dims = order;
        reduce(&mut out.dirs);
        out
    }

    /// Restore the representation: directions in reduced row echelon form, and a quantity every value of which is
    /// reachable on its own (a unit direction) projected away.
    fn normalize(&mut self) {
        reduce(&mut self.dirs);
        loop {
            let free = self
                .dirs
                .iter()
                .position(|r| r.iter().filter(|x| !x.is_zero()).count() == 1);
            let Some(row) = free else { break };
            let col = pivot(&self.dirs[row]).expect("nonzero");
            self.dirs.remove(row);
            self.dims.remove(col);
            if let Some(p) = &mut self.point {
                p.remove(col);
            }
            for dir in &mut self.dirs {
                dir.remove(col);
            }
        }
        if self.dims.len() > MAX_DIMS {
            *self = AffineSpace::top();
        }
    }
}

impl PartialEq for AffineSpace {
    fn eq(&self, other: &Self) -> bool {
        self.leq(other) && other.leq(self)
    }
}

impl Eq for AffineSpace {}

/// The first nonzero column of `row`.
fn pivot(row: &[Rational]) -> Option<usize> {
    row.iter().position(|x| !x.is_zero())
}

/// Gauss–Jordan elimination: `rows` in reduced row echelon form, zero rows removed, sorted by pivot column.
fn reduce(rows: &mut Vec<Vec<Rational>>) {
    let width = rows.first().map_or(0, Vec::len);
    let mut r = 0;
    for col in 0..width {
        let Some(found) = (r..rows.len()).find(|&i| !rows[i][col].is_zero()) else {
            continue;
        };
        rows.swap(r, found);
        let inv = rows[r][col].recip().expect("nonzero pivot");
        rows[r] = rows[r].iter().map(|x| x.mul(&inv)).collect();
        for i in 0..rows.len() {
            if i != r && !rows[i][col].is_zero() {
                let f = rows[i][col].clone();
                let pivot_row = rows[r].clone();
                for (x, p) in rows[i].iter_mut().zip(&pivot_row) {
                    *x = x.sub(&f.mul(p));
                }
            }
        }
        r += 1;
    }
    rows.truncate(r);
}

/// Whether `v` lies in the span of `rows`, a basis in reduced row echelon form.
fn in_span(rows: &[Vec<Rational>], v: &[Rational]) -> bool {
    let mut rest = v.to_vec();
    for row in rows {
        let p = pivot(row).expect("no zero rows");
        if !rest[p].is_zero() {
            let f = rest[p].clone();
            for (x, r) in rest.iter_mut().zip(row) {
                *x = x.sub(&f.mul(r));
            }
        }
    }
    rest.iter().all(Rational::is_zero)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/karr/tests.rs"]
mod tests;
