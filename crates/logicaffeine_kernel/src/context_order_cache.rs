//! Context-scoped storage for completed, issuer-separated recurrence certificates.
use super::*;
use std::collections::BTreeSet;
use std::sync::Weak;

const MAX_ENTRIES: usize = 64;
const MAX_NAME_BYTES: usize = 256;

/// Full conversion can inspect more registries than structural integer
/// arithmetic attestation. Weak owners detect COW and unique-owner make_mut
/// without retaining registry payloads or permitting allocation-address reuse.
#[derive(Debug)]
struct OrderGeneration {
    arithmetic: ArithmeticGeneration,
    structures: Weak<HashMap<String, StructInfo>>,
    quotient_primitives: Weak<HashMap<String, crate::quotient::Certificate>>,
    prelude_quotient: Option<Weak<crate::quotient::PreludeCertificate>>,
    observational: Option<Weak<crate::observational::Certificate>>,
    definition_deps: Weak<HashMap<String, Arc<[String]>>>,
    hints: Weak<Vec<String>>,
    // The following tokens conservatively preserve the complete semantic
    // snapshot even where current kernel normalization uses All transparency
    // or current checks read declaration data rather than admission receipts.
    mutual_blocks: Weak<HashMap<String, Vec<String>>>,
    reducibility: Weak<HashMap<String, Reducibility>>,
    simp_rules: Weak<Vec<SimpRule>>,
    simp_by_head: Weak<HashMap<String, Vec<usize>>>,
    checked_opaque: Weak<HashMap<String, crate::prelude::CheckedOpaqueDependencies>>,
    checked_opaque_graph: Weak<HashMap<String, Arc<BTreeSet<String>>>>,
    checked_opaque_dependents: Weak<HashMap<String, BTreeSet<String>>>,
}
impl OrderGeneration {
    fn of(ctx: &Context) -> Self {
        Self {
            arithmetic: ArithmeticGeneration::of(ctx),
            structures: Arc::downgrade(&ctx.structures),
            quotient_primitives: Arc::downgrade(&ctx.quotient_primitives),
            prelude_quotient: ctx.prelude_quotient.as_ref().map(Arc::downgrade),
            observational: ctx.observational.as_ref().map(Arc::downgrade),
            definition_deps: Arc::downgrade(&ctx.definition_deps),
            hints: Arc::downgrade(&ctx.hints),
            mutual_blocks: Arc::downgrade(&ctx.mutual_blocks),
            reducibility: Arc::downgrade(&ctx.reducibility),
            simp_rules: Arc::downgrade(&ctx.simp_rules),
            simp_by_head: Arc::downgrade(&ctx.simp_by_head),
            checked_opaque: Arc::downgrade(&ctx.checked_opaque),
            checked_opaque_graph: Arc::downgrade(&ctx.checked_opaque_graph),
            checked_opaque_dependents: Arc::downgrade(&ctx.checked_opaque_dependents),
        }
    }
    fn matches(&self, ctx: &Context) -> bool {
        self.arithmetic.matches(ctx)
            && self.structures.ptr_eq(&Arc::downgrade(&ctx.structures))
            && self
                .quotient_primitives
                .ptr_eq(&Arc::downgrade(&ctx.quotient_primitives))
            && match (&self.prelude_quotient, &ctx.prelude_quotient) {
                (None, None) => true,
                (Some(a), Some(b)) => a.ptr_eq(&Arc::downgrade(b)),
                _ => false,
            }
            && match (&self.observational, &ctx.observational) {
                (None, None) => true,
                (Some(a), Some(b)) => a.ptr_eq(&Arc::downgrade(b)),
                _ => false,
            }
            && self
                .definition_deps
                .ptr_eq(&Arc::downgrade(&ctx.definition_deps))
            && self.hints.ptr_eq(&Arc::downgrade(&ctx.hints))
            && self
                .mutual_blocks
                .ptr_eq(&Arc::downgrade(&ctx.mutual_blocks))
            && self.reducibility.ptr_eq(&Arc::downgrade(&ctx.reducibility))
            && self.simp_rules.ptr_eq(&Arc::downgrade(&ctx.simp_rules))
            && self.simp_by_head.ptr_eq(&Arc::downgrade(&ctx.simp_by_head))
            && self
                .checked_opaque
                .ptr_eq(&Arc::downgrade(&ctx.checked_opaque))
            && self
                .checked_opaque_graph
                .ptr_eq(&Arc::downgrade(&ctx.checked_opaque_graph))
            && self
                .checked_opaque_dependents
                .ptr_eq(&Arc::downgrade(&ctx.checked_opaque_dependents))
    }
}

// A main-kernel order certificate must NEVER satisfy the independent kernel's
// equality equations, even when operation and outcome names happen to coincide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CertificateDomain {
    MainOrder,
    IndependentEquality,
    MainDivision,
    IndependentDivision,
}
#[derive(Debug, PartialEq, Eq, Hash)]
struct CertificateKey {
    domain: CertificateDomain,
    operation: String,
}

