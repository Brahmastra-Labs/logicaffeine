//! Lowering a component's `View:` into the DOM calls every tier already runs.
//!
//! The tree-walker renders an LGX component from its first-class AST, with the whole runtime
//! behind it: instances, transactional turns, the journal. That is the REFERENCE semantics, and
//! it stays.
//!
//! The bytecode VM — and therefore WebAssembly, and therefore a browser — reaches the same page
//! by a different road. A component's view is compiled ONCE, here, into ordinary LOGOS: a
//! `## To __render_<Name> (parent):` function whose body is the `createElement` /
//! `setAttribute` / `appendChild` calls the retiring dialect has always lowered to. Those
//! builtins are already compiled by the VM and already lowered to wasm, so the frontend reaches
//! the browser with NO new ISA surface — no opcode to classify in six device catalogs, no
//! `wasm_aot_lock` bucket that would have to lie about a lowering that does not exist.
//!
//! Both forms are emitted. The interpreter reads the `ComponentDef` and ignores the function;
//! the VM compiles the function and treats the definition as a declaration. `Mount App.` means
//! "render the instance" on one tier and "call `__render_App`" on the other, and
//! `lgx_vm_parity_lock` holds the two to the same bytes.
//!
//! What is NOT here is everything that needs the runtime rather than the DOM: actions, turns,
//! rollback, the journal, stores, directives. Those need a host that can catch a trap, which the
//! VM has no way to express — so a component using them has no render function, and the VM
//! refuses it LOUDLY rather than rendering half a page.
//!
//! Split by concern across this directory's modules: naming/binding conventions (`naming`), the
//! lowerability/refusal gate every construct passes through before compiling (`gate`), the
//! derived JSON codec (`json`), sessions, `## Route`s and server rendering (`server`), the per-component driver (`components`), DOM events and dispatch (`events`),
//! patchable sites and per-instance state (`sites`), the page clock and the unmount sweep
//! (`sweep`), typed `fetch` (`fetch`), `## Store` lowering (`stores`), element/localStorage
//! commands (`elements`), `Mount` (`mount`), turn wrapping and its epilogue (`turns`), and the
//! view-tree walk that builds a render function (`render`). Every item this module used to export
//! at the flat `lgx_lower::` path still does, through the `pub use` lines below — this split is a
//! pure reorganization, not a change to what any other file may call.

mod components;
mod elements;
mod events;
mod fetch;
mod gate;
mod json;
mod mount;
mod naming;
mod render;
mod server;
mod shared;
mod sites;
mod stores;
mod sweep;
mod turns;
mod views;

pub use events::*;
pub use gate::*;
pub use naming::*;
pub use server::*;
pub(crate) use views::ViewLowering;

pub(crate) use shared::{shared_field_kind, SharedFieldKind};
