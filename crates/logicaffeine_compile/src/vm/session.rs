//! A program the VM has run, held open so a host can call back into it.
//!
//! A browser instantiates a page's module, runs `main` once, and from then on calls INTO it —
//! an event handler, a fetch completion, a timer tick, a failure handler. Each is the host
//! invoking an export by name or a closure the module handed it earlier, against the globals
//! `main` built. A trap unwinds to the host and leaves the instance callable.
//!
//! [`Session`] is that shape for the in-process bytecode VM: [`Session::run_main`] once, then
//! [`Session::invoke`] and [`Session::call_value`] for as long as the host likes. It is the
//! seam `page_host` drives a lowered page through, and what lets a host-delivered completion run
//! in-process at all.

use super::instruction::FuncIdx;
use super::machine::Vm;
use crate::intern::Interner;
use crate::interpreter::RuntimeValue;
use std::collections::HashMap;

/// A VM whose program has run (or is about to), open to calls from the host.
pub struct Session<'p> {
    vm: Vm<'p>,
    /// Every named function of the program, by its source name.
    functions: HashMap<String, FuncIdx>,
    main_ran: bool,
}

impl<'p> Session<'p> {
    /// Hold `vm` open for host calls. Nothing runs until [`Session::run_main`].
    pub fn new(vm: Vm<'p>, interner: &Interner) -> Self {
        let functions = vm
            .program()
            .fn_index
            .iter()
            .map(|(sym, fi)| (interner.resolve(*sym).to_string(), *fi))
            .collect();
        Session {
            vm,
            functions,
            main_ran: false,
        }
    }

    /// Run the program's main code, once. A failure is reported and the session stays callable,
    /// exactly as a browser can still call into a module whose `main` trapped.
    pub fn run_main(&mut self) -> Result<(), String> {
        if self.main_ran {
            return Err(
                "vm session: main has already run, and running it again would \
                        re-initialize every global the program is using"
                    .to_string(),
            );
        }
        self.main_ran = true;
        let outcome = self.vm.run_keeping_entries();
        if outcome.is_err() {
            self.vm.quiesce();
        }
        outcome
    }

    /// The failure a page entry the program took itself contained and no handler absorbed, with
    /// its failure site — the host carrying the page on takes it as the page's failure
    /// ([`Vm::take_unhandled`]).
    pub fn take_unhandled(&mut self) -> Option<(String, Option<u32>)> {
        self.vm.take_unhandled()
    }

    /// Call the function the program names `name`, with `args` in parameter order.
    pub fn invoke(&mut self, name: &str, args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
        let fi = *self
            .functions
            .get(name)
            .ok_or_else(|| format!("vm session: this program has no function `{name}`"))?;
        self.require_main(name)?;
        let expected = self.vm.program().functions[fi as usize].param_count as usize;
        if args.len() != expected {
            return Err(format!(
                "`{name}` expects {expected} arguments, got {}",
                args.len()
            ));
        }
        self.vm.invoke_function(fi, args)
    }

    /// Call a closure the program handed out — a completion, a listener, a route handler.
    pub fn call_value(
        &mut self,
        callee: &RuntimeValue,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, String> {
        self.require_main("a closure")?;
        self.vm.call_closure(callee, args)
    }

    /// A page reload: the program re-instantiated with the same configuration and its main run
    /// again. Every global starts over; what lives outside the program (the document recorder,
    /// browser storage) is the host's to keep or clear.
    pub fn reload(&mut self) -> Result<(), String> {
        self.vm = self.vm.reinstantiate();
        self.main_ran = false;
        self.run_main()
    }

    /// Whether the program defines a function named `name`.
    pub fn has(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    /// The current value of the named global, shown exactly as `Show` would render it — `None`
    /// when the program declares no such global, or it has not been assigned one yet. The read a
    /// host needs for state a lowering writes into a global specifically so the host can see it.
    pub fn global_text(&self, name: &str) -> Option<String> {
        self.vm
            .debug_view()
            .globals
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    /// The number of top-level elements the named global holds, when it is a `List`/`Set`
    /// (`[a, b, c]`) or a `Map` (`{k: v, k: v}`) — `None` when the global does not exist, has no
    /// value yet, or does not display as one of those two bracket shapes. Reads
    /// [`Session::global_text`]'s display string rather than the VM's own value representation
    /// (nothing in this crate exposes that to a host), so it walks the string BRACKET-DEPTH-AWARE
    /// — counting a comma only at depth 1, never inside a nested collection's own brackets — the
    /// one thing that would otherwise make this indistinguishable from a naive split. It remains
    /// unsound only for a leaf value whose OWN display contains an unbracketed `,`/`[`/`{`/`}`/`]`
    /// (a `Text`, which displays raw and unquoted) — exactly why this is a length probe for a
    /// table of plain scalars (an `Int`-keyed listener/site map, the shape a boundedness proof
    /// measures), not a general-purpose value parser.
    pub fn global_len(&self, name: &str) -> Option<usize> {
        let text = self.global_text(name)?;
        let inner = text
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .or_else(|| text.strip_prefix('{').and_then(|s| s.strip_suffix('}')))?;
        if inner.is_empty() {
            return Some(0);
        }
        let mut depth = 0i32;
        let mut count = 1usize;
        for c in inner.chars() {
            match c {
                '[' | '{' | '(' => depth += 1,
                ']' | '}' | ')' => depth -= 1,
                ',' if depth == 0 => count += 1,
                _ => {}
            }
        }
        Some(count)
    }

    /// Every line the program has shown, main and every call since, in order.
    pub fn lines(&self) -> &[String] {
        self.vm.lines()
    }

    /// The machine itself, for the structural tests beside it.
    #[cfg(test)]
    pub(super) fn machine(&self) -> &Vm<'p> {
        &self.vm
    }

    fn require_main(&self, what: &str) -> Result<(), String> {
        if self.main_ran {
            Ok(())
        } else {
            Err(format!(
                "vm session: cannot call {what} before main has run — main initializes the \
                 globals every function reads"
            ))
        }
    }
}
