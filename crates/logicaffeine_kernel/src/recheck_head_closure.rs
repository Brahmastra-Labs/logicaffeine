//! Optional independent beta/zeta/delta congruence over borrowed head closures.
//!
//! This is a positive proof search, not a replacement evaluator. Original syntax
//! is borrowed for the complete call. Only existing bounded delta exposure may
//! allocate syntax; its closed result is flattened into a private indexed arena.
//! Environments contain terms, not LocalTypes. An argument retains its creation
//! environment, and fresh comparison binders cannot alias external Db indices.
use super::*;
use std::collections::HashMap;

const DEPTH: usize = 128;
const SPINE: usize = 64;
const OWNED: usize = 512;
const ENVS: usize = 512;
const PAYLOAD: usize = 4096;

#[derive(Clone, Copy)]
enum Node<'a> {
    Borrowed(&'a Db),
    Owned(usize),
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Neutral {
    Free(usize),
    Fresh(usize),
}
#[derive(Clone, Copy)]
enum Closure<'a> {
    Term(Node<'a>, usize),
    Neutral(Neutral),
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Borrowed(usize, usize),
    Owned(usize, usize),
    Neutral(Neutral),
}
impl Closure<'_> {
    fn key(self) -> Key {
        match self {
            Self::Term(Node::Borrowed(t), e) => Key::Borrowed(t as *const Db as usize, e),
            Self::Term(Node::Owned(i), e) => Key::Owned(i, e),
            Self::Neutral(n) => Key::Neutral(n),
        }
    }
}
struct Binding<'a> {
    parent: usize,
    value: Closure<'a>,
    depth: usize,
}
// Atom contains only Sort/Global/Const/Lit. All recursive edges are arena IDs,
// so dropping the arena cannot recursively destroy an application/lambda tree.
enum Stored {
    Atom(Db),
    Var(usize),
    Pi(usize, usize),
    Lam(usize, usize),
    App(usize, usize),
    Let {
        _annotation: usize,
        value: usize,
        body: usize,
    },
}
#[derive(Clone, Copy)]
enum View<'a> {
    Atom(Node<'a>),
    Var(usize),
    Pi(Node<'a>, Node<'a>),
    Lam(Node<'a>, Node<'a>),
    App(Node<'a>, Node<'a>),
    Let(Node<'a>, Node<'a>),
    Unsupported,
}
struct Head<'a> {
    core: Closure<'a>,
    args: Vec<Closure<'a>>,
}
struct Probe<'a, 'c, 'f> {
    ctx: &'c Context,
    fuel: &'f mut usize,
    owned: Vec<Stored>,
    envs: Vec<Binding<'a>>,
    env_ids: HashMap<(usize, Key), usize>,
    fresh: usize,
}

