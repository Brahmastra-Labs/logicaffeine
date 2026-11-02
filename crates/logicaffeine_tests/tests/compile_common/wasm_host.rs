//! ★ THE ONE IMPLEMENTATION of every wasm host import a test cannot have an opinion about. ★
//!
//! A compiled LOGOS module reaches the outside world through imported functions, so every wasm test
//! must supply a host. Some of those imports genuinely differ per test — which node id
//! `dom_create_element` hands back, whether `dom_set_text` records into an op-log — and a test
//! supplying its own is the test doing its job.
//!
//! ★ THE FORMATTERS ARE NOT LIKE THAT. `fmt_i64_into(buf, val) -> i32` writes the decimal of `val`
//! into `buf` and returns THE BYTE LENGTH. There is one right answer and a test can only get it
//! right or wrong. Seventeen test files used to write these out by hand, independently, with nothing
//! anywhere comparing them to the ABI or to `runtime.js`.
//!
//! ★★★ AND ONE OF THEM WAS WRONG, WHICH IS WHY THIS FILE EXISTS. `wasm_memory_bound_lock` returned
//! `buf` — the POINTER — where the contract says LENGTH. The pointer grows with the heap; it landed
//! in a Text header as `len`; the next concat allocated `len_a + len_b`, a buffer the size of the
//! whole heap. Memory doubled every turn and a page "died after twelve clicks". The measurements
//! were real, the curve was real, and the product was fine. That false diagnosis spent a task, an
//! unsound experiment, and a design for a two-region allocator that was never needed.
//!
//! ★ WHY IT SURVIVED SO LONG, which decides where the fix belongs: the broken stub was in the one
//! test that never checked its output. Every other file asserts on rendered text, so a formatter
//! returning nonsense fails immediately and loudly there. A test asking only "did it survive?"
//! cannot see its own oracle lying — so "write the host carefully" is not a fix, and "assert on
//! output" is not always available. Implementing it ONCE is.
//!
//! ★ DERIVED FROM THE PRODUCT, NOT RE-DESCRIBED. A float goes through
//! `compile::display_float_like_logos`, the same shortest-round-trip path
//! `RuntimeValue::Float::to_display_string` uses — so this host cannot drift from what LOGOS
//! actually displays, which a hand-written `format!("{v}")` silently would (it prints `2` as `2`
//! and LOGOS prints `2`, but `1e21` and subnormals diverge). A second description of a behaviour is
//! a second thing to keep right.
//!
//! `wasm_host_stub_lock` holds the other half: no test may hand-write one of these again.

use super::logicaffeine_compile;
use wasmi::{Caller, Linker};

/// The wasmi value type of a host-import table's value-type byte.
pub fn value_type(code: u8) -> wasmi::core::ValueType {
    match code {
        0x7F => wasmi::core::ValueType::I32,
        0x7E => wasmi::core::ValueType::I64,
        0x7D => wasmi::core::ValueType::F32,
        0x7C => wasmi::core::ValueType::F64,
        other => {
            panic!("the host-import table names value type 0x{other:02x}, which no host can link")
        }
    }
}

/// ★ CALLED LAST BY EVERY HARNESS: links each declared host import (`host_import_signatures`) the
/// harness has not linked itself as a stub that traps NAMING the import, typed from the module's own
/// table. A module importing it still instantiates, a page that actually calls it fails by name, and
/// a host import added to the table can never break a harness at instantiate time. A name the harness
/// already linked keeps the harness's own definition.
pub fn link_unmodelled_imports<T: 'static>(l: &mut Linker<T>) {
    for (name, params, results) in logicaffeine_compile::vm::wasm::host_import_signatures() {
        let ty = wasmi::FuncType::new(
            params.iter().map(|&p| value_type(p)),
            results.iter().map(|&r| value_type(r)),
        );
        let trap = move |_c: Caller<'_, T>, _args: &[wasmi::Value], _out: &mut [wasmi::Value]| {
            Err(wasmi::core::Trap::new(format!(
                "this test host does not model `{name}`"
            )))
        };
        match l.func_new("env", name, ty, trap) {
            Ok(_) | Err(wasmi::errors::LinkerError::DuplicateDefinition { .. }) => {}
            Err(other) => panic!("linking `{name}`: {other}"),
        }
    }
}

/// Appends `n` as an unsigned LEB128 — how the wasm binary format writes every count and index.
fn leb128(out: &mut Vec<u8>, mut n: u32) {
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A module that imports every row of `table` from `env` with exactly the row's signature and does
/// nothing else: it instantiates against a host iff that host links the whole table.
pub fn import_every_row(table: &[(&str, Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut types = Vec::new();
    leb128(&mut types, table.len() as u32);
    for (_, params, results) in table {
        types.push(0x60);
        leb128(&mut types, params.len() as u32);
        types.extend_from_slice(params);
        leb128(&mut types, results.len() as u32);
        types.extend_from_slice(results);
    }
    let mut imports = Vec::new();
    leb128(&mut imports, table.len() as u32);
    for (index, (name, _, _)) in table.iter().enumerate() {
        leb128(&mut imports, 3);
        imports.extend_from_slice(b"env");
        leb128(&mut imports, name.len() as u32);
        imports.extend_from_slice(name.as_bytes());
        imports.push(0x00);
        leb128(&mut imports, index as u32);
    }
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    for (id, body) in [(1u8, types), (2u8, imports)] {
        module.push(id);
        leb128(&mut module, body.len() as u32);
        module.extend(body);
    }
    module
}

/// Write `text` into the module's linear memory at `buf` and answer its BYTE LENGTH — the contract
/// every `fmt_*_into` import shares, in one place, so no caller can get the return value wrong.
fn write_into<T>(c: &mut Caller<'_, T>, buf: i32, text: &str) -> i32 {
    let mem = c
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("a module using a formatter has a heap, and a heap module exports its memory");
    mem.write(&mut *c, buf as usize, text.as_bytes())
        .expect("the module allocates the scratch buffer it passes, so a write inside it must fit");
    text.len() as i32
}

/// Allocate a `Text` block through the MODULE's own `__logos_text_new` export and write `s` into
/// it — the host-callback-side mirror of `runtime.js`'s `allocText`, which every response-reading
/// import (`dom_response_body`, `dom_response_header`, …) reaches for the same reason: host text
/// answers through the module's own allocator, so it is as reachable to the release protocol as a
/// module-built Text.
fn alloc_text<T>(c: &mut Caller<'_, T>, s: &str) -> i32 {
    let new_text = c
        .get_export("__logos_text_new")
        .and_then(|e| e.into_func())
        .expect("a module reading a fetch response exports __logos_text_new");
    let typed = new_text
        .typed::<i32, i32>(&*c)
        .expect("__logos_text_new: i32 -> i32");
    let handle = typed
        .call(&mut *c, s.len() as i32)
        .expect("__logos_text_new does not trap");
    let mem = c
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory");
    let data_ptr = {
        let data = mem.data(&*c);
        i32::from_le_bytes(
            data[handle as usize + 8..handle as usize + 12]
                .try_into()
                .expect("4 bytes"),
        )
    };
    mem.write(&mut *c, data_ptr as usize, s.as_bytes())
        .expect("__logos_text_new allocated at least this many bytes");
    handle
}

/// A Text written into a mounted module's heap the way `runtime.js`'s `allocText` writes one: a
/// block from the module's own `__logos_text_new`, filled at its data pointer.
pub fn host_text<T>(store: &mut wasmi::Store<T>, inst: &wasmi::Instance, s: &str) -> i32 {
    let handle = inst
        .get_typed_func::<i32, i32>(&*store, "__logos_text_new")
        .expect("a page the host enters with a Text exports __logos_text_new")
        .call(&mut *store, s.len() as i32)
        .expect("__logos_text_new does not trap");
    let mem = inst
        .get_export(&*store, "memory")
        .and_then(|e| e.into_memory())
        .expect("memory");
    let data_ptr = {
        let data = mem.data(&*store);
        i32::from_le_bytes(
            data[handle as usize + 8..handle as usize + 12]
                .try_into()
                .expect("4 bytes"),
        )
    };
    mem.write(&mut *store, data_ptr as usize, s.as_bytes())
        .expect("__logos_text_new allocated at least this many bytes");
    handle
}

/// Hand `event` on `node` to a mounted page as `runtime.js`'s `dispatchNode` does: the event's name
/// and the value it carries written into the module's heap ([`host_text`]), then one call of the
/// `__lgx_dispatch_node(node, event, value) -> Bool` entry, then — the turn over, as `runtime.js`'s
/// `guarded` ends one at depth 0 — `__logos_safepoint` when the module reclaims. Answers whether
/// anything on the page answered it.
pub fn dispatch_node<T>(
    store: &mut wasmi::Store<T>,
    inst: &wasmi::Instance,
    node: i64,
    event: &str,
    value: &str,
) -> Result<bool, wasmi::core::Trap> {
    let event = host_text(store, inst, event);
    let value = host_text(store, inst, value);
    let answered = inst
        .get_typed_func::<(i64, i32, i32), i64>(&*store, "__lgx_dispatch_node")
        .expect("a page that listens exports __lgx_dispatch_node(node, event, value) -> Bool")
        .call(&mut *store, (node, event, value))?;
    if let Ok(safepoint) = inst.get_typed_func::<(), ()>(&*store, "__logos_safepoint") {
        safepoint.call(&mut *store, ())?;
    }
    Ok(answered != 0)
}

/// Build a `Seq of Int` through the MODULE's own `__logos_seq_new` export and fill its slots — the
/// host-callback-side mirror of how `runtime.js` hands a list over (a dropped file's bytes): the
/// module allocates the sequence, exactly as it allocates its own, and the host only writes.
pub fn alloc_seq_i64<T>(c: &mut Caller<'_, T>, values: &[i64]) -> i32 {
    let new_seq = c
        .get_export("__logos_seq_new")
        .and_then(|e| e.into_func())
        .expect("a module taking a host-built list exports __logos_seq_new");
    let typed = new_seq
        .typed::<(i32, i32), i32>(&*c)
        .expect("__logos_seq_new: (i32, i32) -> i32");
    let handle = typed
        .call(&mut *c, (values.len() as i32, 0))
        .expect("__logos_seq_new does not trap");
    let mem = c
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory");
    let data_ptr = {
        let data = mem.data(&*c);
        i32::from_le_bytes(
            data[handle as usize + 8..handle as usize + 12]
                .try_into()
                .expect("4 bytes"),
        )
    };
    for (i, v) in values.iter().enumerate() {
        mem.write(&mut *c, data_ptr as usize + 8 * i, &v.to_le_bytes())
            .expect("__logos_seq_new allocated every slot");
    }
    handle
}

/// Read a `Seq of Text` handle `[len][cap][data_ptr]`: `len` 8-byte slots, each holding a Text handle
/// in its low four bytes.
pub fn read_text_seq<T>(c: &Caller<'_, T>, handle: i32) -> Vec<String> {
    let (len, data) = header(c, handle);
    let mem = c
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("the module exports its memory");
    let bytes = mem.data(c);
    (0..len)
        .map(|i| {
            let at = data + i * 8;
            let slot = i32::from_le_bytes(bytes[at..at + 4].try_into().expect("a 4-byte handle"));
            read_text(c, slot)
        })
        .collect()
}

/// The patch stream one `dom_apply_words(words, wordCount, strings, stringCount)` call hands its
/// host, decoded: the first `wordCount` 8-byte slots of the `Seq of Int` and the first `stringCount`
/// Text handles of the `Seq of Text` (each sequence is scratch the module reuses, longer than what it
/// holds), read through `patch::decode_words`, the wire format's own decoder. A stream that is not
/// wire v2 is the module's bug and panics naming it.
pub fn applied_words<T>(
    c: &Caller<'_, T>,
    words: i32,
    word_count: i64,
    strings: i32,
    string_count: i64,
) -> Vec<logicaffeine_compile::semantics::dom::DomOp> {
    let word_slots = slots(c, words);
    let string_slots = slots(c, strings);
    let words: Vec<i64> = word_slots[..word_count as usize]
        .iter()
        .map(|slot| i64::from_le_bytes(*slot))
        .collect();
    let strings: Vec<String> = string_slots[..string_count as usize]
        .iter()
        .map(|slot| read_text(c, i32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]])))
        .collect();
    logicaffeine_compile::semantics::patch::decode_words(&words, &strings).unwrap_or_else(|e| {
        panic!("the module handed dom_apply_words a stream that is not wire v2: {e}")
    })
}

