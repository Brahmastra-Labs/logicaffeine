//! Which integer SLOTS of a C program hold the exact tagged `LInt` — a function's return, a function's
//! parameter, a struct's field, an enum variant's field — decided once for the whole program.
//!
//! LOGOS has one integer type, and it is arbitrary precision. The C tier carries it in two
//! representations: the machine word `int64_t`, and the tagged `LInt` (a word until it overflows, then a
//! bignum). A local takes `LInt` when it can grow past `i64` (`types::collect_bigint_vars`). Every slot a
//! value then travels through must agree with it: an `LInt` handed to an `int64_t` parameter or stored in
//! an `int64_t` field is a C program that does not build, and narrowing it there would turn a value the
//! tree-walker keeps exact into an abort. So a slot is `LInt` exactly when some value stored in it can be:
//!
//!   - a function's `Int` return, when one of its `Return`s yields an exact value;
//!   - a function's `Int` parameter, when some call or task launch passes an exact value in its position;
//!   - a function's `mutable` `Int` parameter, when its body makes it exact: the caller's variable comes back
//!     holding what the body made it, so it is exact too, at every call that lends one there;
//!   - a record's or a variant's `Int` field, when some construction or `Set … field` stores one;
//!   - an `Int` component of a function's tuple or named-record return, when a `Return` yields an exact
//!     value in its position — and so the same component of every local bound to such a call.
//!
//! Each decision feeds the others — a call's result is a value, a field read is a value, a parameter is a
//! local of its body — so they are taken together and repeated until nothing changes. Every set only
//! grows, so the fixpoint ends. The locals it settles on are the ones emission declares `LInt`, from the
//! same computation, so the analysis and the code it drives cannot disagree about a local.
//!
//! A counter-CRDT field (its cell holds a `logos_crdt*` handle) keeps the machine word. A task's argument and a
//! waiting call's argument or result cross a scalar cell, which carries an `LInt` too (`lint_to_cell`).

use std::collections::{HashMap, HashSet};

use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