fn spend(fuel: &mut usize, n: usize) -> Option<()> {
    if *fuel < n {
        return None;
    }
    *fuel -= n;
    Some(())
}
fn string_work(s: &str, fuel: &mut usize) -> Option<usize> {
    if s.len() > 256 {
        return None;
    }
    spend(fuel, s.len())?;
    Some(s.len())
}
// Exact structural universe equality is sufficient for this positive probe.
// Equivalent differently written levels remain the ordinary checker's work.
fn level_work(u: &Universe, fuel: &mut usize, depth: usize) -> Option<usize> {
    if depth == 0 {
        return None;
    }
    // Reserve an additional visit for the later structural comparison/clone.
    spend(fuel, 2)?;
    Some(
        1 + match u {
            Universe::Var(s) => string_work(s, fuel)?,
            Universe::Succ(a) => level_work(a, fuel, depth - 1)?,
            Universe::Max(a, b) | Universe::IMax(a, b) => {
                level_work(a, fuel, depth - 1)?.checked_add(level_work(b, fuel, depth - 1)?)?
            }
            Universe::SProp | Universe::Prop | Universe::Type(_) => 0,
        },
    )
}
// Borrowed declaration inspection is itself charged, including failed attempts.
// This deliberately admits only the syntax the private output arena understands.
fn declaration_work(t: &Term, fuel: &mut usize, depth: usize, nodes: &mut usize) -> Option<usize> {
    if depth == 0 || *nodes == OWNED {
        return None;
    }
    spend(fuel, 1)?;
    *nodes += 1;
    let work = 1usize.checked_add(match t {
        Term::Var(n) | Term::Global(n) => string_work(n, fuel)?,
        Term::Sort(u) => level_work(u, fuel, 64)?,
        Term::Const { name, levels } => {
            if levels.len() > 64 {
                return None;
            }
            let mut n = string_work(name, fuel)?;
            for u in levels {
                n = n.checked_add(level_work(u, fuel, 64)?)?;
            }
            n
        }
        Term::App(a, b) => declaration_work(a, fuel, depth - 1, nodes)?
            .checked_add(declaration_work(b, fuel, depth - 1, nodes)?)?,
        Term::Pi {
            param,
            param_type,
            body_type,
        } => string_work(param, fuel)?
            .checked_add(declaration_work(param_type, fuel, depth - 1, nodes)?)?
            .checked_add(declaration_work(body_type, fuel, depth - 1, nodes)?)?,
        Term::Lambda {
            param,
            param_type,
            body,
        } => string_work(param, fuel)?
            .checked_add(declaration_work(param_type, fuel, depth - 1, nodes)?)?
            .checked_add(declaration_work(body, fuel, depth - 1, nodes)?)?,
        Term::Let {
            name,
            ty,
            value,
            body,
        } => string_work(name, fuel)?
            .checked_add(declaration_work(ty, fuel, depth - 1, nodes)?)?
            .checked_add(declaration_work(value, fuel, depth - 1, nodes)?)?
            .checked_add(declaration_work(body, fuel, depth - 1, nodes)?)?,
        Term::Lit(Literal::Text(s)) => string_work(s, fuel)?,
        Term::Lit(Literal::Nat(n)) => {
            if n.to_i64().is_none() || n.is_negative() {
                return None;
            }
            0
        }
        Term::Lit(Literal::BigInt(n)) => {
            n.to_i64()?;
            0
        }
        Term::Lit(Literal::Float(_)) => return None,
        Term::Lit(_) => 0,
        _ => return None,
    })?;
    (work <= PAYLOAD).then_some(work)
}
fn atom_work(t: &Db, fuel: &mut usize) -> Option<usize> {
    spend(fuel, 1)?;
    Some(
        1 + match t {
            Db::Global(s) => string_work(s, fuel)?,
            Db::Const { name, levels } => {
                if levels.len() > 64 {
                    return None;
                }
                let mut n = string_work(name, fuel)?;
                for u in levels {
                    n = n.checked_add(level_work(u, fuel, 64)?)?;
                }
                n
            }
            Db::Sort(u) => level_work(u, fuel, 64)?,
            Db::Lit(Literal::Text(s)) => string_work(s, fuel)?,
            Db::Lit(Literal::Nat(n)) => {
                if n.to_i64().is_none() || n.is_negative() {
                    return None;
                }
                0
            }
            Db::Lit(Literal::BigInt(n)) => {
                n.to_i64()?;
                0
            }
            // Float has total bitwise equality in Literal, but is conservatively
            // outside this first optional fragment. No pointer reflexivity shortcut.
            Db::Lit(Literal::Float(_)) => return None,
            Db::Lit(_) => 0,
            _ => return None,
        },
    )
}
fn atom_equal(a: &Db, b: &Db, fuel: &mut usize) -> Option<bool> {
    atom_work(a, fuel)?;
    atom_work(b, fuel)?;
    Some(match (a, b) {
        (Db::Global(a), Db::Global(b)) => a == b,
        (Db::Const { name: a, levels: u }, Db::Const { name: b, levels: v }) => a == b && u == v,
        (Db::Sort(a), Db::Sort(b)) => a == b,
        (Db::Lit(a), Db::Lit(b)) => a == b,
        _ => false,
    })
}

