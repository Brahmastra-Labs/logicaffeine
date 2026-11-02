//! Hand a value that is read once straight on: the register-machine lowering stores every result
//! into its register's local and loads it back where it is used, and when the very next instruction
//! is that one use, the store and the load are a round trip through a local the value never needed.
//!
//! [`stackify`] decodes a finished function body, drops each `local.set x` immediately followed by
//! the only `local.get x` the body has (the value simply stays on the stack), turns a `local.set` of
//! a local nothing reads into a `drop`, and repeats until nothing more goes. Both rewrites leave the
//! operand stack and every observable value exactly as they were. The decoder is total on the
//! opcodes the emitter writes and declines on anything else, returning the body unchanged.

/// One decoded instruction: its byte range in the body, and the local it names when it is a
/// `local.get`/`local.set`/`local.tee`.
#[derive(Clone, Copy)]
struct Instr {
    start: usize,
    end: usize,
    local: Option<(u8, u32)>,
}

fn read_uleb(bytes: &[u8], i: &mut usize) -> Option<u32> {
    let mut r: u64 = 0;
    let mut shift = 0;
    loop {
        let b = *bytes.get(*i)?;
        *i += 1;
        r |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return u32::try_from(r).ok();
        }
        shift += 7;
        if shift > 35 {
            return None;
        }
    }
}

fn skip_sleb(bytes: &[u8], i: &mut usize) -> Option<()> {
    loop {
        let b = *bytes.get(*i)?;
        *i += 1;
        if b & 0x80 == 0 {
            return Some(());
        }
    }
}

/// The locals prefix's end and the instructions of `body`, or `None` on anything the emitter never
/// writes.
fn decode(body: &[u8]) -> Option<(usize, Vec<Instr>)> {
    let mut i = 0;
    let groups = read_uleb(body, &mut i)?;
    for _ in 0..groups {
        read_uleb(body, &mut i)?;
        i += 1;
    }
    let locals_end = i;
    let mut out = Vec::new();
    while i < body.len() {
        let start = i;
        let op = body[i];
        i += 1;
        let mut local = None;
        match op {
            0x02..=0x04 => {
                let b = *body.get(i)?;
                if matches!(b, 0x40 | 0x7f | 0x7e | 0x7d | 0x7c | 0x7b | 0x70 | 0x6f) {
                    i += 1;
                } else {
                    skip_sleb(body, &mut i)?;
                }
            }
            0x00 | 0x01 | 0x05 | 0x0b | 0x0f | 0x1a | 0x1b | 0x45..=0xc4 => {}
            0x0c | 0x0d | 0x10 | 0x23 | 0x24 => {
                read_uleb(body, &mut i)?;
            }
            0x0e => {
                let n = read_uleb(body, &mut i)?;
                for _ in 0..=n {
                    read_uleb(body, &mut i)?;
                }
            }
            0x11 => {
                read_uleb(body, &mut i)?;
                read_uleb(body, &mut i)?;
            }
            0x20..=0x22 => local = Some((op, read_uleb(body, &mut i)?)),
            0x28..=0x3e => {
                read_uleb(body, &mut i)?;
                read_uleb(body, &mut i)?;
            }
            0x3f | 0x40 => i += 1,
            0x41 | 0x42 => skip_sleb(body, &mut i)?,
            0x43 => i += 4,
            0x44 => i += 8,
            0xfc => match read_uleb(body, &mut i)? {
                0..=7 => {}
                10 => i += 2,
                11 => i += 1,
                _ => return None,
            },
            _ => return None,
        }
        if i > body.len() {
            return None;
        }
        out.push(Instr {
            start,
            end: i,
            local,
        });
    }
    Some((locals_end, out))
}

/// Whether `op` ends a straight-line run: control flow, after which the next instruction need not
/// be the next one to execute.
fn is_control(op: u8) -> bool {
    matches!(op, 0x00 | 0x02..=0x05 | 0x0b..=0x0f)
}

