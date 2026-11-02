use std::collections::HashMap;
use std::fmt::Write;

use crate::analysis::TypeRegistry;
use crate::ast::stmt::*;
use crate::intern::{Interner, Symbol};
use logicaffeine_language::ast_depth::{expr_children, for_each_node, walk_nodes, Visited, Walk};

pub mod concurrency;
pub mod cshake;
pub(crate) mod emit;
mod absent;
mod equality;
mod exact;
pub mod from_program;
mod numbers;
pub(crate) mod runtime;
mod sharing;
mod stepped;
pub(crate) mod tower;
pub(crate) mod types;
pub mod value_runtime;
mod values;
mod word_ranges;
pub(crate) mod words;

use runtime::{
    c_net_runtime, c_runtime, C_CONCURRENCY_RUNTIME, C_CRDT_RUNTIME, C_STEPPED_HOST,
    STEPPED_SCHEDULER,
};

/// Marks the boundary between the fixed runtime/type prelude and the program's own
/// code in a `codegen_program_c` translation unit. The CUDA AOT tier splits here to
/// lift the user code onto the device; it is an ordinary C comment to gcc.
pub const USER_CODE_SENTINEL: &str = "// ==== LOGOS user code (device-liftable) ====";

/// The C family's scheduler: `logicaffeine_runtime::Scheduler::process_step` ported once, in C every
/// tier compiles — the host C and RISC-V coroutine runtimes, and the stepped tier's frames CUDA and the
/// bytecode walker run. A tier defines its qualifiers, its allocator and its own task fields before
/// this text, and supplies only the yield (see the file's header).
pub const SCHED_CORE: &str = include_str!("sched_core.c");

/// The C family's convergent collections: the data crate's observed-remove set (add-wins and remove-wins) and
/// its observed-remove map of last-writer values, ported once, in C every tier compiles — both C generators,
/// the stepped tier and RISC-V. A tier defines its qualifiers and its allocator before this text (see the
/// file's header).
pub const CRDT_CORE: &str = include_str!("crdt_core.c");

/// The files a program reads, writes and mounts (`Read … from file`, `Write … to file`, `Mount`), resolved under
/// the working directory as the native file system resolves them. Emitted only for a program that uses one.
pub const FILE_RUNTIME: &str = include_str!("file_io.c");

/// The list of values-or-nothing (`Seq_logos_opt`): its storage, whatever the values hold.
const OPT_SEQ_FAMILY: &str = "typedef struct { logos_opt *data; size_t len; size_t cap; } Seq_logos_opt;
__attribute__((unused)) static Seq_logos_opt seq_logos_opt_new(void) { return (Seq_logos_opt){NULL, 0, 0}; }
__attribute__((unused)) static void seq_logos_opt_push(Seq_logos_opt *s, logos_opt val) { if (s->len == s->cap) { s->cap = s->cap ? s->cap * 2 : 8; s->data = (logos_opt *)lc_gc_realloc(s->data, s->cap * sizeof(logos_opt)); } s->data[s->len++] = val; }
__attribute__((unused)) static logos_opt seq_logos_opt_get(Seq_logos_opt *s, int64_t idx) { return s->data[logos_idx0(idx, s->len)]; }
__attribute__((unused)) static void seq_logos_opt_set(Seq_logos_opt *s, int64_t idx, logos_opt val) { s->data[logos_idx0(idx, s->len)] = val; }
__attribute__((unused)) static int64_t seq_logos_opt_len(Seq_logos_opt *s) { return (int64_t)s->len; }
";

/// A value-or-nothing (`logos_opt`) for a program without the scheduler, whose runtime carries it otherwise.
const OPT_RUNTIME: &str = concat!(
    "\n#ifndef LOGOS_API\n#define LOGOS_API static __attribute__((unused))\n#endif\n",
    include_str!("opt.c"),
);

/// The names a zone or a spawned agent binds to nothing that the program also reads or sets by name: the only
/// ones a variable is needed for.
fn nothing_names(stmts: &[Stmt]) -> std::collections::HashSet<Symbol> {
    let (mut bound, mut named) = (
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    );
    for_each_node(stmts, |node| match node {
        Visited::Stmt(Stmt::Zone { name, .. } | Stmt::Spawn { name, .. }) => {
            bound.insert(*name);
        }
        Visited::Expr(Expr::Identifier(sym)) | Visited::Stmt(Stmt::Set { target: sym, .. }) => {
            named.insert(*sym);
        }
        _ => {}
    });
    bound.intersection(&named).copied().collect()
}

/// The functions a call to which neither fails nor acts: every parameter and the result a number or a truth
/// value, and a body of bindings of its own names, branches, loops and `Return`s over sums, differences,
/// products and comparisons of numbers (which never fail: an `Int` past `i64` is exact) and calls to such
/// functions — nothing read from outside the call. The largest such set: a function assumed quiet stays so unless
/// its body proves otherwise, so a recursive one is quiet when its recursion is.
fn quiet_functions(stmts: &[Stmt], interner: &Interner) -> std::collections::HashSet<Symbol> {
    use std::collections::HashSet;
    let scalar = |t: &TypeExpr| matches!(t, TypeExpr::Primitive(n) | TypeExpr::Named(n) if matches!(interner.resolve(*n), "Int" | "Real" | "Float" | "Bool"));
    let candidates: Vec<(Symbol, HashSet<Symbol>, &[Stmt])> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef {
                name,
                params,
                body,
                return_type: Some(rt),
                is_native: false,
                ..
            } if scalar(rt) && params.iter().all(|(_, t)| scalar(t)) => {
                Some((*name, params.iter().map(|(p, _)| *p).collect(), *body))
            }
            _ => None,
        })
        .collect();
    let mut quiet: HashSet<Symbol> = candidates.iter().map(|(n, _, _)| *n).collect();
    fn expr_quiet(e: &Expr, names: &HashSet<Symbol>, quiet: &HashSet<Symbol>) -> bool {
        match e {
            Expr::Literal(
                crate::ast::stmt::Literal::Number(_)
                | crate::ast::stmt::Literal::Float(_)
                | crate::ast::stmt::Literal::Boolean(_),
            ) => true,
            Expr::Identifier(s) => names.contains(s),
            Expr::Not { operand } => expr_quiet(operand, names, quiet),
            Expr::BinaryOp { op, left, right } => {
                use crate::ast::stmt::BinaryOpKind as Op;
                matches!(
                    op,
                    Op::Add
                        | Op::Subtract
                        | Op::Multiply
                        | Op::Eq
                        | Op::NotEq
                        | Op::Lt
                        | Op::Gt
                        | Op::LtEq
                        | Op::GtEq
                        | Op::And
                        | Op::Or
                ) && expr_quiet(left, names, quiet)
                    && expr_quiet(right, names, quiet)
            }
            Expr::Call { function, args } => {
                quiet.contains(function) && args.iter().all(|a| expr_quiet(a, names, quiet))
            }
            _ => false,
        }
    }
    fn block_quiet(body: &[Stmt], names: &mut HashSet<Symbol>, quiet: &HashSet<Symbol>) -> bool {
        body.iter().all(|s| match s {
            Stmt::Let { var, value, .. } => {
                let ok = expr_quiet(value, names, quiet);
                names.insert(*var);
                ok
            }
            Stmt::Set { target, value } => {
                names.contains(target) && expr_quiet(value, names, quiet)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                expr_quiet(cond, names, quiet)
                    && block_quiet(then_block, &mut names.clone(), quiet)
                    && else_block.map_or(true, |b| block_quiet(b, &mut names.clone(), quiet))
            }
            Stmt::While { cond, body, .. } => {
                expr_quiet(cond, names, quiet) && block_quiet(body, &mut names.clone(), quiet)
            }
            Stmt::Return { value: Some(v) } => expr_quiet(v, names, quiet),
            _ => false,
        })
    }
    loop {
        let before = quiet.len();
        for (name, params, body) in &candidates {
            if quiet.contains(name) && !block_quiet(body, &mut params.clone(), &quiet) {
                quiet.remove(name);
            }
        }
        if quiet.len() == before {
            return quiet;
        }
    }
}

/// The C of a program every run of which fails before any output, `reason` on stderr: what a program the VM
/// refuses before running it does.
pub fn refused_program(reason: &str) -> String {
    format!(
        "#include <stdio.h>\n\nint main(void) {{\n    fputs({}, stderr);\n    return 1;\n}}\n",
        emit::c_string_literal(&format!("{reason}\n"))
    )
}

/// Texts a program appends to in place, and texts joined from several parts (see `text_builder.c`).
const TEXT_BUILDER: &str = include_str!("text_builder.c");
/// Machine-word arithmetic that notices when it leaves the word, and the exact `&` it falls back to.
const CHECKED_INT: &str = include_str!("checked_int.c");

/// A function value: the code it runs, which takes the environment first, and the environment.
const CLOSURE_TYPE: &str = "\ntypedef struct { void *call; void *env; } LClosure;\n";

/// Whether the program reads, writes or mounts a file anywhere.
fn program_uses_files(stmts: &[Stmt]) -> bool {
    let mut uses = false;
    for_each_node(stmts, |node| {
        uses |= matches!(
            node,
            Visited::Stmt(
                Stmt::ReadFrom {
                    source: ReadSource::File(_),
                    ..
                } | Stmt::WriteFile { .. }
                    | Stmt::Mount { .. }
            )
        );
    });
    uses
}

/// How a concurrent program's tasks run: as coroutines on stacks of their own (`ucontext`, host C), or as
/// resumable step functions over heap frames (the stepped tier: the host walker and the GPU).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTier {
    Coroutines,
    Stepped,
}

/// Is this statement one of LOGOS's deterministic-concurrency operations — channels, tasks, `Await the
/// first of`, virtual-time `Sleep`?
pub(super) fn is_concurrency_stmt(s: &Stmt) -> bool {
    matches!(
        s,
        Stmt::LaunchTask { .. }
            | Stmt::LaunchTaskWithHandle { .. }
            | Stmt::CreatePipe { .. }
            | Stmt::SendPipe { .. }
            | Stmt::ReceivePipe { .. }
            | Stmt::TrySendPipe { .. }
            | Stmt::TryReceivePipe { .. }
            | Stmt::StopTask { .. }
            | Stmt::CloseChannel { .. }
            | Stmt::AwaitTask { .. }
            | Stmt::Select { .. }
            | Stmt::Sleep { .. }
    )
}

/// Whether the program uses concurrency anywhere — Main, every function body, every nested block. This
/// is the GATE for the scheduler runtime and the `main`-as-a-task bootstrap: when it is `false`,
/// `codegen_program_c` emits the C a world without the scheduler would, byte for byte.
pub(crate) fn program_uses_concurrency(stmts: &[Stmt]) -> bool {
    let mut uses = false;
    for_each_node(stmts, |node| {
        if let Visited::Stmt(s) = node {
            uses |= is_concurrency_stmt(s);
        }
    });
    uses
}

/// Whether a statement uses the offline networking transport — the same kind of gate as
/// `program_uses_concurrency`, so a non-net program emits byte-identical C (the ~1246 non-net tests stay
/// fixed). `Listen`/`ConnectTo`/`LetPeerAgent`/`Sync`/`SendMessage`/`StreamMessage`/`AwaitMessage`.
fn stmt_uses_net(s: &Stmt) -> bool {
    match s {
        Stmt::Listen { .. }
        | Stmt::ConnectTo { .. }
        | Stmt::LetPeerAgent { .. }
        | Stmt::Sync { .. }
        | Stmt::SendMessage { .. }
        | Stmt::StreamMessage { .. }
        | Stmt::AwaitMessage { .. } => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            then_block.iter().any(stmt_uses_net)
                || else_block
                    .map(|b| b.iter().any(stmt_uses_net))
                    .unwrap_or(false)
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::FunctionDef { body, .. } => {
            body.iter().any(stmt_uses_net)
        }
        _ => false,
    }
}

/// Whether the whole program uses the offline networking transport (Main + every function body).
fn program_uses_net(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_uses_net)
}

/// NOTHING delegates to the AST-C backend anymore — the flat Op-walker lowers EVERY bytecode op it can reach
/// natively, including the LAST holdout, the real-socket duplex TCP client (`Tcp`, over the `C_TCP_RUNTIME`
/// POSIX socket runtime), alongside the offline networking transport (`Net*`, the loopback `c_net_runtime`),
/// the single-replica CRDT lattice (`Crdt*`/`NewCrdt`), and runtime authorization (`CheckPolicy`). The gate is
/// EMPTY: this always returns `false`. It is kept (rather than deleted) as the honest, self-documenting proof
/// that the delegation set is closed — a future op that genuinely cannot be lowered would be added to
/// `ASTC_ONLY_OPS` here, and the census locks would force that decision. Reading the FINALIZED bytecode (not
/// the AST) means it can never disagree with what actually runs.
pub(crate) fn program_needs_astc_delegation(program: &crate::vm::CompiledProgram) -> bool {
    const ASTC_ONLY_OPS: &[&str] = &[];
    // Every function body is compiled into the single flat `code` op stream (indexed by `entry_pc`),
    // so one scan of `code` sees Main and every function.
    program
        .code
        .iter()
        .any(|op| ASTC_ONLY_OPS.contains(&op.name()))
}

/// Whether a field's DECLARED type is a single-replica counter CRDT — a `ConvergentCount` (grow-only
/// G-Counter) or a `Tally` (PN-Counter). Such a field's C cell holds a `logos_crdt*` handle (cast to the
/// `int64_t` the declared `Int` field lays out as); reading it SUMS the per-replica map. The `GCounter`/
/// `PNCounter` aliases are the internal names the same types are known by (matching the tree-walker's
/// `new_counter`/`new_pn_counter` field detection + the RISC-V tier).
fn field_type_is_counter_crdt(ft: &crate::analysis::FieldType, interner: &Interner) -> bool {
    matches!(
        ft,
        crate::analysis::FieldType::Named(sym) | crate::analysis::FieldType::Primitive(sym)
            if matches!(interner.resolve(*sym), "ConvergentCount" | "GCounter" | "Tally" | "PNCounter")
    )
}

/// Every `(struct symbol, field symbol)` in the registry whose field is a counter CRDT. This is the
/// side-set the codegen consults: seeding a `New` struct's counter fields, projecting a `FieldAccess` on
/// one to the map SUM, and taking the per-replica MAX on `Merge`/`Join`.
fn counter_crdt_fields(
    registry: &TypeRegistry,
    interner: &Interner,
) -> std::collections::HashSet<(Symbol, Symbol)> {
    let mut out = std::collections::HashSet::new();
    for (name, def) in registry.iter_types() {
        if let crate::analysis::TypeDef::Struct { fields, .. } = def {
            for f in fields {
                if field_type_is_counter_crdt(&f.ty, interner) {
                    out.insert((*name, f.name));
                }
            }
        }
    }
    out
}

/// Whether the program has ANY counter-CRDT field — the GATE for the `logos_crdt` runtime. When false,
/// the CRDT runtime is not emitted and the output is byte-identical to a world without it (the invariant
/// that keeps the ~1246 non-CRDT C tests fixed).
fn program_uses_crdt_counter(registry: &TypeRegistry, interner: &Interner) -> bool {
    !counter_crdt_fields(registry, interner).is_empty()
}

/// What each function's calls read and write of the program's file-scope statics ([`static_reach`]), for the
/// shared operand sequencing (`optimize::sequence`).
pub(crate) fn call_effects(stmts: &[Stmt]) -> crate::optimize::sequence::CallEffects {
    let reach = static_reach(stmts);
    let nonempty = |m: &HashMap<Symbol, std::collections::HashSet<Symbol>>| {
        m.iter()
            .filter(|(_, s)| !s.is_empty())
            .map(|(f, s)| (*f, s.clone()))
            .collect()
    };
    crate::optimize::sequence::CallEffects {
        reads: nonempty(&reach.reach),
        writes: nonempty(&reach.writes),
    }
}

