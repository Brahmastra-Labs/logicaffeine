//! Monomorphization: a generic program rewritten into the concrete program it stands for, for the
//! tiers with no generics of their own.
//!
//! Every call of a generic function becomes a call of one INSTANCE — a copy of the function with
//! its type parameters replaced by the call's type arguments — and every applied generic type
//! (`Box of Int`, `Maybe of Text`, core `Result of Int and Text`) becomes one registered concrete
//! type. The type arguments come from the checker ([`crate::analysis::infer_instantiations`]),
//! which solved each site once; a site inside a generic body names that function's own type
//! variables, substituted per instance. When the backend has no `Option` of its own, `some x` /
//! `none` and `Option of T` lower to the core `Option` enum's instances. An empty sequence literal
//! whose element type the context fixes (and is not `Int`, the reading every backend already
//! gives `[]`) becomes `a new Seq of X`.
//!
//! A program with no generic site is returned unchanged, statement for statement.

use std::collections::{HashMap, HashSet};

use crate::analysis::unify::{InferType, TyVar, TypeScope};
use crate::analysis::{
    infer_instantiations, FieldDef, FieldType, Instantiations, Site, TypeDef, TypeRegistry,
    VariantDef,
};
use crate::ast::stmt::{Expr, MatchArm, Stmt, TypeExpr};
use crate::ast_map::{self, Arenas, AstMap};
use crate::intern::{Interner, Symbol};

/// How far instantiation may go before a program is judged to need infinitely many instances
/// (polymorphic recursion: `f of [T]` calling `f` at `Seq of T`).
const INSTANCE_BUDGET: usize = 4096;

/// What the backend lacks natively and monomorphization must lower.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Lower `some` / `none` / `Option of T` to instances of the core `Option` enum.
    pub lower_option: bool,
}

/// One registered concrete type: the generic type it applies, and to what.
#[derive(Clone, Debug)]
pub struct TypeInstance {
    pub generic: Symbol,
    pub args: Vec<InferType>,
}

/// A monomorphic program: its statements, the registry with every instance type added, and each
/// instance type's origin (so a backend shows `Box of Int`'s instance as a `Box`).
pub struct Monomorphized<'a> {
    pub stmts: Vec<Stmt<'a>>,
    pub registry: TypeRegistry,
    pub instances: HashMap<Symbol, TypeInstance>,
}

/// Rewrite `stmts` into a program with no generic function, no applied generic type, and (under
/// `options.lower_option`) no built-in `Option`.
pub fn monomorphize<'a>(
    stmts: &[Stmt<'a>],
    registry: &TypeRegistry,
    interner: &mut Interner,
    arenas: Arenas<'a>,
    options: Options,
) -> Result<Monomorphized<'a>, String> {
    let inst = infer_instantiations(stmts, interner, registry);
    let generic_fns: HashMap<Symbol, &Stmt<'a>> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef { name, generics, .. } if !generics.is_empty() => Some((*name, s)),
            _ => None,
        })
        .collect();
    if !needs_monomorphization(stmts, registry, interner, &inst, options) {
        return Ok(Monomorphized {
            stmts: stmts.to_vec(),
            registry: registry.clone(),
            instances: HashMap::new(),
        });
    }

    let mut taken: HashSet<String> = registry
        .iter_types()
        .map(|(s, _)| interner.resolve(*s).to_string())
        .collect();
    for s in stmts {
        if let Stmt::FunctionDef { name, .. } = s {
            taken.insert(interner.resolve(*name).to_string());
        }
    }
    let twins = registry.namespaced_twins(interner);
    let mut mono = Mono {
        arenas,
        registry,
        inst: &inst,
        options,
        twins,
        interner,
        generic_fns,
        taken,
        fn_instances: HashMap::new(),
        pending: std::collections::VecDeque::new(),
        instances_of: HashMap::new(),
        type_instances: HashMap::new(),
        out_registry: registry.clone(),
        instance_info: HashMap::new(),
        subst: HashMap::new(),
        param_types: HashMap::new(),
        error: None,
    };

    enum Segment<'a> {
        Stmt(Stmt<'a>),
        InstancesOf(Symbol),
    }
    let mut segments = Vec::with_capacity(stmts.len());
    for s in stmts {
        match s {
            Stmt::FunctionDef { name, generics, .. } if !generics.is_empty() => {
                segments.push(Segment::InstancesOf(*name))
            }
            _ => segments.push(Segment::Stmt(mono.map_stmt(s))),
        }
    }
    while let Some((generic, args, name)) = mono.pending.pop_front() {
        let def = mono.instantiate_function(generic, &args, name);
        mono.instances_of.entry(generic).or_default().push(def);
    }
    if let Some(e) = mono.error.take() {
        return Err(e);
    }

    let mut out = Vec::with_capacity(segments.len());
    for seg in segments {
        match seg {
            Segment::Stmt(s) => out.push(s),
            Segment::InstancesOf(f) => out.extend(mono.instances_of.remove(&f).unwrap_or_default()),
        }
    }
    Ok(Monomorphized {
        stmts: out,
        registry: mono.out_registry,
        instances: mono.instance_info,
    })
}

