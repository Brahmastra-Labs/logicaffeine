//! Typing context for the kernel.
//!
//! A context maps variable names to their types.
//! Used during type checking to track what variables are in scope.

use crate::term::{Term, Universe};
use std::collections::HashMap;
use std::sync::Arc;

/// Typing context: maps variable names to their types.
///
/// The context is immutable-by-default: `extend` creates a new context
/// with the additional binding, preserving the original.
///
/// Also stores global definitions:
/// - Inductive types (e.g., Nat : Type 0)
/// - Constructors (e.g., Zero : Nat, Succ : Nat -> Nat)
/// - Declarations (e.g., hypotheses like h1 : P -> Q)
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Local variable bindings (from λ and Π) — the only part that grows during type
    /// inference (one entry per enclosing binder). The context is `extend`ed (cloned) at
    /// every binder, so the binding TYPES are shared behind `Arc`: cloning the map copies
    /// pointers, not whole proposition types.
    bindings: HashMap<String, Arc<Term>>,

    /// The global environment below is FIXED during inference but the context is
    /// `extend`ed (cloned) at every λ/Π. Sharing it behind `Rc` makes that clone O(1)
    /// instead of deep-copying every premise type — the difference between linear and
    /// quadratic checking of a large certified proof.
    ///
    /// Inductive type definitions: name -> sort (e.g., "Nat" -> Type 0)
    inductives: Arc<HashMap<String, Term>>,

    /// Constructor definitions: name -> (inductive_name, type)
    constructors: Arc<HashMap<String, (String, Term)>>,

    /// How many LEADING arguments of an inductive's arity are uniform PARAMETERS (as
    /// opposed to INDICES that vary per constructor). `List (A:Type)` has 1 parameter and
    /// 0 indices; `Eq (A) (x) : A → Prop` has 2 parameters and 1 index; `Vector (A) : Nat
    /// → Type` has 1 parameter and 1 index. Absence means "all of the arity is
    /// parameters" (0 indices) — so every non-indexed inductive behaves exactly as before
    /// this map existed, and indexed elimination is a strict extension.
    inductive_params: Arc<HashMap<String, usize>>,

    /// Order of constructor registration per inductive.
    /// HashMap doesn't preserve insertion order, so we track it explicitly.
    constructor_order: Arc<HashMap<String, Vec<String>>>,

    /// Declaration bindings (axioms/hypotheses): name -> type
    /// Used for certifying proofs where hypotheses are assumed.
    declarations: Arc<HashMap<String, Term>>,

    /// Definition bodies: name -> (type, body)
    /// Definitions are transparent - they unfold during normalization.
    /// Distinguished from declarations (axioms) which have no body.
    definitions: Arc<HashMap<String, (Term, Term)>>,

    /// Hint database: theorem names marked as hints for auto tactic.
    /// When auto fails with decision procedures, it tries to apply these hints.
    hints: Arc<Vec<String>>,

    /// Universe-polymorphic definitions (R3): name -> (universe params, type, body). A
    /// `Term::Const { name, levels }` reference instantiates these params with `levels`.
    universe_polys: Arc<HashMap<String, (Vec<String>, Term, Term)>>,

    /// Typeclass instance database (R4): each `(type, value)` is an instance the
    /// elaborator may resolve for an instance-implicit argument — e.g.
    /// `(Inhabited Nat, mk Nat Zero)`. Searched by unifying `type` against the required
    /// class type.
    instances: Arc<Vec<(Term, Term)>>,

    /// Registered COERCIONS (E1): each `(from, to, coe)` is a function `coe : from → to` the
    /// elaborator may insert when an argument of type `from` is supplied where `to` is
    /// expected — Lean's `Coe`/`↑`. Searched by unifying `from`/`to` against the mismatch.
    coercions: Arc<Vec<(Term, Term, Term)>>,

    /// PER-BINDER implicitness (E2): a global's parameter kinds in order — implicit,
    /// explicit, and instance may INTERLEAVE (Lean's `BinderInfo`). Absent means the legacy
    /// "`implicit_args` leading implicits, rest explicit" model.
    binder_kinds: Arc<HashMap<String, Vec<crate::elaborate::ParamKind>>>,

    /// How many LEADING parameters of a global are implicit (declared with `{…}` in the
    /// surface). The surface elaborator inserts and infers that many arguments at each
    /// application of the global, so the user writes `id 0` for `id Int 0`.
    implicit_args: Arc<HashMap<String, usize>>,

    /// Registered STRUCTURES (Rung 0c): a structure type name → its metadata. Only
    /// these one-constructor inductives get DEFINITIONAL ETA (`p ≡ ⟨p.1, …, p.n⟩`),
    /// keyed here so the rule is local and testable, never inferred for arbitrary
    /// one-constructor inductives.
    structures: Arc<HashMap<String, StructInfo>>,

    /// MUTUAL inductive blocks (K3): each member name → the full ordered list of the
    /// block's members. `Even`/`Odd`, `Tree`/`Forest` — an inductive registered
    /// alone maps to nothing (its recursor recurses only on itself). The mutual
    /// recursor derivation reads this to give each member a motive and to route a
    /// recursive occurrence of a SIBLING to the sibling's fixpoint.
    mutual_blocks: Arc<HashMap<String, Vec<String>>>,
}