impl<'a, 'c, 'f> Probe<'a, 'c, 'f> {
    fn view(&self, n: Node<'a>) -> View<'a> {
        match n {
            Node::Borrowed(t) => match t {
                Db::Var(i) => View::Var(*i),
                Db::Pi(a, b) => View::Pi(Node::Borrowed(a), Node::Borrowed(b)),
                Db::Lam(a, b) => View::Lam(Node::Borrowed(a), Node::Borrowed(b)),
                Db::App(a, b) => View::App(Node::Borrowed(a), Node::Borrowed(b)),
                Db::Let(_, b, c) => View::Let(Node::Borrowed(b), Node::Borrowed(c)),
                Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => View::Atom(n),
                _ => View::Unsupported,
            },
            Node::Owned(i) => match &self.owned[i] {
                Stored::Var(i) => View::Var(*i),
                Stored::Pi(a, b) => View::Pi(Node::Owned(*a), Node::Owned(*b)),
                Stored::Lam(a, b) => View::Lam(Node::Owned(*a), Node::Owned(*b)),
                Stored::App(a, b) => View::App(Node::Owned(*a), Node::Owned(*b)),
                Stored::Let { value, body, .. } => {
                    View::Let(Node::Owned(*value), Node::Owned(*body))
                }
                Stored::Atom(_) => View::Atom(n),
            },
        }
    }
    fn extend(&mut self, parent: usize, value: Closure<'a>) -> Option<usize> {
        spend(self.fuel, 1)?;
        let key = (parent, value.key());
        if let Some(id) = self.env_ids.get(&key) {
            return Some(*id);
        }
        let depth = if parent == 0 {
            1
        } else {
            self.envs[parent - 1].depth.checked_add(1)?
        };
        if depth > DEPTH || self.envs.len() == ENVS {
            return None;
        }
        spend(self.fuel, 1)?;
        let id = self.envs.len() + 1;
        self.envs.push(Binding {
            parent,
            value,
            depth,
        });
        self.env_ids.insert(key, id);
        Some(id)
    }
    fn resolve(&mut self, mut index: usize, mut env: usize) -> Option<Closure<'a>> {
        while env != 0 {
            spend(self.fuel, 1)?;
            let binding = &self.envs[env - 1];
            if index == 0 {
                return Some(binding.value);
            }
            index -= 1;
            env = binding.parent;
        }
        Some(Closure::Neutral(Neutral::Free(index)))
    }
    fn head(&mut self, mut core: Closure<'a>, mut args: Vec<Closure<'a>>) -> Option<Head<'a>> {
        for _ in 0..DEPTH {
            spend(self.fuel, 1)?;
            let Closure::Term(n, e) = core else {
                return Some(Head { core, args });
            };
            match self.view(n) {
                View::Var(i) => core = self.resolve(i, e)?,
                View::App(f, a) => {
                    if args.len() == SPINE {
                        return None;
                    }
                    spend(self.fuel, 1)?;
                    args.push(Closure::Term(a, e));
                    core = Closure::Term(f, e);
                }
                View::Let(value, body) => {
                    let env = self.extend(e, Closure::Term(value, e))?;
                    core = Closure::Term(body, env);
                }
                View::Lam(_, body) if !args.is_empty() => {
                    let env = self.extend(e, args.pop().unwrap())?;
                    core = Closure::Term(body, env);
                }
                View::Unsupported => return None,
                _ => return Some(Head { core, args }),
            }
        }
        None
    }
    fn structural(&mut self, a: Closure<'a>, b: Closure<'a>, depth: usize) -> Option<bool> {
        match (a, b) {
            (Closure::Neutral(a), Closure::Neutral(b)) => return Some(a == b),
            (Closure::Term(n, e), other) if matches!(self.view(n), View::Var(_)) => {
                let View::Var(i) = self.view(n) else {
                    unreachable!()
                };
                let resolved = self.resolve(i, e)?;
                return self.equal(resolved, other, depth - 1);
            }
            (other, Closure::Term(n, e)) if matches!(self.view(n), View::Var(_)) => {
                let View::Var(i) = self.view(n) else {
                    unreachable!()
                };
                let resolved = self.resolve(i, e)?;
                return self.equal(other, resolved, depth - 1);
            }
            _ => {}
        }
        let (Closure::Term(a, ea), Closure::Term(b, eb)) = (a, b) else {
            return Some(false);
        };
        match (self.view(a), self.view(b)) {
            (View::Atom(a), View::Atom(b)) => {
                // Borrowing distinct fields permits bounded comparisons without
                // cloning either atom while the allowance is debited.
                let (owned, fuel) = (&self.owned, &mut *self.fuel);
                let get = |n| match n {
                    Node::Borrowed(t) => t,
                    Node::Owned(i) => match &owned[i] {
                        Stored::Atom(t) => t,
                        _ => unreachable!(),
                    },
                };
                atom_equal(get(a), get(b), fuel)
            }
            (View::App(af, aa), View::App(bf, ba)) => {
                if !self.equal(Closure::Term(af, ea), Closure::Term(bf, eb), depth - 1)? {
                    return Some(false);
                }
                self.equal(Closure::Term(aa, ea), Closure::Term(ba, eb), depth - 1)
            }
            (View::Lam(ad, ab), View::Lam(bd, bb)) | (View::Pi(ad, ab), View::Pi(bd, bb)) => {
                if !self.equal(Closure::Term(ad, ea), Closure::Term(bd, eb), depth - 1)? {
                    return Some(false);
                }
                if self.fresh == ENVS {
                    return None;
                }
                spend(self.fuel, 1)?;
                let fresh = Closure::Neutral(Neutral::Fresh(self.fresh));
                self.fresh += 1;
                let ea = self.extend(ea, fresh)?;
                let eb = self.extend(eb, fresh)?;
                self.equal(Closure::Term(ab, ea), Closure::Term(bb, eb), depth - 1)
            }
            _ => Some(false),
        }
    }
    fn equal(&mut self, a: Closure<'a>, b: Closure<'a>, depth: usize) -> Option<bool> {
        if depth == 0 {
            return None;
        }
        let _guard = nest(stack_mark())?;
        spend(self.fuel, 1)?;
        if self.structural(a, b, depth)? {
            return Some(true);
        }
        let a = self.head(a, Vec::new())?;
        let b = self.head(b, Vec::new())?;
        self.heads_equal(a, b, depth)
    }
    fn heads_equal(&mut self, a: Head<'a>, b: Head<'a>, depth: usize) -> Option<bool> {
        if depth == 0 {
            return None;
        }
        let _guard = nest(stack_mark())?;
        spend(self.fuel, 1)?;
        if a.args.len() == b.args.len() && self.structural(a.core, b.core, depth)? {
            let mut same = true;
            for (x, y) in a.args.iter().rev().zip(b.args.iter().rev()) {
                if !self.equal(*x, *y, depth - 1)? {
                    same = false;
                    break;
                }
            }
            if same {
                return Some(true);
            }
        }
        // Each success is an actual beta/zeta/delta/congruence derivation. A
        // recursive definition cannot create success by revisiting a state:
        // there is no coinductive visited-set success or cached proof verdict.
        if let Some(n) = self.expose(a.core)? {
            let a = self.head(Closure::Term(n, 0), a.args)?;
            return self.heads_equal(a, b, depth - 1);
        }
        if let Some(n) = self.expose(b.core)? {
            let b = self.head(Closure::Term(n, 0), b.args)?;
            return self.heads_equal(a, b, depth - 1);
        }
        Some(false)
    }
    fn expose(&mut self, c: Closure<'a>) -> Option<Option<Node<'a>>> {
        let Closure::Term(n, _) = c else {
            return Some(None);
        };
        if !matches!(self.view(n), View::Atom(_)) {
            return Some(None);
        }
        let output = {
            let (owned, fuel) = (&self.owned, &mut *self.fuel);
            let t = match n {
                Node::Borrowed(t) => t,
                Node::Owned(i) => match &owned[i] {
                    Stored::Atom(t) => t,
                    _ => unreachable!(),
                },
            };
            let (name, levels) = match t {
                Db::Global(n) => (n.as_str(), None),
                Db::Const { name, levels } => (name.as_str(), Some(levels.as_slice())),
                _ => return Some(None),
            };
            // Bounded hashing/lookup before consulting immutable declarations.
            string_work(name, fuel)?;
            let (ty, body, parameters) = match levels {
                Some(_) => match self.ctx.get_universe_poly(name) {
                    Some((p, t, b)) => (Some(t), b, p.as_slice()),
                    None => match (
                        self.ctx.get_global(name),
                        self.ctx.get_definition_body(name),
                    ) {
                        (Some(t), Some(b)) => (Some(t), b, &[][..]),
                        _ => return Some(None),
                    },
                },
                None => match self.ctx.get_definition_body(name) {
                    Some(b) => (None, b, &[][..]),
                    None => return Some(None),
                },
            };
            if parameters.len() > 64 {
                return Some(None);
            }
            let endpoint_work = atom_work(t, fuel)?;
            for parameter in parameters {
                string_work(parameter, fuel)?;
            }
            if let Some(ty) = ty {
                declaration_work(ty, fuel, 64, &mut 0)?;
            }
            let body_work = declaration_work(body, fuel, 64, &mut 0)?;
            if levels.is_some() {
                // atom_work includes root+name followed by every supplied level
                // node and payload byte. Bound copies BEFORE const_body:
                // resolve_const_levels copies the supplied level list once;
                // instantiate_universes and to_db can each copy it at every
                // source level occurrence. body_work counts all such source
                // occurrences, even Var("") with zero name bytes. Retain the
                // existing delta reserve as well; nothing is refunded.
                let level_copy_work = endpoint_work.checked_sub(1 + name.len())?;
                let copies = body_work.checked_mul(2)?.checked_add(1)?;
                spend(fuel, copies.checked_mul(level_copy_work)?)?;
            }
            // Native exclusions, exact universe arity/instantiation and closed
            // free-name admission remain the existing independent delta door.
            delta::reduce_with_limit(self.ctx, t, fuel, 512)
        };
        let Some(output) = output else {
            return Some(None);
        };
        let mut count = 0;
        let mut work = 0;
        self.admit_output(&output, 64, &mut count, &mut work)?;
        if count > OWNED.saturating_sub(self.owned.len()) {
            return None;
        }
        // One bound covers flattening plus any exclusive Owned descendants
        // cloned by an aliased DbBox::Shared::into_db. Nested Shared descendants
        // stop that clone; their own eventual demotion is paid by their logical
        // occurrences in this same scan. Original borrowed arguments never enter.
        spend(self.fuel, work.checked_mul(2)?)?;
        self.owned.reserve(count);
        let id = self.flatten(output);
        Some(Some(Node::Owned(id)))
    }
    fn admit_output(
        &mut self,
        t: &Db,
        depth: usize,
        nodes: &mut usize,
        work: &mut usize,
    ) -> Option<()> {
        if depth == 0 || *nodes == OWNED {
            return None;
        }
        spend(self.fuel, 1)?;
        *nodes += 1;
        *work = work.checked_add(1)?;
        match t {
            Db::Var(_) => {}
            Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => {
                self.admit_output(a, depth - 1, nodes, work)?;
                self.admit_output(b, depth - 1, nodes, work)?;
            }
            Db::Let(a, b, c) => {
                self.admit_output(a, depth - 1, nodes, work)?;
                self.admit_output(b, depth - 1, nodes, work)?;
                self.admit_output(c, depth - 1, nodes, work)?;
            }
            Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => {
                *work = work.checked_add(atom_work(t, self.fuel)?)?;
                if *work > PAYLOAD {
                    return None;
                }
            }
            _ => return None,
        }
        Some(())
    }
    fn flatten(&mut self, t: Db) -> usize {
        let node = match t {
            Db::Var(i) => Stored::Var(i),
            Db::Pi(a, b) => Stored::Pi(self.flatten(a.into_db()), self.flatten(b.into_db())),
            Db::Lam(a, b) => Stored::Lam(self.flatten(a.into_db()), self.flatten(b.into_db())),
            Db::App(a, b) => Stored::App(self.flatten(a.into_db()), self.flatten(b.into_db())),
            Db::Let(a, b, c) => Stored::Let {
                // Keep the admitted annotation in storage even though zeta's
                // head view needs only value/body and binds no variable in it.
                _annotation: self.flatten(a.into_db()),
                value: self.flatten(b.into_db()),
                body: self.flatten(c.into_db()),
            },
            t @ (Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_)) => Stored::Atom(t),
            _ => unreachable!("complete bounded output admission precedes flattening"),
        };
        let id = self.owned.len();
        self.owned.push(node);
        id
    }
}

pub(super) fn equivalent(ctx: &Context, a: &Db, b: &Db, fuel: &mut usize) -> bool {
    if *fuel == 0 {
        return false;
    }
    let mut probe = Probe {
        ctx,
        fuel,
        owned: Vec::new(),
        envs: Vec::new(),
        env_ids: HashMap::new(),
        fresh: 0,
    };
    probe
        .equal(
            Closure::Term(Node::Borrowed(a), 0),
            Closure::Term(Node::Borrowed(b), 0),
            DEPTH,
        )
        .unwrap_or(false)
}
