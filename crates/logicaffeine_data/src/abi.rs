//! The C ABI over [`LogosValue`]: every operation a compiled program performs on a value the exact
//! extension reaches, so the C backends and linked WebAssembly compute with this crate's value model
//! and base's tower instead of a copy of them.
//!
//! A value crosses as a handle, a `*mut LogosValue` that only this module makes and only
//! [`logos_rt_value_drop`] frees. An operation borrows its operands and answers a fresh handle; the
//! three that change a list (`push`, `set_item`, `pop`) change the value their first handle holds.
//! An operation that has no answer returns a null handle, or a status of `0`, and keeps the sentence
//! the interpreter stops with for [`logos_rt_value_error`]. A panic is caught and kept the same way,
//! so nothing unwinds across the boundary. Text crosses as UTF-8 bytes and their length; text this
//! module hands out is the caller's to give back through [`logos_rt_value_text_free`].
//!
//! A C program links this as a static library whose root installs [`Counting`] as its global
//! allocator, so [`logos_rt_value_heap_bytes`] tells the program's collector how much memory its
//! handles hold beyond its own heap. A linked WebAssembly module links the same functions from this
//! crate built for `wasm32`, where a panic ends the module instead of being caught: that target does
//! not unwind.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Once;

use logicaffeine_base::numeric::{BigInt, Decimal, Rational};

use crate::exact::{ExtensionBuiltin, LogosValue, Rel};

/// The system allocator, counting the bytes it holds for the program.
pub struct Counting;

static HEAP_BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE_HANDLES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            HEAP_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            HEAP_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
        HEAP_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let q = System.realloc(p, layout, new_size);
        if !q.is_null() {
            HEAP_BYTES.fetch_add(new_size, Ordering::Relaxed);
            HEAP_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        q
    }
}

thread_local! {
    static ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn keep(why: String) {
    ERROR.with(|e| *e.borrow_mut() = why);
}

/// `f`'s answer, or `None` with its refusal (or its panic's message) kept for [`logos_rt_value_error`].
fn answer<T>(f: impl FnOnce() -> Result<T, String>) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Some(v),
        Ok(Err(why)) => {
            keep(why);
            None
        }
        Err(panic) => {
            keep(match panic.downcast::<String>() {
                Ok(s) => *s,
                Err(panic) => match panic.downcast::<&'static str>() {
                    Ok(s) => (*s).to_string(),
                    Err(_) => "the value runtime stopped".to_string(),
                },
            });
            None
        }
    }
}

fn handle(v: LogosValue) -> *mut LogosValue {
    LIVE_HANDLES.fetch_add(1, Ordering::Relaxed);
    Box::into_raw(Box::new(v))
}

fn made(f: impl FnOnce() -> Result<LogosValue, String>) -> *mut LogosValue {
    answer(f).map_or(std::ptr::null_mut(), handle)
}

fn status(f: impl FnOnce() -> Result<(), String>) -> i32 {
    answer(f).map_or(0, |()| 1)
}

/// The value a handle holds.
///
/// # Safety
/// `h` is null or a handle this module made and has not freed.
unsafe fn held<'h>(h: *const LogosValue) -> Result<&'h LogosValue, String> {
    h.as_ref().ok_or_else(|| "a value handle is missing".to_string())
}

/// # Safety
/// As [`held`], and no other reference to the value is alive.
unsafe fn held_mut<'h>(h: *mut LogosValue) -> Result<&'h mut LogosValue, String> {
    h.as_mut().ok_or_else(|| "a value handle is missing".to_string())
}

/// # Safety
/// `ptr` points at `len` readable bytes, or `len` is 0.
unsafe fn bytes<'b>(ptr: *const u8, len: usize) -> &'b [u8] {
    if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

/// # Safety
/// As [`bytes`].
unsafe fn utf8<'b>(ptr: *const u8, len: usize) -> Result<&'b str, String> {
    std::str::from_utf8(bytes(ptr, len)).map_err(|_| "text that is not UTF-8".to_string())
}

/// # Safety
/// `items` points at `n` handles this module made, or `n` is 0.
unsafe fn values(items: *const *const LogosValue, n: usize) -> Result<Vec<LogosValue>, String> {
    if n == 0 {
        return Ok(Vec::new());
    }
    std::slice::from_raw_parts(items, n)
        .iter()
        .map(|h| held(*h).cloned())
        .collect()
}

/// Make the runtime quiet: a panic it catches is answered as a refusal, never printed on its own.
/// A C program calls this once, before its first other call.
#[no_mangle]
pub extern "C" fn logos_rt_value_init() {
    static QUIET: Once = Once::new();
    QUIET.call_once(|| std::panic::set_hook(Box::new(|_| {})));
}

/// The sentence the last operation that had no answer stopped with, valid until the next call on
/// this thread.
///
/// # Safety
/// `len` points at writable memory.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_error(len: *mut usize) -> *const u8 {
    ERROR.with(|e| {
        let e = e.borrow();
        *len = e.len();
        e.as_ptr()
    })
}

