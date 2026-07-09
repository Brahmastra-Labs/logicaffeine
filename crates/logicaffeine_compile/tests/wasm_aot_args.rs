//! WASM AOT — command-line ARGUMENTS (`args()` / `parseInt`). A program that reads its input from
//! argv (`parseInt(item 2 of args())`) compiles to standalone wasm: `args()` lowers to an imported
//! `env.args` host that returns a `Seq of Text` handle the host builds in the module's linear memory,
//! and `parseInt` to an imported `env.parse_int`. This test supplies those hosts (building the argv
//! sequence with the SAME `[len][cap][data_ptr]` layout the AOT emits) and asserts the wasm output is
//! byte-identical to the argv-aware tree-walker oracle (`tw_outcome_with_args`). The `Seq of Text` +
//! `parseInt` round-trip through real linear memory is the actual coverage.
#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use logicaffeine_compile::compile::{compile_to_wasm, display_float_like_logos, tw_outcome, tw_outcome_with_args, vm_outcome, vm_outcome_concurrent, vm_outcome_net, vm_outcome_with_args};
use std::cell::RefCell;
use std::rc::Rc;

/// Build an argv `Seq of Text` in `data` at a fixed high address (clear of the small programs'
/// bump-allocated heap) — the exact layout the AOT reads: a 16-byte header `[len][cap][data_ptr]`
/// for the sequence and for each `Text`, sequence elements in 8-byte slots holding a `Text` handle.
fn build_argv(data: &mut [u8], args: &[String]) -> i32 {
    let seq = 60000usize;
    let n = args.len();
    let seq_data = seq + 16;
    let mut p = seq_data + n * 8;
    let mut handles = Vec::new();
    for s in args {
        let t = p;
        let b = s.as_bytes();
        let td = t + 16;
        data[t..t + 4].copy_from_slice(&(b.len() as i32).to_le_bytes());
        data[t + 4..t + 8].copy_from_slice(&(b.len() as i32).to_le_bytes());
        data[t + 8..t + 12].copy_from_slice(&(td as i32).to_le_bytes());
        data[td..td + b.len()].copy_from_slice(b);
        handles.push(t as i64);
        p = (td + b.len() + 7) & !7;
    }
    data[seq..seq + 4].copy_from_slice(&(n as i32).to_le_bytes());
    data[seq + 4..seq + 8].copy_from_slice(&(n as i32).to_le_bytes());
    data[seq + 8..seq + 12].copy_from_slice(&(seq_data as i32).to_le_bytes());
    for (i, h) in handles.iter().enumerate() {
        let sl = seq_data + i * 8;
        data[sl..sl + 8].copy_from_slice(&h.to_le_bytes());
    }
    seq as i32
}

/// Read the `i64` element slots of a `Seq`/`Set` at `handle` (header `[len][cap][data_ptr]`, then
/// 8-byte slots) out of the module's exported memory — the host side of the collection formatters.
fn read_i64_slots(c: &wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32) -> Vec<i64> {
    let mem = c.get_export("memory").unwrap().into_memory().unwrap();
    let d = mem.data(c);
    let h = handle as usize;
    let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
    let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
    (0..len).map(|i| i64::from_le_bytes(d[dp + i * 8..dp + i * 8 + 8].try_into().unwrap())).collect()
}

/// Write `bytes` into the module's linear memory at `buf` (the formatter hosts' scratch buffer).
fn mem_write(c: &mut wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, bytes: &[u8]) {
    let mem = c.get_export("memory").unwrap().into_memory().unwrap();
    let d = mem.data_mut(c);
    let b = buf as usize;
    d[b..b + bytes.len()].copy_from_slice(bytes);
}

fn run_aot(module: &[u8], argv: Vec<String>) -> String {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let out: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let mut store = wasmi::Store::new(&engine, out.clone());
    let mut l = wasmi::Linker::<Rc<RefCell<Vec<String>>>>::new(&engine);
    l.func_wrap("env", "print_i64", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
        c.data().borrow_mut().push(v.to_string());
    })
    .unwrap();
    l.func_wrap("env", "print_rational", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, num: i64, den: i64| {
        c.data().borrow_mut().push(if den == 1 { num.to_string() } else { format!("{num}/{den}") });
    })
    .unwrap();
    l.func_wrap("env", "print_nothing", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>| {
        c.data().borrow_mut().push("nothing".into());
    })
    .unwrap();
    l.func_wrap("env", "print_bool", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i32| {
        c.data().borrow_mut().push(if v != 0 { "true".into() } else { "false".into() });
    })
    .unwrap();
    l.func_wrap("env", "print_char", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
        c.data().borrow_mut().push(char::from_u32(v as u32).map(|ch| ch.to_string()).unwrap_or_default());
    })
    .unwrap();
    // A `Word32`/`Word64` shown directly: its UNSIGNED decimal (`Show`ing a word, not `intOfWord*`).
    l.func_wrap("env", "print_word", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
        c.data().borrow_mut().push((v as u64).to_string());
    })
    .unwrap();
    l.func_wrap("env", "print_f64", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: wasmi::core::F64| {
        c.data().borrow_mut().push(display_float_like_logos(f64::from(v)));
    })
    .unwrap();
    // A `Duration`/`Time` (i64 nanosecond tick count) formats via the VM's OWN `to_display_string`
    // (`5s`/`3h` for Duration, `HH:MM:SS[.frac]` for Time) — zero divergence from the interpreter.
    l.func_wrap("env", "print_duration", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
        c.data().borrow_mut().push(logicaffeine_compile::interpreter::RuntimeValue::Duration(v).to_display_string());
    })
    .unwrap();
    l.func_wrap("env", "print_time", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
        c.data().borrow_mut().push(logicaffeine_compile::interpreter::RuntimeValue::Time(v).to_display_string());
    })
    .unwrap();
    // A `Span` packs `months` (high i32) + `days` (low i32) into one i64; unpack and format via the VM's
    // own `RuntimeValue::Span` display (`1 year and 2 months and 3 days`).
    l.func_wrap("env", "print_span", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
        let months = (v >> 32) as i32;
        let days = v as i32;
        c.data().borrow_mut().push(logicaffeine_compile::interpreter::RuntimeValue::Span { months, days }.to_display_string());
    })
    .unwrap();
    // A `Moment` (nanoseconds since the epoch, i64) / `Date` (days since the epoch, i32): rendered by
    // the VM's own `RuntimeValue` display, so the AOT's `Show` of an `add_seconds`/`date_of` result is
    // byte-identical to the interpreter.
    l.func_wrap("env", "print_moment", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
        c.data().borrow_mut().push(logicaffeine_compile::interpreter::RuntimeValue::Moment(v).to_display_string());
    })
    .unwrap();
    l.func_wrap("env", "print_date", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i32| {
        c.data().borrow_mut().push(logicaffeine_compile::interpreter::RuntimeValue::Date(v).to_display_string());
    })
    .unwrap();
    // `parse_timestamp(text) -> Moment nanos`: read the RFC-3339 `Text` handle and parse it with the
    // SAME `base::temporal` parser the VM uses, so a `parse_timestamp("…")`-seeded program agrees.
    l.func_wrap("env", "parse_timestamp", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| -> i64 {
        let mem = c.get_export("memory").unwrap().into_memory().unwrap();
        let d = mem.data(&c);
        let h = h as usize;
        let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
        let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
        let s = std::str::from_utf8(&d[dp..dp + len]).unwrap().trim();
        logicaffeine_base::temporal::parse_rfc3339(s).expect("valid RFC 3339 timestamp")
    })
    .unwrap();
    // `write_wire_residual(data_ptr, len) -> len` — the residual-emit sink: the embedder frames the bytes
    // (`[len:u32][bytes]`) to its wire stream and returns the byte count (what `writeWireResidual` yields).
    l.func_wrap("env", "write_wire_residual", |_c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, _ptr: i32, len: i32| -> i64 {
        len as i64
    })
    .unwrap();
    // A `Text` handle: read its `[len][cap][data_ptr]` header and emit the UTF-8 bytes. Used by the
    // string-output benchmarks (`Show "" + item 1 of arr + " " + …`), which build a Text via `+`.
    l.func_wrap("env", "print_text", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| {
        let mem = c.get_export("memory").unwrap().into_memory().unwrap();
        let d = mem.data(&c);
        let h = h as usize;
        let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
        let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
        c.data().borrow_mut().push(String::from_utf8_lossy(&d[dp..dp + len]).to_string());
    })
    .unwrap();
    // The scalar→Text formatters a `+`-concatenation calls to stringify a non-Text operand: write the
    // bytes into `buf` and return the length (the AOT wraps a Text header around them).
    l.func_wrap("env", "fmt_i64_into", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, v: i64| -> i32 {
        let bytes = v.to_string();
        let b = bytes.as_bytes();
        mem_write(&mut c, buf, b);
        b.len() as i32
    })
    .unwrap();
    l.func_wrap("env", "fmt_f64_into", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, v: wasmi::core::F64| -> i32 {
        let bytes = display_float_like_logos(f64::from(v));
        let b = bytes.as_bytes();
        mem_write(&mut c, buf, b);
        b.len() as i32
    })
    .unwrap();
    l.func_wrap("env", "fmt_bool_into", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, v: i64| -> i32 {
        let b: &[u8] = if v != 0 { b"true" } else { b"false" };
        mem_write(&mut c, buf, b);
        b.len() as i32
    })
    .unwrap();
    l.func_wrap("env", "fmt_f64_prec_into", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, v: wasmi::core::F64, prec: i32| -> i32 {
        let s = format!("{:.prec$}", f64::from(v), prec = prec as usize);
        mem_write(&mut c, buf, s.as_bytes());
        s.len() as i32
    })
    .unwrap();
    // Alignment/width spec (`{x:>6}`): read the already-stringified `Text`, pad it to `width` with the
    // SAME Rust `format!` `apply_format_spec` uses, and write it into `buf`. align: 0 right, 1 left, 2 center.
    l.func_wrap("env", "fmt_align_into", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, text: i32, width: i32, align: i32| -> i32 {
        let s = {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = text as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            String::from_utf8_lossy(&d[dp..dp + len]).to_string()
        };
        let w = width as usize;
        let padded = match align {
            0 => format!("{:>w$}", s, w = w),
            1 => format!("{:<w$}", s, w = w),
            2 => format!("{:^w$}", s, w = w),
            _ => s,
        };
        mem_write(&mut c, buf, padded.as_bytes());
        padded.len() as i32
    })
    .unwrap();
    let av = argv.clone();
    l.func_wrap("env", "args", move |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>| -> i32 {
        let mem = c.get_export("memory").unwrap().into_memory().unwrap();
        let d = mem.data_mut(&mut c);
        build_argv(d, &av)
    })
    .unwrap();
    l.func_wrap("env", "parse_int", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| -> i64 {
        let mem = c.get_export("memory").unwrap().into_memory().unwrap();
        let d = mem.data_mut(&mut c);
        let h = h as usize;
        let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
        let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
        std::str::from_utf8(&d[dp..dp + len]).unwrap().trim().parse::<i64>().unwrap_or(0)
    })
    .unwrap();
    // Collection→Text formatters (a whole `Seq of Int`/`Set of Int` stringified for a `+`/`format`):
    // read the i64 slots, format `[…]`/`{…}` (matching `RuntimeValue::List`/`Set`), write into `buf`.
    l.func_wrap("env", "fmt_seq_i64_into", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, handle: i32| -> i32 {
        let s = format!("[{}]", read_i64_slots(&c, handle).iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "));
        mem_write(&mut c, buf, s.as_bytes());
        s.len() as i32
    })
    .unwrap();
    l.func_wrap("env", "fmt_set_i64_into", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, handle: i32| -> i32 {
        let s = format!("{{{}}}", read_i64_slots(&c, handle).iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "));
        mem_write(&mut c, buf, s.as_bytes());
        s.len() as i32
    })
    .unwrap();
    // A whole `Seq of Bool` display `[true, false, …]`: each i64-0/1 slot renders as `true`/`false`
    // (matching `RuntimeValue::List` of `ListRepr::Bools`), NOT the integer `1`/`0`.
    fn bool_seq(c: &wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32) -> String {
        format!("[{}]", read_i64_slots(c, handle).iter().map(|v| if *v != 0 { "true" } else { "false" }).collect::<Vec<_>>().join(", "))
    }
    l.func_wrap("env", "print_seq_bool", move |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
        let s = bool_seq(&c, handle);
        c.data().borrow_mut().push(s);
    })
    .unwrap();
    l.func_wrap("env", "fmt_seq_bool_into", move |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, handle: i32| -> i32 {
        let s = bool_seq(&c, handle);
        mem_write(&mut c, buf, s.as_bytes());
        s.len() as i32
    })
    .unwrap();
    // Whole `Seq of Word32`/`Word64` display `[u, …]` — each 8-byte slot's value as UNSIGNED decimal
    // (Word32 = low 32 bits, Word64 = full 64 bits), matching `RuntimeValue::List` of `Word`.
    l.func_wrap("env", "print_seq_word32", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
        let s = format!("[{}]", read_i64_slots(&c, handle).iter().map(|v| (*v as u32).to_string()).collect::<Vec<_>>().join(", "));
        c.data().borrow_mut().push(s);
    })
    .unwrap();
    l.func_wrap("env", "print_seq_word64", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
        let s = format!("[{}]", read_i64_slots(&c, handle).iter().map(|v| (*v as u64).to_string()).collect::<Vec<_>>().join(", "));
        c.data().borrow_mut().push(s);
    })
    .unwrap();
    // Whole `Seq of Int` display `[n, …]` — each 8-byte slot as a SIGNED decimal, matching `RuntimeValue::List`.
    l.func_wrap("env", "print_seq_i64", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
        let s = format!("[{}]", read_i64_slots(&c, handle).iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "));
        c.data().borrow_mut().push(s);
    })
    .unwrap();
    // `Set of Text` display `{s0, s1, …}`: each 8-byte slot's low word is a `Text` handle (read its
    // header + bytes), elements unquoted + insertion-ordered — matching `RuntimeValue::Set`.
    l.func_wrap("env", "print_set_text", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
        let mem = c.get_export("memory").unwrap().into_memory().unwrap();
        let d = mem.data(&c);
        let h = handle as usize;
        let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
        let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
        let parts: Vec<String> = (0..len)
            .map(|i| {
                let th = i32::from_le_bytes(d[dp + i * 8..dp + i * 8 + 4].try_into().unwrap()) as usize;
                let tl = i32::from_le_bytes(d[th..th + 4].try_into().unwrap()) as usize;
                let tdp = i32::from_le_bytes(d[th + 8..th + 12].try_into().unwrap()) as usize;
                String::from_utf8_lossy(&d[tdp..tdp + tl]).to_string()
            })
            .collect();
        c.data().borrow_mut().push(format!("{{{}}}", parts.join(", ")));
    })
    .unwrap();
    let inst = l.instantiate(&mut store, &m).unwrap().start(&mut store).unwrap();
    inst.get_typed_func::<(), ()>(&store, "main").unwrap().call(&mut store, ()).expect("main runs without trapping");
    let lines = out.borrow().clone();
    lines.join("\n")
}

