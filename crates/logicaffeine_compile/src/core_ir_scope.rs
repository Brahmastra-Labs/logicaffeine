//! The Core IR's one namespace per function, made from the language's lexical scope.
//!
//! The Core IR the specializer and the self-interpreter read (`compile::encode_program_source`, its compact form and
//! the native builder) names a variable by its spelling: the self-interpreter keeps a function's variables in one
//! map, and the specializer splices a decided branch or an unrolled loop pass into the block that holds it. The
//! language scopes them by the tree-walker's rule (stated in `program_bindings`): a block's binding hides an outer
//! binding of its name until the block ends; a function's frame holds its parameters and its body's own bindings —
//! a `Let` of a parameter's name rebinds the parameter — then the program's bindings, the ones `## Main` makes at its
//! root, and never its caller's.
//!
//! [`for_core_ir`] rewrites a program so a spelling means what the scope meant, in two passes over [`AstMap`]:
//! - every binding that would hide a visible one — an outer block's, or inside a function a program binding — gets a
//!   name no symbol of the program has, and each use and write that means it takes the new name; so does a block's
//!   binding of a name its function reads or writes where no binding of it is visible — after the block ends, or in a
//!   loop's pass before the pass binds it — so that use stays unbound and stops the program as the language's scope
//!   stops it (`Undefined variable`);
//! - a function that reads or writes a program binding, itself or through the functions it calls, takes it as a
//!   `mutable` parameter of the same name, and every call hands it the caller's: the program's own binding from
//!   `## Main`, the parameter from a function.
//!
//! A record is built whole, as the tree-walker builds it: a field its construction leaves out starts at the value
//! its declared type starts at (`interpreter::default_for_declared_type`, the language's one table) — written here
//! when an expression spells it, and otherwise (a CRDT replica, nothing) left for the encoders to hold as the Core
//! IR's `CDeclaredDefault` — and the record holds its fields in the order the language prints them, by name. A variant holds its fields in the order its
//! declaration lists them. Either way the fields are evaluated in the order the construction wrote them: when two
//! written fields could act and the orders differ, the construction becomes a closure applied to them in the written
//! order, which builds the value in the held one.
//!
//! A debug hole (`{name=}`) is labelled with the name the program spelled, so before any name is rewritten it becomes
//! that label as text (`name=`) followed by the hole.
//!
//! A program in which no binding hides another, no function reaches a program binding and every record is built with
//! all its fields comes back unchanged.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::ast::stmt::{ClosureBody, Expr, Literal, MatchArm, Pattern, SelectBranch, Stmt, StringPart, TypeExpr};
use crate::ast_map::{self, Arenas, AstMap};
use crate::intern::{Interner, Symbol};
use crate::interpreter::{default_for_declared_type, RuntimeValue};

/// What the program's declarations say about building its values.
#[derive(Clone, Default)]
pub(crate) struct TypeShapes {
    /// Each record type's declared fields, in order, with the type each is declared as.
    pub(crate) records: HashMap<Symbol, Vec<(Symbol, crate::analysis::FieldType)>>,
    /// Each variant's declared fields, in order, keyed by its enum and its own name.
    pub(crate) variants: HashMap<(Symbol, Symbol), Vec<Symbol>>,
}

