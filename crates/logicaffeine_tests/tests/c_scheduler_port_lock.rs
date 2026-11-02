//! The C family's scheduler core IS `logicaffeine_runtime`'s scheduler, step for step.
//!
//! Every C-family tier — the host C and RISC-V coroutine runtimes, the stackless frames CUDA and the
//! bytecode walker run — schedules through one text, `codegen_c::SCHED_CORE`, the runtime's
//! `process_step` ported to C. A port is only as good as its agreement, so this lock runs the toy
//! scheduler scripts (`common/sched_scripts`) on both and demands the same observations: every line
//! in the same order, the same values through every pipe and select, the same deadlocks, at every
//! seed. The C side is a toy tier of its own: each script runs as a frame over the core's operations,
//! yielding after each, and one binary runs the whole corpus from its input.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use logicaffeine_compile::codegen_c::SCHED_CORE;

#[path = "common/sched_scripts.rs"]
mod sched_scripts;
use sched_scripts::{generate, program, run_on_runtime, Op, Program, Sel, SCHEDULING_SEEDS};

/// The toy tier: a script's ops interpreted as frames over the core, printing the observations the
/// runtime's script tasks print.
const HARNESS: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define LOGOS_FN static
#define LOGOS_VAR static
#define logos_alloc(n) calloc(1, (n))
#define logos_release(p) free(p)
#define LOGOS_TASK_TIER \
    int which; int pc; struct logos_chan **chans; int nchans; struct logos_task **handles; int nhandles; \
    int waiting; int wait_arg; int arm_kinds[16];
@CORE@
enum { OP_SHOW, OP_CHAN, OP_SEND, OP_TRYSEND, OP_RECV, OP_TRYRECV, OP_SLEEP, OP_SPAWN, OP_STOP, OP_AWAIT, OP_CLOSE, OP_SELECT };
enum { W_NONE, W_RECV, W_TRYRECV, W_TRYSEND, W_SELECT };
typedef struct { int kind; long long a, b; int n; int arm_kind[16]; long long arm_arg[16]; } toy_op;
static toy_op **scripts;
static int *script_len;

static logos_task *toy_task(int which, logos_chan **chans, int nchans) {
    logos_task *t = (logos_task *)calloc(1, sizeof(logos_task));
    t->which = which;
    t->chans = (logos_chan **)calloc((size_t)(nchans + 64), sizeof(logos_chan *));
    for (int i = 0; i < nchans; i++) t->chans[i] = chans[i];
    t->nchans = nchans;
    t->handles = (logos_task **)calloc(64, sizeof(logos_task *));
    return t;
}

static void show_resume(char *out, logos_task *t) {
    if (t->resume_nothing) strcpy(out, "nothing"); else sprintf(out, "%lld", t->resume);
}

