//! Values at the end of a supertask: what depends on the infinite generator's residue, the limit
//! rules that turn such a value into one number, and the provisional answer of an unfinished
//! search.
//!
//! A program's infinitesimal is `dt = 1/H` for an infinite natural `H` the program never fixes. A
//! loop that runs `H` times can end in a state that depends on `H` itself — Thomson's lamp, toggled
//! `H` times, is on exactly when `H` is odd. Which residues `H` may take is all the program knows
//! (`Let dt be an infinitesimal whose reciprocal is even.`), recorded as a [`ResidueType`]: the
//! residues allowed modulo some modulus. In Presburger arithmetic the complete type of an infinite
//! natural is exactly its residue system — an element of the profinite integers — so a residue type
//! is the most a program can say about `H` without naming it.
//!
//! A [`Depends`] value is the same quantity on every residue class of `H`, held modulo the smallest
//! modulus that still separates its branches. When the residue type pins a single branch it is an
//! ordinary value; otherwise it prints its dependence (`0 when ⌈1/dt⌉ is even, 1 when it is odd`)
//! and a [`LimitRule`] resolves it: the limit superior or inferior (the extreme branch, the
//! infinite-time Turing machine's rule at a limit stage), the Cesàro mean (each admitted residue
//! weighted by its density), or the limit itself, which exists only when every branch agrees.
//!
//! A [`Converging`] value is the answer of a search no proof has settled: the current guess, how
//! many cases were checked and how often the guess changed — Shoenfield's limit lemma made honest.

use crate::algebraic::Algebraic;
use crate::numeric::BigInt;
use std::fmt;

/// The most residue classes one value may be split over: the wire's collection bound, so every
/// value a program holds can travel.
pub const MAX_MODULUS: u64 = crate::exact_codec::MAX_WIRE_LEN as u64;

/// Why a residue system cannot be formed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResidueError {
    /// No natural satisfies every fact.
    Contradiction,
    /// The value would split the generator over more than [`MAX_MODULUS`] classes.
    TooFine { classes: BigInt },
}

impl ResidueError {
    /// The refusal, with the generator written `generator`.
    pub fn describe(&self, generator: &str) -> String {
        match self {
            ResidueError::Contradiction => format!(
                "the facts about {generator} contradict each other: no natural satisfies them all"
            ),
            ResidueError::TooFine { classes } => format!(
                "{generator} would split over {classes} residue classes; one value holds at most 65,536"
            ),
        }
    }
}

impl fmt::Display for ResidueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe("the generator"))
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// `lcm(a, b)` when it is within [`MAX_MODULUS`].
pub(crate) fn bounded_lcm(a: u64, b: u64) -> Result<u64, ResidueError> {
    let l = u128::from(a / gcd(a, b)) * u128::from(b);
    bounded(l)
}

fn bounded(classes: u128) -> Result<u64, ResidueError> {
    if classes > u128::from(MAX_MODULUS) {
        let (hi, lo) = ((classes >> 64) as u64, classes as u64);
        return Err(ResidueError::TooFine {
            classes: BigInt::from_u64(hi).shl_bits(64).add(&BigInt::from_u64(lo)),
        });
    }
    Ok(classes as u64)
}

/// The residues the infinite generator may take. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResidueType {
    /// The smallest modulus that describes the allowed set.
    modulus: u64,
    /// `allowed[r]`: whether `H ≡ r (mod modulus)` is possible.
    allowed: Vec<bool>,
}

impl ResidueType {
    /// Nothing is known.
    pub fn any() -> ResidueType {
        ResidueType {
            modulus: 1,
            allowed: vec![true],
        }
    }

    /// `H ≡ r (mod m)`.
    ///
    /// # Panics
    ///
    /// When `m` is `0` or above [`MAX_MODULUS`]; a modulus a program supplies goes through
    /// [`ResidueType::from_allowed`], which refuses it instead.
    pub fn congruent(r: u64, m: u64) -> ResidueType {
        assert!(m > 0 && m <= MAX_MODULUS, "a modulus is in 1..=MAX_MODULUS");
        Self::canonical(m, (0..m).map(|x| x == r % m).collect())
    }

    fn canonical(m: u64, allowed: Vec<bool>) -> ResidueType {
        for d in (1..=m).filter(|d| m.is_multiple_of(*d)) {
            if (0..m as usize).all(|x| allowed[x] == allowed[x % d as usize]) {
                return ResidueType {
                    modulus: d,
                    allowed: allowed[..d as usize].to_vec(),
                };
            }
        }
        unreachable!("the modulus itself is a period")
    }

    pub fn modulus(&self) -> u64 {
        self.modulus
    }