/// The shapes a type registry declares.
pub(crate) fn type_shapes(registry: &crate::analysis::TypeRegistry) -> TypeShapes {
    use crate::analysis::TypeDef;
    let mut out = TypeShapes::default();
    for (name, def) in registry.iter_types() {
        match def {
            TypeDef::Struct { fields, .. } => {
                out.records.insert(*name, fields.iter().map(|f| (f.name, f.ty.clone())).collect());
            }
            TypeDef::Enum { variants, .. } => {
                for v in variants {
                    out.variants
                        .insert((*name, v.name), v.fields.iter().map(|f| f.name).collect());
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether evaluating `e` could do anything besides produce its value: anything but literals, names and the
/// structure built from them is taken to act.
fn may_act(e: &Expr) -> bool {
    match e {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone => false,
        Expr::BinaryOp { left, right, .. } => may_act(left) || may_act(right),
        Expr::Not { operand } => may_act(operand),
        Expr::Length { collection } | Expr::Copy { expr: collection } => may_act(collection),
        Expr::Index { collection, index } => may_act(collection) || may_act(index),
        Expr::FieldAccess { object, .. } => may_act(object),
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|i| may_act(i)),
        Expr::New { init_fields, .. } => init_fields.iter().any(|(_, v)| may_act(v)),
        Expr::NewVariant { fields, .. } => fields.iter().any(|(_, v)| may_act(v)),
        _ => true,
    }
}

/// `stmts` with every binding named apart from what it would hide and every function handed the program bindings it
/// reaches (see the module docs).
pub(crate) fn for_core_ir<'a>(
    stmts: &[Stmt<'a>],
    interner: &mut Interner,
    arenas: Arenas<'a>,
    shapes: &TypeShapes,
) -> Vec<Stmt<'a>> {
    let mut shapes = shapes.clone();
    for s in stmts {
        if let Stmt::StructDef { name, fields, .. } = s {
            shapes
                .records
                .entry(*name)
                .or_insert_with(|| fields.iter().map(|(f, ty, _)| (*f, crate::analysis::FieldType::Named(*ty))).collect());
        }
    }
    let functions: HashSet<Symbol> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef {
                name,
                is_native: false,
                ..
            } => Some(*name),
            _ => None,
        })
        .collect();
    let mut roots: HashMap<Symbol, Option<&'a TypeExpr<'a>>> = HashMap::new();
    for s in stmts {
        root_binding(s, &mut roots);
    }

    let mut scout = Hygiene {
        arenas,
        interner,
        roots: &roots,
        functions: &functions,
        shapes: &shapes,
        frames: vec![HashMap::new()],
        current: None,
        reached: BTreeMap::new(),
        calls: BTreeMap::new(),
        unbound: BTreeMap::new(),
        scouting: true,
    };
    for s in stmts {
        match s {
            Stmt::FunctionDef { .. } => scout.function(s),
            _ => scout.map_stmt(s),
        };
    }
    let unbound = std::mem::take(&mut scout.unbound);
    let mut hygiene = Hygiene {
        arenas,
        interner: scout.interner,
        roots: &roots,
        functions: &functions,
        shapes: &shapes,
        frames: vec![HashMap::new()],
        current: None,
        reached: BTreeMap::new(),
        calls: BTreeMap::new(),
        unbound,
        scouting: false,
    };
    let renamed: Vec<Stmt<'a>> = stmts
        .iter()
        .map(|s| match s {
            Stmt::FunctionDef { .. } => hygiene.function(s),
            _ => hygiene.map_stmt(s),
        })
        .collect();

    let needs = reach_through_calls(&hygiene.reached, &hygiene.calls);
    if needs.values().all(BTreeSet::is_empty) {
        return renamed;
    }
    let any = hygiene.interner.intern("Any");
    let mut ordered: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
    for (f, names) in &needs {
        let mut list: Vec<Symbol> = names.iter().copied().collect();
        list.sort_by(|a, b| hygiene.interner.resolve(*a).cmp(hygiene.interner.resolve(*b)));
        ordered.insert(*f, list);
    }
    let mut lift = Lift {
        arenas,
        needs: &ordered,
        roots: &roots,
        any,
    };
    renamed.iter().map(|s| lift.map_stmt(s)).collect()
}

/// The name a statement at `## Main`'s root binds, with the type its `Let` wrote.
fn root_binding<'a>(s: &Stmt<'a>, roots: &mut HashMap<Symbol, Option<&'a TypeExpr<'a>>>) {
    let bound = match s {
        Stmt::Let { var, ty, .. } => {
            roots.entry(*var).or_insert(*ty);
            return;
        }
        Stmt::Pop { into: Some(x), .. }
        | Stmt::TrySendPipe { result: Some(x), .. }
        | Stmt::AwaitTask { result: Some(x), .. } => *x,
        Stmt::ReadFrom { var, .. }
        | Stmt::LetPeerAgent { var, .. }
        | Stmt::Mount { var, .. }
        | Stmt::CreatePipe { var, .. }
        | Stmt::ReceivePipe { var, .. }
        | Stmt::TryReceivePipe { var, .. } => *var,
        Stmt::AwaitMessage { into, .. } => *into,
        Stmt::LaunchTaskWithHandle { handle, .. } => *handle,
        Stmt::Spawn { name, .. } => *name,
        _ => return,
    };
    roots.entry(bound).or_insert(None);
}

