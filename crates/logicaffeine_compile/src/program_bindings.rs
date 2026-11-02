//! Program bindings on the tiers that compile a program ahead of time.
//!
//! A `## To` function reads and writes the bindings the program's root declares: `Set counter to
//! counter + 1` inside `## To bump:` moves the `counter` that `Let mutable counter be 0.` made. The
//! tree-walker keeps them in one table (`Interpreter::globals`) and the VM as program globals. A tier
//! that compiles `## Main` into a `main` function has nowhere for them but that function's locals,
//! which no other function can name.
//!
//! The rule for what a name means is the tree-walker's (`define_map`, `lookup`, `push_frame`):
//! - a `Let` at the program's root defines a program binding; a `Let` inside a block of `## Main`
//!   is that block's, and no function sees it;
//! - a function's frame holds its parameters and its own `Let`s, block by block, and then the
//!   program's bindings — never its caller's locals;
//! - `Set x to v` writes the innermost binding of `x` in scope, else the program's.
//!
//! The lift makes the program bindings some function reaches into the fields of ONE record, handed
//! to every function that reaches them — directly or through a call — as a `mutable` parameter: the
//! language's own in-out convention. Every later pass then sees a field of a lent record, which a
//! call may move, and never a local that moves behind its back. A program in which no function
//! reaches a program binding is returned unchanged, statement for statement.

use std::collections::{HashMap, HashSet};

use crate::analysis::unify::InferType;
use crate::analysis::{FieldDef, FieldType, TypeDef, TypeRegistry};
use crate::ast::stmt::{
    ClosureBody, DomQuery, Expr, MatchArm, Pattern, ReadSource, SelectBranch, Stmt, StringPart,
    TypeExpr,
};
use crate::ast_map::{self, Arenas, AstMap};
use crate::intern::{Interner, Symbol};

/// The record the lifted program bindings are fields of.
pub const PROGRAM_RECORD: &str = "__Program";

/// The binding that holds it in `## Main`, and the first parameter of every function that reaches it.
pub const PROGRAM_BINDING: &str = "__program";

/// A program whose bindings are fields of [`PROGRAM_RECORD`], with that record registered.
pub struct Lifted<'a> {
    pub stmts: Vec<Stmt<'a>>,
    pub registry: TypeRegistry,
}

