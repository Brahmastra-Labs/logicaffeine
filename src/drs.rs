use crate::intern::Symbol;
use std::fmt;

// ============================================
// CORE DISCOURSE TYPES (moved from context.rs)
// ============================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeRelation {
    Precedes,
    Equals,
}

#[derive(Debug, Clone)]
pub struct TimeConstraint {
    pub left: String,
    pub relation: TimeRelation,
    pub right: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gender {
    Male,
    Female,
    Neuter,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Number {
    Singular,
    Plural,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    Subject,
    Object,
    Possessive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OwnershipState {
    #[default]
    Owned,
    Moved,
    Borrowed,
}

// ============================================
// SCOPE ERROR TYPES
// ============================================

/// Error when pronoun resolution fails due to scope constraints
#[derive(Debug, Clone, PartialEq)]
pub enum ScopeError {
    /// Referent exists but is trapped in an inaccessible scope
    InaccessibleReferent {
        gender: Gender,
        blocking_scope: BoxType,
        reason: String,
    },
    /// No matching referent found at all
    NoMatchingReferent {
        gender: Gender,
        number: Number,
    },
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeError::InaccessibleReferent { gender, blocking_scope, reason } => {
                write!(f, "Cannot resolve {:?} pronoun: referent is trapped in {:?} scope. {}",
                    gender, blocking_scope, reason)
            }
            ScopeError::NoMatchingReferent { gender, number } => {
                write!(f, "Cannot resolve {:?} {:?} pronoun: no matching referent in accessible scope",
                    gender, number)
            }
        }
    }
}

impl std::error::Error for ScopeError {}

// ============================================
// TELESCOPE SUPPORT
// ============================================

/// Path segment for navigating to insertion point during AST restructuring
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopePath {
    /// Enter body of ∀ or ∃ quantifier
    QuantifierBody,
    /// Enter consequent of → implication
    ImplicationRight,
    /// Enter right side of ∧ conjunction
    ConjunctionRight,
}

/// A referent that may be accessed via telescoping across sentence boundaries
#[derive(Debug, Clone)]
pub struct TelescopeCandidate {
    pub variable: Symbol,
    pub noun_class: Symbol,
    pub gender: Gender,
    /// The box index where this referent was introduced
    pub origin_box: usize,
    /// Path to navigate AST for scope extension
    pub scope_path: Vec<ScopePath>,
    /// Whether this referent was introduced in a modal scope
    pub in_modal_scope: bool,
}

// ============================================
// MODAL SUBORDINATION SUPPORT
// ============================================

/// Modal context for tracking hypothetical worlds across sentences.
/// Enables modal subordination: "A wolf might walk in. It would eat you."
#[derive(Debug, Clone)]
pub struct ModalContext {
    /// Whether we're currently inside a modal scope
    pub active: bool,
    /// The modal flavor (epistemic vs root)
    pub is_epistemic: bool,
    /// Force value (0.0 = impossibility, 1.0 = necessity)
    pub force: f32,
}

// ============================================
// WORLD STATE (Unified Discourse State)
// ============================================

/// The unified discourse state that persists across sentences.
#[derive(Debug, Clone)]
pub struct WorldState {
    /// The global DRS (box hierarchy for scope tracking)
    pub drs: Drs,
    /// Event variable counter (e1, e2, e3...)
    event_counter: usize,
    /// Event history for temporal ordering
    event_history: Vec<String>,
    /// Reference time counter (r1, r2, r3...)
    reference_time_counter: usize,
    /// Current reference time
    current_reference_time: Option<String>,
    /// Temporal constraints between events
    time_constraints: Vec<TimeConstraint>,
    /// Telescope candidates from previous sentence
    telescope_candidates: Vec<TelescopeCandidate>,
    /// Whether we're in discourse mode (processing multi-sentence discourse)
    /// When true, unresolved pronouns should error instead of deictic fallback
    discourse_mode: bool,
    /// Current modal context (if any) for tracking modal scope
    current_modal_context: Option<ModalContext>,
    /// Modal context from previous sentence for subordination
    prior_modal_context: Option<ModalContext>,
}

impl WorldState {
    pub fn new() -> Self {
        Self {
            drs: Drs::new(),
            event_counter: 0,
            event_history: Vec::new(),
            reference_time_counter: 0,
            current_reference_time: None,
            time_constraints: Vec::new(),
            telescope_candidates: Vec::new(),
            discourse_mode: false,
            current_modal_context: None,
            prior_modal_context: None,
        }
    }