/// `body` (a function body: its locals, its instructions, its `end`) with every round trip through
/// a once-read local taken out, or `body` itself when it holds an opcode the decoder does not know.
/// Two rewrites, to a fixed point:
///
/// - a `local.set x` whose value the very next instruction reads back as the body's only
///   `local.get x` goes, with that read — the value stays on the stack; a `local.set` of a local
///   nothing reads becomes a `drop`;
/// - a `local.set x` of a constant, or of another local `a`, whose only read comes later in the same
///   straight-line run — no control flow between, and no write to `a` — goes with its producer, and
///   the read becomes the producer: the constant, or `local.get a`.
pub(crate) fn stackify(body: Vec<u8>) -> Vec<u8> {
    let Some((locals_end, instrs)) = decode(&body) else {
        return body;
    };
    let mut gets: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut sets: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for ins in &instrs {
        match ins.local {
            Some((0x20, x)) => *gets.entry(x).or_default() += 1,
            Some((_, x)) => *sets.entry(x).or_default() += 1,
            None => {}
        }
    }
    // What each instruction becomes: kept, gone, a `drop`, or the bytes of another instruction.
    #[derive(Clone, Copy, PartialEq)]
    enum Fate {
        Keep,
        Gone,
        Drop,
        As(usize),
    }
    let op_at = |k: usize| body[instrs[k].start];
    let is_producer = |k: usize| {
        matches!(op_at(k), 0x41 | 0x42 | 0x44) || matches!(instrs[k].local, Some((0x20, _)))
    };
    let mut fate = vec![Fate::Keep; instrs.len()];
    loop {
        let mut changed = false;
        // A `drop` stays in the stream: it takes a value off the stack between its neighbours.
        let live: Vec<usize> = (0..instrs.len())
            .filter(|&k| fate[k] != Fate::Gone)
            .collect();
        let mut w = 0;
        while w < live.len() {
            let k = live[w];
            if let (Fate::Keep, Some((0x21, x))) = (fate[k], instrs[k].local) {
                let reads = gets.get(&x).copied().unwrap_or(0);
                let next = live.get(w + 1).map(|&n| instrs[n].local);
                if reads == 1 && next == Some(Some((0x20, x))) && fate[live[w + 1]] == Fate::Keep {
                    fate[k] = Fate::Gone;
                    fate[live[w + 1]] = Fate::Gone;
                    gets.insert(x, 0);
                    changed = true;
                    w += 2;
                    continue;
                }
                if reads == 0 {
                    fate[k] = Fate::Drop;
                    changed = true;
                } else if reads == 1 && sets.get(&x).copied() == Some(1) && w > 0 {
                    let p = live[w - 1];
                    let source = match instrs[p].local {
                        Some((0x20, a)) if a != x => Some(a),
                        _ => None,
                    };
                    if fate[p] == Fate::Keep && is_producer(p) {
                        // The read, if it follows in this straight-line run with `source` unwritten.
                        let mut found = None;
                        for &n in &live[w + 1..] {
                            if is_control(op_at(n)) {
                                break;
                            }
                            match instrs[n].local {
                                Some((0x20, y)) if y == x => {
                                    if fate[n] == Fate::Keep {
                                        found = Some(n);
                                    }
                                    break;
                                }
                                Some((0x21 | 0x22, y)) if Some(y) == source => break,
                                _ => {}
                            }
                        }
                        if let Some(n) = found {
                            fate[p] = Fate::Gone;
                            fate[k] = Fate::Gone;
                            fate[n] = Fate::As(p);
                            gets.insert(x, 0);
                            changed = true;
                        }
                    }
                }
            }
            w += 1;
        }
        if !changed {
            break;
        }
    }
    let mut out = Vec::with_capacity(body.len());
    out.extend_from_slice(&body[..locals_end]);
    for (ins, f) in instrs.iter().zip(&fate) {
        match *f {
            Fate::Keep => out.extend_from_slice(&body[ins.start..ins.end]),
            Fate::Drop => out.push(0x1a),
            Fate::As(p) => out.extend_from_slice(&body[instrs[p].start..instrs[p].end]),
            Fate::Gone => {}
        }
    }
    out
}

#[cfg(all(test, feature = "wasm-jit", not(target_arch = "wasm32")))]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/stackify/tests.rs"]
mod tests;