/// Whether anything in the program is generic enough to need rewriting: a call of a generic
/// function, a construction or inspection of an applied generic type (or, lowered, an `Option`),
/// an empty literal whose element the context fixes, or a signature naming an applied generic.
fn needs_monomorphization(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    interner: &Interner,
    inst: &Instantiations,
    options: Options,
) -> bool {
    let generic_type = |t: &InferType| mentions_generic(t, options);
    if !inst.calls.is_empty() {
        return true;
    }
    if inst.constructed.values().any(generic_type)
        || inst.empty_lists.values().any(is_retyped_empty_list)
    {
        return true;
    }
    if inst.scrutinees.values().any(generic_type) {
        return true;
    }
    let empty = HashMap::new();
    let twins = registry.namespaced_twins(interner);
    let scope = TypeScope {
        interner,
        registry: Some(registry),
        params: &empty,
        twins: Some(&twins),
    };
    stmts.iter().any(|s| match s {
        Stmt::FunctionDef {
            params,
            return_type,
            ..
        } => params
            .iter()
            .map(|(_, t)| *t)
            .chain(return_type.iter().copied())
            .any(|t| generic_type(&InferType::from_type_expr_in(t, &scope))),
        _ => false,
    })
}

fn mentions_generic(t: &InferType, options: Options) -> bool {
    let rec = |x: &InferType| mentions_generic(x, options);
    match t {
        InferType::UserDefined(_, args) => !args.is_empty() || args.iter().any(rec),
        InferType::Option(inner) => options.lower_option || rec(inner),
        InferType::Seq(i) | InferType::Set(i) => rec(i),
        InferType::Map(k, v) => rec(k) || rec(v),
        InferType::Function(ps, r) => ps.iter().any(rec) || rec(r),
        _ => false,
    }
}

/// An empty literal (`[]`) the context types as a sequence of something other than `Int` — the
/// element every backend already reads `[]` as.
fn is_retyped_empty_list(t: &InferType) -> bool {
    matches!(t, InferType::Seq(elem) if !matches!(**elem, InferType::Int | InferType::Var(_) | InferType::Unknown))
}

struct Mono<'a, 'r> {
    arenas: Arenas<'a>,
    registry: &'r TypeRegistry,
    inst: &'r Instantiations,
    options: Options,
    interner: &'r mut Interner,
    generic_fns: HashMap<Symbol, &'r Stmt<'a>>,
    /// Every name a type or function already has, so a minted instance name never collides.
    taken: HashSet<String>,
    fn_instances: HashMap<(Symbol, String), Symbol>,
    pending: std::collections::VecDeque<(Symbol, Vec<InferType>, Symbol)>,
    instances_of: HashMap<Symbol, Vec<Stmt<'a>>>,
    type_instances: HashMap<(Symbol, String), Symbol>,
    out_registry: TypeRegistry,
    instance_info: HashMap<Symbol, TypeInstance>,
    /// The generic function being instantiated: its type variables, and its written type
    /// parameters, each to the ground type this instance gives it.
    subst: HashMap<TyVar, InferType>,
    param_types: HashMap<Symbol, InferType>,
    error: Option<String>,
    /// Each bare type name that IS a namespaced type, so both spellings are one type.
    twins: HashMap<Symbol, Symbol>,
}

/// A canonical key for a ground type (its structure, with symbols by identity).
fn key_of(types: &[InferType]) -> String {
    format!("{types:?}")
}