/// The texts a `mutable` parameter lends, by name (see `CContext::plain_texts`): each `mutable` Text parameter,
/// and every name a call passes to one.
fn lent_texts(stmts: &[Stmt], ctx: &CContext) -> std::collections::HashSet<String> {
    let lends_text = |function: Symbol, i: usize| {
        ctx.fn_inout
            .get(&function)
            .and_then(|ps| ps.get(i))
            .copied()
            .unwrap_or(false)
            && ctx.fn_params.get(&function).and_then(|ps| ps.get(i)) == Some(&CType::String)
    };
    let mut out = std::collections::HashSet::new();
    logicaffeine_language::ast_depth::for_each_node(stmts, |node| {
        if let logicaffeine_language::ast_depth::Visited::Stmt(Stmt::FunctionDef {
            name,
            params,
            ..
        }) = node
        {
            for (i, (p, _)) in params.iter().enumerate() {
                if lends_text(*name, i) {
                    out.insert(ctx.interner.resolve(*p).to_string());
                }
            }
        }
    });
    crate::analysis::callgraph::for_each_call_site(stmts, &mut |function, args| {
        for (i, arg) in args.iter().enumerate() {
            if let Expr::Identifier(x) = arg {
                if lends_text(function, i) {
                    out.insert(ctx.interner.resolve(*x).to_string());
                }
            }
        }
    });
    out
}

/// The program's bindings a function reaches, and what the C tier refuses about them.
struct ProgramStatics {
    /// Each `## Main` root `Let` some `## To` body names, in the order Main binds them.
    names: Vec<Symbol>,
    /// Why the C tier cannot keep them faithfully, one sentence each (an `#error` apiece).
    refusals: Vec<String>,
}

/// Main's statements at its root, through the blocks that run in place (`Splice`, `Reversibly`).
fn root_statements<'s, 'a>(stmts: &'s [Stmt<'a>], out: &mut Vec<&'s Stmt<'a>>) {
    for s in stmts {
        match s {
            Stmt::FunctionDef { .. } => {}
            Stmt::Splice { body } | Stmt::Reversibly { body } => root_statements(body, out),
            other => out.push(other),
        }
    }
}

/// The names a body reads or sets, and the functions it calls.
fn names_and_calls(
    body: &[Stmt],
) -> (
    std::collections::HashSet<Symbol>,
    std::collections::HashSet<Symbol>,
) {
    let (mut names, mut calls) = (
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    );
    for_each_node(body, |node| match node {
        Visited::Expr(Expr::Identifier(sym)) => {
            names.insert(*sym);
        }
        Visited::Expr(Expr::Call { function, .. }) | Visited::Stmt(Stmt::Call { function, .. }) => {
            calls.insert(*function);
        }
        Visited::Stmt(Stmt::Set { target, .. }) => {
            names.insert(*target);
        }
        _ => {}
    });
    (names, calls)
}

/// The program's bindings a function reaches, and, through every call, which of them each function reads and which
/// it writes.
pub(super) struct StaticReach<'s, 'a> {
    /// Each `## Main` root `Let` some `## To` body names, in the order Main binds them.
    pub(super) names: Vec<Symbol>,
    /// The bindings each function reads or writes, its callees' included.
    pub(super) reach: HashMap<Symbol, std::collections::HashSet<Symbol>>,
    /// The bindings each function writes, its callees' included.
    pub(super) writes: HashMap<Symbol, std::collections::HashSet<Symbol>>,
    /// Main's root statements.
    root: Vec<&'s Stmt<'a>>,
    /// Each function's body and parameters.
    functions: HashMap<Symbol, (&'s [Stmt<'a>], Vec<Symbol>)>,
}

/// The program's bindings a function reaches: every free name of a `## To` body (one it reads or sets but neither
/// takes nor declares, `optimize::capture::free_names`) that `## Main` binds with a `Let` at its root, each a
/// file-scope static in C; and what each function reaches and writes of them through the functions it calls.
pub(super) fn static_reach<'s, 'a>(stmts: &'s [Stmt<'a>]) -> StaticReach<'s, 'a> {
    let mut root = Vec::new();
    root_statements(stmts, &mut root);
    let mut root_lets: Vec<Symbol> = Vec::new();
    for s in &root {
        if let Stmt::Let { var, .. } = s {
            if !root_lets.contains(var) {
                root_lets.push(*var);
            }
        }
    }
    let functions: HashMap<Symbol, (&[Stmt], Vec<Symbol>)> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef {
                name,
                params,
                body,
                is_native: false,
                ..
            } => Some((*name, (&body[..], params.iter().map(|(p, _)| *p).collect()))),
            _ => None,
        })
        .collect();
    let mut direct: HashMap<Symbol, std::collections::HashSet<Symbol>> = HashMap::new();
    let mut direct_writes: HashMap<Symbol, std::collections::HashSet<Symbol>> = HashMap::new();
    let mut callees: HashMap<Symbol, std::collections::HashSet<Symbol>> = HashMap::new();
    for (f, (body, params)) in &functions {
        let (_, calls) = names_and_calls(body);
        let free = crate::optimize::capture::free_names(params, body);
        direct.insert(
            *f,
            free.into_iter().filter(|n| root_lets.contains(n)).collect(),
        );
        let writes = crate::optimize::capture::free_writes(params, body);
        direct_writes.insert(
            *f,
            writes
                .into_iter()
                .filter(|n| root_lets.contains(n))
                .collect(),
        );
        callees.insert(
            *f,
            calls
                .into_iter()
                .filter(|c| functions.contains_key(c))
                .collect(),
        );
    }
    let names: Vec<Symbol> = root_lets
        .iter()
        .copied()
        .filter(|g| direct.values().any(|reached| reached.contains(g)))
        .collect();
    // What each function reaches, and what it writes, through the functions it calls.
    let through_calls = |mut sets: HashMap<Symbol, std::collections::HashSet<Symbol>>| loop {
        let mut grew = false;
        for (f, cs) in &callees {
            let via: Vec<Symbol> = cs.iter().flat_map(|c| sets[c].iter().copied()).collect();
            let mine = sets.get_mut(f).expect("every function has a set");
            for g in via {
                grew |= mine.insert(g);
            }
        }
        if !grew {
            return sets;
        }
    };
    let reach = through_calls(direct);
    let writes = through_calls(direct_writes);
    StaticReach {
        names,
        reach,
        writes,
        root,
        functions,
    }
}

/// The program's bindings a function reaches ([`static_reach`]), and what the C tier refuses about them: since the
/// static would answer where the interpreter fails, a call from Main's root that reaches a binding before its `Let`
/// has run, and a task launched on a function that reaches one (a task sees none of the program's bindings); and an
/// expression whose operands the C tier cannot put in order ([`unsequenced_static_reads`]).
fn program_statics(stmts: &[Stmt], interner: &Interner) -> ProgramStatics {
    let StaticReach {
        names,
        reach,
        writes,
        root,
        functions,
    } = static_reach(stmts);
    if names.is_empty() {
        return ProgramStatics {
            names,
            refusals: Vec::new(),
        };
    }
    let first_reached = |f: &Symbol| -> Option<Symbol> {
        names
            .iter()
            .copied()
            .find(|g| reach.get(f).is_some_and(|r| r.contains(g)))
    };
    let mut refusals = Vec::new();
    for_each_node(stmts, |node| {
        if let Visited::Stmt(
            Stmt::LaunchTask { function, .. } | Stmt::LaunchTaskWithHandle { function, .. },
        ) = node
        {
            if let Some(g) = first_reached(function) {
                refusals.push(format!(
                    "a task launched on `{}`, which reaches the program's `{}` (a task sees none of the program's bindings)",
                    interner.resolve(*function),
                    interner.resolve(g)
                ));
            }
        }
    });
    let mut bound: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
    for s in &root {
        let (_, calls) = names_and_calls(std::slice::from_ref(*s));
        for c in calls.iter().filter(|c| functions.contains_key(c)) {
            if let Some(g) = names
                .iter()
                .copied()
                .find(|g| reach[c].contains(g) && !bound.contains(g))
            {
                refusals.push(format!(
                    "a call to `{}` that reaches the program's `{}` before its Let runs — declare it before the first call",
                    interner.resolve(*c),
                    interner.resolve(g)
                ));
            }
        }
        if let Stmt::Let { var, .. } = s {
            bound.insert(*var);
        }
    }
    refusals.extend(unsequenced_static_reads(
        stmts, &names, &reach, &writes, interner,
    ));
    ProgramStatics { names, refusals }
}

/// C leaves unspecified the order it evaluates an operator's operands and a call's arguments in; LOGOS
/// evaluates them left to right. Once a function can set a program static, the order shows: in
/// `counter + next()`, where `next` sets `counter`, the interpreter reads 0 and C may read 1. Such an
/// expression — one operand writing a static a sibling operand reads (directly, or through a call) — is
/// refused by name, never compiled in an order the program did not state.
fn unsequenced_static_reads(
    stmts: &[Stmt],
    statics: &[Symbol],
    reach: &HashMap<Symbol, std::collections::HashSet<Symbol>>,
    writes: &HashMap<Symbol, std::collections::HashSet<Symbol>>,
    interner: &Interner,
) -> Vec<String> {
    use logicaffeine_language::ast_depth::{expr_children, stmt_expr_children};
    type Set = std::collections::HashSet<Symbol>;
    // The statics an expression reads and writes, calls included; a closure's body runs later.
    fn effects(
        e: &Expr,
        statics: &[Symbol],
        reach: &HashMap<Symbol, Set>,
        writes: &HashMap<Symbol, Set>,
    ) -> (Set, Set) {
        let (mut r, mut w) = (Set::new(), Set::new());
        match e {
            Expr::Identifier(s) if statics.contains(s) => {
                r.insert(*s);
            }
            Expr::Call { function, .. } => {
                r.extend(reach.get(function).into_iter().flatten().copied());
                w.extend(writes.get(function).into_iter().flatten().copied());
            }
            _ => {}
        }
        for child in expr_children(e) {
            let (cr, cw) = effects(child, statics, reach, writes);
            r.extend(cr);
            w.extend(cw);
        }
        (r, w)
    }
    let conflict = |children: &[&Expr]| -> Option<Symbol> {
        let fx: Vec<(Set, Set)> = children
            .iter()
            .map(|c| effects(c, statics, reach, writes))
            .collect();
        for (i, (_, wi)) in fx.iter().enumerate() {
            for (j, (rj, _)) in fx.iter().enumerate() {
                if i != j {
                    if let Some(s) = statics.iter().find(|s| wi.contains(s) && rj.contains(s)) {
                        return Some(*s);
                    }
                }
            }
        }
        None
    };
    let mut refusals = Vec::new();
    for_each_node(stmts, |node| {
        let children = match node {
            Visited::Expr(e) => expr_children(e),
            Visited::Stmt(s) => stmt_expr_children(s),
        };
        if children.len() > 1 {
            if let Some(s) = conflict(&children) {
                refusals.push(format!(
                    "an expression that reads the program's `{}` beside a call that sets it, in an order C does not fix — bind the call's result first",
                    interner.resolve(s)
                ));
            }
        }
    });
    refusals.sort();
    refusals.dedup();
    refusals
}

/// `## Main` lowered: under the stepped tier, its frame and step beside the C `main` that boots the
/// scheduler on it; otherwise the C `main` alone.
struct MainCode {
    frame: String,
    step: String,
    entry: String,
}

/// Lower `## Main`: as the first task under the scheduler when the program is concurrent — a step
/// function under the stepped tier, a coroutine otherwise — else as `main`.
fn emit_main(
    stmts: &[Stmt],
    ctx: &mut CContext,
    uses_concurrency: bool,
    tier: TaskTier,
) -> MainCode {
    ctx.nonnegative_vars = crate::codegen::expr::collect_always_nonneg(stmts);

    let mut changed = values::mutated_roots(stmts, &ctx.fn_inout);
    changed.extend(ctx.changed_statics.iter().copied());
    ctx.mutated = changed;
    if uses_concurrency && tier == TaskTier::Stepped {
        ctx.bigint_vars = ctx.exact.main_locals.clone();
        ctx.number_locals = numbers::number_locals(stmts, ctx);
        let body: Vec<Stmt> = stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
            .cloned()
            .collect();
        ctx.main_root_depth = Some(ctx.decl_scopes.len() + 1);
        let (frame, step) = ctx.in_body(true, |ctx| {
            stepped::emit_step(MAIN_STEP, &[], CType::Void, &body, ctx)
        });
        ctx.main_root_depth = None;
        let mut entry = String::new();
        writeln!(entry, "int main(int argc, char **argv) {{").unwrap();
        writeln!(entry, "    _logos_argc = argc;").unwrap();
        writeln!(entry, "    _logos_argv = argv;").unwrap();
        writeln!(entry, "    LOGOS_STEP_HOOK();").unwrap();
        writeln!(
            entry,
            "    logos_sched_bootstrap(logos_step_{MAIN_STEP}, sizeof(logos_frame_{MAIN_STEP}), LOGOS_RUN_SEED);"
        )
        .unwrap();
        writeln!(entry, "    return 0;").unwrap();
        writeln!(entry, "}}").unwrap();
        return MainCode { frame, step, entry };
    }
    let mut out = String::new();
    // Main's own locals get the same exact-integer pre-pass a function body receives (settled with the
    // program's slots, see `exact`), so a mutable accumulator declared in Main that overflows i64 — or is
    // reassigned an exact `**`/`pow`, or a call's exact result — is declared `LInt` from the start (a `Set`
    // cannot retype a name mid-block). Float-guarded so a `double` accumulator in Main is never mistaken for
    // an arbitrary-precision integer.
    ctx.bigint_vars = ctx.exact.main_locals.clone();
    ctx.number_locals = numbers::number_locals(stmts, ctx);
    let branchless_words =
        direct_c_branchless_word_vars(stmts, &ctx.bigint_vars, &ctx.exact.pure_calls);
    ctx.bigint_vars
        .retain(|sym| !branchless_words.contains(sym));

    if uses_concurrency {
        // Main runs AS THE FIRST TASK: its body becomes `logos_main_body` (a `long long`-returning
        // trampoline-compatible entry), and `main` boots the scheduler with it, draining to quiescence.
        // Program args are stashed before the scheduler starts (a `Show args` inside a task still works).
        writeln!(
            out,
            "static long long logos_main_body(const long long *__targs) {{"
        )
        .unwrap();
        writeln!(out, "    (void)__targs;").unwrap();
        ctx.enter_scope();
        ctx.main_root_depth = Some(ctx.decl_scopes.len());
        let ownership = body_ownership(stmts, &std::collections::HashSet::new(), ctx);
        ctx.in_body(true, |ctx| {
            ctx.ownership = Some(ownership);
            for stmt in stmts {
                match stmt {
                    Stmt::FunctionDef { .. } => continue,
                    _ => codegen_stmt(stmt, ctx, &mut out, 1),
                }
            }
        });
        ctx.main_root_depth = None;
        ctx.exit_scope();
        writeln!(out, "    return 0;").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out, "int main(int argc, char **argv) {{").unwrap();
        writeln!(out, "    _logos_argc = argc;").unwrap();
        writeln!(out, "    _logos_argv = argv;").unwrap();
        writeln!(out, "    logos_sched_bootstrap((void *)logos_main_body);").unwrap();
        writeln!(out, "    return 0;").unwrap();
        writeln!(out, "}}").unwrap();
    } else {
        writeln!(out, "int main(int argc, char **argv) {{").unwrap();
        writeln!(out, "    _logos_argc = argc;").unwrap();
        writeln!(out, "    _logos_argv = argv;").unwrap();
        ctx.enter_scope();
        ctx.main_root_depth = Some(ctx.decl_scopes.len());
        let ownership = body_ownership(stmts, &std::collections::HashSet::new(), ctx);
        ctx.in_body(true, |ctx| {
            ctx.ownership = Some(ownership);
            for stmt in stmts {
                match stmt {
                    Stmt::FunctionDef { .. } => continue,
                    _ => codegen_stmt(stmt, ctx, &mut out, 1),
                }
            }
        });
        ctx.main_root_depth = None;
        ctx.exit_scope();
        writeln!(out, "    return 0;").unwrap();
        writeln!(out, "}}").unwrap();
    }
    MainCode {
        frame: String::new(),
        step: String::new(),
        entry: out,
    }
}

/// The C name of `## Main`'s step and frame under the stepped tier — one no LOGOS name spells.
const MAIN_STEP: &str = "__main";

/// Collect, per spawned task function, its argument count (`Launch a task to fn with a, b` → arity 2).
/// A function may be spawned at several sites; the LOGOS type system fixes its arity, so the max seen is
/// its true arity. Drives the task-entry wrappers: each spawned function gets a `long long`-returning
/// trampoline-compatible entry, so the `ucontext` trampoline can call it uniformly by pointer.
fn collect_spawned_arities(stmts: &[Stmt], out: &mut HashMap<Symbol, usize>) {
    for_each_node(stmts, |node| {
        if let Visited::Stmt(
            Stmt::LaunchTask { function, args } | Stmt::LaunchTaskWithHandle { function, args, .. },
        ) = node
        {
            let e = out.entry(*function).or_insert(0);
            *e = (*e).max(args.len());
        }
    });
}
use types::{
    c_param_decl, c_type_str_resolved, escape_c_ident, field_type_to_ctype,
    resolve_type_expr_with_registry, tuple_c_name, CContext, CType,
};