/// `dom_get_attribute` for a host that keeps no attributes: every element reads as having none. A
/// page's mount asks its mount point for `data-logos-prerendered` (`lgx_lower::PRERENDERED_ATTR`),
/// and on a host no build pre-rendered, the answer is that it is not marked.
pub fn link_no_attributes<T: 'static>(l: &mut Linker<T>) {
    l.func_wrap(
        "env",
        "dom_get_attribute",
        |mut c: Caller<'_, T>, _el: i64, _name: i32| -> i32 { alloc_text(&mut c, "") },
    )
    .expect("dom_get_attribute");
}

/// `dom_get_attribute` answered by the native recorder (`semantics::dom`), as the native
/// `getAttribute` builtin answers it.
pub fn link_native_attributes<T: 'static>(l: &mut Linker<T>) {
    l.func_wrap(
        "env",
        "dom_get_attribute",
        |mut c: Caller<'_, T>, el: i64, name: i32| -> i32 {
            let name = read_text(&c, name);
            let value =
                logicaffeine_compile::semantics::dom::attribute(el, &name).unwrap_or_default();
            alloc_text(&mut c, &value)
        },
    )
    .expect("dom_get_attribute");
}

/// `dom_apply_words` applied to the native recorder (`semantics::dom`), exactly as the native
/// `applyPatchWords` builtin applies a stream: the module's stream lands in the one document every
/// in-process tier draws into, so a test reads a wasm page with the same readers it reads the VM's
/// with. Answers what the builtin answers: the stream's hydration mismatches.
pub fn link_native_stream<T: 'static>(l: &mut Linker<T>) {
    l.func_wrap(
        "env",
        "dom_apply_words",
        |c: Caller<'_, T>, words: i32, word_count: i64, strings: i32, string_count: i64| -> i64 {
            let ops = applied_words(&c, words, word_count, strings, string_count);
            logicaffeine_compile::semantics::patch::apply(&ops).unwrap_or_else(|e| {
                panic!("the module emitted a stream the host cannot honour: {e}")
            })
        },
    )
    .expect("dom_apply_words");
}

/// One `noteTurn(name, key, changes, failure)` call as a host received it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalCall {
    pub name: String,
    pub key: String,
    pub changes: Vec<String>,
    pub failure: String,
}

/// Read a LOGOS `Text` handle out of linear memory: `[len][cap][data_ptr]`.
pub fn read_text<T>(c: &Caller<'_, T>, handle: i32) -> String {
    let mem = c
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory");
    let data = mem.data(c);
    let word =
        |o: usize| i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as usize;
    let (len, ptr) = (word(handle as usize), word(handle as usize + 8));
    String::from_utf8_lossy(&data[ptr..ptr + len]).into_owned()
}

thread_local! {
    /// The integers modules on this thread handed their host to hold (`exact_*`): slot `h` is the integer
    /// the tagged word `(h << 1) | 1` names. A slot given back is never reused, so a handle used after
    /// `exact_drop`, or given back twice, panics by name instead of reading another integer.
    static HELD: std::cell::RefCell<Vec<Option<logicaffeine_base::BigInt>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// How many integers this thread's host still holds for its modules: those a module never gave back.
pub fn held_integers() -> usize {
    HELD.with(|held| held.borrow().iter().filter(|slot| slot.is_some()).count())
}

/// The integer a tagged word names: `t >> 1` when it is even, the held integer `t >> 1` when odd.
fn exact_value(t: i64) -> logicaffeine_compile::interpreter::RuntimeValue {
    use logicaffeine_compile::interpreter::RuntimeValue;
    if t & 1 == 0 {
        return RuntimeValue::Int(t >> 1);
    }
    HELD.with(|held| match held.borrow().get((t >> 1) as usize) {
        Some(Some(big)) => RuntimeValue::from_bigint(big.clone()),
        Some(None) => panic!(
            "the module used held integer {} after giving it back",
            t >> 1
        ),
        None => panic!(
            "the module named held integer {}, which the host never held",
            t >> 1
        ),
    })
}

/// The tagged word of an integer: shifted when it is from -2^62 up to 2^62, else a fresh held one.
fn exact_word(v: logicaffeine_compile::interpreter::RuntimeValue) -> i64 {
    use logicaffeine_compile::interpreter::RuntimeValue;
    let big = match v {
        RuntimeValue::Int(i) if (i << 1) >> 1 == i => return i << 1,
        RuntimeValue::Int(i) => logicaffeine_base::BigInt::from_i64(i),
        RuntimeValue::BigInt(big) => (*big).clone(),
        other => panic!("an exact integer import answered a {}", other.type_name()),
    };
    HELD.with(|held| {
        let mut held = held.borrow_mut();
        held.push(Some(big));
        (((held.len() - 1) as i64) << 1) | 1
    })
}

fn exact_big(t: i64) -> logicaffeine_base::BigInt {
    use logicaffeine_compile::interpreter::RuntimeValue;
    match exact_value(t) {
        RuntimeValue::Int(i) => logicaffeine_base::BigInt::from_i64(i),
        RuntimeValue::BigInt(big) => (*big).clone(),
        other => unreachable!("an exact integer is an Int, not a {}", other.type_name()),
    }
}

/// The host's half of the module's tagged words, computed by the reference tier's own arithmetic
/// (`semantics::arith`) on the integers they name.
fn link_exact_integers<T: 'static>(l: &mut Linker<T>) {
    use logicaffeine_compile::interpreter::RuntimeValue;
    use logicaffeine_compile::semantics::arith;
    type Op = fn(RuntimeValue, RuntimeValue) -> Result<RuntimeValue, String>;
    let binary: [(&str, Op); 6] = [
        ("exact_add", arith::add),
        ("exact_sub", arith::subtract),
        ("exact_mul", arith::multiply),
        ("exact_div", arith::divide),
        ("exact_mod", arith::modulo),
        ("exact_floordiv", arith::floor_divide),
    ];
    for (name, op) in binary {
        l.func_wrap(
            "env",
            name,
            move |_c: Caller<'_, T>, a: i64, b: i64| -> i64 {
                exact_word(
                    op(exact_value(a), exact_value(b)).unwrap_or_else(|e| panic!("{name}: {e}")),
                )
            },
        )
        .expect(name);
    }
    l.func_wrap(
        "env",
        "exact_pow",
        |_c: Caller<'_, T>, base: i64, exp: i64| -> i64 {
            exact_word(
                arith::power(exact_value(base), RuntimeValue::Int(exp))
                    .unwrap_or_else(|e| panic!("exact_pow: {e}")),
            )
        },
    )
    .expect("exact_pow");
    l.func_wrap("env", "exact_box", |_c: Caller<'_, T>, v: i64| -> i64 {
        exact_word(RuntimeValue::Int(v))
    })
    .expect("exact_box");
    l.func_wrap("env", "exact_narrow", |_c: Caller<'_, T>, t: i64| -> i64 {
        match exact_value(t) {
            RuntimeValue::Int(i) => i,
            _ => 0,
        }
    })
    .expect("exact_narrow");
    l.func_wrap(
        "env",
        "exact_cmp",
        |_c: Caller<'_, T>, a: i64, b: i64| -> i32 {
            match exact_big(a).cmp(&exact_big(b)) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }
        },
    )
    .expect("exact_cmp");
    l.func_wrap(
        "env",
        "exact_f64",
        |_c: Caller<'_, T>, t: i64| -> wasmi::core::F64 { exact_big(t).to_f64().into() },
    )
    .expect("exact_f64");
    l.func_wrap("env", "exact_clone", |_c: Caller<'_, T>, t: i64| -> i64 {
        exact_word(exact_value(t))
    })
    .expect("exact_clone");
    l.func_wrap("env", "exact_drop", |_c: Caller<'_, T>, t: i64| {
        HELD.with(|held| match held.borrow_mut().get_mut((t >> 1) as usize) {
            Some(slot @ Some(_)) => *slot = None,
            Some(None) => panic!("the module gave held integer {} back twice", t >> 1),
            None => panic!(
                "the module gave back held integer {}, which the host never held",
                t >> 1
            ),
        })
    })
    .expect("exact_drop");
    l.func_wrap("env", "exact_text", |mut c: Caller<'_, T>, t: i64| -> i32 {
        alloc_text(&mut c, &exact_value(t).to_display_string())
    })
    .expect("exact_text");
}