/// One member of a MUTUAL inductive block: its name, arity sort, uniform parameter
/// count, and constructors (name + full type). Constructor types may reference ANY
/// member of the block — that is the whole point of a mutual declaration.
#[derive(Clone, Debug)]
pub struct MutualInductive {
    /// The inductive's name (e.g. `Even`).
    pub name: String,
    /// Its arity sort (e.g. `Nat → Prop`).
    pub sort: Term,
    /// How many leading arity arguments are uniform parameters (the rest are indices).
    pub num_params: usize,
    /// Constructors: `(name, full type)`, types possibly mentioning sibling members.
    pub constructors: Vec<(String, Term)>,
}

/// Metadata for a registered structure (record): its single constructor, how many
/// leading type PARAMETERS it takes, and the projection function names in field order.
#[derive(Clone, Debug)]
pub struct StructInfo {
    /// The constructor name (e.g. `Prod_mk`).
    pub mk: String,
    /// Number of leading type parameters (e.g. 2 for `Prod A B`).
    pub num_params: usize,
    /// Projection definition names, in field order (e.g. `[Prod_fst, Prod_snd]`).
    pub projections: Vec<String>,
}

impl Context {
    /// Create an empty context.
    pub fn new() -> Self {
        Context {
            bindings: HashMap::new(),
            inductives: Arc::new(HashMap::new()),
            constructors: Arc::new(HashMap::new()),
            inductive_params: Arc::new(HashMap::new()),
            constructor_order: Arc::new(HashMap::new()),
            declarations: Arc::new(HashMap::new()),
            definitions: Arc::new(HashMap::new()),
            hints: Arc::new(Vec::new()),
            universe_polys: Arc::new(HashMap::new()),
            instances: Arc::new(Vec::new()),
            coercions: Arc::new(Vec::new()),
            binder_kinds: Arc::new(HashMap::new()),
            implicit_args: Arc::new(HashMap::new()),
            structures: Arc::new(HashMap::new()),
            mutual_blocks: Arc::new(HashMap::new()),
        }
    }

    /// Record structure metadata (used by [`Context::add_structure`]).
    pub fn register_struct_info(&mut self, name: &str, info: StructInfo) {
        Arc::make_mut(&mut self.structures).insert(name.to_string(), info);
    }

    /// The structure metadata for an inductive type name, if it is a registered
    /// structure (record). `None` for ordinary inductives — eta never fires for them.
    pub fn struct_info(&self, name: &str) -> Option<&StructInfo> {
        self.structures.get(name)
    }

    /// If `ctor` is the constructor of a registered structure, return `(structure
    /// name, its info)`. Used to detect an η-expandable constructor head.
    pub fn struct_of_constructor(&self, ctor: &str) -> Option<(&str, &StructInfo)> {
        let ind = self.constructor_inductive(ctor)?;
        let info = self.structures.get(ind)?;
        Some((ind, info))
    }

