//! Names: the Lean name behind an importer key, and the deterministic, reversible allocation of a
//! LOGOS name for every declaration a document writes.
//!
//! The importer keys a Lean constant structurally (`/s:"LE_Emit"/s:"Color"`), with four fixed
//! identities (`Nat`, `Zero`, `Succ`, the kernel quotient primitives). A LOGOS name is a
//! hierarchical name the term door can spell and whose flat key reads back as itself.
//!
//! Allocation walks the TRIE of Lean names, so a Lean namespace becomes exactly one LOGOS
//! namespace: siblings get distinct components, so two Lean namespaces never merge, and a Lean
//! root the LOGOS prelude already uses (`Eq`, `Nat`, …) moves whole — with everything inside it —
//! under one fresh root (`Lean.Eq`, `Lean.Eq.mpr`), so no Lean name can collide with or merge
//! into a prelude name. The mapping is recorded pair by pair in the correspondence, which is what
//! makes it reversible.

use super::terms::{is_plain_identifier, is_reserved_word};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// One component of a Lean name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LeanPart {
    Str(String),
    Num(u64),
}

/// The Lean name an importer key stands for, or `None` for a constant the importer made itself.
pub(super) fn lean_name(key: &str) -> Option<Vec<LeanPart>> {
    let s = |x: &str| LeanPart::Str(x.to_string());
    match key {
        "Nat" => return Some(vec![s("Nat")]),
        "Zero" => return Some(vec![s("Nat"), s("zero")]),
        "Succ" => return Some(vec![s("Nat"), s("succ")]),
        _ => {}
    }
    for kind in [
        crate::quotient::Kind::Type,
        crate::quotient::Kind::Constructor,
        crate::quotient::Kind::Lift,
        crate::quotient::Kind::Induction,
    ] {
        if key == kind.name() {
            let mut parts = vec![s("Quot")];
            match kind {
                crate::quotient::Kind::Type => {}
                crate::quotient::Kind::Constructor => parts.push(s("mk")),
                crate::quotient::Kind::Lift => parts.push(s("lift")),
                crate::quotient::Kind::Induction => parts.push(s("ind")),
            }
            return Some(parts);
        }
    }
    if let Some(rest) = key.strip_prefix("Nat/") {
        let mut parts = vec![s("Nat")];
        parts.extend(decode_structural(&format!("/{rest}"))?);
        return Some(parts);
    }
    decode_structural(key)
}

/// `/s:"A"/n:3/s:"b"` → `[A, 3, b]`. `None` for anything else.
fn decode_structural(key: &str) -> Option<Vec<LeanPart>> {
    let mut parts = Vec::new();
    let mut rest = key;
    while !rest.is_empty() {
        rest = rest.strip_prefix('/')?;
        if let Some(after) = rest.strip_prefix("s:") {
            let mut stream = serde_json::Deserializer::from_str(after).into_iter::<String>();
            let text = stream.next()?.ok()?;
            let used = stream.byte_offset();
            parts.push(LeanPart::Str(text));
            rest = &after[used..];
        } else if let Some(after) = rest.strip_prefix("n:") {
            let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if digits == 0 {
                return None;
            }
            parts.push(LeanPart::Num(after[..digits].parse().ok()?));
            rest = &after[digits..];
        } else {
            return None;
        }
    }
    (!parts.is_empty()).then_some(parts)
}