/// Each function's program bindings: those it names and those of every function it calls, to a fixpoint.
fn reach_through_calls(
    reached: &BTreeMap<Symbol, BTreeSet<Symbol>>,
    calls: &BTreeMap<Symbol, BTreeSet<Symbol>>,
) -> BTreeMap<Symbol, BTreeSet<Symbol>> {
    let mut needs = reached.clone();
    for f in calls.keys() {
        needs.entry(*f).or_default();
    }
    loop {
        let mut grew = false;
        for (f, callees) in calls {
            let mut gained: BTreeSet<Symbol> = BTreeSet::new();
            for g in callees {
                if let Some(theirs) = needs.get(g) {
                    gained.extend(theirs.iter().copied());
                }
            }
            let mine = needs.entry(*f).or_default();
            let before = mine.len();
            mine.extend(gained);
            grew |= mine.len() != before;
        }
        if !grew {
            return needs;
        }
    }
}

/// The renaming pass: scope frames innermost last; at `## Main` the first frame is the program's root, in a function
/// it holds the parameters and the body's own top level.
struct Hygiene<'a, 'r> {
    arenas: Arenas<'a>,
    interner: &'r mut Interner,
    roots: &'r HashMap<Symbol, Option<&'a TypeExpr<'a>>>,
    functions: &'r HashSet<Symbol>,
    shapes: &'r TypeShapes,
    frames: Vec<HashMap<Symbol, Symbol>>,
    current: Option<Symbol>,
    reached: BTreeMap<Symbol, BTreeSet<Symbol>>,
    calls: BTreeMap<Symbol, BTreeSet<Symbol>>,
    /// The names each function (`None`: `## Main`) uses where no binding of the name is visible.
    unbound: BTreeMap<Option<Symbol>, BTreeSet<Symbol>>,
    /// The first pass: it finds `unbound` and renames nothing.
    scouting: bool,
}