    /// Record that the global `name` has `count` leading implicit parameters, so the
    /// surface elaborator inserts that many inferred arguments at each application.
    pub fn set_implicit_args(&mut self, name: &str, count: usize) {
        Arc::make_mut(&mut self.implicit_args).insert(name.to_string(), count);
    }

    /// How many leading parameters of `name` are implicit (0 if none/unknown).
    pub fn implicit_args(&self, name: &str) -> usize {
        self.implicit_args.get(name).copied().unwrap_or(0)
    }

    /// Register a typeclass instance: a `value` of type `ty` (e.g. `mk Nat Zero` of type
    /// `Inhabited Nat`). The elaborator resolves an instance-implicit argument by
    /// searching these for a `ty` that unifies with the required class type.
    pub fn add_instance(&mut self, ty: Term, value: Term) {
        Arc::make_mut(&mut self.instances).push((ty, value));
    }

    /// Register a coercion `coe : from → to` — the elaborator may insert it when an
    /// argument of type `from` appears where `to` is expected.
    pub fn add_coercion(&mut self, from: Term, to: Term, coe: Term) {
        Arc::make_mut(&mut self.coercions).push((from, to, coe));
    }

    /// All registered coercions, as `(from, to, coe)` triples.
    pub fn coercions(&self) -> &[(Term, Term, Term)] {
        &self.coercions
    }

    /// Record a global's per-parameter kinds (implicit/explicit/instance, in order), so the
    /// elaborator can insert implicit and instance arguments at their real positions.
    pub fn set_binder_kinds(&mut self, name: &str, kinds: Vec<crate::elaborate::ParamKind>) {
        Arc::make_mut(&mut self.binder_kinds).insert(name.to_string(), kinds);
    }

    /// A global's per-parameter kinds, if recorded.
    pub fn binder_kinds(&self, name: &str) -> Option<&[crate::elaborate::ParamKind]> {
        self.binder_kinds.get(name).map(|v| v.as_slice())
    }

    /// All registered typeclass instances, as `(type, value)` pairs.
    pub fn instances(&self) -> &[(Term, Term)] {
        &self.instances
    }

    /// Register a universe-polymorphic definition `name.{params} : ty := body`. A
    /// `Term::Const { name, levels }` reference later instantiates `params` with `levels`
    /// (the `.{ℓ…}` syntax), so one definition is reused at every level.
    pub fn add_universe_poly(&mut self, name: &str, params: Vec<String>, ty: Term, body: Term) {
        Arc::make_mut(&mut self.universe_polys).insert(name.to_string(), (params, ty, body));
    }

    /// Look up a universe-polymorphic definition: `(universe params, type, body)`.
    pub fn get_universe_poly(&self, name: &str) -> Option<&(Vec<String>, Term, Term)> {
        self.universe_polys.get(name)
    }

    /// Add a local binding to this context (mutates in place).
    pub fn add(&mut self, name: &str, ty: Term) {
        self.bindings.insert(name.to_string(), Arc::new(ty));
    }

    /// Look up a local variable's type in the context.
    pub fn get(&self, name: &str) -> Option<&Term> {
        self.bindings.get(name).map(|t| t.as_ref())
    }

    /// Create a new context extended with an additional local binding.
    ///
    /// Does not mutate the original context.
    pub fn extend(&self, name: &str, ty: Term) -> Context {
        let mut new_ctx = self.clone();
        new_ctx.add(name, ty);
        new_ctx
    }

    /// Register an inductive type.
    ///
    /// The `sort` is the type of the inductive (e.g., Type 0 for Nat).
    ///
    /// All of the arity is treated as uniform parameters (0 indices) unless
    /// [`set_inductive_params`](Self::set_inductive_params) records a smaller parameter
    /// count — see [`add_indexed_inductive`](Self::add_indexed_inductive).
    pub fn add_inductive(&mut self, name: &str, sort: Term) {
        Arc::make_mut(&mut self.inductives).insert(name.to_string(), sort);
    }

