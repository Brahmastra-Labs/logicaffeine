//! I/O Operations for LOGOS Programs
//!
//! Provides input/output primitives for LOGOS programs including:
//!
//! - [`show`]: Natural formatting output (primitives without quotes, collections with brackets)
//! - `print`, `println`, `eprintln`: Standard output functions
//! - [`read_line`]: Read a line from stdin
//!
//! The [`Showable`] trait enables custom types to integrate with the `show` verb.
//!
//! # Example
//!
//! ```no_run
//! use logicaffeine_system::io::{show, println, read_line};
//!
//! // Natural formatting with show
//! show(&42);           // Prints: 42
//! show(&"hello");      // Prints: hello (no quotes)
//! show(&vec![1, 2, 3]); // Prints: [1, 2, 3]
//!
//! // Standard output
//! println("Enter your name:");
//! let name = read_line();
//! println(format!("Hello, {}!", name));
//! ```

use std::fmt::{self, Display};

/// Custom trait for LOGOS Show verb - provides clean, natural output.
/// Primitives display without quotes, collections display with brackets.
pub trait Showable {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result;
}

// Blanket impl for references: &T is Showable if T is Showable
impl<T: Showable + ?Sized> Showable for &T {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (**self).format_show(f)
    }
}

// Primitives: use Display formatting
impl Showable for i32 {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for i64 {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for u64 {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for usize {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for f64 {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// Exact rationals show as `n/d` (or a bare integer when whole) — `Display` already
// reduces, so `Let x: Rational be 7 / 2; Show x` prints `7/2`.
/// The exact compiled integer (i64 fast path, BigInt spill) prints as the
/// plain number — indistinguishable from an `i64` `Show`.
impl Showable for logicaffeine_data::LogosInt {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

/// A value the exact extension reaches prints as the interpreter prints it: its canonical form, the
/// exact types in their own notation.
impl Showable for logicaffeine_data::LogosValue {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.display_string())
    }
}

impl Showable for logicaffeine_data::LogosRational {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// Exact base-10 decimals (money) show with their scale faithful — `Display` prints
// `19.99`/`20.00`, so `Show decimal("19.99")` prints `19.99`, never a lossy float.
impl Showable for logicaffeine_data::LogosDecimal {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// Exact complex numbers show as `3+4i` / `i` / `-2i` — `Display` already formats them.
impl Showable for logicaffeine_data::LogosComplex {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

/// The floating-point complex prints like the exact one (`2+i`, `4+6i`) via its `Display`.
impl Showable for logicaffeine_data::LogosComplexF {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// ℤ/nℤ elements show as `3 (mod 7)`.
impl Showable for logicaffeine_data::LogosModular {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// Physical quantities show as magnitude + unit (`42/127 ft`, `20 °C`) — `Display` formats them.
impl Showable for logicaffeine_data::LogosQuantity {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for logicaffeine_data::LogosMoney {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for logicaffeine_data::LogosUuid {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// `nothing`, the value a compiled program holds as `()`
impl Showable for () {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("nothing")
    }
}

impl Showable for bool {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for u8 {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for char {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for String {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Showable for &str {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// Sequences: bracket notation with recursive formatting
impl<T: Showable> Showable for Vec<T> {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        for (i, item) in self.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            item.format_show(f)?;
        }
        write!(f, "]")
    }
}

impl<T: Showable> Showable for logicaffeine_data::LogosSeq<T> {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let inner = self.borrow();
        write!(f, "[")?;
        for (i, item) in inner.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            item.format_show(f)?;
        }
        write!(f, "]")
    }
}

impl<K: Showable + Eq + std::hash::Hash, V: Showable> Showable
    for logicaffeine_data::LogosMap<K, V>
{
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let inner = self.borrow();
        write!(f, "{{")?;
        for (i, (k, v)) in inner.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            k.format_show(f)?;
            write!(f, ": ")?;
            v.format_show(f)?;
        }
        write!(f, "}}")
    }
}

// A Set shows as `{e0, e1, …}` in INSERTION order — matching the
// tree-walker's Vec-backed set display and the direct-WASM linear set
// (the LOGOS `Set` alias is an insertion-ordered IndexSet).
impl<T: Showable, S: std::hash::BuildHasher> Showable for indexmap::IndexSet<T, S> {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{")?;
        for (i, v) in self.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            v.format_show(f)?;
        }
        write!(f, "}}")
    }
}

// Slices: same as Vec
impl<T: Showable> Showable for [T] {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        for (i, item) in self.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            item.format_show(f)?;
        }
        write!(f, "]")
    }
}

// Note: &[T] is covered by the blanket `impl<T: Showable + ?Sized> Showable for &T`
// since `[T]: Showable`.

// Option type: shows "nothing" or the value
impl<T: Showable> Showable for Option<T> {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Some(v) => v.format_show(f),
            None => write!(f, "nothing"),
        }
    }
}

// A place that can hold `nothing`: shows the value it holds, or "nothing"
impl<T: Showable> Showable for logicaffeine_data::LogosOpt<T> {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.format_show(f)
    }
}

// A `Result` is a variant like any other: `Ok(v)` / `Err(e)`.
impl<T: Showable, E: Showable> Showable for Result<T, E> {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (name, payload): (&str, &dyn Showable) = match self {
            Ok(v) => ("Ok", v),
            Err(e) => ("Err", e),
        };
        write!(f, "{name}(")?;
        payload.format_show(f)?;
        write!(f, ")")
    }
}

// A variant's self-referential field is boxed; it shows as what it holds.
impl<T: Showable + ?Sized> Showable for Box<T> {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (**self).format_show(f)
    }
}

// CRDT types: show the value
impl Showable for logicaffeine_data::crdt::GCounter {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

impl Showable for logicaffeine_data::crdt::PNCounter {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

// Shared collections render exactly as the interpreter does: a set SORTED in `{…}` (a set has no
// intrinsic order, so sorting is the order every replica agrees on), a sequence in convergent order in
// `[…]`, and a map to plain values as `{key: value, …}` by key.
fn show_joined<'a, T: Showable + 'a>(
    f: &mut fmt::Formatter,
    open: &str,
    items: impl Iterator<Item = &'a T>,
    close: &str,
) -> fmt::Result {
    write!(f, "{open}")?;
    for (i, item) in items.enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        item.format_show(f)?;
    }
    write!(f, "{close}")
}

impl<T: Showable + std::hash::Hash + Eq + Clone + Ord, B: logicaffeine_data::crdt::SetBias> Showable
    for logicaffeine_data::crdt::ORSet<T, B>
{
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut elements: Vec<&T> = self.iter().collect();
        elements.sort();
        show_joined(f, "{", elements.into_iter(), "}")
    }
}

impl<T: Showable + Clone + PartialEq> Showable for logicaffeine_data::crdt::RGA<T> {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        show_joined(f, "[", self.iter(), "]")
    }
}

impl<T: Showable + Clone + PartialEq> Showable for logicaffeine_data::crdt::YATA<T> {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        show_joined(f, "[", self.iter(), "]")
    }
}

