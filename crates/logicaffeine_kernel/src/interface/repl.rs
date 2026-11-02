//! REPL for the Vernacular interface.
//!
//! Orchestrates command parsing and kernel execution.

use super::command::Command;
use super::error::{InterfaceError, ParseError};
use super::parse_command_in_ctx;
use super::surface::{fill_defaults, resolve_names_in, Name, Resolution, Surface};
use super::term_parser::{BinderKind, BinderSurface, TermParser};
use super::with_env;
use crate::{
    auto_bind_implicits, bind_self_recursion, fill_match_motives, infer_type, normalize,
    surface_elaborate, surface_elaborate_against, Context, KernelError, Term,
};
use std::borrow::Cow;

/// The Vernacular REPL.
///
/// Maintains a kernel context and executes commands against it.
pub struct Repl {
    ctx: Context,
    /// The SURFACE: the notation, fixity and macros this session has declared. It sits beside the
    /// kernel context and never inside it — a way of spelling a declaration is not a declaration.
    surface: Surface,
    /// How many commands this session has run under the shadowing snapshot
    /// ([`Repl::shadow_snapshots`]).
    snapshots: usize,
}

/// A declaration elaborated against its declared type — what every checked declaration door
/// hands to its kernel gateway.
struct Elaborated {
    ty: Term,
    body: Term,
    binders: Vec<BinderSurface>,
    implicit_count: usize,
}

impl Repl {
    /// Create a new REPL with the standard library loaded.
    pub fn new() -> Self {
        Self {
            ctx: super::standard_context(),
            surface: Surface::new(),
            snapshots: 0,
        }
    }

    /// Execute a command string.
    ///
    /// Returns the output string (for Check/Eval) or empty string (for Definition/Inductive).
    pub fn execute(&mut self, input: &str) -> Result<String, InterfaceError> {
        // A declaration's terms are resolved and elaborated by recursive walks, so the whole
        // command runs on a stack sized for the text — a printed declaration of the deepest term
        // the kernel admits reads, checks and registers on an ordinary caller's thread.
        let depth = super::term_parser::syntactic_depth(input);
        super::term_parser::on_stack_for(depth, move || {
            let cmd = parse_command_in_ctx(&self.ctx, &self.surface, input)?;
            let declared = self.declared_keys(&cmd);
            if !self.redeclares(&declared) {
                return self.run(cmd);
            }
            // A retry READS THE COMMAND AGAIN: a door that resolves names as it parses (the literate
            // one) must see the shadow the refusal just recorded.
            let mut parsed = Some(cmd);
            self.shadowing(&declared, |repl| {
                let cmd = match parsed.take() {
                    Some(cmd) => cmd,
                    None => parse_command_in_ctx(&repl.ctx, &repl.surface, input)?,
                };
                repl.run(cmd)
            })
        })
    }

    /// Whether a command declaring the written names `declared` names anything this environment
    /// already holds — the only way it can meet the stable-meaning gate, and so the only case
    /// [`shadowing`](Self::shadowing) is asked to handle. A command that declares only fresh names
    /// runs straight through, without the context snapshot a retry needs: taking one makes the
    /// command's first write to every table it touches copy that whole table, which a document of
    /// thousands of fresh declarations must not pay.
    fn redeclares(&self, declared: &[String]) -> bool {
        declared.iter().any(|written| {
            let key = self
                .ctx
                .shadow_of(written)
                .unwrap_or_else(|| written.clone());
            self.ctx.get_global(&key).is_some() || self.ctx.poly_declaration(&key).is_some()
        })
    }

    /// The absolute WRITTEN names `cmd` declares here, its constructors included — the current
    /// namespace applied, a shadow not: these are what a shadow is keyed by. A constructor is keyed
    /// in the scope its inductive's own name opens, as registration keys it.
    fn declared_keys(&mut self, cmd: &Command) -> Vec<String> {
        match cmd {
            Command::Inductive {
                name, constructors, ..
            } => self.member_keys(name, constructors.iter().map(|(c, _)| c.as_str())),
            Command::InductiveBlock { members } => {
                let mut keys = Vec::new();
                for member in members {
                    keys.extend(self.member_keys(
                        &member.name,
                        member.constructors.iter().map(|(c, _)| c.as_str()),
                    ));
                }
                keys
            }
            Command::Universes { inner, .. }
            | Command::Protected(inner)
            | Command::OpenIn { inner, .. } => self.declared_keys(inner),
            other => declared_names(other)
                .iter()
                .map(|name| self.written_key(name))
                .collect(),
        }
    }