/// The bytes the runtime's allocator holds, when the program installed [`Counting`].
#[no_mangle]
pub extern "C" fn logos_rt_value_heap_bytes() -> u64 {
    HEAP_BYTES.load(Ordering::Relaxed) as u64
}

/// The handles made and not yet freed.
#[no_mangle]
pub extern "C" fn logos_rt_value_live() -> u64 {
    LIVE_HANDLES.load(Ordering::Relaxed)
}

/// Free a handle. A null handle frees nothing.
///
/// # Safety
/// `h` is null or a handle this module made and has not freed; it is not used again.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_drop(h: *mut LogosValue) {
    if !h.is_null() {
        LIVE_HANDLES.fetch_sub(1, Ordering::Relaxed);
        drop(Box::from_raw(h));
    }
}

/// A second handle to the value `h` holds.
///
/// # Safety
/// `h` is a live handle.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_clone(h: *const LogosValue) -> *mut LogosValue {
    made(|| held(h).cloned())
}

#[no_mangle]
pub extern "C" fn logos_rt_value_int(n: i64) -> *mut LogosValue {
    handle(LogosValue::Int(n))
}

#[no_mangle]
pub extern "C" fn logos_rt_value_float(x: f64) -> *mut LogosValue {
    handle(LogosValue::Float(x))
}

#[no_mangle]
pub extern "C" fn logos_rt_value_bool(b: i32) -> *mut LogosValue {
    handle(LogosValue::Bool(b != 0))
}

#[no_mangle]
pub extern "C" fn logos_rt_value_nothing() -> *mut LogosValue {
    handle(LogosValue::Nothing)
}

/// A calendar date, as its days since 1970-01-01.
#[no_mangle]
pub extern "C" fn logos_rt_value_date(days: i32) -> *mut LogosValue {
    handle(LogosValue::other(crate::temporal::LogosDate(days)))
}

/// An instant, as its nanoseconds since 1970-01-01T00:00:00Z.
#[no_mangle]
pub extern "C" fn logos_rt_value_moment(nanos: i64) -> *mut LogosValue {
    handle(LogosValue::other(crate::temporal::LogosMoment(nanos)))
}

/// A time of day, as its nanoseconds since midnight.
#[no_mangle]
pub extern "C" fn logos_rt_value_time(nanos: i64) -> *mut LogosValue {
    handle(LogosValue::other(crate::temporal::LogosTime(nanos)))
}

/// A calendar span of whole months and days.
#[no_mangle]
pub extern "C" fn logos_rt_value_span(months: i32, days: i32) -> *mut LogosValue {
    handle(LogosValue::other(crate::temporal::LogosSpan::new(months, days)))
}

/// # Safety
/// `ptr` points at `len` bytes of UTF-8.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_text(ptr: *const u8, len: usize) -> *mut LogosValue {
    made(|| utf8(ptr, len).map(LogosValue::text))
}

/// An exact number from its digits: an integer (`-123…`) or a ratio of two (`7/2`), in its smallest
/// form.
///
/// # Safety
/// `ptr` points at `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_exact(ptr: *const u8, len: usize) -> *mut LogosValue {
    made(|| {
        let digits = utf8(ptr, len)?;
        Rational::parse(digits)
            .map(LogosValue::from_rational)
            .ok_or_else(|| format!("`{digits}` is not an exact number"))
    })
}

/// A decimal from its digits (`12.50`).
///
/// # Safety
/// `ptr` points at `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_decimal(ptr: *const u8, len: usize) -> *mut LogosValue {
    made(|| {
        let digits = utf8(ptr, len)?;
        Decimal::parse(digits.trim())
            .map(|d| LogosValue::Decimal(std::sync::Arc::new(d)))
            .ok_or_else(|| format!("`{digits}` is not a decimal"))
    })
}

