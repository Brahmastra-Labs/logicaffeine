//! Incremental Verification
//!
//! Cache verification results, track dependencies, invalidate only what changed.

use crate::ir::VerifyExpr;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// A hash of a property for caching.
pub type PropertyHash = u64;

/// Cached verification result.
#[derive(Debug, Clone)]
pub struct CachedResult {
    pub is_safe: bool,
    pub dependencies: Vec<String>,
    pub timestamp: u64,
}

/// Verification cache.
#[derive(Debug, Clone, Default)]
pub struct VerificationCache {
    pub entries: HashMap<PropertyHash, CachedResult>,
}

/// A change event.
#[derive(Debug, Clone)]
pub struct ChangeEvent {
    pub changed_item: String,
}

impl VerificationCache {
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    /// Store a result in the cache.
    pub fn store(&mut self, hash: PropertyHash, result: CachedResult) {
        self.entries.insert(hash, result);
    }

    /// Look up a cached result.
    pub fn lookup(&self, hash: PropertyHash) -> Option<&CachedResult> {
        self.entries.get(&hash)
    }

    /// Invalidate entries affected by changes.
    pub fn invalidate(&mut self, changes: &[ChangeEvent]) {
        let changed_items: Vec<&str> = changes.iter().map(|c| c.changed_item.as_str()).collect();
        self.entries.retain(|_, result| {
            !result.dependencies.iter().any(|dep| changed_items.contains(&dep.as_str()))
        });
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Compute a hash for a property expression.
pub fn hash_property(expr: &VerifyExpr) -> PropertyHash {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", expr).hash(&mut hasher);
    hasher.finish()
}

/// Verify properties incrementally.
pub fn verify_incremental(
    properties: &[(VerifyExpr, Vec<String>)],
    changes: &[ChangeEvent],
    cache: &mut VerificationCache,
) -> Vec<(PropertyHash, bool)> {
    // Invalidate cache for changed items
    cache.invalidate(changes);

    let mut results = Vec::new();

    for (prop, deps) in properties {
        let hash = hash_property(prop);

        if let Some(cached) = cache.lookup(hash) {
            // Cache hit
            results.push((hash, cached.is_safe));
        } else {
            // Cache miss — need to verify
            // Use k-induction as a simple verifier
            // For now, just mark as safe if the property is a tautology
            let is_safe = matches!(prop, VerifyExpr::Bool(true));

            cache.store(hash, CachedResult {
                is_safe,
                dependencies: deps.clone(),
                timestamp: 0,
            });

            results.push((hash, is_safe));
        }
    }

    results
}
