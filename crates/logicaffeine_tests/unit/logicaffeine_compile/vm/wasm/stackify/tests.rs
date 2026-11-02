use super::*;
use std::cell::RefCell;
use std::rc::Rc;

const I64: u8 = 0x7E;
/// Locals `0..FREE` are the generated code's variables; `FREE + depth` counts the iterations of a
/// loop at that nesting depth, and nothing else writes it; from `TEMPS` on, each local is a
/// temporary, stored once and read where the generator chooses — the register traffic the
/// emitter's lowering produces and stackify exists to remove.
const FREE: u8 = 7;
const TEMPS: u8 = FREE + 3;
const LOCALS: u8 = TEMPS + 40;

const OBSERVE: [u8; 2] = [0x10, 0x00];
const ADD: [u8; 1] = [0x7C];
const MUL: [u8; 1] = [0x7E];
const DROP: [u8; 1] = [0x1A];
const LOOP: [u8; 2] = [0x03, 0x40];
const END: [u8; 1] = [0x0B];

fn get(x: u8) -> [u8; 2] {
    [0x20, x]
}
fn set(x: u8) -> [u8; 2] {
    [0x21, x]
}
fn tee(x: u8) -> [u8; 2] {
    [0x22, x]
}
fn konst(v: u8) -> [u8; 2] {
    assert!(v < 64, "a one-byte sleb");
    [0x42, v]
}

macro_rules! seq {
    ($($part:expr),* $(,)?) => {{
        let mut v: Vec<u8> = Vec::new();
        $( v.extend_from_slice(&$part); )*
        v
    }};
}

/// A function body over `LOCALS` i64 locals: the locals prefix, `code`, the closing `end`.
fn body(code: &[u8]) -> Vec<u8> {
    let mut b = vec![0x01, LOCALS, I64];
    b.extend_from_slice(code);
    b.push(0x0B);
    b
}

fn leb(out: &mut Vec<u8>, mut v: usize) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn section(m: &mut Vec<u8>, id: u8, payload: &[u8]) {
    m.push(id);
    leb(m, payload.len());
    m.extend_from_slice(payload);
}

/// A module whose export `f: () -> ()` is `body`, and whose one import `env.observe(i64)` is the
/// function `call 0` names.
fn module(body: &[u8]) -> Vec<u8> {
    let mut m = b"\0asm\x01\0\0\0".to_vec();
    section(&mut m, 1, &[0x02, 0x60, 0x01, I64, 0x00, 0x60, 0x00, 0x00]);
    section(
        &mut m,
        2,
        &seq![[0x01, 0x03], *b"env", [0x07], *b"observe", [0x00, 0x00]],
    );
    section(&mut m, 3, &[0x01, 0x01]);
    section(&mut m, 7, &[0x01, 0x01, b'f', 0x00, 0x01]);
    let mut code = vec![0x01];
    leb(&mut code, body.len());
    code.extend_from_slice(body);
    section(&mut m, 10, &code);
    m
}

/// What `body` passes to `observe`, in order, run by a real engine — which also validates it — on
/// a fuel budget no generated body comes near, so a rewrite that breaks a loop's exit fails here
/// instead of hanging.
fn observed(body: &[u8]) -> Vec<i64> {
    let mut config = wasmi::Config::default();
    config.consume_fuel(true);
    let engine = wasmi::Engine::new(&config);
    let module = wasmi::Module::new(&engine, &module(body)[..])
        .unwrap_or_else(|e| panic!("the body does not validate: {e}\n{body:02x?}"));
    let out: Rc<RefCell<Vec<i64>>> = Rc::new(RefCell::new(Vec::new()));
    let mut store = wasmi::Store::new(&engine, out.clone());
    store.add_fuel(10_000_000).expect("fuel metering is on");
    let mut linker = wasmi::Linker::new(&engine);
    linker
        .func_wrap(
            "env",
            "observe",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<i64>>>>, v: i64| {
                c.data().borrow_mut().push(v);
            },
        )
        .unwrap();
    let instance = linker
        .instantiate(&mut store, &module)
        .unwrap()
        .start(&mut store)
        .unwrap();
    instance
        .get_typed_func::<(), ()>(&store, "f")
        .unwrap()
        .call(&mut store, ())
        .unwrap_or_else(|e| panic!("the body stopped: {e}\n{body:02x?}"));
    let r = out.borrow().clone();
    r
}

/// `code`'s body stackified, checked to observe exactly what `code` does.
fn stackified(code: &[u8]) -> Vec<u8> {
    let before = body(code);
    let after = stackify(before.clone());
    assert_eq!(
        observed(&after),
        observed(&before),
        "stackify changed what the body does:\n{before:02x?}\n{after:02x?}"
    );
    after
}