#[derive(Debug)]
pub(super) struct OrderAttestation {
    generation: OrderGeneration,
    // Outcome order is [false, true], as certified by the original equations.
    // Names are re-interned in each fresh HeadReducer's own TermStore.
    results: HashMap<CertificateKey, [String; 2]>,
    // What each program's certification concluded, refusals included: a fresh evaluator's
    // certification is a function of the registry generation alone.
    programs: HashMap<String, Option<crate::reduction::CertifiedProgram>>,
    // Definitive refusals of an issuer that retains them: a certification that ran to its end
    // without a resource stop and certified nothing.
    refusals: std::collections::HashSet<CertificateKey>,
    // The independent checker's arithmetic recurrences by program, refusals included: the
    // operation's own name in that checker's spelling.
    independent_arithmetic: HashMap<String, Option<&'static str>>,
}

impl OrderAttestation {
    fn new(generation: OrderGeneration) -> Self {
        Self {
            generation,
            results: HashMap::new(),
            programs: HashMap::new(),
            refusals: std::collections::HashSet::new(),
            independent_arithmetic: HashMap::new(),
        }
    }
}

impl Context {
    /// Reuse only a completed positive recurrence certificate in its exact
    /// registry generation. None (including exhausted speculation) is not cached.
    pub(crate) fn cached_order_recurrence(
        &self,
        operation: &str,
        attest: impl FnOnce() -> Option<[String; 2]>,
    ) -> Option<[String; 2]> {
        self.cached_recurrence(CertificateDomain::MainOrder, operation, attest)
    }

    /// Reuse a completed division certificate (`[operation, pair constructor]`) in its exact
    /// registry generation, as [`Self::cached_order_recurrence`] does for orders.
    pub(crate) fn cached_division_recurrence(
        &self,
        program: &str,
        attest: impl FnOnce() -> Option<[String; 2]>,
    ) -> Option<[String; 2]> {
        self.cached_recurrence(CertificateDomain::MainDivision, program, attest)
    }

    /// Storage for certificates established by the independent de Bruijn checker.
    /// The callback must perform that checker's complete recurrence attestation;
    /// this method supplies no evidence on a miss and cannot read main results.
    pub(crate) fn cached_independent_equality_recurrence(
        &self,
        operation: &str,
        attest: impl FnOnce() -> Option<[String; 2]>,
    ) -> Option<[String; 2]> {
        self.cached_recurrence(CertificateDomain::IndependentEquality, operation, attest)
    }

    /// The independent de Bruijn checker's own division certificate for `program`
    /// (`[operation, pair constructor]`), refusals included: one certification per registry
    /// generation. `certify` also says whether its conclusion is definitive — a refusal reached
    /// after a resource stop is not, and is not retained. It never answers from the main kernel's
    /// division issuer.
    pub(crate) fn cached_independent_division(
        &self,
        program: &str,
        certify: impl FnOnce() -> (Option<[String; 2]>, bool),
    ) -> Option<[String; 2]> {
        if self.checking_order_recurrence || !self.simp_rules().is_empty() {
            return None;
        }
        if program.len() > MAX_NAME_BYTES {
            return certify().0;
        }
        let key = CertificateKey {
            domain: CertificateDomain::IndependentDivision,
            operation: program.to_owned(),
        };
        if let Ok(cache) = self.order_attestation.lock() {
            if let Some(entry) = cache
                .as_ref()
                .filter(|entry| entry.generation.matches(self))
            {
                if let Some(outcomes) = entry.results.get(&key) {
                    return Some(outcomes.clone());
                }
                if entry.refusals.contains(&key) {
                    return None;
                }
            }
        }
        let (outcomes, definitive) = certify();
        if outcomes
            .as_ref()
            .is_some_and(|o| o.iter().any(|name| name.len() > MAX_NAME_BYTES))
            || (outcomes.is_none() && !definitive)
        {
            return outcomes;
        }
        if let Ok(mut cache) = self.order_attestation.lock() {
            if !cache
                .as_ref()
                .is_some_and(|entry| entry.generation.matches(self))
            {
                *cache = Some(OrderAttestation::new(OrderGeneration::of(self)));
            }
            if let Some(entry) = cache.as_mut() {
                match &outcomes {
                    Some(outcomes) if entry.results.len() < MAX_ENTRIES => {
                        entry.results.entry(key).or_insert_with(|| outcomes.clone());
                    }
                    None if entry.refusals.len() < MAX_ENTRIES => {
                        entry.refusals.insert(key);
                    }
                    _ => {}
                }
            }
        }
        outcomes
    }