    /// Whether `H = x` is consistent with the type.
    pub fn admits(&self, x: u64) -> bool {
        self.allowed[(x % self.modulus) as usize]
    }

    /// Whether some admitted `H` has `H ≡ r (mod m)`. By the Chinese remainder theorem that is
    /// an admitted class `s` with `s ≡ r` modulo `gcd(m, modulus)`, so any `m` is answered in time
    /// linear in the type's own modulus.
    pub fn allows(&self, r: u64, m: u64) -> bool {
        assert!(m > 0, "a modulus is positive");
        let g = gcd(m, self.modulus);
        self.residues().iter().any(|s| s % g == r % m % g)
    }

    /// Which classes modulo `g`, a divisor of the modulus, contain an admitted class.
    fn admitted_modulo(&self, g: u64) -> Vec<bool> {
        let mut out = vec![false; g as usize];
        for s in self.residues() {
            out[(s % g) as usize] = true;
        }
        out
    }

    /// The type allowing exactly `allowed` modulo `m` (residues reduce modulo `m`): a
    /// contradiction when nothing is allowed, too fine when `m` passes [`MAX_MODULUS`].
    pub fn from_allowed(m: u64, allowed: &[u64]) -> Result<ResidueType, ResidueError> {
        assert!(m > 0, "a modulus is positive");
        let m = bounded(u128::from(m))?;
        let mut set = vec![false; m as usize];
        for r in allowed {
            set[(r % m) as usize] = true;
        }
        if !set.iter().any(|&a| a) {
            return Err(ResidueError::Contradiction);
        }
        Ok(Self::canonical(m, set))
    }

    /// The allowed residues modulo [`ResidueType::modulus`], ascending.
    pub fn residues(&self) -> Vec<u64> {
        (0..self.modulus).filter(|&r| self.allowed[r as usize]).collect()
    }

    /// Both facts at once.
    pub fn and(&self, o: &ResidueType) -> Result<ResidueType, ResidueError> {
        let l = bounded_lcm(self.modulus, o.modulus)?;
        let allowed: Vec<bool> = (0..l).map(|x| self.admits(x) && o.admits(x)).collect();
        if !allowed.iter().any(|&a| a) {
            return Err(ResidueError::Contradiction);
        }
        Ok(Self::canonical(l, allowed))
    }
}

/// A quantity on every residue class of the generator. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Depends<T> {
    ty: ResidueType,
    modulus: u64,
    /// `branches[r]` for `H ≡ r (mod modulus)`; `None` for a class the type rules out.
    branches: Vec<Option<T>>,
}

impl<T: Clone + PartialEq> Depends<T> {
    pub fn constant(v: T, ty: &ResidueType) -> Depends<T> {
        Depends {
            ty: ty.clone(),
            modulus: 1,
            branches: vec![Some(v)],
        }
    }

    /// The quantity `f(H mod m)` on the classes the type admits; refused when `m` and the type's
    /// modulus together pass [`MAX_MODULUS`].
    pub fn from_fn(m: u64, ty: &ResidueType, f: impl Fn(u64) -> T) -> Result<Depends<T>, ResidueError> {
        assert!(m > 0, "a modulus is positive");
        let l = bounded_lcm(m, ty.modulus)?;
        let raw: Vec<Option<T>> = (0..l).map(|x| ty.admits(x).then(|| f(x % m))).collect();
        Ok(Self::collapse(ty.clone(), l, raw))
    }

    /// Hold `raw` (indexed by `H mod l`) modulo the smallest divisor of `l` on which every
    /// admitted class agrees.
    fn collapse(ty: ResidueType, l: u64, raw: Vec<Option<T>>) -> Depends<T> {
        for d in (1..=l).filter(|d| l.is_multiple_of(*d)) {
            let mut reduced: Vec<Option<T>> = vec![None; d as usize];
            let mut consistent = true;
            for (x, v) in raw.iter().enumerate() {
                let Some(v) = v else { continue };
                let slot = &mut reduced[x % d as usize];
                match slot {
                    Some(w) if w != v => {
                        consistent = false;
                        break;
                    }
                    Some(_) => {}
                    None => *slot = Some(v.clone()),
                }
            }
            if consistent {
                return Depends {
                    ty,
                    modulus: d,
                    branches: reduced,
                };
            }
        }
        unreachable!("the full modulus is always consistent")
    }

    /// Rebuild a quantity from its value on every class modulo `m` (`None` exactly for the classes
    /// `ty` rules out), held at its smallest modulus; `None` when the classes and the type disagree.
    pub fn from_branches(ty: ResidueType, m: u64, raw: Vec<Option<T>>) -> Option<Depends<T>> {
        if m == 0 || raw.len() as u64 != m || bounded_lcm(m, ty.modulus).is_err() {
            return None;
        }
        let g = gcd(m, ty.modulus);
        let admitted = ty.admitted_modulo(g);
        if raw.iter().enumerate().any(|(x, v)| v.is_some() != admitted[x % g as usize]) {
            return None;
        }
        Some(Self::collapse(ty, m, raw))
    }

