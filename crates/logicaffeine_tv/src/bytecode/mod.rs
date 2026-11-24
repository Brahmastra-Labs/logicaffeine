//! Translation validation of the register VM's bytecode: for every input, each compiled function computes what its
//! source computes.
//!
//! The source side is the program as written, lowered to a control-flow graph whose meaning is the tree-walker's
//! ([`source`]); the bytecode side is the [`CompiledProgram`] a target compiles ([`Target`]), taken op by op as the
//! VM's handlers compute ([`machine`]). Both run symbolically over exact integers — the runtime promotes an
//! overflowing `i64` rather than wrapping — from each function's entry and each loop head to the next, and the proof
//! pairs them ([`product`]): Houdini infers, at each loop head, which registers equal which variables, and Z3 shows
//! that every pair of paths that can run together ends alike having shown the same values. A loop is proved for every
//! trip count, not unrolled.
//!
//! A call is taken the same way on both sides: a loop-free, non-recursive, silent callee runs in place (each side its
//! own code); a looping or recursive silent one is an uninterpreted function of its arguments, its own proof assumed
//! and named in the verdict; one that shows a value is outside the fragment.
//!
//! The two executors are themselves held to the engines they model: [`check_executors`] runs each concretely on
//! `## Main` and compares what it shows with the tree-walker and with the VM running the same compiled program.

pub mod machine;
pub mod path;
pub mod product;
pub mod source;
pub mod value;

/// The source side of a proof: one lowered function, run by the source executor.
pub struct SourceSide<'a, 'p> {
    pub run: &'a SourceRun<'p>,
    pub f: &'a source::SFunc,
}

impl product::Side for SourceSide<'_, '_> {
    type Loc = source::VarId;

    fn loops(&self) -> &[path::LoopNode] {
        &self.f.loops
    }

    fn entry(&self, params: &[SymVal]) -> State<source::VarId> {
        State { env: self.f.params.iter().zip(params).map(|((v, _), p)| (*v, p.clone())).collect(), depth: 0 }
    }

    fn explore(
        &self,
        from: Option<usize>,
        state: State<source::VarId>,
    ) -> Result<Vec<path::Path<source::VarId>>, crate::symexec::Unsupported> {
        self.run.explore(self.f, from.map_or(0, |k| self.f.loops[k].head), state)
    }

    fn describe(&self, l: &source::VarId) -> String {
        self.f.names[*l as usize].clone()
    }
}

/// The bytecode side of a proof: one compiled function, run by the bytecode executor.
pub struct MachineSide<'a, 'p, 'c> {
    pub run: &'a MachineRun<'p, 'c>,
    pub f: &'a machine::BFunc,
}

impl product::Side for MachineSide<'_, '_, '_> {
    type Loc = machine::Loc;

    fn loops(&self) -> &[path::LoopNode] {
        &self.f.loops
    }

    fn entry(&self, params: &[SymVal]) -> State<machine::Loc> {
        self.run.entry_state(self.f, params)
    }

    fn explore(
        &self,
        from: Option<usize>,
        state: State<machine::Loc>,
    ) -> Result<Vec<path::Path<machine::Loc>>, crate::symexec::Unsupported> {
        self.run.explore(self.f, from.map_or(self.f.entry, |k| self.f.loops[k].head), state)
    }

    fn describe(&self, l: &machine::Loc) -> String {
        l.describe()
    }
}

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_compile::analysis::{PolicyRegistry, TypeRegistry};
use logicaffeine_compile::ast::stmt::Stmt;
use logicaffeine_compile::vm::{CompiledProgram, Compiler, Vm};
use logicaffeine_compile::Interner;

use machine::{analyze_program, BProgram, MachineRun};
use path::{fresh, Budget, CallKind, Calls, End, Mode, State};
use product::{Finding, Prover};
use source::{lower_program, SProgram, SourceRun};
use value::{Kind, SymVal};

/// Which compiled program is validated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The program as written, compiled with the oracle's facts: what the engine differentials run.
    Vm,
    /// The program after the run-path optimizer, compiled the same way: what `largo run` runs.
    Run,
}