/// The Lean spelling of a name: components joined by `.`, a component Lean cannot read bare
/// written `«…»`.
pub(super) fn lean_display(parts: &[LeanPart]) -> String {
    parts
        .iter()
        .map(|p| match p {
            LeanPart::Num(n) => n.to_string(),
            LeanPart::Str(s) if lean_bare(s) => s.clone(),
            LeanPart::Str(s) => format!("\u{ab}{s}\u{bb}"),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn lean_bare(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '\'' | '!' | '?'))
}

/// The LOGOS component a Lean component asks for, before sibling collisions are settled, and
/// whether it had to be changed to be a LOGOS identifier at all.
fn preferred_component(part: &LeanPart) -> (String, bool) {
    match part {
        LeanPart::Str(s) if is_plain_identifier(s) => (s.clone(), false),
        LeanPart::Str(s) => {
            let mut out: String = s
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '_' || c == '\'' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            if !out
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
            {
                out.insert(0, '_');
            }
            if is_reserved_word(&out) || !is_plain_identifier(&out) {
                out.push('_');
            }
            (out, true)
        }
        LeanPart::Num(n) => (format!("_{n}"), true),
    }
}

/// The names the LOGOS prelude already declares, as the allocator needs them.
pub(super) struct Taken {
    /// Every first component of a prelude key.
    pub roots: HashSet<String>,
    /// Every full prelude key.
    pub keys: HashSet<String>,
}

/// One allocation: a LOGOS key for every importer key given.
#[derive(Debug)]
pub(super) struct Allocation {
    pub logos: BTreeMap<String, String>,
    /// The root under which Lean roots the prelude already uses were placed, if any was needed.
    pub relocated_under: Option<String>,
}

/// Allocate a LOGOS key for every Lean-named importer key in `keys`.
///
/// `fixed` are keys whose LOGOS name is decided already (a prelude identity: Lean `Nat` IS the
/// prelude's `Nat`); `reserved` are full LOGOS keys the fresh context will derive on its own (an
/// inductive's recursor family, a definition's equation lemmas) and so must stay free.
pub(super) fn allocate(
    keys: &BTreeSet<String>,
    fixed: &BTreeMap<String, String>,
    taken: &Taken,
    reserved: &dyn Fn(&BTreeMap<String, String>) -> BTreeSet<String>,
) -> Result<Allocation, String> {
    let mut named: BTreeMap<Vec<LeanPart>, String> = BTreeMap::new();
    for key in keys {
        if fixed.contains_key(key) {
            continue;
        }
        let parts = lean_name(key).ok_or_else(|| format!("`{key}` has no Lean name"))?;
        if named.insert(parts.clone(), key.clone()).is_some() {
            return Err(format!(
                "two importer keys denote the Lean name `{}`",
                lean_display(&parts)
            ));
        }
    }

    // A component the fresh context will derive beside an inductive or a definition is refused to
    // the sibling that would take it, and allocation runs again until nothing new is refused.
    let mut refused: BTreeMap<Vec<String>, BTreeSet<String>> = BTreeMap::new();
    for _ in 0..=keys.len() {
        let allocation = allocate_once(&named, fixed, taken, &refused)?;
        let derived = reserved(&allocation.logos);
        let mut changed = false;
        for (key, logos) in &allocation.logos {
            if fixed.contains_key(key) || !derived.contains(logos) {
                continue;
            }
            let mut parts: Vec<String> = logos.split('.').map(str::to_string).collect();
            let last = parts.pop().expect("a LOGOS key has a component");
            changed |= refused.entry(parts).or_default().insert(last);
        }
        if !changed {
            return Ok(allocation);
        }
    }
    Err("LOGOS name allocation did not settle beside the names the context derives".to_string())
}

fn allocate_once(
    named: &BTreeMap<Vec<LeanPart>, String>,
    fixed: &BTreeMap<String, String>,
    taken: &Taken,
    refused: &BTreeMap<Vec<String>, BTreeSet<String>>,
) -> Result<Allocation, String> {
    // The trie of every Lean name, including the namespaces that only prefix others.
    let mut nodes: BTreeSet<Vec<LeanPart>> = BTreeSet::new();
    for parts in named.keys() {
        for len in 1..=parts.len() {
            nodes.insert(parts[..len].to_vec());
        }
    }
    let lean_roots: BTreeSet<String> = nodes
        .iter()
        .filter(|n| n.len() == 1)
        .map(|n| preferred_component(&n[0]).0)
        .collect();
    // A fixed identity is not a trie node (only the names it prefixes are), so a Lean root spelled like
    // a prelude name is never that identity: Lean's class `Zero` is not the prelude's `Zero`, which is
    // Lean's `Nat.zero`.
    let collides_with_prelude =
        |component: &str| taken.roots.contains(component) || taken.keys.contains(component);
    let needs_relocation = |root: &LeanPart| {
        let (c, _) = preferred_component(root);
        collides_with_prelude(&c) || fixed_root_collides(root, named, fixed)
    };
    let relocated_under = if nodes
        .iter()
        .any(|n| n.len() == 1 && needs_relocation(&n[0]))
    {
        let base = "Lean";
        let chosen = std::iter::successors(Some(base.to_string()), |s| Some(format!("{s}'")))
            .find(|c| {
                !taken.roots.contains(c) && !taken.keys.contains(c) && !lean_roots.contains(c)
            })
            .expect("an unbounded sequence of candidates has a free one");
        Some(chosen)
    } else {
        None
    };

    // Children of each LOGOS prefix, settled in a fixed order: names that needed no change first
    // (so a Lean `a_b` keeps its name when a Lean `«a b»` also wants it), then by Lean name.
    let mut logos_of: BTreeMap<Vec<LeanPart>, Vec<String>> = BTreeMap::new();
    let mut used: BTreeMap<Vec<String>, BTreeSet<String>> = BTreeMap::new();
    let mut by_depth: Vec<&Vec<LeanPart>> = nodes.iter().collect();
    by_depth.sort_by(|a, b| {
        let (pa, sa) = preferred_component(a.last().unwrap());
        let (pb, sb) = preferred_component(b.last().unwrap());
        (a.len(), a[..a.len() - 1].to_vec(), sa, pa, *a).cmp(&(
            b.len(),
            b[..b.len() - 1].to_vec(),
            sb,
            pb,
            *b,
        ))
    });
    for node in by_depth {
        let parent: Vec<String> = if node.len() == 1 {
            match &relocated_under {
                Some(root) if needs_relocation(&node[0]) => vec![root.clone()],
                _ => Vec::new(),
            }
        } else {
            logos_of[&node[..node.len() - 1].to_vec()].clone()
        };
        let (preferred, _) = preferred_component(node.last().unwrap());
        let siblings = used.entry(parent.clone()).or_default();
        let refused_here = refused.get(&parent);
        let free = |c: &str| {
            !siblings.contains(c)
                && !refused_here.is_some_and(|r| r.contains(c))
                && (!parent.is_empty()
                    || (!collides_with_prelude(c) && !lean_root_is_relocation(c, &relocated_under)))
        };
        let component = std::iter::successors(Some(preferred.clone()), |s| Some(format!("{s}'")))
            .find(|c| free(c))
            .expect("an unbounded sequence of candidates has a free one");
        siblings.insert(component.clone());
        let mut path = parent;
        path.push(component);
        logos_of.insert(node.clone(), path);
    }

    let mut logos = BTreeMap::new();
    for (key, name) in fixed {
        logos.insert(key.clone(), name.clone());
    }
    for (parts, key) in named {
        logos.insert(key.clone(), logos_of[parts].join("."));
    }
    Ok(Allocation {
        logos,
        relocated_under,
    })
}

fn lean_root_is_relocation(component: &str, relocated_under: &Option<String>) -> bool {
    relocated_under.as_deref() == Some(component)
}

/// A Lean root whose own name is fixed to a prelude identity (`Nat`) while other Lean names live
/// under it (`Nat.add`): those others must not land inside the prelude's namespace.
fn fixed_root_collides(
    root: &LeanPart,
    named: &BTreeMap<Vec<LeanPart>, String>,
    fixed: &BTreeMap<String, String>,
) -> bool {
    let root_key_fixed = fixed
        .keys()
        .any(|k| lean_name(k).is_some_and(|p| p.len() == 1 && &p[0] == root));
    root_key_fixed && named.keys().any(|p| p.len() > 1 && &p[0] == root)
}