    /// An inductive's written key and its constructors', read in the scope its name opens.
    fn member_keys<'n>(
        &mut self,
        name: &str,
        constructors: impl Iterator<Item = &'n str>,
    ) -> Vec<String> {
        let (key, mark) = match Name::parse(name) {
            Some(declared) => self.surface.enter_declaration(&declared),
            None => (name.to_string(), self.surface.namespaces().mark()),
        };
        let mut keys = vec![key];
        keys.extend(constructors.map(|c| self.written_key(c)));
        self.surface.namespaces_mut().rewind(mark);
        keys
    }

    /// The absolute written name a declaration name written HERE declares — no shadow applied.
    fn written_key(&self, written: &str) -> String {
        match Name::parse(written) {
            Some(n) => self.surface.namespaces().declaration_key(&n),
            None => written.to_string(),
        }
    }

    /// Run one declaring step under the shadowing rule: a declaration the stable-meaning gate refuses
    /// because its name already means something others were checked against is registered under a
    /// FRESH IDENTITY instead, and its written name denotes that identity from then on.
    ///
    /// The gate stays the only judge. `attempt` runs exactly as it always did; when the kernel
    /// answers `StableMeaningViolation` for a name the step declares, the context is restored to
    /// what it was before the step, the written name is recorded as denoting a freshly minted
    /// identity, and the step runs again — READ again, so every name in it, its own included,
    /// resolves through the new shadow. A second refusal names a second declared name (an
    /// inductive's constructor, after the inductive itself), so the loop mints once per name and
    /// ends. Anything else the step answers is its answer — a refusal of a name the step does not
    /// declare (`declared`, the written absolute names) included: that is not the step's name to
    /// shadow. An identical redeclaration is kept by the gate, and a name nothing depends on is
    /// replaced in place, so neither mints anything.
    fn shadowing<T>(
        &mut self,
        declared: &[String],
        mut attempt: impl FnMut(&mut Self) -> Result<T, InterfaceError>,
    ) -> Result<T, InterfaceError> {
        let before = self.ctx.clone();
        self.snapshots += 1;
        let mut minted: Vec<(String, String)> = Vec::new();
        loop {
            let refused = match attempt(self) {
                Err(InterfaceError::Kernel(KernelError::StableMeaningViolation {
                    name,
                    change,
                    dependents,
                })) => (name, change, dependents),
                outcome => return self.settle(before, &minted, outcome),
            };
            let written = self
                .ctx
                .spelled_by(&refused.0)
                .unwrap_or_else(|| refused.0.clone());
            if !declared.contains(&written) || minted.iter().any(|(w, _)| *w == written) {
                let (name, change, dependents) = refused;
                let violation = KernelError::StableMeaningViolation {
                    name,
                    change,
                    dependents,
                };
                return self.settle(before, &minted, Err(InterfaceError::Kernel(violation)));
            }
            self.ctx = before.clone();
            let identity = self.ctx.mint_shadow_identity(&written);
            minted.push((written, identity));
            for (written, identity) in &minted {
                self.ctx.record_shadow(written, identity);
            }
        }
    }

    /// A shadowing step's outcome. A step that minted identities and still failed leaves nothing
    /// behind: the identities and the partial registration its retry made are taken back with it.
    fn settle<T>(
        &mut self,
        before: Context,
        minted: &[(String, String)],
        outcome: Result<T, InterfaceError>,
    ) -> Result<T, InterfaceError> {
        if outcome.is_err() && !minted.is_empty() {
            self.ctx = before;
        }
        outcome
    }

    /// Resolve every name written in `term` against the namespace scope, BEFORE any elaboration
    /// sees it. Lean's staging: `resolveGlobalName` runs to completion before a term reaches the
    /// kernel, which is why the kernel needs to know nothing about namespaces at all.
    ///
    /// `pending` are the keys this very command is about to declare — a recursive definition refers
    /// to itself by its short name from inside its own namespace, and a constructor's type names the
    /// inductive it belongs to; neither is in the context yet. `locals` shields names that are BOUND
    /// rather than global (an inductive's type parameters arrive from the parser as `Global`s and
    /// only become `Var`s at registration).
    ///
    /// Then every OMITTED DEFAULT ARGUMENT is filled, in the same pass and for the same reason:
    /// `withDefault 1` is a spelling of `withDefault 1 3` exactly as `token` under `open Alpha` is
    /// a spelling of `Alpha.token`, and both are expansions the elaborator must never have to know
    /// about. Resolution runs FIRST — a default is recorded under the declaration's absolute key,
    /// so the head has to be the key before it can be looked up.
    ///
    /// Borrows when nothing needed resolving, so an empty scope costs one read-only walk.
    fn resolve<'t>(
        &self,
        term: &'t Term,
        pending: &[String],
        locals: &mut Vec<String>,
    ) -> Result<Cow<'t, Term>, InterfaceError> {
        let resolved = with_env(&self.ctx, pending, |env| {
            resolve_names_in(self.surface.namespaces(), env, locals, term)
        })?;
        Ok(fill_defaults(&self.surface, resolved))
    }

    /// The absolute key a declaration written here gets, and the scope its body elaborates in.
    /// Restore the returned mark once the body is done, whatever the outcome.
    ///
    /// A name no TERM can spell gets no namespace treatment: it keeps the flat key it arrived with
    /// and opens nothing. That is not a concession, it is the honest reading — a name that cannot be
    /// written cannot be written RELATIVELY either, so there is nothing for a namespace to do with
    /// it. The Coq-style door already refuses such a name (`declaration_name` shares the term
    /// grammar); this is the guarantee that whatever a literate surface produces keeps behaving
    /// exactly as it did.
    ///
    /// The key is the one the written name DENOTES: a name a shadow redirects declares at the
    /// shadow's identity, exactly as a reference to it resolves there.
    fn enter_declaration(&mut self, written: &str) -> (String, super::surface::ScopeMark) {
        let (key, mark) = match Name::parse(written) {
            Some(declared) => self.surface.enter_declaration(&declared),
            None => (written.to_string(), self.surface.namespaces().mark()),
        };
        (self.ctx.shadow_of(&key).unwrap_or(key), mark)
    }

    /// Execute ONE already-parsed command.
    ///
    /// Split out from [`execute`] because `Open … in <command>` is CONTROL FLOW: it has to bracket
    /// the execution of the command it scopes and wind the scope back afterwards, which a body that
    /// parses and then matches cannot do.
    fn run(&mut self, cmd: Command) -> Result<String, InterfaceError> {
        match cmd {
            Command::Surface(decl) => {
                // ★ A SCOPE FORM IS A SCOPE IN BOTH STACKS. `Section`/`End` and `Namespace`/`End`
                // moved the SURFACE's scope — which notation is in force — and left the KERNEL's at
                // zero, so a `section` scoped how things were SPELLED and nothing about what was in
                // force inside it: a `local` instance, coercion, simp rule, hint or reducibility
                // edit made inside one never left at its `end`. Lean scopes `local` at both forms,
                // and so does this.
                //
                // The two depths are MIRRORED, never restated: the surface already counts its own
                // open scopes and the kernel is moved to agree with that count, so a third scope
                // form added later is one in the kernel too without a line here, and the two can
                // never drift into disagreeing about how many are open.
                self.surface.declare(decl)?;
                self.match_kernel_scopes();
                Ok(String::new()) // a way of spelling something is not a declaration
            }

            Command::Open(clauses) => {
                // All or nothing: a list with one clause that does not check opens none of them, so
                // a typo cannot leave half a scope behind.
                let mark = self.surface.namespaces().mark();
                match self.apply_opens(clauses) {
                    Ok(()) => Ok(String::new()), // an opening is a spelling, not a declaration
                    Err(e) => {
                        self.surface.namespaces_mut().rewind(mark);
                        Err(e)
                    }
                }
            }

            Command::OpenIn { opens, inner } => {
                let mark = self.surface.namespaces().mark();
                let out = match self.apply_opens(opens) {
                    Ok(()) => self.run(*inner),
                    Err(e) => Err(e),
                };
                // Wound back on FAILURE too: a refused command must not leave its opening behind,
                // or the next command reads names in a scope nobody wrote.
                self.surface.namespaces_mut().rewind(mark);
                out
            }

            Command::Definition { .. }
            | Command::Theorem { .. }
            | Command::TheoremBy { .. }
            | Command::Opaque { .. }
            | Command::Axiom { .. }
            | Command::InductiveBlock { .. }
            | Command::Quotient { .. } => self.declare(None, cmd),

            Command::Universes { params, inner } => self.declare(Some(params), *inner),

            Command::Export { namespace, names } => {
                // The environment is the whole point: an alias to a name the source namespace does
                // not declare, or one the current namespace already declares, is refused here
                // because only here can it be SEEN.
                let Repl { ctx, surface, .. } = self;
                let known = |key: &str| ctx.get_global(key).is_some();
                surface
                    .namespaces_mut()
                    .export(&namespace, &names, &known)?;
                Ok(String::new()) // an alias is a spelling, and a spelling is not a declaration
            }

            Command::Protected(inner) => {
                // The key the inner command will land at, read BEFORE it runs — the declaration's
                // own namespace frame comes off again, so afterwards the current namespace no
                // longer says where it went.
                let declared = declared_names(inner.as_ref());
                if declared.is_empty() {
                    return Err(ParseError::Unsupported {
                        what: "`Protected` on a command that declares nothing".to_string(),
                        why:
                            "`protected` changes which SHORT NAME reaches a declaration, so there \
                              has to be one. Write it on a `Definition`, `Theorem`, `Opaque`, \
                              `Axiom` or `Inductive`"
                                .to_string(),
                    }
                    .into());
                }
                // The WRITTEN absolute key, and not the identity a shadow gives it: `protected`
                // decides which short name reaches a written name, and a shadow is reached through
                // exactly that written name.
                let keys: Vec<String> = declared
                    .into_iter()
                    .map(|declared| match Name::parse(&declared) {
                        Some(name) => self.surface.namespaces().declaration_key(&name),
                        None => declared,
                    })
                    .collect();
                let out = self.run(*inner)?;
                // Marked only on SUCCESS: a declaration that did not check protects nothing.
                for key in &keys {
                    self.surface.namespaces_mut().record_protected(key);
                }
                Ok(out)
            }

            Command::PrintAxioms(written) => {
                // A REFERENCE, so it reads in the scope it is written in — the same resolution a
                // term's name gets, asked of the same door.
                let Repl { ctx, surface, .. } = self;
                let key =
                    match with_env(ctx, &[], |env| env.resolve(surface.namespaces(), &written)) {
                        Resolution::Found(key) => key,
                        Resolution::Unresolved(key) => key,
                        Resolution::Ambiguous {
                            written,
                            candidates,
                        } => {
                            return Err(ParseError::AmbiguousName {
                                written,
                                candidates,
                            }
                            .into())
                        }
                    };
                // ONE implementation, three surfaces. `axiom_dependencies_of` is what the
                // re-check gate and the trusted-base census already read; a second closure written
                // here could disagree with them about what a proof rests on, and the whole value of
                // the report is that it cannot.
                match crate::axiom_dependencies_of(ctx, &key) {
                    Some(usage) => Ok(usage.render(&key)),
                    None => Err(InterfaceError::Kernel(crate::KernelError::UnboundVariable(
                        key,
                    ))),
                }
            }

            Command::Comment => Ok(String::new()), // prose declares nothing

            Command::Structure {
                inductive,
                projections,
            } => {
                let key = |repl: &Self, written: String| -> Result<String, InterfaceError> {
                    let reference = Term::Global(written);
                    match repl.resolve(&reference, &[], &mut Vec::new())?.into_owned() {
                        Term::Global(key) => Ok(key),
                        other => Ok(other.to_string()),
                    }
                };
                let inductive = key(self, inductive)?;
                let projections = projections
                    .into_iter()
                    .map(|p| key(self, p))
                    .collect::<Result<Vec<_>, _>>()?;
                self.ctx
                    .register_structure_checked(&inductive, &projections)
                    .map_err(|e| {
                        InterfaceError::Kernel(crate::KernelError::InvalidDeclaration(
                            e.to_string(),
                        ))
                    })?;
                Ok(String::new())
            }

            Command::Check(term) => {
                let term = self.resolve(&term, &[], &mut Vec::new())?.into_owned();
                // Infer `match` motives, insert implicit arguments, then infer the type.
                let term = fill_match_motives(&self.ctx, &term, None)?;
                let term = surface_elaborate(&self.ctx, &term)?;
                let ty = infer_type(&self.ctx, &term)?;
                Ok(format!("{} : {}", self.written(&term), self.written(&ty)))
            }

            Command::Eval(term) => {
                let term = self.resolve(&term, &[], &mut Vec::new())?.into_owned();
                // Infer `match` motives, insert implicits, type check, then normalize.
                let term = fill_match_motives(&self.ctx, &term, None)?;
                let term = surface_elaborate(&self.ctx, &term)?;
                let _ = infer_type(&self.ctx, &term)?;
                let result = normalize(&self.ctx, &term);
                Ok(format!("{}", self.written(&result)))
            }

            Command::Inductive {
                name,
                params,
                sort,
                constructors,
            } => {
                let (key, mark) = self.enter_declaration(&name);
                let out = self.register_inductive(key, params, sort, constructors);
                self.surface.namespaces_mut().rewind(mark);
                out
            }
        }
    }

    /// Execute one DECLARATION, bound by `universes` when a telescope was written on it.
    ///
    /// The telescope is a wrapper around the declaration it binds (`Command::Universes`), and this
    /// is the one place it is taken apart, so every declaration door receives it the same way and a
    /// telescope on anything that declares nothing is refused here rather than ignored.
    fn declare(
        &mut self,
        universes: Option<Vec<String>>,
        cmd: Command,
    ) -> Result<String, InterfaceError> {
        let telescope = |universes: Option<Vec<String>>| universes.unwrap_or_default();
        match cmd {
            Command::Definition {
                name,
                ty,
                body,
                is_hint,
                implicit_count,
                proof_goal,
            } => {
                let (key, mark) = self.enter_declaration(&name);
                let out = self.define(
                    key,
                    universes,
                    ty,
                    body,
                    is_hint,
                    implicit_count,
                    proof_goal,
                );
                self.surface.namespaces_mut().rewind(mark);
                out
            }
            Command::Theorem { name, ty, proof } => {
                let (key, mark) = self.enter_declaration(&name);
                let out = self.declare_opaque(key, telescope(universes), ty, proof, true);
                self.surface.namespaces_mut().rewind(mark);
                out
            }
            Command::TheoremBy { name, ty, script } => {
                let (key, mark) = self.enter_declaration(&name);
                let out = self.prove_by_script(key, telescope(universes), ty, &script);
                self.surface.namespaces_mut().rewind(mark);
                out
            }
            Command::Opaque { name, ty, value } => {
                let (key, mark) = self.enter_declaration(&name);
                let out = self.declare_opaque(key, telescope(universes), ty, value, false);
                self.surface.namespaces_mut().rewind(mark);
                out
            }
            Command::Axiom { name, ty } => {
                let (key, mark) = self.enter_declaration(&name);
                let out = self.declare_axiom(key, telescope(universes), ty);
                self.surface.namespaces_mut().rewind(mark);
                out
            }
            Command::InductiveBlock { members } => self.register_block(universes, members),
            Command::Quotient { kind, equality } => {
                let Some(params) = universes else {
                    return Err(ParseError::Missing(format!(
                        "the universe parameters of the quotient primitive `{}` \u{2014} written \
                         `.{{u}}` after its name",
                        kind.name()
                    ))
                    .into());
                };
                let equality = match equality {
                    Some(written) => {
                        let reference = Term::Global(written);
                        let resolved = self.resolve(&reference, &[], &mut Vec::new())?.into_owned();
                        match resolved {
                            Term::Global(key) => key,
                            other => other.to_string(),
                        }
                    }
                    None => String::new(),
                };
                crate::quotient::install(&mut self.ctx, kind, params, &equality)
                    .map_err(InterfaceError::Kernel)?;
                self.surface
                    .namespaces_mut()
                    .record_declaration(kind.name());
                Ok(String::new())
            }
            other => Err(ParseError::Unsupported {
                what: format!(
                    "a universe telescope on {}",
                    match other {
                        Command::Universes { .. } => "a declaration that already has one",
                        Command::Inductive { .. } => "the undeclared-arity `Inductive` form",
                        _ => "a command that declares nothing",
                    }
                ),
                why: "universe parameters bind over ONE declaration \u{2014} a `Definition`, \
                      `Theorem`, `Opaque`, `Axiom`, or an `Inductive` that declares its arity"
                    .to_string(),
            }
            .into()),
        }
    }

    /// Register an inductive BLOCK exactly as declared, through the kernel's checked block door —
    /// `Context::add_mutual_inductives`, or `add_universe_poly_inductives` under a telescope — the
    /// door the Lean importer registers its inductives through.
    ///
    /// Unlike the legacy `Inductive` path, nothing is inferred: each member's parameter count is
    /// the number of parameters written, the rest of its declared arity is indices, and the block
    /// is one mutual block because the writer said so, not because its members happen to mention
    /// one another. Then the declaration-time family (`{I}_rec`, `{I}_casesOn`, …) is derived for
    /// every member, as the mutual path already does.
    fn register_block(
        &mut self,
        universes: Option<Vec<String>>,
        members: Vec<super::command::InductiveMember>,
    ) -> Result<String, InterfaceError> {
        let mut keys: Vec<(String, Vec<String>)> = Vec::with_capacity(members.len());
        for member in &members {
            let (key, mark) = self.enter_declaration(&member.name);
            let ctors: Vec<String> = member
                .constructors
                .iter()
                .map(|(cn, _)| self.declaration_key(cn))
                .collect();
            self.surface.namespaces_mut().rewind(mark);
            keys.push((key, ctors));
        }
        let mut pending: Vec<String> = Vec::new();
        for (key, ctors) in &keys {
            pending.push(key.clone());
            pending.extend(ctors.iter().cloned());
        }
        let mut block = Vec::with_capacity(members.len());
        for (member, (key, ctor_keys)) in members.iter().zip(&keys) {
            let (_, mark) = self.enter_declaration(&member.name);
            let resolved = (|| -> Result<crate::context::MutualInductive, InterfaceError> {
                let mut locals: Vec<String> =
                    member.params.iter().map(|(p, _)| p.clone()).collect();
                let resolve = |t: &Term, locals: &mut Vec<String>| {
                    self.resolve(t, &pending, locals).map(Cow::into_owned)
                };
                let params: Vec<(String, Term)> = member
                    .params
                    .iter()
                    .map(|(p, t)| resolve(t, &mut locals).map(|t| (p.clone(), t)))
                    .collect::<Result<_, _>>()?;
                let arity = resolve(&member.sort, &mut locals)?;
                // A RESTATED telescope is registered exactly as written: its binders bind the
                // parameters, so nothing is converted and nothing is prepended.
                let constructors: Vec<(String, Term)> = ctor_keys
                    .iter()
                    .zip(&member.constructors)
                    .map(|(ck, (_, cty))| {
                        if member.full_constructors {
                            resolve(cty, &mut Vec::new()).map(|t| (ck.clone(), t))
                        } else {
                            resolve(cty, &mut locals)
                                .map(|t| (ck.clone(), parameter_telescope(&params, t)))
                        }
                    })
                    .collect::<Result<_, _>>()?;
                Ok(crate::context::MutualInductive {
                    name: key.clone(),
                    sort: parameter_telescope(&params, arity),
                    num_params: params.len(),
                    constructors,
                })
            })();
            self.surface.namespaces_mut().rewind(mark);
            block.push(resolved?);
        }
        match &universes {
            Some(levels) => self.ctx.add_universe_poly_inductives(levels, &block),
            None => self.ctx.add_mutual_inductives(&block),
        }
        .map_err(InterfaceError::Kernel)?;
        for member in &block {
            self.surface
                .namespaces_mut()
                .record_declaration(&member.name);
            for (cn, _) in &member.constructors {
                self.surface.namespaces_mut().record_declaration(cn);
            }
        }
        // The declaration-time family, through the door that records its PROVENANCE, so a derived
        // constant is known as derived (`Context::is_derived_aux`) and a later re-derivation
        // replaces its own generation rather than refusing to touch a "hand-written" one.
        for member in &block {
            self.ctx.regenerate_auxiliaries(&member.name);
        }
        Ok(String::new())
    }

    /// How many SCOPES the surface has open — sections plus namespaces, which are the two forms
    /// Lean scopes `local` at. The kernel's [`Context::scope_depth`] tracks this number.
    fn open_scopes(&self) -> usize {
        self.surface.section_depth() + self.surface.namespaces().depth()
    }

    /// Move the kernel's scope depth to whatever the surface's is NOW.
    ///
    /// Stated absolutely rather than as a delta, and that is not a detail: the document door winds
    /// the surface back wholesale on a refusal, so a command that opened a scope and a document
    /// that abandoned three both arrive here as "the surface has N open, make the kernel agree".
    /// A delta would have to be computed by whichever path did the winding, which is exactly the
    /// bookkeeping that drifts.
    ///
    /// The REPL owns [`Context::scope_depth`] for as long as it owns the context.
    fn match_kernel_scopes(&mut self) {
        let want = self.open_scopes();
        while self.ctx.scope_depth() > want {
            self.ctx.pop_scope();
        }
        while self.ctx.scope_depth() < want {
            self.ctx.push_scope();
        }
    }

    /// Apply a list of `open` clauses against the ENVIRONMENT.
    ///
    /// A clause that names members (`Open Alpha (a b).`, `Open Alpha hiding (a).`) can only be
    /// checked here: a name the namespace does not declare narrows the opening to nothing and reads
    /// exactly like a clause that worked. The caller winds the scope back on failure, so a list
    /// whose second clause is wrong opens neither.
    fn apply_opens(
        &mut self,
        clauses: Vec<super::surface::OpenClause>,
    ) -> Result<(), InterfaceError> {
        let Repl { ctx, surface, .. } = self;
        let known = |key: &str| ctx.get_global(key).is_some();
        for clause in clauses {
            surface.namespaces_mut().open_clause(clause, &known)?;
        }
        Ok(())
    }

    /// Register one `Definition` at the ABSOLUTE key `name`, with the namespace scope its body
    /// elaborates in already entered by the caller. `universes` is its universe telescope when one
    /// was written (`Definition name.{u} …`), and then it registers through the kernel's
    /// universe-polymorphic door, which refuses a level variable the telescope does not bind.
    #[allow(clippy::too_many_arguments)]
    fn define(
        &mut self,
        name: String,
        universes: Option<Vec<String>>,
        ty: Option<Term>,
        body: Term,
        is_hint: bool,
        implicit_count: usize,
        proof_goal: Option<Term>,
    ) -> Result<String, InterfaceError> {
        let pending = [name.clone()];
        let Elaborated {
            ty,
            body,
            binders,
            implicit_count,
        } = self.elaborate_declaration(&name, ty, body, implicit_count)?;
        let proof_goal = match proof_goal {
            Some(g) => Some(self.resolve(&g, &pending, &mut Vec::new())?.into_owned()),
            None => None,
        };
        // Soundness gate for literate tactic proofs (`## Theorem … Proof: <tactic>.`).
        // The body is a reflected `Derivation`; `infer_type` only confirms it is WELL-FORMED,
        // and a FAILED tactic still yields a well-typed *error* derivation. So type-checking
        // alone would accept a false theorem. `proof_goal` carries the goal the derivation
        // must establish: require `concludes body` to reduce to exactly that goal. A failed
        // or malformed tactic concludes to the `SName "Error"` sentinel instead, and is
        // rejected here. (Only literate theorem proofs set `proof_goal`; a raw
        // `d : Derivation := …` has `None` and is left untouched.)
        if let Some(goal_syntax) = &proof_goal {
            let concluded = normalize(
                &self.ctx,
                &Term::App(
                    Box::new(Term::Global("concludes".to_string())),
                    Box::new(body.clone()),
                ),
            );
            let want = normalize(&self.ctx, goal_syntax);
            if concluded != want {
                let name = self.ctx.written_name(&name);
                return Err(InterfaceError::Kernel(
                    crate::KernelError::CertificationError(format!(
                        "tactic proof of `{name}` did not certify: the tactic did not \
                         establish the stated goal"
                    )),
                ));
            }
        }

        // Add definition to context
        match universes {
            Some(params) => self
                .ctx
                .declare_universe_poly(&name, params, ty, body)
                .map_err(InterfaceError::Kernel)?,
            None => self
                .ctx
                .declare(&name, ty, body)
                .map_err(InterfaceError::Kernel)?,
        }
        self.record_declaration_surface(&name, binders, implicit_count);

        // Register as hint if marked
        if is_hint {
            self.ctx.add_hint(&name);
        }

        Ok(String::new()) // Silent success
    }

    /// Elaborate a declaration's body against its declared type, exactly as every checked
    /// declaration door does: split the declared surface off the kernel type, resolve names in the
    /// scope the declaration is written in, bind the free type variables Lean's `autoImplicit`
    /// would, recover a recursive body's `fix`, infer match motives, insert implicit arguments, and
    /// check the body's type against the declared one.
    ///
    /// ONE implementation for `Definition`, `Theorem` and `Opaque`, so the three cannot come to
    /// disagree about what a body written in the vernacular MEANS — they differ only in which
    /// kernel door the elaborated pair then goes through.
    fn elaborate_declaration(
        &mut self,
        name: &str,
        ty: Option<Term>,
        body: Term,
        implicit_count: usize,
    ) -> Result<Elaborated, InterfaceError> {
        // ★ THE DECLARED TYPE ARRIVES WITH ITS SURFACE STILL ON IT, and the two must part here.
        // `Definition f : Int -> (m : Int := 3) -> Int` parses to a `Pi` whose parameter type is a
        // reserved marker application carrying the default; registering THAT is what used to hand
        // the kernel an unbound global with a `⟨default⟩` in its name. `declaration_surface` splits
        // it into the kernel type the context is keyed at and the per-parameter surface a CALL SITE
        // needs, which is the only thing either half is for.
        let (ty, body, mut binders) = match ty {
            Some(t) => {
                let declared = TermParser::declaration_surface(&t);
                (Some(declared.kernel_type), body, declared.binders)
            }
            None => {
                let declared = TermParser::declaration_surface(&body);
                (None, declared.kernel_type, declared.binders)
            }
        };
        // Commands built without parser markers still carry their leading implicit count.
        for binder in binders.iter_mut().take(implicit_count) {
            binder.kind = BinderKind::Implicit;
        }
        let declared_implicits = implicit_count;
        // A recursive definition refers to itself by the short name it is being given, so
        // its own key resolves before the context holds it.
        let pending = [name.to_string()];
        let ty = match ty {
            Some(t) => Some(self.resolve(&t, &pending, &mut Vec::new())?.into_owned()),
            None => None,
        };
        let body = self.resolve(&body, &pending, &mut Vec::new())?.into_owned();
        // ★ IN LOGOS THE PARAMETER NAMES ARE IN THE BODY. `A -> B -> C` is the only spelling of a
        // non-dependent signature and it names nothing, so a default that mentions an earlier
        // parameter (`(m : Int := mul n 2)`) can only be closed once the body's own λ-telescope has
        // said which position `n` is. Without this the default is inlined as an OPEN TERM with a
        // free `n` in it, which is a silent wrong answer rather than an error.
        TermParser::name_anonymous_binders(&mut binders, &body);
        // A default is SOURCE, written in the scope the declaration is written in, so it resolves
        // like any other term — with the parameter names shielded, because a parameter reference is
        // not a global and must not be resolved against the namespace scope.
        if binders.iter().any(|b| b.default.is_some()) {
            let param_names: Vec<String> = binders.iter().map(|b| b.name.clone()).collect();
            for index in 0..binders.len() {
                let Some(value) = binders[index].default.clone() else {
                    continue;
                };
                let mut locals = param_names.clone();
                let resolved = self.resolve(&value, &pending, &mut locals)?.into_owned();
                binders[index].default = Some(resolved);
            }
        }
        // Auto-bind free type variables (`id : A -> A`) as leading implicits, then
        // elaborate the body against its declared type — implicit arguments are
        // filled from the body's own arguments AND from the expected type.
        let (ty, body, implicit_count) = match ty {
            Some(t) => {
                let (t, b, k) = auto_bind_implicits(&self.ctx, &t, &body, implicit_count);
                (Some(self.elaborate_type(&t)?), b, k)
            }
            None => (None, body, implicit_count),
        };
        // An auto-bound implicit is a binder the SOURCE never wrote, prepended to the telescope —
        // so the recorded surface has to grow at the front by exactly as many, or every later
        // position is off by that many and the alignment above is wrong again. They are anonymous
        // on purpose: nothing may substitute for a binder whose argument the elaborator invents.
        let invented = implicit_count.saturating_sub(declared_implicits);
        if invented > 0 && !binders.is_empty() {
            let mut telescope: Vec<BinderSurface> = (0..invented)
                .map(|_| BinderSurface {
                    name: "_".to_string(),
                    kind: BinderKind::Implicit,
                    declared: Term::Hole,
                    default: None,
                    auto: None,
                })
                .collect();
            telescope.append(&mut binders);
            binders = telescope;
        }
        // Recursive-definition sugar: a body that calls the definition by name is
        // bound with a `fix` (the kernel's termination guard certifies it).
        let body = bind_self_recursion(name, &body);
        let body = fill_match_motives(&self.ctx, &body, ty.as_ref())?;
        let body = surface_elaborate_against(&self.ctx, &body, ty.as_ref())?;
        let inferred_ty = infer_type(&self.ctx, &body)?;

        // A DECLARED type is a CLAIM, and a claim must be checked. `infer_type` above only
        // says the body is well-formed *somehow* — it says nothing about the type the user
        // wrote. Taking the declared type on faith let the REPL stamp any term with any
        // type:
        //
        //     Definition oops : False := I.     => Ok("")   ; Check oops => "oops : False"
        //
        // i.e. a `False` inhabitant from the ordinary user surface, and from a proof of a
        // false proposition everything follows. `surface_elaborate_against` elaborates the
        // body against the declared type but returns `Ok` on a leaf it cannot reconcile, so
        // it is not a substitute for this comparison.
        let ty = match ty {
            Some(declared) => {
                if !crate::is_subtype(&self.ctx, &inferred_ty, &declared) {
                    return Err(InterfaceError::Kernel(crate::KernelError::TypeMismatch {
                        expected: declared.to_string(),
                        found: inferred_ty.to_string(),
                    }));
                }
                declared
            }
            None => inferred_ty,
        };

        Ok(Elaborated {
            ty,
            body,
            binders,
            implicit_count,
        })
    }

    /// Elaborate a DECLARED TYPE, as Lean's `elabType` does before the body is looked at: match
    /// motives are inferred, implicit arguments inserted, and a numeral is built at the carrier its
    /// position asks for. `Theorem t : Eq Nat 2 2` states an equation between two `Nat`s; left
    /// unelaborated its numerals are the `Int`s they are read as, the statement is not a
    /// proposition about `Nat` at all, and no proof of it could check.
    fn elaborate_type(&self, ty: &Term) -> Result<Term, InterfaceError> {
        let ty = fill_match_motives(&self.ctx, ty, None)?;
        Ok(surface_elaborate(&self.ctx, &ty)?)
    }

    /// Record how a declaration may be SPELLED at a call site — its namespace, its binder kinds,
    /// its defaults and its leading-implicit count — once the kernel has accepted it.
    fn record_declaration_surface(
        &mut self,
        name: &str,
        binders: Vec<BinderSurface>,
        implicit_count: usize,
    ) {
        // The namespaces this key's prefixes name now EXIST, so they can be opened.
        self.surface.namespaces_mut().record_declaration(name);
        // …and how this declaration may be SPELLED at a call site. A redeclaration replaces the
        // entry and a redeclaration without defaults removes it, so a stale default can never fill
        // an argument the current declaration does not offer.
        if binders
            .iter()
            .any(|binder| binder.kind != BinderKind::Explicit)
        {
            self.ctx.set_binder_kinds(
                name,
                binders
                    .iter()
                    .map(|binder| binder.kind.as_param_kind())
                    .collect(),
            );
        }
        self.surface.record_defaults(name, binders);

        // Record how many leading arguments are implicit, so later uses of this
        // global (`name 0`) get them inserted and inferred.
        if implicit_count > 0 {
            self.ctx.set_implicit_args(name, implicit_count);
        }
    }

    /// Register a `Theorem` (`theorem` true) or an `Opaque` at the ABSOLUTE key `name`.
    ///
    /// The body is elaborated exactly as a `Definition`'s, then handed to the checked gateway the
    /// Lean importer uses for the same kind — `Context::declare_theorem_checked` for a theorem,
    /// `Context::declare_opaque_checked` for an `Opaque` — which checks the type and the body in
    /// BOTH kernels, requires the body's type to be the declared type by conversion in both
    /// directions (not the cumulative subtyping a `Definition` is allowed), refuses a hole or an
    /// open term and records what the proof rests on. A theorem then unfolds as Lean's kernel
    /// unfolds one; an `Opaque` keeps only its header.
    fn declare_opaque(
        &mut self,
        name: String,
        universes: Vec<String>,
        ty: Term,
        value: Term,
        theorem: bool,
    ) -> Result<String, InterfaceError> {
        let leading = TermParser::binder_kinds_of(&ty)
            .iter()
            .take_while(|k| **k == BinderKind::Implicit)
            .count();
        let Elaborated {
            ty,
            body,
            binders,
            implicit_count,
        } = self.elaborate_declaration(&name, Some(ty), value, leading)?;
        if theorem {
            // A theorem STATES A PROPOSITION. Lean refuses `theorem` at a type that is not a
            // `Prop` and so does the importer; data that must stay sealed is `Opaque`.
            let sort = crate::normalize(&self.ctx, &infer_type(&self.ctx, &ty)?);
            let is_prop = matches!(&sort, Term::Sort(u) if u.equiv(&crate::Universe::Prop));
            if !is_prop {
                let (name, ty, sort) = (
                    self.ctx.written_name(&name),
                    self.written(&ty),
                    self.written(&sort),
                );
                return Err(InterfaceError::Kernel(
                    crate::KernelError::CertificationError(format!(
                    "`{name}` is stated as a theorem, but its statement `{ty}` lives in `{sort}`, \
                     not in `Prop` \u{2014} a theorem states a proposition. Seal data with \
                     `Opaque {name} : {ty} := \u{2026}.`"
                )),
                ));
            }
        }
        // A document read a second time meets its own declarations: the one already registered
        // here, identically, IS this declaration, and the append-only door is not asked again.
        if self
            .ctx
            .holds_checked_declaration(&name, &universes, &ty, &body, theorem)
        {
            return Ok(String::new());
        }
        let published = if theorem {
            self.ctx.declare_theorem_checked(&name, universes, ty, body)
        } else {
            self.ctx.declare_opaque_checked(&name, universes, ty, body)
        };
        published.map_err(InterfaceError::Kernel)?;
        self.record_declaration_surface(&name, binders, implicit_count);
        Ok(String::new())
    }

    /// Prove `ty` by `script` over the kernel's own goals, then publish the finished proof term as
    /// a [`Command::Theorem`]'s proof is published. Each step's terms are resolved in the scope the
    /// statement is, with the focused goal's hypotheses bound, so `exact h` names the hypothesis
    /// and `exact Nat.zero_le` the global.
    fn prove_by_script(
        &mut self,
        name: String,
        universes: Vec<String>,
        ty: Term,
        script: &[crate::tactics::TacticStep],
    ) -> Result<String, InterfaceError> {
        use crate::tactics::{GoalState, ScriptError, TacticStep};
        let declared = TermParser::declaration_surface(&ty);
        let statement = self
            .resolve(&declared.kernel_type, &[], &mut Vec::new())?
            .into_owned();
        let statement = self.elaborate_type(&statement)?;
        let mut state = GoalState::new(&self.ctx, &statement).map_err(|error| ScriptError {
            step: 0,
            tactic: None,
            error,
        })?;
        for (i, step) in script.iter().enumerate() {
            let mut hyps: Vec<String> = state
                .goals()
                .first()
                .map(|g| g.hyps.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default();
            let mut resolve = |t: &Term| -> Result<Term, InterfaceError> {
                Ok(self.resolve(t, &[], &mut hyps)?.into_owned())
            };
            let step = match step {
                TacticStep::Exact(t) => TacticStep::Exact(resolve(t)?),
                TacticStep::Apply(t) => TacticStep::Apply(resolve(t)?),
                TacticStep::Exists(t) => TacticStep::Exists(resolve(t)?),
                TacticStep::Change(t) => TacticStep::Change(resolve(t)?),
                TacticStep::Have(n, t) => TacticStep::Have(n.clone(), resolve(t)?),
                TacticStep::Rewrite(t, reverse) => TacticStep::Rewrite(resolve(t)?, *reverse),
                other => other.clone(),
            };
            state.step(&step).map_err(|error| ScriptError {
                step: i,
                tactic: Some(step.clone()),
                error,
            })?;
        }
        let proof = state.qed().map_err(|error| ScriptError {
            step: script.len(),
            tactic: None,
            error,
        })?;
        self.declare_opaque(name, universes, ty, proof, true)
    }

    /// Register an `Axiom` at the ABSOLUTE key `name`, through `Context::add_universe_poly_declaration`
    /// — the importer's axiom door: the type must be a type, every level variable must be in the
    /// telescope, and no existing declaration may be replaced. It has no body, so `Print Axioms`
    /// reports it on itself and on everything that rests on it.
    fn declare_axiom(
        &mut self,
        name: String,
        universes: Vec<String>,
        ty: Term,
    ) -> Result<String, InterfaceError> {
        let declared = TermParser::declaration_surface(&ty);
        let ty = self
            .resolve(&declared.kernel_type, &[], &mut Vec::new())?
            .into_owned();
        let ty = self.elaborate_type(&ty)?;
        if self.ctx.holds_assumption(&name, &universes, &ty) {
            return Ok(String::new());
        }
        self.ctx
            .add_universe_poly_declaration(&name, universes, ty)
            .map_err(InterfaceError::Kernel)?;
        let implicit_count = declared.leading_implicits();
        self.record_declaration_surface(&name, declared.binders, implicit_count);
        Ok(String::new())
    }

    /// Register one `Inductive` at the ABSOLUTE key `name`, with the namespace scope its
    /// constructors are declared in already entered by the caller.
    ///
    /// Constructors are declared in the namespace the COMMAND is written in — `Namespace N.
    /// Inductive Foo := Mk.` gives `N.Foo` and `N.Mk`. Lean puts a constructor inside the
    /// inductive's own namespace (`Nat.succ`), which is a nicer spelling and a different
    /// convention; ours is the one that is byte-identical to today's flat registration whenever no
    /// namespace is open, and that continuity is worth more than the spelling.
    fn register_inductive(
        &mut self,
        name: String,
        params: Vec<(String, Term)>,
        sort: Term,
        constructors: Vec<(String, Term)>,
    ) -> Result<String, InterfaceError> {
        // The inductive's own key and every constructor key resolve before the context
        // holds any of them: a constructor type names the type it builds.
        let ctor_keys: Vec<String> = constructors
            .iter()
            .map(|(cn, _)| self.declaration_key(cn))
            .collect();
        let mut pending = vec![name.clone()];
        pending.extend(ctor_keys.iter().cloned());
        // Type parameters arrive as `Global`s and become `Var`s only at registration, so
        // they are shielded from resolution by hand.
        let mut locals: Vec<String> = params.iter().map(|(p, _)| p.clone()).collect();
        let params: Vec<(String, Term)> = params
            .iter()
            .map(|(p, t)| {
                self.resolve(t, &pending, &mut locals)
                    .map(|r| (p.clone(), r.into_owned()))
            })
            .collect::<Result<_, _>>()?;
        let constructors: Vec<(String, Term)> = ctor_keys
            .iter()
            .zip(constructors.iter())
            .map(|(key, (_, cty))| {
                self.resolve(cty, &pending, &mut locals)
                    .map(|r| (key.clone(), r.into_owned()))
            })
            .collect::<Result<_, _>>()?;

        // Build polymorphic sort: Π(p1:T1). Π(p2:T2). ... Type
        let poly_sort = build_polymorphic_sort(&params, sort);

        // Register the inductive type with its polymorphic sort
        self.ctx.declare_inductive(&name, poly_sort)?;
        self.surface.namespaces_mut().record_declaration(&name);

        // Register constructors with prepended parameters
        for (ctor_name, ctor_ty) in constructors {
            // Prepend params to constructor type:
            // If ctor_ty = A -> List A -> List A
            // And params = [(A, Type)]
            // Result = Π(A:Type). A -> List A -> List A
            let poly_ctor_ty = build_polymorphic_constructor(&params, ctor_ty);
            // Strict positivity is enforced on the trusted, user-facing
            // registration path: a negative-recursive constructor (e.g.
            // `Cons : (Bad -> False) -> Bad`) would let one inhabit `False`.
            self.ctx
                .add_constructor_checked(&ctor_name, &name, poly_ctor_ty)?;
            self.surface.namespaces_mut().record_declaration(&ctor_name);
        }

        // ★ NOTHING IS DERIVED HERE, AND THAT IS THE FIX. `add_constructor_checked` — the door
        // every constructor above went through — synthesises the whole declaration-time family
        // (`{I}_rec`, `{I}_noConfusionType`, `{I}_noConfusion`, `{C}_inj`, `{I}_below`,
        // `{I}_brecOn`, `decEq{I}`), each one kernel-checked and provenance-keyed so a
        // re-derivation replaces its own prior generation and never a hand-written constant. This
        // used to re-derive `{I}_rec` by hand afterwards and register it with `add_definition`,
        // which paid for a second derivation of a constant that already existed AND disowned the
        // generated one — after which `{I}_below` and `{I}_brecOn` were the only members of the
        // family the user-facing door did not hand back.
        Ok(String::new()) // Silent success
    }

    /// Register one genuine mutual cycle of a run: resolve the whole block at once, register it
    /// through the trusted mutual door, record its names, and derive every member's family. The
    /// mutual door registers through `add_constructor`, the BULK door, which queues synthesis
    /// rather than paying for it per constructor — so unlike the single path (where
    /// `add_constructor_checked` has already done it) the block has to ask. Asking per member rather
    /// than `seal()`ing is deliberate: a seal would also synthesise everything the PRELUDE queued,
    /// which is a second of work for tables this declaration did not ask for.
    fn register_mutual_component(
        &mut self,
        run: &[(&str, &Command)],
        comp: &[usize],
    ) -> Result<(), InterfaceError> {
        // Namespace resolution has to see the WHOLE block at once: every member's key and every
        // constructor's key is pending together, because a mutual block's whole point is that its
        // constructors name each other's types before any of them exists.
        let block = self.resolve_mutual_block(run, comp)?;
        self.ctx
            .add_mutual_inductives(&block)
            .map_err(InterfaceError::Kernel)?;
        for member in &block {
            self.surface
                .namespaces_mut()
                .record_declaration(&member.name);
            for (cn, _) in &member.constructors {
                self.surface.namespaces_mut().record_declaration(cn);
            }
        }
        for member in &block {
            for aux in crate::recursor::derive_auxiliaries(&self.ctx, &member.name) {
                self.ctx.add_definition(aux.name, aux.ty, aux.term);
            }
        }
        Ok(())
    }

    /// Namespace-resolve one mutual-inductive component into the block the trusted machinery takes.
    ///
    /// Two passes, because a mutual block is exactly the case where nothing is in the context yet:
    /// first every member's absolute key and every constructor's absolute key, then each member's
    /// parameter and constructor types resolved with ALL of those keys pending. Each member is
    /// resolved inside the scope its own declaration name spells, exactly as the single path does,
    /// and the scope is wound back whether it succeeds or fails.
    fn resolve_mutual_block(
        &mut self,
        run: &[(&str, &Command)],
        comp: &[usize],
    ) -> Result<Vec<crate::context::MutualInductive>, InterfaceError> {
        let members: Vec<(&String, &Vec<(String, Term)>, &Term, &Vec<(String, Term)>)> = comp
            .iter()
            .map(|&node| match run[node].1 {
                Command::Inductive {
                    name,
                    params,
                    sort,
                    constructors,
                } => (name, params, sort, constructors),
                _ => unreachable!("a component holds only Inductive commands"),
            })
            .collect();

        let mut keys: Vec<(String, Vec<String>)> = Vec::with_capacity(members.len());
        for (name, _, _, constructors) in &members {
            let (key, mark) = self.enter_declaration(name);
            let ctors: Vec<String> = constructors
                .iter()
                .map(|(cn, _)| self.declaration_key(cn))
                .collect();
            self.surface.namespaces_mut().rewind(mark);
            keys.push((key, ctors));
        }
        let mut pending: Vec<String> = Vec::new();
        for (key, ctors) in &keys {
            pending.push(key.clone());
            pending.extend(ctors.iter().cloned());
        }

        let mut block = Vec::with_capacity(members.len());
        for ((name, params, sort, constructors), (key, ctor_keys)) in members.iter().zip(&keys) {
            let (_, mark) = self.enter_declaration(name);
            let resolved = (|| -> Result<crate::context::MutualInductive, InterfaceError> {
                let mut locals: Vec<String> = params.iter().map(|(p, _)| p.clone()).collect();
                let params: Vec<(String, Term)> = params
                    .iter()
                    .map(|(p, t)| {
                        self.resolve(t, &pending, &mut locals)
                            .map(|r| (p.clone(), r.into_owned()))
                    })
                    .collect::<Result<_, _>>()?;
                let ctors: Vec<(String, Term)> = ctor_keys
                    .iter()
                    .zip(constructors.iter())
                    .map(|(ck, (_, cty))| {
                        self.resolve(cty, &pending, &mut locals).map(|r| {
                            (
                                ck.clone(),
                                build_polymorphic_constructor(&params, r.into_owned()),
                            )
                        })
                    })
                    .collect::<Result<_, _>>()?;
                Ok(crate::context::MutualInductive {
                    name: key.clone(),
                    sort: build_polymorphic_sort(&params, (*sort).clone()),
                    num_params: params.len(),
                    constructors: ctors,
                })
            })();
            self.surface.namespaces_mut().rewind(mark);
            block.push(resolved?);
        }
        Ok(block)
    }

    /// The absolute key a name written HERE declares — the current namespace, then the name. A name
    /// no term can spell keeps its flat key, for the reason [`enter_declaration`](Self::enter_declaration)
    /// gives.
    fn declaration_key(&self, written: &str) -> String {
        let key = self.written_key(written);
        self.ctx.shadow_of(&key).unwrap_or(key)
    }

    /// Execute a batch of Vernacular commands, recovering mutual-inductive grouping.
    ///
    /// [`execute`](Self::execute) registers each `Inductive` independently, so a forward
    /// reference between two separately-declared inductives — `Tree` whose `Node`
    /// constructor mentions `Forest`, declared next — fails: `Forest` is not yet in scope
    /// when `Tree`'s constructor is universe-checked. True mutual inductives must be
    /// registered together, through the trusted [`Context::add_mutual_inductives`], which
    /// runs whole-block positivity and a header-first universe check.
    ///
    /// This entry point recovers that grouping from a flat statement sequence: a maximal
    /// run of consecutive `Inductive` commands is split into strongly-connected components
    /// of the "a constructor mentions sibling `b`" graph. A genuine cycle (`Tree`↔`Forest`)
    /// is registered as one mutual block; an acyclic reference (`Leafy` → `Bare`) becomes
    /// two singletons registered dependency-first (`Bare`, then `Leafy`) through the
    /// ordinary single-inductive path. Every non-inductive command — and every inductive
    /// with no forward reference — runs exactly as `execute` would, in source order, so a
    /// batch without forward references is byte-identical to running the statements singly.
    pub fn execute_batch(&mut self, inputs: &[String]) -> Vec<Result<String, InterfaceError>> {
        self.run_batch(inputs, false)
    }

    /// Execute a whole DOCUMENT: every command in it, in order, stopping at the first failure.
    ///
    /// [`execute`](Self::execute) is a single-command door and refuses a prefix, which is what
    /// stops a second `## Theorem` block from being discarded in silence. This is the other half:
    /// the door that actually reads a document. It is fail-loud and fail-EARLY — a block that does
    /// not check aborts the run, so nothing after a refused theorem is registered and the caller
    /// gets the error rather than a success that covered a smaller set of blocks than it was
    /// handed. Mutual-inductive grouping is the same as [`Self::execute_batch`]'s.
    ///
    /// It is also BALANCED: a document must leave the scope stack exactly as it found it. A
    /// `Namespace N.` with no `End N.` would write every LATER command into `N` with nothing said,
    /// and a document that closes a scope the caller opened moves the caller's later declarations
    /// somewhere nobody wrote. Lean does NOT refuse one — measured on lean 4.14.0, a file that ends inside `namespace N` compiles without a word, because a Lean MODULE's scopes close at its end and the next file starts clean. A document here is not a module: it is handed to a live session that goes on executing commands after it, so an unclosed `Namespace N.` keys every later one into `N` and says nothing. The balance rule is therefore ours and deliberate, and the reason is the session rather than Lean.
    /// A refused document has its namespace scope wound back to where the caller handed it over.
    pub fn execute_document(&mut self, input: &str) -> Result<Vec<String>, InterfaceError> {
        let entry_scope = self.surface.namespaces().mark();
        let entry_namespaces = self.surface.namespaces().depth();
        let entry_sections = self.surface.section_depth();

        let mut outcome = self.read_document(input);

        // \u{2605} A DOCUMENT IS BALANCED. `Namespace N.` with no `End N.` is the quietest failure this
        // surface has: every later declaration lands in `N`, nothing is reported, and the file
        // still checks. Lean itself does NOT refuse one (measured: a file ending inside
        // `namespace N` compiles) — a MODULE's scopes close at its end, and a document handed to a
        // live session has no such end. Balance, rather than "empty
        // at the end", is the right statement of it \u{2014} a caller already inside a namespace when it
        // hands the door a document is not doing anything wrong, and a document that unwinds PAST
        // the scope it was handed is the same mistake pointing the other way.
        if outcome.is_ok() {
            let frames = self.surface.namespaces().frame_names();
            let sections = self.surface.section_names();
            let unclosed: Vec<String> = frames
                .iter()
                .skip(entry_namespaces)
                .chain(sections.iter().skip(entry_sections))
                .cloned()
                .collect();
            let overclosed = entry_namespaces.saturating_sub(frames.len())
                + entry_sections.saturating_sub(sections.len());
            if !unclosed.is_empty() || overclosed > 0 {
                outcome = Err(ParseError::UnbalancedDocument {
                    unclosed,
                    overclosed,
                }
                .into());
            }
        }

        // A refused document must not leave its scope behind, for the same reason a refused
        // `Open \u{2026} in <command>` must not: the NEXT command would read its names in a scope nobody
        // wrote. The namespace scope is restored exactly; a section the document closed took its
        // notations with it and is reported above rather than pretended back into existence.
        if outcome.is_err() {
            self.surface.namespaces_mut().rewind(entry_scope);
            self.surface.close_sections_to(entry_sections);
            // The KERNEL scope is wound back with it. A refused document that left one open would
            // have the next document's `local` registrations owned by a depth nobody wrote, and
            // nothing would ever revert them.
            self.match_kernel_scopes();
        }
        outcome
    }

    /// [`execute_document`](Self::execute_document) without the balance rule: split the document
    /// and run every block, fail-early.
    fn read_document(&mut self, input: &str) -> Result<Vec<String>, InterfaceError> {
        let blocks = split_document(&self.surface, input)?;
        let mut out = Vec::with_capacity(blocks.len());
        for result in self.run_batch(&blocks, true) {
            out.push(result?);
        }
        Ok(out)
    }

    /// The shared engine of [`execute_batch`] and [`execute_document`]: run each input in source
    /// order, grouping maximal runs of `Inductive` commands so mutual cycles register together.
    /// `stop_on_error` decides whether a failure aborts the rest (a document) or is recorded and
    /// the run continues (a batch).
    fn run_batch(
        &mut self,
        inputs: &[String],
        stop_on_error: bool,
    ) -> Vec<Result<String, InterfaceError>> {
        let parsed: Vec<Option<Command>> = inputs
            .iter()
            .map(|s| parse_command_in_ctx(&self.ctx, &self.surface, s).ok())
            .collect();
        let mut out: Vec<Option<Result<String, InterfaceError>>> =
            (0..inputs.len()).map(|_| None).collect();
        let mut i = 0;
        while i < inputs.len() {
            if !matches!(parsed[i], Some(Command::Inductive { .. })) {
                let r = self.execute(&inputs[i]);
                let failed = r.is_err();
                out[i] = Some(r);
                i += 1;
                if failed && stop_on_error {
                    break;
                }
                continue;
            }
            // A maximal run of consecutive inductive commands, registered as a group.
            let start = i;
            while i < inputs.len() && matches!(parsed[i], Some(Command::Inductive { .. })) {
                i += 1;
            }
            let run: Vec<(&str, &Command)> = (start..i)
                .map(|k| (inputs[k].as_str(), parsed[k].as_ref().unwrap()))
                .collect();
            let mut failed = false;
            for (k, r) in (start..i).zip(self.register_inductive_run(&run)) {
                failed |= r.is_err();
                out[k] = Some(r);
            }
            if failed && stop_on_error {
                break;
            }
        }
        // A document that stopped early leaves the untouched tail unreported: it was never run,
        // and calling it `Ok("")` would be a success nobody earned.
        out.into_iter().flatten().collect()
    }

    /// Register one maximal run of `Inductive` commands, grouping mutual cycles into
    /// blocks and registering acyclic references dependency-first. See [`execute_batch`].
    fn register_inductive_run(
        &mut self,
        run: &[(&str, &Command)],
    ) -> Vec<Result<String, InterfaceError>> {
        use std::collections::{BTreeSet, HashMap};
        let n = run.len();
        let names: Vec<&str> = run
            .iter()
            .map(|(_, c)| match c {
                Command::Inductive { name, .. } => name.as_str(),
                _ => unreachable!("run holds only Inductive commands"),
            })
            .collect();
        let name_idx: HashMap<&str, usize> =
            names.iter().enumerate().map(|(k, &nm)| (nm, k)).collect();

        // Dependency edges: `a → b` iff a constructor of member `a` mentions sibling `b`
        // (b ≠ a; a self-reference is handled inside single/mutual registration).
        let mut deps: Vec<BTreeSet<usize>> = (0..n).map(|_| BTreeSet::new()).collect();
        for (a, (_, cmd)) in run.iter().enumerate() {
            if let Command::Inductive { constructors, .. } = cmd {
                let mut mentioned = BTreeSet::new();
                for (_, cty) in constructors.iter() {
                    collect_global_names(cty, &mut mentioned);
                }
                for name in &mentioned {
                    if let Some(&b) = name_idx.get(name.as_str()) {
                        if b != a {
                            deps[a].insert(b);
                        }
                    }
                }
            }
        }

        let comps = tarjan_scc(&deps);
        let mut comp_of = vec![0usize; n];
        for (cid, comp) in comps.iter().enumerate() {
            for &node in comp {
                comp_of[node] = cid;
            }
        }
        // Condensation edges: which components each component references.
        let mut comp_deps: Vec<BTreeSet<usize>> =
            (0..comps.len()).map(|_| BTreeSet::new()).collect();
        for (a, edges) in deps.iter().enumerate() {
            for &b in edges {
                if comp_of[a] != comp_of[b] {
                    comp_deps[comp_of[a]].insert(comp_of[b]);
                }
            }
        }
        // Emit components dependency-first (a referenced component before its referrer),
        // tie-breaking by earliest source position so independent inductives keep order.
        let mut emitted = vec![false; comps.len()];
        let mut order: Vec<usize> = Vec::with_capacity(comps.len());
        while order.len() < comps.len() {
            let mut pick: Option<usize> = None;
            for c in 0..comps.len() {
                if emitted[c] || !comp_deps[c].iter().all(|d| emitted[*d]) {
                    continue;
                }
                let earliest = *comps[c].iter().min().unwrap();
                match pick {
                    Some(p) if earliest >= *comps[p].iter().min().unwrap() => {}
                    _ => pick = Some(c),
                }
            }
            let c = pick.expect("an SCC condensation is a DAG — a ready component exists");
            emitted[c] = true;
            order.push(c);
        }

        let mut results: Vec<Result<String, InterfaceError>> =
            (0..n).map(|_| Ok(String::new())).collect();
        for &cid in &order {
            let comp = &comps[cid];
            if comp.len() == 1 {
                // Singleton (including a self-recursive inductive): the ordinary path, which
                // also derives the recursor. Its dependencies are already registered.
                let node = comp[0];
                results[node] = self.execute(run[node].0);
            } else {
                // A genuine mutual cycle → one block through the trusted mutual machinery.
                let mut declared = Vec::new();
                for &node in comp {
                    declared.extend(self.declared_keys(run[node].1));
                }
                let outcome = if self.redeclares(&declared) {
                    self.shadowing(&declared, |repl| repl.register_mutual_component(run, comp))
                } else {
                    self.register_mutual_component(run, comp)
                };
                let outcome = outcome.map_err(|e| match e {
                    InterfaceError::Kernel(e) => {
                        format!("mutual inductive block failed to register: {e}")
                    }
                    e => format!("mutual inductive block failed to register: {e}"),
                });
                if let Err(msg) = outcome {
                    for &node in comp {
                        results[node] = Err(InterfaceError::Kernel(
                            crate::KernelError::CertificationError(msg.clone()),
                        ));
                    }
                }
            }
        }
        results
    }

    /// The names this session shadows, as `(written name, kernel identity)` pairs: a written name
    /// in this table denotes the session's own declaration at that identity, and the environment's
    /// declaration under the same name keeps its meaning.
    pub fn shadows(&self) -> Vec<(String, String)> {
        self.ctx.shadows().to_vec()
    }

    /// How many commands this session has run under the shadowing snapshot — exactly the commands
    /// that redeclared a name the environment already held. The snapshot is what makes the retry
    /// possible and is not free: while it lives, a command's first write to each table it touches
    /// copies the whole table. A command declaring only fresh names never takes one, and this count
    /// is how that is held.
    pub fn shadow_snapshots(&self) -> usize {
        self.snapshots
    }

    /// `term` with every constant named as it is WRITTEN in this session
    /// ([`Context::written_name`]) — what `Check` and `Eval` print, so a shadowing declaration
    /// prints as the name its author wrote and never as its kernel identity.
    fn written<'t>(&self, term: &'t Term) -> Cow<'t, Term> {
        if self.ctx.shadows().is_empty() {
            return Cow::Borrowed(term);
        }
        Cow::Owned(rename_constants(term, &|key| self.ctx.written_name(key)))
    }

    /// Get a reference to the surface environment — the notation and macros in scope.
    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Get a reference to the underlying context.
    pub fn context(&self) -> &Context {
        &self.ctx
    }

    /// Get a mutable reference to the underlying context.
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.ctx
    }
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