    /// Register a STRUCTURE (record) `{name} (params) := {name}_mk (fields)` — a
    /// one-constructor inductive with auto-derived projections and definitional eta.
    ///
    /// `params` are the leading type parameters `(A : Type 0)`, `(B : Type 0)`, …;
    /// `fields` are `(fst : A)`, `(snd : B)`, … where a field type may reference the
    /// params and any EARLIER field (by name). Registers:
    /// - the inductive `{name} : Π(params). Type 0`,
    /// - the constructor `{name}_mk : Π(params). Π(fields). {name} params`,
    /// - a projection `{name}_{fieldᵢ} : Π(params). Π(s:{name} params). Tᵢ` for each
    ///   field (its body a `match` on `s`), and
    /// - the [`StructInfo`] that gates the eta rule.
    ///
    /// The structure lives in `Type 0` (fields over `Type 0` carriers) — the common
    /// case for the algebraic hierarchy.
    pub fn add_structure(
        &mut self,
        name: &str,
        params: &[(&str, Term)],
        fields: &[(&str, Term)],
    ) {
        let g = |s: &str| Term::Global(s.to_string());
        let var = |s: &str| Term::Var(s.to_string());
        // Wrap `body` in a Π / λ telescope.
        let pis = |tele: &[(&str, Term)], body: Term| {
            tele.iter().rev().fold(body, |acc, (p, t)| Term::Pi {
                param: p.to_string(),
                param_type: Box::new(t.clone()),
                body_type: Box::new(acc),
            })
        };
        let lams = |tele: &[(&str, Term)], body: Term| {
            tele.iter().rev().fold(body, |acc, (p, t)| Term::Lambda {
                param: p.to_string(),
                param_type: Box::new(t.clone()),
                body: Box::new(acc),
            })
        };
        // `{name} A B …` — the structure applied to its parameter variables.
        let s_applied = params.iter().fold(g(name), |acc, (p, _)| {
            Term::App(Box::new(acc), Box::new(var(p)))
        });

        // 1. The inductive.
        let ind_type = pis(params, Term::Sort(Universe::Type(0)));
        self.add_indexed_inductive(name, ind_type, params.len());

        // 2. The constructor.
        let mk = format!("{name}_mk");
        let ctor_type = pis(params, pis(fields, s_applied.clone()));
        self.add_constructor(&mk, name, ctor_type);

        // 3. The projections.
        let mut proj_names = Vec::new();
        for (i, (fname, ftype)) in fields.iter().enumerate() {
            let proj = format!("{name}_{fname}");
            proj_names.push(proj.clone());

            // Rewrite earlier field references `f_j` (j < i) to `proj_j params disc`,
            // for a chosen discriminant term.
            let field_of = |disc: &Term, ty: &Term| -> Term {
                let mut out = ty.clone();
                for (j, (fj, _)) in fields.iter().enumerate().take(i) {
                    let proj_j_applied = params
                        .iter()
                        .fold(g(&format!("{name}_{fj}")), |acc, (p, _)| {
                            Term::App(Box::new(acc), Box::new(var(p)))
                        });
                    let proj_j_applied = Term::App(Box::new(proj_j_applied), Box::new(disc.clone()));
                    out = crate::type_checker::substitute(&out, fj, &proj_j_applied);
                }
                out
            };

            // Projection type: Π(params). Π(s : {name} params). Tᵢ[fⱼ := projⱼ params s].
            let ret_ty = field_of(&var("s"), ftype);
            let proj_type = pis(
                params,
                Term::Pi {
                    param: "s".to_string(),
                    param_type: Box::new(s_applied.clone()),
                    body_type: Box::new(ret_ty),
                },
            );

            // Body: λparams. λ(s). match s return (λ(s✧). Tᵢ[fⱼ := projⱼ params s✧])
            //                       with | mk => λ(fields). fieldᵢ
            let motive = Term::Lambda {
                param: "s✧".to_string(),
                param_type: Box::new(s_applied.clone()),
                body: Box::new(field_of(&var("s✧"), ftype)),
            };
            let case = lams(fields, var(fname));
            let match_term = Term::Match {
                discriminant: Box::new(var("s")),
                motive: Box::new(motive),
                cases: vec![case],
            };
            let body = lams(
                params,
                Term::Lambda {
                    param: "s".to_string(),
                    param_type: Box::new(s_applied.clone()),
                    body: Box::new(match_term),
                },
            );
            self.add_definition(proj, proj_type, body);
        }

        self.register_struct_info(
            name,
            StructInfo { mk, num_params: params.len(), projections: proj_names },
        );
    }