use super::types::{
    body_returns_exact, collect_bigint_vars, collect_float_vars, exact_decision,
    expr_carries_exact, expr_is_int_power_form, field_type_to_ctype, int_exceeds_i64,
    resolve_type_expr_with_registry, variant_fields, CType,
};
use crate::analysis::TypeRegistry;
use crate::ast::stmt::{Expr, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// The exact-integer slots of one program. Empty for a program whose every integer fits a word: its C is
/// then what it would be without this pass.
#[derive(Debug, Default, Clone)]
pub(crate) struct ExactSlots {
    /// User callees proven pure by the shared whole-program analysis.
    pub(crate) pure_calls: HashSet<Symbol>,
    /// Functions whose `Int` return is `LInt`.
    pub(crate) returns: HashSet<Symbol>,
    /// Each function's `Int` parameters that are `LInt`.
    pub(crate) params: HashMap<Symbol, HashSet<Symbol>>,
    /// The `LInt` fields: `(struct, field)` for a record, `(variant, field)` for an enum variant.
    pub(crate) fields: HashSet<(Symbol, Symbol)>,
    /// The names of the fields in `fields`, by which a field read of a not-yet-known record is judged.
    pub(crate) field_names: HashSet<Symbol>,
    /// Each function's `LInt` locals (its exact parameters among them).
    pub(crate) locals: HashMap<Symbol, HashSet<Symbol>>,
    /// Main's `LInt` locals.
    pub(crate) main_locals: HashSet<Symbol>,
    /// Each function returning a tuple or a named record: which of its components are `LInt`.
    pub(crate) tuple_returns: HashMap<Symbol, Vec<bool>>,
    /// Each local bound to a tuple or a named record with an `LInt` component: which components are.
    pub(crate) tuple_locals: HashMap<Symbol, Vec<bool>>,
    /// Each function's `mutable` parameter positions whose slot is `LInt`: a variable lent there is `LInt` as well.
    pub(crate) lent_exact: HashMap<Symbol, HashSet<usize>>,
    /// The names holding a function value — a parameter of function type, a binding of a closure: a call
    /// through one answers an exact integer, as a function value's integers are exact.
    pub(crate) fn_values: HashSet<Symbol>,
    /// The program bindings some body makes exact: Main declares them `LInt`, and every function reads them so.
    pub(crate) program: HashSet<Symbol>,
    /// Each user function, with the words its declared result is ([`crate::analysis::exactness::word_results`]).
    pub(crate) word_results: HashMap<Symbol, Option<crate::analysis::exactness::WordShape>>,
}

impl ExactSlots {
    fn size(&self) -> (usize, usize, usize, usize, usize, usize) {
        let components =
            |m: &HashMap<Symbol, Vec<bool>>| m.values().flatten().filter(|c| **c).count();
        (
            self.returns.len(),
            self.params.values().map(HashSet::len).sum(),
            self.fields.len(),
            components(&self.tuple_returns) + components(&self.tuple_locals),
            self.lent_exact.values().map(HashSet::len).sum(),
            self.program.len(),
        )
    }

    /// The C type `function` returns, from its declared one: an `Int` the analysis made exact is `LInt`, and so
    /// is each exact component of a tuple or named record.
    pub(crate) fn widened_return(&self, function: Symbol, declared: CType) -> CType {
        let exact = |i: usize| {
            self.tuple_returns
                .get(&function)
                .is_some_and(|c| c.get(i).copied().unwrap_or(false))
        };
        match declared {
            CType::Int64 if self.returns.contains(&function) => CType::BigInt,
            CType::Tuple(elems) => CType::Tuple(
                elems
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| if exact(i) { CType::BigInt } else { t })
                    .collect(),
            ),
            CType::Record(fields) => CType::Record(
                fields
                    .into_iter()
                    .enumerate()
                    .map(|(i, (n, t))| (n, if exact(i) { CType::BigInt } else { t }))
                    .collect(),
            ),
            other => other,
        }
    }

    /// Whether component `k` (1-based, as `item k of`) of the tuple `collection` evaluates to is exact.
    pub(crate) fn component_is_exact(&self, collection: &Expr, k: i64) -> bool {
        let flags = match collection {
            Expr::Identifier(t) => self.tuple_locals.get(t),
            Expr::Call { function, .. } => self.tuple_returns.get(function),
            _ => None,
        };
        k >= 1 && flags.is_some_and(|c| c.get(k as usize - 1).copied().unwrap_or(false))
    }

    /// Marks component `i` of `flags` exact, growing it to hold `i`.
    fn mark_component(flags: &mut Vec<bool>, i: usize) {
        if flags.len() <= i {
            flags.resize(i + 1, false);
        }
        flags[i] = true;
    }

    /// Whether `param` of `function` is `LInt`.
    pub(crate) fn param_is_exact(&self, function: Symbol, param: Symbol) -> bool {
        self.params
            .get(&function)
            .is_some_and(|ps| ps.contains(&param))
    }

    fn mark_field(&mut self, owner: Symbol, field: Symbol) {
        self.fields.insert((owner, field));
        self.field_names.insert(field);
    }
}

/// A user function as the analysis reads it.
struct UserFn<'s, 'a> {
    name: Symbol,
    params: Vec<(Symbol, CType)>,
    /// The locals that hold no `Int`: its Reals and its words.
    no_int: HashSet<Symbol>,
    /// Which parameters are `mutable`: the caller's variable, passed by reference.
    inout: Vec<bool>,
    body: &'s [Stmt<'a>],
    returns_int: bool,
    /// The declared tuple or named-record return's component types, each with its name in a named record.
    returns_components: Option<Vec<(Option<Symbol>, CType)>>,
}

