//! ★ LOCK — the host puts a string on the MODULE's own heap, not a page it grows itself. ★
//!
//! Before this step, `runtime.js`'s `allocText` bump-allocated host text into linear-memory pages it
//! grew and owned itself, ABOVE every page the module's own allocator ever reaches — a second heap,
//! with its own bump pointer and no way back into the free-list machinery Steps 1-2 built. A string
//! the host hands the module (a DOM read, a stored value, a fetch response) could never be freed by
//! anything this allocator knows about.
//!
//! `__logos_text_new(len) -> handle` closes that: it allocates the block through
//! `__logos_alloc` — a `len`-byte data buffer plus a 16-byte header, exactly the shape a module-side
//! Text already has — so a host string is a heap block like any other, indistinguishable at the next
//! allocation site from one the module built itself.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::mount_interactive;

/// A page interactive enough to carry the release-protocol exports: one click listener, doing
/// nothing this file cares about.
const CLICKER: &str = "## Component App\n\
   \n\
   State:\n\
  \x20   Let n be 0.\n\
   \n\
   Actions:\n\
  \x20   To bump:\n\
  \x20       Set n to n + 1.\n\
   \n\
   View:\n\
  \x20   <main><p id=\"p\">{n}</p><button id=\"b\" on-click={bump}>x</button></main>\n\
   \n\
   ## Main\n\
   Mount App.\n";

/// A program with no callback at all — `main` runs once and the module never hears from a host
/// again — so it has no use for a text-allocating entry point and must not carry one.
const BATCH: &str = "## Main\nShow \"hello\".\n";

#[test]
fn an_interactive_module_exports_text_new() {
    let page = mount_interactive(CLICKER);
    assert!(
        !page.listening("click").is_empty(),
        "the page must have registered its click listener"
    );
    // Exercised through `InteractivePage::text_new`, which panics if the export is missing —
    // reaching the assert below IS the property.
    let mut page = page;
    let h = page.text_new(5);
    assert_ne!(
        h, 0,
        "__logos_text_new must return a real block, not a null handle"
    );
}

#[test]
fn a_batch_program_does_not_carry_text_new() {
    let module =
        logicaffeine_compile::compile::compile_to_wasm(BATCH).expect("a batch program compiles");
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
    assert!(
        m.exports().all(|e| e.name() != "__logos_text_new"),
        "a program nothing ever calls back into has no host to hand a Text to, and must not pay for the export"
    );
}

#[test]
fn text_new_allocates_through_the_shared_allocator() {
    let mut page = mount_interactive(CLICKER);
    // The page's own paint leaves blocks on the free lists, and a `text_new` may legitimately reuse
    // them — so the free lists are drained first (far more calls than the paint left blocks), and
    // what is measured is calls served by new memory alone: each one's cost, exactly.
    for _ in 0..900 {
        page.text_new(5);
    }
    let before = page.heap_allocated();
    let mut h = 0;
    for _ in 0..100 {
        h = page.text_new(5);
    }
    let after = page.heap_allocated();
    // 5 bytes of data rounds to the 8-byte exact class, plus a counted Text header — its 16 bytes and
    // the true-count word after them, which the page's safepoint frees it by — rounded to 24: 32
    // bytes a call, the same accounting every other Text construction on this tier pays — not a
    // second, unaccounted allocator bolted on beside it.
    assert_eq!(
        after - before,
        100 * 32,
        "text_new(5) must cost exactly one 8-byte data block plus one counted 24-byte header on the \
         module's own allocator (got {} bytes over 100 calls) — a host string must be visible to the \
         same `__logos_heap_allocated` reading every other allocation is",
        after - before
    );
    assert!(h > 0, "text_new must hand back a real address");
}

#[test]
fn text_new_round_trips_the_bytes_the_host_writes() {
    let mut page = mount_interactive(CLICKER);
    let s = "héllo wörld";
    let bytes = s.as_bytes();
    let h = page.text_new(bytes.len() as i32);
    let data_ptr = page.word(h + 8);
    page.write_bytes(data_ptr, bytes);
    assert_eq!(page.read_text(h), s, "the block text_new(len) returns must be exactly len bytes, writable at its data_ptr, and readable back as the same Text a module built itself");
}

#[test]
fn two_texts_of_the_same_length_do_not_alias() {
    let mut page = mount_interactive(CLICKER);
    let a = page.text_new(4);
    page.write_bytes(page.word(a + 8), b"aaaa");
    let b = page.text_new(4);
    page.write_bytes(page.word(b + 8), b"bbbb");
    assert_eq!(
        page.read_text(a),
        "aaaa",
        "writing the second Text must not overwrite the first"
    );
    assert_eq!(page.read_text(b), "bbbb");
}