    /// The arithmetic recurrence the independent de Bruijn checker certifies for `program` (the
    /// operation, in that checker's own spelling), refusals included: one certification per
    /// registry generation. `certify` also says whether its conclusion is definitive — one reached
    /// after a resource stop is not, and is not retained. It never answers from the main kernel's
    /// certificates.
    pub(crate) fn cached_independent_arithmetic(
        &self,
        program: &str,
        certify: impl FnOnce() -> (Option<&'static str>, bool),
    ) -> Option<&'static str> {
        if self.checking_order_recurrence || !self.simp_rules().is_empty() {
            return None;
        }
        if program.len() > MAX_NAME_BYTES {
            return certify().0;
        }
        if let Ok(cache) = self.order_attestation.lock() {
            if let Some(entry) = cache
                .as_ref()
                .filter(|entry| entry.generation.matches(self))
            {
                if let Some(known) = entry.independent_arithmetic.get(program) {
                    return *known;
                }
            }
        }
        let (concluded, definitive) = certify();
        if concluded.is_none() && !definitive {
            return None;
        }
        if let Ok(mut cache) = self.order_attestation.lock() {
            if !cache
                .as_ref()
                .is_some_and(|entry| entry.generation.matches(self))
            {
                *cache = Some(OrderAttestation::new(OrderGeneration::of(self)));
            }
            if let Some(entry) = cache.as_mut() {
                if entry.independent_arithmetic.len() < MAX_ENTRIES * 4 {
                    entry
                        .independent_arithmetic
                        .entry(program.to_owned())
                        .or_insert(concluded);
                }
            }
        }
        concluded
    }

    /// What the main evaluator certifies `name` computes on numerals, by one certification per
    /// registry generation: its conclusion is reused until the registries move. `certify` also says
    /// whether its conclusion is definitive — a refusal reached after a resource stop is not, and is
    /// not retained.
    pub(crate) fn cached_certified_program(
        &self,
        name: &str,
        certify: impl FnOnce() -> (Option<crate::reduction::CertifiedProgram>, bool),
    ) -> Option<crate::reduction::CertifiedProgram> {
        if self.checking_order_recurrence || !self.simp_rules().is_empty() {
            return None;
        }
        if name.len() > MAX_NAME_BYTES {
            return certify().0;
        }
        if let Ok(cache) = self.order_attestation.lock() {
            if let Some(entry) = cache
                .as_ref()
                .filter(|entry| entry.generation.matches(self))
            {
                if let Some(known) = entry.programs.get(name) {
                    return known.clone();
                }
            }
        }
        let (concluded, definitive) = certify();
        if !definitive && concluded.is_none() {
            return None;
        }
        if let Ok(mut cache) = self.order_attestation.lock() {
            if !cache
                .as_ref()
                .is_some_and(|entry| entry.generation.matches(self))
            {
                *cache = Some(OrderAttestation::new(OrderGeneration::of(self)));
            }
            if let Some(entry) = cache.as_mut() {
                if entry.programs.len() < MAX_ENTRIES * 4 {
                    entry
                        .programs
                        .entry(name.to_owned())
                        .or_insert_with(|| concluded.clone());
                }
            }
        }
        concluded
    }

    fn cached_recurrence(
        &self,
        domain: CertificateDomain,
        operation: &str,
        attest: impl FnOnce() -> Option<[String; 2]>,
    ) -> Option<[String; 2]> {
        // These gates precede even a cache hit. Certification's recursive calls
        // cannot use the optimization whose equations they are establishing.
        if self.checking_order_recurrence || !self.simp_rules().is_empty() {
            return None;
        }
        if operation.len() > MAX_NAME_BYTES {
            return attest();
        }
        // One bounded name copy constructs the exact issuer-qualified lookup key.
        let key = CertificateKey {
            domain,
            operation: operation.to_owned(),
        };
        if let Ok(cache) = self.order_attestation.lock() {
            if let Some(entry) = cache
                .as_ref()
                .filter(|entry| entry.generation.matches(self))
            {
                if let Some(outcomes) = entry.results.get(&key) {
                    return Some(outcomes.clone());
                }
            }
        }
        // No mutex is held during typing, normalization, or recursive lookups.
        let outcomes = attest()?;
        if outcomes.iter().any(|name| name.len() > MAX_NAME_BYTES) {
            return Some(outcomes);
        }
        if let Ok(mut cache) = self.order_attestation.lock() {
            if !cache
                .as_ref()
                .is_some_and(|entry| entry.generation.matches(self))
            {
                *cache = Some(OrderAttestation::new(OrderGeneration::of(self)));
            }
            if let Some(entry) = cache.as_mut() {
                // A full cache keeps existing entries and returns this completed
                // certificate without retaining it. Admission is never a verdict.
                if entry.results.len() < MAX_ENTRIES {
                    entry.results.entry(key).or_insert_with(|| outcomes.clone());
                }
            }
        }
        Some(outcomes)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/context/order_cache/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/context/order_cache/independent_equality_cache_controls.rs"]
mod independent_equality_cache_controls;