/// Split a document into its commands, in source order.
///
/// Each block is the exact source text its command occupies (`super::measure_command_prefix`),
/// so a literate theorem keeps its indented `Statement:`/`Proof:` lines and a Coq-style sentence
/// keeps its terminating period. A block is MEASURED here, from its syntax, and read for its
/// meaning only when it runs — against the context every block before it built. A syntax error
/// anywhere aborts the split: a document is read whole or not at all.
fn split_document(surface: &Surface, input: &str) -> Result<Vec<String>, ParseError> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while !input[offset..].trim().is_empty() {
        let consumed = super::measure_command_prefix(surface, &input[offset..])?;
        if consumed == 0 {
            return Err(ParseError::Missing(
                "a command — the document parser made no progress".to_string(),
            ));
        }
        let text = input[offset..offset + consumed].trim();
        if !text.is_empty() {
            blocks.push(text.to_string());
        }
        offset += consumed;
    }
    Ok(blocks)
}

/// Build a polymorphic sort from type parameters.
///
/// For params = [(A, Type), (B, Type)] and base_sort = Type,
/// produces: Π(A:Type). Π(B:Type). Type
fn build_polymorphic_sort(params: &[(String, Term)], base_sort: Term) -> Term {
    // Fold right to build nested Pi types
    params
        .iter()
        .rev()
        .fold(base_sort, |body, (name, ty)| Term::Pi {
            param: name.clone(),
            param_type: Box::new(ty.clone()),
            body_type: Box::new(body),
        })
}