impl<'a, 'r> Mono<'a, 'r> {
    fn fail(&mut self, message: String) {
        if self.error.is_none() {
            self.error = Some(message);
        }
    }

    fn sym(&mut self, name: &str) -> Symbol {
        self.interner.intern(name)
    }

    /// A fresh name built from `parts`, unused by any type or function.
    fn mint(&mut self, parts: &str) -> Symbol {
        let base: String = parts
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let mut name = base.clone();
        let mut n = 2;
        while self.taken.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        self.taken.insert(name.clone());
        self.interner.intern(&name)
    }

    /// `t` with this instance's type variables replaced by their ground types. A variable the
    /// program never constrains (a phantom `E` in `Result of Int and E`) carries no value, so
    /// any type represents it; it becomes `Int`, the one every backend represents.
    fn ground(&self, t: &InferType) -> Result<InferType, String> {
        Ok(match t {
            InferType::Var(tv) => self.subst.get(tv).cloned().unwrap_or(InferType::Int),
            InferType::Unknown => return Err("a type the checker could not determine".to_string()),
            InferType::Seq(i) => InferType::Seq(Box::new(self.ground(i)?)),
            InferType::Set(i) => InferType::Set(Box::new(self.ground(i)?)),
            InferType::Option(i) => InferType::Option(Box::new(self.ground(i)?)),
            InferType::Map(k, v) => {
                InferType::Map(Box::new(self.ground(k)?), Box::new(self.ground(v)?))
            }
            InferType::UserDefined(s, args) => InferType::UserDefined(
                *s,
                args.iter()
                    .map(|a| self.ground(a))
                    .collect::<Result<_, _>>()?,
            ),
            InferType::Function(ps, r) => InferType::Function(
                ps.iter()
                    .map(|p| self.ground(p))
                    .collect::<Result<_, _>>()?,
                Box::new(self.ground(r)?),
            ),
            ground => ground.clone(),
        })
    }

    /// Whether `t` still has a variable no instance binds — a type argument left open.
    fn leaves_open(&self, t: &InferType) -> bool {
        match t {
            InferType::Var(tv) => !self.subst.contains_key(tv),
            InferType::Seq(i) | InferType::Set(i) | InferType::Option(i) => self.leaves_open(i),
            InferType::Map(k, v) => self.leaves_open(k) || self.leaves_open(v),
            InferType::UserDefined(_, args) => args.iter().any(|a| self.leaves_open(a)),
            InferType::Function(ps, r) => {
                ps.iter().any(|p| self.leaves_open(p)) || self.leaves_open(r)
            }
            _ => false,
        }
    }

    fn name_part(&mut self, t: &InferType) -> String {
        match t {
            InferType::Int => "Int".into(),
            InferType::Float => "Real".into(),
            InferType::String => "Text".into(),
            InferType::Bool => "Bool".into(),
            InferType::Seq(i) => format!("Seq_{}", self.name_part(i)),
            InferType::Set(i) => format!("Set_{}", self.name_part(i)),
            InferType::Option(i) => format!("Option_{}", self.name_part(i)),
            InferType::Map(k, v) => format!("Map_{}_{}", self.name_part(k), self.name_part(v)),
            InferType::UserDefined(s, args) if args.is_empty() => {
                self.interner.resolve(*s).to_string()
            }
            InferType::UserDefined(s, args) => {
                let parts: Vec<String> = args.iter().map(|a| self.name_part(a)).collect();
                format!("{}_{}", self.interner.resolve(*s), parts.join("_"))
            }
            InferType::Function(ps, r) => {
                let parts: Vec<String> = ps.iter().map(|p| self.name_part(p)).collect();
                format!("Fn_{}_to_{}", parts.join("_"), self.name_part(r))
            }
            other => other.to_logos_name(),
        }
    }