/// Link every import whose behaviour is fixed by the ABI rather than by the test.
///
/// Generic over the store's data type, so a test keeps whatever recorder it needs and simply stops
/// hand-writing the parts that were never its decision.
pub fn link_value_imports<T: 'static>(l: &mut Linker<T>) {
    l.func_wrap(
        "env",
        "fmt_i64_into",
        |mut c: Caller<'_, T>, buf: i32, v: i64| -> i32 { write_into(&mut c, buf, &v.to_string()) },
    )
    .expect("fmt_i64_into");

    l.func_wrap(
        "env",
        "fmt_f64_into",
        |mut c: Caller<'_, T>, buf: i32, v: wasmi::core::F64| -> i32 {
            let shown = logicaffeine_compile::compile::display_float_like_logos(f64::from(v));
            write_into(&mut c, buf, &shown)
        },
    )
    .expect("fmt_f64_into");

    // A Bool rides an i64 local (0/1), which is why this takes an i64 and not an i32.
    l.func_wrap(
        "env",
        "fmt_bool_into",
        |mut c: Caller<'_, T>, buf: i32, v: i64| -> i32 {
            write_into(&mut c, buf, if v != 0 { "true" } else { "false" })
        },
    )
    .expect("fmt_bool_into");

    l.func_wrap(
        "env",
        "fmt_f64_prec_into",
        |mut c: Caller<'_, T>, buf: i32, v: wasmi::core::F64, prec: i32| -> i32 {
            write_into(
                &mut c,
                buf,
                &format!("{:.*}", prec.max(0) as usize, f64::from(v)),
            )
        },
    )
    .expect("fmt_f64_prec_into");

    link_exact_integers::<T>(l);

    l.func_wrap(
        "env",
        "pow_ff",
        |a: wasmi::core::F64, b: wasmi::core::F64| -> wasmi::core::F64 {
            f64::from(a).powf(f64::from(b)).into()
        },
    )
    .expect("pow_ff");

    l.func_wrap(
        "env",
        "pow_fi",
        |a: wasmi::core::F64, b: i64| -> wasmi::core::F64 { f64::from(a).powf(b as f64).into() },
    )
    .expect("pow_fi");

    l.func_wrap(
        "env",
        "parse_timestamp",
        |c: Caller<'_, T>, h: i32| -> i64 {
            let text = read_text(&c, h);
            logicaffeine_base::temporal::parse_rfc3339(text.trim())
                .expect("a valid RFC 3339 timestamp")
        },
    )
    .expect("parse_timestamp");

    l.func_wrap(
        "env",
        "temporal_component",
        |_c: Caller<'_, T>, nanos: i64, which: i32| -> i64 {
            temporal_component_of_moment(nanos, which)
        },
    )
    .expect("temporal_component");

    l.func_wrap(
        "env",
        "temporal_component_date",
        |_c: Caller<'_, T>, days: i32, which: i32| -> i64 {
            temporal_component_of_date(days, which)
        },
    )
    .expect("temporal_component_date");

    // The clock: honors the SAME thread-local fixed-clock the tree-walker/VM read
    // (`semantics::temporal::{today,now}`), so a test that pins the clock sees tw == vm == wasm;
    // real wall-clock time otherwise.
    l.func_wrap("env", "today", |_c: Caller<'_, T>| -> i32 {
        match logicaffeine_compile::semantics::temporal::today() {
            logicaffeine_compile::interpreter::RuntimeValue::Date(d) => d,
            _ => 0,
        }
    })
    .expect("today");
    l.func_wrap("env", "now", |_c: Caller<'_, T>| -> i64 {
        match logicaffeine_compile::semantics::temporal::now() {
            logicaffeine_compile::interpreter::RuntimeValue::Moment(n) => n,
            _ => 0,
        }
    })
    .expect("now");
    // The product's own draw, so a sealed test world's seed decides it on every tier alike.
    l.func_wrap(
        "env",
        "random_int",
        |_c: Caller<'_, T>, lo: i64, hi: i64| -> Result<i64, wasmi::core::Trap> {
            if lo > hi {
                return Err(wasmi::core::Trap::new(format!(
                    "randomInt({lo}, {hi}): the low bound is above the high bound"
                )));
            }
            Ok(logicaffeine_system::random::randomInt(lo, hi))
        },
    )
    .expect("random_int");
    // env.lg's `get`, in the halves the VM expands it into, and time.lg's `now()`: the product's own
    // reads, so a sealed test world answers them on every tier alike.
    l.func_wrap("env", "env_len", |c: Caller<'_, T>, h: i32| -> i32 {
        logicaffeine_system::env::get(read_text(&c, h)).map_or(-1, |v| v.len() as i32)
    })
    .expect("env_len");
    l.func_wrap("env", "env_into", |mut c: Caller<'_, T>, buf: i32, h: i32| -> i32 {
        let value = logicaffeine_system::env::get(read_text(&c, h)).unwrap_or_default();
        write_into(&mut c, buf, &value)
    })
    .expect("env_into");
    l.func_wrap("env", "time_now", |_c: Caller<'_, T>| -> i64 {
        logicaffeine_system::time::now() as i64
    })
    .expect("time_now");

    l.func_wrap("env", "parse_int", |c: Caller<'_, T>, h: i32| -> i64 {
        read_text(&c, h)
            .parse::<i64>()
            .expect("a valid Int literal")
    })
    .expect("parse_int");
    l.func_wrap(
        "env",
        "parse_float",
        |c: Caller<'_, T>, h: i32| -> wasmi::core::F64 {
            read_text(&c, h)
                .trim()
                .parse::<f64>()
                .expect("a valid Float literal")
                .into()
        },
    )
    .expect("parse_float");

    l.func_wrap(
        "env",
        "text_contains",
        |c: Caller<'_, T>, haystack: i32, needle: i32| -> i64 {
            read_text(&c, haystack).contains(&read_text(&c, needle)) as i64
        },
    )
    .expect("text_contains");

    // The `args` builtin is deliberately NOT registered here: unlike everything else in this
    // function, it has no single right answer — a test reading argv needs to inject SPECIFIC
    // values (see wasm_aot_args.rs), so it is a per-test decision exactly like a DOM import, not an
    // opinion-free formatter (`every_standalone_import_is_implemented_by_all_three_hosts` in
    // wasm_host_abi_lock.rs carries the matching exemption, with the same reasoning). A test with
    // no opinion may still supply an empty Seq of Text as address 0 (the module's own
    // zero-initialized [len=0][cap=0][data_ptr=0] header needs no allocation) — the trick
    // runtime.js's browser host relies on for a browser page, which has no argv at all.

    l.func_wrap(
        "env",
        "fmt_align_into",
        |mut c: Caller<'_, T>, buf: i32, text: i32, width: i32, align: i32| -> i32 {
            let s = read_text(&c, text);
            let w = width.max(0) as usize;
            let out = match align {
                1 => format!("{s:<w$}"),
                2 => format!("{s:^w$}"),
                _ => format!("{s:>w$}"),
            };
            write_into(&mut c, buf, &out)
        },
    )
    .expect("fmt_align_into");

    l.func_wrap(
        "env",
        "fmt_seq_i64_into",
        |mut c: Caller<'_, T>, buf: i32, h: i32| -> i32 {
            let parts: Vec<String> = slots(&c, h)
                .iter()
                .map(|s| i64::from_le_bytes(*s).to_string())
                .collect();
            let out = format!("[{}]", parts.join(", "));
            write_into(&mut c, buf, &out)
        },
    )
    .expect("fmt_seq_i64_into");
    l.func_wrap(
        "env",
        "fmt_seq_bool_into",
        |mut c: Caller<'_, T>, buf: i32, h: i32| -> i32 {
            let parts: Vec<String> = slots(&c, h)
                .iter()
                .map(|s| (i64::from_le_bytes(*s) != 0).to_string())
                .collect();
            let out = format!("[{}]", parts.join(", "));
            write_into(&mut c, buf, &out)
        },
    )
    .expect("fmt_seq_bool_into");
    l.func_wrap(
        "env",
        "fmt_set_i64_into",
        |mut c: Caller<'_, T>, buf: i32, h: i32| -> i32 {
            let parts: Vec<String> = slots(&c, h)
                .iter()
                .map(|s| i64::from_le_bytes(*s).to_string())
                .collect();
            let out = format!("{{{}}}", parts.join(", "));
            write_into(&mut c, buf, &out)
        },
    )
    .expect("fmt_set_i64_into");

    // `write_wire_residual(data_ptr, len) -> the byte count` — the raw bytes at `data_ptr` written
    // straight to stdout, exactly as the tree-walker's own `BuiltinId::WriteWireResidual` does.
    l.func_wrap(
        "env",
        "write_wire_residual",
        |c: Caller<'_, T>, data_ptr: i32, len: i32| -> i64 {
            use std::io::Write;
            let mem = c
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .expect("memory");
            let data = mem.data(&c);
            let bytes = &data[data_ptr as usize..(data_ptr + len) as usize];
            std::io::stdout()
                .write_all(bytes)
                .expect("stdout is writable");
            len as i64
        },
    )
    .expect("write_wire_residual");

    // `obs(site, p0, p1, p2)` — Wave 13 observability. `Op::Obs` is populated only by the OBSERVED
    // compile profile (`Compiler::compile_observed`), which `compile_to_wasm` never uses, so this
    // import is declared for ABI completeness and never actually reached by a real module today.
    l.func_wrap(
        "env",
        "obs",
        |_c: Caller<'_, T>, _site: i32, _p0: i64, _p1: i64, _p2: i64| {},
    )
    .expect("obs");
}