/// Build a polymorphic constructor type by prepending parameters.
///
/// For params = [(A, Type)] and ctor_ty = A -> List A -> List A,
/// produces: Π(A:Type). A -> List A -> List A
///
/// The kernel uses named variables (Var(String)), so we convert
/// Global("A") to Var("A") for parameters.
fn build_polymorphic_constructor(params: &[(String, Term)], ctor_ty: Term) -> Term {
    if params.is_empty() {
        return ctor_ty;
    }

    // Convert Global(param_name) to Var(param_name) for all parameters
    let param_names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    let body = substitute_globals_with_vars(&ctor_ty, &param_names);

    // Wrap with Pi bindings (fold right to build nested Pi)
    params.iter().rev().fold(body, |body, (name, ty)| Term::Pi {
        param: name.clone(),
        param_type: Box::new(ty.clone()),
        body_type: Box::new(body),
    })
}

/// `Π(p₀ : T₀). … Π(pₙ : Tₙ). body` over a block member's written parameters, each parameter's
/// references turned from the `Global` the text-only parser read into the `Var` its binder binds —
/// in the LATER parameters' types as well as in `body`, so a dependent telescope
/// (`(A : Type) (x : A)`) binds what it says. A parameter shadows a global of the same spelling,
/// exactly as a binder does.
fn parameter_telescope(params: &[(String, Term)], body: Term) -> Term {
    let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    let body = substitute_globals_with_vars(&body, &names);
    params
        .iter()
        .enumerate()
        .rev()
        .fold(body, |body, (i, (name, ty))| Term::Pi {
            param: name.clone(),
            param_type: Box::new(substitute_globals_with_vars(ty, &names[..i])),
            body_type: Box::new(body),
        })
}