/// A list of the values `items` hold.
///
/// # Safety
/// `items` points at `n` live handles, or `n` is 0.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_list(items: *const *const LogosValue, n: usize) -> *mut LogosValue {
    made(|| values(items, n).map(LogosValue::list))
}

/// A tuple of the values `items` hold.
///
/// # Safety
/// `items` points at `n` live handles, or `n` is 0.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_tuple(items: *const *const LogosValue, n: usize) -> *mut LogosValue {
    made(|| values(items, n).map(|v| LogosValue::Tuple(std::sync::Arc::new(v))))
}

/// The items of the list `list`, each as the value it is.
///
/// # Safety
/// `list` is a live handle.
unsafe fn items_of<'v>(list: *const LogosValue) -> Result<&'v [LogosValue], String> {
    match held(list)? {
        LogosValue::List(items) => Ok(items),
        other => Err(format!("Expected List, found {}", other.type_name())),
    }
}

/// A tuple of the items of the list `list`.
///
/// # Safety
/// `list` is a live handle.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_tuple_of(list: *const LogosValue) -> *mut LogosValue {
    made(|| Ok(LogosValue::Tuple(std::sync::Arc::new(items_of(list)?.to_vec()))))
}

/// The integer a runtime holds as base's own `BigInt`, as a value in its smallest form: for a runtime that keeps
/// its integers past a machine word in Rust.
///
/// # Safety
/// `b` points at a live `BigInt`.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_of_bigint(b: *const BigInt) -> *mut LogosValue {
    made(|| Ok(LogosValue::from_bigint((*b).clone())))
}

/// The rational a runtime holds as base's own `Rational`, as a value in its smallest form.
///
/// # Safety
/// `r` points at a live `Rational`.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_of_rational(r: *const Rational) -> *mut LogosValue {
    made(|| Ok(LogosValue::from_rational((*r).clone())))
}

/// The decimal a runtime holds as base's own `Decimal`, as a value.
///
/// # Safety
/// `d` points at a live `Decimal`.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_of_decimal(d: *const Decimal) -> *mut LogosValue {
    made(|| Ok(LogosValue::Decimal(std::sync::Arc::new((*d).clone()))))
}

/// The builtin of the extension at `index` of `extension::Builtin::ALL`, applied to the items of the list `args`.
///
/// # Safety
/// `args` is a live handle.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_apply(index: usize, args: *const LogosValue) -> *mut LogosValue {
    made(|| {
        let builtin = *ExtensionBuiltin::ALL
            .get(index)
            .ok_or_else(|| format!("no builtin of the exact extension at {index}"))?;
        logicaffeine_base::extension::call(builtin, items_of(args)?.to_vec())
    })
}

/// The builtin of the extension named `name`, applied to `args`.
///
/// # Safety
/// `name` points at `name_len` bytes; `args` at `argc` live handles, or `argc` is 0.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_call(
    name: *const u8,
    name_len: usize,
    args: *const *const LogosValue,
    argc: usize,
) -> *mut LogosValue {
    made(|| {
        let named = utf8(name, name_len)?;
        let builtin =
            ExtensionBuiltin::named(named).ok_or_else(|| format!("`{named}` is not a builtin of the exact extension"))?;
        logicaffeine_base::extension::call(builtin, values(args, argc)?)
    })
}

macro_rules! binary {
    ($($name:ident => $method:ident;)*) => {$(
        /// # Safety
        /// `a` and `b` are live handles.
        #[no_mangle]
        pub unsafe extern "C" fn $name(a: *const LogosValue, b: *const LogosValue) -> *mut LogosValue {
            made(|| held(a)?.$method(held(b)?))
        }
    )*};
}

binary! {
    logos_rt_value_add => add;
    logos_rt_value_sub => sub;
    logos_rt_value_mul => mul;
    logos_rt_value_div => div;
    logos_rt_value_exact_div => exact_div;
    logos_rt_value_floor_div => floor_div;
    logos_rt_value_rem => rem;
    logos_rt_value_pow => pow;
    logos_rt_value_approx_eq => approx_eq;
    logos_rt_value_seq_concat => seq_concat;
    logos_rt_value_bit_and => bit_and;
    logos_rt_value_bit_or => bit_or;
    logos_rt_value_bit_xor => bit_xor;
    logos_rt_value_shl => shl;
    logos_rt_value_shr => shr;
    logos_rt_value_min => min;
    logos_rt_value_max => max;
    logos_rt_value_power => power;
    logos_rt_value_item => item;
}

