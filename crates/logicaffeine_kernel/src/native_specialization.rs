//! Private names for the complete nested specialization, without replacing any source declaration.
//! Shared by ordinary and bounded generation; this is syntax preparation, not a checking bypass.
use super::{Elimination, KernelError, Term};
use crate::Context;
use std::collections::HashMap;

pub(super) trait Meter {
    type Error;
    fn work(&mut self, n: u64) -> Result<(), Self::Error>;
    fn reserve(&mut self, bytes: u64) -> Result<(), Self::Error>;
    fn inventory(&mut self, n: usize) -> Result<(), Self::Error>;
    fn admit(&mut self, term: &Term) -> Result<(), Self::Error>;
    fn image(
        &mut self,
        name: &str,
        parameters: &[(String, Term)],
        levels: &[String],
    ) -> Result<Term, Self::Error>;
    fn invalid(&self, message: &str) -> Self::Error;
}
pub(super) struct Ordinary;
impl Meter for Ordinary {
    type Error = KernelError;
    fn work(&mut self, _: u64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn reserve(&mut self, _: u64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn inventory(&mut self, _: usize) -> Result<(), Self::Error> {
        Ok(())
    }
    fn admit(&mut self, _: &Term) -> Result<(), Self::Error> {
        Ok(())
    }
    fn image(
        &mut self,
        name: &str,
        parameters: &[(String, Term)],
        levels: &[String],
    ) -> Result<Term, Self::Error> {
        Ok(super::apps(
            super::reference_at(name, levels),
            parameters.iter().map(|(name, _)| super::v(name)).collect(),
        ))
    }
    fn invalid(&self, message: &str) -> Self::Error {
        KernelError::CertificationError(message.into())
    }
}
fn copy<M: Meter>(m: &mut M, name: &str) -> Result<String, M::Error> {
    m.work(name.len() as u64 + 1)?;
    m.reserve(name.len() as u64 + 64)?;
    Ok(name.to_owned())
}
fn contains<M: Meter>(m: &mut M, name: &str, token: &str) -> Result<bool, M::Error> {
    m.work((name.len() as u64 + 1).saturating_mul(token.len() as u64 + 1))?;
    Ok(name.contains(token))
}

/// The token is absent from every existing global name, including generated auxiliaries and
/// pending/referenced names. Consequently names automatically derived from our new identifiers
/// are fresh too, regardless of whether the generator prefixes or suffixes the family name.
fn namespace<M: Meter>(
    ctx: &Context,
    elimination: &Elimination,
    m: &mut M,
) -> Result<String, M::Error> {
    let mut counter = 0usize;
    loop {
        m.work(96)?;
        m.reserve(128)?;
        let token = format!("arena.native.private.{counter}.");
        // Hash-map iteration visits capacity as well as occupied buckets.
        m.work(ctx.private_specialization_name_capacity() as u64)?;
        let mut conflict = false;
        for name in ctx.private_specialization_names() {
            if contains(m, name, &token)? {
                conflict = true;
                break;
            }
        }
        if !conflict {
            for member in &elimination.block {
                if contains(m, &member.name, &token)? {
                    conflict = true;
                    break;
                }
                for (name, _) in &member.constructors {
                    if contains(m, name, &token)? {
                        conflict = true;
                        break;
                    }
                }
                if conflict {
                    break;
                }
            }
        }
        if !conflict {
            return Ok(token);
        }
        counter = counter
            .checked_add(1)
            .ok_or_else(|| m.invalid("private specialization namespace exhausted"))?;
    }
}
fn fresh<M: Meter>(
    m: &mut M,
    token: &str,
    family: usize,
    constructor: Option<usize>,
) -> Result<String, M::Error> {
    let size = token.len() as u64 + 64;
    m.work(size)?;
    m.reserve(size + 64)?;
    Ok(match constructor {
        None => format!("{token}I{family}"),
        Some(c) => format!("{token}C{family}.{c}"),
    })
}

// Include every resident key even for an ordinary constant-time hash lookup: collisions may
// compare all strings. The +1 per key also covers empty names and bucket probes.
fn map_work<M: Meter>(m: &mut M, query: &str, resident_key_work: u64) -> Result<(), M::Error> {
    m.work(
        resident_key_work
            .saturating_add(query.len() as u64)
            .saturating_add(1),
    )
}

/// Change global identities only. Mutable traversal reuses every Box/vector and never changes
/// binder strings, local variables, universe syntax, mutual-fix ordering or application shape.
fn rename<M: Meter>(
    term: &mut Term,
    names: &HashMap<String, String>,
    resident_key_work: u64,
    m: &mut M,
) -> Result<(), M::Error> {
    m.admit(term)?;
    m.inventory(1)?;
    let mut todo = vec![term];
    while let Some(term) = todo.pop() {
        m.work(1)?;
        match term {
            Term::Global(name) | Term::Const { name, .. } => {
                map_work(m, name, resident_key_work)?;
                if let Some(replacement) = names.get(name) {
                    *name = copy(m, replacement)?;
                }
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                m.inventory(2)?;
                todo.push(param_type);
                todo.push(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                m.inventory(2)?;
                todo.push(param_type);
                todo.push(body);
            }
            Term::App(f, a) => {
                m.inventory(2)?;
                todo.push(f);
                todo.push(a);
            }
            Term::Let {
                ty, value, body, ..
            } => {
                m.inventory(3)?;
                todo.push(ty);
                todo.push(value);
                todo.push(body);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                m.inventory(cases.len().saturating_add(2))?;
                todo.push(discriminant);
                todo.push(motive);
                todo.extend(cases.iter_mut());
            }
            Term::Fix { body, .. } => {
                m.inventory(1)?;
                todo.push(body);
            }
            Term::MutualFix { defs, .. } => {
                m.inventory(defs.len())?;
                todo.extend(defs.iter_mut().map(|(_, body)| body));
            }
            Term::Annotated { term, .. } => {
                m.inventory(1)?;
                todo.push(term);
            }
            Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
        }
    }
    Ok(())
}

/// Keep elimination discovery order and restore all private identities to the original source.
/// Existing restoration images stay in the original namespace and must NOT themselves be renamed.
pub(super) fn prepare<M: Meter>(
    ctx: &Context,
    mut elimination: Elimination,
    levels: &[String],
    m: &mut M,
) -> Result<Elimination, M::Error> {
    m.inventory(elimination.block.len())?;
    let token = namespace(ctx, &elimination, m)?;
    m.work(ctx.private_specialization_name_capacity() as u64)?;
    let context_key_work = ctx
        .private_specialization_names()
        .fold(0u64, |total, name| {
            total.saturating_add(name.len() as u64).saturating_add(1)
        });
    // Admit complete inventories before allocating. Fixed capacity avoids uncharged growth
    // rehashes, which would otherwise revisit every previously inserted long key.
    let mut identity_count = 0usize;
    let mut name_key_work = 0u64;
    for member in &elimination.block {
        for name in
            std::iter::once(&member.name).chain(member.constructors.iter().map(|(name, _)| name))
        {
            m.work(1)?;
            identity_count = identity_count
                .checked_add(1)
                .ok_or_else(|| m.invalid("private identity inventory overflow"))?;
            name_key_work = name_key_work
                .saturating_add(name.len() as u64)
                .saturating_add(1);
        }
    }
    m.inventory(identity_count)?;
    m.reserve(
        (identity_count as u64)
            .saturating_mul(std::mem::size_of::<(String, String)>() as u64)
            .saturating_mul(4),
    )?;
    let mut names = HashMap::with_capacity(identity_count);
    for (i, member) in elimination.block.iter().enumerate() {
        m.inventory(member.constructors.len().saturating_add(1))?;
        let key = copy(m, &member.name)?;
        map_work(m, &key, name_key_work)?;
        if names.insert(key, fresh(m, &token, i, None)?).is_some() {
            return Err(m.invalid("duplicate specialized family identity"));
        }
        for (j, (name, _)) in member.constructors.iter().enumerate() {
            let key = copy(m, name)?;
            map_work(m, &key, name_key_work)?;
            if names.insert(key, fresh(m, &token, i, Some(j))?).is_some() {
                return Err(m.invalid("duplicate specialized constructor identity"));
            }
        }
    }
    m.inventory(elimination.restoration.len())?;
    m.reserve(
        (elimination.restoration.len() as u64)
            .saturating_mul(std::mem::size_of::<(String, Term)>() as u64)
            .saturating_mul(4),
    )?;
    let image_key_work = elimination
        .restoration
        .iter()
        .fold(0u64, |total, (name, _)| {
            total.saturating_add(name.len() as u64).saturating_add(1)
        });
    let mut images = HashMap::with_capacity(elimination.restoration.len());
    for (name, image) in elimination.restoration {
        map_work(m, &name, image_key_work)?;
        if images.insert(name, image).is_some() {
            return Err(m.invalid("duplicate native restoration identity"));
        }
    }
    m.inventory(identity_count)?;
    m.reserve(
        (identity_count as u64).saturating_mul(std::mem::size_of::<(String, Term)>() as u64),
    )?;
    let mut restoration = Vec::with_capacity(identity_count);
    for member in &mut elimination.block {
        m.inventory(member.constructors.len().saturating_add(1))?;
        // Build each image while its old name is still available; only then move the new name.
        for name in std::iter::once(&mut member.name)
            .chain(member.constructors.iter_mut().map(|(name, _)| name))
        {
            map_work(m, name, image_key_work)?;
            let image = match images.remove(name) {
                Some(image) => image,
                None => {
                    // Both context queries may compare resident strings under hash collisions.
                    map_work(m, name, context_key_work)?;
                    map_work(m, name, context_key_work)?;
                    if ctx.get_global(name).is_none() && ctx.poly_declaration(name).is_none() {
                        return Err(m.invalid(
                            "specialized auxiliary is missing its source restoration image",
                        ));
                    }
                    m.image(name, &elimination.parameters, levels)?
                }
            };
            map_work(m, name, name_key_work)?;
            let replacement = names
                .get(name)
                .ok_or_else(|| m.invalid("missing private specialization identity"))?;
            restoration.push((copy(m, replacement)?, image));
            *name = copy(m, replacement)?;
        }
        rename(&mut member.sort, &names, name_key_work, m)?;
        for (_, ty) in &mut member.constructors {
            rename(ty, &names, name_key_work, m)?;
        }
    }
    if !images.is_empty() {
        return Err(m.invalid("native restoration contains a foreign specialization identity"));
    }
    Ok(Elimination {
        block: elimination.block,
        restoration,
        parameters: elimination.parameters,
    })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/inductive_compile/native_specialization/controls.rs"]
mod controls;