/// `temporal_component(nanos, which)` — one calendar/clock component of a `Moment`, computed by
/// the SAME `logicaffeine_base::temporal` decomposition `vm/wasm/module.rs` emits, so a test
/// reading a date/time component gets the reference tier's own answer rather than a second guess.
fn temporal_component_of_moment(nanos: i64, which: i32) -> i64 {
    use logicaffeine_base::temporal;
    let civil = temporal::civil_from_unix_nanos(nanos);
    match which {
        0 => civil.year,
        1 => civil.month as i64,
        2 => civil.day as i64,
        3 => civil.hour as i64,
        4 => civil.minute as i64,
        5 => civil.second as i64,
        6 => temporal::iso_weekday_from_days(nanos.div_euclid(temporal::NANOS_PER_DAY)) as i64,
        7 => temporal::iso_week_from_days(nanos.div_euclid(temporal::NANOS_PER_DAY)).1 as i64,
        8 => (civil.month as i64 - 1) / 3 + 1,
        _ => unreachable!("temporal_component: unknown component selector {which}"),
    }
}

/// `temporal_component_date(days, which)` — the calendar component of a `Date` (days since the
/// epoch), straight from `civil_from_days` with no nanos round-trip (a `Moment` round-trip would
/// overflow past ~year 2262).
fn temporal_component_of_date(days: i32, which: i32) -> i64 {
    use logicaffeine_base::temporal;
    let (y, m, d) = temporal::civil_from_days(days as i64);
    match which {
        0 => y,
        1 => m as i64,
        2 => d as i64,
        6 => temporal::iso_weekday_from_days(days as i64) as i64,
        7 => temporal::iso_week_from_days(days as i64).1 as i64,
        8 => (m as i64 - 1) / 3 + 1,
        _ => unreachable!("temporal_component_date: unknown component selector {which}"),
    }
}

/// The runtime failure hooks, as a DEFAULT a test may take or leave.
///
/// A program that divides by zero, indexes past the end, or reads a key a map does not hold calls
/// one of these, and the module then executes its OWN `unreachable` — every call site in
/// `vm/wasm/module.rs` is followed by opcode 0x00 — so ignoring the report still stops the program.
///
/// ★ DELIBERATELY NOT IN THE ENFORCED SET, and the reason is the rule this module is built on: a
/// test may legitimately have an OPINION about these. `runtime_error_message_lock` exists to read
/// the failure text, and forcing it onto a shared stub that discards the message would take away
/// the very thing it tests. What must be true is only that a host DEFINES them — a module importing
/// a function its host lacks does not lose a feature, it fails to INSTANTIATE, so a page that
/// starts indexing a map suddenly reports "does not load" for an unrelated change. This is the
/// convenience for the tests with no opinion; the others write their own and keep it.
pub fn link_failure_hooks<T: 'static>(l: &mut Linker<T>) {
    l.func_wrap("env", "logos_rt_fail", |_c: Caller<'_, T>, _h: i32| {})
        .expect("logos_rt_fail");
    l.func_wrap(
        "env",
        "logos_rt_fail_at",
        |_c: Caller<'_, T>, _code: i32, _v: i64| {},
    )
    .expect("logos_rt_fail_at");
}

/// Where a host's `Show` sinks put each displayed line. Implemented by every recorder the shared
/// runners use, so one set of sinks ([`link_show_sinks`]) serves them all.
pub trait ShowSink: 'static {
    fn show(&self, line: String);
}

impl ShowSink for std::rc::Rc<std::cell::RefCell<Vec<String>>> {
    fn show(&self, line: String) {
        self.borrow_mut().push(line);
    }
}

/// The `[len][cap][data_ptr]` header of the sequence / set / text at `handle`.
fn header<T>(c: &Caller<'_, T>, handle: i32) -> (usize, usize) {
    let mem = c
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("a module showing a heap value exports its memory");
    let data = mem.data(c);
    let word =
        |o: usize| i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as usize;
    (word(handle as usize), word(handle as usize + 8))
}

/// Each 8-byte slot of the sequence or set at `handle`, in stored order.
fn slots<T>(c: &Caller<'_, T>, handle: i32) -> Vec<[u8; 8]> {
    let (len, ptr) = header(c, handle);
    let mem = c
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory");
    let data = mem.data(c);
    (0..len)
        .map(|i| {
            data[ptr + i * 8..ptr + i * 8 + 8]
                .try_into()
                .expect("an 8-byte slot")
        })
        .collect()
}

fn slot_handle(slot: &[u8; 8]) -> i32 {
    i32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]])
}

/// Link the exact-integer display sink to a test's own output recorder.
pub fn link_exact_show<T: 'static>(l: &mut Linker<T>, show: fn(&T, String)) {
    l.func_wrap("env", "print_exact", move |c: Caller<'_, T>, t: i64| {
        show(c.data(), exact_value(t).to_display_string());
    })
    .expect("print_exact");
}