/// What the validator concluded about one function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// For every input, the bytecode shows what the source shows and ends as it ends. `invariants` names, per loop,
    /// the relation between registers and variables the proof found inductive; `assumed`, the callees taken as
    /// uninterpreted functions, whose own proofs this one rests on.
    Proven { invariants: Vec<Vec<String>>, assumed: Vec<String> },
    /// For every input the compiled code shows what the source shows and ends as it ends, or stops with a loud integer
    /// overflow where the source holds a value past 64 bits: it never computes a wrong answer.
    ProvenUpToOverflow { invariants: Vec<Vec<String>>, assumed: Vec<String> },
    /// A miscompile: an input on which the two, run concretely, do different things. `at` is the obligation that
    /// failed first.
    Refuted { input: String, source: String, bytecode: String, at: String },
    /// An obligation failed on a state the proof could not exclude, and no input tried showed the two apart: the
    /// state may be unreachable (an invariant Houdini could not find) or depend on an uninterpreted callee.
    Unproven { at: String, state: String },
    /// The function is outside the fragment the validator models.
    Unsupported { reason: String },
    /// Z3 gave no answer in time.
    Unknown { reason: String },
}

impl Verdict {
    pub fn is_proven(&self) -> bool {
        matches!(self, Verdict::Proven { .. })
    }
}

/// One function's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionReport {
    pub name: String,
    pub verdict: Verdict,
}

/// Every function's verdict, `## Main` first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytecodeReport {
    pub target: Target,
    pub functions: Vec<FunctionReport>,
}

impl BytecodeReport {
    pub fn verdict(&self, name: &str) -> Option<&Verdict> {
        self.functions.iter().find(|f| f.name == name).map(|f| &f.verdict)
    }

    /// Nothing was refuted.
    pub fn refutes_nothing(&self) -> bool {
        self.functions.iter().all(|f| !matches!(f.verdict, Verdict::Refuted { .. }))
    }
}

/// What the concrete runs of the two executors found against the engines they model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutorCheck {
    /// The source executor shows what the tree-walker shows, and the bytecode executor what the VM shows.
    Agree,
    /// An executor disagrees with its engine: which, and how.
    Disagree(String),
    /// The program is outside the fragment the executors model.
    Unsupported(String),
}

fn compile_parsed(
    parsed: Result<(&[Stmt], &TypeRegistry, PolicyRegistry), String>,
    interner: &Interner,
) -> Result<(CompiledProgram, Vec<String>), String> {
    let (stmts, types, _) = parsed?;
    let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
    let program = Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))?;
    let names = program.functions.iter().map(|f| interner.resolve(f.name).to_string()).collect();
    Ok((program, names))
}

/// The compiled program a target runs, with each function's name.
pub fn compile(source: &str, target: Target) -> Result<(CompiledProgram, Vec<String>), String> {
    match target {
        Target::Vm => logicaffeine_compile::ui_bridge::with_parsed_program(source, compile_parsed),
        Target::Run => logicaffeine_compile::ui_bridge::with_optimized_program(source, compile_parsed),
    }
}

/// The source program as written, every function lowered to its graph.
pub(crate) fn lower_source(source: &str) -> Result<SProgram, String> {
    logicaffeine_compile::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed?;
        Ok(lower_program(stmts, interner))
    })
}

/// How each source function is called: run in place when it has no loop and does not reach itself, uninterpreted
/// when it loops or recurses, refused when it (or anything it calls) shows a value or is outside the fragment.
pub(crate) fn call_policy(program: &SProgram) -> Calls {
    let callees: BTreeMap<&str, BTreeSet<&str>> = program
        .functions
        .iter()
        .map(|(n, f)| (n.as_str(), f.as_ref().map(|f| f.calls.iter().map(String::as_str).collect()).unwrap_or_default()))
        .collect();
    let reach = |from: &str| -> BTreeSet<&str> {
        let mut seen = BTreeSet::new();
        let mut work: Vec<&str> = callees.get(from).into_iter().flatten().copied().collect();
        while let Some(n) = work.pop() {
            if seen.insert(n) {
                work.extend(callees.get(n).into_iter().flatten().copied());
            }
        }
        seen
    };
    let mut calls = Calls::new();
    for (name, f) in &program.functions {
        let kind = match f {
            Err(why) => CallKind::Refused(why.0.clone()),
            Ok(f) => {
                let reached = reach(name);
                let shows = f.shows
                    || reached.iter().any(|n| matches!(program.functions.get(*n), Some(Ok(g)) if g.shows));
                if shows {
                    CallKind::Refused("it shows a value".into())
                } else if f.loops.is_empty() && !reached.contains(name.as_str()) {
                    CallKind::Inline
                } else {
                    CallKind::Opaque { ret: f.ret }
                }
            }
        };
        calls.insert(name.clone(), kind);
    }
    calls
}

/// Validate every function of `source` and `## Main` against the bytecode `target` compiles.
pub fn validate(source: &str, target: Target) -> Result<BytecodeReport, String> {
    validate_edited(source, target, |_, _| {})
}