impl<'a, 'r> Hygiene<'a, 'r> {
    fn local(&self, name: Symbol) -> Option<Symbol> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(&name).copied())
    }

    /// The binding a use or write of `name` means.
    fn use_name(&mut self, name: Symbol) -> Symbol {
        if let Some(bound) = self.local(name) {
            return bound;
        }
        if let Some(f) = self.current {
            if self.roots.contains_key(&name) {
                self.reached.entry(f).or_default().insert(name);
                return name;
            }
        }
        if self.scouting {
            self.unbound.entry(self.current).or_default().insert(name);
        }
        name
    }

    /// The name a call of `function` calls: a local closure's binding, else the function.
    fn call_name(&mut self, function: Symbol) -> Symbol {
        if let Some(bound) = self.local(function) {
            return bound;
        }
        if let Some(f) = self.current {
            if self.functions.contains(&function) {
                self.calls.entry(f).or_default().insert(function);
            }
        }
        function
    }

    /// A binding of `name` in the innermost frame: the frame's own binding again when it has one, a fresh name when it
    /// would hide an outer frame's binding or, inside a function, a program binding, or when it is a nested block's and
    /// its function uses the name where no binding of it is visible.
    fn bind(&mut self, name: Symbol) -> Symbol {
        let innermost = self.frames.len() - 1;
        if let Some(own) = self.frames[innermost].get(&name) {
            return *own;
        }
        let hides = self.frames[..innermost]
            .iter()
            .any(|frame| frame.contains_key(&name))
            || (self.current.is_some() && self.roots.contains_key(&name));
        let read_unbound = innermost > 0
            && self
                .unbound
                .get(&self.current)
                .is_some_and(|names| names.contains(&name));
        let bound = if !self.scouting && (hides || read_unbound) {
            self.fresh(name)
        } else {
            name
        };
        self.frames[innermost].insert(name, bound);
        bound
    }

    fn fresh(&mut self, name: Symbol) -> Symbol {
        let base = self.interner.resolve(name).to_string();
        let mut n = 1usize;
        loop {
            let candidate = format!("{base}__s{n}");
            if self.interner.lookup(&candidate).is_none() {
                return self.interner.intern(&candidate);
            }
            n += 1;
        }
    }

    /// `block` in a new frame that first binds `binders`; returns their names and the mapped block.
    fn scoped(&mut self, binders: &[Symbol], block: &'a [Stmt<'a>]) -> (Vec<Symbol>, &'a [Stmt<'a>]) {
        self.frames.push(HashMap::new());
        let names: Vec<Symbol> = binders.iter().map(|b| self.bind(*b)).collect();
        let mapped = self.in_frame(block);
        self.frames.pop();
        (names, mapped)
    }

    /// `block`'s statements in the current frame.
    fn in_frame(&mut self, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        let mapped: Vec<Stmt<'a>> = block.iter().map(|s| self.map_stmt(s)).collect();
        self.arenas.stmts.alloc_slice(mapped)
    }

    /// A function definition: one frame for its parameters and its body's top level.
    fn function(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        let Stmt::FunctionDef {
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
        } = s
        else {
            return self.map_stmt(s);
        };
        let saved = std::mem::replace(&mut self.frames, vec![HashMap::new()]);
        let saved_current = self.current.replace(*name);
        let params: Vec<(Symbol, &'a TypeExpr<'a>)> =
            params.iter().map(|(p, t)| (self.bind(*p), *t)).collect();
        let body = self.in_frame(body);
        self.frames = saved;
        self.current = saved_current;
        Stmt::FunctionDef {
            name: *name,
            generics: generics.clone(),
            params,
            body,
            return_type: *return_type,
            is_native: *is_native,
            native_path: *native_path,
            is_exported: *is_exported,
            export_target: *export_target,
            opt_flags: *opt_flags,
        }
    }

    /// The expression a field declared as `ty` starts at, when an expression spells its type's default.
    fn default_of(&mut self, ty: &crate::analysis::FieldType) -> Option<&'a Expr<'a>> {
        let name = self.interner.resolve(crate::interpreter::declared_field_type(ty)).to_string();
        let start = match default_for_declared_type(&name) {
            RuntimeValue::Int(n) => Expr::Literal(Literal::Number(n)),
            RuntimeValue::Float(f) => Expr::Literal(Literal::Float(f)),
            RuntimeValue::Bool(b) => Expr::Literal(Literal::Boolean(b)),
            RuntimeValue::Char(c) => Expr::Literal(Literal::Char(c)),
            RuntimeValue::Text(t) => Expr::Literal(Literal::Text(self.interner.intern(&t))),
            RuntimeValue::List(_) => self.empty("Seq", ty),
            RuntimeValue::Set(_) => self.empty("Set", ty),
            RuntimeValue::Map(_) => self.empty("Map", ty),
            _ => return None,
        };
        Some(self.arenas.exprs.alloc(start))
    }

    /// An empty `collection` of the element types `declared` names, as `a new Seq of Text` spells them.
    fn empty(&mut self, collection: &str, declared: &crate::analysis::FieldType) -> Expr<'a> {
        use crate::analysis::FieldType;
        let type_args = match declared {
            FieldType::Generic { params, .. } => {
                params.iter().map(|p| self.type_expr(p)).collect::<Option<Vec<_>>>().unwrap_or_default()
            }
            _ => Vec::new(),
        };
        Expr::New {
            type_name: self.interner.intern(collection),
            type_args,
            init_fields: Vec::new(),
        }
    }

    /// The type expression a declared field type spells, for every type but a named tuple's (its field names have
    /// no arena here).
    fn type_expr(&mut self, ty: &crate::analysis::FieldType) -> Option<TypeExpr<'a>> {
        use crate::analysis::FieldType;
        Some(match ty {
            FieldType::Primitive(s) => TypeExpr::Primitive(*s),
            FieldType::Named(s) | FieldType::TypeParam(s) => TypeExpr::Named(*s),
            FieldType::Generic { base, params } => {
                let params = params.iter().map(|p| self.type_expr(p)).collect::<Option<Vec<_>>>()?;
                TypeExpr::Generic { base: *base, params: self.arenas.types.alloc_slice(params) }
            }
            FieldType::Tuple(items) => {
                let items = items.iter().map(|p| self.type_expr(p)).collect::<Option<Vec<_>>>()?;
                TypeExpr::Tuple(self.arenas.types.alloc_slice(items))
            }
            FieldType::Record(_) => return None,
        })
    }

    /// A construction whose fields are written in one order and held in another (`held`, keyed in `order`): built
    /// directly when the written order is the held one or at most one written field could act; otherwise a closure
    /// applied to the written fields in their order, building the value in the held one.
    fn ordered_construction(
        &mut self,
        written: &[(Symbol, &'a Expr<'a>)],
        held: &[(Symbol, &'a Expr<'a>)],
        order: &[Symbol],
        build: impl FnOnce(Vec<(Symbol, &'a Expr<'a>)>) -> Expr<'a>,
    ) -> &'a Expr<'a> {
        let reordered = written
            .iter()
            .map(|(f, _)| *f)
            .ne(held.iter().map(|(f, _)| *f).filter(|f| written.iter().any(|(w, _)| w == f)));
        if !reordered || written.iter().filter(|(_, v)| may_act(v)).count() < 2 {
            return self.arenas.exprs.alloc(build(held.to_vec()));
        }
        let any = self.interner.intern("Any");
        let any_type: &'a TypeExpr<'a> = self.arenas.types.alloc(TypeExpr::Named(any));
        let mut params: Vec<(Symbol, &'a TypeExpr<'a>)> = Vec::with_capacity(written.len());
        let mut passed: Vec<(Symbol, Symbol)> = Vec::with_capacity(written.len());
        for (field, _) in written {
            let param = self.fresh(*field);
            params.push((param, any_type));
            passed.push((*field, param));
        }
        let fields: Vec<(Symbol, &'a Expr<'a>)> = order
            .iter()
            .filter_map(|f| {
                if let Some((_, param)) = passed.iter().find(|(w, _)| w == f) {
                    Some((*f, &*self.arenas.exprs.alloc(Expr::Identifier(*param))))
                } else {
                    held.iter().find(|(h, _)| h == f).copied()
                }
            })
            .collect();
        let built = self.arenas.exprs.alloc(build(fields));
        let callee = self.arenas.exprs.alloc(Expr::Closure {
            params,
            body: ClosureBody::Expression(built),
            return_type: None,
        });
        self.arenas.exprs.alloc(Expr::CallExpr {
            callee,
            args: written.iter().map(|(_, v)| *v).collect(),
        })
    }

    fn arm(&mut self, arm: &MatchArm<'a>) -> MatchArm<'a> {
        let binders: Vec<Symbol> = arm.bindings.iter().map(|(_, b)| *b).collect();
        let (names, body) = self.scoped(&binders, arm.body);
        MatchArm {
            enum_name: arm.enum_name,
            variant: arm.variant,
            bindings: arm
                .bindings
                .iter()
                .zip(names)
                .map(|((field, _), name)| (*field, name))
                .collect(),
            body,
        }
    }
}