    /// Register an INDEXED inductive: `sort` is its full arity `Π(params). Π(indices).
    /// Sort`, of which the first `num_params` leading arguments are uniform parameters and
    /// the rest are indices that vary per constructor (e.g. `Eq` with `num_params == 2`).
    pub fn add_indexed_inductive(&mut self, name: &str, sort: Term, num_params: usize) {
        self.add_inductive(name, sort);
        self.set_inductive_params(name, num_params);
    }

    /// Record how many leading arguments of `name`'s arity are uniform parameters.
    pub fn set_inductive_params(&mut self, name: &str, num_params: usize) {
        Arc::make_mut(&mut self.inductive_params).insert(name.to_string(), num_params);
    }

    /// The full arity of an inductive — the number of leading `Π`s in its sort (`Nat` →
    /// 0, `TList : Type → Type` → 1, `Eq : Type → A → A → Prop` → 3). `0` for an unknown
    /// name.
    pub fn inductive_arity(&self, name: &str) -> usize {
        self.inductives.get(name).map(count_leading_pis).unwrap_or(0)
    }

    /// The EXPLICITLY declared parameter count for `name`, or `None` if the inductive was
    /// registered without one. Reduction uses this to skip exactly the parameters of an
    /// indexed constructor (`refl A x` → 2), falling back to a syntactic heuristic for the
    /// legacy inductives that never declared a split — so their ι-reduction is untouched.
    pub fn inductive_declared_params(&self, name: &str) -> Option<usize> {
        self.inductive_params.get(name).copied()
    }

    /// How many leading arguments of `name`'s arity are uniform PARAMETERS. Defaults to
    /// the full arity (so a non-indexed inductive is all parameters, 0 indices).
    pub fn inductive_num_params(&self, name: &str) -> usize {
        self.inductive_params
            .get(name)
            .copied()
            .unwrap_or_else(|| self.inductive_arity(name))
    }

    /// How many trailing arguments of `name`'s arity are INDICES (arity − parameters).
    pub fn inductive_num_indices(&self, name: &str) -> usize {
        self.inductive_arity(name).saturating_sub(self.inductive_num_params(name))
    }

    /// Register a constructor for an inductive type.
    ///
    /// The `ty` is the full type of the constructor
    /// (e.g., `Nat` for Zero, `Nat -> Nat` for Succ).
    ///
    /// Constructors are tracked in registration order for match expressions.
    pub fn add_constructor(&mut self, name: &str, inductive: &str, ty: Term) {
        Arc::make_mut(&mut self.constructors)
            .insert(name.to_string(), (inductive.to_string(), ty));

        // Track constructor order for this inductive
        Arc::make_mut(&mut self.constructor_order)
            .entry(inductive.to_string())
            .or_default()
            .push(name.to_string());
    }

    /// Add a declaration (typed assumption/hypothesis).
    ///
    /// Used for proof certification where hypotheses are assumed.
    /// Example: h1 : P -> Q
    pub fn add_declaration(&mut self, name: &str, ty: Term) {
        Arc::make_mut(&mut self.declarations).insert(name.to_string(), ty);
    }

    /// Register a definition: name : type := body
    ///
    /// Definitions are transparent and unfold during normalization (delta reduction).
    /// This distinguishes them from declarations (axioms) which have no body.
    pub fn add_definition(&mut self, name: String, ty: Term, body: Term) {
        Arc::make_mut(&mut self.definitions).insert(name, (ty, body));
    }

    /// Look up a global definition (inductive, constructor, definition, or declaration).
    ///
    /// Returns the type of the global.
    pub fn get_global(&self, name: &str) -> Option<&Term> {
        // Check inductives first
        if let Some(sort) = self.inductives.get(name) {
            return Some(sort);
        }
        // Check constructors
        if let Some((_, ty)) = self.constructors.get(name) {
            return Some(ty);
        }
        // Check definitions (return type, not body)
        if let Some((ty, _)) = self.definitions.get(name) {
            return Some(ty);
        }
        // Check declarations (axioms)
        self.declarations.get(name)
    }