/// Lift the program bindings that functions reach into one lent record (see the module docs).
/// `Ok(None)` when no function reaches one: the program needs nothing. `Err` names what this tier
/// cannot express, in the author's terms.
pub fn lift_program_bindings<'a>(
    stmts: &[Stmt<'a>],
    registry: &TypeRegistry,
    interner: &mut Interner,
    arenas: Arenas<'a>,
) -> Result<Option<Lifted<'a>>, String> {
    // The functions this tier compiles (`callgraph::aot_compiled_functions`): nothing that needs a
    // document, so neither the browser half of a page nor the bindings only it reaches (its listener
    // tables, its instance state) are anything to lift.
    let functions: HashSet<Symbol> =
        crate::analysis::callgraph::aot_compiled_functions(stmts, interner);

    let mut resolver = Resolver::new(&functions);
    for (at, s) in stmts.iter().enumerate() {
        if !matches!(s, Stmt::FunctionDef { .. }) {
            resolver.root_statement(at, s);
        }
    }
    for s in stmts {
        if let Stmt::FunctionDef {
            name, params, body, ..
        } = s
        {
            if functions.contains(name) {
                resolver.function(*name, params.iter().map(|(p, _)| *p), body);
            }
        }
    }

    let lifted: HashSet<Symbol> = resolver.reached.values().flatten().copied().collect();
    if lifted.is_empty() {
        return Ok(None);
    }
    let needs = resolver.needs();
    let reach = resolver.reach_through_calls();
    resolver.refusals(&lifted, &needs, &reach, interner)?;

    let root_types = match crate::analysis::root_binding_types(stmts, interner, registry) {
        Ok(types) => types,
        // The program does not check; the compile reports that where it checks the program.
        Err(_) => return Ok(None),
    };
    let mut order: Vec<Symbol> = lifted.iter().copied().collect();
    order.sort_by(|a, b| interner.resolve(*a).cmp(interner.resolve(*b)));
    let mut fields = Vec::with_capacity(order.len());
    for name in &order {
        let written = root_types.get(name).and_then(|t| field_type(t, interner));
        let Some(ty) = written else {
            return Err(format!(
                "`{}` is one of the program's bindings, and a function reads it, so this tier keeps it \
                 where every function can reach it — which needs its type. Its `Let` does not decide \
                 one; write it: `Let {}: <Type> be …`.",
                interner.resolve(*name),
                interner.resolve(*name)
            ));
        };
        fields.push(FieldDef {
            name: *name,
            ty,
            is_public: true,
        });
    }
    let record = interner.intern(PROGRAM_RECORD);
    let program = interner.intern(PROGRAM_BINDING);
    let mut out_registry = registry.clone();
    out_registry.register(
        record,
        TypeDef::Struct {
            fields,
            generics: Vec::new(),
            is_portable: false,
            is_shared: false,
        },
    );

    let named: &'a TypeExpr<'a> = arenas.types.alloc(TypeExpr::Named(record));
    let lent: &'a TypeExpr<'a> = arenas.types.alloc(TypeExpr::Mutable { inner: named });
    let mut lift = Lift {
        arenas,
        reads: &resolver.reads,
        sets: &resolver.sets,
        lifted: &lifted,
        needs: &needs,
        program,
        lent,
    };
    let fresh = arenas.exprs.alloc(Expr::New {
        type_name: record,
        type_args: Vec::new(),
        init_fields: Vec::new(),
    });
    let mut out = Vec::with_capacity(stmts.len() + 1);
    out.push(Stmt::Let {
        var: program,
        ty: Some(named),
        value: fresh,
        mutable: true,
    });
    for s in stmts {
        match s {
            Stmt::Let { var, value, .. } if lifted.contains(var) => {
                let value = lift.map_expr(value);
                out.push(Stmt::SetField {
                    object: lift.program_ref(),
                    field: *var,
                    value,
                });
            }
            _ => out.push(lift.map_stmt(s)),
        }
    }
    Ok(Some(Lifted {
        stmts: out,
        registry: out_registry,
    }))
}

/// A checked type as a record field's declared type; `None` for one a field cannot be declared as.
fn field_type(t: &InferType, interner: &mut Interner) -> Option<FieldType> {
    let generic =
        |base: &str, params: Vec<FieldType>, interner: &mut Interner| FieldType::Generic {
            base: interner.intern(base),
            params,
        };
    Some(match t {
        InferType::Seq(item) => {
            let item = field_type(item, interner)?;
            generic("Seq", vec![item], interner)
        }
        InferType::Set(item) => {
            let item = field_type(item, interner)?;
            generic("Set", vec![item], interner)
        }
        InferType::Option(item) => {
            let item = field_type(item, interner)?;
            generic("Option", vec![item], interner)
        }
        InferType::Map(k, v) => {
            let (k, v) = (field_type(k, interner)?, field_type(v, interner)?);
            generic("Map", vec![k, v], interner)
        }
        InferType::UserDefined(name, args) if args.is_empty() => FieldType::Named(*name),
        InferType::UserDefined(name, args) => {
            let params = args
                .iter()
                .map(|a| field_type(a, interner))
                .collect::<Option<Vec<_>>>()?;
            FieldType::Generic {
                base: *name,
                params,
            }
        }
        other => FieldType::Primitive(interner.intern(other.primitive_name()?)),
    })
}

