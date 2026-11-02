use super::*;

fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: 128,
        max_tree_nodes: 100_000,
        max_store_nodes: 100_000,
        work: 100_000,
        allocation: 100_000,
        materialized_nodes: 100_000,
    }
}

#[test]
fn equality_stack_wrapper_reuses_an_already_sized_kernel_stack() {
    crate::term_store::grow_stack_for_depth(128, || {
        let thread = std::thread::current().id();
        let bytes = crate::term_store::kernel_stack_bytes();
        let observed = with_level_stack(&mut budget(), |_| {
            Ok((
                std::thread::current().id(),
                crate::term_store::kernel_stack_bytes(),
            ))
        })
        .unwrap();
        assert_eq!(observed, (thread, bytes));
    });
}

#[test]
fn active_frame_guards_release_depth_on_error_and_unwind() {
    let active = std::cell::Cell::new(0);
    let mut b = budget();
    let mut m = Meter {
        budget: &mut b,
        active: &active,
    };
    let error: GenerationResult<()> = (|| {
        let _frame = m.enter(1)?;
        assert_eq!(active.get(), 1);
        Err(resource())
    })();
    assert!(error.is_err());
    assert_eq!(active.get(), 0);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _frame = m.enter(1).unwrap();
        assert_eq!(active.get(), 1);
        panic!("owned guard unwind control");
    }));
    assert!(panic.is_err());
    assert_eq!(active.get(), 0);
    let _frame = m.enter(1).unwrap();
    assert_eq!(active.get(), 1);
}