static void toy_poll(logos_task *t) {
    char v[32];
    switch (t->waiting) {
        case W_RECV: show_resume(v, t); printf("t%d recv#%d %s\n", t->which, t->wait_arg, v); break;
        case W_TRYRECV: show_resume(v, t); printf("t%d try-recv#%d %s\n", t->which, t->wait_arg, v); break;
        case W_TRYSEND: printf("t%d try-send#%d %s\n", t->which, t->wait_arg, t->resume ? "true" : "false"); break;
        case W_SELECT:
            if (t->arm_kinds[t->selected_arm] == LOGOS_ARM_AFTER) strcpy(v, "timeout"); else show_resume(v, t);
            printf("t%d select arm %d: %s\n", t->which, t->selected_arm, v);
            break;
    }
    t->waiting = W_NONE;
    while (t->pc < script_len[t->which]) {
        toy_op *op = &scripts[t->which][t->pc++];
        int n = t->nchans;
        switch (op->kind) {
            case OP_SHOW: printf("t%d show %lld\n", t->which, op->a); break;
            case OP_CHAN: t->chans[t->nchans++] = logos_op_new_chan(t, op->a); return;
            case OP_SEND: if (!n) break; logos_op_send(t, t->chans[op->a % n], op->b); return;
            case OP_TRYSEND:
                if (!n) break;
                t->waiting = W_TRYSEND; t->wait_arg = (int)(op->a % n);
                logos_op_try_send(t, t->chans[t->wait_arg], op->b); return;
            case OP_RECV:
                if (!n) break;
                t->waiting = W_RECV; t->wait_arg = (int)(op->a % n);
                logos_op_recv(t, t->chans[t->wait_arg]); return;
            case OP_TRYRECV:
                if (!n) break;
                t->waiting = W_TRYRECV; t->wait_arg = (int)(op->a % n);
                logos_op_try_recv(t, t->chans[t->wait_arg]); return;
            case OP_SLEEP: if (op->a > 0) { logos_op_sleep(t, op->a); return; } break;
            case OP_SPAWN: {
                logos_task *child = toy_task((int)op->a, t->chans, t->nchans);
                t->handles[t->nhandles++] = child;
                logos_op_spawn(t, child);
                return;
            }
            case OP_STOP: if (!t->nhandles) break; logos_op_abort(t, t->handles[op->a % t->nhandles]); return;
            case OP_AWAIT: if (!t->nhandles) break; logos_op_await(t, t->handles[op->a % t->nhandles]); return;
            case OP_CLOSE: if (!n) break; logos_op_close(t, t->chans[op->a % n]); return;
            case OP_SELECT: {
                int kind[16]; logos_chan *chan[16]; long long ticks[16]; int k = 0;
                for (int i = 0; i < op->n; i++) {
                    if (op->arm_kind[i] == LOGOS_ARM_RECV && !n) continue;
                    kind[k] = op->arm_kind[i];
                    chan[k] = op->arm_kind[i] == LOGOS_ARM_RECV ? t->chans[op->arm_arg[i] % n] : 0;
                    ticks[k] = op->arm_arg[i];
                    k++;
                }
                if (!k) break;
                for (int i = 0; i < k; i++) t->arm_kinds[i] = kind[i];
                t->waiting = W_SELECT;
                logos_op_select(t, k, kind, chan, ticks);
                return;
            }
        }
    }
    logos_op_exit(t);
}

static void toy_run(unsigned long long seed) {
    logos_ready_q.len = 0; logos_ready_q.head = 0;
    logos_clock = 0; logos_timer_n = 0; logos_all_n = 0; logos_failure = 0;
    logos_seed(seed);
    logos_task_begin(toy_task(0, 0, 0));
    for (;;) {
        logos_task *t = logos_pick_ready();
        if (t) { toy_poll(t); continue; }
        if (!logos_advance_timers()) break;
    }
    printf("=== %s\n", logos_deadlocked() ? "deadlock" : "done");
}

int main(void) {
    char word[32];
    while (scanf("%31s", word) == 1) {
        if (word[0] == 'P') {
            int ns; if (scanf("%d", &ns) != 1) return 2;
            scripts = (toy_op **)calloc((size_t)ns, sizeof(toy_op *));
            script_len = (int *)calloc((size_t)ns, sizeof(int));
            for (int s = 0; s < ns; s++) {
                int len; if (scanf(" S %d", &len) != 1) return 2;
                script_len[s] = len;
                scripts[s] = (toy_op *)calloc((size_t)(len + 1), sizeof(toy_op));
                for (int i = 0; i < len; i++) {
                    toy_op *op = &scripts[s][i];
                    if (scanf("%d %lld %lld %d", &op->kind, &op->a, &op->b, &op->n) != 4) return 2;
                    for (int a = 0; a < op->n; a++) if (scanf("%d %lld", &op->arm_kind[a], &op->arm_arg[a]) != 2) return 2;
                }
            }
        } else if (word[0] == 'R') {
            unsigned long long seed; if (scanf("%llu", &seed) != 1) return 2;
            toy_run(seed);
        }
    }
    return 0;
}
"#;

fn harness_binary() -> &'static std::path::Path {
    static BIN: OnceLock<std::path::PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("logos_sched_core_port_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("harness.c");
        std::fs::write(&src, HARNESS.replace("@CORE@", SCHED_CORE)).unwrap();
        let bin = dir.join("harness");
        let build = Command::new("gcc")
            .args(["-std=c99", "-O2", "-Wall", "-Wno-unused-function", "-o"])
            .arg(&bin)
            .arg(&src)
            .output()
            .expect("gcc runs");
        assert!(
            build.status.success(),
            "the core does not build:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        bin
    })
}

