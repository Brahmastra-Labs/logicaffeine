//! Which compiled functions — and which ops — can run.
//!
//! A program's bytecode carries every function the loader spliced in: the author's, and whole
//! standard-library modules a type name or a builtin demanded. Most of a demanded module is never
//! called. A backend that lowers only what runs (the WebAssembly backend's dead-function
//! elimination) must agree with the gate that refuses what it cannot lower (the unlinkable-code
//! gate), or a function nothing calls either breaks the build (the gate refuses dead code) or ships
//! a stub a live call lands on (the backend drops live code). This module is the one answer both
//! ask.
//!
//! # The code stream
//!
//! Main is `code[0 .. main_end)`. Regular (`## To`) functions follow as a contiguous, entry-sorted
//! tail, each ending where the next begins. A closure body is INLINE: emitted inside its enclosing
//! region and jumped over by a `Jump` at `entry_pc - 1` whose target is the body's end, so a region
//! contains its closures' bodies as holes.
//!
//! # The edges
//!
//! A function runs when Main or a running function names it: a `Call`, a `MakeClosure` body, or a
//! `Spawn`/`SpawnHandle` task. Two facts no op carries are part of the graph too, because the
//! backends act on them:
//!
//! - `text_from_bytes` validates through `validUtf8 (data: Seq of Int) -> Bool`, a call the wasm
//!   backend writes from the builtin;
//! - a page's entries (`lgx_lower::LGX_ENTRIES`) are entered by the HOST, never by the module; so
//!   is the mount of a component the host was asked to mount on demand ([`HostEntries`]).
//!
//! Three facts gate edges:
//!
//! - a drawn component's members (its actions, bindings, patchers and callback trampolines) run only
//!   for an instance, so an edge into one from outside the component is followed once the
//!   component's render is reached (`ComponentOwners`);
//! - a `vdom.lg` feature's functions run only on a tape holding records its writers laid down
//!   (`lgx_lower::VDOM_FEATURES`), so an edge into one is followed once a writer is reached
//!   (`VdomFeatures`);
//! - hydration (`vdHydrate`) runs only for a page its build pre-rendered, which the host says
//!   ([`HostEntries::drawing_fresh`] says it never is).

use super::instruction::{CompiledProgram, Op};
use crate::semantics::builtins::BuiltinId;
use crate::Interner;

/// How the flat code stream partitions into functions.
pub struct CodeLayout {
    /// Main's region end (exclusive): the first regular-function entry, or `code.len()`.
    pub main_end: usize,
    /// Per-function `[start, end)` in the code stream, holes included.
    pub func_region: Vec<(usize, usize)>,
    /// Every closure body's `[start, end)` — the holes to excise from an enclosing region.
    pub closure_ranges: Vec<(usize, usize)>,
    /// `is_closure[fi]` — whether function `fi` is a closure body (a `MakeClosure` target).
    pub is_closure: Vec<bool>,
}

/// Resolve the [`CodeLayout`]: a `MakeClosure` target is a closure body spanning
/// `[entry, jump-over target)`; the regular functions are the entry-sorted tail after Main.
/// `Err` names a closure body with no jump-over, which no compiler output has.
pub fn code_layout(program: &CompiledProgram) -> Result<CodeLayout, &'static str> {
    let code_len = program.code.len();
    let nf = program.functions.len();
    let mut is_closure = vec![false; nf];
    for op in &program.code {
        if let Op::MakeClosure { func, .. } = *op {
            if (func as usize) < nf {
                is_closure[func as usize] = true;
            }
        }
    }
    let mut func_region = vec![(0usize, 0usize); nf];
    let mut closure_ranges = Vec::new();
    for (fi, f) in program.functions.iter().enumerate() {
        if is_closure[fi] {
            let entry = f.entry_pc;
            let end = match (entry >= 1).then(|| program.code.get(entry - 1)).flatten() {
                Some(Op::Jump { target }) if *target > entry && *target <= code_len => *target,
                _ => return Err("closure body without a jump-over"),
            };
            func_region[fi] = (entry, end);
            closure_ranges.push((entry, end));
        }
    }
    let mut regular: Vec<usize> = (0..nf).filter(|&fi| !is_closure[fi]).collect();
    regular.sort_by_key(|&fi| program.functions[fi].entry_pc);
    let main_end = regular
        .first()
        .map(|&fi| program.functions[fi].entry_pc)
        .unwrap_or(code_len);
    for w in 0..regular.len() {
        let fi = regular[w];
        let start = program.functions[fi].entry_pc;
        let end = regular
            .get(w + 1)
            .map(|&g| program.functions[g].entry_pc)
            .unwrap_or(code_len);
        func_region[fi] = (start, end);
    }
    Ok(CodeLayout {
        main_end,
        func_region,
        closure_ranges,
        is_closure,
    })
}