/// The keys a declaring command WRITES — what a `Protected` in front of it protects.
fn declared_names(cmd: &Command) -> Vec<String> {
    match cmd {
        Command::Definition { name, .. }
        | Command::Inductive { name, .. }
        | Command::Theorem { name, .. }
        | Command::TheoremBy { name, .. }
        | Command::Opaque { name, .. }
        | Command::Axiom { name, .. } => vec![name.clone()],
        Command::InductiveBlock { members } => members.iter().map(|m| m.name.clone()).collect(),
        Command::Universes { inner, .. }
        | Command::Protected(inner)
        | Command::OpenIn { inner, .. } => declared_names(inner),
        _ => Vec::new(),
    }
}

/// `term` with every constant — a `Global` or a `Const` — renamed by `name`; binders, literals and
/// every other node are kept as they are.
fn rename_constants(term: &Term, name: &dyn Fn(&str) -> String) -> Term {
    let go = |t: &Term| Box::new(rename_constants(t, name));
    match term {
        Term::Global(n) => Term::Global(name(n)),
        Term::Const { name: n, levels } => Term::Const {
            name: name(n),
            levels: levels.clone(),
        },
        Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => term.clone(),
        Term::App(f, a) => Term::App(go(f), go(a)),
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: go(param_type),
            body: go(body),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: go(param_type),
            body_type: go(body_type),
        },
        Term::Fix { name: n, body } => Term::Fix {
            name: n.clone(),
            body: go(body),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), rename_constants(b, name)))
                .collect(),
            index: *index,
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: go(discriminant),
            motive: go(motive),
            cases: cases.iter().map(|c| rename_constants(c, name)).collect(),
        },
        Term::Let {
            name: n,
            ty,
            value,
            body,
        } => Term::Let {
            name: n.clone(),
            ty: go(ty),
            value: go(value),
            body: go(body),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: go(term),
        },
    }
}