#[test]
fn a_value_read_once_right_after_its_store_stays_on_the_stack() {
    let code = seq![get(0), get(0), ADD, set(1), get(1), OBSERVE];
    assert_eq!(stackified(&code), body(&seq![get(0), get(0), ADD, OBSERVE]));
}

#[test]
fn a_chain_of_such_stores_goes_whole() {
    let code = seq![
        konst(5),
        set(1),
        get(1),
        set(2),
        get(2),
        set(3),
        get(3),
        OBSERVE
    ];
    assert_eq!(stackified(&code), body(&seq![konst(5), OBSERVE]));
}

#[test]
fn a_store_nothing_reads_is_a_drop() {
    let code = seq![konst(5), set(1), get(0), OBSERVE];
    assert_eq!(
        stackified(&code),
        body(&seq![konst(5), DROP, get(0), OBSERVE])
    );
}

#[test]
fn a_local_read_twice_keeps_its_value_for_the_second_read() {
    let code = seq![konst(5), set(1), get(1), OBSERVE, get(1), OBSERVE];
    stackified(&code);
    assert_eq!(observed(&body(&code)), [5, 5]);
}

/// The one read of a local stored twice sees the second store: forwarding the first store's
/// value to it would read a value the local no longer holds.
#[test]
fn a_local_stored_twice_answers_its_second_store() {
    let code = seq![
        konst(7),
        set(1),
        konst(8),
        set(1),
        get(0),
        OBSERVE,
        get(1),
        OBSERVE
    ];
    stackified(&code);
    assert_eq!(observed(&body(&code)), [0, 8]);
}

#[test]
fn a_constant_moves_to_its_only_read_later_in_the_same_run() {
    let code = seq![konst(7), set(1), get(0), OBSERVE, get(1), OBSERVE];
    assert_eq!(
        stackified(&code),
        body(&seq![get(0), OBSERVE, konst(7), OBSERVE])
    );
}

#[test]
fn a_copied_local_moves_to_the_copys_only_read() {
    let code = seq![get(0), set(1), konst(3), OBSERVE, get(1), OBSERVE];
    assert_eq!(
        stackified(&code),
        body(&seq![konst(3), OBSERVE, get(0), OBSERVE])
    );
}

/// The copy's source is overwritten before the copy is read: reading the source there instead
/// would read the new value.
#[test]
fn a_copy_read_after_its_source_is_stored_again_answers_the_old_value() {
    let code = seq![
        konst(9),
        set(0),
        get(0),
        set(1),
        konst(3),
        set(0),
        get(1),
        OBSERVE,
        get(0),
        OBSERVE
    ];
    stackified(&code);
    assert_eq!(observed(&body(&code)), [9, 3]);
}

#[test]
fn a_copy_read_after_its_source_is_teed_again_answers_the_old_value() {
    let code = seq![
        konst(9),
        set(0),
        get(0),
        set(1),
        konst(3),
        tee(0),
        OBSERVE,
        get(1),
        OBSERVE,
        get(0),
        OBSERVE
    ];
    stackified(&code);
    assert_eq!(observed(&body(&code)), [3, 9, 3]);
}

/// The copy is read inside a loop whose body then overwrites the source: a later iteration reads
/// the copy's value, which the source no longer holds. Control flow ends the run a read may move
/// within.
#[test]
fn a_copy_read_in_a_loop_that_stores_its_source_answers_the_old_value() {
    const NE: [u8; 1] = [0x52];
    const BR_IF_0: [u8; 2] = [0x0D, 0x00];
    let code = seq![
        konst(9),
        set(0),
        get(0),
        set(1),
        LOOP,
        get(1),
        OBSERVE,
        get(0),
        konst(1),
        ADD,
        set(0),
        get(0),
        konst(11),
        NE,
        BR_IF_0,
        END,
    ];
    stackified(&code);
    assert_eq!(observed(&body(&code)), [9, 9]);
}