/// Collect every distinct tuple/record `CType` reachable from a function signature (nested-first, deduped
/// by mangled name) so its `typedef struct { … }` can be emitted before any use.
fn collect_tuple_types(
    cty: &CType,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<(String, Vec<CType>)>,
    interner: &Interner,
) {
    // A named record SHARES the positional tuple struct, and a Seq-of-tuple's element IS a tuple — so all
    // three surface the same underlying element tuple.
    let elems: Option<Vec<CType>> = match cty {
        CType::Tuple(elems) | CType::SeqTuple(elems) => Some(elems.clone()),
        CType::Record(fields) => Some(fields.iter().map(|(_, t)| t.clone()).collect()),
        _ => None,
    };
    if let Some(elems) = elems {
        for e in &elems {
            collect_tuple_types(e, seen, out, interner);
        }
        let name = tuple_c_name(&elems, interner);
        if seen.insert(name.clone()) {
            out.push((name, elems));
        }
    }
}

/// Collect every distinct NESTED-sequence `CType::SeqSeq` reachable from a signature (deepest-first,
/// deduped) so its outer `Seq_<inner>` wrapper typedef can be emitted before use.
fn collect_seqseq_types(
    cty: &CType,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<CType>,
    interner: &Interner,
) {
    if let CType::SeqSeq(inner) = cty {
        collect_seqseq_types(inner, seen, out, interner);
        let name = c_type_str_resolved(cty, interner);
        if seen.insert(name) {
            out.push(cty.clone());
        }
    }
}

/// Collect nested-sequence (`CType::SeqSeq`) types reachable from STATEMENT expressions — `Let` values,
/// `Set` values, `Show` objects, and `Return`s — not just function signatures, so a bare `Show [[1,2]]`
/// or `Let g be [[1,2]]` gets its `Seq_Seq_…` family emitted. A forward pass registers `Let`/param types
/// via `infer_expr_type` so a nested-seq bound to a variable and shown later still resolves.
fn collect_stmt_seqseq_types<'a>(
    body: &'a [Stmt<'a>],
    ctx: &mut CContext<'a>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<CType>,
    interner: &Interner,
) {
    for s in body {
        match s {
            Stmt::Let { var, value, .. } => {
                let ty = absent::binding(s, crate::codegen_c::types::infer_expr_type(value, ctx), ctx);
                collect_seqseq_types(&ty, seen, out, interner);
                ctx.vars.insert(*var, ty);
            }
            Stmt::Set { value, .. } => {
                let ty = crate::codegen_c::types::infer_expr_type(value, ctx);
                collect_seqseq_types(&ty, seen, out, interner);
            }
            Stmt::Show { object, .. } => {
                let ty = crate::codegen_c::types::infer_expr_type(object, ctx);
                collect_seqseq_types(&ty, seen, out, interner);
            }
            Stmt::Return { value: Some(v) } => {
                let ty = crate::codegen_c::types::infer_expr_type(v, ctx);
                collect_seqseq_types(&ty, seen, out, interner);
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_stmt_seqseq_types(then_block, ctx, seen, out, interner);
                if let Some(eb) = else_block {
                    collect_stmt_seqseq_types(eb, ctx, seen, out, interner);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_stmt_seqseq_types(body, ctx, seen, out, interner);
            }
            Stmt::FunctionDef {
                name, body, params, ..
            } => {
                let saved = ctx.vars.clone();
                let saved_scope = std::mem::replace(&mut ctx.tower_scope, Some(*name));
                for (pn, pt) in params.iter() {
                    ctx.vars.insert(*pn, collected_param_ctype(ctx, *name, *pn, pt));
                }
                collect_stmt_seqseq_types(body, ctx, seen, out, interner);
                ctx.vars = saved;
                ctx.tower_scope = saved_scope;
            }
            _ => {}
        }
    }
}
/// The C type a statement collector gives parameter `param` of `function`: a value of the exact extension where the
/// extension reaches it, else its declared type.
fn collected_param_ctype(ctx: &CContext, function: Symbol, param: Symbol, declared: &TypeExpr) -> CType {
    if tower::param(ctx, function, param) {
        return CType::Tower;
    }
    let declared = resolve_type_expr_with_registry(declared, ctx.interner, Some(ctx.registry));
    absent::parameter(function, param, declared, ctx)
}

/// Collect tuple/record (`CType::Tuple`/`Record`/`SeqTuple`) types reached from STATEMENT expressions —
/// `Let`/`Set` values, `Show` objects, `Return`s — not just signatures, so a bare `Show (1, 2)` /
/// `Let t be (1, 2)` gets its `LTuple_…` family. Mirrors [`collect_stmt_seqseq_types`].
fn collect_stmt_tuple_types<'a>(
    body: &'a [Stmt<'a>],
    ctx: &mut CContext<'a>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<(String, Vec<CType>)>,
    interner: &Interner,
) {
    for s in body {
        match s {
            Stmt::Let { var, value, .. } => {
                let ty = absent::binding(s, crate::codegen_c::types::infer_expr_type(value, ctx), ctx);
                collect_tuple_types(&ty, seen, out, interner);
                ctx.vars.insert(*var, ty);
            }
            Stmt::Set { value, .. } => {
                collect_tuple_types(
                    &crate::codegen_c::types::infer_expr_type(value, ctx),
                    seen,
                    out,
                    interner,
                );
            }
            Stmt::Show { object, .. } => {
                collect_tuple_types(
                    &crate::codegen_c::types::infer_expr_type(object, ctx),
                    seen,
                    out,
                    interner,
                );
            }
            Stmt::Return { value: Some(v) } => {
                collect_tuple_types(
                    &crate::codegen_c::types::infer_expr_type(v, ctx),
                    seen,
                    out,
                    interner,
                );
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_stmt_tuple_types(then_block, ctx, seen, out, interner);
                if let Some(eb) = else_block {
                    collect_stmt_tuple_types(eb, ctx, seen, out, interner);
                }
            }
            // One name over a map binds each entry, `(key, value)`.
            Stmt::Repeat {
                pattern: crate::ast::stmt::Pattern::Identifier(var),
                iterable,
                body,
            } => {
                let over = crate::codegen_c::types::infer_expr_type(iterable, ctx);
                if let Some((key, value)) = crate::codegen_c::types::map_entry_types(&over) {
                    let entry = CType::Tuple(vec![key, value]);
                    collect_tuple_types(&entry, seen, out, interner);
                    ctx.vars.insert(*var, entry);
                }
                collect_stmt_tuple_types(body, ctx, seen, out, interner);
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_stmt_tuple_types(body, ctx, seen, out, interner);
            }
            Stmt::FunctionDef {
                name, body, params, ..
            } => {
                let saved = ctx.vars.clone();
                let saved_scope = std::mem::replace(&mut ctx.tower_scope, Some(*name));
                for (pn, pt) in params.iter() {
                    ctx.vars.insert(*pn, collected_param_ctype(ctx, *name, *pn, pt));
                }
                collect_stmt_tuple_types(body, ctx, seen, out, interner);
                ctx.vars = saved;
                ctx.tower_scope = saved_scope;
            }
            _ => {}
        }
    }
}

/// Record a map whose VALUE no fixed family holds — a Seq (`Map of Text to Seq of Int` → `Map_str_Seq_i64`), a value
/// of the exact extension (`Map of Int to Hyperreal` → `Map_i64_LTower`) — so its generic family can be synthesized.
/// Value-TYPE maps (`Map of Text to Money`) keep their own fixed family, so those are skipped here (avoids a duplicate
/// typedef).
fn collect_mapvalue_from_ctype(
    cty: &CType,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<CType>,
    interner: &Interner,
) {
    if let CType::MapStrValue(v) | CType::MapI64Value(v) = cty {
        if !crate::codegen_c::types::is_c_value_type(v) {
            let name = c_type_str_resolved(cty, interner);
            if seen.insert(name) {
                out.push(cty.clone());
            }
        }
    }
}

/// Collect seq-valued string-map types reached from STATEMENT expressions (a bare
/// `Let m be a new Map of Text to Seq of Int`), not just signatures. Mirrors [`collect_stmt_seqseq_types`].
fn collect_stmt_mapvalue_types<'a>(
    body: &'a [Stmt<'a>],
    ctx: &mut CContext<'a>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<CType>,
    interner: &Interner,
) {
    for s in body {
        match s {
            Stmt::Let { var, value, .. } => {
                let ty = crate::codegen_c::types::infer_expr_type(value, ctx);
                collect_mapvalue_from_ctype(&ty, seen, out, interner);
                ctx.vars.insert(*var, ty);
            }
            Stmt::Set { value, .. } => {
                let ty = crate::codegen_c::types::infer_expr_type(value, ctx);
                collect_mapvalue_from_ctype(&ty, seen, out, interner);
            }
            Stmt::Show { object, .. } => {
                let ty = crate::codegen_c::types::infer_expr_type(object, ctx);
                collect_mapvalue_from_ctype(&ty, seen, out, interner);
            }
            Stmt::Return { value: Some(v) } => {
                let ty = crate::codegen_c::types::infer_expr_type(v, ctx);
                collect_mapvalue_from_ctype(&ty, seen, out, interner);
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_stmt_mapvalue_types(then_block, ctx, seen, out, interner);
                if let Some(eb) = else_block {
                    collect_stmt_mapvalue_types(eb, ctx, seen, out, interner);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_stmt_mapvalue_types(body, ctx, seen, out, interner);
            }
            Stmt::FunctionDef {
                name, body, params, ..
            } => {
                let saved = ctx.vars.clone();
                let saved_scope = std::mem::replace(&mut ctx.tower_scope, Some(*name));
                for (pn, pt) in params.iter() {
                    ctx.vars.insert(*pn, collected_param_ctype(ctx, *name, *pn, pt));
                    collect_mapvalue_from_ctype(
                        &resolve_type_expr_with_registry(pt, interner, Some(ctx.registry)),
                        seen,
                        out,
                        interner,
                    );
                }
                collect_stmt_mapvalue_types(body, ctx, seen, out, interner);
                ctx.vars = saved;
                ctx.tower_scope = saved_scope;
            }
            _ => {}
        }
    }
}

/// Emit the insertion-ordered generic map family of the map C type `map` — `Map_str_<V>` for a `MapStrValue`,
/// `Map_i64_<V>` for a `MapI64Value` — for an arbitrary showable VALUE `CType` (Seq values and the exact extension's;
/// value types use their own fixed family). Mirrors the fixed families, but renders the value via
/// [`c_field_to_string_expr`] so any showable value works, and a read of a key the map does not hold fails as theirs
/// do. The value's own C type must already be defined (it is — scalar seqs live in `C_RUNTIME`, value/struct/nested
/// seqs are emitted earlier, `LTower` with the extension's runtime). `rendered` false emits no renderer: a map of
/// values-or-nothing is one family whatever they hold, and what renders them is the payload's (`c_field_to_string_expr`).
fn emit_generic_map_family(output: &mut String, map: &CType, interner: &Interner, rendered: bool) {
    use std::fmt::Write;
    let (v, k, key_t, key_param, hash, store_key, key_fmt) = match map {
        CType::MapStrValue(v) => (v, "str", "char *", "const char *key", "_map_hash_str", "lc_gc_strdup(key)", "%s"),
        CType::MapI64Value(v) => (v, "i64", "int64_t ", "int64_t key", "_map_hash", "key", "%\" PRId64 \""),
        _ => return,
    };
    let eq = |slot: &str| match k {
        "str" => format!("strcmp({slot}, key) == 0"),
        _ => format!("{slot} == key"),
    };
    let eq_idx = eq("m->keys[idx]");
    let ne_i = match k {
        "str" => "strcmp(t->keys[i], key) != 0".to_string(),
        _ => "t->keys[i] != key".to_string(),
    };
    let vname = c_type_str_resolved(v, interner); // e.g. "Seq_i64"
    let mp = format!("Map_{k}_{vname}");
    let f = format!("map_{k}_{vname}");
    let vrender = crate::codegen_c::types::c_field_to_string_expr(v, "m->vals[si]", interner);
    writeln!(output, "typedef struct {{ {key_t}*keys; {vname} *vals; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len; }} {mp};").unwrap();
    writeln!(output, "__attribute__((unused)) static {mp} {f}_new(void) {{ size_t cap = 16; {mp} m = {{({key_t}*)lc_gc_calloc(cap, sizeof({key_t})), ({vname} *)lc_gc_calloc(cap, sizeof({vname})), (uint8_t *)lc_gc_calloc(cap, 1), cap, 0, (size_t *)lc_gc_calloc(cap, sizeof(size_t)), 0}}; return m; }}").unwrap();
    writeln!(output, "__attribute__((unused)) static void _{f}_resize({mp} *m) {{ {key_t}*ok = m->keys; {vname} *ov = m->vals; uint8_t *os = m->state; size_t *oo = m->order; size_t ool = m->ord_len; size_t nc = m->cap * 2; m->keys = ({key_t}*)lc_gc_calloc(nc, sizeof({key_t})); m->vals = ({vname} *)lc_gc_calloc(nc, sizeof({vname})); m->state = (uint8_t *)lc_gc_calloc(nc, 1); m->order = (size_t *)lc_gc_calloc(nc, sizeof(size_t)); m->cap = nc; m->len = 0; m->ord_len = 0; for (size_t oi = 0; oi < ool; oi++) {{ size_t si = oo[oi]; if (!os[si]) continue; size_t idx = {hash}(ok[si], nc); while (m->state[idx]) idx = (idx + 1) & (nc - 1); m->keys[idx] = ok[si]; m->vals[idx] = ov[si]; m->state[idx] = 1; m->len++; m->order[m->ord_len++] = idx; }} lc_gc_free(ok); lc_gc_free(ov); lc_gc_free(os); lc_gc_free(oo); }}").unwrap();
    writeln!(output, "__attribute__((unused)) static void {f}_set({mp} *m, {key_param}, {vname} val) {{ if (m->ord_len * 4 >= m->cap * 3) _{f}_resize(m); size_t idx = {hash}(key, m->cap); while (m->state[idx]) {{ if ({eq_idx}) {{ m->vals[idx] = val; return; }} idx = (idx + 1) & (m->cap - 1); }} m->keys[idx] = {store_key}; m->vals[idx] = val; m->state[idx] = 1; m->len++; m->order[m->ord_len++] = idx; }}").unwrap();
    writeln!(output, "__attribute__((unused)) static {vname} {f}_get({mp} *m, {key_param}) {{ size_t idx = {hash}(key, m->cap); while (m->state[idx]) {{ if ({eq_idx}) return m->vals[idx]; idx = (idx + 1) & (m->cap - 1); }} logos_map_miss(); {vname} z; memset(&z, 0, sizeof({vname})); return z; }}").unwrap();
    writeln!(output, "__attribute__((unused)) static bool {f}_contains({mp} *m, {key_param}) {{ size_t idx = {hash}(key, m->cap); while (m->state[idx]) {{ if ({eq_idx}) return true; idx = (idx + 1) & (m->cap - 1); }} return false; }}").unwrap();
    writeln!(output, "__attribute__((unused)) static void {f}_remove({mp} *t, {key_param}) {{ size_t mask = t->cap - 1, i = {hash}(key, t->cap); while (t->state[i] && {ne_i}) i = (i + 1) & mask; if (!t->state[i]) return; _logos_order_drop(t->order, &t->ord_len, i); t->len--; for (size_t j = (i + 1) & mask; t->state[j]; j = (j + 1) & mask) {{ if (_logos_slot_backs({hash}(t->keys[j], t->cap), i, j, mask)) {{ t->keys[i] = t->keys[j]; t->vals[i] = t->vals[j]; _logos_order_move(t->order, t->ord_len, j, i); i = j; }} }} t->state[i] = 0; }}").unwrap();
    writeln!(output, "__attribute__((unused)) static int64_t {f}_len({mp} *m) {{ return (int64_t)m->len; }}").unwrap();
    if !rendered {
        return;
    }
    writeln!(output, "__attribute__((unused)) static char *{f}_to_string({mp} *m) {{ char *out = lc_gc_strdup(\"{{\"); int first = 1; for (size_t oi = 0; oi < m->ord_len; oi++) {{ size_t si = m->order[oi]; if (!m->state[si]) continue; char *__v = {vrender}; char *e = logos_dyn_sprintf(\"%s{key_fmt}: %s\", first ? \"\" : \", \", m->keys[si], __v); lc_gc_free(__v); char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0; }} char *nw = str_concat(out, \"}}\"); lc_gc_free(out); return nw; }}").unwrap();
    writeln!(output, "__attribute__((unused)) static void show_{f}({mp} *m) {{ char *t = {f}_to_string(m); printf(\"%s\\n\", t); lc_gc_free(t); }}").unwrap();
}
use emit::codegen_stmt;

/// Promote an integer `CType` to the EXACT arbitrary-precision `LInt` (`CType::BigInt`) in place, recursing into
/// a function-pointer's return + params. Non-integer types are untouched.
fn exactify_int_ctype(t: &mut CType) {
    match t {
        CType::Int64 => *t = CType::BigInt,
        CType::FnPtr { ret, params } => {
            exactify_int_ctype(ret.as_mut());
            for p in params.iter_mut() {
                exactify_int_ctype(p);
            }
        }
        _ => {}
    }
}

/// The SHAPE half of an EXACT higher-order function: takes a `fn`-typed (closure) param AND no sequence param
/// (a SCALAR HOF like `apply(f, x)`). Its integers become exact `LInt` so a closure that overflows promotes
/// instead of wrapping — SELECTIVELY, excluding seq-HOFs (exactifying them breaks passing an i64 seq element to
/// an `LInt` closure). The CALL-SITE half (inline closures at every site, sites not provably in range) is the
/// shared `codegen::bigint_promote::exact_hof_fns`; both must hold.
fn is_exact_hof(param_ctypes: &[CType]) -> bool {
    // A higher-order function OVER a seq (`map`/`fold`/`reduceSeq`) is never exactified: its closure receives
    // raw (i64/typed) seq elements.
    param_ctypes
        .iter()
        .any(|t| matches!(t, CType::FnPtr { .. }))
        && !param_ctypes.iter().any(types::is_seq_ctype)
}

/// The C type of `param` of `function`: its declared type, every integer of an exact HOF's exactified, and
/// `LInt` where the exact-slot analysis found an exact value passed in its position.
fn param_ctype(ctx: &CContext, function: Symbol, param: Symbol, declared: &TypeExpr) -> CType {
    if tower::param(ctx, function, param) {
        return CType::Tower;
    }
    let mut t = resolve_type_expr_with_registry(declared, ctx.interner, Some(ctx.registry));
    if ctx.exact_hofs.contains(&function) {
        exactify_int_ctype(&mut t);
    } else if t == CType::Int64 && ctx.exact.param_is_exact(function, param) {
        t = CType::BigInt;
    }
    absent::parameter(function, param, t, ctx)
}

/// The `LInt` locals of `function`'s body: the exact-slot analysis's, for a function it read (every
/// top-level one); otherwise the body's own pre-pass under the program's slots.
fn body_exact_locals(
    ctx: &CContext,
    function: Symbol,
    body: &[Stmt],
    param_types: &[(Symbol, CType)],
) -> std::collections::HashSet<Symbol> {
    if let Some(locals) = ctx.exact.locals.get(&function) {
        return locals.clone();
    }
    let mut no_int = types::collect_float_vars(body, param_types, ctx.interner, ctx.registry);
    no_int.extend(crate::analysis::exactness::word_vars(
        body,
        &[],
        &ctx.exact.word_results,
        ctx.registry,
        ctx.interner,
    ));
    let seeds = param_types
        .iter()
        .filter(|(_, t)| *t == CType::BigInt)
        .map(|(p, _)| *p)
        .collect();
    types::collect_bigint_vars(
        body,
        &no_int,
        ctx.interner,
        &ctx.exact,
        ctx.int_oracle.as_deref(),
        &seeds,
    )
}

/// The sharing and liveness of `body` (see `sharing`), under the program's changed statics and its functions'
/// changed parameters, a program static being read at any time; and the ranges of its integer names (see
/// `word_ranges`).
/// Fresh local buffers the direct-C function owns alone and can destroy before it leaves.
fn owned_drop_plan(
    body: &[Stmt],
    params: &std::collections::HashSet<Symbol>,
    sharing: &sharing::Sharing,
    ctx: &CContext,
) -> std::collections::HashSet<Symbol> {
    use std::collections::{HashMap, HashSet};

    fn expr_reads(expr: &Expr, out: &mut HashSet<Symbol>) {
        if let Expr::Identifier(sym) = expr {
            out.insert(*sym);
        }
        for child in expr_children(expr) {
            expr_reads(child, out);
        }
    }

    let mut bindings: HashMap<Symbol, (usize, bool)> = HashMap::new();
    let mut return_reads = HashSet::new();
    let mut has_nested_scope = false;
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
            // A returned closure may capture a local buffer even when the
            // sharing graph does not represent that environment edge. Keep
            // the lifetime optimization conservative for closure-bearing
            // functions until capture ownership is explicit in this backend.
            has_nested_scope = true;
            Walk::Past
        }
        Visited::Stmt(Stmt::Let { var, value, .. }) => {
            let entry = bindings.entry(*var).or_insert((0, true));
            entry.0 += 1;
            entry.1 &= values::is_fresh(value);
            Walk::Into
        }
        Visited::Stmt(Stmt::Return { value: Some(value) }) => {
            // Cleanup precedes C's return expression, so exclude every value needed to evaluate it,
            // not merely buffers the returned result may alias.
            expr_reads(value, &mut return_reads);
            Walk::Into
        }
        _ => Walk::Into,
    });

    if has_nested_scope {
        return HashSet::new();
    }

    let mut escaping = params.clone();
    escaping.extend(ctx.program_statics.iter().copied());
    escaping.extend(return_reads);
    let escaping = sharing.closure(&escaping);

    bindings
        .into_iter()
        .filter_map(|(sym, (count, fresh))| {
            (count == 1 && fresh && !escaping.contains(&sym) && sharing.class_of(sym) == vec![sym])
                .then_some(sym)
        })
        .collect()
}