/// What can run.
pub struct Reach {
    /// `functions[fi]` — whether function `fi` can be entered.
    pub functions: Vec<bool>,
    /// `code[pc]` — whether the op at `pc` lies in Main or in the region of a function that can be
    /// entered. A region is scanned whole, holes included, so an inline closure body inside a
    /// running region counts as running with it: the edges it names are followed either way.
    pub code: Vec<bool>,
}

/// The functions a page's HOST enters by name, beyond Main: every fixed page entry of
/// `lgx_lower::LGX_ENTRIES` the program defines, always; and the mount of each component the host
/// is asked to be able to mount on demand — a test that mounts a component at a statement of its
/// own. A shipped page has no such components (its only mount is the one its own `main` performs),
/// so the default is the fixed page entries alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostEntries {
    mounts: Vec<String>,
    /// The host serves no pre-rendered markup, so the page's mount never adopts any.
    fresh: bool,
}

impl HostEntries {
    /// A page whose host may also mount each of `components`, by name, at any time.
    pub fn mounting<I, S>(components: I) -> HostEntries
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut mounts: Vec<String> = components.into_iter().map(Into::into).collect();
        mounts.sort();
        mounts.dedup();
        HostEntries {
            mounts,
            fresh: false,
        }
    }

    /// These entries for a host that serves the page with no pre-rendered markup — a page whose
    /// build could not pre-render it, a module emitted on its own: its mount draws fresh and never
    /// hydrates, so the hydration paths are left out.
    pub fn drawing_fresh(mut self) -> HostEntries {
        self.fresh = true;
        self
    }

    /// Whether the page's mount may adopt markup a build pre-rendered.
    pub fn hydrates(&self) -> bool {
        !self.fresh
    }

    /// The components the host may mount on demand, sorted.
    pub fn mounts(&self) -> &[String] {
        &self.mounts
    }

    /// Every function the host enters by name: the fixed page entries, then the requested mounts.
    pub fn names(&self) -> Vec<String> {
        use logicaffeine_language::parser::lgx_lower::{mount_fn_name, EntryHost, LGX_ENTRIES};
        LGX_ENTRIES
            .iter()
            .filter(|e| !e.family && e.host == EntryHost::Page)
            .map(|e| e.name.to_string())
            .chain(self.mounts.iter().map(|c| mount_fn_name(c)))
            .collect()
    }
}

/// The function `text_from_bytes` validates through.
pub const UTF8_VALIDATOR: &str = "validUtf8";

/// Walk the call graph from Main and a shipped page's host entries.
pub fn reach(program: &CompiledProgram, layout: &CodeLayout, interner: &Interner) -> Reach {
    reach_with(program, layout, interner, &HostEntries::default())
}

