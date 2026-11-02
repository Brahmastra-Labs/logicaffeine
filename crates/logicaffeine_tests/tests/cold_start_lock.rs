//! ★ LOCK — a program pays at startup for the library it uses, never for the whole library. ★
//!
//! The prelude's names, natives, cross-module pulls, splice runs and library sections are an index the build script
//! writes. Splicing a program that names nothing of the library, and establishing which of its functions are the
//! library's, read none of the library's text; splicing a program that names one module copies that module and
//! what it pulls in. Deriving any of it per process cost `largo run --interpret` of `Show 0.` 102 million
//! instructions where 0.10.0 took 11 million; reading the index, it takes 7 million.
//!
//! ★ THE YARDSTICK IS A BYTE COUNT, NOT A CLOCK. This binary's allocator counts the bytes the calling thread
//! allocates, and every bound is a fraction of the library's own size, so a derivation over the library fails it
//! by a wide margin whatever the box's load. Each test is the first splice of its process under nextest.

use logicaffeine_compile::loader::{
    apply_prelude_mapped, prelude_module_sources, LibraryProvenance,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        System.alloc(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

fn count(bytes: usize) {
    let _ = COUNTING.try_with(|on| {
        if on.get() {
            let _ = BYTES.try_with(|b| b.set(b.get() + bytes as u64));
        }
    });
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// What `f` returns and the bytes this thread allocated running it.
fn allocated_by<T>(f: impl FnOnce() -> T) -> (T, u64) {
    BYTES.with(|b| b.set(0));
    COUNTING.with(|on| on.set(true));
    let out = f();
    COUNTING.with(|on| on.set(false));
    (out, BYTES.with(|b| b.get()))
}

/// The bytes of every prelude module's source.
fn library_bytes() -> u64 {
    prelude_module_sources()
        .iter()
        .map(|s| s.len() as u64)
        .sum()
}

const SHOW_ZERO: &str = "## Main\nShow 0.\n";

#[test]
fn splicing_a_program_that_names_no_library_reads_none_of_it() {
    let ((spliced, _), bytes) = allocated_by(|| apply_prelude_mapped(SHOW_ZERO));
    assert_eq!(spliced, SHOW_ZERO);
    let library = library_bytes();
    eprintln!("splice of Show 0.: {bytes} bytes; library {library} bytes");
    assert!(
        bytes * 64 < library,
        "{bytes} bytes allocated to splice nothing, against a library of {library}"
    );
}

#[test]
fn the_provenance_of_a_program_that_names_no_library_reads_none_of_it() {
    let (_scope, bytes) = allocated_by(|| LibraryProvenance::of(SHOW_ZERO));
    let library = library_bytes();
    eprintln!("provenance of Show 0.: {bytes} bytes; library {library} bytes");
    assert!(
        bytes * 64 < library,
        "{bytes} bytes allocated for no library function, against a library of {library}"
    );
}

#[test]
fn splicing_one_module_copies_that_module_and_what_it_pulls() {
    let program = "## Main\nShow trim(\"  a  \").\n";
    let ((spliced, _), bytes) = allocated_by(|| apply_prelude_mapped(program));
    let copied = (spliced.len() - program.len()) as u64;
    let library = library_bytes();
    eprintln!("splice of trim: {bytes} bytes for {copied} spliced; library {library} bytes");
    assert!(spliced.contains("## To trim"), "{spliced}");
    assert!(
        copied * 4 < library,
        "trim pulls in {copied} of the library's {library} bytes"
    );
    assert!(
        bytes < 3 * copied + library / 64,
        "{bytes} bytes allocated to splice {copied}, against a library of {library}"
    );
}

#[test]
fn the_library_functions_of_a_spliced_program_are_found_by_lookup() {
    let program = "## Main\nShow trim(\"  a  \").\n";
    let (spliced, _) = apply_prelude_mapped(program);
    let spliced = spliced.into_owned();
    let (_scope, bytes) = allocated_by(|| LibraryProvenance::of(&spliced));
    let library = library_bytes();
    eprintln!(
        "provenance of the trim splice: {bytes} bytes for {} spliced; library {library}",
        spliced.len()
    );
    assert!(logicaffeine_compile::loader::library_owned_functions(&spliced).contains("trim"));
    assert!(
        bytes < 3 * spliced.len() as u64 + library / 64,
        "{bytes} bytes allocated for the provenance of {} bytes, against a library of {library}",
        spliced.len()
    );
}

#[test]
fn interpreting_show_zero_allocates_less_than_the_library() {
    let (result, bytes) =
        allocated_by(|| logicaffeine_compile::ui_bridge::interpret_for_ui_sync(SHOW_ZERO));
    let library = library_bytes();
    eprintln!("interpreting Show 0.: {bytes} bytes; library {library} bytes");
    assert_eq!(result.lines, vec!["0".to_string()], "{:?}", result.error);
    assert!(
        bytes < library,
        "{bytes} bytes allocated to show 0, against a library of {library}"
    );
}
