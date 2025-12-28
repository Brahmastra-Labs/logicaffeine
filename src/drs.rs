use crate::context::Gender;
use crate::intern::Symbol;

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
        }
    }
}

#[derive(Debug, Clone)]
pub struct Referent {
    pub variable: Symbol,
    pub noun_class: Symbol,
    pub gender: Gender,
    pub source: ReferentSource,
    pub used_by_pronoun: bool,
}

impl Referent {
    pub fn new(variable: Symbol, noun_class: Symbol, gender: Gender, source: ReferentSource) -> Self {
        Self {
            variable,
            noun_class,
            gender,
            source,
            used_by_pronoun: false,
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

    pub fn introduce_referent(&mut self, variable: Symbol, noun_class: Symbol, gender: Gender) {
        let source = self.boxes[self.current_box]
            .box_type
            .map(|bt| bt.to_referent_source())
            .unwrap_or(ReferentSource::MainClause);

        let referent = Referent::new(variable, noun_class, gender, source);
        self.boxes[self.current_box].universe.push(referent);
    }

    pub fn introduce_proper_name(&mut self, variable: Symbol, name: Symbol, gender: Gender) {
        let referent = Referent::new(variable, name, gender, ReferentSource::ProperName);
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
        if let Some(bt) = target.box_type {
            match bt {
                BoxType::NegationScope | BoxType::Disjunct => {
                    // These boxes are NOT accessible from outside
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

    /// Resolve a pronoun by finding accessible referents matching gender
    pub fn resolve_pronoun(&mut self, from_box: usize, gender: Gender) -> Option<Symbol> {
        // Search current box and accessible ancestors/siblings
        let mut candidates = Vec::new();

        // Check all boxes for accessibility
        for (box_idx, drs_box) in self.boxes.iter().enumerate() {
            if self.is_accessible(box_idx, from_box) {
                for referent in &drs_box.universe {
                    let gender_match = gender == Gender::Unknown
                        || referent.gender == Gender::Unknown
                        || referent.gender == gender
                        || gender == Gender::Neuter; // "it" can refer to things

                    if gender_match {
                        candidates.push((box_idx, referent.variable));
                    }
                }
            }
        }

        // Return most recent (last) candidate
        if let Some((box_idx, var)) = candidates.last() {
            // Mark as used by pronoun
            let box_idx = *box_idx;
            let var = *var;
            for referent in &mut self.boxes[box_idx].universe {
                if referent.variable == var {
                    referent.used_by_pronoun = true;
                    return Some(var);
                }
            }
        }

        None
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
        drs.introduce_referent(x, farmer, Gender::Male);
        assert_eq!(drs.boxes[0].universe[0].source, ReferentSource::MainClause);

        // Enter conditional antecedent
        drs.enter_box(BoxType::ConditionalAntecedent);
        let y = interner.intern("y");
        let donkey = interner.intern("Donkey");
        drs.introduce_referent(y, donkey, Gender::Neuter);
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
        drs.introduce_referent(y, donkey, Gender::Neuter);
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
        drs.introduce_referent(x, farmer, Gender::Male);

        drs.enter_box(BoxType::ConditionalAntecedent);
        let y = interner.intern("y");
        let donkey = interner.intern("Donkey");
        drs.introduce_referent(y, donkey, Gender::Neuter);

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
        drs.introduce_referent(y, donkey, Gender::Neuter);

        // Resolve "it" - should find donkey
        let resolved = drs.resolve_pronoun(drs.current_box, Gender::Neuter);
        assert_eq!(resolved, Some(y));

        // Should be marked as used
        assert!(drs.boxes[1].universe[0].used_by_pronoun);
    }
}
