//! SUPERCRUSH Sprint S4B: Incremental Verification

#![cfg(feature = "verification")]

use logicaffeine_verify::incremental::*;
use logicaffeine_verify::VerifyExpr;

#[test]
fn incr_empty_cache() {
    let mut cache = VerificationCache::new();
    assert!(cache.is_empty());
}

#[test]
fn incr_store_and_lookup() {
    let mut cache = VerificationCache::new();
    let hash = hash_property(&VerifyExpr::bool(true));
    cache.store(hash, CachedResult {
        is_safe: true,
        dependencies: vec!["spec.txt".into()],
        timestamp: 1,
    });
    assert!(cache.lookup(hash).is_some());
    assert!(cache.lookup(hash).unwrap().is_safe);
}

#[test]
fn incr_cache_hit_same_result() {
    let mut cache = VerificationCache::new();
    let hash = 12345;
    cache.store(hash, CachedResult {
        is_safe: true,
        dependencies: vec![],
        timestamp: 0,
    });
    let result = cache.lookup(hash).unwrap();
    assert!(result.is_safe);
}

#[test]
fn incr_invalidate_on_change() {
    let mut cache = VerificationCache::new();
    cache.store(1, CachedResult {
        is_safe: true,
        dependencies: vec!["spec_a.txt".into()],
        timestamp: 0,
    });
    cache.store(2, CachedResult {
        is_safe: true,
        dependencies: vec!["spec_b.txt".into()],
        timestamp: 0,
    });
    assert_eq!(cache.len(), 2);

    cache.invalidate(&[ChangeEvent { changed_item: "spec_a.txt".into() }]);
    assert_eq!(cache.len(), 1);
    assert!(cache.lookup(1).is_none());
    assert!(cache.lookup(2).is_some());
}

#[test]
fn incr_transitive_invalidation() {
    let mut cache = VerificationCache::new();
    cache.store(1, CachedResult {
        is_safe: true,
        dependencies: vec!["module_c".into()],
        timestamp: 0,
    });
    // Change module_c → invalidates property 1
    cache.invalidate(&[ChangeEvent { changed_item: "module_c".into() }]);
    assert!(cache.lookup(1).is_none());
}

#[test]
fn incr_no_invalidation_unrelated() {
    let mut cache = VerificationCache::new();
    cache.store(1, CachedResult {
        is_safe: true,
        dependencies: vec!["spec_a.txt".into()],
        timestamp: 0,
    });
    cache.invalidate(&[ChangeEvent { changed_item: "spec_z.txt".into() }]);
    assert!(cache.lookup(1).is_some(), "Unrelated change should not invalidate");
}

#[test]
fn incr_hash_deterministic() {
    let expr = VerifyExpr::and(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let h1 = hash_property(&expr);
    let h2 = hash_property(&expr);
    assert_eq!(h1, h2, "Same expression should give same hash");
}

#[test]
fn incr_verify_incremental_basic() {
    let mut cache = VerificationCache::new();
    let props = vec![
        (VerifyExpr::bool(true), vec!["src".into()]),
    ];
    let results = verify_incremental(&props, &[], &mut cache);
    assert_eq!(results.len(), 1);
    assert!(results[0].1); // bool(true) is safe
}

#[test]
fn incr_cache_persists() {
    let mut cache = VerificationCache::new();
    let props = vec![
        (VerifyExpr::bool(true), vec![]),
    ];
    let _ = verify_incremental(&props, &[], &mut cache);
    assert!(!cache.is_empty(), "Cache should have entries after verification");

    // Second call should hit cache
    let results2 = verify_incremental(&props, &[], &mut cache);
    assert_eq!(results2.len(), 1);
}

#[test]
fn incr_all_changed_forces_reverify() {
    let mut cache = VerificationCache::new();
    cache.store(hash_property(&VerifyExpr::bool(true)), CachedResult {
        is_safe: true,
        dependencies: vec!["all".into()],
        timestamp: 0,
    });
    cache.invalidate(&[ChangeEvent { changed_item: "all".into() }]);
    assert!(cache.is_empty(), "All changed should clear cache");
}