impl<K: Showable + std::hash::Hash + Eq + Clone + Ord, T: Showable + Ord + Clone + Default> Showable
    for logicaffeine_data::crdt::ORMap<K, logicaffeine_data::crdt::LatestWrite<T>>
{
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{")?;
        for (i, (k, v)) in self.sorted_entries().into_iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            k.format_show(f)?;
            write!(f, ": ")?;
            v.get().format_show(f)?;
        }
        write!(f, "}}")
    }
}

// LWWRegister: show the current value
impl<T: Showable> Showable for logicaffeine_data::crdt::LWWRegister<T> {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.get().format_show(f)
    }
}

// MVRegister: show single value or conflict notation
impl<T: Showable + Clone + PartialEq> Showable for logicaffeine_data::crdt::MVRegister<T> {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let values = self.values();
        if values.len() == 1 {
            values[0].format_show(f)
        } else if values.is_empty() {
            write!(f, "nothing")
        } else {
            // Multiple concurrent values - show as conflict
            write!(f, "conflict[")?;
            for (i, val) in values.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                val.format_show(f)?;
            }
            write!(f, "]")
        }
    }
}

// Dynamic Value type for tuples
impl Showable for logicaffeine_data::Value {
    #[inline(always)]
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

// A tuple renders `(a, b, …)` — PARENS, distinct from a Seq's brackets. Because `LogosTuple` is its own
// type, this holds even nested (`[(1, 2), (3, 4)]`): the `Seq`'s `Showable` calls THIS per element.
impl Showable for logicaffeine_data::LogosTuple {
    fn format_show(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(")?;
        for (i, v) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            v.format_show(f)?;
        }
        write!(f, ")")
    }
}

/// The Show verb - prints value with natural formatting
/// Takes a reference to avoid moving the value.
#[inline(always)]
pub fn show<T: Showable>(value: &T) {
    struct Wrapper<'a, T>(&'a T);
    impl<T: Showable> Display for Wrapper<'_, T> {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            self.0.format_show(f)
        }
    }
    out::line(Wrapper(value));
}

