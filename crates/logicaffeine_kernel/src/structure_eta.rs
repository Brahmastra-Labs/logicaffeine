//! Structure η — the one door that grants it.
//!
//! Registering [`StructInfo`] for an inductive `I` switches on the conversion rule
//! `x ≡ mk p̄ (proj₀ p̄ x) … (projₙ p̄ x)` for every `x : I p̄` (both kernels' `try_structure_eta`).
//! That rule is sound only when `I` really is a structure and the named projections really are
//! its projections, so η is granted only after [`Context::register_structure_checked`] has
//! decided, in this order:
//!
//! 1. `I` is a declared inductive, not already a structure, and not sealed: a native nested
//!    positivity check that depends on `I` forbids changing it, and the door answers `Ok` only
//!    for a grant it has made;
//! 2. `I` has exactly one constructor and no indices;
//! 3. the constructor binds all of `I`'s parameters;
//! 4. `I`'s mutual block is not recursive: no constructor of any member takes an argument whose
//!    type mentions a member, even under a container or through a definition (Lean's `isRec`,
//!    which its `isStructureLike` requires false — η is for structures, never recursive types);
//! 5. there is exactly one projection per field;
//! 6. each projection is a transparent definition over exactly `I`'s universe parameters;
//! 7. for a generic constructor application `mk p̄ f̄` — parameters and fields as fresh local
//!    variables — `projᵢ p̄ (mk p̄ f̄)` is definitionally `fᵢ`, and
//! 8. well typed at the field's own type `Fᵢ`. Reduction preserves types, so a well-typed
//!    projection that computes its field always passes; this refuses the ill-typed application
//!    that still reduces to `fᵢ` (a projection over a different instance of the structure).
//!
//! The vernacular `Structure` command, the environment certificate's `structure` entry and the
//! Lean reader all call this function; none of them registers η any other way. The public metadata
//! door [`Context::register_struct_info`] asks the same decision before it registers anything, so
//! the only η granted without it is the kernel's own structure builder's, whose constructor and
//! projections it generates itself.
use std::fmt;

use crate::context::{Context, StructInfo};
use crate::term::{Term, Universe};

/// Why structure η was refused. Every variant names the first condition that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureRefusal {
    /// The named inductive is not declared.
    NotAnInductive { inductive: String },
    /// The inductive already has structure η.
    AlreadyAStructure { inductive: String },
    /// A native nested positivity check depends on the inductive, so it cannot change.
    Sealed { inductive: String },
    /// A structure has exactly one constructor and no indices.
    NotStructureShaped {
        inductive: String,
        constructors: usize,
        indices: usize,
    },
    /// The constructor's type binds fewer binders than the inductive has parameters.
    ShortConstructor { constructor: String, params: usize },
    /// The inductive's mutual block is recursive.
    Recursive { inductive: String },
    /// The number of projections differs from the number of fields.
    ProjectionCount {
        inductive: String,
        fields: usize,
        projections: usize,
    },
    /// A projection is not a transparent definition.
    NotADefinition { projection: String },
    /// A projection's universe telescope is not the inductive's.
    UniverseArity {
        projection: String,
        inductive: String,
        expected: usize,
        found: usize,
    },
    /// `projᵢ p̄ (mk p̄ f̄)` is not definitionally `fᵢ`.
    DoesNotCompute {
        projection: String,
        field: usize,
        inductive: String,
    },
    /// `projᵢ p̄ (mk p̄ f̄)` computes `fᵢ` but is ill typed, or not of the field's type.
    WrongFieldType {
        projection: String,
        field: usize,
        inductive: String,
    },
}