/// Which name each identifier and each named write in the program means, by the tree-walker's rule.
struct Resolver<'a, 'f> {
    functions: &'f HashSet<Symbol>,
    /// Every name the program's root has defined so far — all of them, once the root is read.
    roots: HashSet<Symbol>,
    /// Where each program binding is first defined: the index of its root statement.
    defined_at: HashMap<Symbol, usize>,
    /// The frame being read, innermost block last; empty at the program's root.
    scopes: Vec<HashSet<Symbol>>,
    /// The function being read; `None` in `## Main`.
    current: Option<Symbol>,
    /// The root statement being read.
    root_at: usize,
    /// Each identifier that names a program binding, by node.
    reads: HashSet<*const Expr<'a>>,
    /// Each `Set` whose target is a program binding, by node.
    sets: HashSet<*const Stmt<'a>>,
    /// Each function's program bindings, named in its own body.
    reached: HashMap<Symbol, HashSet<Symbol>>,
    /// Each function's calls.
    calls: HashMap<Symbol, HashSet<Symbol>>,
    /// The functions each root statement calls (a `Serve` handler included).
    root_calls: HashMap<usize, HashSet<Symbol>>,
    /// A function named other than by a call: `(function, how, where)`.
    as_values: Vec<(Symbol, &'static str, Option<Symbol>)>,
    /// A program binding written or read by a form that names it rather than holding an
    /// expression: `(binding, form, where)`.
    named_slots: Vec<(Symbol, &'static str, Option<Symbol>)>,
}

impl<'a, 'f> Resolver<'a, 'f> {
    fn new(functions: &'f HashSet<Symbol>) -> Self {
        Self {
            functions,
            roots: HashSet::new(),
            defined_at: HashMap::new(),
            scopes: Vec::new(),
            current: None,
            root_at: 0,
            reads: HashSet::new(),
            sets: HashSet::new(),
            reached: HashMap::new(),
            calls: HashMap::new(),
            root_calls: HashMap::new(),
            as_values: Vec::new(),
            named_slots: Vec::new(),
        }
    }

    fn root_statement(&mut self, at: usize, s: &Stmt<'a>) {
        self.current = None;
        self.root_at = at;
        self.stmt(s);
    }

    fn function(
        &mut self,
        name: Symbol,
        params: impl Iterator<Item = Symbol>,
        body: &'a [Stmt<'a>],
    ) {
        self.current = Some(name);
        self.scopes = vec![params.collect()];
        self.stmts(body);
        self.scopes.clear();
        self.current = None;
    }

    fn is_local(&self, name: Symbol) -> bool {
        self.scopes.iter().any(|scope| scope.contains(&name))
    }

    /// `name` defined where the reader stands: a local of the innermost block, or at the root a
    /// program binding.
    fn define(&mut self, name: Symbol) {
        match self.scopes.last_mut() {
            Some(scope) => {
                scope.insert(name);
            }
            None => {
                self.roots.insert(name);
                self.defined_at.entry(name).or_insert(self.root_at);
            }
        }
    }

    /// A program binding the current reader names.
    fn reach(&mut self, name: Symbol) {
        if let Some(f) = self.current {
            self.reached.entry(f).or_default().insert(name);
        }
    }

    fn names_program_binding(&self, name: Symbol) -> bool {
        !self.is_local(name) && self.roots.contains(&name)
    }

    fn read(&mut self, e: &'a Expr<'a>, name: Symbol) {
        if self.names_program_binding(name) {
            self.reads.insert(e as *const _);
            self.reach(name);
        } else if !self.is_local(name) && self.functions.contains(&name) {
            self.as_values
                .push((name, "passed as a value", self.current));
        }
    }

    fn call(&mut self, function: Symbol) {
        if self.is_local(function) || !self.functions.contains(&function) {
            return;
        }
        match self.current {
            Some(f) => {
                self.calls.entry(f).or_default().insert(function);
            }
            None => {
                self.root_calls
                    .entry(self.root_at)
                    .or_default()
                    .insert(function);
            }
        }
    }

    /// A form that writes a binding by name and holds no expression for it.
    fn named_write(&mut self, name: Symbol, form: &'static str) {
        if self.names_program_binding(name) {
            self.named_slots.push((name, form, self.current));
            self.reach(name);
        }
    }

    /// A form that defines a binding by name: a local inside a function or a block, a program
    /// binding at the root — one this tier can hold only if no function reaches it.
    fn named_define(&mut self, name: Symbol, form: &'static str) {
        if self.scopes.is_empty() {
            self.named_slots.push((name, form, None));
        }
        self.define(name);
    }

    fn stmts(&mut self, block: &'a [Stmt<'a>]) {
        for s in block {
            self.stmt(s);
        }
    }

