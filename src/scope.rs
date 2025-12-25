use std::collections::HashMap;
use crate::context::OwnershipState;

#[derive(Debug, Clone)]
pub struct ScopeEntry {
    pub symbol: String,
    pub ownership: OwnershipState,
}

impl ScopeEntry {
    pub fn variable(name: &str) -> Self {
        Self {
            symbol: name.to_string(),
            ownership: OwnershipState::Owned,
        }
    }
}

#[derive(Debug, Default)]
pub struct ScopeStack {
    scopes: Vec<HashMap<String, ScopeEntry>>,
}

impl ScopeStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn bind(&mut self, name: &str, entry: ScopeEntry) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), entry);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&ScopeEntry> {
        for scope in self.scopes.iter().rev() {
            if let Some(entry) = scope.get(name) {
                return Some(entry);
            }
        }
        None
    }

    pub fn lookup_mut(&mut self, name: &str) -> Option<&mut ScopeEntry> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(entry) = scope.get_mut(name) {
                return Some(entry);
            }
        }
        None
    }
}