impl fmt::Display for StructureRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StructureRefusal::NotAnInductive { inductive } => {
                write!(f, "structure η names `{inductive}`, which is not a declared inductive")
            }
            StructureRefusal::AlreadyAStructure { inductive } => {
                write!(f, "`{inductive}` already has structure η")
            }
            StructureRefusal::Sealed { inductive } => write!(
                f,
                "`{inductive}` is sealed by a native nested positivity check that depends on it, so it cannot gain structure η"
            ),
            StructureRefusal::NotStructureShaped { inductive, constructors, indices } => write!(
                f,
                "`{inductive}` is not a structure: it has {constructors} constructors and {indices} indices, \
                 where a structure has exactly one constructor and no indices"
            ),
            StructureRefusal::ShortConstructor { constructor, params } => {
                write!(f, "`{constructor}` binds fewer than its inductive's {params} parameters")
            }
            StructureRefusal::Recursive { inductive } => {
                write!(f, "`{inductive}` is recursive, and structure η is for structures")
            }
            StructureRefusal::ProjectionCount { inductive, fields, projections } => {
                write!(f, "`{inductive}` has {fields} fields but names {projections} projections")
            }
            StructureRefusal::NotADefinition { projection } => {
                write!(f, "`{projection}` is not a transparent definition")
            }
            StructureRefusal::UniverseArity { projection, inductive, expected, found } => write!(
                f,
                "`{projection}` binds {found} universe parameters, not `{inductive}`'s {expected}"
            ),
            StructureRefusal::WrongFieldType { projection, field, inductive } => write!(
                f,
                "`{projection}` applied to a constructor of `{inductive}` does not have the type of field {field}"
            ),
            StructureRefusal::DoesNotCompute { projection, field, inductive } => {
                write!(f, "`{projection}` does not compute field {field} of `{inductive}`")
            }
        }
    }
}

impl std::error::Error for StructureRefusal {}

impl Context {
    /// Grant structure η to `inductive` through `projections`, after deciding each condition a
    /// `StructureRefusal` variant names: one constructor, no indices, no recursion, and
    /// projections that compute their fields at the fields' types. On refusal the context is
    /// unchanged.
    pub fn register_structure_checked(
        &mut self,
        inductive: &str,
        projections: &[String],
    ) -> Result<(), StructureRefusal> {
        let info = self.structure_eta_evidence(inductive, projections)?;
        self.insert_struct_info(inductive, info);
        Ok(())
    }

    /// Lean's `isRec`, decided for the whole mutual block as Lean's kernel decides it: some
    /// constructor of some member takes an argument whose type mentions a member, directly, under
    /// a container, or through a definition. The door refuses such a family
    /// ([`StructureRefusal::Recursive`]); a caller may ask first to avoid building projections
    /// the door would refuse.
    pub(crate) fn block_is_recursive(&self, inductive: &str) -> bool {
        let block: Vec<&str> = match self.mutual_block_of(inductive) {
            Some(members) => members.iter().map(String::as_str).collect(),
            None => vec![inductive],
        };
        block.iter().any(|member| {
            self.get_constructors(member).into_iter().any(|(_, ty)| {
                let mut rest = ty;
                while let Term::Pi {
                    param_type,
                    body_type,
                    ..
                } = rest
                {
                    if crate::positivity::mentions_inductive_block(self, &block, param_type) {
                        return true;
                    }
                    rest = body_type;
                }
                false
            })
        })
    }

