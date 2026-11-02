use super::*;

/// `GlobalGet r0..r3` then one `CallBuiltin` over them — the shape a lowered `vdApply()` hands its
/// patch buffers to the host in.
fn globals_into(builtin: BuiltinId, arg_count: u16) -> Vec<Op> {
    let mut ops: Vec<Op> = (0..arg_count)
        .map(|r| Op::GlobalGet { dst: r, idx: r })
        .collect();
    ops.push(Op::CallBuiltin {
        dst: arg_count,
        builtin,
        args_start: 0,
        arg_count,
    });
    ops.push(Op::Return { src: arg_count });
    ops
}

/// ★ A global handed to a host import that only READS it stays transient: the host takes the
/// buffers by value and keeps nothing, so retaining them would mark the global's list shared and the
/// next write to it would copy the whole list — once a turn, for every page that patches through
/// `applyPatchWords`.
#[test]
fn a_global_read_by_a_host_import_is_a_transient_read() {
    let ops = globals_into(BuiltinId::DomApplyWords, 4);
    let transient = transient_reads(&ops, 5, &[]);
    assert!(
        transient[..4].iter().all(|&t| t),
        "applyPatchWords only reads its buffers: {transient:?}"
    );
}

/// The parameters an import KEEPS are no transient read: `addEventListener`'s handler is a
/// `CALLBACK` the host re-enters through after the call returns.
#[test]
fn a_parameter_the_host_keeps_is_not_transient() {
    let ops = globals_into(BuiltinId::DomAddEventListener, 3);
    let transient = transient_reads(&ops, 4, &[]);
    assert!(
        transient[0] && transient[1],
        "the element and the event name are only read: {transient:?}"
    );
    assert!(
        !transient[2],
        "the handler is kept by the host: {transient:?}"
    );
}

/// A builtin that is not a host import says nothing about what it keeps, so its arguments stay
/// non-transient.
#[test]
fn a_builtin_that_is_no_host_import_keeps_its_arguments_retained() {
    let ops = globals_into(BuiltinId::Length, 1);
    let transient = transient_reads(&ops, 2, &[]);
    assert!(!transient[0], "{transient:?}");
}