/// A plain number's value as a float, whatever machine width the generated code gave it: an integer
/// literal nothing else constrains is an `i32` to rustc, and a narrowed integer any other width.
fn plain_number(any: &dyn std::any::Any) -> Option<f64> {
    macro_rules! widths {
        ($($t:ty),*) => {
            $(if let Some(n) = any.downcast_ref::<$t>() { return Some(*n as f64); })*
        };
    }
    widths!(i64, i32, i16, i8, isize, u64, u32, u16, u8, usize, f64, f32);
    None
}

/// A currency hole (`{x:$}`), rendered exactly as the interpreter's `apply_format_spec`: an integer
/// or a float as money with two decimals, an exact integer beyond `i64` with an exact `.00`, and
/// every other value — an exact decimal keeps its own digits — as `$` before its own display.
pub fn format_currency<T: Showable + std::any::Any>(value: &T) -> String {
    let any = value as &dyn std::any::Any;
    if let Some(n) = plain_number(any) {
        return format!("${:.2}", n);
    }
    if let Some(i) = any.downcast_ref::<logicaffeine_data::LogosInt>() {
        return match i.to_i64() {
            Some(n) => format!("${:.2}", n as f64),
            None => format!("${}.00", show_string(i)),
        };
    }
    format!("${}", show_string(value))
}

/// A fixed-precision hole (`{x:.2}`), rendered exactly as the interpreter's `apply_format_spec`:
/// an integer or a float to `precision` places, and every other value as its own display.
pub fn format_fixed<T: Showable + std::any::Any>(value: &T, precision: usize) -> String {
    let any = value as &dyn std::any::Any;
    if let Some(n) = plain_number(any) {
        return format!("{:.precision$}", n);
    }
    if let Some(i) = any.downcast_ref::<logicaffeine_data::LogosInt>() {
        let wide: f64 = show_string(i).parse().unwrap_or(f64::NAN);
        return format!("{:.precision$}", i.to_i64().map_or(wide, |n| n as f64));
    }
    show_string(value)
}

/// Render any `Showable` value to a `String` using the SAME natural formatting as `show` — used by
/// string interpolation holes so `"{s}"` matches `Show s` byte-for-byte, even for types (Sets, user
/// structs) that implement `Showable` but not `std::fmt::Display`.
#[inline(always)]
pub fn show_string<T: Showable>(value: &T) -> String {
    struct Wrapper<'a, T>(&'a T);
    impl<T: Showable> Display for Wrapper<'_, T> {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            self.0.format_show(f)
        }
    }
    format!("{}", Wrapper(value))
}

pub fn read_line() -> String {
    flush_stdout();
    let mut buffer = String::new();
    std::io::stdin().read_line(&mut buffer).unwrap_or(0);
    buffer.trim().to_string()
}

#[inline(always)]
pub fn print<T: Display>(x: T) {
    out::text(x);
}