impl<'a, 'r> AstMap<'a> for Hygiene<'a, 'r> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_block(&mut self, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        self.scoped(&[], block).1
    }

    fn map_arm(&mut self, arm: &MatchArm<'a>) -> MatchArm<'a> {
        self.arm(arm)
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        match e {
            Expr::InterpolatedString(parts) if parts.iter().any(|p| matches!(p, StringPart::Expr { debug: true, .. })) => {
                let mut held = Vec::with_capacity(parts.len() + 1);
                for part in parts {
                    match part {
                        StringPart::Literal(text) => held.push(StringPart::Literal(*text)),
                        StringPart::Expr { value, format_spec, debug } => {
                            if *debug {
                                let label = match value {
                                    Expr::Identifier(name) => self.interner.resolve(*name).to_string(),
                                    _ => "expr".to_string(),
                                };
                                held.push(StringPart::Literal(self.interner.intern(&format!("{label}="))));
                            }
                            held.push(StringPart::Expr { value: self.map_expr(value), format_spec: *format_spec, debug: false });
                        }
                    }
                }
                self.arenas.exprs.alloc(Expr::InterpolatedString(held))
            }
            Expr::Identifier(name) => {
                let bound = self.use_name(*name);
                if bound == *name {
                    e
                } else {
                    self.arenas.exprs.alloc(Expr::Identifier(bound))
                }
            }
            Expr::Call { function, args } => {
                let function = self.call_name(*function);
                let args = args.iter().map(|a| self.map_expr(a)).collect();
                self.arenas.exprs.alloc(Expr::Call { function, args })
            }
            Expr::Closure {
                params,
                body,
                return_type,
            } => {
                self.frames.push(HashMap::new());
                let params: Vec<(Symbol, &'a TypeExpr<'a>)> =
                    params.iter().map(|(p, t)| (self.bind(*p), *t)).collect();
                let body = match body {
                    ClosureBody::Expression(inner) => ClosureBody::Expression(self.map_expr(inner)),
                    ClosureBody::Block(block) => ClosureBody::Block(self.in_frame(block)),
                };
                self.frames.pop();
                self.arenas.exprs.alloc(Expr::Closure {
                    params,
                    body,
                    return_type: *return_type,
                })
            }
            Expr::New {
                type_name,
                type_args,
                init_fields,
            } if !self.scouting && self.shapes.records.contains_key(type_name) => {
                let written: Vec<(Symbol, &'a Expr<'a>)> =
                    init_fields.iter().map(|(f, v)| (*f, self.map_expr(v))).collect();
                let mut held: Vec<(Symbol, &'a Expr<'a>)> = written.clone();
                let declared = self.shapes.records[type_name].clone();
                for (field, ty) in &declared {
                    if written.iter().all(|(given, _)| given != field) {
                        if let Some(start) = self.default_of(ty) {
                            held.push((*field, start));
                        }
                    }
                }
                held.sort_by(|(a, _), (b, _)| self.interner.resolve(*a).cmp(self.interner.resolve(*b)));
                let order: Vec<Symbol> = held.iter().map(|(f, _)| *f).collect();
                let type_name = *type_name;
                let type_args = type_args.clone();
                self.ordered_construction(&written, &held, &order, move |fields| Expr::New {
                    type_name,
                    type_args,
                    init_fields: fields,
                })
            }
            Expr::NewVariant {
                enum_name,
                variant,
                fields,
            } if !self.scouting && self.shapes.variants.contains_key(&(*enum_name, *variant)) => {
                let written: Vec<(Symbol, &'a Expr<'a>)> =
                    fields.iter().map(|(f, v)| (*f, self.map_expr(v))).collect();
                let order = self.shapes.variants[&(*enum_name, *variant)].clone();
                let held: Vec<(Symbol, &'a Expr<'a>)> = order
                    .iter()
                    .filter_map(|d| written.iter().find(|(f, _)| f == d).copied())
                    .collect();
                let (enum_name, variant) = (*enum_name, *variant);
                self.ordered_construction(&written, &held, &order, move |fields| Expr::NewVariant {
                    enum_name,
                    variant,
                    fields,
                })
            }
            _ => ast_map::walk_expr(self, e),
        }
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        match s {
            Stmt::FunctionDef { .. } => self.function(s),
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                let iterable = self.map_expr(iterable);
                let binders: Vec<Symbol> = match pattern {
                    Pattern::Identifier(x) => vec![*x],
                    Pattern::Tuple(xs) => xs.clone(),
                };
                let (names, body) = self.scoped(&binders, body);
                let pattern = match pattern {
                    Pattern::Identifier(_) => Pattern::Identifier(names[0]),
                    Pattern::Tuple(_) => Pattern::Tuple(names),
                };
                Stmt::Repeat {
                    pattern,
                    iterable,
                    body,
                }
            }
            Stmt::Select { branches } => Stmt::Select {
                branches: branches
                    .iter()
                    .map(|branch| match branch {
                        SelectBranch::Receive { var, pipe, body } => {
                            let pipe = self.map_expr(pipe);
                            let (names, body) = self.scoped(&[*var], body);
                            SelectBranch::Receive {
                                var: names[0],
                                pipe,
                                body,
                            }
                        }
                        SelectBranch::Timeout { milliseconds, body } => SelectBranch::Timeout {
                            milliseconds: self.map_expr(milliseconds),
                            body: self.map_block(body),
                        },
                    })
                    .collect(),
            },
            Stmt::Call { function, args } => {
                let function = self.call_name(*function);
                let args = args.iter().map(|a| self.map_expr(a)).collect();
                Stmt::Call { function, args }
            }
            Stmt::Splice { body } => Stmt::Splice {
                body: self.in_frame(body),
            },
            Stmt::Reversibly { body } => Stmt::Reversibly {
                body: self.in_frame(body),
            },
            Stmt::Concurrent { tasks } => Stmt::Concurrent {
                tasks: self.in_frame(tasks),
            },
            Stmt::Parallel { tasks } => Stmt::Parallel {
                tasks: self.in_frame(tasks),
            },
            _ => {
                let mut out = ast_map::walk_stmt(self, s);
                match &mut out {
                    Stmt::Let { var, .. } => *var = self.bind(*var),
                    Stmt::Set { target, .. } => *target = self.use_name(*target),
                    Stmt::Sync { var, .. } => *var = self.use_name(*var),
                    Stmt::Check { subject, object, .. } => {
                        *subject = self.use_name(*subject);
                        if let Some(object) = object {
                            *object = self.use_name(*object);
                        }
                    }
                    Stmt::Swap { a, b } => {
                        *a = self.use_name(*a);
                        *b = self.use_name(*b);
                    }
                    Stmt::Pop { into: Some(x), .. }
                    | Stmt::TrySendPipe { result: Some(x), .. }
                    | Stmt::AwaitTask { result: Some(x), .. } => *x = self.bind(*x),
                    Stmt::ReadFrom { var, .. }
                    | Stmt::LetPeerAgent { var, .. }
                    | Stmt::Mount { var, .. }
                    | Stmt::CreatePipe { var, .. }
                    | Stmt::ReceivePipe { var, .. }
                    | Stmt::TryReceivePipe { var, .. } => *var = self.bind(*var),
                    Stmt::AwaitMessage { into, .. } => *into = self.bind(*into),
                    Stmt::LaunchTaskWithHandle { handle, .. } => *handle = self.bind(*handle),
                    Stmt::Spawn { name, .. } => *name = self.bind(*name),
                    _ => {}
                }
                out
            }
        }
    }
}