fn body_ownership(
    body: &[Stmt],
    params: &std::collections::HashSet<Symbol>,
    ctx: &CContext,
) -> std::rc::Rc<types::Ownership> {
    let sharing = sharing::Sharing::of(
        body,
        &ctx.changed_statics,
        &ctx.mutating,
        &ctx.call_shapes,
        &ctx.fn_inout,
    );
    let owned_drops = owned_drop_plan(body, params, &sharing, ctx);
    std::rc::Rc::new(types::Ownership {
        sharing,
        live: sharing::Liveness::of(body, &ctx.program_statics),
        words: word_ranges::WordRanges::of(body, &ctx.program_statics),
        owned_drops,
        bounded_count_accumulators:
            crate::codegen::stmt::collect_bounded_count_accumulators_with_purity(
                body,
                &ctx.bigint_vars,
                &ctx.exact.pure_calls,
            ),
    })
}

/// Exact integer locals for which direct C can use its native word representation while
/// emitting `BranchlessAccumulate`. The shared bounded-counter proof establishes that the value
/// starts at zero, is changed only by `+1`, and cannot exceed an `i64` loop bound.
///
/// Keep the syntactic predicate-guard check here in lockstep with `emit::codegen_stmt`: merely
/// proving that an accumulator is bounded must not silently change its representation under an
/// optimization that never fires. When the pass is disabled this returns no variables, restoring
/// the ordinary exact `LInt` path.
fn direct_c_branchless_word_vars(
    body: &[Stmt],
    bigint_vars: &std::collections::HashSet<Symbol>,
    pure_calls: &std::collections::HashSet<Symbol>,
) -> std::collections::HashSet<Symbol> {
    use crate::optimization::Opt;
    use std::collections::HashSet;

    if !crate::optimize::active_config().is_on(Opt::BranchlessAccumulate) {
        return HashSet::new();
    }

    let bounded = crate::codegen::stmt::collect_bounded_count_accumulators_with_purity(
        body,
        bigint_vars,
        pure_calls,
    );
    if bounded.is_empty() {
        return HashSet::new();
    }

    let mut guarded = HashSet::new();
    walk_nodes(body, |node| match node {
        // A nested definition receives its own exactness and lowering context.
        Visited::Stmt(Stmt::FunctionDef { .. }) => Walk::Past,
        Visited::Stmt(Stmt::If {
            then_block,
            else_block: None,
            ..
        }) if then_block.len() == 1 => {
            if let Stmt::Set {
                target,
                value:
                    Expr::BinaryOp {
                        op: BinaryOpKind::Add,
                        left,
                        right,
                    },
            } = &then_block[0]
            {
                let unit_self_add = matches!(left, Expr::Identifier(sym) if *sym == *target)
                    && matches!(right, Expr::Literal(Literal::Number(1)));
                if unit_self_add && bounded.contains(target) {
                    guarded.insert(*target);
                }
            }
            Walk::Into
        }
        _ => Walk::Into,
    });
    guarded
}

/// Read-only text_bytes bindings that can borrow their source text as a byte
/// view. This consumes the same conservative proof as Rust's TextBytesView:
/// the byte binding is used only by length/index/iteration reads, and its
/// source text is not used after the binding.
fn c_text_bytes_view_plan(
    stmts: &[Stmt],
    interner: &crate::intern::Interner,
) -> HashMap<usize, Symbol> {
    fn visit(block: &[Stmt], interner: &crate::intern::Interner, out: &mut HashMap<usize, Symbol>) {
        for (idx, stmt) in block.iter().enumerate() {
            if let Stmt::Let {
                var,
                value: Expr::Call { function, args },
                mutable: false,
                ..
            } = stmt
            {
                if interner.resolve(*function) == "text_bytes"
                    && args.len() == 1
                    && matches!(args[0], Expr::Identifier(source) if *source != *var)
                {
                    let Expr::Identifier(source) = args[0] else {
                        unreachable!()
                    };
                    let remaining = &block[idx + 1..];
                    let remaining_refs: Vec<&Stmt> = remaining.iter().collect();
                    if crate::codegen::peephole::text_bytes_view_safe(remaining, *var)
                        && !crate::codegen::detection::symbol_appears_in_stmts(
                            *source,
                            &remaining_refs,
                        )
                    {
                        out.insert(stmt as *const Stmt as usize, *source);
                    }
                }
            }

            match stmt {
                Stmt::While { body, .. }
                | Stmt::Repeat { body, .. }
                | Stmt::FunctionDef { body, .. }
                | Stmt::Zone { body, .. }
                | Stmt::Splice { body }
                | Stmt::Reversibly { body }
                | Stmt::During { body, .. }
                | Stmt::Concurrent { tasks: body }
                | Stmt::Parallel { tasks: body } => visit(body, interner, out),
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    visit(then_block, interner, out);
                    if let Some(otherwise) = else_block {
                        visit(otherwise, interner, out);
                    }
                }
                Stmt::Inspect { arms, .. } => {
                    for arm in arms {
                        visit(arm.body, interner, out);
                    }
                }
                _ => {}
            }
        }
    }

    if !crate::optimize::active_config().is_on(crate::optimization::Opt::TextBytesView) {
        return HashMap::new();
    }
    let mut out = HashMap::new();
    visit(stmts, interner, &mut out);
    out
}

/// Sequence parameters that the C sharing analysis proves read-only. Passing
/// a small stack-backed sequence descriptor by value is safe at these sites:
/// the callee can read the backing storage but cannot retain or mutate it.
fn c_readonly_seq_params(
    stmts: &[Stmt],
    ctx: &CContext,
) -> HashMap<Symbol, std::collections::HashSet<usize>> {
    let mut out = HashMap::new();
    for stmt in stmts {
        let Stmt::FunctionDef {
            name,
            params,
            is_native: false,
            ..
        } = stmt
        else {
            continue;
        };
        let changed = ctx.mutating.get(name);
        let readonly = params
            .iter()
            .enumerate()
            .filter_map(|(i, (_, ty))| {
                let cty = resolve_type_expr_with_registry(ty, ctx.interner, Some(ctx.registry));
                let is_seq = matches!(
                    cty,
                    CType::SeqI64(_) | CType::SeqBool | CType::SeqStr | CType::SeqF64
                );
                (is_seq && !changed.and_then(|v| v.get(i)).copied().unwrap_or(true)).then_some(i)
            })
            .collect::<std::collections::HashSet<_>>();
        if !readonly.is_empty() {
            out.insert(*name, readonly);
        }
    }
    out
}

/// Use the shared Rust/C/bytecode proof for stack-backed fixed sequences. The
/// owner symbol is part of the key so identically named locals in different
/// functions can never steal one another's capacity.
fn c_fixed_stack_plan(stmts: &[Stmt], ctx: &CContext) -> HashMap<(Option<Symbol>, Symbol), usize> {
    let borrow = c_readonly_seq_params(stmts, ctx);
    crate::codegen::affine_array::fixed_stack_seq_plan(stmts, &borrow, ctx.interner)
}