    /// The instance of generic function `f` a call at `site` needs, queued for instantiation the
    /// first time it is asked for.
    fn call_instance(&mut self, f: Symbol, site: Site) -> Option<Symbol> {
        let fname = self.interner.resolve(f).to_string();
        let Some(call) = self.inst.calls.get(&site).cloned() else {
            self.fail(format!("the call of the generic function `{fname}` could not be type-checked, so its type arguments are unknown"));
            return None;
        };
        if !call.precise && call.type_args.iter().any(|t| self.leaves_open(t)) {
            self.fail(format!(
                "cannot determine the type arguments of this call of `{fname}`: an argument's type is not known before running the program"
            ));
            return None;
        }
        let mut args = Vec::with_capacity(call.type_args.len());
        for t in &call.type_args {
            match self.ground(t) {
                Ok(g) => args.push(g),
                Err(why) => {
                    self.fail(format!("cannot instantiate `{fname}`: {why}"));
                    return None;
                }
            }
        }
        let key = (f, key_of(&args));
        if let Some(name) = self.fn_instances.get(&key) {
            return Some(*name);
        }
        if self.fn_instances.len() + self.type_instances.len() >= INSTANCE_BUDGET {
            self.fail(format!(
                "`{fname}` needs more than {INSTANCE_BUDGET} concrete copies — it calls itself at an ever-larger type, which a tier without generics cannot compile"
            ));
            return None;
        }
        let parts: Vec<String> = args.iter().map(|a| self.name_part(a)).collect();
        let name = self.mint(&format!("{fname}__{}", parts.join("__")));
        self.fn_instances.insert(key, name);
        self.pending.push_back((f, args, name));
        Some(name)
    }

    /// The generic definition an applied type instantiates. `Option` is the one generic the
    /// prelude never declares — `some`/`none` are built in, so core's declaration is dropped
    /// before prepending and the registry holds only its built-in arity — and lowers from core's
    /// own definition,
    /// `An Option of [T] is either: A Some (value: T). A None.`, unless the program declares its
    /// own.
    fn template(&mut self, generic: Symbol) -> Option<TypeDef> {
        if let Some(def @ (TypeDef::Struct { .. } | TypeDef::Enum { .. })) =
            self.registry.get(generic)
        {
            return Some(def.clone());
        }
        if self.interner.resolve(generic) != "Option" {
            return None;
        }
        let t = self.sym("T");
        let (some, none, value) = (self.sym("Some"), self.sym("None"), self.sym("value"));
        Some(TypeDef::Enum {
            variants: vec![
                VariantDef {
                    name: some,
                    fields: vec![FieldDef {
                        name: value,
                        ty: FieldType::TypeParam(t),
                        is_public: true,
                    }],
                },
                VariantDef {
                    name: none,
                    fields: Vec::new(),
                },
            ],
            generics: vec![t],
            is_portable: false,
            is_shared: false,
        })
    }

    /// The concrete type an applied generic type (or, lowered, an `Option`) becomes, registered
    /// the first time it is asked for. `None` for a type that stays as it is.
    fn type_instance(&mut self, t: &InferType) -> Option<Symbol> {
        let (generic, args) = match t {
            InferType::UserDefined(g, args) if !args.is_empty() => (*g, args.clone()),
            InferType::Option(inner) if self.options.lower_option => {
                (self.sym("Option"), vec![(**inner).clone()])
            }
            _ => return None,
        };
        let key = (generic, key_of(&args));
        if let Some(name) = self.type_instances.get(&key) {
            return Some(*name);
        }
        if self.fn_instances.len() + self.type_instances.len() >= INSTANCE_BUDGET {
            let gname = self.interner.resolve(generic).to_string();
            self.fail(format!(
                "`{gname}` needs more than {INSTANCE_BUDGET} concrete types"
            ));
            return None;
        }
        let parts: Vec<String> = args.iter().map(|a| self.name_part(a)).collect();
        let gname = self.interner.resolve(generic).to_string();
        let name = self.mint(&format!("{gname}__{}", parts.join("__")));
        self.type_instances.insert(key, name);
        self.instance_info.insert(
            name,
            TypeInstance {
                generic,
                args: args.clone(),
            },
        );
        let def = match self.template(generic) {
            Some(TypeDef::Struct {
                fields,
                generics,
                is_portable,
                is_shared,
            }) => {
                let params: HashMap<Symbol, InferType> =
                    generics.iter().copied().zip(args.iter().cloned()).collect();
                let fields = fields
                    .iter()
                    .map(|f| FieldDef {
                        name: f.name,
                        ty: self.field_type(&f.ty, &params),
                        is_public: f.is_public,
                    })
                    .collect();
                TypeDef::Struct {
                    fields,
                    generics: Vec::new(),
                    is_portable,
                    is_shared,
                }
            }
            Some(TypeDef::Enum {
                variants,
                generics,
                is_portable,
                is_shared,
            }) => {
                let params: HashMap<Symbol, InferType> =
                    generics.iter().copied().zip(args.iter().cloned()).collect();
                let variants = variants
                    .iter()
                    .map(|v| VariantDef {
                        name: v.name,
                        fields: v
                            .fields
                            .iter()
                            .map(|f| FieldDef {
                                name: f.name,
                                ty: self.field_type(&f.ty, &params),
                                is_public: f.is_public,
                            })
                            .collect(),
                    })
                    .collect();
                TypeDef::Enum {
                    variants,
                    generics: Vec::new(),
                    is_portable,
                    is_shared,
                }
            }
            _ => {
                self.fail(format!(
                    "`{gname}` is applied to type arguments but is not a generic record or enum"
                ));
                return None;
            }
        };
        self.out_registry.register(name, def);
        Some(name)
    }