/// The lifting pass: a function that reaches program bindings takes each as a `mutable` parameter, and each call of
/// it passes them on.
struct Lift<'a, 'r> {
    arenas: Arenas<'a>,
    needs: &'r HashMap<Symbol, Vec<Symbol>>,
    roots: &'r HashMap<Symbol, Option<&'a TypeExpr<'a>>>,
    any: Symbol,
}

impl<'a, 'r> Lift<'a, 'r> {
    /// `args` and then, for a call of a function that reaches program bindings, each of them by name.
    fn handed_on(&mut self, function: Symbol, args: &[&'a Expr<'a>]) -> Vec<&'a Expr<'a>> {
        let mut out: Vec<&'a Expr<'a>> = args.iter().map(|a| self.map_expr(a)).collect();
        if let Some(names) = self.needs.get(&function) {
            for name in names {
                out.push(self.arenas.exprs.alloc(Expr::Identifier(*name)));
            }
        }
        out
    }

    /// `mutable T` for a program binding whose `Let` wrote `T`, `mutable Any` otherwise.
    fn lent_type(&self, name: Symbol) -> &'a TypeExpr<'a> {
        let inner: &'a TypeExpr<'a> = match self.roots.get(&name).copied().flatten() {
            Some(TypeExpr::Mutable { inner }) => inner,
            Some(written) => written,
            None => self.arenas.types.alloc(TypeExpr::Named(self.any)),
        };
        self.arenas.types.alloc(TypeExpr::Mutable { inner })
    }
}

impl<'a, 'r> AstMap<'a> for Lift<'a, 'r> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        match e {
            Expr::Call { function, args } if self.needs.contains_key(function) => {
                let args = self.handed_on(*function, args);
                self.arenas.exprs.alloc(Expr::Call {
                    function: *function,
                    args,
                })
            }
            _ => ast_map::walk_expr(self, e),
        }
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        match s {
            Stmt::Call { function, args } if self.needs.contains_key(function) => Stmt::Call {
                function: *function,
                args: self.handed_on(*function, args),
            },
            Stmt::FunctionDef { name, .. } if self.needs.contains_key(name) => {
                let mut out = ast_map::walk_stmt(self, s);
                if let Stmt::FunctionDef { params, .. } = &mut out {
                    for binding in &self.needs[name] {
                        params.push((*binding, self.lent_type(*binding)));
                    }
                }
                out
            }
            _ => ast_map::walk_stmt(self, s),
        }
    }
}