/// [`validate`] against the bytecode `edit` leaves of the compiled program — a seeded miscompile, which the validator
/// must refute. `edit` is handed the program and each function's name. The proofs run in a Z3 context of their own,
/// so a verdict does not depend on what the process asked Z3 before: the solver's search follows the order its terms
/// were made in.
pub fn validate_edited(
    source: &str,
    target: Target,
    edit: impl FnOnce(&mut CompiledProgram, &[String]),
) -> Result<BytecodeReport, String> {
    let sp = lower_source(source)?;
    let (mut program, names) = compile(source, target)?;
    edit(&mut program, &names);
    let bp = analyze_program(&program, &names);
    let calls = call_policy(&sp);
    let functions = z3::with_z3_config(&z3::Config::new(), || {
        let budget = Budget::new(0);
        let src = SourceRun { program: &sp, calls: &calls, mode: Mode::Symbolic, budget: &budget };
        let mac = MachineRun { program: &bp, calls: &calls, mode: Mode::Symbolic, budget: &budget };
        let replay = Replay { sp: &sp, bp: &bp, calls: &calls };
        let mut functions = vec![FunctionReport {
            name: "## Main".into(),
            verdict: prove_one(&src, &mac, &calls, &replay, sp.main.as_ref(), None, bp.main.as_ref()),
        }];
        for (name, sf) in &sp.functions {
            let verdict = match bp.functions.get(name) {
                Some((_, bf)) => prove_one(&src, &mac, &calls, &replay, sf.as_ref(), sp.tail.get(name), bf.as_ref()),
                None => Verdict::Unsupported { reason: "the compiled program has no such function".into() },
            };
            functions.push(FunctionReport { name: name.clone(), verdict });
        }
        functions
    });
    Ok(BytecodeReport { target, functions })
}

fn prove_one(
    src: &SourceRun,
    mac: &MachineRun,
    calls: &Calls,
    replay: &Replay,
    sf: Result<&source::SFunc, &crate::symexec::Unsupported>,
    looped: Option<&source::SFunc>,
    bf: Result<&machine::BFunc, &crate::symexec::Unsupported>,
) -> Verdict {
    let (sf, bf) = match (sf, bf) {
        (Ok(s), Ok(b)) => (s, b),
        (Err(why), _) | (_, Err(why)) => return Verdict::Unsupported { reason: why.0.clone() },
    };
    let nest = |ls: &[path::LoopNode]| ls.iter().map(|l| l.parent).collect::<Vec<_>>();
    let sf = match looped {
        Some(l) if nest(&sf.loops) != nest(&bf.loops) && nest(&l.loops) == nest(&bf.loops) => l,
        _ => sf,
    };
    if bf.in_out {
        return Verdict::Unsupported { reason: "a parameter is copied back to the caller".into() };
    }
    let params: Vec<SymVal> = sf
        .params
        .iter()
        .map(|(v, k)| fresh(*k, &format!("{}.{}", sf.name, sf.names[*v as usize])))
        .collect();
    let assumed = sf
        .calls
        .iter()
        .filter(|c| matches!(calls.get(*c), Some(CallKind::Opaque { .. })))
        .cloned()
        .collect();
    let (source, target) = (SourceSide { run: src, f: sf }, MachineSide { run: mac, f: bf });
    let prover = Prover { source: &source, target: &target, what: "the bytecode" };
    match prover.prove(&params, assumed) {
        Ok(Finding::Proven { invariants, assumed, overflow: false }) => Verdict::Proven { invariants, assumed },
        Ok(Finding::Proven { invariants, assumed, overflow: true }) => Verdict::ProvenUpToOverflow { invariants, assumed },
        Ok(Finding::Unknown { reason }) => Verdict::Unknown { reason },
        Ok(Finding::Failed { at, state, values }) => match replay.search(sf, bf, &values) {
            Some((input, source, bytecode)) => Verdict::Refuted { input, source, bytecode, at },
            None => Verdict::Unproven { at, state },
        },
        Err(why) => Verdict::Unsupported { reason: why.0 },
    }
}

/// The integers a replay tries for each `Int` parameter, after the ones Z3's model names: zero and its neighbours,
/// small primes and powers of two, and the `i64` rims.
const GRID: [i64; 20] = [
    0,
    1,
    -1,
    2,
    -2,
    3,
    -3,
    5,
    7,
    8,
    -8,
    10,
    13,
    100,
    -100,
    1 << 31,
    -(1 << 31),
    i64::MAX,
    i64::MIN,
    -7,
];