    /// A declared field type at one application of its record or enum, `params` giving each of
    /// the type's own parameters. A field naming no parameter and no applied generic is kept
    /// exactly as written.
    fn field_type(&mut self, ft: &FieldType, params: &HashMap<Symbol, InferType>) -> FieldType {
        if !self.field_mentions(ft, params) {
            return ft.clone();
        }
        // A tuple is opaque to inference: substitute inside it position by position.
        match ft {
            FieldType::Tuple(items) => {
                return FieldType::Tuple(items.iter().map(|t| self.field_type(t, params)).collect())
            }
            FieldType::Record(fields) => {
                return FieldType::Record(
                    fields
                        .iter()
                        .map(|(n, t)| (*n, self.field_type(t, params)))
                        .collect(),
                )
            }
            _ => {}
        }
        let ty = InferType::from_field_type_in(
            ft,
            &TypeScope {
                interner: self.interner,
                registry: Some(self.registry),
                params,
                twins: Some(&self.twins),
            },
        );
        match self.ground(&ty) {
            Ok(g) => self.ground_field_type(&g),
            Err(why) => {
                self.fail(format!("cannot lay out a field: {why}"));
                ft.clone()
            }
        }
    }

    fn field_mentions(&self, ft: &FieldType, params: &HashMap<Symbol, InferType>) -> bool {
        match ft {
            FieldType::TypeParam(p) | FieldType::Named(p) => params.contains_key(p),
            FieldType::Primitive(_) => false,
            FieldType::Generic { base, params: ps } => {
                self.is_generic_type(*base) || ps.iter().any(|p| self.field_mentions(p, params))
            }
            FieldType::Tuple(items) => items.iter().any(|t| self.field_mentions(t, params)),
            FieldType::Record(fields) => fields.iter().any(|(_, t)| self.field_mentions(t, params)),
        }
    }

    /// A registered generic record or enum, or (lowered) the built-in `Option`.
    fn is_generic_type(&self, base: Symbol) -> bool {
        match self.registry.get(base) {
            Some(TypeDef::Struct { generics, .. } | TypeDef::Enum { generics, .. }) => {
                !generics.is_empty()
            }
            _ => false,
        }
    }

    fn ground_field_type(&mut self, t: &InferType) -> FieldType {
        if let Some(inst) = self.type_instance(t) {
            return FieldType::Named(inst);
        }
        match t {
            InferType::Seq(i) => FieldType::Generic {
                base: self.sym("Seq"),
                params: vec![self.ground_field_type(i)],
            },
            InferType::Set(i) => FieldType::Generic {
                base: self.sym("Set"),
                params: vec![self.ground_field_type(i)],
            },
            InferType::Option(i) => FieldType::Generic {
                base: self.sym("Option"),
                params: vec![self.ground_field_type(i)],
            },
            InferType::Map(k, v) => FieldType::Generic {
                base: self.sym("Map"),
                params: vec![self.ground_field_type(k), self.ground_field_type(v)],
            },
            InferType::UserDefined(s, _) => FieldType::Named(*s),
            other => match other.primitive_name() {
                Some(name) => FieldType::Primitive(self.sym(name)),
                None => {
                    let shown = other.to_logos_name();
                    self.fail(format!("a record field cannot hold a {shown} on this tier"));
                    FieldType::Primitive(self.sym("Int"))
                }
            },
        }
    }