/// Compile `src`, run it with `argv`, and assert byte-identical to BOTH argv-aware oracles.
fn assert_args(src: &str, argv: &[&str]) {
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!(tw.error, None, "tree-walker oracle errored:\n{src}");
    let vm = vm_outcome_with_args(src, &argv, None);
    assert_eq!(vm.error, None, "VM oracle errored:\n{src}");
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed: {e:?}\n{src}"));
    let got = run_aot(&module, argv);
    assert_eq!(got.trim(), tw.output.trim(), "AOT wasm disagrees with the tree-walker for:\n{src}");
    assert_eq!(got.trim(), vm.output.trim(), "AOT wasm disagrees with the VM for:\n{src}");
}

const ARGS_PRELUDE: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n";

/// Compile a CONCURRENT program and assert the AOT output equals BOTH the tree-walker AND the
/// scheduler-driven VM (`vm_outcome_concurrent`) — the plain VM oracle errors on concurrency, so the
/// driven one is the parity oracle. Proves tw == VM == AOT for the deterministic single-thread shape.
fn assert_concurrent(src: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker errored on:\n{src}");
    let vc = vm_outcome_concurrent(src);
    assert_eq!(vc.error, None, "driven-VM errored on:\n{src}");
    assert_eq!(tw.output, vc.output, "tree-walker != driven-VM (base parity) for:\n{src}");
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed: {e:?}\n{src}"));
    let got = run_aot(&module, vec![]);
    assert_eq!(got.trim(), tw.output.trim(), "AOT wasm != tree-walker for:\n{src}");
    assert_eq!(got.trim(), vc.output.trim(), "AOT wasm != driven-VM for:\n{src}");
}

/// Compile a NETWORKING program and assert the AOT output equals BOTH the tree-walker (offline local
/// mode) AND the VM net runner (`vm_outcome_net`, local `NetInbox`). Proves tw == VM == AOT for the
/// deterministic local-mode net shapes (Listen/Sync/PeerAgent/Send = no-ops; the Shows + local CRDT
/// values are the output). A real relay deployment links the transport via the P2 linker phase.
fn assert_net(src: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker errored on:\n{src}");
    let vn = vm_outcome_net(src);
    assert_eq!(vn.error, None, "VM net runner errored on:\n{src}");
    assert_eq!(tw.output, vn.output, "tree-walker != VM-net (base parity) for:\n{src}");
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed: {e:?}\n{src}"));
    let got = run_aot(&module, vec![]);
    assert_eq!(got.trim(), tw.output.trim(), "AOT wasm != tree-walker for:\n{src}");
    assert_eq!(got.trim(), vn.output.trim(), "AOT wasm != VM-net for:\n{src}");
}

/// `Listen on <addr>` (`NetListen`) — Syntax Guide `network-listen`. Offline single node: listening on
/// one's own address is a local no-op, so the `Show` prints. tw == VM-net == AOT.
#[test]
fn aot_net_listen() {
    assert_net("## Main\nListen on \"/ip4/0.0.0.0/tcp/8000\".\nShow \"Server listening on port 8000\".\n");
}

/// `a PeerAgent at <addr>` (`NetMakePeer`) + `Send` — Syntax Guide `network-peer-agent`/`send-message`.
/// A peer handle is locally its address Text; a `Send` is a fire-and-forget no-op.
#[test]
fn aot_net_peer_and_send() {
    assert_net("## A Greeting is Portable and has:\n    a message (Text).\n\n\
                ## Main\nLet remote be a PeerAgent at \"/ip4/127.0.0.1/tcp/8000\".\n\
                Let msg be a new Greeting with message \"Hello, peer!\".\nShow \"Sending: \" + msg's message.\nSend msg to remote.\n");
}

/// `Sync <crdt> on <topic>` (`NetSync`) — Syntax Guide `crdt-sync-counter`/`crdt-sync-profile`. Offline
/// single node: sync is a no-op, so the local counter value stands. tw == VM-net == AOT.
#[test]
fn aot_net_crdt_sync() {
    assert_net("## A GameScore is Shared and has:\n    a points, which is ConvergentCount.\n\n\
                ## Main\nLet mutable score be a new GameScore.\nSync score on \"game-leaderboard\".\nIncrease score's points by 100.\nShow score's points.\n");
}

/// OFFLINE LOOPBACK peer messaging (`NetSend` → `NetAwait`) — the VM/AOT are offline determinism
/// oracles, so a `Send` delivers to the local inbox and a matching `Await` reads it back (output is
/// transport-independent). A self-addressed round-trip: send `42` to our own inbox, await it, show it.
/// tw == VM-net == AOT, all `42`.
#[test]
fn aot_net_send_await_loopback() {
    assert_net(
        "## Main\nListen on \"/ip4/127.0.0.1/tcp/9000\".\nLet peer be a PeerAgent at \"/ip4/127.0.0.1/tcp/9000\".\n\
         Send 42 to peer.\nAwait response from peer into x.\nShow x.\n",
    );
}

/// OFFLINE LOOPBACK batch STREAM (`NetStream` -> `NetAwait stream`) - `Stream [values] to <self>`
/// delivers the whole list to the local inbox and `Await stream ... into xs` reads it back. tw ==
/// VM-net == AOT.
#[test]
fn aot_net_stream_await_loopback() {
    assert_net(
        "## Main\nListen on \"/ip4/127.0.0.1/tcp/9100\".\nLet peer be a PeerAgent at \"/ip4/127.0.0.1/tcp/9100\".\n\
         Stream [10, 20, 30] to peer.\nAwait stream from peer into xs.\nShow length of xs.\n",
    );
}

/// `Connect to <addr>` (`NetConnect`) — Syntax Guide `network-connect`. Offline single node: there is
/// no relay to dial, so `Connect` is a local no-op and the `Show` prints "Connected to server". tw ==
/// VM-net == AOT (the deterministic oracles run offline; a real deployment dials via the relay driver).
#[test]
fn aot_net_connect() {
    assert_net("## Main\nLet server_addr be \"/ip4/127.0.0.1/tcp/8000\".\nConnect to server_addr.\nShow \"Connected to server\".\n");
}

/// `Listen` + `Sync` together — Syntax Guide `network-mdns` shape. Both are offline no-ops; the two
/// `Show`s are the output.
#[test]
fn aot_net_listen_and_sync() {
    assert_net("## A GameState is Shared and has:\n    a score, which is ConvergentCount.\n\n\
                ## Main\nListen on \"/ip4/0.0.0.0/tcp/0\".\nShow \"Listening... mDNS will auto-discover peers\".\n\n\
                Let mutable state be a new GameState.\nSync state on \"game-session\".\nShow \"Synced to game-session topic\".\n");
}

/// A single-threaded `Pipe`/channel (`ChanNew`/`Send`/`Receive`) — the Syntax Guide `pipe-send-receive`.
/// A buffered channel used send-then-receive is a FIFO queue: 42 then 100, receive → 42 (the FRONT).
#[test]
fn aot_pipe_send_receive_fifo() {
    assert_concurrent("## Main\nLet messages be a new Pipe of Int.\nSend 42 into messages.\nSend 100 into messages.\nReceive x from messages.\nShow \"Got: \" + x.\n");
}