    fn scoped(&mut self, binds: impl IntoIterator<Item = Symbol>, block: &'a [Stmt<'a>]) {
        self.scopes.push(binds.into_iter().collect());
        self.stmts(block);
        self.scopes.pop();
    }

    fn arm(&mut self, arm: &MatchArm<'a>) {
        self.scoped(arm.bindings.iter().map(|(_, b)| *b), arm.body);
    }

    fn exprs(&mut self, exprs: &[&'a Expr<'a>]) {
        for e in exprs {
            self.expr(e);
        }
    }

    fn expr(&mut self, e: &'a Expr<'a>) {
        match e {
            Expr::Identifier(name) => self.read(e, *name),
            Expr::Literal(_) | Expr::Escape { .. } | Expr::OptionNone | Expr::View(_) => {}
            Expr::BinaryOp { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Not { operand } => self.expr(operand),
            Expr::Call { function, args } => {
                self.call(*function);
                self.exprs(args);
            }
            Expr::Index { collection, index } => {
                self.expr(collection);
                self.expr(index);
            }
            Expr::Slice {
                collection,
                start,
                end,
            } => {
                self.expr(collection);
                self.expr(start);
                self.expr(end);
            }
            Expr::Copy { expr } => self.expr(expr),
            Expr::Give { value } => self.expr(value),
            Expr::Length { collection } => self.expr(collection),
            Expr::Contains { collection, value } => {
                self.expr(collection);
                self.expr(value);
            }
            Expr::Union { left, right } | Expr::Intersection { left, right } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::ManifestOf { zone } => self.expr(zone),
            Expr::ChunkAt { index, zone } => {
                self.expr(index);
                self.expr(zone);
            }
            Expr::List(items) | Expr::Tuple(items) => self.exprs(items),
            Expr::Range { start, end } => {
                self.expr(start);
                self.expr(end);
            }
            Expr::FieldAccess { object, .. } => self.expr(object),
            Expr::New { init_fields, .. } => {
                for (_, value) in init_fields {
                    self.expr(value);
                }
            }
            Expr::NewVariant { fields, .. } => {
                for (_, value) in fields {
                    self.expr(value);
                }
            }
            Expr::OptionSome { value } => self.expr(value),
            Expr::WithCapacity { value, capacity } => {
                self.expr(value);
                self.expr(capacity);
            }
            Expr::Closure { params, body, .. } => {
                self.scopes.push(params.iter().map(|(p, _)| *p).collect());
                match body {
                    ClosureBody::Expression(x) => self.expr(x),
                    ClosureBody::Block(b) => self.stmts(b),
                }
                self.scopes.pop();
            }
            Expr::CallExpr { callee, args } => {
                match callee {
                    Expr::Identifier(f) if !self.is_local(*f) && self.functions.contains(f) => {
                        self.call(*f)
                    }
                    _ => self.expr(callee),
                }
                self.exprs(args);
            }
            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let StringPart::Expr { value, .. } = part {
                        self.expr(value);
                    }
                }
            }
            Expr::Dom(q) => match q {
                DomQuery::Text { selector }
                | DomQuery::Count { selector }
                | DomQuery::Value { selector }
                | DomQuery::Visible { selector }
                | DomQuery::Hidden { selector }
                | DomQuery::Disabled { selector }
                | DomQuery::Checked { selector }
                | DomQuery::Focused { selector } => self.expr(selector),
                DomQuery::Journal
                | DomQuery::JournalShows { .. }
                | DomQuery::Title
                | DomQuery::Url => {}
            },
        }
    }

    fn stmt(&mut self, s: &Stmt<'a>) {
        match s {
            Stmt::Let { var, value, .. } => {
                self.expr(value);
                self.define(*var);
            }
            Stmt::Set { target, value } => {
                self.expr(value);
                if self.names_program_binding(*target) {
                    self.sets.insert(s as *const _);
                    self.reach(*target);
                }
            }
            Stmt::Call { function, args } => {
                self.call(*function);
                self.exprs(args);
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                self.expr(cond);
                self.scoped([], then_block);
                if let Some(b) = else_block {
                    self.scoped([], b);
                }
            }
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                self.expr(cond);
                if let Some(d) = decreasing {
                    self.expr(d);
                }
                self.scoped([], body);
            }
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                self.expr(iterable);
                let binds = match pattern {
                    Pattern::Identifier(x) => vec![*x],
                    Pattern::Tuple(xs) => xs.clone(),
                };
                self.scoped(binds, body);
            }
            Stmt::Return { value } => {
                if let Some(v) = value {
                    self.expr(v);
                }
            }
            Stmt::RuntimeAssert {
                condition, reason, ..
            } => {
                self.expr(condition);
                if let Some(r) = reason {
                    self.expr(r);
                }
            }
            Stmt::Fail { reason } => self.expr(reason),
            Stmt::Give { object, recipient } => {
                self.expr(object);
                self.expr(recipient);
            }
            Stmt::Show { object, recipient } => {
                self.expr(object);
                match recipient {
                    Expr::Identifier(f) if !self.is_local(*f) && self.functions.contains(f) => {
                        self.call(*f)
                    }
                    _ => self.expr(recipient),
                }
            }
            Stmt::SetField { object, value, .. } => {
                self.expr(object);
                self.expr(value);
            }
            Stmt::FunctionDef {
                name, params, body, ..
            } => {
                let (outer_scopes, outer) = (std::mem::take(&mut self.scopes), self.current);
                self.function(*name, params.iter().map(|(p, _)| *p), body);
                self.scopes = outer_scopes;
                self.current = outer;
            }
            // `## Test` bodies run on the interpreter tier only; a compiled program never holds one.
            Stmt::TestDef { .. } => {}
            Stmt::Expect { condition } => self.expr(condition),
            Stmt::ExpectFail { body, .. } => self.scoped([], body),
            Stmt::AdvanceTime { millis } => self.expr(millis),
            Stmt::Observe { payloads, .. } => {
                for (_, value) in payloads {
                    self.expr(value);
                }
            }
            Stmt::During { body, .. } => self.scoped([], body),
            Stmt::ForAll {
                vars,
                constraint,
                body,
                ..
            } => {
                self.scopes.push(vars.iter().copied().collect());
                if let Some(c) = constraint {
                    self.expr(c);
                }
                self.stmts(body);
                self.scopes.pop();
            }
            Stmt::Inspect { target, arms, .. } => {
                self.expr(target);
                for arm in arms {
                    self.arm(arm);
                }
            }
            Stmt::Push { value, collection }
            | Stmt::Add { value, collection }
            | Stmt::Remove { value, collection } => {
                self.expr(value);
                self.expr(collection);
            }
            Stmt::Pop { collection, into } => {
                self.expr(collection);
                if let Some(x) = into {
                    self.named_define(*x, "`Pop … into`");
                }
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                self.expr(collection);
                self.expr(index);
                self.expr(value);
            }
            // A splice's statements stand in the block that holds it.
            Stmt::Splice { body } => self.stmts(body),
            Stmt::Reversibly { body } => self.scoped([], body),
            Stmt::Zone { body, .. } => self.scoped([], body),
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => self.scoped([], tasks),
            Stmt::ReadFrom { var, source } => {
                if let ReadSource::File(path) = source {
                    self.expr(path);
                }
                self.named_define(*var, "`Read … into`");
            }
            Stmt::WriteFile { content, path } => {
                self.expr(content);
                self.expr(path);
            }
            Stmt::SendMessage {
                message,
                destination,
                ..
            } => {
                self.expr(message);
                self.expr(destination);
            }
            Stmt::AwaitMessage { source, into, .. } => {
                self.expr(source);
                self.named_define(*into, "`Await … into`");
            }
            Stmt::StreamMessage {
                values,
                destination,
            } => {
                self.expr(values);
                self.expr(destination);
            }
            Stmt::MergeCrdt { source, target } | Stmt::JoinCrdt { source, target } => {
                self.expr(source);
                self.expr(target);
            }
            Stmt::IncreaseCrdt { object, amount, .. }
            | Stmt::DecreaseCrdt { object, amount, .. } => {
                self.expr(object);
                self.expr(amount);
            }
            Stmt::AppendToSequence { sequence, value } => {
                self.expr(sequence);
                self.expr(value);
            }
            Stmt::ResolveConflict { object, value, .. } => {
                self.expr(object);
                self.expr(value);
            }
            Stmt::Listen { address, secure } | Stmt::ConnectTo { address, secure } => {
                self.expr(address);
                if let Some(pad) = secure {
                    self.expr(pad.pad);
                }
            }
            Stmt::Serve { port, handler } => {
                self.expr(port);
                match handler {
                    Expr::Identifier(h) if !self.is_local(*h) && self.functions.contains(h) => {
                        self.call(*h)
                    }
                    _ => self.expr(handler),
                }
            }
            Stmt::LetPeerAgent { var, address } => {
                self.expr(address);
                self.named_define(*var, "`Let … be a peer agent`");
            }
            Stmt::Sleep { milliseconds } => self.expr(milliseconds),
            Stmt::Sync { var, topic } => {
                self.expr(topic);
                self.named_write(*var, "`Sync`");
            }
            // A page's request; a compiled program refuses it where it stands.
            Stmt::Fetch { url, body, .. } => {
                self.expr(url);
                if let Some(b) = body {
                    self.expr(b);
                }
            }
            Stmt::GivenFetch {
                url, status, body, ..
            } => {
                self.expr(url);
                self.expr(status);
                self.expr(body);
            }
            // The browser surface: lowered to functions before this pass, or refused by the tier.
            Stmt::DirectiveDef { .. } | Stmt::MiddlewareDef { .. } => {}
            Stmt::Mount { var, path } => {
                self.expr(path);
                self.named_define(*var, "`Mount … at`");
            }
            Stmt::LaunchTask { function, args } => {
                self.call(*function);
                if self.functions.contains(function) && !self.is_local(*function) {
                    self.as_values
                        .push((*function, "launched as a task", self.current));
                }
                self.exprs(args);
            }
            Stmt::LaunchTaskWithHandle {
                handle,
                function,
                args,
            } => {
                self.call(*function);
                if self.functions.contains(function) && !self.is_local(*function) {
                    self.as_values
                        .push((*function, "launched as a task", self.current));
                }
                self.exprs(args);
                self.named_define(*handle, "`Launch a task … with handle`");
            }
            Stmt::SendPipe { value, pipe } => {
                self.expr(value);
                self.expr(pipe);
            }
            Stmt::ReceivePipe { var, pipe } | Stmt::TryReceivePipe { var, pipe } => {
                self.expr(pipe);
                self.named_define(*var, "`Receive … from`");
            }
            Stmt::TrySendPipe {
                value,
                pipe,
                result,
            } => {
                self.expr(value);
                self.expr(pipe);
                if let Some(r) = result {
                    self.named_define(*r, "`Try to send … into`");
                }
            }
            Stmt::StopTask { handle } => self.expr(handle),
            Stmt::CloseChannel { channel } => self.expr(channel),
            Stmt::AwaitTask { handle, result } => {
                self.expr(handle);
                if let Some(r) = result {
                    self.named_define(*r, "`Await … into`");
                }
            }
            Stmt::Select { branches } => {
                for branch in branches {
                    match branch {
                        SelectBranch::Receive { var, pipe, body } => {
                            self.expr(pipe);
                            self.scoped([*var], body);
                        }
                        SelectBranch::Timeout { milliseconds, body } => {
                            self.expr(milliseconds);
                            self.scoped([], body);
                        }
                    }
                }
            }
            Stmt::Swap { a, b } => {
                self.named_write(*a, "`Swap`");
                self.named_write(*b, "`Swap`");
            }
            Stmt::CreatePipe { var, .. } => self.named_define(*var, "`Let … be a new Pipe`"),
            Stmt::Spawn { name, .. } => self.named_define(*name, "`Spawn`"),
            Stmt::Check { subject, .. } => self.named_write(*subject, "`Check`"),
            Stmt::Break
            | Stmt::StructDef { .. }
            | Stmt::ExpectOutput { .. }
            | Stmt::GivenFile { .. }
            | Stmt::Replay { .. }
            | Stmt::ServeFetches
            | Stmt::RedirectDef { .. }
            | Stmt::Escape { .. }
            | Stmt::Require { .. }
            | Stmt::Site(_) => {}
            Stmt::Assert { .. }
            | Stmt::Trust { .. }
            | Stmt::Theorem(_)
            | Stmt::Definition(_)
            | Stmt::Axiom(_)
            | Stmt::Theory(_) => {}
            Stmt::Ui(_) | Stmt::ComponentDef(_) | Stmt::RouteDef { .. } => {}
        }
    }

    /// The functions that reach a program binding, directly or through a call.
    fn needs(&self) -> HashSet<Symbol> {
        let mut needs: HashSet<Symbol> = self
            .reached
            .iter()
            .filter(|(_, names)| !names.is_empty())
            .map(|(f, _)| *f)
            .collect();
        loop {
            let more: Vec<Symbol> = self
                .calls
                .iter()
                .filter(|(f, callees)| {
                    !needs.contains(*f) && callees.iter().any(|c| needs.contains(c))
                })
                .map(|(f, _)| *f)
                .collect();
            if more.is_empty() {
                return needs;
            }
            needs.extend(more);
        }
    }

    /// Every program binding each function reaches, through the functions it calls too.
    fn reach_through_calls(&self) -> HashMap<Symbol, HashSet<Symbol>> {
        let mut reach: HashMap<Symbol, HashSet<Symbol>> = self.reached.clone();
        loop {
            let mut grew = false;
            for (f, callees) in &self.calls {
                let inherited: HashSet<Symbol> = callees
                    .iter()
                    .filter_map(|c| reach.get(c))
                    .flatten()
                    .copied()
                    .collect();
                let own = reach.entry(*f).or_default();
                let before = own.len();
                own.extend(inherited);
                grew |= own.len() != before;
            }
            if !grew {
                return reach;
            }
        }
    }

    /// What this tier cannot hold once the lifted bindings live in a lent record.
    fn refusals(
        &self,
        lifted: &HashSet<Symbol>,
        needs: &HashSet<Symbol>,
        reach: &HashMap<Symbol, HashSet<Symbol>>,
        interner: &Interner,
    ) -> Result<(), String> {
        let name = |s: Symbol| interner.resolve(s).to_string();
        let within = |w: Option<Symbol>| match w {
            Some(f) => format!("in `{}`", interner.resolve(f)),
            None => "in `## Main`".to_string(),
        };
        let reached_of = |f: Symbol| -> String {
            let mut names: Vec<String> = reach
                .get(&f)
                .into_iter()
                .flatten()
                .filter(|x| lifted.contains(x))
                .map(|x| name(*x))
                .collect();
            names.sort();
            names.join("`, `")
        };
        for (binding, form, at) in &self.named_slots {
            if lifted.contains(binding) {
                return Err(format!(
                    "{form} {} writes `{}`, one of the program's bindings that a function reads. On this \
                     tier such a binding is made with `Let {} be …` and changed with `Set {} to …`.",
                    within(*at),
                    name(*binding),
                    name(*binding),
                    name(*binding)
                ));
            }
        }
        for (function, how, at) in &self.as_values {
            if needs.contains(function) {
                return Err(format!(
                    "`{}` is {how} {}, and it reaches the program's `{}`. On this tier a function that \
                     reaches the program's bindings runs only where it is called.",
                    name(*function),
                    within(*at),
                    reached_of(*function)
                ));
            }
        }
        let mut calls: Vec<(usize, Symbol)> = self
            .root_calls
            .iter()
            .flat_map(|(at, fs)| fs.iter().map(move |f| (*at, *f)))
            .collect();
        calls.sort_by_key(|(at, f)| (*at, name(*f)));
        for (at, function) in calls {
            let mut early: Vec<Symbol> = reach
                .get(&function)
                .into_iter()
                .flatten()
                .filter(|x| lifted.contains(x) && self.defined_at.get(x).is_none_or(|d| *d >= at))
                .copied()
                .collect();
            early.sort_by_key(|x| name(*x));
            if let Some(binding) = early.first() {
                return Err(format!(
                    "`{}` reads the program's `{}`, and `## Main` calls it before `Let {} be …` has run. \
                     Declare `{}` before the first call.",
                    name(function),
                    name(*binding),
                    name(*binding),
                    name(*binding)
                ));
            }
        }
        Ok(())
    }
}