#[test]
fn a_body_the_decoder_does_not_know_is_left_as_it_is() {
    const REF_NULL_FUNC: [u8; 2] = [0xD0, 0x70];
    let code = seq![get(0), set(1), get(1), OBSERVE, REF_NULL_FUNC, DROP];
    assert_eq!(stackify(body(&code)), body(&code));
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Where a generated body stands: the next unused temporary, and the temporaries stored and not
/// yet read.
struct Temps {
    next: u8,
    pending: Vec<u8>,
}

/// Valid straight-line and structured code that leaves the stack as it found it: pushes, stores,
/// tees, arithmetic, drops and observations over the variables; temporaries stored and read back
/// at once, later in the run, inside a later construct, twice, or never; and `block`/`br_if`,
/// `if`/`else` and counted loops nested up to three deep — entered at any stack height, since a
/// construct's body starts on a stack of its own.
fn generate(rng: &mut Rng, depth: u8, temps: &mut Temps, out: &mut Vec<u8>) {
    let mut height = 0u32;
    for _ in 0..rng.below(14) + 2 {
        let x = rng.below(u64::from(FREE)) as u8;
        match rng.below(16) {
            0 | 1 => {
                out.extend_from_slice(&konst(rng.below(64) as u8));
                height += 1;
            }
            2 | 3 => {
                out.extend_from_slice(&get(x));
                height += 1;
            }
            4 if height >= 1 => {
                out.extend_from_slice(&set(x));
                height -= 1;
            }
            5 if height >= 1 => out.extend_from_slice(&tee(x)),
            6 if height >= 2 => {
                out.extend_from_slice(if rng.below(2) == 0 { &ADD } else { &MUL });
                height -= 1;
            }
            7 if height >= 1 => {
                out.extend_from_slice(if rng.below(3) == 0 {
                    &DROP[..]
                } else {
                    &OBSERVE[..]
                });
                height -= 1;
            }
            8 | 9 if height >= 1 && temps.next < LOCALS => {
                // A temporary: stored, and read back at once or left for a later read.
                let t = temps.next;
                temps.next += 1;
                out.extend_from_slice(&set(t));
                if rng.below(2) == 0 {
                    out.extend_from_slice(&get(t));
                } else {
                    temps.pending.push(t);
                    height -= 1;
                }
            }
            10 | 11 if !temps.pending.is_empty() => {
                // A read of a stored temporary; now and then it stays readable for a second read.
                let i = rng.below(temps.pending.len() as u64) as usize;
                let t = if rng.below(4) == 0 {
                    temps.pending[i]
                } else {
                    temps.pending.swap_remove(i)
                };
                out.extend_from_slice(&get(t));
                height += 1;
            }
            12 if depth < 3 => {
                // block; …; br_if 0 on (x == 0); …; end
                out.extend_from_slice(&[0x02, 0x40]);
                generate(rng, depth + 1, temps, out);
                out.extend_from_slice(&seq![get(x), [0x50, 0x0D, 0x00]]);
                generate(rng, depth + 1, temps, out);
                out.extend_from_slice(&END);
            }
            13 if depth < 3 => {
                // if (x == 0) …; else …; end
                out.extend_from_slice(&seq![get(x), [0x50, 0x04, 0x40]]);
                generate(rng, depth + 1, temps, out);
                out.push(0x05);
                generate(rng, depth + 1, temps, out);
                out.extend_from_slice(&END);
            }
            14 if depth < 3 => {
                // counter = 1..=3; loop …; counter -= 1; br_if 0 on (counter != 0); end
                let counter = FREE + depth;
                out.extend_from_slice(&seq![konst(1 + rng.below(3) as u8), set(counter), LOOP]);
                generate(rng, depth + 1, temps, out);
                out.extend_from_slice(&seq![
                    get(counter),
                    konst(1),
                    [0x7D],
                    tee(counter),
                    [0x50, 0x45, 0x0D, 0x00],
                    END
                ]);
            }
            _ => {}
        }
    }
    for _ in 0..height {
        out.extend_from_slice(&OBSERVE);
    }
}

/// ★ Over thousands of generated bodies, the stackified body validates, observes exactly what the
/// original does, and is never longer — and the corpus is one stackify has work in.
#[test]
fn every_generated_body_observes_the_same_stackified() {
    let mut rng = Rng(0x5EED_57AC);
    let (mut changed, mut saved) = (0usize, 0usize);
    const BODIES: usize = 4000;
    for _ in 0..BODIES {
        let mut code = Vec::new();
        generate(
            &mut rng,
            0,
            &mut Temps {
                next: TEMPS,
                pending: Vec::new(),
            },
            &mut code,
        );
        let before = body(&code);
        let after = stackified(&code);
        assert!(
            after.len() <= before.len(),
            "stackify grew a body:\n{before:02x?}\n{after:02x?}"
        );
        if after != before {
            changed += 1;
            saved += before.len() - after.len();
        }
    }
    assert!(
        changed > BODIES / 2,
        "stackify changed only {changed} of {BODIES} bodies — the corpus stopped exercising it"
    );
    assert!(
        saved > BODIES,
        "stackify saved only {saved} bytes over {BODIES} bodies"
    );
}