#[inline(always)]
pub fn eprintln<T: Display>(x: T) {
    flush_stdout();
    eprintln!("{}", x);
}

#[inline(always)]
pub fn println<T: Display>(x: T) {
    out::line(x);
}

/// Write everything shown so far. Called wherever stdout's order against something else becomes observable: before
/// stderr, before input is read, before a program's own Rust writes to stdout, and when a program fails.
pub fn flush_stdout() {
    out::flush();
}

/// Write everything shown so far unless the buffer is held at this moment, never waiting for it: the flush a report
/// made from a signal handler can make.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn flush_stdout_if_free() {
    out::flush_if_free();
}

/// LOGOS's standard output: one process-wide buffer that every `Show` writes through.
///
/// `std::io::Stdout` is line-buffered whatever it is connected to, so a program showing a line at a time paid one
/// `write(2)` per line. This buffer holds 64 KiB when stdout is a pipe or a file and passes each line on at once when
/// it is a terminal, the C library's convention. Rust never drops a static, so the buffer is also emptied by an
/// `atexit` handler, which both a return from `main` and `std::process::exit` run; a `panic = "abort"` build ends in
/// the failure hook, which flushes first. A value is formatted into a reusable per-thread `String` before the lock is
/// taken and every error is raised after it is released, so nothing that can panic runs while the buffer is held and
/// the failure hook's flush can always take it.
#[cfg(not(target_arch = "wasm32"))]
mod out {
    use std::cell::RefCell;
    use std::fmt::{Display, Write as _};
    use std::io::{BufWriter, IsTerminal, Stdout, Write};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    struct Out {
        writer: BufWriter<Stdout>,
        terminal: bool,
    }

    static OUT: OnceLock<Mutex<Out>> = OnceLock::new();

    extern "C" {
        fn atexit(callback: extern "C" fn()) -> core::ffi::c_int;
    }

    extern "C" fn flush_at_exit() {
        flush_if_free();
    }

    /// Write out what is buffered unless the buffer is held, never waiting for it: safe where the holder cannot
    /// release it, at exit and inside a signal handler.
    pub fn flush_if_free() {
        if let Some(cell) = OUT.get() {
            if let Ok(mut out) = cell.try_lock() {
                let _ = out.writer.flush();
            }
        }
    }

    fn lock() -> MutexGuard<'static, Out> {
        let cell = OUT.get_or_init(|| {
            unsafe { atexit(flush_at_exit) };
            let stdout = std::io::stdout();
            let terminal = stdout.is_terminal();
            Mutex::new(Out {
                writer: BufWriter::with_capacity(64 * 1024, stdout),
                terminal,
            })
        });
        cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    thread_local! {
        static SCRATCH: RefCell<String> = RefCell::new(String::new());
    }

    fn emit(value: impl Display, newline: bool) {
        // Taken out of the cell while in use, so a `Display` that shows something itself formats into a fresh one.
        let mut text = SCRATCH
            .try_with(|scratch| scratch.take())
            .unwrap_or_default();
        text.clear();
        let _ = write!(text, "{value}");
        if newline {
            text.push('\n');
        }
        let written = {
            let mut out = lock();
            let terminal = out.terminal;
            out.writer.write_all(text.as_bytes()).and_then(|()| {
                if terminal && newline {
                    out.writer.flush()
                } else {
                    Ok(())
                }
            })
        };
        let _ = SCRATCH.try_with(|scratch| scratch.replace(text));
        if let Err(e) = written {
            panic!("failed printing to stdout: {e}");
        }
    }

    pub fn line(value: impl Display) {
        emit(value, true);
    }

    pub fn text(value: impl Display) {
        emit(value, false);
    }

    pub fn flush() {
        let Some(cell) = OUT.get() else { return };
        let flushed = cell
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .writer
            .flush();
        if let Err(e) = flushed {
            panic!("failed printing to stdout: {e}");
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod out {
    use std::fmt::Display;

    pub fn line(value: impl Display) {
        println!("{value}");
    }

    pub fn text(value: impl Display) {
        print!("{value}");
    }

    pub fn flush() {}
}