/// Link every `print_*` display sink, each rendering exactly as the reference tier's
/// `to_display_string` does: a sequence `[e0, e1]`, a set `{e0, e1}` in stored order, text unquoted,
/// a Float through the product's own float display, a Word as its unsigned value.
pub fn link_show_sinks<T: ShowSink>(l: &mut Linker<T>) {
    use logicaffeine_compile::compile::display_float_like_logos;
    use logicaffeine_compile::interpreter::RuntimeValue;
    l.func_wrap("env", "print_i64", |c: Caller<'_, T>, v: i64| {
        c.data().show(v.to_string())
    })
    .expect("print_i64");
    link_exact_show(l, |sink, line| sink.show(line));
    l.func_wrap("env", "print_bool", |c: Caller<'_, T>, v: i32| {
        c.data().show((v != 0).to_string())
    })
    .expect("print_bool");
    l.func_wrap("env", "print_char", |c: Caller<'_, T>, v: i64| {
        c.data().show(
            char::from_u32(v as u32)
                .map(String::from)
                .unwrap_or_default(),
        )
    })
    .expect("print_char");
    l.func_wrap(
        "env",
        "print_f64",
        |c: Caller<'_, T>, v: wasmi::core::F64| {
            c.data().show(display_float_like_logos(f64::from(v)))
        },
    )
    .expect("print_f64");
    l.func_wrap("env", "print_word", |c: Caller<'_, T>, v: i64| {
        c.data().show((v as u64).to_string())
    })
    .expect("print_word");
    l.func_wrap("env", "print_duration", |c: Caller<'_, T>, v: i64| {
        c.data().show(RuntimeValue::Duration(v).to_display_string())
    })
    .expect("print_duration");
    l.func_wrap("env", "print_time", |c: Caller<'_, T>, v: i64| {
        c.data().show(RuntimeValue::Time(v).to_display_string())
    })
    .expect("print_time");
    // `print_span(packed: i64)` — a `Span`'s two i32 fields packed months-high/days-low
    // (`vm/wasm/module.rs`'s `Constant::Span` lowering), unpacked and displayed exactly as
    // `RuntimeValue::Span::to_display_string`.
    l.func_wrap("env", "print_span", |c: Caller<'_, T>, packed: i64| {
        let months = (packed >> 32) as i32;
        let days = packed as i32;
        c.data()
            .show(RuntimeValue::Span { months, days }.to_display_string());
    })
    .expect("print_span");
    l.func_wrap(
        "env",
        "print_rational",
        |c: Caller<'_, T>, num: i64, den: i64| {
            c.data().show(if den == 1 {
                num.to_string()
            } else {
                format!("{num}/{den}")
            })
        },
    )
    .expect("print_rational");
    l.func_wrap("env", "print_nothing", |c: Caller<'_, T>| {
        c.data().show("nothing".into())
    })
    .expect("print_nothing");
    l.func_wrap("env", "print_date", |c: Caller<'_, T>, d: i32| {
        c.data().show(RuntimeValue::Date(d).to_display_string())
    })
    .expect("print_date");
    l.func_wrap("env", "print_moment", |c: Caller<'_, T>, n: i64| {
        c.data().show(RuntimeValue::Moment(n).to_display_string())
    })
    .expect("print_moment");
    l.func_wrap("env", "print_text", |c: Caller<'_, T>, h: i32| {
        let text = read_text(&c, h);
        c.data().show(text)
    })
    .expect("print_text");
    type Render = fn(&[u8; 8]) -> String;
    let seqs: [(&str, &str, &str, Render); 6] = [
        ("print_seq_i64", "[", "]", |s| {
            i64::from_le_bytes(*s).to_string()
        }),
        ("print_seq_bool", "[", "]", |s| {
            (i64::from_le_bytes(*s) != 0).to_string()
        }),
        ("print_seq_f64", "[", "]", |s| {
            display_float_like_logos(f64::from_le_bytes(*s))
        }),
        ("print_seq_word32", "[", "]", |s| {
            u32::from_le_bytes([s[0], s[1], s[2], s[3]]).to_string()
        }),
        ("print_seq_word64", "[", "]", |s| {
            u64::from_le_bytes(*s).to_string()
        }),
        ("print_set_i64", "{", "}", |s| {
            i64::from_le_bytes(*s).to_string()
        }),
    ];
    for (name, open, close, render) in seqs {
        l.func_wrap("env", name, move |c: Caller<'_, T>, h: i32| {
            let parts: Vec<String> = slots(&c, h).iter().map(render).collect();
            c.data().show(format!("{open}{}{close}", parts.join(", ")))
        })
        .expect(name);
    }
    for (name, open, close) in [("print_seq_text", "[", "]"), ("print_set_text", "{", "}")] {
        l.func_wrap("env", name, move |c: Caller<'_, T>, h: i32| {
            let parts: Vec<String> = slots(&c, h)
                .iter()
                .map(|s| read_text(&c, slot_handle(s)))
                .collect();
            c.data().show(format!("{open}{}{close}", parts.join(", ")))
        })
        .expect(name);
    }
}

/// Compile `src`, instantiate it with the shared imports, run `main`, and answer what it displayed —
/// or the reason it could not, including a failure it reported before stopping.
///
/// ★ IT VALIDATES THE MODULE, which is not the same as compiling one. `compile_to_wasm` returns
/// `Ok` for bytes that no engine will accept, and a lock reading only that reports a page as working
/// when it would never load. Constructing the `Module` is the check, and it is why this lives here
/// rather than being re-typed per test.
pub fn run_show(src: &str) -> Result<String, String> {
    let run = run_outcome(src)?;
    match run.failure {
        None => Ok(run.output),
        Some(reason) => Err(format!("failed: {reason}")),
    }
}

/// What a module did when it ran: everything it `Show`ed, and — when it stopped — the reason it
/// handed its host on the way out.
#[derive(Debug, Default)]
pub struct WasmRun {
    pub output: String,
    /// The shown lines `output` joins: unlike it, they tell no output from one empty line.
    pub lines: Vec<String>,
    pub failure: Option<String>,
    /// Where the heap's bump pointer (`__heap_ptr`) stood when `main` returned — how much address
    /// space the run claimed. `None` for a module without a heap.
    pub heap_ptr: Option<u32>,
}

#[derive(Default)]
pub struct Recorder {
    lines: Vec<String>,
    failure: Option<String>,
    /// The first failure a page entry the program took itself contained and no handler absorbed
    /// (`dom_page_entry`) — the run's own failure once `main` has finished.
    unhandled: Option<String>,
}

impl ShowSink for std::rc::Rc<std::cell::RefCell<Recorder>> {
    fn show(&self, line: String) {
        self.borrow_mut().lines.push(line);
    }
}

/// `dom_page_entry(node, event, value)` for a test host: a driving verb the program runs on its own
/// page, taken as the host's entry into the page exactly as `runtime.js`'s `dom_page_entry` takes
/// it — `__lgx_dispatch_node` entered from the host, a trap in it contained, the page's failure door
/// heard (`__lgx_failure_context`, then `__lgx_on_failure`), and a failure no handler absorbed handed
/// to `unhandled`. `why` takes the message the module left the host on its way down (what its
/// `logos_rt_fail`/`logos_rt_fail_at` recorded), so the failure is the entry's and not the run's. A
/// failed turn was heard.
pub fn link_page_entry<T: 'static>(
    l: &mut Linker<T>,
    why: fn(&Caller<'_, T>) -> String,
    unhandled: fn(&Caller<'_, T>, String),
) {
    l.func_wrap(
        "env",
        "dom_page_entry",
        move |mut c: Caller<'_, T>, node: i64, event: i32, value: i32| -> i64 {
            let export =
                |c: &Caller<'_, T>, name: &str| c.get_export(name).and_then(|e| e.into_func());
            let dispatch = export(&c, "__lgx_dispatch_node")
                .expect("a page that drives itself exports __lgx_dispatch_node");
            let mut answer = [wasmi::Value::I64(0)];
            let args = [
                wasmi::Value::I64(node),
                wasmi::Value::I32(event),
                wasmi::Value::I32(value),
            ];
            if dispatch.call(&mut c, &args, &mut answer).is_ok() {
                return match answer[0] {
                    wasmi::Value::I64(heard) => heard,
                    _ => 0,
                };
            }
            let reason = why(&c);
            let context = export(&c, "__lgx_failure_context")
                .and_then(|f| {
                    let mut text = [wasmi::Value::I32(0)];
                    f.call(&mut c, &[], &mut text)
                        .ok()
                        .map(|()| text[0].clone())
                })
                .map(|text| match text {
                    wasmi::Value::I32(h) => read_text(&c, h),
                    _ => String::new(),
                })
                .unwrap_or_default();
            let message = format!("{context}{reason}");
            let absorbed = export(&c, "__lgx_on_failure").is_some_and(|hook| {
                let text = alloc_text(&mut c, &message);
                let mut answer = [wasmi::Value::I64(0)];
                let held = hook
                    .call(&mut c, &[wasmi::Value::I32(text)], &mut answer)
                    .is_ok();
                let _ = why(&c);
                held && matches!(answer[0], wasmi::Value::I64(v) if v != 0)
            });
            if !absorbed {
                unhandled(&c, message);
            }
            1
        },
    )
    .expect("dom_page_entry");
}

/// One in-flight `fetchSend(method, url, body, headers, handler)` — the fields a test needs to
/// answer it later, mirroring the `{ handler, done, status, body, … }` record `runtime.js`'s own
/// `fetchReqs` map keeps per request id.
struct PendingFetch {
    url: String,
    handler: i32,
    status: i64,
    body: String,
}