/// `Let r: Rational be a / b` (`ExactDiv`) — division in a Rational context builds an exact reduced
/// Rational, Shown `num/den` (or `num` when whole, matching the VM's downsize). tw == VM == AOT.
#[test]
fn aot_exact_div_rational() {
    for (src, want) in [
        ("## Main\nLet r: Rational be 7 / 2.\nShow r.\n", "7/2"),
        ("## Main\nLet r: Rational be 6 / 2.\nShow r.\n", "3"),
        ("## Main\nLet r: Rational be 10 / 4.\nShow r.\n", "5/2"),
        ("## Main\nLet r: Rational be 1 / 3.\nShow r.\n", "1/3"),
        ("## Main\nLet r: Rational be 100 / 8.\nShow r.\n", "25/2"),
    ] {
        let tw = tw_outcome(src);
        assert_eq!(tw.error, None, "tree-walker errored on:\n{src}");
        let vm = vm_outcome(src);
        assert_eq!(vm.error, None, "VM errored on:\n{src}");
        assert_eq!(tw.output.trim(), vm.output.trim(), "tree-walker != VM on:\n{src}");
        assert_eq!(tw.output.trim(), want, "unexpected tree-walker output for:\n{src}");
        let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed: {e:?}\n{src}"));
        let got = run_aot(&module, vec![]);
        assert_eq!(got.trim(), want, "WASM AOT != {want:?} for:\n{src}");
    }
}

/// `Sleep N` in a scheduler-driven program (`Op::Sleep`) — advances virtual time only, so the
/// send-then-receive still yields the value. tw == driven-VM == AOT (Sleep is a no-op in the AOT).
#[test]
fn aot_sleep_is_virtual_time_noop() {
    assert_concurrent("## Main\nLet p be a new Pipe of Int.\nSend 5 into p.\nSleep 10.\nReceive x from p.\nShow \"Got: \" + x.\n");
}

/// Fire-and-forget tasks (`Spawn`) — the Syntax Guide `launch-task`. Each `Launch a task to worker(n)`
/// runs synchronously (the deterministic scheduler runs each to completion in launch order).
#[test]
fn aot_launch_tasks_run_in_order() {
    assert_concurrent("## To worker (id: Int):\n    Show \"Worker \" + id + \" started\".\n\n## Main\nLaunch a task to worker(1).\nLaunch a task to worker(2).\nShow \"Tasks launched\".\n");
}

/// Task handle + `Stop` (`SpawnHandle`/`TaskAbort`) — the Syntax Guide `task-with-handle`. The task
/// runs synchronously (so `"Working..."` prints), the handle is a dummy, and `Stop` is a no-op.
#[test]
fn aot_task_handle_and_stop() {
    assert_concurrent("## To long_running:\n    Show \"Working...\".\n\n## Main\nLet job be Launch a task to long_running.\nShow \"Task spawned\".\nStop job.\nShow \"Task cancelled\".\n");
}

/// `select` over an EMPTY channel + a timeout (`SelectArmRecv`/`SelectArmTimeout`/`SelectWait`) — the
/// Syntax Guide `select-timeout`. An empty `Pipe of Text` (never sent to) has no ready recv arm, so the
/// deterministic scheduler fires the timeout arm → "No message received". The `Pipe of Text` element
/// type (carried on `ChanNew`) types the recv arm's `msg` even though the pipe is never sent to.
#[test]
fn aot_select_timeout_fires_on_empty_pipe() {
    assert_concurrent(
        "## Main\nLet inbox be a new Pipe of Text.\n\n\
         Await the first of:\n    Receive msg from inbox:\n        Show \"Message: \" + msg.\n\
         \n    After 2 seconds:\n        Show \"No message received\".\n",
    );
}

/// `select` whose recv arm IS ready (a value was sent first) — the recv arm wins over the timeout,
/// binds `msg`, and runs its body. Proves the FIRST-ready-recv branch of `SelectWait` (pop-front into
/// the arm's var), the dual of the timeout case.
#[test]
fn aot_select_recv_wins_when_message_waiting() {
    assert_concurrent(
        "## Main\nLet inbox be a new Pipe of Text.\nSend \"ping\" into inbox.\n\n\
         Await the first of:\n    Receive msg from inbox:\n        Show \"Message: \" + msg.\n\
         \n    After 2 seconds:\n        Show \"No message received\".\n",
    );
}

/// Non-blocking `Try to receive` on an EMPTY pipe binds `Nothing` (the scheduler's `do_try_recv`
/// resumes with `Nothing` when the queue is empty), so `Show x` prints "nothing". This is the AOT
/// analog of `vm_concurrency::vm_try_receive_empty_is_nothing`. Exercises the Optional value model:
/// `ChanTryRecv` yields a `Kind::Optional` (i32 handle, 0 = Nothing) and `Show` of it prints "nothing".
#[test]
fn aot_try_receive_on_empty_pipe_is_nothing() {
    assert_concurrent("## Main\nLet ch be a new Pipe of Int.\nTry to receive x from ch.\nShow x.\n");
}

/// Non-blocking `Try to receive` on a NON-EMPTY pipe pops the front value (the scheduler resumes with
/// the raw value, not a wrapper), so `Show x` prints the Int. Exercises the `Some` arm of the Optional
/// value model: `ChanTryRecv` boxes the popped scalar (handle != 0) and `Show` loads + prints it.
#[test]
fn aot_try_receive_on_nonempty_pipe_yields_the_value() {
    assert_concurrent("## Main\nLet ch be a new Pipe of Int.\nSend 42 into ch.\nTry to receive x from ch.\nShow x.\n");
}

/// `x is equal to nothing` on a `Try to receive` result routes through `Op::Eq` against
/// `Constant::Nothing`; on the AOT that is an is-nothing check (handle == 0). Empty pipe → the branch
/// taken is the `nothing` one. Proves the Optional flows through a conditional, not just `Show`.
#[test]
fn aot_try_receive_is_nothing_branch() {
    assert_concurrent(
        "## Main\nLet ch be a new Pipe of Int.\nTry to receive x from ch.\n\
         If x is equal to nothing:\n    Show \"empty\".\nOtherwise:\n    Show \"got a value\".\n",
    );
}

/// Non-blocking `Try to send` (`ChanTrySend`) always succeeds on the unbounded FIFO (queues the
/// value) — proven observably: a subsequent `Try to receive` pops the very value just sent, so
/// `Show x` prints it. Exercises `ChanTrySend` (append) feeding `ChanTryRecv` (`Some` pop). The
/// success `Bool` the op also yields is unobservable from source (`Try to send` never binds it).
#[test]
fn aot_try_send_then_try_receive_round_trips() {
    assert_concurrent("## Main\nLet ch be a new Pipe of Int.\nTry to send 99 into ch.\nTry to receive x from ch.\nShow x.\n");
}

#[test]
fn aot_args_parse_int_arithmetic() {
    let src = format!("{ARGS_PRELUDE}## Main\nLet a be args().\nLet n be parseInt(item 2 of a).\nShow n * n.\nShow n + 1.\n");
    assert_args(&src, &["prog", "7"]);
}

#[test]
fn aot_args_drives_a_recursive_function() {
    let src = format!(
        "{ARGS_PRELUDE}## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    \
         Return fib(n - 1) + fib(n - 2).\n## Main\nLet a be args().\nLet n be parseInt(item 2 of a).\nShow fib(n).\n"
    );
    assert_args(&src, &["prog", "10"]);
}

/// An argv-SIZED `Seq of Int` filled in a loop with i32-fit values (`% 1000000`) — the optimizer
/// narrows the list to `NewEmptyListI32`, which the AOT lowers identically to the i64 `NewEmptyList`.
/// This is the exact shape of the array benchmarks (array_fill / two_sum / prefix_sum), now compiling.
#[test]
fn aot_args_fills_an_i32_array() {
    let src = format!(
        "{ARGS_PRELUDE}## Main\nLet a be args().\nLet n be parseInt(item 2 of a).\n\
         Let mutable arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n:\n    \
         Push (i * 7 + 3) % 1000000 to arr.\n    Set i to i + 1.\nLet mutable sum be 0.\nSet i to 1.\n\
         While i is at most n:\n    Set sum to sum + item i of arr.\n    Set i to i + 1.\nShow sum.\n"
    );
    assert_args(&src, &["prog", "5"]);
}

#[test]
fn aot_args_two_arguments() {
    let src = format!(
        "{ARGS_PRELUDE}## Main\nLet a be args().\nLet x be parseInt(item 2 of a).\n\
         Let y be parseInt(item 3 of a).\nShow x * y.\nShow x + y.\n"
    );
    assert_args(&src, &["prog", "6", "7"]);
}

/// A self-contained `Show "" + a + " " + b` — string concatenation via the polymorphic `+` (the
/// shape every benchmark's final `Show` uses to format its result line). The `+` whose result is
/// Text routes through `lower_concat`, stringifying the Int operands via the formatter hosts.
#[test]
fn aot_string_concat_via_plus() {
    let src = format!(
        "{ARGS_PRELUDE}## Main\nLet a be args().\nLet n be parseInt(item 2 of a).\n\
         Show \"\" + n + \" squared is \" + (n * n) + \" done\".\n"
    );
    assert_args(&src, &["prog", "9"]);
}

/// Interpolation ALIGNMENT/WIDTH format specs (`{x:>6}` right, `{x:<6}` left, `{x:^7}` center, and the
/// bare-width `{x:6}` = right-align). `apply_format_spec` pads `to_display_string()` to the width with
/// spaces (Rust `format!("{:>w$}", s)`, char-counted). The AOT stringifies the value and pads via the
/// `fmt_align_into` host (the same Rust `format!`), so `[    42]`/`[42    ]`/`[  42   ]` are byte-identical.
#[test]
fn aot_interpolation_alignment_specs() {
    let src = "## Main\nLet x be 42.\n\
               Show \"[{x:>6}]\".\nShow \"[{x:<6}]\".\nShow \"[{x:^7}]\".\nShow \"[{x:6}]\".\n";
    assert_args(src, &["prog"]);
}

/// Alignment on a NON-numeric value — a Text (`{name:>8}`) and the left/center variants — proving the
/// pad path is value-kind-agnostic (it aligns the stringified display, not a re-formatted number).
#[test]
fn aot_interpolation_alignment_of_text() {
    let src = "## Main\nLet name be \"cat\".\nShow \"[{name:>8}]\".\nShow \"[{name:<8}]\".\nShow \"[{name:^8}]\".\n";
    assert_args(src, &["prog"]);
}

/// Character literals (`` `a` ``) are a scalar Unicode code point: `Constant::Char` lowers to an
/// `i64.const` holding `char as u32`, and a `Show` of a `Char` routes to `print_char` (the host
/// emits the UTF-8 character, not the numeric code point) — byte-identical to the tree-walker's
/// `Char` display. This is the Syntax Guide's `char-literals` example verbatim (the escaped `\n`/
/// `\t`/`\\` code points are also constructed, exercising the non-ASCII `LoadConst Char` arm).
#[test]
fn aot_shows_a_character_literal() {
    let src = r#"## Main
Let letter be `a`.
Let newline be `\n`.
Let tab be `\t`.
Let escaped be `\\`.
Show letter.
Show "Char type uses backticks"."#;
    assert_args(src, &["prog"]);
}

