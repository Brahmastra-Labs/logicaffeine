use super::*;
use crate::channel::ChanId;
use crate::task::TaskId;

/// W12 — `deadlock_explain_lock`: the deadlock narration names WHO waits, on
/// WHAT, and each channel's fill state — a proof in prose, not a heuristic.
#[test]
fn narration_names_the_blocked_tasks_and_channels() {
    let snap = SchedSnapshot {
        clock: 1204,
        ready_len: 0,
        tasks: vec![
            TaskView {
                id: TaskId(0),
                kind: TaskStateKind::BlockedRecv,
                is_main: true,
            },
            TaskView {
                id: TaskId(1),
                kind: TaskStateKind::BlockedSend,
                is_main: false,
            },
        ],
        channels: vec![ChanView {
            id: ChanId(0),
            depth: 4,
            capacity: Some(4),
            blocked_senders: 1,
            blocked_receivers: 0,
            closed: false,
        }],
    };
    let prose = narrate_deadlock(&snap);
    assert!(
        prose.contains("Deadlock at tick 1204"),
        "names the moment: {prose}"
    );
    assert!(
        prose.contains("task 0 (main) is waiting to RECEIVE"),
        "{prose}"
    );
    assert!(
        prose.contains("task 1 (spawned) is waiting to SEND"),
        "{prose}"
    );
    assert!(
        prose.contains("channel 0 — 4/4 full"),
        "names the full channel: {prose}"
    );
    assert!(prose.contains("1 blocked sender"), "{prose}");
}

/// Ready tasks are not part of the cycle — they are not narrated as blocked.
#[test]
fn ready_tasks_are_not_in_the_cycle() {
    let snap = SchedSnapshot {
        clock: 5,
        ready_len: 1,
        tasks: vec![TaskView {
            id: TaskId(2),
            kind: TaskStateKind::Ready,
            is_main: false,
        }],
        channels: vec![],
    };
    let prose = narrate_deadlock(&snap);
    assert!(
        !prose.contains("task 2"),
        "a ready task is not a blocked task: {prose}"
    );
}