    /// The whole decision, without registering anything: the [`StructInfo`] η would be granted
    /// with, or the first condition that failed. [`Context::register_struct_info`] asks it too.
    pub(crate) fn structure_eta_evidence(
        &self,
        inductive: &str,
        projections: &[String],
    ) -> Result<StructInfo, StructureRefusal> {
        let named = || inductive.to_string();
        if !self.is_inductive(inductive) {
            return Err(StructureRefusal::NotAnInductive { inductive: named() });
        }
        if self.struct_info(inductive).is_some() {
            return Err(StructureRefusal::AlreadyAStructure { inductive: named() });
        }
        if self.check_native_dependency_mutation(inductive).is_err() {
            return Err(StructureRefusal::Sealed { inductive: named() });
        }
        let constructors = self.get_constructors(inductive);
        let indices = self.inductive_num_indices(inductive);
        if constructors.len() != 1 || indices != 0 {
            return Err(StructureRefusal::NotStructureShaped {
                inductive: named(),
                constructors: constructors.len(),
                indices,
            });
        }
        let (mk, mk_ty) = (constructors[0].0.to_string(), constructors[0].1.clone());
        let levels: Vec<Universe> = self
            .poly_declaration(inductive)
            .map(|(params, _)| params.iter().cloned().map(Universe::Var).collect())
            .unwrap_or_default();
        let constant = |name: &str| {
            if levels.is_empty() {
                Term::Global(name.to_string())
            } else {
                Term::Const {
                    name: name.to_string(),
                    levels: levels.clone(),
                }
            }
        };

        let num_params = self.inductive_num_params(inductive);
        let mut params = Vec::with_capacity(num_params);
        let mut rest = mk_ty;
        for i in 0..num_params {
            let Term::Pi {
                param,
                param_type,
                body_type,
            } = rest
            else {
                return Err(StructureRefusal::ShortConstructor {
                    constructor: mk,
                    params: num_params,
                });
            };
            let fresh = format!("structure.parameter.{i}");
            rest = crate::substitute(&body_type, &param, &Term::Var(fresh.clone()));
            params.push((fresh, *param_type));
        }
        let mut fields = Vec::new();
        while let Term::Pi {
            param,
            param_type,
            body_type,
        } = rest
        {
            let fresh = format!("structure.field.{}", fields.len());
            rest = crate::substitute(&body_type, &param, &Term::Var(fresh.clone()));
            fields.push((fresh, *param_type));
        }

        if self.block_is_recursive(inductive) {
            return Err(StructureRefusal::Recursive { inductive: named() });
        }
        if projections.len() != fields.len() {
            return Err(StructureRefusal::ProjectionCount {
                inductive: named(),
                fields: fields.len(),
                projections: projections.len(),
            });
        }

        let mut local = self.clone();
        for (name, ty) in params.iter().chain(fields.iter()) {
            local = local.extend(name, ty.clone());
        }
        let apply_all = |head: Term, args: &mut dyn Iterator<Item = Term>| {
            args.fold(head, |f, a| Term::App(Box::new(f), Box::new(a)))
        };
        let param_terms: Vec<Term> = params.iter().map(|(n, _)| Term::Var(n.clone())).collect();
        let value = apply_all(
            constant(&mk),
            &mut param_terms
                .iter()
                .cloned()
                .chain(fields.iter().map(|(n, _)| Term::Var(n.clone()))),
        );
        for (i, projection) in projections.iter().enumerate() {
            let transparent = self.get_definition_body(projection).is_some()
                || self.get_universe_poly(projection).is_some();
            if !transparent {
                return Err(StructureRefusal::NotADefinition {
                    projection: projection.clone(),
                });
            }
            let arity = self
                .poly_declaration(projection)
                .map(|(p, _)| p.len())
                .unwrap_or(0);
            if arity != levels.len() {
                return Err(StructureRefusal::UniverseArity {
                    projection: projection.clone(),
                    inductive: named(),
                    expected: levels.len(),
                    found: arity,
                });
            }
            let applied = apply_all(
                constant(projection),
                &mut param_terms.iter().cloned().chain([value.clone()]),
            );
            if !crate::type_checker::def_eq(&local, &applied, &Term::Var(fields[i].0.clone())) {
                return Err(StructureRefusal::DoesNotCompute {
                    projection: projection.clone(),
                    field: i,
                    inductive: named(),
                });
            }
            let typed = crate::infer_type(&local, &applied)
                .is_ok_and(|ty| crate::type_checker::def_eq(&local, &ty, &fields[i].1));
            if !typed {
                return Err(StructureRefusal::WrongFieldType {
                    projection: projection.clone(),
                    field: i,
                    inductive: named(),
                });
            }
        }
        Ok(StructInfo {
            mk,
            num_params,
            projections: projections.to_vec(),
            params,
            fields,
            parents: Vec::new(),
            out_params: Vec::new(),
        })
    }
}