fn codegen_function(stmt: &Stmt, ctx: &mut CContext, output: &mut String) {
    if let Stmt::FunctionDef {
        name,
        params,
        body,
        return_type,
        is_native,
        ..
    } = stmt
    {
        if *is_native {
            return;
        }

        let func_name = escape_c_ident(ctx.interner.resolve(*name));

        // The signature the first pass registered (declared types, `LInt` where exact) and the `LInt` locals
        // the exact-slot analysis settled on for this body — the same computation that decided the slots,
        // so a local and the parameter or field it reaches cannot disagree.
        let ret_type = ctx.funcs.get(name).cloned().unwrap_or_else(|| {
            return_type.map_or(CType::Void, |rt| {
                resolve_type_expr_with_registry(rt, ctx.interner, Some(ctx.registry))
            })
        });
        let param_types: Vec<(crate::intern::Symbol, CType)> = params
            .iter()
            .map(|(pn, pt)| (*pn, param_ctype(ctx, *name, *pn, pt)))
            .collect();
        let mut bigint_vars = body_exact_locals(ctx, *name, body, &param_types);
        let branchless_words =
            direct_c_branchless_word_vars(body, &bigint_vars, &ctx.exact.pure_calls);
        bigint_vars.retain(|sym| !branchless_words.contains(sym));
        let inout: Vec<bool> = params.iter().map(|(_, pt)| types::is_inout(pt)).collect();
        let param_strs: Vec<String> = param_types
            .iter()
            .zip(&inout)
            .map(|((pn, t), &by_ref)| {
                if by_ref {
                    types::c_inout_param_decl(t, &ctx.resolve(*pn), ctx.interner)
                } else {
                    c_param_decl(t, &ctx.resolve(*pn), ctx.interner)
                }
            })
            .collect();

        write!(
            output,
            "{} {}({})",
            c_type_str_resolved(&ret_type, ctx.interner),
            func_name,
            param_strs.join(", ")
        )
        .unwrap();
        writeln!(output, " {{").unwrap();

        let saved_vars = ctx.vars.clone();
        let saved_scope = std::mem::replace(&mut ctx.tower_scope, Some(*name));
        let saved_big = std::mem::take(&mut ctx.bigint_vars);
        let saved_numbers = std::mem::take(&mut ctx.number_locals);
        let saved_returns = ctx.current_fn_returns.clone();
        let saved_nonnegative = std::mem::replace(
            &mut ctx.nonnegative_vars,
            crate::codegen::expr::collect_always_nonneg(body),
        );
        ctx.bigint_vars = bigint_vars;
        ctx.current_fn_returns = ret_type.clone();
        // A parameter shares the body's outermost C block: the body's binding of its name with another
        // type takes a variable of its own.
        ctx.enter_scope();
        let entry_vars = ctx.vars.clone();
        for (pn, pt) in &param_types {
            ctx.vars.insert(*pn, pt.clone());
        }
        ctx.number_locals = numbers::number_locals(body, ctx);
        ctx.vars = entry_vars;
        let mut changed = values::mutated_roots(body, &ctx.fn_inout);
        changed.extend(ctx.changed_statics.iter().copied());
        for ((pname, ptype), &by_ref) in param_types.iter().zip(&inout) {
            let cname = ctx.bind_in_place(*pname, ptype.clone());
            // A `mutable` parameter is the caller's variable: every read and write of its name goes through the
            // pointer, and its text stays in the representation the caller holds.
            if by_ref {
                ctx.renamed.insert(*pname, format!("(*{cname})"));
                continue;
            }
            // A text the body appends to is a builder of its own; any other parameter the body changes arrives
            // as the function's own, its callers having copied it where they read it again (see `sharing`).
            if changed.contains(pname)
                && *ptype == CType::String
                && !ctx.plain_texts.contains(ctx.interner.resolve(*pname))
            {
                writeln!(output, "    {cname} = logos_text_builder({cname}, 0);").unwrap();
            }
            // An exact value the body changes in place is a cell of its own; the caller's stays as it was.
            if changed.contains(pname) && *ptype == CType::Tower {
                writeln!(output, "    {cname} = ltower_own({cname});").unwrap();
            }
        }
        let saved_changed = std::mem::replace(&mut ctx.mutated, changed);
        let param_names = param_types.iter().map(|(name, _)| *name).collect();
        let ownership = body_ownership(body, &param_names, ctx);
        let heap_fast = crate::optimize::active_config()
            .is_on(crate::optimization::Opt::Peephole)
            .then(|| {
                crate::codegen::entry_guard::detect_heap_sift_guard(params, body, ctx.interner)
            })
            .flatten()
            .and_then(|guard| {
                let word_vars = crate::codegen::entry_guard::heap_sift_word_vars(
                    params,
                    body,
                    &guard,
                    ctx.interner,
                )?;
                let plain_exact_param = |sym| {
                    param_types
                        .iter()
                        .position(|(name, _)| *name == sym)
                        .is_some_and(|i| param_types[i].1 == CType::BigInt && !inout[i])
                };
                let seq_param = param_types
                    .iter()
                    .find(|(name, _)| *name == guard.arr)
                    .is_some_and(|(_, ty)| types::is_seq_ctype(ty));
                (plain_exact_param(guard.start) && plain_exact_param(guard.end) && seq_param)
                    .then_some((guard, word_vars))
            });

        if let Some((guard, word_vars)) = heap_fast {
            // Rust's entry-guard lowering keeps these indices as machine words.
            // Direct C historically kept the same values in `LInt`, putting
            // tagged compares/adds and repeated narrowing in every sift step.
            // Preserve exact semantics with a guarded word fast path and emit
            // the original exact body unchanged as the cold fallback.
            let arr = ctx.resolve(guard.arr).to_string();
            let start_exact = ctx.resolve(guard.start).to_string();
            let end_exact = ctx.resolve(guard.end).to_string();
            writeln!(
                output,
                "    /* heap-sift indices: guarded native-word fast path */"
            )
            .unwrap();
            writeln!(
                output,
                "    if (__builtin_expect(!({start_exact}).big && !({end_exact}).big && ({start_exact}).small >= 0 && ({end_exact}).small >= 0 && ({end_exact}).small < (int64_t)({arr}).len, 1)) {{"
            )
            .unwrap();

            ctx.enter_scope();
            let exact_bigints = ctx.bigint_vars.clone();
            ctx.bigint_vars.retain(|sym| !word_vars.contains(sym));
            let start_word = ctx.declare(guard.start, CType::Int64);
            let end_word = ctx.declare(guard.end, CType::Int64);
            writeln!(
                output,
                "        int64_t {start_word} = ({start_exact}).small;"
            )
            .unwrap();
            writeln!(output, "        int64_t {end_word} = ({end_exact}).small;").unwrap();
            ctx.in_body(false, |ctx| {
                ctx.ownership = Some(ownership.clone());
                for s in *body {
                    codegen_stmt(s, ctx, output, 2);
                }
            });
            ctx.bigint_vars = exact_bigints;
            ctx.exit_scope();

            writeln!(output, "    }} else {{").unwrap();
            ctx.enter_scope();
            ctx.in_body(false, |ctx| {
                ctx.ownership = Some(ownership.clone());
                for s in *body {
                    codegen_stmt(s, ctx, output, 2);
                }
            });
            ctx.exit_scope();
            writeln!(output, "    }}").unwrap();
            crate::optimize::mark_fired(crate::optimization::Opt::Peephole);
        } else {
            ctx.in_body(false, |ctx| {
                ctx.ownership = Some(ownership);
                for s in *body {
                    codegen_stmt(s, ctx, output, 1);
                }
            });
        }
        ctx.mutated = saved_changed;
        ctx.exit_scope();

        ctx.vars = saved_vars;
        ctx.tower_scope = saved_scope;
        ctx.bigint_vars = saved_big;
        ctx.number_locals = saved_numbers;
        ctx.current_fn_returns = saved_returns;
        ctx.nonnegative_vars = saved_nonnegative;
        writeln!(output, "}}\n").unwrap();
    }
}

// =============================================================================
// Entry Point
// =============================================================================

/// Every OTHER struct/enum a struct or enum's fields name AS A DIRECT VALUE (`Named`/`Primitive`,
/// never through a `Seq`/`Map`/`Set` generic) — the true "must be a COMPLETE type first" ordering
/// constraint. A container's element is stored behind a pointer (`Seq_<Name>` is `{ Name *data;
/// … }`), which C only needs FORWARD-DECLARED, not complete — `codegen_c_type_defs`'s Phase A
/// forward-declares every type unconditionally, so a container reference is never an ordering
/// dependency at all (treating it as one is what turned `JEntry { val: Json }` / `Json { …
/// entries: Seq of JEntry }` into an unresolvable false CYCLE: each direction "needed" the other's
/// FULL body). A type's OWN symbol is dropped: self-reference is a per-type LAYOUT concern
/// (forward-declare + box the field), never a cross-type ordering constraint.
fn type_value_deps(sym: Symbol, registry: &TypeRegistry) -> Vec<Symbol> {
    fn direct_dep(ty: &crate::analysis::FieldType, registry: &TypeRegistry, out: &mut Vec<Symbol>) {
        if let crate::analysis::FieldType::Named(s) | crate::analysis::FieldType::Primitive(s) = ty
        {
            if matches!(
                registry.get(*s),
                Some(
                    crate::analysis::TypeDef::Struct { .. } | crate::analysis::TypeDef::Enum { .. }
                )
            ) {
                out.push(*s);
            }
        }
    }
    let mut out = Vec::new();
    match registry.get(sym) {
        Some(crate::analysis::TypeDef::Struct { fields, .. }) => {
            for f in fields {
                direct_dep(&f.ty, registry, &mut out);
            }
        }
        Some(crate::analysis::TypeDef::Enum { variants, .. }) => {
            for v in variants {
                for f in &v.fields {
                    direct_dep(&f.ty, registry, &mut out);
                }
            }
        }
        _ => {}
    }
    out.retain(|d| *d != sym);
    out
}

/// One declaration order for EVERY struct and enum the program defines (`defined`, see
/// `callgraph::program_type_defs`), structs and enums interleaved by dependency — a struct can hold an enum by value (`JEntry { val: Json }`) just as
/// readily as an enum can hold another enum (`JsonResult::JOk(v: Json)`), so ordering the two
/// kinds as separate blocks (all structs, THEN all enums) leaves a struct-before-its-enum-field
/// case with no forward declaration at all (`error: unknown type name 'Json'`) — the C tier bug
/// this migration exists to close. `defined` is a hash set, so two independent
/// types (no dependency either way) would otherwise emit in a nondeterministic order too; the
/// stable name sort below (before the topological pass reorders true dependencies) makes
/// `compile_to_c` reproducible for those.
fn ordered_type_defs(
    registry: &TypeRegistry,
    interner: &Interner,
    defined: &std::collections::HashSet<Symbol>,
) -> Vec<Symbol> {
    use std::collections::HashSet;

    let mut syms: Vec<Symbol> = defined.iter().copied().collect();
    syms.sort_by(|a, b| interner.resolve(*a).cmp(interner.resolve(*b)));

    // Simple iterative topological sort (O(n^2) but n is small).
    let mut emitted: HashSet<Symbol> = HashSet::new();
    let mut ordered: Vec<Symbol> = Vec::new();
    let mut remaining = syms;
    while !remaining.is_empty() {
        let prev_len = remaining.len();
        remaining.retain(|sym| {
            let deps = type_value_deps(*sym, registry);
            if deps.iter().all(|d| emitted.contains(d)) {
                emitted.insert(*sym);
                ordered.push(*sym);
                return false; // remove from remaining
            }
            true
        });
        if remaining.len() == prev_len {
            // Circular dependency or missing type — emit remaining as-is.
            for sym in &remaining {
                ordered.push(*sym);
            }
            break;
        }
    }
    ordered
}

/// Every aggregate the C tier defines as a `struct` — the program's records and enums, the tuples
/// its signatures and statements use, and the nested-sequence wrappers — in four phases, each safe
/// only once the PRIOR phase has run for every type (the forward-declare/body split a genuinely
/// cyclic container reference needs, see `type_value_deps`):
/// A. forward-declare every record, enum and tuple (`typedef struct Name Name;`) — always legal, a
///    struct tag can be declared before its members are known.
/// B. every CONTAINER — `Seq_<Name>` for each of those, then each nested `Seq_Seq_…` innermost
///    first. A container holds its element behind a pointer, so it needs only A's declaration, and
///    a record may then hold any sequence by value.
/// C. every value body in TRUE by-value dependency order: records and enums interleaved (see
///    `ordered_type_defs`), then tuples, which may hold a record but never appear inside one.
/// D. every FUNCTION — each container's `new`/`push`/`get`/`set`/`len`, each `_to_string` and
///    `show_seq_…` — which copy elements BY VALUE and so need every body from C. Every Phase D
///    PROTOTYPE precedes every Phase D definition: the helpers call across types in whatever
///    direction the fields point (`Grid_to_string` renders a `Seq of Point` field through
///    `seq_Point_to_string`), and a container reference puts no order between the two, so no
///    emission order alone can put every callee first.
fn codegen_c_aggregates(
    registry: &TypeRegistry,
    interner: &Interner,
    defined: &std::collections::HashSet<Symbol>,
    display_names: &HashMap<Symbol, Symbol>,
    facts: &exact::FieldFacts,
    tuples: &[(String, Vec<CType>)],
    seqseqs: &[CType],
    output: &mut String,
) {
    use std::fmt::Write;

    let ordered = ordered_type_defs(registry, interner, defined);
    let users: Vec<(Symbol, String)> = ordered
        .iter()
        .map(|sym| (*sym, escape_c_ident(interner.resolve(*sym))))
        .collect();

    for (_, name) in &users {
        writeln!(output, "typedef struct {0} {0};", name).unwrap();
    }
    for (tname, _) in tuples {
        writeln!(output, "typedef struct {0} {0};", tname).unwrap();
    }

    for name in users
        .iter()
        .map(|(_, n)| n)
        .chain(tuples.iter().map(|(t, _)| t))
    {
        writeln!(
            output,
            "typedef struct {{ {name} *data; size_t len; size_t cap; }} Seq_{name};"
        )
        .unwrap();
    }
    for st in seqseqs {
        if let CType::SeqSeq(inner) = st {
            let elem = c_type_str_resolved(inner, interner);
            writeln!(
                output,
                "typedef struct {{ {elem} *data; size_t len; size_t cap; }} Seq_{elem};"
            )
            .unwrap();
        }
    }

    for body in aggregate_body_order(registry, interner, &users, tuples) {
        match body {
            Aggregate::User(sym) => match registry.get(sym) {
                Some(crate::analysis::TypeDef::Struct { .. }) => {
                    emit_struct_body(sym, registry, interner, facts, output)
                }
                Some(crate::analysis::TypeDef::Enum { .. }) => {
                    emit_enum_body(sym, registry, interner, facts, output)
                }
                _ => {}
            },
            Aggregate::Tuple(i) => {
                let (tname, elems) = &tuples[i];
                writeln!(output, "struct {tname} {{").unwrap();
                for (k, e) in elems.iter().enumerate() {
                    writeln!(output, "    {} f{};", c_type_str_resolved(e, interner), k).unwrap();
                }
                writeln!(output, "}};").unwrap();
            }
        }
    }

    let mut phase_d: Vec<CFn> = Vec::new();
    for (sym, name) in &users {
        phase_d.extend(seq_family(name, name));
        let shown = match registry.get(*sym) {
            Some(crate::analysis::TypeDef::Struct { .. }) => {
                struct_show(*sym, registry, interner, display_names, facts)
            }
            Some(crate::analysis::TypeDef::Enum { .. }) => {
                enum_show(*sym, registry, interner, display_names, facts)
            }
            _ => None,
        };
        if let Some(to_string) = shown {
            phase_d.push(to_string);
            phase_d.extend(seq_show(name, &format!("{name}_to_string(s->data[i])")));
        }
    }
    for (tname, elems) in tuples {
        phase_d.extend(seq_family(tname, tname));
        phase_d.push(tuple_show(tname, elems, interner));
        phase_d.extend(seq_show(tname, &format!("{tname}_to_string(s->data[i])")));
    }
    for st in seqseqs {
        if let CType::SeqSeq(inner) = st {
            let elem = c_type_str_resolved(inner, interner);
            phase_d.extend(seq_family(&elem, &elem));
            // Each element (itself a `Seq_…`) renders via its own `*_to_string` (`c_field_to_string_expr`
            // prepends the `&`), so a nested `Show [[1,2]]` prints `[[1, 2], …]` — matching the oracle.
            let render =
                crate::codegen_c::types::c_field_to_string_expr(inner, "s->data[i]", interner);
            phase_d.extend(seq_show(&elem, &render));
        }
    }
    for f in &phase_d {
        writeln!(output, "{};", f.sig).unwrap();
    }
    for f in &phase_d {
        writeln!(output, "{} {}", f.sig, f.body).unwrap();
    }
}

/// One value body Phase C emits: a record or enum, or a tuple (by its index).
#[derive(Clone, Copy)]
enum Aggregate {
    User(Symbol),
    Tuple(usize),
}

/// Every value body in by-value dependency order: a body follows every record, enum and tuple it
/// holds by value (a record may hold a tuple, a tuple a record). A container reference holds its
/// element behind a pointer and adds no edge, nor does an enum's boxed field naming itself. The
/// records keep `ordered_type_defs`' order and the tuples their collected order wherever no edge
/// decides; a genuine by-value cycle (not legal C either) is emitted as it stands.
fn aggregate_body_order(
    registry: &TypeRegistry,
    interner: &Interner,
    users: &[(Symbol, String)],
    tuples: &[(String, Vec<CType>)],
) -> Vec<Aggregate> {
    fn held(ct: &CType, interner: &Interner, out: &mut Vec<String>) {
        match ct {
            CType::Struct(s) | CType::Enum(s) => out.push(escape_c_ident(interner.resolve(*s))),
            CType::Tuple(_) | CType::Record(_) => out.push(c_type_str_resolved(ct, interner)),
            _ => {}
        }
    }
    let deps = |a: Aggregate| -> Vec<String> {
        let mut out = Vec::new();
        match a {
            Aggregate::User(sym) => {
                let field_types: Vec<&crate::analysis::FieldType> = match registry.get(sym) {
                    Some(crate::analysis::TypeDef::Struct { fields, .. }) => fields.iter().map(|f| &f.ty).collect(),
                    Some(crate::analysis::TypeDef::Enum { variants, .. }) => variants
                        .iter()
                        .flat_map(|v| v.fields.iter().map(|f| &f.ty))
                        .filter(|ft| !matches!(ft, crate::analysis::FieldType::Named(fsym) if *fsym == sym))
                        .collect(),
                    _ => Vec::new(),
                };
                for ft in field_types {
                    held(
                        &field_type_to_ctype(ft, interner, registry),
                        interner,
                        &mut out,
                    );
                }
                let own = escape_c_ident(interner.resolve(sym));
                out.retain(|d| *d != own);
            }
            Aggregate::Tuple(i) => {
                for e in &tuples[i].1 {
                    held(e, interner, &mut out);
                }
            }
        }
        out
    };
    let name = |a: Aggregate| match a {
        Aggregate::User(sym) => escape_c_ident(interner.resolve(sym)),
        Aggregate::Tuple(i) => tuples[i].0.clone(),
    };
    let mut remaining: Vec<Aggregate> = users
        .iter()
        .map(|(sym, _)| Aggregate::User(*sym))
        .chain((0..tuples.len()).map(Aggregate::Tuple))
        .collect();
    let defined: std::collections::HashSet<String> = remaining.iter().map(|a| name(*a)).collect();
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut order = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let before = remaining.len();
        remaining.retain(|a| {
            let ready = deps(*a)
                .iter()
                .all(|d| emitted.contains(d) || !defined.contains(d));
            if ready {
                emitted.insert(name(*a));
                order.push(*a);
            }
            !ready
        });
        if remaining.len() == before {
            order.extend(remaining.drain(..));
        }
    }
    order
}

/// One generated C function, its signature apart from its body so the prototype and the definition
/// are printed from the same string.
struct CFn {
    sig: String,
    body: String,
}