/// A program in the harness's input form.
fn encode(p: &Program) -> String {
    let mut out = format!("P {}\n", p.len());
    for script in p.iter() {
        out.push_str(&format!("S {}\n", script.len()));
        for op in script {
            let line = match op {
                Op::Show(n) => format!("0 {n} 0 0"),
                Op::NewChan(cap) => format!("1 {} 0 0", cap.map_or(-1, |c| c as i64)),
                Op::Send(c, v) => format!("2 {c} {v} 0"),
                Op::TrySend(c, v) => format!("3 {c} {v} 0"),
                Op::Recv(c) => format!("4 {c} 0 0"),
                Op::TryRecv(c) => format!("5 {c} 0 0"),
                Op::Sleep(t) => format!("6 {t} 0 0"),
                Op::Spawn(k) => format!("7 {k} 0 0"),
                Op::Stop(h) => format!("8 {h} 0 0"),
                Op::Await(h) => format!("9 {h} 0 0"),
                Op::Close(c) => format!("10 {c} 0 0"),
                Op::Select(arms) => {
                    let mut l = format!("11 0 0 {}", arms.len());
                    for a in arms {
                        match a {
                            Sel::Recv(c) => l.push_str(&format!(" 0 {c}")),
                            Sel::After(t) => l.push_str(&format!(" 1 {t}")),
                        }
                    }
                    l
                }
            };
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// Run every (program, seed) on the core in one process, in order.
fn run_on_core(runs: &[(Program, u64)]) -> Vec<(Vec<String>, String)> {
    let mut input = String::new();
    for (p, seed) in runs {
        input.push_str(&encode(p));
        input.push_str(&format!("R {seed}\n"));
    }
    let mut child = Command::new(harness_binary())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the harness runs");
    // Fed from its own thread: the harness answers as it reads, and a parent that wrote the whole
    // corpus before reading would stall both sides on full pipes.
    let mut stdin = child.stdin.take().unwrap();
    let feeder = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let out = child.wait_with_output().unwrap();
    feeder
        .join()
        .expect("the feeder thread")
        .expect("the corpus reaches the harness");
    assert!(out.status.success(), "the harness failed: {:?}", out.status);
    let mut results = Vec::new();
    let mut log = Vec::new();
    for line in String::from_utf8(out.stdout).unwrap().lines() {
        match line.strip_prefix("=== ") {
            Some(outcome) => results.push((std::mem::take(&mut log), outcome.to_string())),
            None => log.push(line.to_string()),
        }
    }
    assert_eq!(results.len(), runs.len(), "one outcome per run");
    results
}

fn assert_same_on_core(what: &str, runs: &[(Program, u64)]) {
    for ((p, seed), (log, outcome)) in runs.iter().zip(run_on_core(runs)) {
        let (want_log, want_outcome) = run_on_runtime(p, *seed);
        assert_eq!(
            (log.clone(), outcome.as_str()),
            (want_log, want_outcome),
            "{what}: the C core diverged from the scheduler at seed {seed}.\nprogram: {p:#?}"
        );
    }
}

#[test]
fn every_generated_program_runs_identically_on_the_core() {
    if !common::require_tool("gcc") {
        return;
    }
    let mut runs = Vec::new();
    for program_seed in 0..3_000u64 {
        let p = generate(program_seed);
        for &seed in SCHEDULING_SEEDS {
            runs.push((p.clone(), seed));
        }
    }
    let got = run_on_core(&runs);
    let (mut deadlocks, mut selects, mut timeouts, mut closed_reads, mut lines) = (0, 0, 0, 0, 0);
    for ((p, seed), (log, outcome)) in runs.iter().zip(got) {
        let (want_log, want_outcome) = run_on_runtime(p, *seed);
        assert_eq!(
            (log.clone(), outcome.as_str()),
            (want_log, want_outcome),
            "the C core diverged from the scheduler at seed {seed}.\nprogram: {p:#?}"
        );
        deadlocks += usize::from(outcome == "deadlock");
        selects += log.iter().filter(|l| l.contains(" select arm ")).count();
        timeouts += log.iter().filter(|l| l.ends_with(": timeout")).count();
        closed_reads += log
            .iter()
            .filter(|l| l.contains(" recv#") && l.ends_with(" nothing"))
            .count();
        lines += log.len();
    }
    // The same corpus `replay_executor_port_lock` measures, at the same floors.
    assert!(deadlocks >= 3_200, "only {deadlocks} runs deadlocked");
    assert!(selects >= 4_400, "only {selects} selects resolved");
    assert!(timeouts >= 3_700, "only {timeouts} select timeouts fired");
    assert!(
        closed_reads >= 120,
        "only {closed_reads} receives read a closed, drained pipe"
    );
    assert!(
        lines >= 22_000,
        "only {lines} observations in the whole corpus"
    );
}

#[test]
fn a_select_naming_one_pipe_twice_withdraws_both_registrations() {
    if !common::require_tool("gcc") {
        return;
    }
    // The waiting select is registered on pipe 0 twice; once a value wins it, the second
    // registration must not outlive the select and steal the next send.
    let p = program(vec![
        vec![
            Op::NewChan(None),
            Op::NewChan(None),
            Op::Spawn(1),
            Op::Select(vec![Sel::Recv(0), Sel::Recv(0)]),
            Op::Recv(1),
        ],
        vec![Op::Send(0, 1), Op::Send(0, 2), Op::Send(1, 99)],
    ]);
    let runs: Vec<(Program, u64)> = SCHEDULING_SEEDS.iter().map(|s| (p.clone(), *s)).collect();
    assert_same_on_core("duplicate select arm", &runs);
    let (log, _) = run_on_runtime(&p, 0);
    assert!(
        log.contains(&"t0 recv#1 99".to_string()),
        "the later receive reads its own pipe: {log:?}"
    );
}

#[test]
fn a_close_resolves_a_waiting_select_once() {
    if !common::require_tool("gcc") {
        return;
    }
    let p = program(vec![
        vec![Op::NewChan(None), Op::Spawn(1), Op::Close(0), Op::Show(1)],
        vec![
            Op::Select(vec![Sel::Recv(0), Sel::Recv(0), Sel::After(5)]),
            Op::Show(2),
        ],
    ]);
    let runs: Vec<(Program, u64)> = SCHEDULING_SEEDS.iter().map(|s| (p.clone(), *s)).collect();
    assert_same_on_core("close wakes a select", &runs);
}

#[test]
fn a_stopped_sender_hands_over_nothing() {
    if !common::require_tool("gcc") {
        return;
    }
    let p = program(vec![
        vec![
            Op::NewChan(Some(0)),
            Op::Spawn(1),
            Op::Stop(0),
            Op::TryRecv(0),
            Op::Show(1),
        ],
        vec![Op::Send(0, 7), Op::Show(10)],
    ]);
    let runs: Vec<(Program, u64)> = SCHEDULING_SEEDS.iter().map(|s| (p.clone(), *s)).collect();
    assert_same_on_core("stopped sender", &runs);
    let (log, _) = run_on_runtime(&p, 0);
    assert_eq!(
        log,
        ["t0 try-recv#0 nothing", "t0 show 1"],
        "a stopped sender's value is never delivered"
    );
}

#[test]
fn a_stopped_task_wakes_its_awaiters() {
    if !common::require_tool("gcc") {
        return;
    }
    let p = program(vec![
        vec![
            Op::NewChan(None),
            Op::Spawn(1),
            Op::Spawn(2),
            Op::Stop(0),
            Op::Show(1),
        ],
        vec![Op::Recv(0), Op::Show(10)],
        vec![Op::Await(0), Op::Show(20)],
    ]);
    let runs: Vec<(Program, u64)> = SCHEDULING_SEEDS.iter().map(|s| (p.clone(), *s)).collect();
    assert_same_on_core("stop wakes awaiters", &runs);
}

#[test]
fn a_full_default_pipe_blocks_its_thirty_third_send() {
    if !common::require_tool("gcc") {
        return;
    }
    let mut producer: Vec<Op> = (0..40).map(|i| Op::Send(0, i)).collect();
    producer.push(Op::Show(1));
    let p = program(vec![
        vec![Op::NewChan(None), Op::Spawn(1), Op::Show(2), Op::Recv(0)],
        producer,
    ]);
    let runs: Vec<(Program, u64)> = SCHEDULING_SEEDS.iter().map(|s| (p.clone(), *s)).collect();
    assert_same_on_core("default capacity", &runs);
}
