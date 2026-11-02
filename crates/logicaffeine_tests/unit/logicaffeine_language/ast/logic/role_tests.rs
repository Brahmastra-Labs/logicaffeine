use super::ThematicRole;

#[test]
fn every_role_sits_at_its_own_index() {
    for (i, role) in ThematicRole::ALL.iter().enumerate() {
        assert_eq!(role.index(), i, "{role:?}");
    }
}

#[test]
fn every_role_has_its_own_name() {
    for (i, a) in ThematicRole::ALL.iter().enumerate() {
        for b in &ThematicRole::ALL[i + 1..] {
            assert_ne!(a.name(), b.name());
        }
    }
}