    /// Generate next event variable (e1, e2, e3...)
    pub fn next_event_var(&mut self) -> String {
        self.event_counter += 1;
        let var = format!("e{}", self.event_counter);
        self.event_history.push(var.clone());
        var
    }

    /// Get event history for temporal ordering
    pub fn event_history(&self) -> &[String] {
        &self.event_history
    }

    /// Generate next reference time (r1, r2, r3...)
    pub fn next_reference_time(&mut self) -> String {
        self.reference_time_counter += 1;
        let var = format!("r{}", self.reference_time_counter);
        self.current_reference_time = Some(var.clone());
        var
    }

    /// Get current reference time
    pub fn current_reference_time(&self) -> String {
        self.current_reference_time.clone().unwrap_or_else(|| "S".to_string())
    }

    /// Add a temporal constraint
    pub fn add_time_constraint(&mut self, left: String, relation: TimeRelation, right: String) {
        self.time_constraints.push(TimeConstraint { left, relation, right });
    }

    /// Get all time constraints
    pub fn time_constraints(&self) -> &[TimeConstraint] {
        &self.time_constraints
    }

    /// Clear time constraints (for sentence boundary reset if needed)
    pub fn clear_time_constraints(&mut self) {
        self.time_constraints.clear();
        self.reference_time_counter = 0;
        self.current_reference_time = None;
    }

    /// Mark a sentence boundary - collect telescope candidates
    pub fn end_sentence(&mut self) {
        // Collect referents that can telescope from current DRS state
        let mut candidates = self.drs.get_telescope_candidates();

        // MODAL BARRIER: If this sentence had a modal, mark ALL its referents as modal-sourced.
        // This handles "A wolf might enter" where the wolf is introduced BEFORE we see "might".
        // The wolf should be marked as hypothetical even though it's in the main DRS box.
        if self.current_modal_context.is_some() {
            for candidate in &mut candidates {
                candidate.in_modal_scope = true;
            }
        }

        self.telescope_candidates = candidates;
        // Capture modal context for subordination in next sentence
        self.prior_modal_context = self.current_modal_context.take();
        // Mark that we're now in discourse mode (multi-sentence context)
        self.discourse_mode = true;
    }

    /// Check if we're in discourse mode (multi-sentence context)
    /// In discourse mode, unresolved pronouns should error instead of deictic fallback
    pub fn in_discourse_mode(&self) -> bool {
        self.discourse_mode
    }

    /// Get telescope candidates from previous sentence
    pub fn telescope_candidates(&self) -> &[TelescopeCandidate] {
        &self.telescope_candidates
    }

    /// Try to resolve a pronoun via telescoping
    pub fn resolve_via_telescope(&mut self, gender: Gender) -> Option<TelescopeCandidate> {
        // MODAL BARRIER: Check if we can access hypothetical entities
        // Reality (indicative) cannot see into imagination (modal scope)
        // Only modal subordination (e.g., "would" following "might") can access modal candidates
        let can_access_modal = self.in_modal_context();

        #[cfg(debug_assertions)]
        eprintln!("[TELESCOPE DEBUG] can_access_modal={}, candidates={:?}",
            can_access_modal,
            self.telescope_candidates.iter()
                .map(|c| (c.in_modal_scope, c.gender))
                .collect::<Vec<_>>()
        );

        // Apply same Gender Accommodation rules as resolve_pronoun:
        // - Exact match (Male=Male, Female=Female, etc)
        // - Unknown referent matches any pronoun (Gender Accommodation)
        // - Unknown pronoun matches any referent
        for candidate in &self.telescope_candidates {
            // MODAL BARRIER CHECK: Skip hypothetical entities when in reality mode
            if candidate.in_modal_scope && !can_access_modal {
                // Wolf in imagination cannot be referenced from reality
                #[cfg(debug_assertions)]
                eprintln!("[TELESCOPE DEBUG] BLOCKED modal candidate: {:?}", candidate.gender);
                continue;
            }

            let gender_match = candidate.gender == gender
                || candidate.gender == Gender::Unknown  // Gender Accommodation
                || gender == Gender::Unknown;

            if gender_match {
                return Some(candidate.clone());
            }
        }

        None
    }