/// Walk the call graph from Main and every one of `entries`.
pub fn reach_with(
    program: &CompiledProgram,
    layout: &CodeLayout,
    interner: &Interner,
    entries: &HostEntries,
) -> Reach {
    let by_name = |name: &str| {
        program
            .functions
            .iter()
            .position(|f| interner.resolve(f.name) == name)
    };
    let utf8_validator = by_name(UTF8_VALIDATOR);
    let hydrate = if entries.hydrates() {
        None
    } else {
        by_name(HYDRATE)
    };
    let owners = ComponentOwners::of(program, interner);
    let mut features = VdomFeatures::of(program, interner);
    let mut functions = vec![false; program.functions.len()];
    let mut code = vec![false; program.code.len()];
    let mut pending: Vec<Vec<usize>> = vec![Vec::new(); owners.renders.len()];
    let mut stack: Vec<usize> = entries.names().iter().filter_map(|n| by_name(n)).collect();
    // `from` is the component that owns the region being scanned (`None` for Main and the page's
    // own entries): an edge from outside a drawn component into one of its members waits for that
    // component's render to be reached.
    let scan = |start: usize,
                end: usize,
                from: Option<usize>,
                code: &mut [bool],
                functions: &[bool],
                stack: &mut Vec<usize>,
                pending: &mut Vec<Vec<usize>>,
                features: &mut VdomFeatures| {
        for pc in start..end {
            code[pc] = true;
            match program.code[pc] {
                Op::Call { func, .. }
                | Op::MakeClosure { func, .. }
                | Op::Spawn { func, .. }
                | Op::SpawnHandle { func, .. } => {
                    let func = func as usize;
                    if Some(func) == hydrate {
                        continue;
                    }
                    match owners.gate(func, from) {
                        Some(c) if !functions[owners.renders[c]] => pending[c].push(func),
                        _ => features.follow(func, stack),
                    }
                }
                Op::CallBuiltin {
                    builtin: BuiltinId::TextFromBytes,
                    ..
                } => stack.extend(utf8_validator),
                _ => {}
            }
        }
    };
    scan(
        0,
        layout.main_end.min(program.code.len()),
        None,
        &mut code,
        &functions,
        &mut stack,
        &mut pending,
        &mut features,
    );
    while let Some(fi) = stack.pop() {
        if fi >= functions.len() || functions[fi] {
            continue;
        }
        functions[fi] = true;
        if let Some(c) = owners.renders.iter().position(|&r| r == fi) {
            stack.append(&mut pending[c]);
        }
        features.reached(fi, &mut stack);
        let (s, e) = layout.func_region[fi];
        if s <= e && e <= program.code.len() {
            scan(
                s,
                e,
                owners.owner[fi],
                &mut code,
                &functions,
                &mut stack,
                &mut pending,
                &mut features,
            );
        }
    }
    Reach { functions, code }
}

/// The function a page's mount adopts pre-rendered markup through.
const HYDRATE: &str = "vdHydrate";

/// Which `vdom.lg` feature each function belongs to (`lgx_lower::VDOM_FEATURES`), and which features
/// a reached writer has opened: an edge into a feature waits, in `pending`, for its first writer.
struct VdomFeatures {
    feature_of: Vec<Option<usize>>,
    writers: Vec<Vec<usize>>,
    open: Vec<bool>,
    pending: Vec<Vec<usize>>,
}

impl VdomFeatures {
    fn of(program: &CompiledProgram, interner: &Interner) -> VdomFeatures {
        use logicaffeine_language::parser::lgx_lower::VDOM_FEATURES;
        let names: Vec<&str> = program
            .functions
            .iter()
            .map(|f| interner.resolve(f.name))
            .collect();
        let feature_of = names
            .iter()
            .map(|n| VDOM_FEATURES.iter().position(|f| n.starts_with(f.prefix)))
            .collect();
        let writers = VDOM_FEATURES
            .iter()
            .map(|f| {
                names
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| f.writers.contains(n))
                    .map(|(fi, _)| fi)
                    .collect()
            })
            .collect();
        VdomFeatures {
            feature_of,
            writers,
            open: vec![false; VDOM_FEATURES.len()],
            pending: vec![Vec::new(); VDOM_FEATURES.len()],
        }
    }

    /// Follows an edge into `func` now, or once its feature opens.
    fn follow(&mut self, func: usize, stack: &mut Vec<usize>) {
        match self.feature_of.get(func).copied().flatten() {
            Some(f) if !self.open[f] => self.pending[f].push(func),
            _ => stack.push(func),
        }
    }

    /// `fi` was reached: every feature it writes opens, and the edges that waited on it are followed.
    fn reached(&mut self, fi: usize, stack: &mut Vec<usize>) {
        for f in 0..self.writers.len() {
            if !self.open[f] && self.writers[f].contains(&fi) {
                self.open[f] = true;
                stack.append(&mut self.pending[f]);
            }
        }
    }
}