/// The dynamic-array family over one element type: `Seq_<key>` with `seq_<key>_new/push/get/set/len`
/// (1-based `get`/`set`, doubling growth). `elem` is the element's C type, `key` the name the family
/// is spelled with (they differ only for a nested sequence's element type).
fn seq_family(elem: &str, key: &str) -> Vec<CFn> {
    let sq = format!("Seq_{key}");
    let f = |sig: String, body: String| CFn { sig, body };
    vec![
        f(format!("__attribute__((unused)) static {sq} seq_{key}_new(void)"), format!("{{ return ({sq}){{NULL, 0, 0}}; }}")),
        f(
            format!("__attribute__((unused)) static void seq_{key}_push({sq} *s, {elem} val)"),
            format!("{{ if (s->len == s->cap) {{ s->cap = s->cap ? s->cap * 2 : 8; s->data = ({elem} *)lc_gc_realloc(s->data, s->cap * sizeof({elem})); }} s->data[s->len++] = val; }}"),
        ),
        f(format!("__attribute__((unused)) static {elem} seq_{key}_get({sq} *s, int64_t idx)"), "{ return s->data[logos_idx0(idx, s->len)]; }".to_string()),
        f(format!("__attribute__((unused)) static void seq_{key}_set({sq} *s, int64_t idx, {elem} val)"), "{ s->data[logos_idx0(idx, s->len)] = val; }".to_string()),
        f(format!("__attribute__((unused)) static int64_t seq_{key}_len({sq} *s)"), "{ return (int64_t)s->len; }".to_string()),
    ]
}

/// `seq_<key>_to_string` (`[a, b]`, each element rendered by `render`, an expression over
/// `s->data[i]` yielding a fresh `char*`) and the printing `show_seq_<key>` beside it.
fn seq_show(key: &str, render: &str) -> Vec<CFn> {
    let sq = format!("Seq_{key}");
    vec![
        CFn {
            sig: format!("__attribute__((unused)) static char *seq_{key}_to_string({sq} *s)"),
            body: format!("{{ char *out = lc_gc_strdup(\"[\"); for (size_t i = 0; i < s->len; i++) {{ if (i > 0) {{ char *c1 = str_concat(out, \", \"); lc_gc_free(out); out = c1; }} char *__e = {render}; char *c2 = str_concat(out, __e); lc_gc_free(out); lc_gc_free(__e); out = c2; }} char *cf = str_concat(out, \"]\"); lc_gc_free(out); return cf; }}"),
        },
        CFn {
            sig: format!("__attribute__((unused)) static void show_seq_{key}({sq} *s)"),
            body: format!("{{ char *t = seq_{key}_to_string(s); printf(\"%s\\n\", t); lc_gc_free(t); }}"),
        },
    ]
}

/// Whole-tuple Show: `(v0, v1, …)` — PARENS (distinct from a Seq's brackets), each field through
/// its own renderer.
fn tuple_show(tname: &str, elems: &[CType], interner: &Interner) -> CFn {
    use std::fmt::Write;

    let fmt: String = (0..elems.len())
        .map(|_| "%s")
        .collect::<Vec<_>>()
        .join(", ");
    let mut body = String::from("{\n");
    let mut args = Vec::new();
    for (i, e) in elems.iter().enumerate() {
        let render =
            crate::codegen_c::types::c_field_to_string_expr(e, &format!("v.f{i}"), interner);
        writeln!(body, "    char *__s{i} = {render};").unwrap();
        args.push(format!("__s{i}"));
    }
    writeln!(
        body,
        "    char *out = logos_dyn_sprintf(\"({fmt})\", {});",
        args.join(", ")
    )
    .unwrap();
    for i in 0..elems.len() {
        writeln!(body, "    lc_gc_free(__s{i});").unwrap();
    }
    writeln!(body, "    return out;").unwrap();
    body.push('}');
    CFn {
        sig: format!("__attribute__((unused)) static char *{tname}_to_string({tname} v)"),
        body,
    }
}

/// Phase C for a struct: its full field list, as `struct Name { … };` — matching Phase A's prior
/// `typedef struct Name Name;` (a plain tagged struct definition, not a second `typedef`).
fn emit_struct_body(
    sym: Symbol,
    registry: &TypeRegistry,
    interner: &Interner,
    facts: &exact::FieldFacts,
    output: &mut String,
) {
    use std::fmt::Write;

    if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = registry.get(sym) {
        let name = escape_c_ident(interner.resolve(sym));
        writeln!(output, "struct {name} {{").unwrap();
        for field in fields {
            let field_name = escape_c_ident(interner.resolve(field.name));
            let ctype = exact::field_ctype(sym, field, interner, registry, facts);
            let type_str = c_type_str_resolved(&ctype, interner);
            writeln!(output, "    {} {};", type_str, field_name).unwrap();
        }
        writeln!(output, "}};").unwrap();
    }
}

/// Phase D for a struct: the whole-value `Show` — `<Name>_to_string` renders `Name { field: value }`,
/// byte-identical to the interpreter oracle (brace-delimited, fields SORTED by name, nested values
/// through their own renderers). `None` when a field is not C-renderable; such a struct keeps the
/// inline field-show path.
fn struct_show(
    sym: Symbol,
    registry: &TypeRegistry,
    interner: &Interner,
    display_names: &HashMap<Symbol, Symbol>,
    facts: &exact::FieldFacts,
) -> Option<CFn> {
    use std::fmt::Write;

    let Some(crate::analysis::TypeDef::Struct { fields, .. }) = registry.get(sym) else {
        return None;
    };
    if !crate::codegen_c::types::c_struct_showable(sym, registry, interner) {
        return None;
    }
    let name = escape_c_ident(interner.resolve(sym));
    let ts = crate::codegen_c::types::struct_to_string_fn(sym, interner);
    let disp = crate::codegen_c::types::display_name_of(sym, display_names, interner);
    let mut sorted: Vec<&crate::analysis::FieldDef> = fields.iter().collect();
    sorted.sort_by(|a, b| interner.resolve(a.name).cmp(interner.resolve(b.name)));
    let mut body = String::from("{\n");
    if sorted.is_empty() {
        writeln!(body, "    return lstrdup(\"{disp}\");").unwrap();
    } else {
        let mut fmt_fields = Vec::new();
        let mut args = Vec::new();
        for (i, fd) in sorted.iter().enumerate() {
            let field_name = escape_c_ident(interner.resolve(fd.name));
            let ct = exact::field_ctype(sym, fd, interner, registry, facts);
            let render = crate::codegen_c::types::c_field_to_string_expr(
                &ct,
                &format!("v.{}", field_name),
                interner,
            );
            writeln!(body, "    char *__s{i} = {render};").unwrap();
            fmt_fields.push(format!("{}: %s", interner.resolve(fd.name)));
            args.push(format!("__s{i}"));
        }
        writeln!(
            body,
            "    char *out = logos_dyn_sprintf(\"{} {{ {} }}\", {});",
            disp,
            fmt_fields.join(", "),
            args.join(", ")
        )
        .unwrap();
        for i in 0..sorted.len() {
            writeln!(body, "    lc_gc_free(__s{i});").unwrap();
        }
        writeln!(body, "    return out;").unwrap();
    }
    body.push('}');
    Some(CFn {
        sig: format!("__attribute__((unused)) static char *{ts}({name} v)"),
        body,
    })
}

/// Phase D for an enum: `<Name>_to_string`, rendering a value exactly as the interpreter shows it —
/// a unit variant as its name, a variant with a payload as `Variant(a, b)` (positional, each field
/// through its own renderer, a boxed recursive field dereferenced), and an instance of the core
/// `Option` as its payload, or `nothing`. `None` when a field is not renderable.
fn enum_show(
    sym: Symbol,
    registry: &TypeRegistry,
    interner: &Interner,
    display_names: &HashMap<Symbol, Symbol>,
    facts: &exact::FieldFacts,
) -> Option<CFn> {
    use std::fmt::Write;

    let Some(crate::analysis::TypeDef::Enum { variants, .. }) = registry.get(sym) else {
        return None;
    };
    if !crate::codegen_c::types::c_enum_showable(sym, registry, interner) {
        return None;
    }
    let name = escape_c_ident(interner.resolve(sym));
    let ts = crate::codegen_c::types::struct_to_string_fn(sym, interner);
    let is_option = display_names
        .get(&sym)
        .is_some_and(|g| interner.resolve(*g) == "Option");
    let mut body = String::from("{\n    switch (v.tag) {\n");
    for v in variants {
        let vname = escape_c_ident(interner.resolve(v.name));
        let shown = interner.resolve(v.name);
        writeln!(body, "    case {name}_{vname}: {{").unwrap();
        let renders: Vec<String> = v
            .fields
            .iter()
            .map(|f| {
                let fname = escape_c_ident(interner.resolve(f.name));
                let boxed =
                    matches!(&f.ty, crate::analysis::FieldType::Named(fsym) if *fsym == sym);
                let access = if boxed {
                    format!("(*v.data.{vname}.{fname})")
                } else {
                    format!("v.data.{vname}.{fname}")
                };
                let ct = exact::field_ctype(v.name, f, interner, registry, facts);
                crate::codegen_c::types::c_field_to_string_expr(&ct, &access, interner)
            })
            .collect();
        match (is_option, renders.as_slice()) {
            (true, []) => writeln!(body, "        return lstrdup(\"nothing\");").unwrap(),
            (true, [payload]) => writeln!(body, "        return {payload};").unwrap(),
            (_, []) => writeln!(body, "        return lstrdup(\"{shown}\");").unwrap(),
            _ => {
                for (i, r) in renders.iter().enumerate() {
                    writeln!(body, "        char *__s{i} = {r};").unwrap();
                }
                let fmt = vec!["%s"; renders.len()].join(", ");
                let args: Vec<String> = (0..renders.len()).map(|i| format!("__s{i}")).collect();
                writeln!(
                    body,
                    "        char *out = logos_dyn_sprintf(\"{shown}({fmt})\", {});",
                    args.join(", ")
                )
                .unwrap();
                for i in 0..renders.len() {
                    writeln!(body, "        lc_gc_free(__s{i});").unwrap();
                }
                writeln!(body, "        return out;").unwrap();
            }
        }
        writeln!(body, "    }}").unwrap();
    }
    body.push_str("    }\n    return lstrdup(\"\");\n}");
    Some(CFn {
        sig: format!("__attribute__((unused)) static char *{ts}({name} v)"),
        body,
    })
}

/// Phase C for an enum: the tag enum, then the tagged-union body as `struct Name { … };` —
/// matching Phase A's prior forward declaration exactly like a struct's body does. A field
/// DIRECTLY naming its own enum (never through a container) still boxes to a pointer, exactly as
/// before — that per-field self-reference is a LAYOUT concern this phase split does not change.
fn emit_enum_body(
    sym: Symbol,
    registry: &TypeRegistry,
    interner: &Interner,
    facts: &exact::FieldFacts,
    output: &mut String,
) {
    use std::fmt::Write;
    if let Some(crate::analysis::TypeDef::Enum { variants, .. }) = registry.get(sym) {
        let name = escape_c_ident(interner.resolve(sym));

        // Tag enum
        write!(output, "typedef enum {{ ").unwrap();
        for (i, v) in variants.iter().enumerate() {
            let vname = escape_c_ident(interner.resolve(v.name));
            if i > 0 {
                write!(output, ", ").unwrap();
            }
            write!(output, "{}_{}", name, vname).unwrap();
        }
        writeln!(output, " }} {}_tag;\n", name).unwrap();

        let has_data = variants.iter().any(|v| !v.fields.is_empty());

        writeln!(output, "struct {} {{", name).unwrap();
        writeln!(output, "    {}_tag tag;", name).unwrap();
        if has_data {
            writeln!(output, "    union {{").unwrap();
            for v in variants {
                if v.fields.is_empty() {
                    continue;
                }
                let vname = escape_c_ident(interner.resolve(v.name));
                writeln!(output, "        struct {{").unwrap();
                for f in &v.fields {
                    let fname = escape_c_ident(interner.resolve(f.name));
                    let is_self_ref = if let crate::analysis::FieldType::Named(fsym) = &f.ty {
                        *fsym == sym
                    } else {
                        false
                    };
                    if is_self_ref {
                        writeln!(output, "            {} *{};", name, fname).unwrap();
                    } else {
                        let ctype = exact::field_ctype(v.name, f, interner, registry, facts);
                        let type_str = c_type_str_resolved(&ctype, interner);
                        writeln!(output, "            {} {};", type_str, fname).unwrap();
                    }
                }
                writeln!(output, "        }} {};", vname).unwrap();
            }
            writeln!(output, "    }} data;").unwrap();
        }
        writeln!(output, "}};\n").unwrap();
    }
}

pub fn codegen_program_c(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    interner: &Interner,
    policies: &crate::analysis::policy::PolicyRegistry,
) -> String {
    codegen_monomorphic_program_c(
        stmts,
        registry,
        interner,
        policies,
        &HashMap::new(),
        TaskTier::Coroutines,
    )
}