/// The rewrite: a lifted binding becomes a field of the lent record, and every function that
/// reaches one takes the record first.
struct Lift<'a, 's> {
    arenas: Arenas<'a>,
    reads: &'s HashSet<*const Expr<'a>>,
    sets: &'s HashSet<*const Stmt<'a>>,
    lifted: &'s HashSet<Symbol>,
    needs: &'s HashSet<Symbol>,
    program: Symbol,
    lent: &'a TypeExpr<'a>,
}

impl<'a, 's> Lift<'a, 's> {
    fn program_ref(&self) -> &'a Expr<'a> {
        self.arenas.exprs.alloc(Expr::Identifier(self.program))
    }

    fn lent_args(&mut self, args: &[&'a Expr<'a>]) -> Vec<&'a Expr<'a>> {
        let mut out = Vec::with_capacity(args.len() + 1);
        out.push(self.program_ref());
        out.extend(args.iter().map(|a| self.map_expr(a)));
        out
    }
}

impl<'a, 's> AstMap<'a> for Lift<'a, 's> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        match e {
            Expr::Identifier(name)
                if self.lifted.contains(name) && self.reads.contains(&(e as *const _)) =>
            {
                let object = self.program_ref();
                self.arenas.exprs.alloc(Expr::FieldAccess {
                    object,
                    field: *name,
                })
            }
            Expr::Call { function, args } if self.needs.contains(function) => {
                let args = self.lent_args(args);
                self.arenas.exprs.alloc(Expr::Call {
                    function: *function,
                    args,
                })
            }
            Expr::CallExpr {
                callee: Expr::Identifier(f),
                args,
            } if self.needs.contains(f) => {
                let args = self.lent_args(args);
                self.arenas.exprs.alloc(Expr::Call { function: *f, args })
            }
            _ => ast_map::walk_expr(self, e),
        }
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        match s {
            Stmt::Set { target, value }
                if self.lifted.contains(target) && self.sets.contains(&(s as *const _)) =>
            {
                let value = self.map_expr(value);
                Stmt::SetField {
                    object: self.program_ref(),
                    field: *target,
                    value,
                }
            }
            Stmt::Call { function, args } if self.needs.contains(function) => Stmt::Call {
                function: *function,
                args: self.lent_args(args),
            },
            Stmt::Show {
                object,
                recipient: Expr::Identifier(f),
            } if self.needs.contains(f) => Stmt::Call {
                function: *f,
                args: self.lent_args(&[*object]),
            },
            Stmt::FunctionDef {
                name,
                generics,
                params,
                body,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            } if self.needs.contains(name) => {
                let mut lent_params = Vec::with_capacity(params.len() + 1);
                lent_params.push((self.program, self.lent));
                lent_params.extend(params.iter().map(|(p, t)| (*p, self.map_type(t))));
                Stmt::FunctionDef {
                    name: *name,
                    generics: generics.clone(),
                    params: lent_params,
                    body: self.map_block(body),
                    return_type: return_type.map(|t| self.map_type(t)),
                    is_native: *is_native,
                    native_path: *native_path,
                    is_exported: *is_exported,
                    export_target: *export_target,
                    opt_flags: *opt_flags,
                }
            }
            _ => ast_map::walk_stmt(self, s),
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/program_bindings/tests.rs"]
mod tests;
