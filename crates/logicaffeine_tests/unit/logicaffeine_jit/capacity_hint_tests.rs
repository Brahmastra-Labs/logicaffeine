use super::*;

#[test]
fn whole_function_jit_preserves_runtime_list_capacity() {
    let ops = vec![
        Op::LoadConst { dst: 0, idx: 0 },
        Op::NewEmptyListWithCapacity {
            dst: 1,
            capacity: 0,
        },
        Op::LoadConst { dst: 2, idx: 1 },
        Op::ListPush { list: 1, value: 2 },
        Op::Length {
            dst: 3,
            collection: 1,
        },
        Op::Return { src: 3 },
    ];
    let constants = vec![Constant::Int(64), Constant::Int(7)];
    let table = FnTable::new(1);
    let call_ctx = CallCtx {
        table_addr: table.slot_addr(0),
        depth_addr: 0,
        status_addr: 0,
        limit_slot: 0,
        depth_limit: logicaffeine_compile::semantics::MAX_CALL_DEPTH as i64,
        self_fi: 0,
        table: &table,
        callees: &[],
    };

    let adapted = adapt_function(
        &ops,
        0,
        &constants,
        &[],
        0,
        4,
        0,
        &[],
        &[],
        Some(SlotKind::Int),
        &call_ctx,
        &[],
    )
    .expect("runtime-capacity list must stay eligible for the whole-function JIT");

    assert!(adapted.micro.iter().any(|op| {
        matches!(
            op,
            MicroOp::NewList { helper_addr, .. }
                if *helper_addr == logos_rt_alloc_list_i64_fixed as *const () as usize as i64
        )
    }));
    assert!(
        adapted
            .micro
            .iter()
            .any(|op| { matches!(op, MicroOp::Move { src: 0, .. }) }),
        "the runtime capacity register must feed the fixed-capacity allocator"
    );
}