/// How many argument tuples a replay tries.
const REPLAYS: usize = 512;

/// The steps one concrete replay may take before it is abandoned as inconclusive.
pub(crate) const REPLAY_STEPS: u64 = 20_000;

/// The steps every replay of one function may take together.
pub(crate) const REPLAY_TOTAL: u64 = 2_000_000;

/// The argument tuples a replay of `sf` tries: the integers Z3's model named first, then the grid.
pub(crate) fn replay_inputs(sf: &source::SFunc, hints: &[String]) -> Vec<Vec<SymVal>> {
    let mut ints: Vec<String> = hints.iter().filter_map(|h| value::numeral(h)).collect();
    ints.extend(GRID.iter().map(i64::to_string));
    let mut seen = BTreeSet::new();
    ints.retain(|n| seen.insert(n.clone()));
    let choices: Vec<Vec<SymVal>> = sf
        .params
        .iter()
        .map(|(_, k)| match k {
            Kind::Bool => vec![value::Lit::Bool(false).value(), value::Lit::Bool(true).value()],
            _ => ints.iter().filter_map(|n| n.parse::<z3::ast::Int>().ok().map(SymVal::Int)).collect(),
        })
        .collect();
    tuples(&choices, REPLAYS)
}

/// Concrete runs of one function on chosen inputs, through both executors.
struct Replay<'a, 'c> {
    sp: &'a SProgram,
    bp: &'a BProgram<'c>,
    calls: &'a Calls,
}