    /// Check if a name is a definition (has a body that can be unfolded).
    pub fn is_definition(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    /// Get the body of a definition, if it exists.
    ///
    /// Returns None for axioms, constructors, and inductives (only definitions have bodies).
    pub fn get_definition_body(&self, name: &str) -> Option<&Term> {
        self.definitions.get(name).map(|(_, body)| body)
    }

    /// Get the type of a definition, if it exists.
    pub fn get_definition_type(&self, name: &str) -> Option<&Term> {
        self.definitions.get(name).map(|(ty, _)| ty)
    }

    /// Check if a name is a constructor.
    pub fn is_constructor(&self, name: &str) -> bool {
        self.constructors.contains_key(name)
    }

    /// Get the inductive type a constructor belongs to.
    pub fn constructor_inductive(&self, name: &str) -> Option<&str> {
        self.constructors.get(name).map(|(ind, _)| ind.as_str())
    }

    /// Check if a name is an inductive type.
    pub fn is_inductive(&self, name: &str) -> bool {
        self.inductives.contains_key(name)
    }

    /// Get all constructors for an inductive type, in registration order.
    ///
    /// Returns a vector of (constructor_name, constructor_type) pairs.
    pub fn get_constructors(&self, inductive: &str) -> Vec<(&str, &Term)> {
        self.constructor_order
            .get(inductive)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| {
                        self.constructors
                            .get(name)
                            .map(|(_, ty)| (name.as_str(), ty))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Iterate over all declarations (hypotheses).
    ///
    /// Used by the certifier to find hypothesis by type.
    pub fn iter_declarations(&self) -> impl Iterator<Item = (&str, &Term)> {
        self.declarations.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate over all definitions.
    ///
    /// Used by the UI to display definitions.
    pub fn iter_definitions(&self) -> impl Iterator<Item = (&str, &Term, &Term)> {
        self.definitions.iter().map(|(k, (ty, body))| (k.as_str(), ty, body))
    }

    /// Iterate over all inductive types.
    ///
    /// Used by the UI to display inductive types.
    pub fn iter_inductives(&self) -> impl Iterator<Item = (&str, &Term)> {
        self.inductives.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Add a constructor with strict positivity checking.
    ///
    /// Returns an error if the inductive type appears negatively in the
    /// constructor type. This prevents paradoxes like:
    /// ```text
    /// Inductive Bad := Cons : (Bad -> False) -> Bad
    /// ```
    pub fn add_constructor_checked(
        &mut self,
        name: &str,
        inductive: &str,
        ty: Term,
    ) -> crate::error::KernelResult<()> {
        // Check strict positivity first
        crate::positivity::check_positivity(inductive, name, &ty)?;
        // Then the CIC universe constraint (a `Type k` inductive cannot store a field of a
        // larger sort — the Girard/Hurkens inconsistency).
        crate::type_checker::check_constructor_universes(self, inductive, name, &ty)?;

        // If it passes, add the constructor normally
        self.add_constructor(name, inductive, ty);
        Ok(())
    }

    /// Register a MUTUAL block of inductives whose constructors may reference one
    /// another (`Even`/`Odd`, `Tree`/`Forest`). Strict positivity is checked over the
    /// WHOLE block up front — a sibling occurrence is a recursive occurrence, a
    /// sibling in a negative position is a cross-block paradox and rejected — and the
    /// registration is TRANSACTIONAL: if any constructor violates positivity, nothing
    /// is added. On success every member's header (with its parameter split), every
    /// constructor, and the block-membership registry are populated, ready for the
    /// auto-derived mutual recursors.
    pub fn add_mutual_inductives(
        &mut self,
        block: &[MutualInductive],
    ) -> crate::error::KernelResult<()> {
        let names: Vec<&str> = block.iter().map(|m| m.name.as_str()).collect();
        // 1. Positivity of every constructor against the whole block, BEFORE mutating.
        for member in block {
            for (cname, cty) in &member.constructors {
                crate::positivity::check_positivity_mutual(&names, cname, cty)?;
            }
        }
        // 1b. UNIVERSE CONSISTENCY of every constructor — checked in a TEMP env carrying the
        // block's headers (so a recursive/sibling field resolves), before mutating `self`,
        // keeping the whole registration transactional.
        {
            let mut temp = self.clone();
            for member in block {
                temp.add_indexed_inductive(&member.name, member.sort.clone(), member.num_params);
            }
            for member in block {
                for (cname, cty) in &member.constructors {
                    crate::type_checker::check_constructor_universes(&temp, &member.name, cname, cty)?;
                }
            }
        }
        // 2. All headers first (so each constructor's sibling references resolve).
        for member in block {
            self.add_indexed_inductive(&member.name, member.sort.clone(), member.num_params);
        }
        // 3. All constructors.
        for member in block {
            for (cname, cty) in &member.constructors {
                self.add_constructor(cname, &member.name, cty.clone());
            }
        }
        // 4. Record block membership (only for a genuine block of ≥ 2 members).
        if block.len() > 1 {
            let members: Vec<String> = block.iter().map(|m| m.name.clone()).collect();
            let reg = Arc::make_mut(&mut self.mutual_blocks);
            for member in block {
                reg.insert(member.name.clone(), members.clone());
            }
        }
        Ok(())
    }

    /// The mutual block `name` belongs to (the full ordered member list), or `None`
    /// if `name` is a standalone inductive. Used by the recursor derivation to give
    /// every block member a motive and route sibling recursion.
    pub fn mutual_block_of(&self, name: &str) -> Option<&[String]> {
        self.mutual_blocks.get(name).map(|v| v.as_slice())
    }

    /// Register a NESTED inductive (`RTree := rnode : TList RTree → RTree`) by compiling
    /// it — via the UNTRUSTED [`inductive_compile`](crate::inductive_compile) front-end —
    /// to a mutual block plus conversion isos, then registering the block and CHECKING
    /// every iso through the trusted kernel. A mis-compiled sibling is caught by mutual
    /// positivity; a mis-typed iso is caught here (its inferred type must match its
    /// declared conversion type). Soundness rests entirely on those trusted checks — the
    /// compiler adds no trusted code, exactly as Lean lowers nested inductives to mutual.
    pub fn add_nested_inductive(
        &mut self,
        decl: &crate::inductive_compile::NestedDecl,
    ) -> crate::error::KernelResult<crate::inductive_compile::NestedInfo> {
        let compiled = crate::inductive_compile::compile_nested(self, decl)?;
        // The mutual block is checked by the trusted mutual machinery (block positivity).
        self.add_mutual_inductives(&compiled.block)?;
        // Each iso is KERNEL-CHECKED before it is trusted: infer its type and require it
        // be the declared conversion type. A wrong iso is rejected here.
        for (name, ty, body) in &compiled.isos {
            let inferred = crate::infer_type(self, body)?;
            if !crate::is_subtype(self, &inferred, ty) || !crate::is_subtype(self, ty, &inferred) {
                return Err(crate::error::KernelError::CertificationError(format!(
                    "nested-compile: iso '{name}' inferred type {inferred} ≠ declared {ty}"
                )));
            }
            self.add_definition(name.clone(), ty.clone(), body.clone());
        }
        Ok(crate::inductive_compile::NestedInfo {
            siblings: compiled.siblings,
            isos: compiled.iso_names,
        })
    }

    /// Register a theorem as a hint for the auto tactic.
    ///
    /// Hints are theorems that auto will try to apply when decision
    /// procedures fail. This allows auto to "learn" from proven theorems.
    pub fn add_hint(&mut self, name: &str) {
        if !self.hints.contains(&name.to_string()) {
            Arc::make_mut(&mut self.hints).push(name.to_string());
        }
    }

    /// Get all registered hints.
    ///
    /// Returns the names of theorems registered as hints.
    pub fn get_hints(&self) -> &[String] {
        &self.hints
    }

    /// Check if a theorem is registered as a hint.
    pub fn is_hint(&self, name: &str) -> bool {
        self.hints.contains(&name.to_string())
    }
}

/// Count the leading `Π`s of a term — an inductive's arity, or a constructor's parameter
/// count.
fn count_leading_pis(t: &Term) -> usize {
    let mut n = 0;
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        n += 1;
        cur = body_type;
    }
    n
}