/// `Show` of a `Duration` (each magnitude bucket) and a `Time` literal — the AOT gives them their own
/// `Kind::Duration`/`Kind::Time` (i64 tick count) and renders via `print_duration`/`print_time` (the
/// VM's own `to_display_string`: `3h`/`5min`/`2s`/`500ms`/`50μs`/`100ns`, `HH:MM:SS`), NOT the raw nanos
/// a plain `Int` would show. A duration variable + a comparison exercise the Move + i64-compare paths.
#[test]
fn aot_shows_duration_and_time_literals() {
    let src = "## Main\n\
        Show 3h.\n\
        Show 5min.\n\
        Show 2s.\n\
        Show 500ms.\n\
        Show 50us.\n\
        Show 100ns.\n\
        Show noon.\n\
        Show midnight.\n\
        Let d be 2s.\n\
        Show d.\n\
        Show 3s is greater than 2s.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a `Span` literal (self-contained): the AOT packs `months`/`days` into one i64
/// (`Constant::Span`) and renders via `print_span` (the VM's own `RuntimeValue::Span` display —
/// `1 year and 2 months and 3 days`, end-of-list `and`-joined), byte-identical to the VM.
#[test]
fn aot_shows_span_literals() {
    let src = "## Main\n\
        Show 3 days.\n\
        Show 1 year.\n\
        Show 2 months.\n";
    assert_args(src, &["prog"]);
}

/// Byte interop (self-contained): `text_bytes("…")` builds a `Seq of Int` of the UTF-8 bytes and
/// `text_from_bytes(seq)` rebuilds the `Text` — the emitter constructs the seq/Text in linear memory
/// (no runtime), round-tripping byte-identically to the VM.
#[test]
fn aot_text_bytes_roundtrip() {
    let src = "## Main\n\
        Show text_bytes(\"abc\").\n\
        Let bytes be [72, 105].\n\
        Show text_from_bytes(bytes).\n\
        Show text_from_bytes(text_bytes(\"round trip\")).\n";
    assert_args(src, &["prog"]);
}

/// `writeWireResidual(text) -> Int` — the residual-emit half of the wire-program protocol: the Text's
/// bytes are framed out to the host wire sink (`write_wire_residual`), and the builtin yields the byte
/// COUNT (what `Show` observes), byte-identical to the VM. Self-contained (a host import, both modes).
#[test]
fn aot_write_wire_residual() {
    let src = "## Main\n\
        Show writeWireResidual(\"hello\").\n\
        Show writeWireResidual(\"\").\n\
        Show writeWireResidual(\"a longer residual payload\").\n";
    assert_args(src, &["prog"]);
}

/// `copy(x)` — the builtin deep clone. A `copy` of a heap value is INDEPENDENT (mutating the source does
/// not reach the copy — value semantics), and a `copy` of a scalar is just the value. Lowered identically
/// to `Op::DeepClone`, byte-identical to the VM.
#[test]
fn aot_copy_deep_clones() {
    let src = "## Main\n\
        Let a be [1, 2, 3].\n\
        Let b be copy(a).\n\
        Push 99 to a.\n\
        Show b.\n\
        Show a.\n\
        Let x be 5.\n\
        Let y be copy(x).\n\
        Show y.\n";
    assert_args(src, &["prog"]);
}

/// Extended temporal builtins — SELF-CONTAINED `Moment` arithmetic + calendar/clock extraction, lowered
/// to inline i64/i32 ops that match the VM's `builtins.rs` EXACTLY (no host, no runtime): `seconds_between`
/// / `the seconds between` (`(b-a)/1e9`, truncating), `add_seconds` (`m + n·1e9`, a `Moment` shown via
/// `print_moment`), `the date of` (`div_euclid(NANOS_PER_DAY)` → a `Date`), `the time of`
/// (`rem_euclid(NANOS_PER_DAY)` → a `Time`). Moments are seeded by `parse_timestamp`. The PRE-EPOCH
/// moment (negative nanoseconds) is the load-bearing case: it forces the FLOOR-division / EUCLIDEAN-
/// remainder correction, so a truncating-only lowering (`date_of` off by one, `time_of` negative) fails
/// here. A reversed `seconds_between(b, a)` exercises the negative (truncating-toward-zero) result.
#[test]
fn aot_extended_temporal_matches_the_vm() {
    let src = "## Main\n\
        Let a be parse_timestamp(\"2024-03-10T07:30:00Z\").\n\
        Let b be parse_timestamp(\"2024-03-10T07:31:30Z\").\n\
        Show seconds_between(a, b).\n\
        Show seconds_between(b, a).\n\
        Show the seconds between a and b.\n\
        Show add_seconds(a, 90).\n\
        Show the date of a.\n\
        Show the time of a.\n\
        Let z be parse_timestamp(\"1969-12-31T23:30:00Z\").\n\
        Show the date of z.\n\
        Show the time of z.\n\
        Show seconds_between(z, a).\n";
    assert_args(src, &["prog"]);
}

/// `Show` of `Duration` ARITHMETIC results — `Duration ± Duration = Duration`, so the sum/difference
/// keeps `Kind::Duration` and renders formatted (`8s`/`6s`/`3h`/`45min`), not the raw nanos a plain
/// `Int` result would. Both literal operands and a variable-fed sum exercise the kind propagation.
#[test]
fn aot_shows_duration_arithmetic() {
    let src = "## Main\n\
        Show 5s + 3s.\n\
        Show 10s - 4s.\n\
        Show 2h + 1h.\n\
        Let a be 30min.\n\
        Let b be 15min.\n\
        Show a + b.\n\
        Show a - b.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a whole heterogeneous tuple — the Syntax Guide `tuple-create` example verbatim. The
/// tuple's `(e0, e1, …)` display is assembled inline from the static element layout (Int/Text/Bool
/// each stringified by its own formatter, `", "`-joined, parenthesized) and printed via `print_text`,
/// byte-identical to `RuntimeValue::Tuple`'s deterministic display.
#[test]
fn aot_shows_whole_tuples() {
    let src = "## Main\nLet point be (10, 20).\nLet record be (\"Alice\", 25, true).\nShow point.\nShow record.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a NESTED int sequence (`[[1, 2, 3], [4, 5], [6]]`) — `RuntimeValue::List` of `List`s,
/// rendered `[[1, 2, 3], [4, 5], [6]]` (deterministic: both the VM and AOT store lists in insertion
/// order). Each inner `Seq of Int` stringifies via the existing seq formatter; the outer wraps them in
/// `[…]` with `", "` separators. Ragged inner lengths pin that the inner render reads its own length.
#[test]
fn aot_shows_a_nested_int_sequence() {
    let src = "## Main\nShow [[1, 2, 3], [4, 5], [6]].\n";
    assert_args(src, &["prog"]);
}

/// Nested-seq boundaries: a single inner list (`[[42]]` — no outer separator), an EMPTY inner (`[]`
/// renders with the loop body never running for it), and a nested seq grown by `Push` rather than a
/// literal (proving the Show reads the runtime outer length, not a compile-time layout).
#[test]
fn aot_shows_nested_int_sequence_edge_cases() {
    let src = "## Main\nShow [[42]].\nShow [[], [1], [2, 3]].\n\
               Let mutable grid be [[1, 2]].\nPush [3, 4] to grid.\nPush [5] to grid.\nShow grid.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a whole `Seq of Enum` (`[North, South, East]`) renders `[North, South, East]` — each
/// element's constructor name via the enum's tag→name dispatch, `", "`-joined and bracketed. The
/// element ENUM TYPE (its variant set) is recovered from the seq's construction, not per handle.
#[test]
fn aot_shows_a_nullary_enum_sequence() {
    let src = "## A Dir is one of:\n    A North.\n    A South.\n    An East.\n\n\
               ## Main\nLet xs be [a new North, a new South, a new East].\nShow xs.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a whole `Seq of Word32` / `Seq of Word64` — the crypto state-array display `[u, u, …]`
/// with each element as its UNSIGNED decimal (`4294967295`, not `-1`), matching `RuntimeValue::List`
/// of `Word`. Word32 elements ride the low word of their 8-byte slot; Word64 the full slot.
#[test]
fn aot_shows_a_word32_sequence() {
    let src = "## Main\nLet xs be [word32(1), word32(2), word32(4294967295)].\nShow xs.\n";
    assert_args(src, &["prog"]);
}

#[test]
fn aot_shows_a_word64_sequence() {
    // `word64(0) - word64(1)` wraps to u64::MAX (18446744073709551615) — exercises the high-bit
    // UNSIGNED display without an out-of-i64-range decimal literal.
    let src = "## Main\nLet big be word64(0) - word64(1).\nLet xs be [word64(10), word64(20), big].\nShow xs.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a whole STRUCT — `TypeName { field: val, … }` with fields in DETERMINISTIC (alphabetical)
/// order. The VM's `StructValue.fields` is a `HashMap` (random order), now sorted by field name in
/// `to_display_string`; the AOT sorts the declared fields the same way, so tw == vm == wasm.
#[test]
fn aot_shows_a_struct() {
    let src = "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
               ## Main\nLet p be a new Point with x 10 and y 20.\nShow p.\n";
    assert_args(src, &["prog"]);
}

/// A struct whose DECLARED order (`name`, `age`) differs from alphabetical (`age`, `name`), with a Text
/// field — pins that the display SORTS by field name (`Person { age: 30, name: Alice }`), not slot order,
/// and stringifies a Text field. A single-field struct pins the no-separator boundary.
#[test]
fn aot_shows_a_struct_sorted_with_text() {
    let src = "## A Person has:\n    A name: Text.\n    An age: Int.\n\n\
               ## A Wrapper has:\n    A value: Int.\n\n\
               ## Main\nLet p be a new Person with name \"Alice\" and age 30.\nShow p.\n\
               Let w be a new Wrapper with value 7.\nShow w.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a whole `Seq of Struct` — `[Point { x: 1, y: 2 }, Point { x: 3, y: 4 }]`, each element
/// struct rendered in the same deterministic (alphabetical) field order, insertion order preserved.
#[test]
fn aot_shows_a_struct_sequence() {
    let src = "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
               ## Main\nLet ps be [a new Point with x 1 and y 2, a new Point with x 3 and y 4].\nShow ps.\n";
    assert_args(src, &["prog"]);
}

/// A `Seq of Enum` with PAYLOAD variants (`[Circle(5), Dot, Circle(9)]`) — each element renders as its
/// nullary name OR `Ctor(fields)`, exactly as a single enum `Show` would, assembled per-element inside
/// the sequence loop. Mixes payload and nullary variants; a single-element seq pins the no-separator case.
#[test]
fn aot_shows_a_payload_enum_sequence() {
    let src = "## A Shape is one of:\n    A Circle with radius Int.\n    A Dot.\n\n\
               ## Main\nLet xs be [a new Circle with radius 5, a new Dot, a new Circle with radius 9].\nShow xs.\n\
               Let solo be [a new Dot].\nShow solo.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a whole `Seq of Bool` (`[true, false, true]`) must render `[true, false, true]` —
/// NOT `[1, 0, 1]`. A bool seq rides the `SeqInt` i64-0/1 representation, so the whole-seq display
/// must know the elements are Bools, not Ints (the VM's `ListRepr::Bools` renders `true`/`false`).
#[test]
fn aot_shows_a_bool_sequence() {
    let src = "## Main\nShow [true, false, true].\n";
    assert_args(src, &["prog"]);
}

/// An INDEXED bool element (`item 1 of s`) must also Show as `true`/`false`, not `1`/`0` — the element
/// kind read out of a bool seq is a Bool, not an Int.
#[test]
fn aot_shows_an_indexed_bool_element() {
    let src = "## Main\nLet s be [true, false].\nShow item 1 of s.\nShow item 2 of s.\n";
    assert_args(src, &["prog"]);
}

/// A tuple with a Float element (`(\"Bob\", 30, 5.9)`) — exercises the dedicated `f64` element-temp
/// scratch in the inline tuple stringify (the i64/i32 scratch cannot type a Float slot value).
#[test]
fn aot_shows_a_tuple_with_a_float() {
    let src = "## Main\nLet person be (\"Bob\", 30, 5.9).\nShow person.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a PAYLOAD-carrying enum value — the tree-walker renders `RuntimeValue::Inductive` with
/// non-empty args as `Ctor(arg0, arg1, …)` (positional, `to_display_string` per field). Exercises a
/// single-field variant (`Circle(10)`) and a multi-field variant (`Rectangle(3, 4)`) of the SAME
/// enum: the AOT dispatches on the stored tag and assembles each live variant's `name(fields)` inline.
#[test]
fn aot_shows_a_payload_enum() {
    let src = "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
               ## Main\nLet s be a new Circle with radius 10.\nLet r be a new Rectangle with width 3 and height 4.\n\
               Show s.\nShow r.\n";
    assert_args(src, &["prog"]);
}

/// A single enum type MIXING a nullary variant (`Ping` → bare name) with a payload variant carrying a
/// Text AND an Int field (`Tag(hi, 7)`) — the payload-Show path must (a) still render nullary variants
/// as just the name and (b) stringify a Text field (i32 handle) beside an Int field in one display.
#[test]
fn aot_shows_a_mixed_arity_enum_with_text_field() {
    let src = "## A Msg is one of:\n    A Ping.\n    A Tag with label Text and n Int.\n\n\
               ## Main\nLet a be a new Ping.\nLet b be a new Tag with label \"hi\" and n 7.\n\
               Show a.\nShow b.\n";
    assert_args(src, &["prog"]);
}

/// Payload fields of the non-i64 scalar widths: a Float field (`Temp(5.9)` — needs the dedicated `f64`
/// element temp, the i64/i32 scratch cannot type it) and a Bool field (`Flag(true)`). Locks the width
/// dispatch on the enum payload-Show path, mirroring the tuple-with-a-float coverage.
#[test]
fn aot_shows_a_payload_enum_with_float_and_bool_fields() {
    let src = "## A Reading is one of:\n    A Temp with celsius Float.\n    A Flag with on Bool.\n\n\
               ## Main\nLet a be a new Temp with celsius 5.9.\nLet b be a new Flag with on true.\n\
               Show a.\nShow b.\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a whole MAP — `RuntimeValue::Map::to_display_string` = `{k: v, …}` in INSERTION order.
/// The VM's `MapStorage` is an `IndexMap` and the AOT's linear map appends in that same order, so the
/// rendering is byte-identical. `lower_show_map` iterates the entries in a runtime loop, stringifying
/// each Int key and Int value; the key/value kinds come from the `Set item … of m to …` registers.
#[test]
fn aot_shows_a_map_int_to_int() {
    let src = "## Main\nLet mutable m be a new Map of Int to Int.\n\
               Set item 1 of m to 10.\nSet item 2 of m to 20.\nSet item 3 of m to 30.\nShow m.\n";
    assert_args(src, &["prog"]);
}

/// A map with TEXT keys (`{alice: 1, bob: 2}`) — the key stringify path copies the Text handle rather
/// than formatting a scalar, and a Text key rides the entry's low i32 word. Insertion order preserved.
#[test]
fn aot_shows_a_map_text_to_int() {
    let src = "## Main\nLet mutable m be a new Map of Text to Int.\n\
               Set item \"alice\" of m to 1.\nSet item \"bob\" of m to 2.\nShow m.\n";
    assert_args(src, &["prog"]);
}

/// A map whose VALUES are a non-i64 width — `Map of Text to Float` (`{pi: 3.14, e: 2.72}`) exercises
/// the `f64` value temp + the float value formatter in the entry loop.
#[test]
fn aot_shows_a_map_text_to_float() {
    let src = "## Main\nLet mutable m be a new Map of Text to Float.\n\
               Set item \"pi\" of m to 3.14.\nSet item \"e\" of m to 2.72.\nShow m.\n";
    assert_args(src, &["prog"]);
}

/// A SINGLE-entry map (`{7: 42}`) has no separator — pinning the `", "`-only-between-entries logic at
/// the boundary. (An EMPTY map's `Show` is a sound deferral: with no `Set`, the key/value kinds are
/// unrecoverable — `new Map of K to V` discards its declared type at bytecode emission — and defaulting
/// would be unsound for a map filled through a callee, so the AOT refuses rather than miscompiles.)
#[test]
fn aot_shows_a_singleton_map() {
    let src = "## Main\nLet mutable s be a new Map of Int to Int.\nSet item 7 of s to 42.\nShow s.\n";
    assert_args(src, &["prog"]);
}

/// `SharedSet of Text` field (OR-Set CRDT) — the Syntax Guide `crdt-sharedset` example. The CRDT set
/// field is MUTABLE-SHARED (`Kind::CrdtSetText`, non-cow-clonable), so `Add`/`Remove` on `p's guests`
/// mutate the field IN PLACE (no COW clone), and `contains`/`length` see the result. tw == VM == AOT.
#[test]
fn aot_crdt_sharedset_field() {
    let src = "## A Party is Shared and has:\n    a guests, which is a SharedSet of Text.\n\n\
               ## Main\nLet mutable p be a new Party.\nAdd \"Alice\" to p's guests.\nAdd \"Bob\" to p's guests.\n\
               Remove \"Alice\" from p's guests.\nIf p's guests contains \"Bob\":\n    Show \"Bob is invited\".\n\
               Show length of p's guests.\n";
    assert_args(src, &["prog"]);
}

/// SharedSet fields with AddWins/RemoveWins bias, shown whole — the Syntax Guide `crdt-sharedset-bias`.
/// Each is a mutable-shared `CrdtSetText`; `Show m's tags`/`m's blocked` → `{safe}`/`{spammer}`.
#[test]
fn aot_crdt_sharedset_bias_show() {
    let src = "## A Moderation is Shared and has:\n    a tags, which is a SharedSet (AddWins) of Text.\n    a blocked, which is a SharedSet (RemoveWins) of Text.\n\n\
               ## Main\nLet mutable m be a new Moderation.\nAdd \"safe\" to m's tags.\nAdd \"spammer\" to m's blocked.\n\
               Show m's tags.\nShow m's blocked.\n";
    assert_args(src, &["prog"]);
}

/// Single-replica Divergent register (`NewCrdt` register + `CrdtResolve`) — the Syntax Guide
/// `crdt-divergent` example. On one replica a divergent register just takes its last value: `Set`/
/// `Resolve page's title` overwrite a Text field, `Show page's title` reads it → "Draft" then "Final".
#[test]
fn aot_crdt_divergent_register() {
    let src = "## A WikiPage is Shared and has:\n    a title, which is Divergent Text.\n\n\
               ## Main\nLet mutable page be a new WikiPage.\nSet page's title to \"Draft\".\nShow page's title.\n\
               Resolve page's title to \"Final\".\nShow page's title.\n";
    assert_args(src, &["prog"]);
}

/// Single-replica SharedSequence/RGA (`NewCrdt` seq + `CrdtAppend`) — the Syntax Guide `crdt-sequence`
/// + `crdt-collaborative` shape. On one replica an RGA is a growable list: `Append X to doc's lines`
/// pushes in place (no COW — the CRDT field is mutable-shared), `Show length` → the count.
#[test]
fn aot_crdt_sequence_append() {
    let src = "## A Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n\n\
               ## Main\nLet mutable doc be a new Document.\nAppend \"Line 1\" to doc's lines.\n\
               Append \"Line 2\" to doc's lines.\nAppend \"Line 3\" to doc's lines.\nShow length of doc's lines.\n";
    assert_args(src, &["prog"]);
}

/// `Merge <remote> into <local>` (`CrdtMerge`) of two counter `Shared` structs — the Syntax Guide
/// `crdt-merge` example. For plain-Int counters, `crdt_merge_field` is a SUM, so merging local (100)
/// and remote (50) yields 150 in `local's views` — byte-identical to the tree-walker.
#[test]
fn aot_crdt_merge_counters() {
    let src = "## A Stats is Shared and has:\n    a views, which is ConvergentCount.\n\n\
               ## Main\nLet local be a new Stats.\nIncrease local's views by 100.\n\
               Let remote be a new Stats.\nIncrease remote's views by 50.\n\
               Merge remote into local.\nShow local's views.\n";
    assert_args(src, &["prog"]);
}

/// Single-replica `ConvergentCount` (`CrdtBump`) — the Syntax Guide `crdt-basic` example. On one
/// replica with no merge, `Increase c's points by 10` is a plain struct-field `+=` (the VM stores the
/// counter as an `Int` field), so `Show c's points` → 10, byte-identical to the tree-walker.
#[test]
fn aot_crdt_single_replica_counter() {
    let src = "## A Counter is Shared and has:\n    a points, which is ConvergentCount.\n\n\
               ## Main\nLet c be a new Counter.\nIncrease c's points by 10.\nShow c's points.\n";
    assert_args(src, &["prog"]);
}

/// `Check that <subject> is <predicate>` (`CheckPolicy`) — the Syntax Guide `security-predicate`
/// example. The `## Policy` condition (`the user's role equals "admin"`) is resolved from the
/// registry and compiled inline (field access + byte compare); the check PASSES, so `"Access granted"`
/// prints — byte-identical to the tree-walker. (A failing check would trap, matching the VM error.)
#[test]
fn aot_check_policy_predicate() {
    let src = "## Definition\nA User has:\n    a role: Text.\n\n\
               ## Policy\nA User is admin if the user's role equals \"admin\".\n\n\
               ## Main\nLet u be a new User with role \"admin\".\nCheck that u is admin.\nShow \"Access granted\".\n";
    assert_args(src, &["prog"]);
}

/// `Check that <subject> can <action> <object>` (a capability with an OR of a predicate call and a
/// cross-field compare) — the Syntax Guide `security-capability` example. Exercises the recursive
/// `Predicate`/`Or`/`SubjectFieldEqualsObjectField` condition compiler (alice's name == doc's owner).
#[test]
fn aot_check_policy_capability() {
    let src = "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\n\
               A Document has:\n    an owner: Text.\n\n\
               ## Policy\nA User is admin if the user's role equals \"admin\".\n\
               A User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n\
               ## Main\nLet alice be a new User with name \"Alice\" and role \"editor\".\n\
               Let doc be a new Document with owner \"Alice\".\nCheck that alice can edit doc.\nShow \"Edit permitted\".\n";
    assert_args(src, &["prog"]);
}

/// `Inside a zone …` — a memory-arena scope is SEMANTICALLY TRANSPARENT (the tree-walker binds the
/// zone name to `Nothing` and runs the body; the size is a discarded hint). The AOT runs the body
/// identically, materializing the dead `Nothing` name binding as a dummy. Guide `zone-sized-kb` shape.
#[test]
fn aot_zone_scope_runs_the_body() {
    let src = "## Main\nInside a zone called \"SmallArena\" of size 64 KB:\n    \
               Let x be 42.\n    Let y be 100.\n    Show x + y.\n";
    assert_args(src, &["prog"]);
}

/// `Set of Text` — the Syntax Guide `set-remove` example verbatim. Add/Remove/Show over a set whose
/// elements are strings: dedup + removal compare by BYTE value (not handle identity), and the
/// `{s0, s1, …}` display (insertion order) is byte-identical to `RuntimeValue::Set`.
#[test]
fn aot_set_of_text_add_remove_show() {
    let src = "## Main\nLet colors be a new Set of Text.\n\
               Add \"red\" to colors.\nAdd \"green\" to colors.\nAdd \"blue\" to colors.\n\
               Show colors.\n\
               Remove \"green\" from colors.\nShow colors.\n";
    assert_args(src, &["prog"]);
}

/// `Set of Text` dedup + membership — adding a duplicate string (a DIFFERENT handle, same bytes)
/// must not grow the set, and `contains` compares bytes. Proves byte-equality, not handle identity.
#[test]
fn aot_set_of_text_dedup_and_contains() {
    let src = "## Main\nLet s be a new Set of Text.\n\
               Add \"apple\" to s.\nAdd \"apple\" to s.\nAdd \"pear\" to s.\n\
               Show length of s.\n\
               If s contains \"pear\":\n    Show \"has pear\".\n\
               If s contains \"grape\":\n    Show \"has grape\".\nOtherwise:\n    Show \"no grape\".\n";
    assert_args(src, &["prog"]);
}

/// Whole-`Seq of Int` stringified into a `+` concat — the Syntax Guide `example-filter` shape
/// (`"Positives: " + positives`). The collection formatter host builds `[e0, e1, …]` out of linear
/// memory, byte-identical to `RuntimeValue::List`'s display.
#[test]
fn aot_concat_with_a_whole_seq() {
    let src = "## Main\nLet data be [-2, 5, -1, 8, 3, -4, 7].\nLet positives be a new Seq of Int.\n\
               Repeat for n in data:\n    If n is greater than 0:\n        Push n to positives.\n\
               Show \"Positives: \" + positives.\n";
    assert_args(src, &["prog"]);
}

/// Whole-`Set of Int` stringified into a `+` concat — the Syntax Guide `set-operations` shape
/// (`"Union: " + either`). The `{e0, e1, …}` display is the VM's insertion-ordered Set, which the
/// AOT's union/intersection preserve, so it is byte-identical.
#[test]
fn aot_concat_with_a_whole_set() {
    let src = "## Main\nLet a be a new Set of Int.\nLet b be a new Set of Int.\n\
               Add 1 to a. Add 2 to a. Add 3 to a.\nAdd 2 to b. Add 3 to b. Add 4 to b.\n\
               Let both be a intersection b.\nLet either be a union b.\n\
               Show \"Intersection: \" + both.\nShow \"Union: \" + either.\n";
    assert_args(src, &["prog"]);
}

/// `format(x)` builtin — `x.to_display_string()` as a Text, the Syntax Guide `stdlib-example` shape
/// (`"… = " + format(length of nums)`). Stringifies an Int/Float/Bool arg exactly as a `+` concat
/// operand does; an empty `format()` yields an empty Text.
#[test]
fn aot_format_builtin() {
    let src = "## Main\nLet nums be [5, -3, 8, -1, 4].\nLet text be \"Hello\".\n\
               Show \"length of nums = \" + format(length of nums).\n\
               Show \"abs(-42) = \" + format(abs(-42)).\n\
               Show \"min(10, 3) = \" + format(min(10, 3)).\n\
               Show \"max(10, 3) = \" + format(max(10, 3)).\n";
    assert_args(src, &["prog"]);
}

/// `Show` of a nullary enum value — the Syntax Guide `enum-direction` example verbatim. A nullary
/// variant displays as just its constructor name; the AOT emits a tag→name dispatch over the enum
/// type's variants (the stored tag is the variant name's constant index), printing exactly the live
/// one — byte-identical to `RuntimeValue::Inductive`'s empty-args display.
#[test]
fn aot_shows_a_nullary_enum() {
    let src = "## Definition\nA Direction is either:\n    North.\n    South.\n    East.\n    West.\n\n\
               ## Main\nLet heading be North.\nShow heading.\n";
    assert_args(src, &["prog"]);
}

/// Two different variants of the same enum in one program — proves the tag→name dispatch selects the
/// RIGHT branch per value (`East` → "East", `West` → "West"), not just a single hard-coded name.
#[test]
fn aot_shows_enum_variants_distinctly() {
    let src = "## Definition\nA Direction is either:\n    North.\n    South.\n    East.\n    West.\n\n\
               ## Main\nLet a be East.\nLet b be West.\nShow a.\nShow b.\n";
    assert_args(src, &["prog"]);
}

/// An Int-initialized accumulator promoted to Float — `Let sum be 0` then a `+ <float>` loop. The
/// single def-use web is genuinely Int-then-Float; the AOT promotes the whole register to `f64`.
#[test]
fn aot_int_accumulator_promoted_to_float() {
    let src = format!(
        "{ARGS_PRELUDE}## Main\nLet a be args().\nLet n be parseInt(item 2 of a).\n\
         Let mutable sum be 0.\nLet mutable i be 0.\nWhile i is less than n:\n    \
         Set sum to sum + 1.5.\n    Set i to i + 1.\nShow sum.\n"
    );
    assert_args(&src, &["prog", "5"]);
}

// The REAL benchmark programs, end-to-end: each reads its size from argv, and its output is built
// with `Show "" + … + checksum`. These lock the regsplit range-anchored split (a register reused
// for both a Call argument and a scalar) AND the string-concat path on actual corpus code.
macro_rules! bench_lock {
    ($name:ident, $prog:literal, $arg:literal) => {
        #[test]
        fn $name() {
            let src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs/", $prog, "/main.lg"));
            assert_args(src, &["prog", $arg]);
        }
    };
}
bench_lock!(aot_benchmark_quicksort, "quicksort", "16");
bench_lock!(aot_benchmark_mergesort, "mergesort", "16");
bench_lock!(aot_benchmark_heap_sort, "heap_sort", "16");
bench_lock!(aot_benchmark_binary_trees, "binary_trees", "6");
bench_lock!(aot_benchmark_counting_sort, "counting_sort", "20");
bench_lock!(aot_benchmark_array_reverse, "array_reverse", "9");
bench_lock!(aot_benchmark_histogram, "histogram", "20");
// Collection-op cluster: sieve builds a `Seq of Bool` (rides SeqInt); string_search and strings
// INDEX a Text (`item i of text` → one-char Text). string_search's `n` is large enough to inject a
// real "XXXXX" needle (one match at pos 1000), exercising the byte-equality search with a hit.
bench_lock!(aot_benchmark_sieve, "sieve", "100");
bench_lock!(aot_benchmark_string_search, "string_search", "1010");
bench_lock!(aot_benchmark_strings, "strings", "50");
// Formatted float output (`Show "{e:.9}"` / `"{result:.15}"`): FormatValue with a `.N` precision spec.
bench_lock!(aot_benchmark_nbody, "nbody", "1");
bench_lock!(aot_benchmark_pi_leibniz, "pi_leibniz", "100");
// A nested bitwise `and`/`or`/`not` chain (`all and not (cols or diag1 or diag2)`) whose and/or
// runtime dispatch must monomorphize to bitwise so the result isn't a live Int/Bool register.
bench_lock!(aot_benchmark_nqueens, "nqueens", "6");
// A register the allocator reused for BOTH a 4-arg call's `Seq of Float` argument and a 1-arg `Sqrt`
// builtin's Float argument — the single-arg range must not pin its operand, so it splits off.
bench_lock!(aot_benchmark_spectral_norm, "spectral_norm", "4");

// ════════════════════════════════════════════════════════════════════════════════════════════════
// DIFFERENTIAL FUZZ — the "robust to absurdity" net. The curated corpus + guide + benchmarks pin
// KNOWN programs; this pins the UNKNOWN: a seeded fuzz of RANDOM in-fragment programs, each asserted
// `compile_to_wasm ∘ run == tree-walker == VM`. A miscompile the curated set happens to miss (a
// register-reuse edge, an operator-precedence slip, a codegen off-by-one) shows up here as a diff.
//
// Every generated program is provably OVERFLOW-FREE: each variable is `Let mutable`, seeded in
// [0, 100), and every mutation is immediately reduced `% 1000`, so a value never exceeds 999 and an
// intermediate never exceeds `999 * 4 = 3996` — far under i64. That matters because a standalone
// wasm module TRAPS on i64 overflow while the VM promotes to BigInt (they would diverge), so the
// generator stays where the two agree by construction. Values are always ≥ 0, so `%` matches the
// VM's remainder exactly (no negative-dividend ambiguity).
// ════════════════════════════════════════════════════════════════════════════════════════════════

/// SplitMix64 — a tiny deterministic PRNG (no `Math.random`), matching the JIT differential's fuzz.
fn fuzz_rand(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// One overflow-free mutation of a random variable, at `indent`: a `Set` (`+`/`*`/`+lit`) immediately
/// reduced `% 1000`. Emits two lines; touches only `v0..vnvars` (never a loop counter).
fn fuzz_set(state: &mut u64, nvars: usize, indent: &str) -> String {
    let k = (fuzz_rand(state) % nvars as u64) as usize;
    let j = (fuzz_rand(state) % nvars as u64) as usize;
    let op = match fuzz_rand(state) % 4 {
        0 => format!("v{k} + v{j}"),
        1 => format!("v{k} * {}", 2 + fuzz_rand(state) % 4),
        // Integer division by a non-zero literal on a non-negative dividend — a distinct codegen path
        // (plain `Op::Div`; the value only shrinks, so it stays bounded).
        2 => format!("v{k} / {}", 1 + fuzz_rand(state) % 9),
        _ => format!("v{k} + {}", fuzz_rand(state) % 50),
    };
    format!("{indent}Set v{k} to {op}.\n{indent}Set v{k} to v{k} % 1000.\n")
}

/// Generate a random, syntactically-valid, overflow-free imperative program over the core control-flow
/// + arithmetic fragment: `Let mutable` bindings, flat `Set`s, bounded `While` loops (a fresh counter
/// each), `If/Otherwise` branches, and `Show`. Every value stays in [0, 1000); every loop terminates
/// (its counter increments once per iteration and is compared to a small literal bound).
fn fuzz_program(state: &mut u64) -> String {
    let nvars = 2 + (fuzz_rand(state) % 3) as usize; // 2..=4 variables
    let mut s = String::from("## Main\n");
    for k in 0..nvars {
        s += &format!("Let mutable v{k} be {}.\n", fuzz_rand(state) % 100);
    }
    let nstmts = 3 + (fuzz_rand(state) % 6) as usize; // 3..=8 top-level statements
    let mut loop_id = 0u64;
    for _ in 0..nstmts {
        match fuzz_rand(state) % 5 {
            0 | 1 => s += &fuzz_set(state, nvars, ""),
            2 => {
                // Bounded `While` with a fresh counter — runs `bound + 1` times, then exits.
                let c = loop_id;
                loop_id += 1;
                let bound = 2 + fuzz_rand(state) % 8; // 2..=9
                s += &format!("Let mutable c{c} be 0.\nWhile c{c} is at most {bound}:\n");
                for _ in 0..(1 + fuzz_rand(state) % 2) {
                    s += &fuzz_set(state, nvars, "    ");
                }
                s += &format!("    Set c{c} to c{c} + 1.\n");
            }
            3 => {
                // `If <var> is at most <lit>: … Otherwise: …`, each branch 1..=2 mutations.
                let k = (fuzz_rand(state) % nvars as u64) as usize;
                let lit = fuzz_rand(state) % 1000;
                s += &format!("If v{k} is at most {lit}:\n");
                for _ in 0..(1 + fuzz_rand(state) % 2) {
                    s += &fuzz_set(state, nvars, "    ");
                }
                s += "Otherwise:\n";
                for _ in 0..(1 + fuzz_rand(state) % 2) {
                    s += &fuzz_set(state, nvars, "    ");
                }
            }
            _ => {
                // NESTED control flow — a `While` whose body holds an `If/Otherwise` (dispatch depth 2),
                // where a block-index or br_table off-by-one would surface. Body indents: While=4, If=8.
                let c = loop_id;
                loop_id += 1;
                let bound = 2 + fuzz_rand(state) % 6; // 2..=7
                let k = (fuzz_rand(state) % nvars as u64) as usize;
                let lit = fuzz_rand(state) % 1000;
                s += &format!("Let mutable c{c} be 0.\nWhile c{c} is at most {bound}:\n");
                s += &format!("    If v{k} is at most {lit}:\n");
                s += &fuzz_set(state, nvars, "        ");
                s += "    Otherwise:\n";
                s += &fuzz_set(state, nvars, "        ");
                s += &format!("    Set c{c} to c{c} + 1.\n");
            }
        }
    }
    for k in 0..nvars {
        s += &format!("Show v{k}.\n");
    }
    s
}

/// 400 random programs over arithmetic + control flow (loops, branches), each asserted
/// WASM == tree-walker == VM — the robust-to-absurdity net over the codegen the corpus can't enumerate.
#[test]
fn aot_fuzz_bounded_arithmetic_matches_vm_and_treewalker() {
    for seed in 0..400u64 {
        let mut state = seed ^ 0x1234_5678_9ABC_DEF0;
        let src = fuzz_program(&mut state);
        let tw = tw_outcome(&src);
        assert!(tw.error.is_none(), "tree-walker errored on generated program:\n{src}\nerror: {:?}", tw.error);
        let vm = vm_outcome(&src);
        assert_eq!(vm.error, None, "VM errored on generated program:\n{src}");
        assert_eq!(tw.output.trim(), vm.output.trim(), "tree-walker != VM (base parity) on:\n{src}");
        let module = compile_to_wasm(&src).unwrap_or_else(|e| panic!("compile_to_wasm failed: {e:?}\n{src}"));
        let got = run_aot(&module, vec![]);
        assert_eq!(got.trim(), tw.output.trim(), "WASM AOT != tree-walker on generated program:\n{src}");
    }
}

/// Generate a random SEQ program: build 1-2 `Seq of Int`s by pushing bounded values in a loop, then
/// read each back (bounds-safe 1-based `item i of xs`, sometimes behind a conditional) and Show its
/// length + a running sum. Exercises the HEAP codegen — `new Seq`/`Push`/`length of`/`Index` — where
/// the subtlest bugs lived (amortized push, header layout, 1-based bounds). Overflow-free (every value
/// `% 1000`); every index stays in `[1, length]` so no out-of-bounds trap.
fn fuzz_seq_program(state: &mut u64) -> String {
    let mut s = String::from("## Main\n");
    let nseq = 1 + (fuzz_rand(state) % 2) as usize; // 1..=2 sequences
    for q in 0..nseq {
        let n = 3 + fuzz_rand(state) % 13; // 3..=15 elements
        let mul = 1 + fuzz_rand(state) % 20;
        let add = fuzz_rand(state) % 50;
        s += &format!("Let mutable xs{q} be a new Seq of Int.\n");
        s += &format!("Let mutable b{q} be 1.\nWhile b{q} is at most {n}:\n");
        s += &format!("    Push (b{q} * {mul} + {add}) % 1000 to xs{q}.\n");
        s += &format!("    Set b{q} to b{q} + 1.\n");
    }
    for q in 0..nseq {
        s += &format!("Let mutable sum{q} be 0.\nLet mutable len{q} be length of xs{q}.\nLet mutable i{q} be 1.\n");
        s += &format!("While i{q} is at most len{q}:\n");
        if fuzz_rand(state) % 2 == 0 {
            let thr = fuzz_rand(state) % 1000;
            s += &format!("    If item i{q} of xs{q} is at most {thr}:\n");
            s += &format!("        Set sum{q} to sum{q} + item i{q} of xs{q}.\n");
            s += &format!("        Set sum{q} to sum{q} % 1000.\n");
        } else {
            s += &format!("    Set sum{q} to sum{q} + item i{q} of xs{q}.\n");
            s += &format!("    Set sum{q} to sum{q} % 1000.\n");
        }
        s += &format!("    Set i{q} to i{q} + 1.\n");
    }
    for q in 0..nseq {
        s += &format!("Show length of xs{q}.\nShow sum{q}.\n");
    }
    s
}

/// 250 random seq-building programs, each asserted WASM == tree-walker == VM — the heap-codegen net.
#[test]
fn aot_fuzz_seq_ops_matches_vm_and_treewalker() {
    for seed in 0..250u64 {
        let mut state = seed ^ 0x0FED_CBA9_8765_4321;
        let src = fuzz_seq_program(&mut state);
        let tw = tw_outcome(&src);
        assert!(tw.error.is_none(), "tree-walker errored on generated seq program:\n{src}\nerror: {:?}", tw.error);
        let vm = vm_outcome(&src);
        assert_eq!(vm.error, None, "VM errored on generated seq program:\n{src}");
        assert_eq!(tw.output.trim(), vm.output.trim(), "tree-walker != VM (base parity) on:\n{src}");
        let module = compile_to_wasm(&src).unwrap_or_else(|e| panic!("compile_to_wasm failed: {e:?}\n{src}"));
        let got = run_aot(&module, vec![]);
        assert_eq!(got.trim(), tw.output.trim(), "WASM AOT != tree-walker on generated seq program:\n{src}");
    }
}

/// Emit a bounds-safe read loop summing `name` into a fresh `sum{tag}` var (`% 1000`), then `Show` it.
fn emit_seq_sum(s: &mut String, name: &str, tag: u64) {
    s.push_str(&format!("Let mutable sum{tag} be 0.\nLet mutable len{tag} be length of {name}.\nLet mutable idx{tag} be 1.\n"));
    s.push_str(&format!("While idx{tag} is at most len{tag}:\n"));
    s.push_str(&format!("    Set sum{tag} to sum{tag} + item idx{tag} of {name}.\n"));
    s.push_str(&format!("    Set sum{tag} to sum{tag} % 1000.\n"));
    s.push_str(&format!("    Set idx{tag} to idx{tag} + 1.\n"));
}

/// Generate a VALUE-SEMANTICS / COPY-ON-WRITE stress program: build `xs`, alias it (`Let ys be xs`),
/// then MUTATE `xs` (a `Push` or a `Set item`), and read BOTH back. Logos sequences are value-semantic,
/// so `ys` must be the pre-mutation snapshot — the AOT must copy-on-write `xs`'s buffer at the mutation
/// (the shared word-12 refcount) or `ys` would wrongly see the change. This is the subtlest codegen
/// area (the documented `seq_param_mutate`/store-aliasing fixes); the fuzz pins it across random shapes.
/// Structure is fixed (correct-by-construction), only the sizes/values vary. Overflow-free (`% 1000`).
fn fuzz_cow_program(state: &mut u64) -> String {
    let n = 3 + fuzz_rand(state) % 8; // 3..=10 initial elements
    let mul = 1 + fuzz_rand(state) % 30;
    let mut s = String::from("## Main\n");
    s.push_str("Let mutable xs be a new Seq of Int.\nLet mutable b be 1.\n");
    s.push_str(&format!("While b is at most {n}:\n    Push (b * {mul}) % 1000 to xs.\n    Set b to b + 1.\n"));
    // Value-semantic alias: `ys` is a snapshot of `xs` here.
    s.push_str("Let ys be xs.\n");
    // Optionally chain a second alias off `ys` BEFORE mutating, so a whole COW chain is exercised.
    let chain = fuzz_rand(state) % 2 == 0;
    if chain {
        s.push_str("Let zs be ys.\n");
    }
    // Mutate `xs` — COW must protect `ys` (and `zs`).
    match fuzz_rand(state) % 2 {
        0 => s.push_str(&format!("Push {} to xs.\n", fuzz_rand(state) % 1000)),
        _ => s.push_str(&format!("Set item 1 of xs to {}.\n", fuzz_rand(state) % 1000)),
    }
    emit_seq_sum(&mut s, "xs", 0);
    emit_seq_sum(&mut s, "ys", 1);
    if chain {
        emit_seq_sum(&mut s, "zs", 2);
    }
    s.push_str("Show length of xs.\nShow length of ys.\n");
    if chain {
        s.push_str("Show length of zs.\n");
    }
    s
}

/// 250 random value-semantics/COW aliasing programs, each asserted WASM == tree-walker == VM — the
/// copy-on-write net, the subtlest heap-codegen area.
#[test]
fn aot_fuzz_value_semantics_cow_matches_vm_and_treewalker() {
    for seed in 0..250u64 {
        let mut state = seed ^ 0xA5A5_5A5A_C3C3_3C3C;
        let src = fuzz_cow_program(&mut state);
        let tw = tw_outcome(&src);
        assert!(tw.error.is_none(), "tree-walker errored on generated COW program:\n{src}\nerror: {:?}", tw.error);
        let vm = vm_outcome(&src);
        assert_eq!(vm.error, None, "VM errored on generated COW program:\n{src}");
        assert_eq!(tw.output.trim(), vm.output.trim(), "tree-walker != VM (base parity) on:\n{src}");
        let module = compile_to_wasm(&src).unwrap_or_else(|e| panic!("compile_to_wasm failed: {e:?}\n{src}"));
        let got = run_aot(&module, vec![]);
        assert_eq!(got.trim(), tw.output.trim(), "WASM AOT != tree-walker on generated COW program:\n{src}");
    }
}

/// A short ASCII token (no quotes/backslashes/newlines) — a fuzz word for the text-concat generator.
fn fuzz_word(state: &mut u64) -> &'static str {
    const WORDS: &[&str] = &["cat", "dog", "42", "Zz", "hello", "a", "world", "QP", "xyz", "7", "Node", "-!", "  "];
    WORDS[(fuzz_rand(state) % WORDS.len() as u64) as usize]
}

/// Generate a random TEXT program: start from a literal and append a random sequence of tokens with
/// `Set t to t + "<word>"`, then `Show` the string and its length. Exercises the text codegen — literal
/// materialization, `Concat` (buffer growth + byte copy), and `length of` — where buffer-sizing /
/// byte-copy bugs would surface. The output is the exact concatenation, so tw/vm/aot must agree byte
/// for byte. ASCII-only so `length` (bytes) equals the character count on every engine.
fn fuzz_text_program(state: &mut u64) -> String {
    let mut s = String::from("## Main\n");
    s.push_str(&format!("Let mutable t be \"{}\".\n", fuzz_word(state)));
    let n = 2 + fuzz_rand(state) % 8; // 2..=9 appends
    for _ in 0..n {
        s.push_str(&format!("Set t to t + \"{}\".\n", fuzz_word(state)));
    }
    s.push_str("Show t.\nShow length of t.\n");
    s
}

/// A `SharedMap from K to V` struct field (single-replica ORMap CRDT) must resolve to the AOT's
/// `Map` layout so the program COMPILES — this is the `crdt-sharedmap` Syntax-Guide example verbatim.
/// Regression (two coupled bugs): (1) the compiler's `boundary_of_field_type` matched only
/// `Map`/`HashMap`, so a `SharedMap` field returned `None` → the whole struct fell out of
/// `struct_types`; and (2) `GetField` on a locally-built struct took the field's kind from the
/// (placeholder) inserted value rather than the DECLARED type, so even once the struct was carried,
/// the map field didn't type as `Map`. Either way `Set s's m[k]` mis-lowered through the sequence
/// path → compile error "unsupported sequence of unknown element kind".
///
/// This example is COMPILE-ONLY (`REQUIRES_COMPILATION`): a single-replica SharedMap's field default
/// is materialized only by the full Rust-compile path — NO interpreter tier models it, so the
/// tree-walker AND the VM both error ("Cannot index into Nothing"). The guide ratchet's contract for
/// this example is therefore COMPILATION; there is no run oracle (the WASM module traps at runtime
/// exactly as tw/vm error, so WASM stays consistent with them). The `assert`s below are guardrails:
/// if any interpreter tier ever learns to RUN a SharedMap field, they fire so we add a run oracle.
#[test]
fn aot_shared_map_struct_field_compiles() {
    let src = "## Definition\nA Inventory is Shared and has:\n an items, which is a SharedMap from Text to Int.\n\n## Main\nLet mutable inv be a new Inventory.\nSet inv's items[\"wood\"] to 50.\nSet inv's items[\"stone\"] to 30.\nShow inv's items[\"wood\"].\n";
    assert!(tw_outcome(src).error.is_some(), "tree-walker unexpectedly RAN a SharedMap field — add a run-oracle assertion here\n{src}");
    assert!(vm_outcome(src).error.is_some(), "VM unexpectedly RAN a SharedMap field — add a run-oracle assertion here\n{src}");
    compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed on a SharedMap struct field: {e:?}\n{src}"));
}

/// MD5 compress written entirely IN LOGOS — the AAA proof that a REAL hash algorithm compiles to
/// native WASM. It exercises the whole scalar crypto substrate end-to-end: the `Word32` ℤ/2³² ring
/// (`word32`/`intOfWord32`, wrapping `+`, `xor`, `rotl`), the cross-tier `word_and`/`word_or`/
/// `word_not` builtins, `Seq of Word32` state/schedule arrays (build/`item N`/iterate), `Seq of
/// Word32` FUNCTION PARAMETERS (`md5Compress(state, m, kk)`), and `Repeat for i from A to B` range
/// loops with `item (i+1) of kk` mixed Int-index-into-Word-seq. The digest of the "abc"-block state
/// equals Python hashlib's, so the entire ℤ/2³² arithmetic chain is bit-exact: WASM == VM ==
/// tree-walker == reference. A REAL hash algorithm, written in Logos, compiling to native WebAssembly.
///
/// The base compiler's block-scope register recycling packs an `Int` loop-index temp and a `Word32`
/// arithmetic temp into one slot with disjoint live ranges (once as a `Rotl` shift-count, once as a
/// `Rotl` word value), which the per-register kind inference could not give a single wasm local type.
/// The [`regsplit`](super) live-range splitter (def-use webs + argument materialization) separates
/// them into distinct locals before inference, so the whole function lowers with no `i32`/`i64` reuse.
#[test]
fn aot_md5_compress_in_logos_matches_reference() {
    let src = include_str!("fixtures/md5_logos.lg");
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker errored on MD5-in-Logos");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM errored on MD5-in-Logos");
    assert_eq!(tw.output.trim(), vm.output.trim(), "tree-walker != VM on MD5-in-Logos");
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed on MD5-in-Logos: {e:?}"));
    let got = run_aot(&module, vec![]);
    assert_eq!(got.trim(), tw.output.trim(), "AOT wasm != tree-walker on MD5-in-Logos");
    assert_eq!(
        got.trim(),
        "2555380112\n2958021180\n2101319382\n1920983336",
        "MD5-in-Logos AOT digest mismatch vs the hashlib reference"
    );
}

/// SHA-256 compress written entirely IN LOGOS — the second crypto AAA proof, and a HARDER one than
/// MD5: it exercises the full 64-round Merkle–Damgård core with the 48-word message-schedule
/// extension, the eight working variables, six helper functions taking SCALAR `Word32` parameters
/// (`ssig0`/`ssig1`/`bsig0`/`bsig1`/`ch`/`maj`), and — the new primitive — `word32Shr` (a LOGICAL
/// right-shift, `i32.shr_u`, NOT a rotate: the vacated high bits are zero) that `σ0`/`σ1` need. The
/// input is the padded single 512-bit block of "abc"; the eight output words equal Python hashlib's
/// `sha256(b"abc")`, so the whole ℤ/2³² schedule + compression chain is bit-exact: WASM == VM ==
/// tree-walker == reference. A second real hash algorithm, in Logos, compiling to native WebAssembly.
#[test]
fn aot_sha256_compress_in_logos_matches_reference() {
    let src = include_str!("fixtures/sha256_logos.lg");
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker errored on SHA-256-in-Logos");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM errored on SHA-256-in-Logos");
    assert_eq!(tw.output.trim(), vm.output.trim(), "tree-walker != VM on SHA-256-in-Logos");
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed on SHA-256-in-Logos: {e:?}"));
    let got = run_aot(&module, vec![]);
    assert_eq!(got.trim(), tw.output.trim(), "AOT wasm != tree-walker on SHA-256-in-Logos");
    // The 8 big-endian words of SHA-256("abc") = ba7816bf 8f01cfea 414140de 5dae2223 b00361a3
    // 96177a9c b410ff61 f20015ad, in decimal.
    assert_eq!(
        got.trim(),
        "3128432319\n2399260650\n1094795486\n1571693091\n2953011619\n2518121116\n3021012833\n4060091821",
        "SHA-256-in-Logos AOT digest mismatch vs the hashlib reference"
    );
}

/// SHA-512 compress written entirely IN LOGOS — the crypto AAA proof extended to the `Word64` (ℤ/2⁶⁴)
/// substrate: the 80-round core, the 64-word schedule extension, and six helper functions on scalar
/// `Word64` params (`ls0`/`ls1`/`bs0`/`bs1`/`ch`/`maj`) using `rotr`/`word64Shr`/`word_and`/`word_not`/
/// `xor`/wrapping `+` on 64-bit words. It also exercises the new FULL-64-BIT hex literal support — the
/// 80 round constants are written `word64(0x…)` with the high bit set (`0xb5c0fbcfec4d3b2f` etc.), which
/// the parser now reads across the whole u64 range (reinterpreted as the i64 bit-pattern `word64`
/// consumes). The input is the padded single 1024-bit block of "abc"; the eight output words equal
/// Python hashlib's `sha512(b"abc")` (shown as signed decimals since `intOfWord64` is `u64 as i64`), so
/// the whole ℤ/2⁶⁴ chain is bit-exact: WASM == VM == tree-walker == reference.
#[test]
fn aot_sha512_compress_in_logos_matches_reference() {
    let src = include_str!("fixtures/sha512_logos.lg");
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker errored on SHA-512-in-Logos");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM errored on SHA-512-in-Logos");
    assert_eq!(tw.output.trim(), vm.output.trim(), "tree-walker != VM on SHA-512-in-Logos");
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed on SHA-512-in-Logos: {e:?}"));
    let got = run_aot(&module, vec![]);
    assert_eq!(got.trim(), tw.output.trim(), "AOT wasm != tree-walker on SHA-512-in-Logos");
    // The 8 big-endian 64-bit words of SHA-512("abc") = ddaf35a1 93617aba … as `u64 as i64` decimals.
    assert_eq!(
        got.trim(),
        "-2472698702324467014\n-3728572256194903759\n1362051152550133410\n765311659573367706\n\
         2419164356178592168\n3943530547489205181\n4993722480620005390\n3069987439919277215",
        "SHA-512-in-Logos AOT digest mismatch vs the hashlib reference"
    );
}

/// A `Word64` constant may use the FULL unsigned 64-bit range via a hex literal (the crux SHA-512
/// needs). `0xFFFFFFFFFFFFFFFF` is the all-ones word (its `intOfWord64` is `-1` = `u64::MAX as i64`),
/// and `0xB5C0FBCFEC4D3B2F` (a real SHA-512 constant, high bit set) shifts right by 60 to its top
/// nibble `0xB` = 11. Proven WASM == VM == tree-walker, i.e. the parser reinterprets the u64 literal
/// as the i64 bit-pattern `word64` consumes, identically on every tier.
#[test]
fn aot_full_u64_hex_word64_literal() {
    let src = "## Main\n    Let x be word64(0xFFFFFFFFFFFFFFFF).\n    Show x.\n    Show intOfWord64(x).\n    \
               Let y be word64(0xB5C0FBCFEC4D3B2F).\n    Show word64Shr(y, 60).\n";
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker errored on the u64 hex literal");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM errored on the u64 hex literal");
    assert_eq!(tw.output.trim(), vm.output.trim(), "tree-walker != VM on the u64 hex literal");
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("compile_to_wasm failed on the u64 hex literal: {e:?}"));
    let got = run_aot(&module, vec![]);
    assert_eq!(got.trim(), tw.output.trim(), "AOT wasm != tree-walker on the u64 hex literal");
    // `Show x` = the word UNSIGNED (u64::MAX); `intOfWord64(x)` = the same bits SIGNED (-1);
    // `word64Shr(0xB5…, 60)` = the top nibble 0xB = 11.
    assert_eq!(got.trim(), "18446744073709551615\n-1\n11", "full-u64 hex Word64 literal mismatch");
}

/// 250 random text-concat programs, each asserted WASM == tree-walker == VM — the text-codegen net.
#[test]
fn aot_fuzz_text_concat_matches_vm_and_treewalker() {
    for seed in 0..250u64 {
        let mut state = seed ^ 0x7777_1111_DDDD_2222;
        let src = fuzz_text_program(&mut state);
        let tw = tw_outcome(&src);
        assert!(tw.error.is_none(), "tree-walker errored on generated text program:\n{src}\nerror: {:?}", tw.error);
        let vm = vm_outcome(&src);
        assert_eq!(vm.error, None, "VM errored on generated text program:\n{src}");
        assert_eq!(tw.output.trim(), vm.output.trim(), "tree-walker != VM (base parity) on:\n{src}");
        let module = compile_to_wasm(&src).unwrap_or_else(|e| panic!("compile_to_wasm failed: {e:?}\n{src}"));
        let got = run_aot(&module, vec![]);
        assert_eq!(got.trim(), tw.output.trim(), "WASM AOT != tree-walker on generated text program:\n{src}");
    }
}