    /// The value on every class modulo [`Depends::modulus`], `None` for a class the type rules out.
    pub fn raw_branches(&self) -> Vec<Option<T>> {
        self.branches.clone()
    }

    pub fn modulus(&self) -> u64 {
        self.modulus
    }

    pub fn residue_type(&self) -> &ResidueType {
        &self.ty
    }

    /// The value, when every admitted class has the same one.
    pub fn single(&self) -> Option<&T> {
        let mut values = self.branches.iter().flatten();
        let first = values.next()?;
        values.all(|v| v == first).then_some(first)
    }

    /// The value at `H = h`; `None` when the type rules `h` out.
    pub fn at(&self, h: u64) -> Option<&T> {
        if !self.ty.admits(h) {
            return None;
        }
        self.branches[(h % self.modulus) as usize].as_ref()
    }

    /// The admitted classes and their values, by residue.
    pub fn branches(&self) -> impl Iterator<Item = (u64, &T)> {
        self.branches
            .iter()
            .enumerate()
            .filter_map(|(r, v)| v.as_ref().map(|v| (r as u64, v)))
    }

    pub fn map<U: Clone + PartialEq>(&self, f: impl Fn(&T) -> U) -> Depends<U> {
        let raw = self.branches.iter().map(|v| v.as_ref().map(&f)).collect();
        Depends::collapse(self.ty.clone(), self.modulus, raw)
    }

    /// Combine two quantities branch by branch; refused when their classes together pass
    /// [`MAX_MODULUS`] or their types admit no common class.
    pub fn zip_with<U: Clone + PartialEq, V: Clone + PartialEq>(
        &self,
        o: &Depends<U>,
        f: impl Fn(&T, &U) -> V,
    ) -> Result<Depends<V>, ResidueError> {
        let l = bounded_lcm(self.modulus, o.modulus)?;
        let ty = self.ty.and(&o.ty)?;
        let l = bounded_lcm(l, ty.modulus)?;
        let raw = (0..l)
            .map(|x| {
                if !ty.admits(x) {
                    return None;
                }
                let a = self.branches[(x % self.modulus) as usize].as_ref()?;
                let b = o.branches[(x % o.modulus) as usize].as_ref()?;
                Some(f(a, b))
            })
            .collect();
        Ok(Depends::collapse(ty, l, raw))
    }

    /// `v` alone when it is single; otherwise the dependence on the generator, written `generator`:
    /// `0 when ⌈1/dt⌉ is even, 1 when it is odd`, or one clause per class modulo the modulus.
    pub fn render(&self, generator: &str, show: impl Fn(&T) -> String) -> String {
        if let Some(v) = self.single() {
            return show(v);
        }
        let classes: Vec<(u64, &T)> = self.branches().collect();
        if self.modulus == 2 && classes.len() == 2 {
            return format!(
                "{} when {generator} is even, {} when it is odd",
                show(classes[0].1),
                show(classes[1].1)
            );
        }
        classes
            .iter()
            .map(|(r, v)| format!("{} when {generator} ≡ {r} (mod {})", show(v), self.modulus))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// How a quantity that depends on the generator becomes one value at the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LimitRule {
    StandardPart,
    Limit,
    LimitSuperior,
    LimitInferior,
    CesaroMean,
}

impl Depends<Algebraic> {
    /// The value under `rule`, the generator written `⌈1/dt⌉` in an error.
    pub fn apply(&self, rule: LimitRule) -> Result<Algebraic, String> {
        self.apply_with(rule, "⌈1/dt⌉")
    }

