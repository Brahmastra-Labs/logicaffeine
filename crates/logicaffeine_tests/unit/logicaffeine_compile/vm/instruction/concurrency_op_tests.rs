use super::*;

/// Every concurrency op must survive the tier cache's `serde_json` roundtrip
/// (`CompiledProgram`/`FnBytecode` are cached as JSON), or a cached program
/// containing one would fail to reload. `Op` is `Copy` but not `PartialEq`,
/// so we compare its `Debug` form.
#[test]
fn concurrency_ops_serde_roundtrip() {
    let ops = [
        Op::ChanNew {
            dst: 1,
            cap: -1,
            elem: ChanElem::Int,
        },
        Op::ChanNew {
            dst: 2,
            cap: 8,
            elem: ChanElem::Text,
        },
        Op::ChanSend { chan: 3, val: 4 },
        Op::ChanRecv { dst: 5, chan: 6 },
        Op::ChanTrySend {
            dst: 7,
            chan: 8,
            val: 9,
        },
        Op::ChanTryRecv { dst: 10, chan: 11 },
        Op::ChanClose { chan: 12 },
        Op::Spawn {
            func: 1,
            args_start: 13,
            arg_count: 2,
        },
        Op::SpawnHandle {
            dst: 14,
            func: 2,
            args_start: 15,
            arg_count: 0,
        },
        Op::TaskAwait {
            dst: 16,
            handle: 17,
        },
        Op::TaskAbort { handle: 18 },
        Op::SelectArmRecv { chan: 19, var: 20 },
        Op::SelectArmTimeout { ticks: 21 },
        Op::SelectWait { dst_arm: 22 },
    ];
    for op in ops {
        let json = serde_json::to_string(&op).expect("serialize");
        let back: Op = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            format!("{op:?}"),
            format!("{back:?}"),
            "roundtrip {op:?} via {json}"
        );
    }
}