    /// A ground type written back as a type expression.
    fn ground_type_expr(&mut self, t: &InferType) -> TypeExpr<'a> {
        if let Some(inst) = self.type_instance(t) {
            return TypeExpr::Named(inst);
        }
        let types = self.arenas.types;
        match t {
            InferType::Seq(i) => {
                let p = self.ground_type_expr(i);
                TypeExpr::Generic {
                    base: self.sym("Seq"),
                    params: types.alloc_slice([p]),
                }
            }
            InferType::Set(i) => {
                let p = self.ground_type_expr(i);
                TypeExpr::Generic {
                    base: self.sym("Set"),
                    params: types.alloc_slice([p]),
                }
            }
            InferType::Option(i) => {
                let p = self.ground_type_expr(i);
                TypeExpr::Generic {
                    base: self.sym("Option"),
                    params: types.alloc_slice([p]),
                }
            }
            InferType::Map(k, v) => {
                let (k, v) = (self.ground_type_expr(k), self.ground_type_expr(v));
                TypeExpr::Generic {
                    base: self.sym("Map"),
                    params: types.alloc_slice([k, v]),
                }
            }
            InferType::Function(ps, r) => {
                let inputs: Vec<TypeExpr<'a>> =
                    ps.iter().map(|p| self.ground_type_expr(p)).collect();
                let output = self.ground_type_expr(r);
                TypeExpr::Function {
                    inputs: types.alloc_slice(inputs),
                    output: types.alloc(output),
                }
            }
            InferType::UserDefined(s, _) => TypeExpr::Named(*s),
            other => match other.primitive_name() {
                Some(name) => TypeExpr::Primitive(self.sym(name)),
                None => {
                    let shown = other.to_logos_name();
                    self.fail(format!(
                        "a {shown} cannot be written as a type on this tier"
                    ));
                    TypeExpr::Primitive(self.sym("Int"))
                }
            },
        }
    }

    /// Whether a written type names this instance's type parameters or an applied generic.
    fn type_mentions(&self, t: &TypeExpr) -> bool {
        match t {
            TypeExpr::Primitive(s) | TypeExpr::Named(s) => self.param_types.contains_key(s),
            TypeExpr::Generic { base, params } => {
                self.is_generic_type(*base)
                    || (self.options.lower_option && self.is_option_name(*base))
                    || params.iter().any(|p| self.type_mentions(p))
            }
            TypeExpr::Tuple(items) => items.iter().any(|p| self.type_mentions(p)),
            TypeExpr::Record { field_types, .. } => {
                field_types.iter().any(|p| self.type_mentions(p))
            }
            TypeExpr::Function { inputs, output } => {
                inputs.iter().any(|p| self.type_mentions(p)) || self.type_mentions(output)
            }
            TypeExpr::Refinement { base, .. } => self.type_mentions(base),
            TypeExpr::Persistent { inner } | TypeExpr::Mutable { inner } => {
                self.type_mentions(inner)
            }
        }
    }

    fn is_option_name(&self, base: Symbol) -> bool {
        let name = self.interner.resolve(base);
        name == "Option"
            || (name == "Maybe"
                && !matches!(
                    self.registry.get(base),
                    Some(TypeDef::Struct { .. } | TypeDef::Enum { .. })
                ))
    }

    /// The instance function for one `(generic function, ground type arguments)`.
    fn instantiate_function(
        &mut self,
        generic: Symbol,
        args: &[InferType],
        name: Symbol,
    ) -> Stmt<'a> {
        let def = self.generic_fns[&generic];
        let Stmt::FunctionDef {
            params,
            body,
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
            ..
        } = def
        else {
            unreachable!("generic_fns holds function definitions only")
        };
        let declared = self
            .inst
            .type_params
            .get(&generic)
            .cloned()
            .unwrap_or_default();
        let saved_subst = std::mem::take(&mut self.subst);
        let saved_params = std::mem::take(&mut self.param_types);
        for ((param, resolved), arg) in declared.iter().zip(args) {
            if let InferType::Var(tv) = resolved {
                self.subst.insert(*tv, arg.clone());
            }
            self.param_types.insert(*param, arg.clone());
        }
        let params = params.iter().map(|(p, t)| (*p, self.map_type(t))).collect();
        let return_type = return_type.map(|t| self.map_type(t));
        let body = self.map_block(body);
        self.subst = saved_subst;
        self.param_types = saved_params;
        Stmt::FunctionDef {
            name,
            generics: Vec::new(),
            params,
            body,
            return_type,
            is_native: *is_native,
            native_path: *native_path,
            is_exported: *is_exported,
            export_target: *export_target,
            opt_flags: *opt_flags,
        }
    }

    /// An arm's bindings against the instance's own variant: a binding naming no field of the
    /// variant is POSITIONAL (`When Some (v)` binds the first field, whatever it is called), which
    /// the parser could only resolve for an enum it had registered — never the built-in `Option`.
    fn fields_by_position(
        &self,
        inst: Symbol,
        variant: Symbol,
        bindings: &[(Symbol, Symbol)],
    ) -> Vec<(Symbol, Symbol)> {
        let fields: Vec<Symbol> = match self.out_registry.get(inst) {
            Some(TypeDef::Enum { variants, .. }) => variants
                .iter()
                .find(|v| v.name == variant)
                .map(|v| v.fields.iter().map(|f| f.name).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        bindings
            .iter()
            .enumerate()
            .map(|(i, (field, binding))| match fields.get(i) {
                Some(declared) if !fields.contains(field) => (*declared, *binding),
                _ => (*field, *binding),
            })
            .collect()
    }

    /// The instance type a site's recorded type becomes, if it is one.
    fn site_instance(&mut self, recorded: Option<&InferType>, what: &str) -> Option<Symbol> {
        let recorded = recorded?.clone();
        match self.ground(&recorded) {
            Ok(g) => self.type_instance(&g),
            Err(why) => {
                self.fail(format!("cannot instantiate {what}: {why}"));
                None
            }
        }
    }
}

impl<'a, 'r> AstMap<'a> for Mono<'a, 'r> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_type(&mut self, t: &'a TypeExpr<'a>) -> &'a TypeExpr<'a> {
        if !self.type_mentions(t) {
            return t;
        }
        match t {
            TypeExpr::Primitive(s) | TypeExpr::Named(s) if self.param_types.contains_key(s) => {
                let g = self.param_types[s].clone();
                let written = self.ground_type_expr(&g);
                self.arenas.types.alloc(written)
            }
            TypeExpr::Generic { base, .. }
                if self.is_generic_type(*base)
                    || (self.options.lower_option && self.is_option_name(*base)) =>
            {
                let params = self.param_types.clone();
                let ty = InferType::from_type_expr_in(
                    t,
                    &TypeScope {
                        interner: self.interner,
                        registry: Some(self.registry),
                        params: &params,
                        twins: Some(&self.twins),
                    },
                );
                match self.ground(&ty) {
                    Ok(g) => {
                        let written = self.ground_type_expr(&g);
                        self.arenas.types.alloc(written)
                    }
                    Err(why) => {
                        self.fail(format!("cannot instantiate a written type: {why}"));
                        t
                    }
                }
            }
            _ => ast_map::walk_type(self, t),
        }
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        let site = Site::of_expr(e);
        let exprs = self.arenas.exprs;
        let mapped = match e {
            Expr::Call { function, args } if self.generic_fns.contains_key(function) => {
                let function = self.call_instance(*function, site).unwrap_or(*function);
                let args = args.iter().map(|a| self.map_expr(a)).collect();
                exprs.alloc(Expr::Call { function, args })
            }
            Expr::New {
                type_name,
                init_fields,
                ..
            } if self.is_generic_type(*type_name) => {
                let what = format!("a new {}", self.interner.resolve(*type_name));
                let recorded = self.inst.constructed.get(&site).cloned();
                let type_name = self
                    .site_instance(recorded.as_ref(), &what)
                    .unwrap_or(*type_name);
                let init_fields = init_fields
                    .iter()
                    .map(|(f, v)| (*f, self.map_expr(v)))
                    .collect();
                exprs.alloc(Expr::New {
                    type_name,
                    type_args: Vec::new(),
                    init_fields,
                })
            }
            Expr::NewVariant {
                enum_name,
                variant,
                fields,
            } if self.is_generic_type(*enum_name) => {
                let what = format!("a new {}", self.interner.resolve(*variant));
                let recorded = self.inst.constructed.get(&site).cloned();
                let enum_name = self
                    .site_instance(recorded.as_ref(), &what)
                    .unwrap_or(*enum_name);
                let fields = fields.iter().map(|(f, v)| (*f, self.map_expr(v))).collect();
                exprs.alloc(Expr::NewVariant {
                    enum_name,
                    variant: *variant,
                    fields,
                })
            }
            Expr::OptionSome { value } if self.options.lower_option => {
                let recorded = self.inst.constructed.get(&site).cloned();
                let value = self.map_expr(value);
                match self.site_instance(recorded.as_ref(), "`some`") {
                    Some(enum_name) => {
                        let (variant, field) = (self.sym("Some"), self.sym("value"));
                        exprs.alloc(Expr::NewVariant {
                            enum_name,
                            variant,
                            fields: vec![(field, value)],
                        })
                    }
                    None => exprs.alloc(Expr::OptionSome { value }),
                }
            }
            Expr::OptionNone if self.options.lower_option => {
                let recorded = self.inst.constructed.get(&site).cloned();
                match self.site_instance(recorded.as_ref(), "`none`") {
                    Some(enum_name) => {
                        let variant = self.sym("None");
                        exprs.alloc(Expr::NewVariant {
                            enum_name,
                            variant,
                            fields: Vec::new(),
                        })
                    }
                    None => e,
                }
            }
            Expr::List(items) if items.is_empty() => {
                let recorded = self.inst.empty_lists.get(&site).cloned();
                let elem = match recorded.map(|t| self.ground(&t)) {
                    Some(Ok(InferType::Seq(elem)))
                        if is_retyped_empty_list(&InferType::Seq(elem.clone())) =>
                    {
                        *elem
                    }
                    _ => return preserve_expr_facts(e, e),
                };
                let written = self.ground_type_expr(&elem);
                let seq = self.sym("Seq");
                exprs.alloc(Expr::New {
                    type_name: seq,
                    type_args: vec![written],
                    init_fields: Vec::new(),
                })
            }
            _ => ast_map::walk_expr(self, e),
        };
        preserve_expr_facts(e, mapped)
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        match s {
            Stmt::Call { function, args } if self.generic_fns.contains_key(function) => {
                let function = self
                    .call_instance(*function, Site::of_stmt(s))
                    .unwrap_or(*function);
                Stmt::Call {
                    function,
                    args: args.iter().map(|a| self.map_expr(a)).collect(),
                }
            }
            Stmt::LaunchTask { function, .. } | Stmt::LaunchTaskWithHandle { function, .. }
                if self.generic_fns.contains_key(function) =>
            {
                let fname = self.interner.resolve(*function).to_string();
                self.fail(format!(
                    "the generic function `{fname}` cannot be launched as a task on this tier"
                ));
                ast_map::walk_stmt(self, s)
            }
            Stmt::Inspect {
                target,
                arms,
                has_otherwise,
            } => {
                let recorded = self.inst.scrutinees.get(&Site::of_stmt(s)).cloned();
                let instance = self.site_instance(recorded.as_ref(), "an inspected value");
                let target = self.map_expr(target);
                let arms = arms
                    .iter()
                    .map(|a| {
                        let mapped = self.map_arm(a);
                        match (instance, mapped.variant) {
                            (Some(inst), Some(variant)) => {
                                let bindings =
                                    self.fields_by_position(inst, variant, &mapped.bindings);
                                MatchArm {
                                    enum_name: Some(inst),
                                    bindings,
                                    ..mapped
                                }
                            }
                            _ => mapped,
                        }
                    })
                    .collect();
                Stmt::Inspect {
                    target,
                    arms,
                    has_otherwise: *has_otherwise,
                }
            }
            _ => ast_map::walk_stmt(self, s),
        }
    }
}

/// Monomorphization changes only representation and type names, never the value or control meaning
/// of an expression. Preserve optimizer proofs on the equivalent arena node it creates so a
/// backend without generics does not silently lose optimizations that generic-capable Rust keeps.
fn preserve_expr_facts<'a>(from: &Expr<'a>, to: &'a Expr<'a>) -> &'a Expr<'a> {
    use crate::optimize::facts::{self, Fact};

    let had_tile_fact = facts::holds_expr(Fact::TileLegal, from);
    facts::copy_expr_facts(from, to);
    if had_tile_fact && std::env::var_os("LOGOS_DEBUG_C_TILE").is_some() {
        eprintln!("C TileLegal handoff: {:p} -> {:p}", from, to);
    }
    to
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/monomorphize/tests.rs"]
mod tests;
