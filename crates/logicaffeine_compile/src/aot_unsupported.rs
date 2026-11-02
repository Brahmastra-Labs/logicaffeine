//! The ops no *single-device* AOT backend can run — concurrency, virtual-time
//! scheduling, and networking — classified once so the device tiers (Tenstorrent
//! baby core, one CUDA kernel launch) cannot drift apart on their refusal set.
//!
//! A Tensix baby core and a GPU are both SPMD single-program engines with no
//! cooperative scheduler and no host IO on the compute path. Whatever else a
//! given tier additionally cannot lower (a GPU thread cannot run the glibc-heavy
//! dynamic runtime, for instance) it layers on top of THIS shared base; the base
//! itself is the one thing every offload target agrees on.

use crate::vm::{CompiledProgram, Op};

/// The category of op that no single-device AOT backend can lower. Each tier maps
/// a category to its own worded reason (a Tensix core vs. a CUDA kernel phrase it
/// differently) but the SET is shared, so a concurrency/networking op can never
/// be silently accepted by one tier and refused by another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offload {
    /// Shared-memory channel ops — no channel exists on a single compute engine.
    Channel,
    /// Task spawn / await / abort — the engine is SPMD, not a thread pool.
    Task,
    /// Sleep / select — needs a cooperative scheduler (virtual time).
    VirtualTime,
    /// Peer networking — host-only IO, off the compute datapath.
    Net,
    /// The DOM, the reactive-signal store, WebRTC — a BROWSER surface. A device has no document
    /// to build and no page to render into, so a program that touches one is a program aimed at
    /// the wrong tier. Without this category those tiers ACCEPTED a page and returned an
    /// artifact, and the mistake only showed up as a blank screen on the device.
    Dom,
}

/// Classify a single op. `None` for every op that is not inherently off a
/// single-device datapath (arithmetic, control flow, collections, …).
pub fn classify(op: &Op) -> Option<Offload> {
    match op {
        Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. }
        | Op::ChanClose { .. } => Some(Offload::Channel),
        Op::Spawn { .. } | Op::SpawnHandle { .. } | Op::TaskAwait { .. } | Op::TaskAbort { .. } => {
            Some(Offload::Task)
        }
        Op::Sleep { .. }
        | Op::SelectArmRecv { .. }
        | Op::SelectArmTimeout { .. }
        | Op::SelectWait { .. } => Some(Offload::VirtualTime),
        Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetMakePeer { .. }
        | Op::NetSync { .. }
        // The real-socket TCP client (`tcpConnect`/…) and the HTTP server (`Serve on port …`) are host-only IO
        // — no single-device AOT datapath (analog/Tenstorrent/CUDA) can lower them, so they offload like the
        // peer-`Net*` ops. Without this, those tiers silently accept a socket program instead of fail-closed.
        | Op::Tcp { .. }
        | Op::Serve { .. } => Some(Offload::Net),
        // The DOM arrives as ordinary BUILTIN calls rather than opcodes of its own — which is why
        // this classifier, matching only on `Op` variants, could not see it and every device tier
        // silently accepted a web page.
        Op::CallBuiltin { builtin, .. } if builtin.is_browser_only() => Some(Offload::Dom),
        _ => None,
    }
}

/// The first offload-unsupported op in `program`, classified — `None` if the whole
/// program is on the single-device datapath.
pub fn first(program: &CompiledProgram) -> Option<Offload> {
    program.code.iter().find_map(classify)
}

/// The first unsupported op REACHABLE from the program's entry — the honest question for a
/// backend deciding whether it can build this program.
///
/// ★ [`first`] scans the WHOLE code array, which includes every standard-library function the
/// prelude loaded and nobody calls. `web.lg` holds the browser fetch wrappers AND the pure-text
/// helpers `htmlEscape`, `isIntText` and `matchRoute`, so scanning everything told any program
/// that escaped a string or matched a route that it "touches the DOM" — which is every real
/// server. A backend must not refuse a program for code it will never emit.
///
/// Reachability walks `Op::Call` from Main, plus every closure body (a closure is called through
/// a handle, so its target is not a static call edge and treating it as reachable is the SAFE
/// direction: an unreachable refusal is a nuisance, a missed one ships a program that cannot run).
pub fn first_reachable(program: &CompiledProgram) -> Option<Offload> {
    // Bodies share one code array; a function owns `[entry_pc, next entry)`. Main is whatever
    // precedes the first function body.
    let mut entries: Vec<(usize, usize)> = program
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.entry_pc, i))
        .collect();
    entries.sort_unstable();
    let range = |i: usize| -> (usize, usize) {
        let start = program.functions[i].entry_pc;
        let end = entries
            .iter()
            .map(|(pc, _)| *pc)
            .find(|pc| *pc > start)
            .unwrap_or(program.code.len());
        (start, end)
    };
    let main_end = entries
        .first()
        .map(|(pc, _)| *pc)
        .unwrap_or(program.code.len());

    let mut seen = vec![false; program.functions.len()];
    // A closure body is entered through a handle rather than a `Call`, so it has no static edge
    // to follow. Seed them all: refusing something unreachable is a nuisance; missing something
    // reachable ships a program that cannot run.
    for (i, f) in program.functions.iter().enumerate() {
        if !f.captures.is_empty() {
            seen[i] = true;
        }
    }
    let mut queue: Vec<usize> = (0..program.functions.len()).filter(|i| seen[*i]).collect();
    let scan = |from: usize, to: usize, queue: &mut Vec<usize>, seen: &mut Vec<bool>| {
        let mut found = None;
        for op in &program.code[from..to] {
            if found.is_none() {
                if let Some(off) = classify(op) {
                    found = Some(off);
                }
            }
            if let crate::vm::Op::Call { func, .. } = op {
                let idx = *func as usize;
                if idx < seen.len() && !seen[idx] {
                    seen[idx] = true;
                    queue.push(idx);
                }
            }
        }
        found
    };
    if let Some(off) = scan(0, main_end, &mut queue, &mut seen) {
        return Some(off);
    }
    while let Some(i) = queue.pop() {
        let (from, to) = range(i);
        if let Some(off) = scan(from, to, &mut queue, &mut seen) {
            return Some(off);
        }
    }
    None
}
