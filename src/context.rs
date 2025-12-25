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

#[derive(Debug, Clone)]
pub struct Entity {
    pub symbol: String,
    pub gender: Gender,
    pub number: Number,
    pub noun_class: String,
    pub ownership: OwnershipState,
}

#[derive(Debug, Clone, Default)]
pub struct DiscourseContext {
    history: Vec<Entity>,
    event_counter: usize,
    event_history: Vec<String>,
    reference_time_counter: usize,
    current_reference_time: Option<String>,
    time_constraints: Vec<TimeConstraint>,
}

impl DiscourseContext {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            event_counter: 0,
            event_history: Vec::new(),
            reference_time_counter: 0,
            current_reference_time: None,
            time_constraints: Vec::new(),
        }
    }

    pub fn next_reference_time(&mut self) -> String {
        self.reference_time_counter += 1;
        let var = format!("r{}", self.reference_time_counter);
        self.current_reference_time = Some(var.clone());
        var
    }

    pub fn current_reference_time(&self) -> String {
        self.current_reference_time.clone().unwrap_or_else(|| "S".to_string())
    }

    pub fn add_time_constraint(&mut self, left: String, relation: TimeRelation, right: String) {
        self.time_constraints.push(TimeConstraint { left, relation, right });
    }

    pub fn time_constraints(&self) -> &[TimeConstraint] {
        &self.time_constraints
    }

    pub fn clear_time_constraints(&mut self) {
        self.time_constraints.clear();
        self.reference_time_counter = 0;
        self.current_reference_time = None;
    }

    pub fn next_event_var(&mut self) -> String {
        self.event_counter += 1;
        let var = format!("e{}", self.event_counter);
        self.event_history.push(var.clone());
        var
    }

    pub fn event_history(&self) -> &[String] {
        &self.event_history
    }

    pub fn register(&mut self, entity: Entity) {
        self.history.push(entity);
    }

    pub fn resolve_pronoun(&self, gender: Gender, number: Number) -> Option<&Entity> {
        self.history
            .iter()
            .rev()
            .find(|e| {
                let gender_match = gender == Gender::Unknown
                    || e.gender == Gender::Unknown
                    || e.gender == gender;
                let number_match = e.number == number;
                gender_match && number_match
            })
    }

    pub fn resolve_definite(&self, noun_class: &str) -> Option<&Entity> {
        self.history
            .iter()
            .rev()
            .find(|e| e.noun_class.to_lowercase() == noun_class.to_lowercase())
    }

    pub fn has_entity_by_noun_class(&self, noun_class: &str) -> bool {
        self.history
            .iter()
            .any(|e| e.noun_class.to_lowercase() == noun_class.to_lowercase())
    }

    /// Resolve bridging anaphora by finding entities whose type contains the noun as a part.
    /// Returns all matching entities for ambiguity handling (parse forest).
    pub fn resolve_bridging(&self, noun_class: &str) -> Vec<(&Entity, &'static str)> {
        use crate::ontology::find_bridging_wholes;

        let Some(wholes) = find_bridging_wholes(noun_class) else {
            return Vec::new();
        };

        let mut matches = Vec::new();
        for whole in wholes {
            for entity in self.history.iter().rev() {
                if entity.noun_class.to_lowercase() == whole.to_lowercase() {
                    matches.push((entity, *whole));
                }
            }
        }
        matches
    }

    pub fn set_ownership(&mut self, noun_class: &str, state: OwnershipState) {
        for entity in self.history.iter_mut() {
            if entity.noun_class.to_lowercase() == noun_class.to_lowercase() {
                entity.ownership = state;
                return;
            }
        }
    }

    pub fn get_ownership(&self, noun_class: &str) -> Option<OwnershipState> {
        self.history
            .iter()
            .find(|e| e.noun_class.to_lowercase() == noun_class.to_lowercase())
            .map(|e| e.ownership)
    }

    pub fn clear(&mut self) {
        self.history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_resolve_male() {
        let mut ctx = DiscourseContext::new();
        ctx.register(Entity {
            symbol: "J".into(),
            gender: Gender::Male,
            number: Number::Singular,
            noun_class: "John".into(),
            ownership: OwnershipState::Owned,
        });
        let resolved = ctx.resolve_pronoun(Gender::Male, Number::Singular);
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().symbol, "J");
    }

    #[test]
    fn resolve_female_pronoun() {
        let mut ctx = DiscourseContext::new();
        ctx.register(Entity {
            symbol: "M".into(),
            gender: Gender::Female,
            number: Number::Singular,
            noun_class: "Mary".into(),
            ownership: OwnershipState::Owned,
        });
        let resolved = ctx.resolve_pronoun(Gender::Female, Number::Singular);
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().symbol, "M");
    }

    #[test]
    fn resolve_most_recent() {
        let mut ctx = DiscourseContext::new();
        ctx.register(Entity {
            symbol: "J".into(),
            gender: Gender::Male,
            number: Number::Singular,
            noun_class: "John".into(),
            ownership: OwnershipState::Owned,
        });
        ctx.register(Entity {
            symbol: "B".into(),
            gender: Gender::Male,
            number: Number::Singular,
            noun_class: "Bob".into(),
            ownership: OwnershipState::Owned,
        });
        let resolved = ctx.resolve_pronoun(Gender::Male, Number::Singular);
        assert_eq!(resolved.unwrap().symbol, "B");
    }

    #[test]
    fn resolve_definite_by_class() {
        let mut ctx = DiscourseContext::new();
        ctx.register(Entity {
            symbol: "D".into(),
            gender: Gender::Neuter,
            number: Number::Singular,
            noun_class: "Dog".into(),
            ownership: OwnershipState::Owned,
        });
        let resolved = ctx.resolve_definite("dog");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().symbol, "D");
    }

    #[test]
    fn gender_filtering() {
        let mut ctx = DiscourseContext::new();
        ctx.register(Entity {
            symbol: "J".into(),
            gender: Gender::Male,
            number: Number::Singular,
            noun_class: "John".into(),
            ownership: OwnershipState::Owned,
        });
        ctx.register(Entity {
            symbol: "M".into(),
            gender: Gender::Female,
            number: Number::Singular,
            noun_class: "Mary".into(),
            ownership: OwnershipState::Owned,
        });
        let he = ctx.resolve_pronoun(Gender::Male, Number::Singular);
        let she = ctx.resolve_pronoun(Gender::Female, Number::Singular);
        assert_eq!(he.unwrap().symbol, "J");
        assert_eq!(she.unwrap().symbol, "M");
    }
}