macro_rules! relation {
    ($($name:ident => $rel:ident;)*) => {$(
        /// # Safety
        /// `a` and `b` are live handles.
        #[no_mangle]
        pub unsafe extern "C" fn $name(a: *const LogosValue, b: *const LogosValue) -> *mut LogosValue {
            made(|| held(a)?.relation(Rel::$rel, held(b)?))
        }
    )*};
}

relation! {
    logos_rt_value_eq => Eq;
    logos_rt_value_ne => NotEq;
    logos_rt_value_lt => Lt;
    logos_rt_value_gt => Gt;
    logos_rt_value_le => LtEq;
    logos_rt_value_ge => GtEq;
}

macro_rules! unary {
    ($($name:ident => $method:ident;)*) => {$(
        /// # Safety
        /// `a` is a live handle.
        #[no_mangle]
        pub unsafe extern "C" fn $name(a: *const LogosValue) -> *mut LogosValue {
            made(|| held(a)?.$method())
        }
    )*};
}

unary! {
    logos_rt_value_not => not;
    logos_rt_value_abs => abs;
    logos_rt_value_sqrt => sqrt;
    logos_rt_value_floor => floor;
    logos_rt_value_ceil => ceil;
    logos_rt_value_round => round;
}

/// How a compiled program answers a function it defines a real number by: the function's code and
/// environment as the program holds them, and the place `n` the runtime asks for, to a fresh handle of
/// the function's answer there, or a null handle with its refusal kept.
pub type Definer =
    unsafe extern "C" fn(code: *mut std::ffi::c_void, env: *mut std::ffi::c_void, n: i64) -> *mut LogosValue;

/// `1` when the value `a` holds may cross to a task or through a channel; `0`, with the interpreter's
/// sentence kept, when it holds a real number a function of the program defines.
///
/// # Safety
/// `a` is a live handle.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_sendable(a: *const LogosValue) -> i32 {
    status(|| held(a)?.sendable())
}

/// The value a handle holds, taken out of it: the handle is freed.
///
/// # Safety
/// `h` is a live handle, not used again.
unsafe fn taken(h: *mut LogosValue) -> LogosValue {
    LIVE_HANDLES.fetch_sub(1, Ordering::Relaxed);
    *Box::from_raw(h)
}

/// The real number a function of the program defines by its digits in `base` or its approximations
/// (`kind`), `name` being how the program spells the function: the runtime calls `call(code, env, n)`
/// for the answer at `n` only when a refinement of the number asks, and the function's code and
/// environment must stay as they are for as long as the program runs.
///
/// # Safety
/// `kind` and `name` point at that many bytes of UTF-8; `call` answers as [`Definer`] says, for as
/// long as the program runs, on every thread the program reads the number on.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_defined_real(
    kind: *const u8,
    kind_len: usize,
    base: i64,
    name: *const u8,
    name_len: usize,
    call: Definer,
    code: *mut std::ffi::c_void,
    env: *mut std::ffi::c_void,
) -> *mut LogosValue {
    made(|| {
        let (kind, name) = (utf8(kind, kind_len)?, utf8(name, name_len)?);
        let (code, env) = (code as usize, env as usize);
        LogosValue::define_real(kind, base, name, move |n| {
            let answer = call(code as *mut std::ffi::c_void, env as *mut std::ffi::c_void, n);
            if answer.is_null() {
                Err(ERROR.with(|e| e.borrow().clone()))
            } else {
                Ok(taken(answer))
            }
        })
    })
}

/// Item `index` (counted from 1) of a list, a tuple or a text.
///
/// # Safety
/// `a` is a live handle.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_item_at(a: *const LogosValue, index: i64) -> *mut LogosValue {
    made(|| held(a)?.item(&LogosValue::Int(index)))
}

/// `items start through end of a`.
///
/// # Safety
/// `a`, `start` and `end` are live handles.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_slice(
    a: *const LogosValue,
    start: *const LogosValue,
    end: *const LogosValue,
) -> *mut LogosValue {
    made(|| held(a)?.slice(held(start)?, held(end)?))
}

