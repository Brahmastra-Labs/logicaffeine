use super::fresh_untitled_name;
use std::collections::HashSet;

#[test]
fn fresh_untitled_name_avoids_collisions() {
    let mut have: HashSet<String> = HashSet::new();
    assert_eq!(
        fresh_untitled_name("logos", |n| have.contains(n)),
        "untitled.logos"
    );
    have.insert("untitled.logos".to_string());
    assert_eq!(
        fresh_untitled_name("logos", |n| have.contains(n)),
        "untitled-1.logos"
    );
    have.insert("untitled-1.logos".to_string());
    assert_eq!(
        fresh_untitled_name("logos", |n| have.contains(n)),
        "untitled-2.logos"
    );
    // A different extension is independent.
    assert_eq!(
        fresh_untitled_name("logic", |n| have.contains(n)),
        "untitled.logic"
    );
}