impl Replay<'_, '_> {
    /// The first input on which the source and the bytecode, run concretely, do different things: the input, and
    /// what each did.
    fn search(&self, sf: &source::SFunc, bf: &machine::BFunc, hints: &[String]) -> Option<(String, String, String)> {
        let mut spent = 0;
        for args in replay_inputs(sf, hints) {
            if spent >= REPLAY_TOTAL {
                break;
            }
            let budget = Budget::new(REPLAY_STEPS);
            let src = SourceRun { program: self.sp, calls: self.calls, mode: Mode::Concrete, budget: &budget };
            let source = src
                .explore(sf, 0, State { env: sf.params.iter().map(|(v, _)| *v).zip(args.iter().cloned()).collect(), depth: 0 })
                .ok()
                .and_then(|paths| outcome(paths));
            spent += REPLAY_STEPS - budget.left();
            let budget = Budget::new(REPLAY_STEPS);
            let mac = MachineRun { program: self.bp, calls: self.calls, mode: Mode::Concrete, budget: &budget };
            let bytecode = mac.explore(bf, bf.entry, mac.entry_state(bf, &args)).ok().and_then(|paths| outcome(paths));
            spent += REPLAY_STEPS - budget.left();
            if let (Some(s), Some(b)) = (source, bytecode) {
                if s != b {
                    let input = sf
                        .params
                        .iter()
                        .zip(&args)
                        .map(|((v, _), a)| format!("{} = {}", sf.names[*v as usize], a.shown().unwrap_or_default()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Some((input, s, b));
                }
            }
        }
        None
    }
}

/// Up to `limit` argument tuples from per-parameter choices, the first choice of each first, in breadth: every tuple
/// over the first `k` choices of each parameter before any using the `k+1`-th.
fn tuples(choices: &[Vec<SymVal>], limit: usize) -> Vec<Vec<SymVal>> {
    if choices.is_empty() {
        return vec![Vec::new()];
    }
    let widest = choices.iter().map(Vec::len).max().unwrap_or(0);
    let mut out: Vec<Vec<SymVal>> = Vec::new();
    let mut emitted = BTreeSet::new();
    for k in 1..=widest {
        let mut idx = vec![0usize; choices.len()];
        loop {
            if idx.iter().zip(choices).all(|(i, c)| *i < c.len()) && emitted.insert(idx.clone()) {
                out.push(idx.iter().zip(choices).map(|(i, c)| c[*i].clone()).collect());
                if out.len() >= limit {
                    return out;
                }
            }
            let mut d = 0;
            loop {
                if d == idx.len() {
                    break;
                }
                idx[d] += 1;
                if idx[d] < k {
                    break;
                }
                idx[d] = 0;
                d += 1;
            }
            if d == idx.len() {
                break;
            }
        }
    }
    out
}

/// A concrete run's outcome, as text two runs are compared by: how it ended, its value, what it showed.
pub(crate) fn outcome<L: Ord>(paths: Vec<path::Path<L>>) -> Option<String> {
    let mut live = paths.into_iter().filter(|p| !path::dead(&p.cond));
    let (Some(p), None) = (live.next(), live.next()) else {
        return None;
    };
    let shown: Vec<String> = p.out.iter().map(|v| v.shown().unwrap_or_else(|| format!("{:?}", v.kind()))).collect();
    let end = match &p.end {
        End::Return(SymVal::Nothing) => "returns nothing".to_string(),
        End::Return(v) => format!("returns {}", v.shown()?),
        End::Raise => "raises".into(),
        End::Overflow => "stops with an integer overflow".into(),
        End::Halt => "finishes".into(),
        End::Cut(_) => return None,
    };
    Some(if shown.is_empty() { end } else { format!("shows {} and {end}", shown.join(", ")) })
}

/// The lines a concrete run showed, and whether it raised.
fn concrete(paths: Vec<path::Path<impl Ord>>) -> Result<(Vec<String>, bool), String> {
    let mut live = paths.into_iter().filter(|p| !path::dead(&p.cond));
    let (Some(p), None) = (live.next(), live.next()) else {
        return Err("a concrete run did not take exactly one path".into());
    };
    let lines = p
        .out
        .iter()
        .map(|v| v.shown().ok_or_else(|| format!("a shown {:?} is not a constant", v.kind())))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((lines, matches!(p.end, End::Raise)))
}

/// Run both executors concretely on `## Main` and hold each to its engine: the source executor to the tree-walker,
/// the bytecode executor to the VM running the same compiled program.
pub fn check_executors(source: &str, target: Target) -> ExecutorCheck {
    const STEPS: u64 = 2_000_000;
    let sp = match lower_source(source) {
        Ok(sp) => sp,
        Err(e) => return ExecutorCheck::Unsupported(e),
    };
    let (program, names) = match compile(source, target) {
        Ok(c) => c,
        Err(e) => return ExecutorCheck::Unsupported(e),
    };
    let bp: BProgram = analyze_program(&program, &names);
    let calls = call_policy(&sp);

    let src_budget = Budget::new(STEPS);
    let src = SourceRun { program: &sp, calls: &calls, mode: Mode::Concrete, budget: &src_budget };
    let mac_budget = Budget::new(STEPS);
    let mac = MachineRun { program: &bp, calls: &calls, mode: Mode::Concrete, budget: &mac_budget };

    let source_run = match &sp.main {
        Ok(main) => src.explore(main, 0, State { env: BTreeMap::new(), depth: 0 }),
        Err(why) => return ExecutorCheck::Unsupported(why.0.clone()),
    };
    let machine_run = match &bp.main {
        Ok(main) => mac.explore(main, main.entry, mac.entry_state(main, &[])),
        Err(why) => return ExecutorCheck::Unsupported(why.0.clone()),
    };
    let (source_run, machine_run) = match (source_run, machine_run) {
        (Ok(s), Ok(m)) => (s, m),
        (Err(why), _) | (_, Err(why)) => return ExecutorCheck::Unsupported(why.0),
    };
    let (src_lines, src_raised) = match concrete(source_run) {
        Ok(r) => r,
        Err(e) => return ExecutorCheck::Unsupported(e),
    };
    let (mac_lines, mac_raised) = match concrete(machine_run) {
        Ok(r) => r,
        Err(e) => return ExecutorCheck::Unsupported(e),
    };

    let (walker_lines, walker_raised) = match logicaffeine_compile::compile::interpret_program(source) {
        Ok(out) => (lines_of(&out), false),
        Err(_) => (None, true),
    };
    if src_raised != walker_raised || (!walker_raised && walker_lines.as_deref() != Some(&src_lines[..])) {
        return ExecutorCheck::Disagree(format!(
            "the source executor shows {src_lines:?} (raised: {src_raised}); the tree-walker {walker_lines:?} (raised: {walker_raised})"
        ));
    }

    let mut vm = Vm::new(&program);
    let vm_raised = vm.run().is_err();
    let vm_lines = vm.lines().to_vec();
    if mac_raised != vm_raised || mac_lines != vm_lines {
        return ExecutorCheck::Disagree(format!(
            "the bytecode executor shows {mac_lines:?} (raised: {mac_raised}); the VM {vm_lines:?} (raised: {vm_raised})"
        ));
    }
    ExecutorCheck::Agree
}

fn lines_of(out: &str) -> Option<Vec<String>> {
    Some(if out.is_empty() { Vec::new() } else { out.split('\n').map(str::to_string).collect() })
}