/// `length of a`, into `out`.
///
/// # Safety
/// `a` is a live handle; `out` points at writable memory.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_length(a: *const LogosValue, out: *mut i64) -> i32 {
    status(|| {
        *out = held(a)?.length_of()?;
        Ok(())
    })
}

/// `a contains v`, into `out` as 0 or 1.
///
/// # Safety
/// `a` and `v` are live handles; `out` points at writable memory.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_contains(a: *const LogosValue, v: *const LogosValue, out: *mut i32) -> i32 {
    status(|| {
        *out = held(a)?.contains_value(held(v)?)? as i32;
        Ok(())
    })
}

/// The truth a condition reads from `a`, into `out` as 0 or 1: refused for a residue-dependent or
/// provisional answer.
///
/// # Safety
/// `a` is a live handle; `out` points at writable memory.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_truth(a: *const LogosValue, out: *mut i32) -> i32 {
    status(|| {
        *out = held(a)?.decided_truth()? as i32;
        Ok(())
    })
}

/// The `Int` that `a` is, into `out`; refused, in the words of a place that holds a machine
/// integer, for any other value.
///
/// # Safety
/// `a` is a live handle; `out` points at writable memory.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_int_of(a: *const LogosValue, out: *mut i64) -> i32 {
    status(|| match held(a)? {
        LogosValue::Int(n) => {
            *out = *n;
            Ok(())
        }
        LogosValue::BigInt(_) => Err(crate::failure::INT_TOO_WIDE.to_string()),
        other => Err(format!("Expected Int, found {}", other.type_name())),
    })
}

/// The `Int` that `a` is where the language takes a machine integer, into `out`; refused with the place's own
/// sentences — `past_word` for an integer past `i64`, `fraction` for a rational that is not whole.
///
/// # Safety
/// `a` is a live handle; `past_word` and `fraction` point at that many bytes of UTF-8; `out` points at writable
/// memory.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_int_at(
    a: *const LogosValue,
    past_word: *const u8,
    past_word_len: usize,
    fraction: *const u8,
    fraction_len: usize,
    out: *mut i64,
) -> i32 {
    status(|| {
        let text = |ptr: *const u8, len: usize| {
            String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len)).into_owned()
        };
        *out = held(a)?.integer_at(&text(past_word, past_word_len), &text(fraction, fraction_len))?;
        Ok(())
    })
}

/// `Push v to a`.
///
/// # Safety
/// `a` and `v` are live handles, and `a` is not `v`.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_push(a: *mut LogosValue, v: *const LogosValue) -> i32 {
    status(|| {
        let item = held(v)?.clone();
        held_mut(a)?.push_value(item)
    })
}

/// `Set item index of a to v`.
///
/// # Safety
/// `a`, `index` and `v` are live handles, and `a` is neither of the others.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_set_item(
    a: *mut LogosValue,
    index: *const LogosValue,
    v: *const LogosValue,
) -> i32 {
    status(|| {
        let (index, item) = (held(index)?.clone(), held(v)?.clone());
        held_mut(a)?.set_item(&index, item)
    })
}

/// `Pop from a`: the last item, taken out of `a`.
///
/// # Safety
/// `a` is a live handle.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_pop(a: *mut LogosValue) -> *mut LogosValue {
    made(|| held_mut(a)?.pop_value())
}

/// The values a `Repeat` over `a` binds, taken once before the loop runs, as a list: a list's items, a
/// text's characters.
///
/// # Safety
/// `a` is a live handle.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_items(a: *const LogosValue) -> *mut LogosValue {
    made(|| held(a)?.items().map(LogosValue::list))
}

/// How `a` prints, as UTF-8 bytes the caller gives back through [`logos_rt_value_text_free`].
///
/// # Safety
/// `a` is a live handle; `len` points at writable memory.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_show(a: *const LogosValue, len: *mut usize) -> *mut u8 {
    match answer(|| held(a).map(LogosValue::display_string)) {
        Some(text) => {
            let text = text.into_bytes().into_boxed_slice();
            *len = text.len();
            Box::into_raw(text) as *mut u8
        }
        None => std::ptr::null_mut(),
    }
}

/// Give back text [`logos_rt_value_show`] handed out.
///
/// # Safety
/// `ptr` and `len` are what one call of [`logos_rt_value_show`] answered, given back once.
#[no_mangle]
pub unsafe extern "C" fn logos_rt_value_text_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/abi/tests.rs"]
mod tests;