/// [`codegen_program_c`] for a monomorphized program: `instances` names the generic type each
/// instance type applies, so its values show under the generic's name.
pub fn codegen_monomorphic_program_c(
    stmts: &[Stmt],
    _registry: &TypeRegistry,
    interner: &Interner,
    policies: &crate::analysis::policy::PolicyRegistry,
    instances: &HashMap<Symbol, crate::monomorphize::TypeInstance>,
    tier: TaskTier,
) -> String {
    let mut output = String::with_capacity(4096);
    let mut ctx = CContext::new(interner, _registry);
    // The top-level context is the only one that emits `Check` statements, so it carries the real
    // policy registry; the helper/child contexts keep the empty default.
    ctx.policies = policies;
    ctx.display_names = instances
        .iter()
        .map(|(inst, of)| (*inst, of.generic))
        .collect();
    // The exact extension's reach: the bindings, parameters and results it reaches hold `LTower`s.
    ctx.tower = tower::reach_of(stmts, _registry, interner);
    // Where a stored `nothing` reaches: those bindings, parameters and results hold value-or-nothing.
    ctx.absence = absent::of_program(stmts, _registry, interner);

    // Does the program use the deterministic concurrency runtime? This single flag gates EVERYTHING
    // concurrency-related below (the `ucontext` scheduler prelude, the task-entry wrappers, wrapping
    // Main as a task + `logos_sched_bootstrap`). When it is false the output is byte-identical to a
    // world without the scheduler — the invariant that keeps the ~1246 non-concurrency C tests fixed.
    let uses_concurrency = program_uses_concurrency(stmts);
    ctx.uses_concurrency = uses_concurrency;
    // Under the stepped tier, the functions that wait run only as step functions, and a call to one from a
    // stepped body pushes its frame.
    let steps =
        (uses_concurrency && tier == TaskTier::Stepped).then(|| stepped::stepped_functions(stmts));
    if let Some(s) = &steps {
        ctx.stepped_fns = s.waits.clone();
    }

    // Does the program use a single-replica counter CRDT (`ConvergentCount`/`Tally`)? This gates the
    // `logos_crdt` runtime and seeds the `(struct, field)` counter side-set the codegen consults. When
    // no struct has a counter field, the runtime is not emitted and the output is byte-identical to a
    // world without CRDTs — the invariant that keeps the ~1246 non-CRDT C tests fixed.
    let uses_crdt = program_uses_crdt_counter(_registry, interner);
    ctx.counter_fields = counter_crdt_fields(_registry, interner);

    // The scalar exact higher-order functions: the call-site half is ONE decision shared with the Rust
    // backend (`codegen::bigint_promote::exact_hof_fns`: every site passes an INLINE closure literal in every
    // `fn`-typed position, since a closure VARIABLE was already emitted `int64_t (*)(int64_t)` at its `Let`,
    // and not every site is provably within `i64`); the shape half is `is_exact_hof`. Then every
    // exact-integer slot of the program (see `exact`), before any type, signature or body is emitted.
    let site_exact_hofs = crate::codegen::bigint_promote::exact_hof_fns(stmts, interner);
    ctx.exact_hofs = stmts
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef {
                name,
                params,
                is_native: false,
                ..
            } => {
                let param_ctypes: Vec<CType> = params
                    .iter()
                    .map(|(_, pt)| resolve_type_expr_with_registry(pt, interner, Some(_registry)))
                    .collect();
                // A function lifted out of a closure is a function value the program wrote: exact, as a closure is.
                let lifted = crate::optimize::is_lifted_closure(*name);
                ((is_exact_hof(&param_ctypes) && site_exact_hofs.contains(name)) || lifted)
                    .then_some(*name)
            }
            _ => None,
        })
        .collect();
    ctx.int_oracle = Some(std::rc::Rc::new(crate::optimize::oracle_analyze_with(
        stmts, interner,
    )));
    ctx.exact = exact::exact_slots(
        stmts,
        _registry,
        interner,
        &ctx.exact_hofs,
        &ctx.counter_fields,
        ctx.int_oracle.as_deref(),
    );
    ctx.quiet_fns = std::rc::Rc::new(quiet_functions(stmts, interner));

    output.push_str(runtime::C_GC_COLLECTOR);
    output.push_str(&c_runtime());
    output.push_str(CLOSURE_TYPE);
    output.push_str(TEXT_BUILDER);
    output.push_str(CHECKED_INT);
    if ctx.tower.is_some() {
        output.push_str(tower::RUNTIME);
    }
    if uses_concurrency {
        if steps.is_some() {
            output.push_str(C_STEPPED_HOST);
            output.push_str(STEPPED_SCHEDULER);
        } else {
            output.push_str(C_CONCURRENCY_RUNTIME);
        }
    }
    if uses_crdt {
        output.push_str(C_CRDT_RUNTIME);
    }
    // The offline loopback networking transport — emitted only when the program uses networking, so a
    // non-net program stays byte-identical.
    if program_uses_net(stmts) {
        output.push_str(&c_net_runtime());
    }
    if program_uses_files(stmts) {
        output.push_str(FILE_RUNTIME);
    }
    ctx.nothing_names = nothing_names(stmts);
    let mut holds_nothing = !ctx.nothing_names.is_empty() || !ctx.absence.is_empty();
    for_each_node(stmts, |node| {
        holds_nothing |= matches!(
            node,
            Visited::Expr(Expr::ChunkAt { .. }) | Visited::Stmt(Stmt::Pop { into: Some(_), .. })
        );
    });
    if !uses_concurrency && holds_nothing {
        output.push_str(OPT_RUNTIME);
    }
    // A list or a map whose items may be nothing holds each as a value-or-nothing: one family for every payload,
    // since a value-or-nothing is one C type; what renders and compares them is the payload's.
    if holds_nothing {
        output.push_str(OPT_SEQ_FAMILY);
        let opt = || Box::new(CType::Opt(Box::new(CType::Int64)));
        emit_generic_map_family(&mut output, &CType::MapStrValue(opt()), interner, false);
        emit_generic_map_family(&mut output, &CType::MapI64Value(opt()), interner, false);
    }

    // Value-type `Seq` families — `Seq of Money/Quantity/Modular/Uuid/Complex/decimal` on the C tier.
    // The element structs are defined in `C_RUNTIME`; here we synthesize the dynamic-array family +
    // a `[a, b]` Show for each. Always emitted (`__attribute__((unused))` → gcc drops the unused ones),
    // so a Seq literal of any value type resolves without threading a collection pass. Keyed on the
    // element C-type; the per-type `*_to_string` renders each element (the `LDec` one takes a pointer).
    for (elem, to_str) in [
        ("LMoney", "money_to_string(s->data[i])"),
        ("LQuant", "quant_to_string(s->data[i])"),
        ("LMod", "mod_to_string(s->data[i])"),
        ("LUuid", "uuid_to_string(s->data[i])"),
        ("LComplex", "cplx_to_string(s->data[i])"),
        ("LDec", "ldec_to_string(&s->data[i])"),
    ] {
        let sq = format!("Seq_{elem}");
        writeln!(
            output,
            "typedef struct {{ {elem} *data; size_t len; size_t cap; }} {sq};"
        )
        .unwrap();
        writeln!(output, "__attribute__((unused)) static {sq} seq_{elem}_new(void) {{ return ({sq}){{NULL, 0, 0}}; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static void seq_{elem}_push({sq} *s, {elem} val) {{ if (s->len == s->cap) {{ s->cap = s->cap ? s->cap * 2 : 8; s->data = ({elem} *)lc_gc_realloc(s->data, s->cap * sizeof({elem})); }} s->data[s->len++] = val; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static {elem} seq_{elem}_get({sq} *s, int64_t idx) {{ return s->data[logos_idx0(idx, s->len)]; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static void seq_{elem}_set({sq} *s, int64_t idx, {elem} val) {{ s->data[logos_idx0(idx, s->len)] = val; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static int64_t seq_{elem}_len({sq} *s) {{ return (int64_t)s->len; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static char *seq_{elem}_to_string({sq} *s) {{ char *out = lc_gc_strdup(\"[\"); for (size_t i = 0; i < s->len; i++) {{ if (i > 0) {{ char *c1 = str_concat(out, \", \"); lc_gc_free(out); out = c1; }} char *__e = {to_str}; char *c2 = str_concat(out, __e); lc_gc_free(out); lc_gc_free(__e); out = c2; }} char *cf = str_concat(out, \"]\"); lc_gc_free(out); return cf; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static void show_seq_{elem}({sq} *s) {{ char *t = seq_{elem}_to_string(s); printf(\"%s\\n\", t); lc_gc_free(t); }}").unwrap();
    }

    // Value-type `Map` families — `Map of Text to Money/Quantity/…` → `Map_str_LMoney` etc., an
    // open-addressing string-keyed hash map whose VALUE is a value-type struct (mirrors `Map_str_i64`,
    // reusing the runtime's `_map_hash_str`). Always emitted, `__attribute__((unused))`. `get` on a
    // missing key returns a zeroed value (correct programs never hit it).
    for (elem, vstr) in [
        ("LMoney", "money_to_string(m->vals[si])"),
        ("LQuant", "quant_to_string(m->vals[si])"),
        ("LMod", "mod_to_string(m->vals[si])"),
        ("LUuid", "uuid_to_string(m->vals[si])"),
        ("LComplex", "cplx_to_string(m->vals[si])"),
        ("LDec", "ldec_to_string(&m->vals[si])"),
    ] {
        let mp = format!("Map_str_{elem}");
        writeln!(output, "typedef struct {{ char **keys; {elem} *vals; uint8_t *state; size_t cap; size_t len; size_t *order; size_t ord_len; }} {mp};").unwrap();
        writeln!(output, "__attribute__((unused)) static {mp} map_str_{elem}_new(void) {{ size_t cap = 16; {mp} m = {{(char **)lc_gc_calloc(cap, sizeof(char *)), ({elem} *)lc_gc_calloc(cap, sizeof({elem})), (uint8_t *)lc_gc_calloc(cap, 1), cap, 0, (size_t *)lc_gc_calloc(cap, sizeof(size_t)), 0}}; return m; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static void _map_str_{elem}_resize({mp} *m) {{ char **ok = m->keys; {elem} *ov = m->vals; uint8_t *os = m->state; size_t *oo = m->order; size_t ool = m->ord_len; size_t nc = m->cap * 2; m->keys = (char **)lc_gc_calloc(nc, sizeof(char *)); m->vals = ({elem} *)lc_gc_calloc(nc, sizeof({elem})); m->state = (uint8_t *)lc_gc_calloc(nc, 1); m->order = (size_t *)lc_gc_calloc(nc, sizeof(size_t)); m->cap = nc; m->len = 0; m->ord_len = 0; for (size_t oi = 0; oi < ool; oi++) {{ size_t si = oo[oi]; if (!os[si]) continue; size_t idx = _map_hash_str(ok[si], nc); while (m->state[idx]) idx = (idx + 1) & (nc - 1); m->keys[idx] = ok[si]; m->vals[idx] = ov[si]; m->state[idx] = 1; m->len++; m->order[m->ord_len++] = idx; }} lc_gc_free(ok); lc_gc_free(ov); lc_gc_free(os); lc_gc_free(oo); }}").unwrap();
        writeln!(output, "__attribute__((unused)) static void map_str_{elem}_set({mp} *m, const char *key, {elem} val) {{ if (m->ord_len * 4 >= m->cap * 3) _map_str_{elem}_resize(m); size_t idx = _map_hash_str(key, m->cap); while (m->state[idx]) {{ if (strcmp(m->keys[idx], key) == 0) {{ m->vals[idx] = val; return; }} idx = (idx + 1) & (m->cap - 1); }} m->keys[idx] = lc_gc_strdup(key); m->vals[idx] = val; m->state[idx] = 1; m->len++; m->order[m->ord_len++] = idx; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static {elem} map_str_{elem}_get({mp} *m, const char *key) {{ size_t idx = _map_hash_str(key, m->cap); while (m->state[idx]) {{ if (strcmp(m->keys[idx], key) == 0) return m->vals[idx]; idx = (idx + 1) & (m->cap - 1); }} {elem} z; memset(&z, 0, sizeof({elem})); return z; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static bool map_str_{elem}_contains({mp} *m, const char *key) {{ size_t idx = _map_hash_str(key, m->cap); while (m->state[idx]) {{ if (strcmp(m->keys[idx], key) == 0) return true; idx = (idx + 1) & (m->cap - 1); }} return false; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static void map_str_{elem}_remove({mp} *t, const char *key) {{ size_t mask = t->cap - 1, i = _map_hash_str(key, t->cap); while (t->state[i] && strcmp(t->keys[i], key) != 0) i = (i + 1) & mask; if (!t->state[i]) return; _logos_order_drop(t->order, &t->ord_len, i); t->len--; for (size_t j = (i + 1) & mask; t->state[j]; j = (j + 1) & mask) {{ if (_logos_slot_backs(_map_hash_str(t->keys[j], t->cap), i, j, mask)) {{ t->keys[i] = t->keys[j]; t->vals[i] = t->vals[j]; _logos_order_move(t->order, t->ord_len, j, i); i = j; }} }} t->state[i] = 0; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static int64_t map_str_{elem}_len({mp} *m) {{ return (int64_t)m->len; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static char *map_str_{elem}_to_string({mp} *m) {{ char *out = lc_gc_strdup(\"{{\"); int first = 1; for (size_t oi = 0; oi < m->ord_len; oi++) {{ size_t si = m->order[oi]; if (!m->state[si]) continue; char *__v = {vstr}; char *e = logos_dyn_sprintf(\"%s%s: %s\", first ? \"\" : \", \", m->keys[si], __v); lc_gc_free(__v); char *nw = str_concat(out, e); lc_gc_free(out); lc_gc_free(e); out = nw; first = 0; }} char *nw = str_concat(out, \"}}\"); lc_gc_free(out); return nw; }}").unwrap();
        writeln!(output, "__attribute__((unused)) static void show_map_str_{elem}({mp} *m) {{ char *t = map_str_{elem}_to_string(m); printf(\"%s\\n\", t); lc_gc_free(t); }}").unwrap();
    }

    // Maps whose value no fixed family holds — `Map of Text to Seq of Int` → the generic `Map_str_Seq_i64`
    // family, `Map of Int to Hyperreal` → `Map_i64_LTower`. Emitted HERE (before struct defs) so a struct may
    // hold such a map; the value's type is already in scope (scalar seqs live in `C_RUNTIME`, value-type seqs
    // just above, `LTower` with the extension's runtime). Collected from signatures AND
    // statement expressions (`is_c_map_value` deliberately excludes struct/nested-seq values, whose Seq
    // families are emitted later).
    {
        let mut mv_seen = std::collections::HashSet::new();
        let mut map_values: Vec<CType> = Vec::new();
        for stmt in stmts {
            if let Stmt::FunctionDef {
                params,
                return_type,
                ..
            } = stmt
            {
                if let Some(rt) = return_type {
                    collect_mapvalue_from_ctype(
                        &resolve_type_expr_with_registry(rt, interner, Some(_registry)),
                        &mut mv_seen,
                        &mut map_values,
                        interner,
                    );
                }
                for (_, pt) in params.iter() {
                    collect_mapvalue_from_ctype(
                        &resolve_type_expr_with_registry(pt, interner, Some(_registry)),
                        &mut mv_seen,
                        &mut map_values,
                        interner,
                    );
                }
            }
        }
        {
            let mut mv_ctx = CContext::new(interner, _registry);
            mv_ctx.exact = ctx.exact.clone();
            mv_ctx.tower = ctx.tower.clone();
            mv_ctx.absence = ctx.absence.clone();
            collect_stmt_mapvalue_types(
                stmts,
                &mut mv_ctx,
                &mut mv_seen,
                &mut map_values,
                interner,
            );
        }
        for map in &map_values {
            emit_generic_map_family(&mut output, map, interner, true);
        }
    }

    // Every aggregate: the program's records and enums, plus each tuple and nested sequence its
    // signatures, statements and record fields use (deduped, nested-first), emitted as one ordered
    // unit (see `codegen_c_aggregates`).
    {
        let (mut tup_seen, mut tuples) = (
            std::collections::HashSet::new(),
            Vec::<(String, Vec<CType>)>::new(),
        );
        let (mut ss_seen, mut seqseqs) = (std::collections::HashSet::new(), Vec::<CType>::new());
        let mut collect = |cty: &CType| {
            collect_tuple_types(cty, &mut tup_seen, &mut tuples, interner);
            collect_seqseq_types(cty, &mut ss_seen, &mut seqseqs, interner);
        };
        let defined = crate::analysis::callgraph::program_type_defs(stmts, _registry);
        for (_, def) in _registry
            .iter_types()
            .filter(|(sym, _)| defined.contains(*sym))
        {
            let field_types: Vec<&crate::analysis::FieldType> = match def {
                crate::analysis::TypeDef::Struct { fields, .. } => {
                    fields.iter().map(|f| &f.ty).collect()
                }
                crate::analysis::TypeDef::Enum { variants, .. } => variants
                    .iter()
                    .flat_map(|v| v.fields.iter().map(|f| &f.ty))
                    .collect(),
                _ => Vec::new(),
            };
            for ft in field_types {
                collect(&field_type_to_ctype(ft, interner, _registry));
            }
        }
        for stmt in stmts {
            if let Stmt::FunctionDef {
                name,
                params,
                return_type,
                ..
            } = stmt
            {
                if let Some(rt) = return_type {
                    collect(&ctx.exact.widened_return(
                        *name,
                        resolve_type_expr_with_registry(rt, interner, Some(_registry)),
                    ));
                }
                for (_, pt) in params.iter() {
                    collect(&resolve_type_expr_with_registry(
                        pt,
                        interner,
                        Some(_registry),
                    ));
                }
            }
        }
        // Also collect from statement expressions (bare `Show (1, 2)`, `Let g be [[1,2]]`, …).
        let mut stmt_ctx = CContext::new(interner, _registry);
        stmt_ctx.exact = ctx.exact.clone();
        stmt_ctx.tower = ctx.tower.clone();
        stmt_ctx.absence = ctx.absence.clone();
        collect_stmt_tuple_types(stmts, &mut stmt_ctx, &mut tup_seen, &mut tuples, interner);
        let mut stmt_ctx = CContext::new(interner, _registry);
        stmt_ctx.exact = ctx.exact.clone();
        stmt_ctx.tower = ctx.tower.clone();
        stmt_ctx.absence = ctx.absence.clone();
        collect_stmt_seqseq_types(stmts, &mut stmt_ctx, &mut ss_seen, &mut seqseqs, interner);
        codegen_c_aggregates(
            _registry,
            interner,
            &defined,
            &ctx.display_names,
            &ctx.field_facts(),
            &tuples,
            &seqseqs,
            &mut output,
        );
    }

    // First pass: register every function's C signature (for forward references): its declared types, with
    // each `Int` return and parameter the exact-slot analysis settled on `LInt`, and an exact HOF's integers.
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            return_type,
            is_native,
            ..
        } = stmt
        {
            if *is_native {
                let fname = interner.resolve(*name);
                let ret_type = match fname {
                    "args" => CType::SeqStr,
                    "parseInt" => CType::Int64,
                    "parseFloat" => CType::Float64,
                    _ => {
                        if let Some(rt) = return_type {
                            resolve_type_expr_with_registry(rt, interner, Some(_registry))
                        } else {
                            CType::Void
                        }
                    }
                };
                ctx.funcs.insert(*name, ret_type);
                ctx.native_funcs.insert(*name);
                let param_types = params
                    .iter()
                    .map(|(_, pt)| resolve_type_expr_with_registry(pt, interner, Some(_registry)))
                    .collect();
                ctx.fn_params.insert(*name, param_types);
                ctx.fn_inout.insert(
                    *name,
                    params.iter().map(|(_, pt)| types::is_inout(pt)).collect(),
                );
            } else {
                let mut ret_type = if let Some(rt) = return_type {
                    ctx.exact.widened_return(
                        *name,
                        resolve_type_expr_with_registry(rt, interner, Some(_registry)),
                    )
                } else {
                    CType::Void
                };
                if ret_type == CType::Int64 && ctx.exact_hofs.contains(name) {
                    ret_type = CType::BigInt;
                }
                if tower::returns(&ctx, *name) {
                    ret_type = CType::Tower;
                }
                let ret_type = absent::returned(*name, ret_type, &ctx);
                ctx.funcs.insert(*name, ret_type);
                let param_types = params
                    .iter()
                    .map(|(pn, pt)| param_ctype(&ctx, *name, *pn, pt))
                    .collect();
                ctx.fn_params.insert(*name, param_types);
                ctx.fn_inout.insert(
                    *name,
                    params.iter().map(|(_, pt)| types::is_inout(pt)).collect(),
                );
            }
        }
    }
    ctx.plain_texts = lent_texts(stmts, &ctx);
    // A function that declares no return type answers what its `Return`s hand back (a closure lifted to a
    // function, a `## To` written without `->`), settled once every function a `Return` may call is.
    for _ in 0..=stmts.len() {
        let mut settled = true;
        for stmt in stmts {
            if let Stmt::FunctionDef {
                name,
                params,
                body,
                return_type: None,
                is_native: false,
                ..
            } = stmt
            {
                let answered = absent::returned(*name, undeclared_return(*name, params, body, &ctx), &ctx);
                if ctx.funcs.get(name) != Some(&answered) {
                    ctx.funcs.insert(*name, answered);
                    settled = false;
                }
            }
        }
        if settled {
            break;
        }
    }

    // A labeled seam between the fixed C runtime/type prelude (above) and the
    // program's own code (below). The CUDA AOT tier lifts everything after this
    // marker onto the device; nothing before it (the glibc-heavy dynamic runtime)
    // is device code. Inert for every other consumer — it is a C comment.
    output.push_str(USER_CODE_SENTINEL);
    output.push('\n');

    // Forward declarations
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            return_type,
            is_native,
            ..
        } = stmt
        {
            if *is_native || steps.as_ref().is_some_and(|s| s.waits.contains(name)) {
                continue;
            }
            let func_name = escape_c_ident(ctx.interner.resolve(*name));
            // Use the first-pass return type (which already widened `Int`→`LInt` for a BigInt-returning
            // function) so the forward declaration matches the definition.
            let ret_type = ctx.funcs.get(name).cloned().unwrap_or_else(|| {
                if let Some(rt) = return_type {
                    resolve_type_expr_with_registry(rt, interner, Some(_registry))
                } else {
                    CType::Void
                }
            });
            // The first pass's parameter types, so prototype and definition agree.
            let param_strs: Vec<String> = params
                .iter()
                .map(|(pname, ptype)| {
                    let ty = param_ctype(&ctx, *name, *pname, ptype);
                    if types::is_inout(ptype) {
                        types::c_inout_param_decl(&ty, &ctx.resolve(*pname), interner)
                    } else {
                        c_param_decl(&ty, &ctx.resolve(*pname), interner)
                    }
                })
                .collect();
            writeln!(
                output,
                "{} {}({});",
                c_type_str_resolved(&ret_type, interner),
                func_name,
                param_strs.join(", ")
            )
            .unwrap();
        }
    }
    // The spawned functions, each with the arity it is launched at, in a stable (name-sorted) order. Their
    // task-entry wrappers are declared here, beside the functions, so a body that launches a task can name
    // a wrapper defined after it. Every entry takes its task's arguments as one array of scalars.
    let spawned: Vec<(Symbol, usize)> = if uses_concurrency && steps.is_none() {
        let mut arities: HashMap<Symbol, usize> = HashMap::new();
        collect_spawned_arities(stmts, &mut arities);
        let mut spawned: Vec<(Symbol, usize)> = arities.into_iter().collect();
        spawned.sort_by(|a, b| interner.resolve(a.0).cmp(interner.resolve(b.0)));
        spawned
    } else {
        Vec::new()
    };
    for &(func, nargs) in &spawned {
        // The callee's own parameter types; an unresolved (parse-only) name takes Ints, so its entry links.
        let params = stmts
            .iter()
            .find_map(|stmt| match stmt {
                Stmt::FunctionDef {
                    name,
                    params,
                    is_native: false,
                    ..
                } if *name == func => Some(
                    params
                        .iter()
                        .map(|(pn, pt)| param_ctype(&ctx, func, *pn, pt))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_else(|| vec![CType::Int64; nargs]);
        ctx.task_params.insert(func, params);
        writeln!(
            output,
            "static long long logos_taskentry_{}(const long long *__targs);",
            escape_c_ident(interner.resolve(func))
        )
        .unwrap();
    }
    output.push('\n');

    // Function definitions + main go into a separate buffer so the lifted lambda functions
    // (accumulated in `ctx.lifted` as a SIDE EFFECT of codegen) can be emitted BEFORE the code
    // that references them. Lambda bodies may call user functions — the forward declarations above
    // cover those — and `main` references the lambda names, so this ordering satisfies both.
    // The program's bindings a function reaches become file-scope statics. Main is lowered first then, so each
    // static's C type is the one its root `Let` gives it, and every function body is lowered knowing it.
    let statics = program_statics(stmts, interner);
    for stmt in stmts {
        if let Stmt::FunctionDef { body, .. } = stmt {
            ctx.changed_statics.extend(
                values::mutated_roots(body, &ctx.fn_inout)
                    .into_iter()
                    .filter(|g| statics.names.contains(g)),
            );
        }
    }
    // `## Main` changes a static in place as a function does: a body that copies the static copies it then too.
    let main: Vec<Stmt> = stmts
        .iter()
        .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
        .cloned()
        .collect();
    ctx.changed_statics.extend(
        values::mutated_roots(&main, &ctx.fn_inout)
            .into_iter()
            .filter(|g| statics.names.contains(g)),
    );
    // The parameters each plain function changes in place, which its callers copy where they read the argument
    // again. A step and a closure copy what they change on entry instead.
    let caller_copied: Vec<(Symbol, Vec<Symbol>, &[Stmt])> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef {
                name,
                params,
                body,
                is_native: false,
                ..
            } if !steps.as_ref().is_some_and(|st| st.waits.contains(name)) => {
                Some((*name, params.iter().map(|(p, _)| *p).collect(), *body))
            }
            _ => None,
        })
        .collect();
    let call_shapes: values::CallShapes = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef {
                name,
                params,
                return_type: Some(rt),
                ..
            } => {
                let holds = |t: &TypeExpr| {
                    values::holds_buffer(
                        &resolve_type_expr_with_registry(t, interner, Some(_registry)),
                        _registry,
                        interner,
                    )
                };
                let result_holds = holds(rt);
                Some((
                    *name,
                    params
                        .iter()
                        .map(|(_, pt)| result_holds && holds(pt))
                        .collect(),
                ))
            }
            _ => None,
        })
        .collect();
    ctx.call_shapes = std::rc::Rc::new(sharing::narrowed_call_shapes(
        &caller_copied,
        &call_shapes,
        &ctx.fn_inout,
    ));
    ctx.mutating = std::rc::Rc::new(sharing::mutating_params(
        &caller_copied,
        &ctx.changed_statics,
        &ctx.call_shapes,
        &ctx.fn_inout,
    ));
    let fixed_stack_plan = c_fixed_stack_plan(stmts, &ctx);
    let fixed_for = |scope: Option<Symbol>| {
        fixed_stack_plan
            .iter()
            .filter_map(|(&(owner, symbol), &capacity)| {
                (owner == scope).then_some((symbol, capacity))
            })
            .collect::<HashMap<_, _>>()
    };
    ctx.text_bytes_candidates = c_text_bytes_view_plan(stmts, interner);
    ctx.fixed_stack_seqs = fixed_for(None);
    for refusal in &statics.refusals {
        output.push_str(&format!(
            "\n#error \"the C backend does not lower {}\"\n",
            refusal.replace('"', "'")
        ));
    }
    let early_main = if statics.names.is_empty() {
        None
    } else {
        ctx.program_statics = statics.names.iter().copied().collect();
        let vars_before_main = ctx.vars.clone();
        if let Some(s) = &steps {
            register_step_params(stmts, s, &mut ctx);
        }
        let main_code = emit_main(stmts, &mut ctx, uses_concurrency, tier);
        ctx.vars = vars_before_main;
        for g in &statics.names {
            if let Some(ty) = ctx.static_types.get(g).cloned() {
                writeln!(
                    output,
                    "static {};",
                    c_param_decl(&ty, &ctx.resolve(*g), interner)
                )
                .unwrap();
                ctx.vars.insert(*g, ty);
            }
        }
        output.push('\n');
        Some(main_code)
    };

    if let Some(s) = &steps {
        register_step_params(stmts, s, &mut ctx);
    }
    let mut body = String::new();
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name,
            is_native: false,
            ..
        } = stmt
        {
            if !steps.as_ref().is_some_and(|s| s.waits.contains(name)) {
                ctx.fixed_stack_seqs = fixed_for(Some(*name));
                codegen_function(stmt, &mut ctx, &mut body);
            }
        }
    }
    // The step functions, in a stable (name-sorted) order: every frame is typed before any step names it.
    let mut frames = String::new();
    let mut step_bodies = String::new();
    let mut step_names: Vec<String> = Vec::new();
    if let Some(s) = &steps {
        let mut stepped_defs: Vec<&Stmt> = stmts
            .iter()
            .filter(|st| matches!(st, Stmt::FunctionDef { name, is_native: false, .. } if s.steps().any(|f| f == name)))
            .collect();
        stepped_defs.sort_by_key(|st| match st {
            Stmt::FunctionDef { name, .. } => interner.resolve(*name).to_string(),
            _ => String::new(),
        });
        for def in stepped_defs {
            if let Stmt::FunctionDef { name, .. } = def {
                ctx.fixed_stack_seqs = fixed_for(Some(*name));
                step_names.push(escape_c_ident(interner.resolve(*name)));
            }
            let (frame, step) = codegen_step_function(def, &mut ctx);
            frames.push_str(&frame);
            step_bodies.push_str(&step);
        }
    }

    // Task-entry wrappers — one `long long logos_taskentry_<fn>(const long long *__targs)` per spawned function,
    // so the scheduler calls every task body the same way: by pointer, with the task's arguments as one
    // array of scalars. Each unpacks its arguments to the callee's real C parameter types (see
    // `emit::task_arg_unpack`); a task answers `nothing`, so the callee's own result is dropped. Emitted
    // ONLY under concurrency, after the definitions they reference.
    for (func, _) in &spawned {
        let fname = escape_c_ident(interner.resolve(*func));
        let params = ctx.task_params.get(func).cloned().unwrap_or_default();
        let call_args: Vec<String> = params
            .iter()
            .enumerate()
            .map(|(i, ty)| emit::task_arg_unpack(ty, &format!("__targs[{i}]"), interner))
            .collect();
        writeln!(
            body,
            "static long long logos_taskentry_{}(const long long *__targs) {{",
            fname
        )
        .unwrap();
        if params.is_empty() {
            writeln!(body, "    (void)__targs;").unwrap();
        }
        writeln!(body, "    {}({});", fname, call_args.join(", ")).unwrap();
        writeln!(body, "    return 0;").unwrap();
        writeln!(body, "}}").unwrap();
    }

    let main_code = match early_main {
        Some(main_code) => main_code,
        None => {
            ctx.fixed_stack_seqs = fixed_for(None);
            emit_main(stmts, &mut ctx, uses_concurrency, tier)
        }
    };

    let mut lifted_once = std::collections::HashSet::new();
    for lam in ctx.lifted.borrow().iter() {
        if lifted_once.insert(lam.clone()) {
            output.push_str(lam);
        }
    }
    output.push('\n');
    if steps.is_some() {
        output.push_str(&frames);
        output.push_str(&main_code.frame);
        for name in step_names
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(MAIN_STEP))
        {
            writeln!(output, "static int logos_step_{name}(void *fp);").unwrap();
        }
        output.push('\n');
    }
    output.push_str(&body);
    output.push_str(&step_bodies);
    output.push_str(&main_code.step);
    output.push_str(&main_code.entry);
    output
}