/// Which drawn component each lowered function belongs to.
///
/// ★ A PAGE ENTRY FANS OUT OVER EVERY COMPONENT. The dispatcher calls every component's actions,
/// the clock every component's timers, the route and failure entries every component's handlers —
/// so rooting the entries would make every component's code reachable whether or not anything
/// draws it. A component's members run only for an instance, and an instance exists only once
/// its render has run: an edge from outside a component into one of its actions, bindings,
/// patchers or callback trampolines is followed when that component's `__render_` is reached,
/// and not before. Its render and its mount are how it comes to be drawn, so edges to those are
/// followed at once.
struct ComponentOwners {
    /// `renders[c]` — the function index of component `c`'s render.
    renders: Vec<usize>,
    /// `owner[fi]` — the component function `fi` is a member of, if any.
    owner: Vec<Option<usize>>,
    /// `drawn_by_edge[fi]` — whether `fi` is a render or a mount: followed from anywhere.
    drawn_by_edge: Vec<bool>,
}

impl ComponentOwners {
    fn of(program: &CompiledProgram, interner: &Interner) -> ComponentOwners {
        use logicaffeine_language::parser::lgx_lower::{
            action_fn_name, bind_fn_name, callback_trampoline_fn_name, mount_fn_name,
            patch_fn_name, render_fn_name,
        };
        let names: Vec<&str> = program
            .functions
            .iter()
            .map(|f| interner.resolve(f.name))
            .collect();
        let render_prefix = render_fn_name("");
        let components: Vec<(String, usize)> = names
            .iter()
            .enumerate()
            .filter_map(|(fi, n)| {
                n.strip_prefix(render_prefix.as_str())
                    .map(|c| (c.to_string(), fi))
            })
            .collect();
        let mut owner = vec![None; names.len()];
        let mut drawn_by_edge = vec![false; names.len()];
        for (fi, name) in names.iter().enumerate() {
            let mut best: Option<usize> = None;
            for (c, (component, _)) in components.iter().enumerate() {
                let drawn = *name == render_fn_name(component) || *name == mount_fn_name(component);
                let member = [
                    action_fn_name(component, ""),
                    bind_fn_name(component, ""),
                    patch_fn_name(component, ""),
                    callback_trampoline_fn_name(component, ""),
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix.as_str()));
                if drawn || member {
                    let longer = best.map_or(true, |b| component.len() > components[b].0.len());
                    if longer {
                        best = Some(c);
                        drawn_by_edge[fi] = drawn;
                    }
                }
            }
            owner[fi] = best;
        }
        ComponentOwners {
            renders: components.iter().map(|(_, fi)| *fi).collect(),
            owner,
            drawn_by_edge,
        }
    }

    /// The component an edge from a region of component `from` into `to` waits for, if any.
    fn gate(&self, to: usize, from: Option<usize>) -> Option<usize> {
        let c = self.owner.get(to).copied().flatten()?;
        (from != Some(c) && !self.drawn_by_edge[to]).then_some(c)
    }
}

/// Every op that can run in a shipped page, with its pc. A program whose layout cannot be resolved
/// is taken whole: the caller is a gate, and a gate that cannot tell must not wave code through.
pub fn running_ops<'p>(program: &'p CompiledProgram, interner: &Interner) -> Vec<(usize, &'p Op)> {
    running_ops_with(program, interner, &HostEntries::default())
}

/// Every op that can run once the host may also enter `entries`, with its pc — what a gate judges
/// when a module is built for a host that mounts components on demand.
pub fn running_ops_with<'p>(
    program: &'p CompiledProgram,
    interner: &Interner,
    entries: &HostEntries,
) -> Vec<(usize, &'p Op)> {
    match code_layout(program) {
        Ok(layout) => {
            let r = reach_with(program, &layout, interner, entries);
            program
                .code
                .iter()
                .enumerate()
                .filter(|(pc, _)| r.code[*pc])
                .collect()
        }
        Err(_) => program.code.iter().enumerate().collect(),
    }
}