/// The exact-integer slots of `stmts` (see the module docs). `exact_hofs` are the scalar higher-order
/// functions whose integers are exact already (every `Int` parameter and the return); `counter_fields` are the
/// counter-CRDT fields.
pub(crate) fn exact_slots(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    interner: &Interner,
    exact_hofs: &HashSet<Symbol>,
    counter_fields: &HashSet<(Symbol, Symbol)>,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> ExactSlots {
    let word_results = crate::analysis::exactness::word_results(stmts, interner);
    let fns: Vec<UserFn> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef {
                name,
                params,
                body,
                return_type,
                is_native: false,
                ..
            } => Some(UserFn {
                name: *name,
                params: params
                    .iter()
                    .map(|(p, t)| {
                        (
                            *p,
                            resolve_type_expr_with_registry(t, interner, Some(registry)),
                        )
                    })
                    .collect(),
                no_int: {
                    let ctypes: Vec<(Symbol, CType)> = params
                        .iter()
                        .map(|(p, t)| {
                            (
                                *p,
                                resolve_type_expr_with_registry(t, interner, Some(registry)),
                            )
                        })
                        .collect();
                    let declared: Vec<(Symbol, &TypeExpr)> =
                        params.iter().map(|(p, t)| (*p, *t)).collect();
                    let mut no_int = collect_float_vars(body, &ctypes, interner, registry);
                    no_int.extend(crate::analysis::exactness::word_vars(
                        body,
                        &declared,
                        &word_results,
                        registry,
                        interner,
                    ));
                    no_int
                },
                inout: params
                    .iter()
                    .map(|(_, t)| super::types::is_inout(t))
                    .collect(),
                body,
                returns_int: return_type.is_some_and(|t| {
                    resolve_type_expr_with_registry(t, interner, Some(registry)) == CType::Int64
                        && crate::analysis::exactness::declared_word(t, interner).is_none()
                }),
                returns_components: return_type.and_then(
                    |t| match resolve_type_expr_with_registry(t, interner, Some(registry)) {
                        CType::Tuple(elems) => Some(elems.into_iter().map(|t| (None, t)).collect()),
                        CType::Record(fields) => {
                            Some(fields.into_iter().map(|(n, t)| (Some(n), t)).collect())
                        }
                        _ => None,
                    },
                ),
            }),
            _ => None,
        })
        .collect();
    let by_name: HashMap<Symbol, &UserFn> = fns.iter().map(|f| (f.name, f)).collect();

    let mut slots = ExactSlots {
        pure_calls: crate::optimize::purity::Purity::of_program(stmts, interner).into_set(),
        word_results,
        ..ExactSlots::default()
    };
    for f in &fns {
        slots.fn_values.extend(
            f.params
                .iter()
                .filter(|(_, t)| matches!(t, CType::FnPtr { .. }))
                .map(|(p, _)| *p),
        );
    }
    walk_nodes(stmts, |node| {
        if let Visited::Stmt(
            Stmt::Let {
                var,
                value: Expr::Closure { .. },
                ..
            }
            | Stmt::Set {
                target: var,
                value: Expr::Closure { .. },
            },
        ) = node
        {
            slots.fn_values.insert(*var);
        }
        Walk::Into
    });
    let fn_values = &slots.fn_values;
    slots.pure_calls.retain(|name| !fn_values.contains(name));
    for f in &fns {
        if exact_hofs.contains(&f.name) {
            let ints: HashSet<Symbol> = f
                .params
                .iter()
                .filter(|(p, t)| *t == CType::Int64 && !f.no_int.contains(p))
                .map(|(p, _)| *p)
                .collect();
            slots.params.insert(f.name, ints);
            if f.returns_int {
                slots.returns.insert(f.name);
            }
        }
    }

    // The program bindings: every name Main binds.
    let mut program_bindings: HashSet<Symbol> = HashSet::new();
    super::types::for_each_binding(stmts, |target, _| {
        program_bindings.insert(target);
    });
    loop {
        let before = slots.size();
        for f in &fns {
            let seeds = slots.params.get(&f.name).cloned().unwrap_or_default();
            let decided = exact_decision(
                f.body,
                Some(f.name),
                &f.no_int,
                interner,
                &slots,
                oracle,
                &seeds,
            );
            let big = decided.vars;
            slots
                .program
                .extend(big.iter().filter(|v| program_bindings.contains(v)).copied());
            // A `mutable` `Int` parameter that is exact anywhere is one slot with every variable lent to it.
            for (i, ((p, t), &by_ref)) in f.params.iter().zip(&f.inout).enumerate() {
                let exact_param = slots.params.get(&f.name).is_some_and(|ps| ps.contains(p));
                if by_ref && *t == CType::Int64 && (big.contains(p) || exact_param) {
                    slots.params.entry(f.name).or_default().insert(*p);
                    slots.lent_exact.entry(f.name).or_default().insert(i);
                }
            }
            if f.returns_int
                && (decided.returns || body_returns_exact(f.body, &big, &slots, interner))
            {
                slots.returns.insert(f.name);
            }
            if let Some(components) = &f.returns_components {
                record_component_returns(f.name, components, f.body, &big, interner, &mut slots);
            }
            record_tuple_locals(f.body, &big, interner, &mut slots);
            record_stores(
                f.body,
                &big,
                &by_name,
                counter_fields,
                registry,
                interner,
                &mut slots,
            );
            slots.locals.insert(f.name, big);
        }
        let mut no_int = collect_float_vars(stmts, &[], interner, registry);
        no_int.extend(crate::analysis::exactness::word_vars(
            stmts,
            &[],
            &slots.word_results,
            registry,
            interner,
        ));
        let big = collect_bigint_vars(stmts, &no_int, interner, &slots, oracle, &HashSet::new());
        slots
            .program
            .extend(big.iter().filter(|v| program_bindings.contains(v)).copied());
        record_tuple_locals(stmts, &big, interner, &mut slots);
        record_stores(
            stmts,
            &big,
            &by_name,
            counter_fields,
            registry,
            interner,
            &mut slots,
        );
        slots.main_locals = big;
        if slots.size() == before {
            break;
        }
    }
    slots
}

