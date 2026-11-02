//! The DDR4 channel model's log (`codegen_sva::ddr4_jedec::verilog_model` with `LOG` set), read and replayed through
//! the controller model it ports.

#![allow(dead_code)]

use std::path::Path;

use logicaffeine_compile::codegen_sva::ddr4_jedec::{check_bus, Ddr4Cmd, Ddr4Facts, Ddr4Timed, Ddr4Timing, Scheduler, LOOKAHEAD};

/// What a channel model scheduled: every line (line, write, arrival, column clock) in order, and every command it
/// placed (clock, kind, group, bank, row or column).
pub struct ChannelLog {
    pub lines: Vec<(u64, bool, u64, u64)>,
    pub commands: Vec<(u64, u64, u64, u64, u64)>,
}

pub fn read_log(path: &Path) -> ChannelLog {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut log = ChannelLog { lines: vec![], commands: vec![] };
    for l in text.lines() {
        let f: Vec<u64> = l.split_whitespace().skip(1).map(|x| x.parse().unwrap()).collect();
        match l.as_bytes()[0] {
            b'L' => log.lines.push((f[0], f[1] == 1, f[2], f[3])),
            b'C' => log.commands.push((f[0], f[1], f[2], f[3], f[4])),
            _ => panic!("{}: {l}", path.display()),
        }
    }
    log
}

/// A command as the model logs it.
pub fn as_logged(c: &Ddr4Timed) -> (u64, u64, u64, u64, u64) {
    match c.cmd {
        Ddr4Cmd::Act { group, bank, row } => (c.at, 0, group as u64, bank as u64, row),
        Ddr4Cmd::Rd { group, bank, col } => (c.at, 1, group as u64, bank as u64, col),
        Ddr4Cmd::Wr { group, bank, col } => (c.at, 2, group as u64, bank as u64, col),
        Ddr4Cmd::Pre { group, bank } => (c.at, 3, group as u64, bank as u64, 0),
        Ddr4Cmd::PreAll => (c.at, 4, 0, 0, 0),
        Ddr4Cmd::Ref => (c.at, 5, 0, 0, 0),
    }
}

/// The log replayed through the controller model at the same arrivals: every line's column clock and every command
/// the model's, the trace legal. The controller clock each read's burst leaves the bus in, in order, and the trace's
/// facts.
pub fn replay(t: &Ddr4Timing, log: &ChannelLog, what: &str) -> (Vec<u64>, Ddr4Facts) {
    let mut trace = Vec::new();
    let mut due = Vec::new();
    {
        let mut sched = Scheduler::new(t, LOOKAHEAD);
        let mut sink = |c: Ddr4Timed| trace.push(c);
        for (k, &(line, write, arrive, cas)) in log.lines.iter().enumerate() {
            let c = sched.line_at(line, write, arrive, &mut sink);
            assert_eq!(c, cas, "{what}: line {k} ({line}, write {write}, arriving {arrive})");
            if !write {
                due.push((c + t.rl + t.burst_ck()).div_ceil(t.grade.rate));
            }
        }
        sched.finish(&mut sink);
    }
    let mut placed = log.commands.clone();
    placed.sort();
    let mine: Vec<_> = trace.iter().map(as_logged).collect();
    assert_eq!(placed, mine, "{what}: the model's commands are the controller model's");
    let facts = check_bus(t, &trace).unwrap_or_else(|v| panic!("{what}: the model's trace: {v}"));
    (due, facts)
}