    /// The value under `rule`; an error naming the dependence (the generator written `generator`)
    /// when the rule demands a limit that does not exist.
    pub fn apply_with(&self, rule: LimitRule, generator: &str) -> Result<Algebraic, String> {
        match rule {
            LimitRule::StandardPart | LimitRule::Limit => self.single().cloned().ok_or_else(|| {
                format!(
                    "no limit: the value is {}",
                    self.render(generator, |v| v.to_string())
                )
            }),
            LimitRule::LimitSuperior => Ok(self
                .branches()
                .map(|(_, v)| v.clone())
                .max()
                .expect("a residue type admits some class")),
            LimitRule::LimitInferior => Ok(self
                .branches()
                .map(|(_, v)| v.clone())
                .min()
                .expect("a residue type admits some class")),
            LimitRule::CesaroMean => {
                let l = bounded_lcm(self.modulus, self.ty.modulus)
                    .expect("a value is held within the bound of its own type");
                let (sum, count) = (0..l)
                    .filter(|&x| self.ty.admits(x))
                    .filter_map(|x| self.branches[(x % self.modulus) as usize].as_ref())
                    .fold((Algebraic::from_i64(0), 0i64), |(s, c), v| (s.add(v), c + 1));
                Ok(sum
                    .div(&Algebraic::from_i64(count))
                    .expect("a residue type admits some class"))
            }
        }
    }
}

/// The answer of a search: settled by a certificate, or provisional with its evidence.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Converging<T> {
    value: T,
    steps: u64,
    revisions: u64,
    certificate: Option<String>,
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

impl<T> Converging<T> {
    /// The current guess after `steps` cases, having changed `revisions` times, with no proof.
    pub fn provisional(value: T, steps: u64, revisions: u64) -> Converging<T> {
        Converging {
            value,
            steps,
            revisions,
            certificate: None,
        }
    }

    /// A value a certificate settles.
    pub fn settled(value: T, certificate: &str) -> Converging<T> {
        Converging {
            value,
            steps: 0,
            revisions: 0,
            certificate: Some(certificate.to_string()),
        }
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    /// The same evidence for `value`.
    pub fn with_value<U>(&self, value: U) -> Converging<U> {
        Converging {
            value,
            steps: self.steps,
            revisions: self.revisions,
            certificate: self.certificate.clone(),
        }
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn revisions(&self) -> u64 {
        self.revisions
    }

    pub fn is_settled(&self) -> bool {
        self.certificate.is_some()
    }

    pub fn certificate(&self) -> Option<&str> {
        self.certificate.as_deref()
    }

    /// The evidence of `value`, made from this answer and `other`: settled when both are (their
    /// certificates joined), the other's evidence when one is settled, and otherwise provisional on
    /// the fewest cases either search checked with every revision either made.
    pub fn joined<U, W>(&self, other: &Converging<U>, value: W) -> Converging<W> {
        match (&self.certificate, &other.certificate) {
            (Some(a), Some(b)) if a == b => Converging::settled(value, a),
            (Some(a), Some(b)) => Converging::settled(value, &format!("{a}; {b}")),
            (Some(_), None) => Converging::provisional(value, other.steps, other.revisions),
            (None, Some(_)) => Converging::provisional(value, self.steps, self.revisions),
            (None, None) => Converging::provisional(
                value,
                self.steps.min(other.steps),
                self.revisions.saturating_add(other.revisions),
            ),
        }
    }

    /// A settled value prints as itself; a provisional one carries its evidence:
    /// `false (provisional: checked 10,000,000 cases, 0 revisions; not proved)`.
    pub fn render(&self, show: impl Fn(&T) -> String) -> String {
        if self.is_settled() {
            return show(&self.value);
        }
        let revisions = if self.revisions == 1 { "revision" } else { "revisions" };
        format!(
            "{} (provisional: checked {} cases, {} {revisions}; not proved)",
            show(&self.value),
            thousands(self.steps),
            self.revisions
        )
    }
}

impl crate::exact_codec::ExactCodec for ResidueType {
    /// The modulus, then the allowed residues.
    fn write(&self, out: &mut Vec<u8>) {
        self.modulus.write(out);
        crate::exact_codec::write_list(&self.residues(), out);
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        let m = u64::read(buf, pos)?;
        if m == 0 || m > MAX_MODULUS {
            return None;
        }
        let allowed: Vec<u64> = crate::exact_codec::read_list(buf, pos)?;
        if allowed.iter().any(|r| *r >= m) {
            return None;
        }
        ResidueType::from_allowed(m, &allowed).ok()
    }
}

impl<T: crate::exact_codec::ExactCodec + Clone + PartialEq> crate::exact_codec::ExactCodec for Depends<T> {
    /// The residue type, the modulus, then each class's value (`0` for a class the type rules out,
    /// `1` and the value otherwise).
    fn write(&self, out: &mut Vec<u8>) {
        self.ty.write(out);
        self.modulus.write(out);
        for b in &self.branches {
            match b {
                None => out.push(0),
                Some(v) => {
                    out.push(1);
                    v.write(out);
                }
            }
        }
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        let ty = ResidueType::read(buf, pos)?;
        let m = u64::read(buf, pos)?;
        if m == 0 || m > MAX_MODULUS {
            return None;
        }
        let mut raw = Vec::with_capacity(m as usize);
        for _ in 0..m {
            raw.push(match crate::exact_codec::read_byte(buf, pos)? {
                0 => None,
                1 => Some(T::read(buf, pos)?),
                _ => return None,
            });
        }
        Depends::from_branches(ty, m, raw)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/limit_value/tests.rs"]
mod tests;