/// Whether `value` is an exact integer where the body's `LInt` locals are `big` — by the rule that makes a
/// local exact (`types::collect_bigint_vars`), so a slot and the local it is read into agree.
fn is_exact(value: &Expr, big: &HashSet<Symbol>, slots: &ExactSlots, interner: &Interner) -> bool {
    int_exceeds_i64(value)
        || expr_carries_exact(value, big, slots)
        || expr_is_int_power_form(value, interner)
}

/// Which components of the tuple `value` evaluates to are exact: a tuple literal's exact items, a call's or a
/// local's recorded ones.
fn tuple_components(
    value: &Expr,
    big: &HashSet<Symbol>,
    slots: &ExactSlots,
    interner: &Interner,
) -> Vec<bool> {
    match value {
        Expr::Tuple(items) => items
            .iter()
            .map(|item| is_exact(item, big, slots, interner))
            .collect(),
        Expr::Identifier(t) => slots.tuple_locals.get(t).cloned().unwrap_or_default(),
        Expr::Call { function, .. } => slots
            .tuple_returns
            .get(function)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The components of `function`'s tuple or named-record return (declared `components`) that some `Return` in
/// `body` fills with an exact value.
fn record_component_returns(
    function: Symbol,
    components: &[(Option<Symbol>, CType)],
    body: &[Stmt],
    big: &HashSet<Symbol>,
    interner: &Interner,
    slots: &mut ExactSlots,
) {
    let mut exact: Vec<usize> = Vec::new();
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Return { value: Some(v) }) => {
            for (i, is) in tuple_components(v, big, slots, interner)
                .into_iter()
                .enumerate()
            {
                if is && components.get(i).is_some_and(|(_, t)| *t == CType::Int64) {
                    exact.push(i);
                }
            }
            Walk::Into
        }
        _ => Walk::Into,
    });
    let flags = slots
        .tuple_returns
        .entry(function)
        .or_insert_with(|| vec![false; components.len()]);
    for &i in &exact {
        ExactSlots::mark_component(flags, i);
    }
    // A named record's exact component is read by its name.
    for i in exact {
        if let Some((Some(name), _)) = components.get(i) {
            slots.field_names.insert(*name);
        }
    }
}

/// The locals of `body` bound to a tuple or named record with an exact component, and which components are.
fn record_tuple_locals(
    body: &[Stmt],
    big: &HashSet<Symbol>,
    interner: &Interner,
    slots: &mut ExactSlots,
) {
    let mut found: Vec<(Symbol, Vec<bool>)> = Vec::new();
    super::types::for_each_binding(body, |target, binding| {
        if let super::types::Binding::Value(value, _) = binding {
            let components = tuple_components(value, big, slots, interner);
            if components.iter().any(|c| *c) {
                found.push((target, components));
            }
        }
    });
    for (target, components) in found {
        let flags = slots.tuple_locals.entry(target).or_default();
        for (i, is) in components.into_iter().enumerate() {
            if is {
                ExactSlots::mark_component(flags, i);
            }
        }
    }
}