/// Convert Global(name) to Var(name) for names in the param list.
/// This makes parameter references in constructor types into bound variables.
fn substitute_globals_with_vars(term: &Term, param_names: &[&str]) -> Term {
    match term {
        Term::Global(n) if param_names.contains(&n.as_str()) => Term::Var(n.clone()),
        Term::Global(n) => Term::Global(n.clone()),
        Term::Const { .. } => term.clone(),
        Term::Var(n) => Term::Var(n.clone()),
        Term::Sort(u) => Term::Sort(u.clone()),
        Term::Lit(l) => Term::Lit(l.clone()),
        Term::App(f, a) => Term::App(
            Box::new(substitute_globals_with_vars(f, param_names)),
            Box::new(substitute_globals_with_vars(a, param_names)),
        ),
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(substitute_globals_with_vars(param_type, param_names)),
            body: Box::new(substitute_globals_with_vars(body, param_names)),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(substitute_globals_with_vars(param_type, param_names)),
            body_type: Box::new(substitute_globals_with_vars(body_type, param_names)),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(substitute_globals_with_vars(body, param_names)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), substitute_globals_with_vars(b, param_names)))
                .collect(),
            index: *index,
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(substitute_globals_with_vars(discriminant, param_names)),
            motive: Box::new(substitute_globals_with_vars(motive, param_names)),
            cases: cases
                .iter()
                .map(|c| substitute_globals_with_vars(c, param_names))
                .collect(),
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(substitute_globals_with_vars(ty, param_names)),
            value: Box::new(substitute_globals_with_vars(value, param_names)),
            body: Box::new(substitute_globals_with_vars(body, param_names)),
        },
        Term::Hole => Term::Hole, // Holes are unchanged
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(substitute_globals_with_vars(term, param_names)),
        },
    }
}