    /// Set ownership state for a referent by noun class
    pub fn set_ownership(&mut self, noun_class: Symbol, state: OwnershipState) {
        self.drs.set_ownership(noun_class, state);
    }

    /// Get ownership state for a referent by noun class
    pub fn get_ownership(&self, noun_class: Symbol) -> Option<OwnershipState> {
        self.drs.get_ownership(noun_class)
    }

    /// Set ownership state for a referent by variable name
    pub fn set_ownership_by_var(&mut self, var: Symbol, state: OwnershipState) {
        self.drs.set_ownership_by_var(var, state);
    }

    /// Get ownership state for a referent by variable name
    pub fn get_ownership_by_var(&self, var: Symbol) -> Option<OwnershipState> {
        self.drs.get_ownership_by_var(var)
    }

    // ============================================
    // MODAL SUBORDINATION METHODS
    // ============================================

    /// Enter a modal context (e.g., "might", "would", "could")
    pub fn enter_modal_context(&mut self, is_epistemic: bool, force: f32) {
        self.current_modal_context = Some(ModalContext {
            active: true,
            is_epistemic,
            force,
        });
        // Also enter a modal box in the DRS
        self.drs.enter_box(BoxType::ModalScope);
    }

    /// Exit the current modal context
    pub fn exit_modal_context(&mut self) {
        self.current_modal_context = None;
        self.drs.exit_box();
    }

    /// Check if we're currently in a modal context
    pub fn in_modal_context(&self) -> bool {
        self.current_modal_context.is_some()
    }

    /// Check if there's a prior modal context for subordination
    pub fn has_prior_modal_context(&self) -> bool {
        self.prior_modal_context.is_some()
    }

    /// Check if current modal can subordinate to prior context
    /// "would" can continue a "might" world
    pub fn can_subordinate(&self) -> bool {
        self.prior_modal_context.is_some()
    }