/// Every slot `body` stores an exact value in: a user function's `Int` parameter at a call, a record's
/// `Int` field at a construction or a `Set … field`, a variant's `Int` field at a construction.
#[allow(clippy::too_many_arguments)]
fn record_stores(
    body: &[Stmt],
    big: &HashSet<Symbol>,
    fns: &HashMap<Symbol, &UserFn>,
    counter_fields: &HashSet<(Symbol, Symbol)>,
    registry: &TypeRegistry,
    interner: &Interner,
    slots: &mut ExactSlots,
) {
    let mut stores: Vec<Store> = Vec::new();
    // A closure's body is walked too: it is lowered on its own, but the records it builds and the functions
    // it calls are the program's. Its parameters are machine words, never in `big`.
    walk_nodes(body, |node| {
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) => return Walk::Past,
            Visited::Expr(Expr::Call { function, args })
            | Visited::Stmt(
                Stmt::Call { function, args }
                | Stmt::LaunchTask { function, args }
                | Stmt::LaunchTaskWithHandle { function, args, .. },
            ) => {
                if let Some(f) = fns.get(function) {
                    for (i, arg) in args.iter().enumerate() {
                        if let Some((p, CType::Int64)) = f.params.get(i) {
                            if is_exact(arg, big, slots, interner) {
                                stores.push(Store::Param(f.name, *p));
                            }
                        }
                    }
                }
            }
            Visited::Expr(Expr::New {
                type_name,
                init_fields,
                ..
            }) => {
                if let Some(crate::analysis::TypeDef::Struct { fields, .. }) =
                    registry.get(*type_name)
                {
                    for (fname, value) in init_fields {
                        let int_field = fields.iter().any(|fd| {
                            fd.name == *fname
                                && field_type_to_ctype(&fd.ty, interner, registry) == CType::Int64
                        });
                        if int_field
                            && !counter_fields.contains(&(*type_name, *fname))
                            && is_exact(value, big, slots, interner)
                        {
                            stores.push(Store::Field(*type_name, *fname));
                        }
                    }
                }
            }
            Visited::Expr(Expr::NewVariant {
                enum_name,
                variant,
                fields,
            }) => {
                for (fname, value) in fields {
                    let int_field = variant_fields(Some(*enum_name), *variant, *fname, registry)
                        .iter()
                        .any(|(_, fd)| {
                            field_type_to_ctype(&fd.ty, interner, registry) == CType::Int64
                        });
                    if int_field && is_exact(value, big, slots, interner) {
                        stores.push(Store::Field(*variant, *fname));
                    }
                }
            }
            // Which record `object` is, is not known before emission: every record declaring an `Int` field
            // of this name takes the exact value.
            Visited::Stmt(Stmt::SetField { field, value, .. }) => {
                if is_exact(value, big, slots, interner) {
                    for (sym, def) in registry.iter_types() {
                        if let crate::analysis::TypeDef::Struct { fields, .. } = def {
                            let int_field = fields.iter().any(|fd| {
                                fd.name == *field
                                    && field_type_to_ctype(&fd.ty, interner, registry)
                                        == CType::Int64
                            });
                            if int_field && !counter_fields.contains(&(*sym, *field)) {
                                stores.push(Store::Field(*sym, *field));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Walk::Into
    });
    for store in stores {
        match store {
            Store::Param(f, p) => {
                slots.params.entry(f).or_default().insert(p);
            }
            Store::Field(owner, field) => slots.mark_field(owner, field),
        }
    }
}

enum Store {
    Param(Symbol, Symbol),
    Field(Symbol, Symbol),
}

/// What decides a field's C type beyond its declaration: the fields the program stores an exact value in, and where a
/// stored `nothing` reaches.
pub(crate) struct FieldFacts<'a> {
    pub(crate) exact: &'a HashSet<(Symbol, Symbol)>,
    pub(crate) absence: &'a crate::analysis::absence::Absence,
}

/// The C type of `field` of `owner` (a struct, or an enum variant): its declared type, `LInt` where the
/// program stores an exact value in a declared `Int`, holding values-or-nothing where a stored `nothing` reaches it.
pub(crate) fn field_ctype(
    owner: Symbol,
    field: &crate::analysis::FieldDef,
    interner: &Interner,
    registry: &TypeRegistry,
    facts: &FieldFacts,
) -> CType {
    let declared = field_type_to_ctype(&field.ty, interner, registry);
    let declared = if declared == CType::Int64 && facts.exact.contains(&(owner, field.name)) {
        CType::BigInt
    } else {
        declared
    };
    super::absent::field(owner, field.name, declared, facts.absence, registry)
}