/// The C type a function declared without a return type answers: its first `Return`'s value, typed with its
/// parameters and the bindings before it in scope; `Void` when no `Return` carries a value.
fn undeclared_return(
    function: Symbol,
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    ctx: &CContext,
) -> CType {
    fn first_answer<'a>(body: &'a [Stmt<'a>], scope: &mut CContext<'a>) -> Option<CType> {
        for s in body {
            match s {
                Stmt::Let { var, value, ty, .. } => {
                    let bound = match ty {
                        Some(t) => {
                            resolve_type_expr_with_registry(t, scope.interner, Some(scope.registry))
                        }
                        None => crate::codegen_c::types::infer_expr_type(value, scope),
                    };
                    scope.vars.insert(*var, bound);
                }
                Stmt::Return { value: Some(v) } => {
                    return Some(crate::codegen_c::types::infer_expr_type(v, scope))
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    if let Some(t) = first_answer(then_block, scope) {
                        return Some(t);
                    }
                    if let Some(t) = else_block.and_then(|b| first_answer(b, scope)) {
                        return Some(t);
                    }
                }
                Stmt::While { body, .. }
                | Stmt::Repeat { body, .. }
                | Stmt::Zone { body, .. }
                | Stmt::Splice { body }
                | Stmt::Reversibly { body }
                | Stmt::During { body, .. } => {
                    if let Some(t) = first_answer(body, scope) {
                        return Some(t);
                    }
                }
                Stmt::Inspect { arms, .. } => {
                    for arm in arms {
                        if let Some(t) = first_answer(arm.body, scope) {
                            return Some(t);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }
    let mut scope = CContext::new(ctx.interner, ctx.registry);
    scope.funcs = ctx.funcs.clone();
    scope.native_funcs = ctx.native_funcs.clone();
    scope.fn_params = ctx.fn_params.clone();
    scope.exact = ctx.exact.clone();
    scope.exact_hofs = ctx.exact_hofs.clone();
    scope.absence = ctx.absence.clone();
    for (p, t) in params {
        scope.vars.insert(*p, param_ctype(ctx, function, *p, t));
    }
    first_answer(body, &mut scope).unwrap_or(CType::Void)
}

/// Record each step function's parameter types, by which a launch or a push packs its arguments.
fn register_step_params(stmts: &[Stmt], steps: &stepped::Stepped, ctx: &mut CContext) {
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            is_native: false,
            ..
        } = stmt
        {
            if steps.steps().any(|f| f == name) {
                let types = params
                    .iter()
                    .map(|(pn, pt)| param_ctype(ctx, *name, *pn, pt))
                    .collect();
                ctx.task_params.insert(*name, types);
            }
        }
    }
}

/// A function that waits or is launched, as its frame and step (see `stepped`), with the per-function
/// pre-passes an ordinary function's body receives.
fn codegen_step_function(stmt: &Stmt, ctx: &mut CContext) -> (String, String) {
    let Stmt::FunctionDef {
        name,
        params,
        body,
        return_type,
        ..
    } = stmt
    else {
        return (String::new(), String::new());
    };
    let param_types: Vec<(Symbol, CType)> = params
        .iter()
        .map(|(pn, pt)| (*pn, param_ctype(ctx, *name, *pn, pt)))
        .collect();
    let bigint_vars = body_exact_locals(ctx, *name, body, &param_types);
    let returns = ctx.funcs.get(name).cloned().unwrap_or_else(|| {
        return_type.map_or(CType::Void, |rt| {
            resolve_type_expr_with_registry(rt, ctx.interner, Some(ctx.registry))
        })
    });
    let saved_big = std::mem::replace(&mut ctx.bigint_vars, bigint_vars);
    let saved_vars = ctx.vars.clone();
    for (pn, pt) in &param_types {
        ctx.vars.insert(*pn, pt.clone());
    }
    let numbers = numbers::number_locals(body, ctx);
    ctx.vars = saved_vars;
    let saved_numbers = std::mem::replace(&mut ctx.number_locals, numbers);
    let saved_returns = std::mem::replace(&mut ctx.current_fn_returns, returns.clone());
    let saved_nonnegative = std::mem::replace(
        &mut ctx.nonnegative_vars,
        crate::codegen::expr::collect_always_nonneg(body),
    );
    let cname = escape_c_ident(ctx.interner.resolve(*name));
    let out = ctx.in_body(false, |ctx| {
        stepped::emit_step(&cname, &param_types, returns, body, ctx)
    });
    ctx.bigint_vars = saved_big;
    ctx.number_locals = saved_numbers;
    ctx.current_fn_returns = saved_returns;
    ctx.nonnegative_vars = saved_nonnegative;
    out
}