/// Collect the names of every `Global`/`Const` referenced anywhere in a term — used to
/// find which sibling inductives a constructor mentions.
fn collect_global_names(term: &Term, out: &mut std::collections::BTreeSet<String>) {
    match term {
        Term::Global(n) => {
            out.insert(n.clone());
        }
        Term::Const { name, .. } => {
            out.insert(name.clone());
        }
        Term::App(f, a) => {
            collect_global_names(f, out);
            collect_global_names(a, out);
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            collect_global_names(param_type, out);
            collect_global_names(body_type, out);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            collect_global_names(param_type, out);
            collect_global_names(body, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            collect_global_names(discriminant, out);
            collect_global_names(motive, out);
            for c in cases {
                collect_global_names(c, out);
            }
        }
        Term::Fix { body, .. } => collect_global_names(body, out),
        Term::MutualFix { defs, .. } => {
            for (_, b) in defs {
                collect_global_names(b, out);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            collect_global_names(ty, out);
            collect_global_names(value, out);
            collect_global_names(body, out);
        }
        Term::Annotated { term, .. } => collect_global_names(term, out),
        Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => {}
    }
}

/// Tarjan's strongly-connected components over a small adjacency list. Each returned
/// vector is one SCC (node indices, ascending); a self-referential singleton is its own
/// SCC of size one. Runs are tiny (a handful of adjacent inductives), so recursion is safe.
fn tarjan_scc(adj: &[std::collections::BTreeSet<usize>]) -> Vec<Vec<usize>> {
    struct State<'a> {
        adj: &'a [std::collections::BTreeSet<usize>],
        index: Vec<usize>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        counter: usize,
        sccs: Vec<Vec<usize>>,
    }
    impl State<'_> {
        fn connect(&mut self, v: usize) {
            self.index[v] = self.counter;
            self.low[v] = self.counter;
            self.counter += 1;
            self.stack.push(v);
            self.on_stack[v] = true;
            let neighbors: Vec<usize> = self.adj[v].iter().copied().collect();
            for w in neighbors {
                if self.index[w] == usize::MAX {
                    self.connect(w);
                    self.low[v] = self.low[v].min(self.low[w]);
                } else if self.on_stack[w] {
                    self.low[v] = self.low[v].min(self.index[w]);
                }
            }
            if self.low[v] == self.index[v] {
                let mut comp = Vec::new();
                loop {
                    let w = self.stack.pop().unwrap();
                    self.on_stack[w] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                comp.sort_unstable();
                self.sccs.push(comp);
            }
        }
    }
    let n = adj.len();
    let mut st = State {
        adj,
        index: vec![usize::MAX; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        counter: 0,
        sccs: Vec::new(),
    };
    for v in 0..n {
        if st.index[v] == usize::MAX {
            st.connect(v);
        }
    }
    st.sccs
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/repl/mutual_batch_tests.rs"]
mod mutual_batch_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/repl/shared_prelude_controls.rs"]
mod shared_prelude_controls;