/// What [`InteractivePage`]'s host records: every registered closure handle, in order, every
/// `listenFlags` registration, every `noteTurn(action)` call's action text, and every `fetchSend`
/// request awaiting an answer, keyed by the id `dom_response_status`/`dom_response_body` will be
/// called with — the things a heap or fetch-completion test past this point needs to observe
/// without hand-writing its own recorder.
#[derive(Default)]
struct InteractiveRecorder {
    handlers: Vec<i32>,
    listens: Vec<(i64, String, i64)>,
    next_node: i64,
    /// `parent → children`, from every `appendChild`, so a `clearChildren` forgets the listeners of
    /// exactly the subtree it took off the page.
    children: std::collections::HashMap<i64, Vec<i64>>,
    turns: Vec<JournalCall>,
    head_html: Vec<String>,
    fetches: std::collections::HashMap<i64, PendingFetch>,
    next_fetch_id: i64,
}

impl InteractiveRecorder {
    /// A fresh node handle: every created node is its own, as in a browser, so two elements are two
    /// keys in the page's listener table. The body is node 0.
    fn mint(&mut self) -> i64 {
        self.next_node += 1;
        self.next_node
    }

    /// `clearChildren(node)`: every descendant of `node` leaves the page, and its listeners with it.
    fn forget_subtree(&mut self, node: i64) {
        let mut gone = Vec::new();
        let mut stack = self.children.remove(&node).unwrap_or_default();
        while let Some(n) = stack.pop() {
            stack.extend(self.children.remove(&n).unwrap_or_default());
            gone.push(n);
        }
        self.listens.retain(|(n, _, _)| !gone.contains(n));
    }

    /// Takes `node` out of whichever parent lists it.
    fn detach(&mut self, node: i64) {
        for kids in self.children.values_mut() {
            kids.retain(|&k| k != node);
        }
    }

    /// `event` on `node` registered with `flags`, replacing an earlier registration's flags — what
    /// `runtime.js`'s `pageListen` does with a second registration of one pair.
    fn listen(&mut self, node: i64, event: String, flags: i64) {
        match self
            .listens
            .iter_mut()
            .find(|(n, e, _)| *n == node && *e == event)
        {
            Some(row) => row.2 = flags,
            None => self.listens.push((node, event, flags)),
        }
    }

    /// One `dom_apply_words` stream, applied to what this host keeps: where each node hangs and
    /// where the page listens. A freed node leaves with its whole subtree and every listener on it;
    /// a removed one is only detached, since the stream may place it again.
    fn apply_stream(&mut self, ops: &[logicaffeine_compile::semantics::dom::DomOp]) {
        use logicaffeine_compile::semantics::dom::DomOp;
        for op in ops {
            match op {
                DomOp::AppendChild { parent, child }
                | DomOp::InsertBefore { parent, child, .. } => {
                    let (parent, child) = (i64::from(*parent), i64::from(*child));
                    self.detach(child);
                    self.children.entry(parent).or_default().push(child);
                }
                DomOp::RemoveNode { node } => self.detach(i64::from(*node)),
                DomOp::FreeNode { node } => {
                    let node = i64::from(*node);
                    self.detach(node);
                    self.forget_subtree(node);
                    self.listens.retain(|(n, _, _)| *n != node);
                }
                DomOp::ClearChildren { node } => self.forget_subtree(i64::from(*node)),
                DomOp::Listen { node, event, flags } => {
                    self.listen(i64::from(*node), event.clone(), i64::from(*flags))
                }
                DomOp::Unlisten { node, event } => {
                    let node = i64::from(*node);
                    self.listens.retain(|(n, e, _)| !(*n == node && e == event));
                }
                _ => {}
            }
        }
    }
}

/// A mounted INTERACTIVE page: instantiated against a host that answers every DOM import inertly
/// (a fresh node handle per created node, no document) except `dom_listen_flags` and a paint's
/// `dom_apply_words` stream, which record where the page listens and how its nodes hang (so a node
/// that leaves takes its listeners with it), `dom_add_event_listener`/`dom_on_window_event`, which
/// record every closure handle they are handed, in registration order, and `dom_note_turn`, which
/// records every journal action a lowered `__act_` reports — the seam every heap test past this
/// point drives a turn through ([`InteractivePage::dispatch`] for an element's event,
/// [`InteractivePage::call_closure`] for a closure), and reads
/// `__logos_heap_allocated`/`__logos_text_new` from directly.
///
/// Reused rather than re-built per file: a page's `interactive` flag (and so whether it carries the
/// release-protocol exports at all) is decided by whether its host re-enters it — a callback it
/// registered, or a function the host enters by name, such as the node dispatcher.
pub struct InteractivePage {
    store: wasmi::Store<std::rc::Rc<std::cell::RefCell<InteractiveRecorder>>>,
    inst: wasmi::Instance,
}

impl InteractivePage {
    /// Every closure handle a `dom_add_event_listener` or `dom_on_window_event` call has handed the
    /// host, in the order it arrived (a re-registration is not deduplicated: a test that cares reads
    /// the LAST one).
    pub fn handlers(&self) -> Vec<i32> {
        self.store.data().borrow().handlers.clone()
    }

    /// Every node the page registered `event` on — through `listenFlags` or a paint's stream — in
    /// registration order.
    pub fn listening(&self, event: &str) -> Vec<i64> {
        self.store
            .data()
            .borrow()
            .listens
            .iter()
            .filter(|(_, e, _)| e == event)
            .map(|(n, _, _)| *n)
            .collect()
    }

    /// Every `(node, event, flags)` the page listens with now, in the order the pairs were first
    /// registered.
    pub fn registrations(&self) -> Vec<(i64, String, i64)> {
        self.store.data().borrow().listens.clone()
    }

    /// Hand `event` on `node` to the page — [`dispatch_node`]. Answers whether anything on the page
    /// answered it.
    pub fn dispatch(
        &mut self,
        node: i64,
        event: &str,
        value: &str,
    ) -> Result<bool, wasmi::core::Trap> {
        dispatch_node(&mut self.store, &self.inst, node, event, value)
    }

    /// Every `noteTurn(action)` call the host has recorded, in the order it arrived — a lowered
    /// `__act_`'s journal entry, on the tier that has no journal of its own to write into.
    pub fn turns(&self) -> Vec<String> {
        self.store
            .data()
            .borrow()
            .turns
            .iter()
            .map(|t| t.name.clone())
            .collect()
    }

    /// Every `noteTurn` call whole — name, instance key, changes and failure — in the order it
    /// arrived.
    pub fn journal(&self) -> Vec<JournalCall> {
        self.store.data().borrow().turns.clone()
    }

    /// Every `setHeadHtml(fragment)` call the host has recorded, in the order it arrived — the
    /// wasmi host has no real `document.head` to merge into, so it simply records what it was
    /// asked to set; `browser_tier_lock`'s real-browser harness is what proves the merge itself.
    pub fn head_html(&self) -> Vec<String> {
        self.store.data().borrow().head_html.clone()
    }

    /// Call a closure handle as one turn, exactly as `runtime.js`'s `dispatch` does.
    pub fn call_closure(&mut self, handle: i32, arg: i64) -> Result<i64, wasmi::core::Trap> {
        self.inst
            .get_typed_func::<(i32, i64), i64>(&self.store, "__logos_call_closure")
            .expect("an interactive module exports __logos_call_closure")
            .call(&mut self.store, (handle, arg))
    }

    /// The request id of the LAST `fetchSend` to `url` still awaiting an answer — the value the
    /// completion closure's own trampoline argument will carry, exactly as `runtime.js`'s
    /// `fetchSend` hands `fetchDone` the same `id` it returned to the module.
    pub fn fetch_request(&self, url: &str) -> Option<i64> {
        self.store
            .data()
            .borrow()
            .fetches
            .iter()
            .filter(|(_, f)| f.url == url)
            .map(|(id, _)| *id)
            .max()
    }