    /// Clear the world state (reset for new discourse)
    pub fn clear(&mut self) {
        self.drs.clear();
        self.event_counter = 0;
        self.event_history.clear();
        self.reference_time_counter = 0;
        self.current_reference_time = None;
        self.time_constraints.clear();
        self.telescope_candidates.clear();
        self.discourse_mode = false;
        self.current_modal_context = None;
        self.prior_modal_context = None;
    }
}

impl Default for WorldState {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================
// REFERENT SOURCE
// ============================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferentSource {
    /// Indefinite in main clause - gets existential force
    MainClause,
    /// Proper name - no quantifier (constant)
    ProperName,
    /// Indefinite in conditional antecedent - gets universal force (DRS signature)
    ConditionalAntecedent,
    /// Indefinite in universal restrictor (relative clause) - gets universal force
    UniversalRestrictor,
    /// Inside negation scope - inaccessible outward
    NegationScope,
    /// Inside disjunction - inaccessible outward
    Disjunct,
    /// Inside modal scope - accessible via modal subordination
    ModalScope,
}

impl ReferentSource {
    pub fn gets_universal_force(&self) -> bool {
        matches!(
            self,
            ReferentSource::ConditionalAntecedent | ReferentSource::UniversalRestrictor
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxType {
    /// Top-level discourse box
    Main,
    /// Antecedent of conditional ("if" clause)
    ConditionalAntecedent,
    /// Consequent of conditional ("then" clause)
    ConditionalConsequent,
    /// Scope of negation
    NegationScope,
    /// Restrictor of universal quantifier (relative clause in "every X who...")
    UniversalRestrictor,
    /// Nuclear scope of universal quantifier
    UniversalScope,
    /// Branch of disjunction
    Disjunct,
    /// Scope of modal operator (might, would, could, etc.)
    /// Allows modal subordination: pronouns can access referents via telescoping
    ModalScope,
}

impl BoxType {
    pub fn to_referent_source(&self) -> ReferentSource {
        match self {
            BoxType::Main => ReferentSource::MainClause,
            BoxType::ConditionalAntecedent => ReferentSource::ConditionalAntecedent,
            BoxType::ConditionalConsequent => ReferentSource::MainClause,
            BoxType::NegationScope => ReferentSource::NegationScope,
            BoxType::UniversalRestrictor => ReferentSource::UniversalRestrictor,
            BoxType::UniversalScope => ReferentSource::MainClause,
            BoxType::Disjunct => ReferentSource::Disjunct,
            BoxType::ModalScope => ReferentSource::ModalScope,
        }
    }

    /// Can referents in this box be accessed via telescoping across sentence boundaries?
    /// Universal quantifiers, conditionals, and modals CAN telescope.
    /// Negation and disjunction CANNOT telescope.
    pub fn can_telescope(&self) -> bool {
        matches!(
            self,
            BoxType::Main
            | BoxType::UniversalScope
            | BoxType::UniversalRestrictor
            | BoxType::ConditionalConsequent
            | BoxType::ConditionalAntecedent
            | BoxType::ModalScope  // Modal subordination allows cross-sentence access
        )
        // NegationScope and Disjunct return false implicitly
    }

    /// Does this box type block accessibility from outside?
    pub fn blocks_accessibility(&self) -> bool {
        matches!(self, BoxType::NegationScope | BoxType::Disjunct)
    }
}

#[derive(Debug, Clone)]
pub struct Referent {
    pub variable: Symbol,
    pub noun_class: Symbol,
    pub gender: Gender,
    pub number: Number,
    pub source: ReferentSource,
    pub used_by_pronoun: bool,
    pub ownership: OwnershipState,
}

impl Referent {
    pub fn new(variable: Symbol, noun_class: Symbol, gender: Gender, number: Number, source: ReferentSource) -> Self {
        Self {
            variable,
            noun_class,
            gender,
            number,
            source,
            used_by_pronoun: false,
            ownership: OwnershipState::Owned,
        }
    }

    pub fn should_be_universal(&self) -> bool {
        self.source.gets_universal_force() || self.used_by_pronoun
    }
}

#[derive(Debug, Clone, Default)]
pub struct DrsBox {
    pub universe: Vec<Referent>,
    pub box_type: Option<BoxType>,
    pub parent: Option<usize>,
}

impl DrsBox {
    pub fn new(box_type: BoxType, parent: Option<usize>) -> Self {
        Self {
            universe: Vec::new(),
            box_type: Some(box_type),
            parent,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Drs {
    boxes: Vec<DrsBox>,
    main_box: usize,
    current_box: usize,
}

impl Drs {
    pub fn new() -> Self {
        let main = DrsBox::new(BoxType::Main, None);
        Self {
            boxes: vec![main],
            main_box: 0,
            current_box: 0,
        }
    }

    pub fn enter_box(&mut self, box_type: BoxType) -> usize {
        let parent = self.current_box;
        let new_box = DrsBox::new(box_type, Some(parent));
        let idx = self.boxes.len();
        self.boxes.push(new_box);
        self.current_box = idx;
        idx
    }

    pub fn exit_box(&mut self) {
        if let Some(parent) = self.boxes[self.current_box].parent {
            self.current_box = parent;
        }
    }

    pub fn current_box_index(&self) -> usize {
        self.current_box
    }

    pub fn current_box_type(&self) -> Option<BoxType> {
        self.boxes.get(self.current_box).and_then(|b| b.box_type)
    }

    pub fn introduce_referent(&mut self, variable: Symbol, noun_class: Symbol, gender: Gender, number: Number) {
        let source = self.boxes[self.current_box]
            .box_type
            .map(|bt| bt.to_referent_source())
            .unwrap_or(ReferentSource::MainClause);

        let referent = Referent::new(variable, noun_class, gender, number, source);
        self.boxes[self.current_box].universe.push(referent);
    }

    /// Introduce a referent with an explicit source (used for negative quantifiers like "No X")
    pub fn introduce_referent_with_source(&mut self, variable: Symbol, noun_class: Symbol, gender: Gender, number: Number, source: ReferentSource) {
        let referent = Referent::new(variable, noun_class, gender, number, source);
        self.boxes[self.current_box].universe.push(referent);
    }

    pub fn introduce_proper_name(&mut self, variable: Symbol, name: Symbol, gender: Gender) {
        // Proper names are always singular
        let referent = Referent::new(variable, name, gender, Number::Singular, ReferentSource::ProperName);
        self.boxes[self.current_box].universe.push(referent);
    }

    /// Check if a referent in box `from_box` can access referents in box `target_box`
    pub fn is_accessible(&self, target_box: usize, from_box: usize) -> bool {
        if target_box == from_box {
            return true;
        }

        let target = &self.boxes[target_box];
        let from = &self.boxes[from_box];

        // Check target box type - some boxes block outward access
        // The "accessibility" principle in DRT:
        // - Reality cannot see into hypotheticals (ModalScope)
        // - Affirmative cannot see into negative (NegationScope)
        // - One disjunct cannot see into another (Disjunct)
        if let Some(bt) = target.box_type {
            match bt {
                BoxType::NegationScope | BoxType::Disjunct | BoxType::ModalScope => {
                    // These boxes are NOT accessible from outside
                    // A wolf in imagination cannot be seen from reality
                    return false;
                }
                _ => {}
            }
        }

        // Check if from_box can see target_box
        // Consequent can see antecedent
        if let (Some(BoxType::ConditionalConsequent), Some(BoxType::ConditionalAntecedent)) =
            (from.box_type, target.box_type)
        {
            // Check if they share the same parent (same conditional)
            if from.parent == target.parent {
                return true;
            }
        }

        // Universal scope can see universal restrictor
        if let (Some(BoxType::UniversalScope), Some(BoxType::UniversalRestrictor)) =
            (from.box_type, target.box_type)
        {
            if from.parent == target.parent {
                return true;
            }
        }

        // Can always access ancestors (parent chain)
        let mut current = from_box;
        while let Some(parent) = self.boxes[current].parent {
            if parent == target_box {
                return true;
            }
            current = parent;
        }

        false
    }

    /// Resolve a pronoun by finding accessible referents matching gender and number
    pub fn resolve_pronoun(&mut self, from_box: usize, gender: Gender, number: Number) -> Result<Symbol, ScopeError> {
        // Phase 1: Search accessible referents
        // A referent is accessible if:
        //   - It's in an accessible box, OR
        //   - It has MainClause/ProperName source (globally accessible, e.g. definite descriptions)
        // Skip referents from NegationScope or Disjunct sources (always inaccessible)
        let mut candidates = Vec::new();

        for (box_idx, drs_box) in self.boxes.iter().enumerate() {
            let box_accessible = self.is_accessible(box_idx, from_box);

            for referent in &drs_box.universe {
                // Skip referents that are from negative quantifiers (No X) or disjuncts
                // Both are inaccessible outward per DRS accessibility
                if matches!(referent.source, ReferentSource::NegationScope | ReferentSource::Disjunct) {
                    continue;
                }

                // Check if this referent is accessible:
                // Either the box is accessible, or the referent has globally accessible source
                let has_global_source = matches!(referent.source, ReferentSource::MainClause | ReferentSource::ProperName);
                if !box_accessible && !has_global_source {
                    continue;
                }

                // Gender matching rules:
                // - Exact match (Male=Male, Female=Female, etc)
                // - Unknown referents match any pronoun (gender accommodation)
                // - Unknown pronouns match any referent
                // This allows "He" to refer to "farmer" even if farmer's gender is Unknown
                let gender_match = referent.gender == gender
                    || referent.gender == Gender::Unknown
                    || gender == Gender::Unknown;

                // Number matching: must match exactly (no number accommodation)
                let number_match = referent.number == number;

                if gender_match && number_match {
                    candidates.push((box_idx, referent.variable));
                }
            }
        }

        // If found in accessible scope, return success
        if let Some((box_idx, var)) = candidates.last() {
            let box_idx = *box_idx;
            let var = *var;
            for referent in &mut self.boxes[box_idx].universe {
                if referent.variable == var {
                    referent.used_by_pronoun = true;
                    return Ok(var);
                }
            }
        }

        // Phase 2: Check inaccessible boxes OR referents with NegationScope/Disjunct source
        // Use the same strict gender matching for consistency
        for (_box_idx, drs_box) in self.boxes.iter().enumerate() {
            for referent in &drs_box.universe {
                // Referents with MainClause or ProperName source are ALWAYS accessible
                // (definite descriptions presuppose existence and are globally accessible)
                if matches!(referent.source, ReferentSource::MainClause | ReferentSource::ProperName) {
                    continue;
                }

                // Check for referents with NegationScope/Disjunct source (from "No X" or disjuncts)
                // OR referents in inaccessible boxes
                let is_inaccessible = matches!(referent.source, ReferentSource::NegationScope | ReferentSource::Disjunct)
                    || !self.is_accessible(_box_idx, from_box);

                if is_inaccessible {
                    // Same matching as Phase 1
                    let gender_match = referent.gender == gender
                        || (gender == Gender::Unknown)
                        || (gender == Gender::Neuter && referent.gender == Gender::Unknown);
                    let number_match = referent.number == number;

                    if gender_match && number_match {
                        // Found a matching referent but it's inaccessible
                        let blocking_scope = if matches!(referent.source, ReferentSource::NegationScope) {
                            BoxType::NegationScope
                        } else if matches!(referent.source, ReferentSource::Disjunct) {
                            BoxType::Disjunct
                        } else {
                            drs_box.box_type.unwrap_or(BoxType::Main)
                        };
                        let noun_class_str = format!("{:?}", referent.noun_class);
                        return Err(ScopeError::InaccessibleReferent {
                            gender,
                            blocking_scope,
                            reason: format!("'{}' is trapped in {:?} scope and cannot be accessed",
                                noun_class_str, blocking_scope),
                        });
                    }
                }
            }
        }

        // Phase 3: Not found anywhere
        Err(ScopeError::NoMatchingReferent {
            gender,
            number,
        })
    }

    /// Resolve a definite description by finding accessible referent matching noun class
    pub fn resolve_definite(&self, from_box: usize, noun_class: Symbol) -> Option<Symbol> {
        for (box_idx, drs_box) in self.boxes.iter().enumerate() {
            if self.is_accessible(box_idx, from_box) {
                for referent in drs_box.universe.iter().rev() {
                    if referent.noun_class == noun_class {
                        return Some(referent.variable);
                    }
                }
            }
        }
        None
    }

    /// Check if a referent exists by variable name (for imperative mode variable validation)
    pub fn has_referent_by_variable(&self, var: Symbol) -> bool {
        for drs_box in &self.boxes {
            for referent in &drs_box.universe {
                if referent.variable == var {
                    return true;
                }
            }
        }
        false
    }

    /// Resolve bridging anaphora by finding referents whose type contains the noun as a part.
    /// Returns matching referent and whole name for PartOf relation.
    pub fn resolve_bridging(&self, interner: &crate::Interner, noun_class: Symbol) -> Option<(Symbol, &'static str)> {
        use crate::ontology::find_bridging_wholes;

        let noun_str = interner.resolve(noun_class);
        let Some(wholes) = find_bridging_wholes(noun_str) else {
            return None;
        };

        // Look for a referent whose noun_class matches one of the possible wholes
        for whole in wholes {
            for drs_box in &self.boxes {
                for referent in drs_box.universe.iter().rev() {
                    let ref_class_str = interner.resolve(referent.noun_class);
                    if ref_class_str.eq_ignore_ascii_case(whole) {
                        return Some((referent.variable, *whole));
                    }
                }
            }
        }
        None
    }

    /// Get all referents that should receive universal quantification
    pub fn get_universal_referents(&self) -> Vec<Symbol> {
        let mut result = Vec::new();
        for drs_box in &self.boxes {
            for referent in &drs_box.universe {
                if referent.should_be_universal() {
                    result.push(referent.variable);
                }
            }
        }
        result
    }

    /// Get all referents that should receive existential quantification
    pub fn get_existential_referents(&self) -> Vec<Symbol> {
        let mut result = Vec::new();
        for drs_box in &self.boxes {
            for referent in &drs_box.universe {
                if !referent.should_be_universal()
                    && !matches!(referent.source, ReferentSource::ProperName)
                {
                    result.push(referent.variable);
                }
            }
        }
        result
    }

    /// Get the most recent event referent (for binding weather adjectives to events)
    pub fn get_last_event_referent(&self, interner: &crate::intern::Interner) -> Option<Symbol> {
        // Search all boxes in reverse order for event referents
        for drs_box in self.boxes.iter().rev() {
            for referent in drs_box.universe.iter().rev() {
                let class_str = interner.resolve(referent.noun_class);
                if class_str == "Event" {
                    return Some(referent.variable);
                }
            }
        }
        None
    }

    /// Check if we're currently in a conditional antecedent
    pub fn in_conditional_antecedent(&self) -> bool {
        matches!(
            self.boxes.get(self.current_box).and_then(|b| b.box_type),
            Some(BoxType::ConditionalAntecedent)
        )
    }

    /// Check if we're currently in a universal restrictor
    pub fn in_universal_restrictor(&self) -> bool {
        matches!(
            self.boxes.get(self.current_box).and_then(|b| b.box_type),
            Some(BoxType::UniversalRestrictor)
        )
    }

    /// Get all referents that can telescope across sentence boundaries.
    /// Only includes referents from boxes where can_telescope() is true.
    /// Excludes referents blocked by negation or disjunction.
    pub fn get_telescope_candidates(&self) -> Vec<TelescopeCandidate> {
        let mut candidates = Vec::new();

        for (box_idx, drs_box) in self.boxes.iter().enumerate() {
            // Check if this box type allows telescoping
            if let Some(box_type) = drs_box.box_type {
                if !box_type.can_telescope() {
                    continue; // Skip negation and disjunction boxes
                }
            }

            // Check if this box is blocked by an ancestor negation/disjunction
            let mut is_blocked = false;
            let mut check_idx = box_idx;
            while let Some(parent_idx) = self.boxes.get(check_idx).and_then(|b| b.parent) {
                if let Some(parent_type) = self.boxes.get(parent_idx).and_then(|b| b.box_type) {
                    if parent_type.blocks_accessibility() {
                        is_blocked = true;
                        break;
                    }
                }
                check_idx = parent_idx;
            }

            if is_blocked {
                continue;
            }

            // Collect referents from this box (skip those with blocking sources)
            let is_modal_box = drs_box.box_type == Some(BoxType::ModalScope);
            for referent in &drs_box.universe {
                // Skip referents that are marked with NegationScope or Disjunct source
                // These are trapped inside negation/disjunction and cannot telescope
                if matches!(referent.source, ReferentSource::NegationScope | ReferentSource::Disjunct) {
                    continue;
                }

                candidates.push(TelescopeCandidate {
                    variable: referent.variable,
                    noun_class: referent.noun_class,
                    gender: referent.gender,
                    origin_box: box_idx,
                    scope_path: Vec::new(), // TODO: Track scope path during parsing
                    in_modal_scope: is_modal_box || referent.source == ReferentSource::ModalScope,
                });
            }
        }

        candidates
    }

    /// Find a referent that matches but is blocked by scope.
    /// Used to generate informative error messages.
    pub fn find_blocked_referent(&self, from_box: usize, gender: Gender) -> Option<(Symbol, BoxType)> {
        for (box_idx, drs_box) in self.boxes.iter().enumerate() {
            // Only check boxes that are NOT accessible
            if self.is_accessible(box_idx, from_box) {
                continue;
            }

            // Check if this box type blocks access
            if let Some(box_type) = drs_box.box_type {
                if box_type.blocks_accessibility() {
                    for referent in &drs_box.universe {
                        let gender_match = gender == Gender::Unknown
                            || referent.gender == Gender::Unknown
                            || referent.gender == gender
                            || gender == Gender::Neuter;

                        if gender_match {
                            return Some((referent.variable, box_type));
                        }
                    }
                }
            }
        }
        None
    }

    /// Set ownership state for a referent by noun class
    pub fn set_ownership(&mut self, noun_class: Symbol, state: OwnershipState) {
        for drs_box in &mut self.boxes {
            for referent in &mut drs_box.universe {
                if referent.noun_class == noun_class {
                    referent.ownership = state;
                    return;
                }
            }
        }
    }

    /// Set ownership state for a referent by variable name
    pub fn set_ownership_by_var(&mut self, var: Symbol, state: OwnershipState) {
        for drs_box in &mut self.boxes {
            for referent in &mut drs_box.universe {
                if referent.variable == var {
                    referent.ownership = state;
                    return;
                }
            }
        }
    }

    /// Get ownership state for a referent by noun class
    pub fn get_ownership(&self, noun_class: Symbol) -> Option<OwnershipState> {
        for drs_box in &self.boxes {
            for referent in &drs_box.universe {
                if referent.noun_class == noun_class {
                    return Some(referent.ownership);
                }
            }
        }
        None
    }

    /// Get ownership state for a referent by variable name
    pub fn get_ownership_by_var(&self, var: Symbol) -> Option<OwnershipState> {
        for drs_box in &self.boxes {
            for referent in &drs_box.universe {
                if referent.variable == var {
                    return Some(referent.ownership);
                }
            }
        }
        None
    }

    pub fn clear(&mut self) {
        self.boxes.clear();
        let main = DrsBox::new(BoxType::Main, None);
        self.boxes.push(main);
        self.main_box = 0;
        self.current_box = 0;
    }
}

impl Default for Drs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intern::Interner;

    #[test]
    fn referent_source_universal_force() {
        assert!(ReferentSource::ConditionalAntecedent.gets_universal_force());
        assert!(ReferentSource::UniversalRestrictor.gets_universal_force());
        assert!(!ReferentSource::MainClause.gets_universal_force());
        assert!(!ReferentSource::ProperName.gets_universal_force());
    }

    #[test]
    fn drs_new_has_main_box() {
        let drs = Drs::new();
        assert_eq!(drs.boxes.len(), 1);
        assert_eq!(drs.current_box, 0);
        assert_eq!(drs.boxes[0].box_type, Some(BoxType::Main));
    }

    #[test]
    fn drs_enter_exit_box() {
        let mut drs = Drs::new();
        assert_eq!(drs.current_box, 0);

        let ant_idx = drs.enter_box(BoxType::ConditionalAntecedent);
        assert_eq!(ant_idx, 1);
        assert_eq!(drs.current_box, 1);
        assert_eq!(drs.boxes[1].parent, Some(0));

        drs.exit_box();
        assert_eq!(drs.current_box, 0);
    }

    #[test]
    fn drs_introduce_referent_tracks_source() {
        let mut interner = Interner::new();
        let mut drs = Drs::new();

        let x = interner.intern("x");
        let farmer = interner.intern("Farmer");

        // In main box - should be MainClause
        drs.introduce_referent(x, farmer, Gender::Male, Number::Singular);
        assert_eq!(drs.boxes[0].universe[0].source, ReferentSource::MainClause);

        // Enter conditional antecedent
        drs.enter_box(BoxType::ConditionalAntecedent);
        let y = interner.intern("y");
        let donkey = interner.intern("Donkey");
        drs.introduce_referent(y, donkey, Gender::Neuter, Number::Singular);
        assert_eq!(
            drs.boxes[1].universe[0].source,
            ReferentSource::ConditionalAntecedent
        );
    }

    #[test]
    fn drs_conditional_antecedent_accessible_from_consequent() {
        let mut interner = Interner::new();
        let mut drs = Drs::new();

        // Enter conditional antecedent
        let ant_idx = drs.enter_box(BoxType::ConditionalAntecedent);
        let y = interner.intern("y");
        let donkey = interner.intern("Donkey");
        drs.introduce_referent(y, donkey, Gender::Neuter, Number::Singular);
        drs.exit_box();

        // Enter conditional consequent
        let cons_idx = drs.enter_box(BoxType::ConditionalConsequent);

        // Consequent should be able to access antecedent
        assert!(drs.is_accessible(ant_idx, cons_idx));
    }

    #[test]
    fn drs_negation_blocks_accessibility() {
        let mut drs = Drs::new();

        // Enter negation scope
        let neg_idx = drs.enter_box(BoxType::NegationScope);
        drs.exit_box();

        // Main box should NOT be able to access negation scope
        assert!(!drs.is_accessible(neg_idx, 0));
    }

    #[test]
    fn drs_get_universal_referents() {
        let mut interner = Interner::new();
        let mut drs = Drs::new();

        let x = interner.intern("x");
        let farmer = interner.intern("Farmer");
        drs.introduce_referent(x, farmer, Gender::Male, Number::Singular);

        drs.enter_box(BoxType::ConditionalAntecedent);
        let y = interner.intern("y");
        let donkey = interner.intern("Donkey");
        drs.introduce_referent(y, donkey, Gender::Neuter, Number::Singular);

        let universals = drs.get_universal_referents();
        assert_eq!(universals.len(), 1);
        assert_eq!(universals[0], y);
    }

    #[test]
    fn drs_pronoun_resolution_marks_used() {
        let mut interner = Interner::new();
        let mut drs = Drs::new();

        drs.enter_box(BoxType::UniversalRestrictor);
        let y = interner.intern("y");
        let donkey = interner.intern("Donkey");
        drs.introduce_referent(y, donkey, Gender::Neuter, Number::Singular);

        // Resolve "it" - should find donkey
        let resolved = drs.resolve_pronoun(drs.current_box, Gender::Neuter, Number::Singular);
        assert_eq!(resolved, Ok(y));

        // Should be marked as used
        assert!(drs.boxes[1].universe[0].used_by_pronoun);
    }
}
