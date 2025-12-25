use logos::{DiscourseContext, OwnershipState};
use logos::context::{Entity, Gender, Number};

#[test]
fn entity_starts_owned() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "thing".to_string(),
        ownership: OwnershipState::Owned,
    });

    let entity = ctx.resolve_definite("thing").unwrap();
    assert_eq!(entity.ownership, OwnershipState::Owned);
}

#[test]
fn ownership_state_can_be_moved() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "book".to_string(),
        ownership: OwnershipState::Owned,
    });

    ctx.set_ownership("book", OwnershipState::Moved);

    let entity = ctx.resolve_definite("book").unwrap();
    assert_eq!(entity.ownership, OwnershipState::Moved);
}

#[test]
fn ownership_state_can_be_borrowed() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "item".to_string(),
        ownership: OwnershipState::Owned,
    });

    ctx.set_ownership("item", OwnershipState::Borrowed);

    let entity = ctx.resolve_definite("item").unwrap();
    assert_eq!(entity.ownership, OwnershipState::Borrowed);
}

#[test]
fn get_ownership_returns_current_state() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "y".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "value".to_string(),
        ownership: OwnershipState::Owned,
    });

    assert_eq!(ctx.get_ownership("value"), Some(OwnershipState::Owned));
    assert_eq!(ctx.get_ownership("unknown"), None);
}

// Step 1.5: Use-After-Move Detection

#[test]
fn moved_variable_detected() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "book".to_string(),
        ownership: OwnershipState::Owned,
    });

    ctx.set_ownership("book", OwnershipState::Moved);

    // After move, get_ownership should return Moved
    assert_eq!(ctx.get_ownership("book"), Some(OwnershipState::Moved));
}

#[test]
fn borrowed_variable_still_accessible() {
    let mut ctx = DiscourseContext::new();
    ctx.register(Entity {
        symbol: "x".to_string(),
        gender: Gender::Neuter,
        number: Number::Singular,
        noun_class: "item".to_string(),
        ownership: OwnershipState::Owned,
    });

    ctx.set_ownership("item", OwnershipState::Borrowed);

    // After borrow, get_ownership should return Borrowed (not Moved)
    assert_eq!(ctx.get_ownership("item"), Some(OwnershipState::Borrowed));
}