    /// Every request id currently awaiting an answer, in ascending (send) order — for a test that
    /// needs to tell two concurrent requests to the same URL apart without relying on `fetch_request`
    /// picking the newest.
    pub fn pending_fetch_ids(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = self.store.data().borrow().fetches.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Answer pending fetch `id` with `status`/`body`, then fire its completion closure exactly as
    /// `runtime.js`'s `fetchDone` does: `__logos_call_closure(handler, id)`. The record is left in
    /// place afterward (this host never forgets a fetch — a test wanting `forgetFetch`'s own
    /// `releaseHandle` behaviour drives `release_handle` itself).
    pub fn complete_fetch(
        &mut self,
        id: i64,
        status: i64,
        body: &str,
    ) -> Result<i64, wasmi::core::Trap> {
        let handler = {
            let mut rec = self.store.data().borrow_mut();
            let f = rec
                .fetches
                .get_mut(&id)
                .expect("complete_fetch: no pending fetch with this id");
            f.status = status;
            f.body = body.to_string();
            f.handler
        };
        self.call_closure(handler, id)
    }

    fn memory(&self) -> wasmi::Memory {
        self.inst
            .get_export(&self.store, "memory")
            .and_then(|e| e.into_memory())
            .expect("a heap module exports its memory")
    }

    /// The word at `addr` in linear memory (little-endian `i32`), for reading a header field or a
    /// free-list slot directly rather than through an export.
    pub fn word(&self, addr: i32) -> i32 {
        let data = self.memory().data(&self.store);
        i32::from_le_bytes(
            data[addr as usize..addr as usize + 4]
                .try_into()
                .expect("4 bytes"),
        )
    }

    /// Write `bytes` starting at `ptr` — the host's half of filling a block `text_new` returned.
    pub fn write_bytes(&mut self, ptr: i32, bytes: &[u8]) {
        let mem = self.memory();
        mem.write(&mut self.store, ptr as usize, bytes)
            .expect("the caller allocated at least this many bytes");
    }

    /// Read a `Text` handle back the way [`read_text`] does, but from the test's own thread rather
    /// than from inside a host callback.
    pub fn read_text(&self, handle: i32) -> String {
        let data = self.memory().data(&self.store);
        let word =
            |o: usize| i32::from_le_bytes(data[o..o + 4].try_into().expect("4 bytes")) as usize;
        let (len, ptr) = (word(handle as usize), word(handle as usize + 8));
        String::from_utf8_lossy(&data[ptr..ptr + len]).into_owned()
    }

    /// `__logos_text_new(len) -> handle` — allocate a Text block through the module's own allocator
    /// for the host to write into (S5 of the heap reclamation plan; see `allocText` in `runtime.js`).
    pub fn text_new(&mut self, len: i32) -> i32 {
        self.inst
            .get_typed_func::<i32, i32>(&self.store, "__logos_text_new")
            .expect("an interactive heap module exports __logos_text_new")
            .call(&mut self.store, len)
            .expect("__logos_text_new does not trap")
    }

    /// The GROSS heap `__logos_heap_allocated` reports: every byte the allocator has ever claimed.
    pub fn heap_allocated(&mut self) -> u32 {
        self.inst
            .get_typed_func::<(), i32>(&self.store, "__logos_heap_allocated")
            .expect("an interactive heap module exports __logos_heap_allocated")
            .call(&mut self.store, ())
            .expect("__logos_heap_allocated does not trap") as u32
    }

    /// The NET heap `__logos_heap_used` reports: the gross heap less every block sitting on a free
    /// list — what the page's live values actually occupy right now.
    pub fn heap_used(&mut self) -> u32 {
        self.inst
            .get_typed_func::<(), i32>(&self.store, "__logos_heap_used")
            .expect("an interactive heap module exports __logos_heap_used")
            .call(&mut self.store, ())
            .expect("__logos_heap_used does not trap") as u32
    }

    /// `__logos_safepoint()` (S8) — drains the per-turn retained list, freeing whatever its true
    /// holder count has just reached 0 (cascading through the class-specific children of a
    /// `HandleHeader`). [`dispatch_node`] ends its turn with one, as `runtime.js` does; after
    /// `call_closure` or a mount a test calls this itself to see anything freed.
    pub fn safepoint(&mut self) {
        self.inst
            .get_typed_func::<(), ()>(&self.store, "__logos_safepoint")
            .expect("an interactive heap module exports __logos_safepoint")
            .call(&mut self.store, ())
            .expect("__logos_safepoint does not trap");
    }

    /// `__logos_heap_census() -> i32` (S8, check-mode) — 0 when every reachable counted object's
    /// stored true count exactly equals its independently recomputed in-degree from the program's
    /// own globals; otherwise the address of the first object whose count disagrees.
    pub fn census(&mut self) -> i32 {
        self.inst
            .get_typed_func::<(), i32>(&self.store, "__logos_heap_census")
            .expect("an interactive heap module exports __logos_heap_census")
            .call(&mut self.store, ())
            .expect("__logos_heap_census does not trap")
    }

    /// `__logos_release(handle)` (S8) — the EXPORTED closure release the host calls once per
    /// registration it drops (`runtime.js`'s `releaseHandle`; see `RuntimeFn::ClosureRelease`'s
    /// doc). Only present on a `reclaim`-on module.
    pub fn release_handle(&mut self, handle: i32) {
        self.inst
            .get_typed_func::<i32, ()>(&self.store, "__logos_release")
            .expect("a reclaim-on interactive heap module exports __logos_release")
            .call(&mut self.store, handle)
            .expect("__logos_release does not trap");
    }
}

/// [`mount_interactive`] as a check-mode module ([`logicaffeine_compile::compile::compile_to_wasm_checked`]):
/// the page a user is shipped, with the heap census exported for [`InteractivePage::census`].
pub fn mount_interactive_checked(src: &str) -> InteractivePage {
    let module = logicaffeine_compile::compile::compile_to_wasm_checked(src)
        .unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    mount_interactive_module(module)
}

/// Compile `src` (a page that registers at least one DOM event listener) and instantiate it against
/// the inert DOM host every heap test needs. Runs `main` (the page's implicit mount) before handing
/// the page back, so its listener is already registered.
pub fn mount_interactive(src: &str) -> InteractivePage {
    let module = logicaffeine_compile::compile::compile_to_wasm(src)
        .unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    mount_interactive_module(module)
}

fn mount_interactive_module(module: Vec<u8>) -> InteractivePage {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).unwrap_or_else(|e| panic!("valid wasm: {e}"));
    type D = std::rc::Rc<std::cell::RefCell<InteractiveRecorder>>;
    let seen: D = std::rc::Rc::new(std::cell::RefCell::new(InteractiveRecorder::default()));
    let mut l = wasmi::Linker::<D>::new(&engine);
    l.func_wrap(
        "env",
        "dom_get_text",
        |_c: wasmi::Caller<'_, D>, _e: i64| -> i32 { 0 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_get_value",
        |_c: wasmi::Caller<'_, D>, _e: i64| -> i32 { 0 },
    )
    .unwrap();
    l.func_wrap("env", "dom_body", |_c: wasmi::Caller<'_, D>| -> i64 { 0 })
        .unwrap();
    l.func_wrap(
        "env",
        "dom_create_element",
        |c: wasmi::Caller<'_, D>, _t: i32| -> i64 { c.data().borrow_mut().mint() },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_create_text_node",
        |c: wasmi::Caller<'_, D>, _t: i32| -> i64 { c.data().borrow_mut().mint() },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_text",
        |_c: wasmi::Caller<'_, D>, _n: i64, _t: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_clear_children",
        |c: wasmi::Caller<'_, D>, n: i64| c.data().borrow_mut().forget_subtree(n),
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_attribute",
        |_c: wasmi::Caller<'_, D>, _n: i64, _k: i32, _v: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_bool_attribute",
        |_c: wasmi::Caller<'_, D>, _n: i64, _k: i32, _on: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_append_child",
        |c: wasmi::Caller<'_, D>, p: i64, child: i64| {
            c.data()
                .borrow_mut()
                .children
                .entry(p)
                .or_default()
                .push(child);
        },
    )
    .unwrap();
    // The host keeps no document, so the page is the only element a selector finds: `body` is node 0,
    // and anything else matches nothing.
    l.func_wrap(
        "env",
        "dom_query_selector",
        |c: wasmi::Caller<'_, D>, sel: i32| -> i64 {
            if read_text(&c, sel) == "body" {
                0
            } else {
                -1
            }
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_add_event_listener",
        |c: wasmi::Caller<'_, D>, _n: i64, _ev: i32, handler: i32| {
            c.data().borrow_mut().handlers.push(handler);
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_on_window_event",
        |c: wasmi::Caller<'_, D>, _ev: i32, handler: i32| {
            c.data().borrow_mut().handlers.push(handler);
        },
    )
    .unwrap();
    // `listenFlags(el, event, flags)`: the page listens for `event` on `el`; a later registration of
    // the same pair replaces its flags, as `runtime.js`'s `pageListen` does.
    l.func_wrap(
        "env",
        "dom_listen_flags",
        |c: wasmi::Caller<'_, D>, node: i64, event: i32, flags: i64| {
            let event = read_text(&c, event);
            c.data().borrow_mut().listen(node, event, flags);
        },
    )
    .unwrap();
    // A paint's patch stream: the nodes it names are the emitter's own, so this host mints nothing
    // for them and keeps only what it keeps for every other door — the tree's shape and the page's
    // listeners.
    l.func_wrap(
        "env",
        "dom_apply_words",
        |c: wasmi::Caller<'_, D>,
         words: i32,
         word_count: i64,
         strings: i32,
         string_count: i64|
         -> i64 {
            let ops = applied_words(&c, words, word_count, strings, string_count);
            c.data().borrow_mut().apply_stream(&ops);
            0
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_note_turn",
        |c: wasmi::Caller<'_, D>, name: i32, key: i32, changes: i32, failure: i32| {
            let call = JournalCall {
                name: read_text(&c, name),
                key: read_text(&c, key),
                changes: read_text_seq(&c, changes),
                failure: read_text(&c, failure),
            };
            c.data().borrow_mut().turns.push(call);
        },
    )
    .unwrap();
    // No drop is ever in flight on this host: a handler that reads the dropped file reads an empty
    // one, and the name it was handed is the one the test dispatched.
    l.func_wrap(
        "env",
        "dom_event_file_name",
        |mut c: wasmi::Caller<'_, D>, _ev: i64| -> i32 { alloc_text(&mut c, "") },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_event_file_size",
        |_c: wasmi::Caller<'_, D>, _ev: i64| -> i64 { 0 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_event_file_type",
        |mut c: wasmi::Caller<'_, D>, _ev: i64| -> i32 { alloc_text(&mut c, "") },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_event_file_bytes",
        |mut c: wasmi::Caller<'_, D>, _ev: i64| -> i32 { alloc_seq_i64(&mut c, &[]) },
    )
    .unwrap();
    // This host records every journal call whole, so it asks for the detail.
    l.func_wrap(
        "env",
        "dom_journal_detail",
        |_c: wasmi::Caller<'_, D>| -> i64 { 1 },
    )
    .unwrap();
    // A host with no wall clock wakes nothing: time moves only when a test calls `__lgx_tick`.
    l.func_wrap(
        "env",
        "dom_wake_after",
        |_c: wasmi::Caller<'_, D>, _ms: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_head_html",
        |c: wasmi::Caller<'_, D>, html: i32| {
            let text = read_text(&c, html);
            c.data().borrow_mut().head_html.push(text);
        },
    )
    .unwrap();
    // `fetchSend(method, url, body, headers, handler) -> id` — records a pending request (mirroring
    // `runtime.js`'s `fetchReqs`) instead of ever actually reaching a network; a test answers it
    // itself through `InteractivePage::complete_fetch`.
    l.func_wrap(
        "env",
        "dom_fetch_send",
        |c: wasmi::Caller<'_, D>,
         _method: i32,
         url: i32,
         _body: i32,
         _headers: i32,
         handler: i32|
         -> i64 {
            let url = read_text(&c, url);
            let mut rec = c.data().borrow_mut();
            rec.next_fetch_id += 1;
            let id = rec.next_fetch_id;
            rec.fetches.insert(
                id,
                PendingFetch {
                    url,
                    handler,
                    status: 0,
                    body: String::new(),
                },
            );
            id
        },
    )
    .unwrap();
    // `dom_fetch_start(url, handler)` — the plain (untyped) `Get URL, then action` request; it takes
    // the same slot in `fetches` as `dom_fetch_send` so `complete_fetch` answers either one.
    l.func_wrap(
        "env",
        "dom_fetch_start",
        |c: wasmi::Caller<'_, D>, url: i32, handler: i32| {
            let url = read_text(&c, url);
            let mut rec = c.data().borrow_mut();
            rec.next_fetch_id += 1;
            let id = rec.next_fetch_id;
            rec.fetches.insert(
                id,
                PendingFetch {
                    url,
                    handler,
                    status: 0,
                    body: String::new(),
                },
            );
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_fetch_abort",
        |_c: wasmi::Caller<'_, D>, _req: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_fetch_set_timeout",
        |_c: wasmi::Caller<'_, D>, _req: i64, _millis: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_status",
        |c: wasmi::Caller<'_, D>, req: i64| -> i64 {
            c.data()
                .borrow()
                .fetches
                .get(&req)
                .map(|f| f.status)
                .unwrap_or(0)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_body",
        |mut c: wasmi::Caller<'_, D>, req: i64| -> i32 {
            let body = c
                .data()
                .borrow()
                .fetches
                .get(&req)
                .map(|f| f.body.clone())
                .unwrap_or_default();
            alloc_text(&mut c, &body)
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_header",
        |mut c: wasmi::Caller<'_, D>, _req: i64, _name: i32| -> i32 { alloc_text(&mut c, "") },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_error",
        |mut c: wasmi::Caller<'_, D>, _req: i64| -> i32 { alloc_text(&mut c, "") },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_response_text",
        |mut c: wasmi::Caller<'_, D>| -> i32 { alloc_text(&mut c, "") },
    )
    .unwrap();
    link_no_attributes::<D>(&mut l);
    link_failure_hooks::<D>(&mut l);
    link_value_imports::<D>(&mut l);
    link_unmodelled_imports::<D>(&mut l);
    let mut store = wasmi::Store::new(&engine, seen);
    let inst = l
        .instantiate(&mut store, &m)
        .unwrap_or_else(|e| panic!("a page must instantiate — a missing host import: {e}"))
        .start(&mut store)
        .unwrap_or_else(|e| panic!("main must not trap: {e}"));
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("mount runs");
    InteractivePage { store, inst }
}

/// Compile, validate, instantiate and run `src`, answering its output AND its failure reason, or why
/// it could not run at all. `logos_rt_fail_at(code, value)` is rendered through `failcode::message`,
/// the interpreter's own wording, so a host can never drift from the reference tier.
pub fn run_outcome(src: &str) -> Result<WasmRun, String> {
    run_outcome_capped(src, None)
}

/// A `Recorder` paired with a [`wasmi::StoreLimits`] — the store's data type when a test needs
/// `main` to run against a host that can refuse to grow memory, rather than one that always has
/// room. `ShowSink` is implemented on the pair so it drops into every helper `Recorder`'s own
/// `Rc<RefCell<_>>` already does; `Store::limiter` reads the `limits` field.
#[derive(Clone)]
pub struct Capped {
    rec: std::rc::Rc<std::cell::RefCell<Recorder>>,
    limits: wasmi::StoreLimits,
}

impl ShowSink for Capped {
    fn show(&self, line: String) {
        self.rec.show(line);
    }
}

/// [`run_outcome`], with the module's linear memory capped at `cap_pages` wasm pages (64 KiB each)
/// when given — `memory.grow` past the cap returns -1 to the module exactly as it would against a
/// real embedder's own limit (`trap_on_grow_failure` stays at its default `false`, so the MODULE's
/// own `__logos_alloc` decides what a refused growth means, rather than wasmi trapping underneath it).
pub fn run_outcome_capped(src: &str, cap_pages: Option<u32>) -> Result<WasmRun, String> {
    run_outcome_hosted(src, cap_pages, |_| {})
}

/// [`run_outcome_capped`] with more host imports than the value/show/failure set — `extra` links
/// the ones a program's own subject needs (a DOM read, a host-built list) onto the same host.
pub fn run_outcome_hosted(
    src: &str,
    cap_pages: Option<u32>,
    extra: impl FnOnce(&mut Linker<Capped>),
) -> Result<WasmRun, String> {
    let module = logicaffeine_compile::compile::compile_to_wasm(src)
        .map_err(|e| format!("refused: {e:?}"))?;
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).map_err(|e| format!("invalid wasm: {e}"))?;
    let mut limits = wasmi::StoreLimitsBuilder::new();
    if let Some(pages) = cap_pages {
        limits = limits.memory_size(pages as usize * 65536);
    }
    let data = Capped {
        rec: Default::default(),
        limits: limits.build(),
    };
    let mut l = Linker::<Capped>::new(&engine);
    link_value_imports::<Capped>(&mut l);
    link_show_sinks::<Capped>(&mut l);
    l.func_wrap("env", "logos_rt_fail", |c: Caller<'_, Capped>, h: i32| {
        let text = read_text(&c, h);
        c.data().rec.borrow_mut().failure = Some(text);
    })
    .expect("logos_rt_fail");
    l.func_wrap(
        "env",
        "logos_rt_fail_at",
        |c: Caller<'_, Capped>, code: i32, value: i64| {
            c.data().rec.borrow_mut().failure = Some(
                logicaffeine_compile::semantics::failcode::message(code, value),
            );
        },
    )
    .expect("logos_rt_fail_at");
    link_page_entry::<Capped>(
        &mut l,
        |c| c.data().rec.borrow_mut().failure.take().unwrap_or_default(),
        |c, message| {
            let mut rec = c.data().rec.borrow_mut();
            rec.unhandled.get_or_insert(message);
        },
    );
    extra(&mut l);
    link_unmodelled_imports::<Capped>(&mut l);
    let mut store = wasmi::Store::new(&engine, data.clone());
    store.limiter(|d: &mut Capped| &mut d.limits as &mut dyn wasmi::ResourceLimiter);
    let inst = l
        .instantiate(&mut store, &m)
        .map_err(|e| format!("does not instantiate: {e}"))?
        .start(&mut store)
        .map_err(|e| format!("start trapped: {e}"))?;
    let ran = inst
        .get_typed_func::<(), ()>(&store, "main")
        .map_err(|e| format!("no main: {e}"))?
        .call(&mut store, ());
    let heap_ptr = inst
        .get_global(&store, "__heap_ptr")
        .and_then(|g| match g.get(&store) {
            wasmi::Value::I32(v) => Some(v as u32),
            _ => None,
        });
    let r = data.rec.borrow();
    match (ran, &r.failure) {
        (Ok(()), None) => Ok(WasmRun {
            output: r.lines.join("\n"),
            lines: r.lines.clone(),
            failure: r.unhandled.clone(),
            heap_ptr,
        }),
        (Err(_), Some(reason)) => Ok(WasmRun {
            output: r.lines.join("\n"),
            lines: r.lines.clone(),
            failure: Some(reason.clone()),
            heap_ptr,
        }),
        (Ok(()), Some(reason)) => Err(format!("reported {reason:?} but kept running")),
        (Err(e), None) => Err(format!("trapped with no reason: {e}")),
    }
}
