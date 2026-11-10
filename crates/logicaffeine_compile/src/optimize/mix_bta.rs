//! The binding-time analysis the offline specializer (`mix.logos`) reads.
//!
//! It turns a program's Core IR ([`crate::compile::CoreNode`]) into the two-level program `mix` specializes, as LOGOS
//! functions that build `mixFuncs` (every function as Core IR, which `core_interp` runs to evaluate static parts),
//! `mixBFuncs` (the two-level functions `mix` unfolds or specializes) and `mixMain` (the two-level main block).
//!
//! Main's free variables are its inputs, each static or dynamic. Binding times are flow-sensitive per variable and
//! joined where control merges; a function is analysed once per binding-time pattern of its arguments (a variant). A
//! static computation runs while specializing (`BSBlock`); an effect, or an operation on a dynamic value, is residual
//! with its static operands lifted. A call to an effectful function, or one with a dynamic argument, is let-bound
//! ahead of its use, left to right: unfolded when its arguments are static (`BSCallLet`/`BSCallS`), its effects
//! reaching the residual and its result the caller; specialized on its static arguments otherwise
//! (`BDCallLet`/`BDCallS`). A branch, loop or `Inspect` on a static value is decided while specializing. One on a
//! dynamic value is a dynamic region, residual: the specializer leaves it with the store it entered with, so static
//! work inside it writes only variables dead at its exit, and the variables it writes that are live there move to
//! the residual ahead of it. A `Return` inside one is residual, even of a static value, and so is a `Break` with one
//! between it and its loop, which makes that loop residual too. Where static control merges, a variable one path
//! leaves static and the merge holds dynamic is lifted at the end of that path.
//!
//! A specialization never reads the run. When the program has a dynamic input, a read of the run (the clock, today's
//! date, a builtin whose answer is not a function of its arguments, `args()`: what `compile::run_time_input`
//! names) is dynamic and an effect: the residual reads it when it runs. With no dynamic input the whole run happens while
//! specializing, and the projections refuse a program whose run would reach a read before they specialize it.

use std::collections::{HashMap, HashSet};

use crate::compile::CoreNode;

/// Constructor → its fields `(name, type)`, from a catalog written as `## A T is one of:` / `    A C with f T and g U.`.
pub fn catalog_shapes(catalog: &str) -> HashMap<String, Vec<(String, String)>> {
    let mut shapes = HashMap::new();
    let mut in_enum = false;
    for line in catalog.lines() {
        if line.starts_with("## A ") || line.starts_with("## An ") {
            in_enum = line.trim_end().ends_with("is one of:");
            continue;
        }
        if line.starts_with("## ") || line.trim().is_empty() {
            in_enum = in_enum && line.trim().is_empty();
            continue;
        }
        if !in_enum {
            continue;
        }
        let Some(rest) = line.strip_prefix("    A ").or_else(|| line.strip_prefix("    An ")) else {
            continue;
        };
        let rest = rest.trim_end().trim_end_matches('.');
        let (ctor, fields_text) = match rest.split_once(" with ") {
            Some((c, f)) => (c, Some(f)),
            None => (rest, None),
        };
        let mut fields: Vec<(String, String)> = Vec::new();
        if let Some(text) = fields_text {
            for part in text.split(" and ") {
                let starts_field = part.chars().next().is_some_and(|c| c.is_ascii_lowercase());
                match (starts_field, fields.last_mut()) {
                    (true, _) | (false, None) => {
                        let (f, t) = part.split_once(' ').unwrap_or((part, ""));
                        fields.push((f.to_string(), t.to_string()));
                    }
                    (false, Some(last)) => {
                        last.1 = format!("{} and {}", last.1, part);
                    }
                }
            }
        }
        shapes.insert(ctor.to_string(), fields);
    }
    shapes
}

fn ind<'a>(node: &'a CoreNode) -> Option<(&'a str, &'a [CoreNode])> {
    match node {
        CoreNode::Ind(c, a) => Some((c.as_str(), a.as_slice())),
        _ => None,
    }
}

fn text(node: &CoreNode) -> Option<&str> {
    match node {
        CoreNode::Text(s) => Some(s.as_str()),
        _ => None,
    }
}

fn items(node: &CoreNode) -> &[CoreNode] {
    match node {
        CoreNode::List(v) => v.as_slice(),
        _ => &[],
    }
}

const EFFECT_STMTS: &[&str] = &[
    "CShow",
    "CReadConsole",
    "CReadFile",
    "CWriteFile",
    "CSleep",
    "CSendPipe",
    "CReceivePipe",
    "CTrySendPipe",
    "CTryReceivePipe",
    "CSendMessage",
    "CStreamMessage",
    "CAwaitMessage",
    "CListen",
    "CConnectTo",
    "CSpawn",
    "CLaunchTask",
    "CStopTask",
    "CEscStmt",
    "CFail",
];

/// Whether `node` is or holds an effect: one [`names_an_effect`] names, or, when the program's reads of the run are
/// dynamic (`run_reads` holds its own functions' names), a read of the run.
fn mentions_effect(node: &CoreNode, effectful: &HashSet<String>, run_reads: Option<&HashSet<String>>) -> bool {
    names_an_effect(node, effectful)
        || run_reads.is_some_and(|functions| crate::compile::run_time_input(node, functions, false).is_some())
}

/// Whether `node` names a call to a function in `effectful`, or is an effect statement, anywhere inside it.
fn names_an_effect(node: &CoreNode, effectful: &HashSet<String>) -> bool {
    match node {
        CoreNode::Ind(c, args) => {
            if EFFECT_STMTS.contains(&c.as_str()) {
                return true;
            }
            if (c == "CCall" || c == "CCallS") && args.first().and_then(text).is_some_and(|f| effectful.contains(f)) {
                return true;
            }
            args.iter().any(|a| names_an_effect(a, effectful))
        }
        CoreNode::List(v) => v.iter().any(|a| names_an_effect(a, effectful)),
        _ => false,
    }
}

/// Whether an annotated loop body holds a residual `Break` of its own loop: one outside every loop nested in it.
fn breaks_in_residual(node: &CoreNode) -> bool {
    match node {
        CoreNode::Ind(c, args) => match c.as_str() {
            "BDStmt" => matches!(args.first().and_then(ind), Some(("CBreak", _))),
            "BSWhile" | "BDWhile" | "BSRepeat" | "BDRepeat" | "BSRepeatRange" | "BDRepeatRange" => false,
            _ => args.iter().any(breaks_in_residual),
        },
        CoreNode::List(items) => items.iter().any(breaks_in_residual),
        _ => false,
    }
}

/// The functions whose bodies show anything, read the run where `run_reads` makes that an effect, or call such a
/// function.
fn effectful_functions(funcs: &[CoreNode], run_reads: Option<&HashSet<String>>) -> HashSet<String> {
    let mut effectful: HashSet<String> = HashSet::new();
    loop {
        let before = effectful.len();
        for f in funcs {
            if let Some(("CFuncDef", args)) = ind(f) {
                let name = args.first().and_then(text).unwrap_or("").to_string();
                if !effectful.contains(&name) && args.get(4).is_some_and(|b| mentions_effect(b, &effectful, run_reads)) {
                    effectful.insert(name);
                }
            }
        }
        if effectful.len() == before {
            return effectful;
        }
    }
}

fn node(ctor: &str, args: Vec<CoreNode>) -> CoreNode {
    CoreNode::Ind(ctor.to_string(), args)
}

/// A binding time: known while specializing (`S`) or only when the residual runs (`D`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Bt {
    S,
    D,
}

impl Bt {
    fn join(self, other: Bt) -> Bt {
        if self == Bt::D || other == Bt::D {
            Bt::D
        } else {
            Bt::S
        }
    }
}

type Env = HashMap<String, Bt>;

fn join_env(a: &Env, b: &Env) -> Env {
    let mut out = a.clone();
    for (k, v) in b {
        let joined = out.get(k).copied().unwrap_or(*v).join(*v);
        out.insert(k.clone(), joined);
    }
    out
}

/// The control a statement runs under: static, or inside a dynamic region — a residual branch, loop or `Inspect` —
/// with the variables live at its exit (`region`). The specializer leaves a region with the store it entered with, so
/// static work inside one may write only variables dead at its exit. `residual_break` tells whether a region stands
/// between the statement and the innermost loop it is in: a `Break` there leaves its loop when the run reaches it, so
/// the residual carries it; under static control relative to its loop, a `Break` ends the unrolling while specializing.
struct Ctl {
    region: Option<HashSet<String>>,
    residual_break: bool,
}

impl Ctl {
    /// A function's body or the main block: static, in no loop.
    fn top() -> Ctl {
        Ctl { region: None, residual_break: false }
    }

    fn is_static(&self) -> bool {
        self.region.is_none()
    }

    /// The control in the body of a loop the specializer unrolls: the loop's region, and a `Break` under static
    /// control relative to the loop.
    fn unrolled_body(&self) -> Ctl {
        Ctl { region: self.region.clone(), residual_break: false }
    }

    fn allows(&self, writes: &HashSet<String>) -> bool {
        match &self.region {
            None => true,
            Some(live) => writes.is_disjoint(live),
        }
    }

    fn allows_var(&self, v: &str) -> bool {
        match &self.region {
            None => true,
            Some(live) => !live.contains(v),
        }
    }
}

/// The variables live before `stmts` when `after` is live after them. Conservative: only a `Let` or `Set` of the
/// block itself ends a variable's life, never one inside a branch or a loop.
fn live_before(stmts: &[CoreNode], after: &HashSet<String>) -> HashSet<String> {
    stmts.iter().rev().fold(after.clone(), |live, s| live_before_stmt(s, &live))
}

/// The variables live before `s` when `after` is live after it.
fn live_before_stmt(s: &CoreNode, after: &HashSet<String>) -> HashSet<String> {
    let reads_of = |n: &CoreNode| {
        let mut r = HashSet::new();
        reads(n, &mut r);
        r
    };
    let Some((c, a)) = ind(s) else { return after.clone() };
    match c {
        "CLet" | "CSet" => {
            let mut live = after.clone();
            if let Some(n) = a.first().and_then(text) {
                live.remove(n);
            }
            live.extend(reads_of(&a[1]));
            live
        }
        "CIf" => {
            let mut live = reads_of(&a[0]);
            live.extend(live_before(items(&a[1]), after));
            live.extend(live_before(items(&a[2]), after));
            live
        }
        "CWhile" => loop_head(after, &reads_of(&a[0]), items(&a[1]), None),
        "CRepeat" => {
            let mut live = loop_head(after, &HashSet::new(), items(&a[2]), a.first().and_then(text));
            live.extend(reads_of(&a[1]));
            live
        }
        "CRepeatRange" => {
            let mut live = loop_head(after, &HashSet::new(), items(&a[3]), a.first().and_then(text));
            live.extend(reads_of(&a[1]));
            live.extend(reads_of(&a[2]));
            live
        }
        "CInspect" => {
            let mut live = reads_of(&a[0]);
            let mut exhaustive = false;
            for arm in items(&a[1]) {
                match ind(arm) {
                    Some(("CWhen", w)) => {
                        let mut arm_live = live_before(items(&w[2]), after);
                        for b in items(&w[1]).iter().filter_map(text) {
                            arm_live.remove(b);
                        }
                        live.extend(arm_live);
                    }
                    Some(("COtherwise", o)) => {
                        exhaustive = true;
                        live.extend(live_before(items(&o[0]), after));
                    }
                    _ => {}
                }
            }
            if !exhaustive {
                live.extend(after.iter().cloned());
            }
            live
        }
        _ => {
            let mut live = after.clone();
            reads(s, &mut live);
            writes(s, &mut live);
            live
        }
    }
}

/// The variables live at the head of a loop whose condition reads `cond`, whose body binds `var` afresh each
/// iteration (a `Repeat`'s), and after which `after` is live.
fn loop_head(after: &HashSet<String>, cond: &HashSet<String>, body: &[CoreNode], var: Option<&str>) -> HashSet<String> {
    let exit: HashSet<String> = after.union(cond).cloned().collect();
    let mut head = exit.clone();
    loop {
        let mut next = live_before(body, &head);
        if let Some(v) = var {
            next.remove(v);
        }
        next.extend(exit.iter().cloned());
        if next == head {
            return head;
        }
        head = next;
    }
}

/// Whether `node` holds a `Return` from the function it is in: in itself or a nested block, never in a closure's body.
fn returns_within(node: &CoreNode) -> bool {
    match ind(node) {
        Some(("CReturn", _)) => true,
        Some(("CClosure", _)) => false,
        Some((_, args)) => args.iter().any(returns_within),
        None => items(node).iter().any(returns_within),
    }
}

/// Whether `node` holds a `Break` that leaves it: in itself or a nested block, never in a loop, closure or task body
/// nested in it.
fn breaks_within(node: &CoreNode) -> bool {
    match ind(node) {
        Some(("CBreak", _)) => true,
        Some(("CWhile" | "CRepeat" | "CRepeatRange" | "CClosure" | "CLaunchTask", _)) => false,
        Some((_, args)) => args.iter().any(breaks_within),
        None => items(node).iter().any(breaks_within),
    }
}

/// The empty construction `empties` holds for `name`, or `COptionNone` when its type is not known.
fn empty_of(empties: &HashMap<String, CoreNode>, name: &str) -> CoreNode {
    empties.get(name).cloned().unwrap_or_else(|| node("COptionNone", vec![]))
}

/// The empty construction of a collection of the LOGOS type `ty` (`Seq of Int`, `Set of Text`, `Map of Text to Int`,
/// with or without `mutable`), or `None` when `ty` is no collection type.
fn empty_construction(ty: &str) -> Option<CoreNode> {
    let ty = ty.strip_prefix("mutable ").unwrap_or(ty).trim();
    let unparen = |t: &str| t.strip_prefix('(').and_then(|t| t.strip_suffix(')')).unwrap_or(t).to_string();
    if let Some(e) = ty.strip_prefix("Seq of ") {
        return Some(node("CNewSeq", vec![CoreNode::Text(unparen(e))]));
    }
    if let Some(e) = ty.strip_prefix("Set of ") {
        return Some(node("CNewSet", vec![CoreNode::Text(unparen(e))]));
    }
    if ty.starts_with("Map of ") {
        return Some(node("CNew", vec![CoreNode::Text(ty.to_string()), CoreNode::List(vec![]), CoreNode::List(vec![])]));
    }
    None
}

/// Whether `e` is the construction of an empty collection whose type it names.
fn is_typed_empty(e: &CoreNode) -> bool {
    match ind(e) {
        Some(("CNewSeq", _)) | Some(("CNewSet", _)) => true,
        Some(("CNew", a)) => {
            a.first().and_then(text).is_some_and(|t| t.starts_with("Map of "))
                && a.get(1).is_some_and(|n| items(n).is_empty())
        }
        _ => false,
    }
}

/// `block` followed by a lift of every variable it leaves static that `merged` — the binding times where control
/// merges after it — holds dynamic: the code after the merge reads it from the residual.
fn lift_at_merge(block: CoreNode, end: &Env, merged: &Env, empties: &HashMap<String, CoreNode>) -> CoreNode {
    let mut names: Vec<&String> =
        end.iter().filter(|(v, b)| **b == Bt::S && merged.get(*v) == Some(&Bt::D)).map(|(v, _)| v).collect();
    if names.is_empty() {
        return block;
    }
    names.sort();
    let mut stmts = items(&block).to_vec();
    stmts.extend(names.into_iter().map(|v| node("BDLift", vec![CoreNode::Text(v.clone()), empty_of(empties, v)])));
    CoreNode::List(stmts)
}

/// The variables a static loop entered with `entry` holds dynamic at its head (`head`, its fixpoint) though they were
/// static on entry: the loop's body reads them from the residual from its first pass on, so they are lifted ahead of it.
fn turned_dynamic(entry: &Env, head: &Env) -> HashSet<String> {
    entry
        .iter()
        .filter(|(v, b)| **b == Bt::S && head.get(*v) == Some(&Bt::D))
        .map(|(v, _)| v.clone())
        .collect()
}

/// The binding times after a dynamic region that writes `w`, with `live` live at its exit, given `env` as it entered:
/// a variable the region writes that is live there is dynamic; every other keeps its binding time from before.
fn leave_region(env: &mut Env, w: &HashSet<String>, live: &HashSet<String>) {
    for v in w.intersection(live) {
        env.insert(v.clone(), Bt::D);
    }
}

/// How a value a function holds relates to its parameters, for the finiteness of specialization: `Strict` a part of
/// a parameter's value (an element, a field, a binding of an `Inspect` of it), `Param` a parameter's value itself or a
/// constant, `Unbounded` anything computed. Ordered so the least is the worst.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Reach {
    Unbounded,
    Param,
    Strict,
}

/// The reach of `e` given the reach of every variable ([`reaches`]).
/// The reach of `e` at the point it is evaluated, from the reach [`reach_uses`] recorded for each variable use in it.
fn reach_at(e: &CoreNode, uses: &HashMap<usize, Reach>) -> Reach {
    match ind(e) {
        Some(("CVar", _)) => uses.get(&(e as *const CoreNode as usize)).copied().unwrap_or(Reach::Unbounded),
        Some(("CInt" | "CBool" | "CText" | "CFloat" | "CDuration" | "CDate" | "CMoment" | "CSpan" | "CTime" | "CChar", _)) => {
            Reach::Param
        }
        Some(("CIndex" | "CFieldAccess" | "CMapGet", a)) => match a.first().map(|b| reach_at(b, uses)) {
            Some(Reach::Param | Reach::Strict) => Reach::Strict,
            _ => Reach::Unbounded,
        },
        _ => Reach::Unbounded,
    }
}

/// Records, in `uses` (keyed by the `CVar` node), the reach of every variable use in the body of a function with
/// parameters `params`: the reach of the binding in scope where it is used. A parameter reaches its own value; a `Let`
/// or `Set`, a `Repeat` binder or an `Inspect` binding reaches what it is taken from; a variable written in place, or
/// written back by a call, is unbounded. Branches and arms meet where they merge, loop heads by fixpoint.
fn reach_uses(
    params: &[String],
    body: &[CoreNode],
    funcs: &HashMap<String, Func>,
    shapes: &HashMap<String, Vec<(String, String)>>,
    uses: &mut HashMap<usize, Reach>,
) {
    let mut env: HashMap<String, Reach> = params.iter().map(|p| (p.clone(), Reach::Param)).collect();
    ReachWalk { funcs, shapes, uses }.block(body, &mut env);
}

struct ReachWalk<'a> {
    funcs: &'a HashMap<String, Func>,
    shapes: &'a HashMap<String, Vec<(String, String)>>,
    uses: &'a mut HashMap<usize, Reach>,
}

impl ReachWalk<'_> {
    /// Record the uses in an expression (closures' bodies are not walked: a closure's calls are opaque).
    fn expr(&mut self, e: &CoreNode, env: &HashMap<String, Reach>) {
        match e {
            CoreNode::Ind(c, a) => {
                if c == "CVar" {
                    if let Some(x) = a.first().and_then(text) {
                        self.uses.insert(e as *const CoreNode as usize, env.get(x).copied().unwrap_or(Reach::Unbounded));
                    }
                    return;
                }
                if c == "CClosure" {
                    return;
                }
                a.iter().for_each(|x| self.expr(x, env));
            }
            CoreNode::List(v) => v.iter().for_each(|x| self.expr(x, env)),
            _ => {}
        }
    }

    fn meet(into: &mut HashMap<String, Reach>, other: &HashMap<String, Reach>) {
        for (k, v) in into.iter_mut() {
            if let Some(o) = other.get(k) {
                *v = (*v).min(*o);
            }
        }
    }

    fn block(&mut self, stmts: &[CoreNode], env: &mut HashMap<String, Reach>) {
        for s in stmts {
            self.stmt(s, env);
        }
    }

    /// `body` as a loop body entered with `env`, binding `var` (if any) to `var_reach` each iteration: walked until the
    /// head is stable, the uses recorded by the last pass; `env` leaves as the head's.
    fn looped(&mut self, body: &[CoreNode], env: &mut HashMap<String, Reach>, var: Option<(&str, Reach)>, cond: Option<&CoreNode>) {
        loop {
            if let Some(c) = cond {
                self.expr(c, env);
            }
            let mut inner = env.clone();
            if let Some((v, r)) = var {
                inner.insert(v.to_string(), r);
            }
            self.block(body, &mut inner);
            let mut head = env.clone();
            Self::meet(&mut head, &inner);
            if head == *env {
                return;
            }
            *env = head;
        }
    }

    fn stmt(&mut self, s: &CoreNode, env: &mut HashMap<String, Reach>) {
        let Some((c, a)) = ind(s) else { return };
        let bound = |r: Reach| if r == Reach::Unbounded { Reach::Unbounded } else { Reach::Strict };
        match c {
            "CLet" | "CSet" => {
                self.expr(&a[1], env);
                if let Some(x) = a.first().and_then(text) {
                    let r = reach_at(&a[1], self.uses);
                    env.insert(x.to_string(), r);
                }
            }
            "CIf" => {
                self.expr(&a[0], env);
                let mut t = env.clone();
                self.block(items(&a[1]), &mut t);
                let mut e = env.clone();
                self.block(items(&a[2]), &mut e);
                Self::meet(&mut t, &e);
                Self::meet(env, &t);
            }
            "CWhile" => self.looped(items(&a[1]), env, None, Some(&a[0])),
            "CRepeat" => {
                self.expr(&a[1], env);
                let r = bound(reach_at(&a[1], self.uses));
                let var = a.first().and_then(text).unwrap_or("");
                self.looped(items(&a[2]), env, Some((var, r)), None);
            }
            "CRepeatRange" => {
                self.expr(&a[1], env);
                self.expr(&a[2], env);
                let var = a.first().and_then(text).unwrap_or("");
                self.looped(items(&a[3]), env, Some((var, Reach::Unbounded)), None);
            }
            "CInspect" => {
                self.expr(&a[0], env);
                let r = bound(reach_at(&a[0], self.uses));
                let mut merged = env.clone();
                for arm in items(&a[1]) {
                    let mut arm_env = env.clone();
                    let body = match ind(arm) {
                        Some(("CWhen", w)) => {
                            for b in items(&w[1]).iter().filter_map(text) {
                                arm_env.insert(b.to_string(), r);
                            }
                            items(&w[2])
                        }
                        Some(("COtherwise", o)) => items(&o[0]),
                        _ => continue,
                    };
                    self.block(body, &mut arm_env);
                    Self::meet(&mut merged, &arm_env);
                }
                *env = merged;
            }
            _ => {
                let fields = self.shapes.get(c).cloned().unwrap_or_default();
                for (i, x) in a.iter().enumerate() {
                    if fields.get(i).is_some_and(|(_, t)| t.contains("CStmt")) {
                        let mut inner = env.clone();
                        self.block(items(x), &mut inner);
                        Self::meet(env, &inner);
                    } else {
                        self.expr(x, env);
                    }
                }
                let mut w = HashSet::new();
                writes(s, &mut w);
                let mut cs = Vec::new();
                calls(s, false, &mut cs);
                for (f, args, _) in cs {
                    if let Some(g) = self.funcs.get(f) {
                        for (t, arg) in g.types.iter().zip(args) {
                            if let (true, Some(("CVar", va))) = (t.starts_with("mutable "), ind(arg)) {
                                if let Some(x) = va.first().and_then(text) {
                                    w.insert(x.to_string());
                                }
                            }
                        }
                    }
                }
                for x in w {
                    env.insert(x, Reach::Unbounded);
                }
            }
        }
    }
}

/// Every function that lies on a cycle of the call graph, with the strongly connected component it belongs to.
fn recursive_components(funcs: &HashMap<String, Func>) -> HashMap<String, usize> {
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for (n, f) in funcs {
        let mut c = Vec::new();
        f.body.iter().for_each(|s| calls(s, false, &mut c));
        let mut out: Vec<&str> = c.into_iter().map(|(g, _, _)| g).filter(|g| funcs.contains_key(*g)).collect();
        out.sort_unstable();
        out.dedup();
        edges.insert(n.as_str(), out);
    }
    struct Tarjan<'a> {
        edges: &'a HashMap<&'a str, Vec<&'a str>>,
        index: HashMap<&'a str, usize>,
        low: HashMap<&'a str, usize>,
        stack: Vec<&'a str>,
        on_stack: HashSet<&'a str>,
        next: usize,
        components: Vec<Vec<&'a str>>,
    }
    impl<'a> Tarjan<'a> {
        fn visit(&mut self, v: &'a str) {
            self.index.insert(v, self.next);
            self.low.insert(v, self.next);
            self.next += 1;
            self.stack.push(v);
            self.on_stack.insert(v);
            for &w in self.edges.get(v).map(Vec::as_slice).unwrap_or(&[]) {
                if !self.index.contains_key(w) {
                    self.visit(w);
                    let lw = self.low[w];
                    let lv = self.low.get_mut(v).expect("visited");
                    *lv = (*lv).min(lw);
                } else if self.on_stack.contains(w) {
                    let iw = self.index[w];
                    let lv = self.low.get_mut(v).expect("visited");
                    *lv = (*lv).min(iw);
                }
            }
            if self.low[v] == self.index[v] {
                let mut component = Vec::new();
                while let Some(w) = self.stack.pop() {
                    self.on_stack.remove(w);
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                self.components.push(component);
            }
        }
    }
    let mut names: Vec<&str> = edges.keys().copied().collect();
    names.sort_unstable();
    let mut t = Tarjan {
        edges: &edges,
        index: HashMap::new(),
        low: HashMap::new(),
        stack: Vec::new(),
        on_stack: HashSet::new(),
        next: 0,
        components: Vec::new(),
    };
    for v in names {
        if !t.index.contains_key(v) {
            t.visit(v);
        }
    }
    let mut out = HashMap::new();
    for (i, component) in t.components.iter().enumerate() {
        let cyclic = component.len() > 1 || edges[component[0]].contains(&component[0]);
        if cyclic {
            for f in component {
                out.insert(f.to_string(), i);
            }
        }
    }
    out
}

struct Func {
    params: Vec<String>,
    types: Vec<String>,
    ret: String,
    body: std::rc::Rc<Vec<CoreNode>>,
}

/// A function analysed at one binding-time pattern of its parameters.
struct Variant {
    name: String,
    ret: Bt,
    node: Option<CoreNode>,
}

struct Annotator<'a> {
    funcs: HashMap<String, Func>,
    effectful: HashSet<String>,
    /// The program's own functions' names when it has a dynamic input: its reads of the run are then dynamic.
    run_reads: Option<HashSet<String>>,
    /// The empty construction of each collection variable of the variant being annotated, by name: what it was
    /// initialized to, or what its declared parameter type builds. A lift of the variable while it is empty prints it,
    /// its element types and all.
    empties: HashMap<String, CoreNode>,
    shapes: &'a HashMap<String, Vec<(String, String)>>,
    variants: HashMap<(String, Vec<Bt>), Variant>,
    queue: Vec<(String, Vec<Bt>)>,
    fresh: usize,
    /// The binding time of the variant being annotated's returns.
    ret: Bt,
    /// The variant being annotated's dynamic `mutable` parameters: the caller reads them back from the residual.
    pinned: HashSet<String>,
    /// The function whose variant is being annotated; `None` for main.
    current: Option<String>,
    /// [`reach_uses`] over every function's body.
    reach: HashMap<usize, Reach>,
    /// [`recursive_components`].
    cycles: HashMap<String, usize>,
}

/// Every variable `node` reads.
fn reads(node: &CoreNode, out: &mut HashSet<String>) {
    match node {
        CoreNode::Ind(c, args) => {
            if c == "CVar" {
                if let Some(n) = args.first().and_then(text) {
                    out.insert(n.to_string());
                }
                return;
            }
            if c == "CClosure" {
                for n in items(&args[2]) {
                    if let Some(n) = text(n) {
                        out.insert(n.to_string());
                    }
                }
                return;
            }
            for a in args {
                reads(a, out);
            }
        }
        CoreNode::List(v) => v.iter().for_each(|a| reads(a, out)),
        _ => {}
    }
}

const WRITE_TARGETS: &[(&str, usize)] = &[
    ("CLet", 0),
    ("CSet", 0),
    ("CPush", 1),
    ("CSetIdx", 0),
    ("CMapSet", 0),
    ("CSetField", 0),
    ("CPop", 0),
    ("CAdd", 1),
    ("CRemove", 1),
    ("CRepeat", 0),
    ("CRepeatRange", 0),
];

/// Every variable a block assigns, at any depth, and the variables an `Inspect` arm binds.
fn writes(node: &CoreNode, out: &mut HashSet<String>) {
    match node {
        CoreNode::Ind(c, args) => {
            if let Some((_, i)) = WRITE_TARGETS.iter().find(|(w, _)| w == c) {
                if let Some(n) = args.get(*i).and_then(text) {
                    out.insert(n.to_string());
                }
            }
            if c == "CWhen" {
                for n in items(&args[1]) {
                    if let Some(n) = text(n) {
                        out.insert(n.to_string());
                    }
                }
            }
            for a in args {
                writes(a, out);
            }
        }
        CoreNode::List(v) => v.iter().for_each(|a| writes(a, out)),
        _ => {}
    }
}

impl Annotator<'_> {
    fn temp(&mut self) -> String {
        self.fresh += 1;
        format!("mixTemp{}", self.fresh)
    }

    fn bt_of(&self, e: &CoreNode, env: &Env) -> Bt {
        if self
            .run_reads
            .as_ref()
            .is_some_and(|functions| crate::compile::run_time_input(e, functions, false).is_some())
        {
            return Bt::D;
        }
        let mut r = HashSet::new();
        reads(e, &mut r);
        if r.iter().all(|v| env.get(v) == Some(&Bt::S)) {
            Bt::S
        } else {
            Bt::D
        }
    }

    fn statics(&self, e: &CoreNode, env: &Env) -> CoreNode {
        let mut r = HashSet::new();
        reads(e, &mut r);
        let mut names: Vec<String> = r.into_iter().filter(|v| env.get(v) == Some(&Bt::S)).collect();
        names.sort();
        CoreNode::List(names.into_iter().map(CoreNode::Text).collect())
    }

    fn bexpr(&self, e: &CoreNode, env: &Env) -> CoreNode {
        match self.bt_of(e, env) {
            Bt::S => node("BStatic", vec![e.clone()]),
            Bt::D => node("BDExpr", vec![e.clone(), self.statics(e, env)]),
        }
    }

    /// The name of `f` analysed at `pattern`, queued for analysis the first time it is asked for.
    fn variant(&mut self, f: &str, pattern: Vec<Bt>) -> (String, Bt) {
        let key = (f.to_string(), pattern.clone());
        if let Some(v) = self.variants.get(&key) {
            return (v.name.clone(), v.ret);
        }
        let name = if pattern.iter().all(|b| *b == Bt::S) {
            f.to_string()
        } else {
            let letters: String = pattern.iter().map(|b| if *b == Bt::S { 'S' } else { 'D' }).collect();
            format!("{f}_b{letters}")
        };
        self.variants.insert(key.clone(), Variant { name: name.clone(), ret: Bt::S, node: None });
        self.queue.push(key);
        (name, Bt::S)
    }

    /// The binding-time pattern of a residual call of `f` with `args` (the call's own argument nodes, as the body
    /// holds them), whose binding times are `pattern`, with
    /// every static argument generalized that could take unboundedly many values: at a call within the caller's own
    /// recursion that hands no static parameter a strict part of the caller's, an argument computed rather than taken
    /// from the caller's parameters would ask for a new specialization at every level of a recursion the residual's
    /// data controls.
    fn generalized(&self, f: &str, args: &[CoreNode], mut pattern: Vec<Bt>) -> Vec<Bt> {
        let Some(caller) = &self.current else { return pattern };
        match (self.cycles.get(caller), self.cycles.get(f)) {
            (Some(a), Some(b)) if a == b => {}
            _ => return pattern,
        }
        let reach: Vec<Reach> = args.iter().map(|a| reach_at(a, &self.reach)).collect();
        let descends = pattern.iter().zip(&reach).any(|(b, r)| *b == Bt::S && *r == Reach::Strict);
        if !descends {
            for (b, r) in pattern.iter_mut().zip(&reach) {
                if *r == Reach::Unbounded {
                    *b = Bt::D;
                }
            }
        }
        pattern
    }

    /// Lift every variable of `vars` that is static in `env` into the residual, and mark it dynamic.
    fn lift(&self, vars: &HashSet<String>, env: &mut Env, out: &mut Vec<CoreNode>) {
        let mut names: Vec<&String> = vars.iter().filter(|v| env.get(*v) == Some(&Bt::S)).collect();
        names.sort();
        for v in names {
            out.push(node("BDLift", vec![CoreNode::Text(v.clone()), empty_of(&self.empties, v)]));
            env.insert(v.clone(), Bt::D);
        }
    }

    /// `expr` with every call that is not a static evaluation let-bound ahead of it (into `pre`), left to right.
    fn anf(&mut self, expr: &CoreNode, env: &mut Env, ctl: &Ctl, pre: &mut Vec<CoreNode>) -> Result<CoreNode, String> {
        let Some((ctor, args)) = ind(expr) else {
            return Ok(expr.clone());
        };
        if ctor == "CClosure" {
            return Ok(expr.clone());
        }
        if ctor == "CBinOp" {
            let op = args.first().and_then(text).unwrap_or("");
            if (op == "&&" || op == "||") && args.get(2).is_some_and(|r| self.needs_hoist(r, env)) {
                return Err(format!("mix_bta: a call to specialize under a short-circuit `{op}` is not annotated yet"));
            }
        }
        let fields = self.shapes.get(ctor).cloned().unwrap_or_default();
        let mut new_args = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let ty = fields.get(i).map(|(_, t)| t.as_str()).unwrap_or("");
            new_args.push(self.anf_field(a, ty, env, ctl, pre)?);
        }
        if ctor == "CCall" {
            let f = new_args.first().and_then(text).unwrap_or("").to_string();
            if self.funcs.contains_key(&f) {
                let cargs = items(&new_args[1]).to_vec();
                let pattern: Vec<Bt> = cargs.iter().map(|a| self.bt_of(a, env)).collect();
                let all_static = pattern.iter().all(|b| *b == Bt::S);
                if all_static && !self.effectful.contains(&f) {
                    return Ok(node(ctor, new_args));
                }
                let t = self.temp();
                if all_static && ctl.allows(&self.write_backs(&f, &cargs)) {
                    let (name, ret) = self.variant(&f, pattern.clone());
                    if ret == Bt::S {
                        pre.push(node("BSCallLet", vec![CoreNode::Text(t.clone()), CoreNode::Text(name), new_args[1].clone()]));
                        env.insert(t.clone(), Bt::S);
                        return Ok(node("CVar", vec![CoreNode::Text(t)]));
                    }
                }
                self.lift_mutable_args(&f, &cargs, env, pre);
                let pattern = self.generalized(&f, items(&args[1]), cargs.iter().map(|a| self.bt_of(a, env)).collect());
                let (name, _) = self.variant(&f, pattern);
                let bargs: Vec<CoreNode> = cargs.iter().map(|a| self.bexpr(a, env)).collect();
                pre.push(node("BDCallLet", vec![CoreNode::Text(t.clone()), CoreNode::Text(name), CoreNode::List(bargs)]));
                env.insert(t.clone(), Bt::D);
                return Ok(node("CVar", vec![CoreNode::Text(t)]));
            }
        }
        Ok(node(ctor, new_args))
    }

    /// The variables a call of `f` with `args` writes back through its `mutable` parameters.
    fn write_backs(&self, f: &str, args: &[CoreNode]) -> HashSet<String> {
        let mut out = HashSet::new();
        if let Some(func) = self.funcs.get(f) {
            for (a, t) in args.iter().zip(&func.types) {
                if let (true, Some(("CVar", va))) = (t.starts_with("mutable "), ind(a)) {
                    if let Some(n) = va.first().and_then(text) {
                        out.insert(n.to_string());
                    }
                }
            }
        }
        out
    }

    /// Every variable `node` may write: its assignments, the names its `Inspect` arms bind, and the variables its
    /// calls write back.
    fn all_writes(&self, node: &CoreNode) -> HashSet<String> {
        let mut w = HashSet::new();
        writes(node, &mut w);
        let mut cs = Vec::new();
        calls(node, false, &mut cs);
        for (f, args, _) in cs {
            w.extend(self.write_backs(f, args));
        }
        w
    }

    /// A residual call writes the variables its `mutable` parameters take: lift them first, dynamic after.
    fn lift_mutable_args(&self, f: &str, args: &[CoreNode], env: &mut Env, pre: &mut Vec<CoreNode>) {
        let backs = self.write_backs(f, args);
        self.lift(&backs, env, pre);
    }

    fn needs_hoist(&self, e: &CoreNode, env: &Env) -> bool {
        match e {
            CoreNode::Ind(c, args) => {
                if c == "CCall" {
                    if let Some(f) = args.first().and_then(text) {
                        if self.funcs.contains_key(f)
                            && (self.effectful.contains(f) || items(&args[1]).iter().any(|a| self.bt_of(a, env) == Bt::D))
                        {
                            return true;
                        }
                    }
                }
                args.iter().any(|a| self.needs_hoist(a, env))
            }
            CoreNode::List(v) => v.iter().any(|a| self.needs_hoist(a, env)),
            _ => false,
        }
    }

    fn anf_field(&mut self, a: &CoreNode, ty: &str, env: &mut Env, ctl: &Ctl, pre: &mut Vec<CoreNode>) -> Result<CoreNode, String> {
        match ty {
            "CExpr" | "CStringPart" => self.anf(a, env, ctl, pre),
            "Seq of CExpr" | "Seq of CStringPart" => {
                let mut out = Vec::new();
                for it in items(a) {
                    out.push(self.anf(it, env, ctl, pre)?);
                }
                Ok(CoreNode::List(out))
            }
            _ => Ok(a.clone()),
        }
    }

    /// Whether `s` runs entirely while specializing: no effect, no call to unfold or specialize, every variable it
    /// reads static, every variable it writes neither dynamic nor live where `ctl` forbids it, and the same for every
    /// block nested in it. Inside a dynamic region it holds no `Return`, and with a region between it and its loop no
    /// `Break` leaving it: the residual carries the return and the break.
    fn runs_static(&self, s: &CoreNode, env: &Env, ctl: &Ctl) -> bool {
        if mentions_effect(s, &self.effectful, self.run_reads.as_ref()) || self.needs_hoist(s, env) {
            return false;
        }
        let mut r = HashSet::new();
        reads(s, &mut r);
        let w = self.all_writes(s);
        r.iter().all(|v| w.contains(v) || env.get(v) == Some(&Bt::S))
            && w.iter().all(|v| env.get(v) != Some(&Bt::D))
            && ctl.allows(&w)
            && !matches!(ind(s), Some(("CReturn", _)))
            && !(!ctl.is_static() && returns_within(s))
            && !(ctl.residual_break && breaks_within(s))
    }

    /// Enter a dynamic region that writes `w`, with `live` live at its exit: the variables it writes that are live
    /// there move to the residual ahead of it, and static work inside it may write only the others.
    fn enter_region(&self, w: &HashSet<String>, live: &HashSet<String>, env: &mut Env, pre: &mut Vec<CoreNode>) -> Ctl {
        let carried: HashSet<String> = w.intersection(live).cloned().collect();
        self.lift(&carried, env, pre);
        Ctl { region: Some(live.clone()), residual_break: true }
    }

    /// `stmts` annotated under `ctl`, with `after` live after them.
    fn block(&mut self, stmts: &[CoreNode], env: &mut Env, ctl: &Ctl, after: &HashSet<String>) -> Result<CoreNode, String> {
        let mut live_out = vec![HashSet::new(); stmts.len()];
        let mut live = after.clone();
        for (i, s) in stmts.iter().enumerate().rev() {
            live_out[i] = live.clone();
            live = live_before_stmt(s, &live);
        }
        let mut out: Vec<CoreNode> = Vec::new();
        let mut run: Vec<CoreNode> = Vec::new();
        for (s, s_after) in stmts.iter().zip(&live_out) {
            if let Some(("CLet", a)) = ind(s) {
                if let (Some(name), Some(e)) = (a.first().and_then(text), a.get(1)) {
                    if is_typed_empty(e) {
                        self.empties.insert(name.to_string(), e.clone());
                    }
                }
            }
            if self.runs_static(s, env, ctl) {
                for v in self.all_writes(s) {
                    env.insert(v, Bt::S);
                }
                run.push(s.clone());
                continue;
            }
            if !run.is_empty() {
                out.push(node("BSBlock", vec![CoreNode::List(std::mem::take(&mut run))]));
            }
            out.extend(self.stmt(s, env, ctl, s_after)?);
        }
        if !run.is_empty() {
            out.push(node("BSBlock", vec![CoreNode::List(run)]));
        }
        Ok(CoreNode::List(out))
    }

    /// Whether a collection or range loop's body, with its variable static, breaks under dynamic control at its own
    /// level: then the input decides how many passes the loop takes, and it cannot be unrolled. The annotation it
    /// makes to tell is discarded.
    fn breaks_under_dynamic_control(&mut self, body: &[CoreNode], env: &Env, ctl: &Ctl, head: &HashSet<String>, var: &str) -> Result<bool, String> {
        let saved = self.fresh;
        let mut probe_env = env.clone();
        probe_env.insert(var.to_string(), Bt::S);
        let probe = self.block(body, &mut probe_env, &ctl.unrolled_body(), head)?;
        self.fresh = saved;
        Ok(breaks_in_residual(&probe))
    }

    fn stmt(&mut self, s: &CoreNode, env: &mut Env, ctl: &Ctl, after: &HashSet<String>) -> Result<Vec<CoreNode>, String> {
        let (ctor, args) = ind(s).ok_or_else(|| "mix_bta: a statement that is not a constructor".to_string())?;
        let mut pre = Vec::new();
        let tail = match ctor {
            "CShow" => {
                let e = self.anf(&args[0], env, ctl, &mut pre)?;
                match self.bt_of(&e, env) {
                    Bt::S => node("BDShow", vec![node("BLift", vec![e])]),
                    Bt::D => node("BDShow", vec![self.bexpr(&e, env)]),
                }
            }
            "CBreak" => {
                if ctl.residual_break {
                    let stmt = node("CBreak", vec![]);
                    let st = self.statics(&stmt, env);
                    node("BDStmt", vec![stmt, st])
                } else {
                    node("BSBreak", vec![])
                }
            }
            "CFail" => {
                let e = self.anf(&args[0], env, ctl, &mut pre)?;
                match self.bt_of(&e, env) {
                    Bt::S => node("BDFail", vec![node("BLift", vec![e])]),
                    Bt::D => node("BDFail", vec![self.bexpr(&e, env)]),
                }
            }
            "CSleep" => {
                let e = self.anf(&args[0], env, ctl, &mut pre)?;
                let stmt = node(ctor, vec![e]);
                let st = self.statics(&stmt, env);
                node("BDStmt", vec![stmt, st])
            }
            "CReturn" => {
                let e = self.anf(&args[0], env, ctl, &mut pre)?;
                if self.bt_of(&e, env) == Bt::S && ctl.is_static() {
                    node("BSReturn", vec![e])
                } else {
                    self.ret = Bt::D;
                    node("BDReturn", vec![self.bexpr(&e, env)])
                }
            }
            "CLet" | "CSet" => {
                let name = text(&args[0]).unwrap_or("").to_string();
                let e = self.anf(&args[1], env, ctl, &mut pre)?;
                if self.bt_of(&e, env) == Bt::S && !self.pinned.contains(&name) && ctl.allows_var(&name) {
                    env.insert(name.clone(), Bt::S);
                    node("BSBlock", vec![CoreNode::List(vec![node(ctor, vec![args[0].clone(), e])])])
                } else {
                    if ctor == "CSet" {
                        self.lift(&[name.clone()].into_iter().collect(), env, &mut pre);
                    }
                    let stmt = node(ctor, vec![args[0].clone(), e]);
                    let st = self.statics(&stmt, env);
                    env.insert(name, Bt::D);
                    node("BDStmt", vec![stmt, st])
                }
            }
            "CCallS" => {
                let f = args.first().and_then(text).unwrap_or("").to_string();
                let cargs = self.anf_field(&args[1], "Seq of CExpr", env, ctl, &mut pre)?;
                let list = items(&cargs).to_vec();
                let pattern: Vec<Bt> = list.iter().map(|a| self.bt_of(a, env)).collect();
                let all_static = pattern.iter().all(|b| *b == Bt::S);
                let unfolds = all_static && ctl.allows(&self.write_backs(&f, &list));
                if !self.funcs.contains_key(&f) {
                    let stmt = node("CCallS", vec![args[0].clone(), cargs]);
                    if all_static && ctl.is_static() {
                        node("BSBlock", vec![CoreNode::List(vec![stmt])])
                    } else {
                        let st = self.statics(&stmt, env);
                        node("BDStmt", vec![stmt, st])
                    }
                } else if unfolds && self.effectful.contains(&f) {
                    let (name, _) = self.variant(&f, pattern);
                    node("BSCallS", vec![CoreNode::Text(name), cargs])
                } else if unfolds {
                    node("BSBlock", vec![CoreNode::List(vec![node("CCallS", vec![args[0].clone(), cargs])])])
                } else {
                    self.lift_mutable_args(&f, &list, env, &mut pre);
                    let pattern = self.generalized(&f, items(&args[1]), list.iter().map(|a| self.bt_of(a, env)).collect());
                    let (name, _) = self.variant(&f, pattern);
                    let bargs: Vec<CoreNode> = list.iter().map(|a| self.bexpr(a, env)).collect();
                    node("BDCallS", vec![CoreNode::Text(name), CoreNode::List(bargs)])
                }
            }
            "CIf" => {
                let c = self.anf(&args[0], env, ctl, &mut pre)?;
                if self.bt_of(&c, env) == Bt::S {
                    let mut te = env.clone();
                    let t = self.block(items(&args[1]), &mut te, ctl, after)?;
                    let mut ee = env.clone();
                    let e = self.block(items(&args[2]), &mut ee, ctl, after)?;
                    let merged = join_env(&te, &ee);
                    let t = lift_at_merge(t, &te, &merged, &self.empties);
                    let e = lift_at_merge(e, &ee, &merged, &self.empties);
                    *env = merged;
                    node("BSIf", vec![c, t, e])
                } else {
                    let mut w = self.all_writes(&args[1]);
                    w.extend(self.all_writes(&args[2]));
                    let region = self.enter_region(&w, after, env, &mut pre);
                    let bc = self.bexpr(&c, env);
                    let mut te = env.clone();
                    let t = self.block(items(&args[1]), &mut te, &region, after)?;
                    let mut ee = env.clone();
                    let e = self.block(items(&args[2]), &mut ee, &region, after)?;
                    leave_region(env, &w, after);
                    node("BDIf", vec![bc, t, e])
                }
            }
            "CWhile" => {
                if self.needs_hoist(&args[0], env) {
                    return Err("mix_bta: a call to unfold or specialize in a `While` condition is not annotated yet".to_string());
                }
                let body = items(&args[1]);
                let mut cond_reads = HashSet::new();
                reads(&args[0], &mut cond_reads);
                let head = loop_head(after, &cond_reads, body, None);
                let unrolled = ctl.unrolled_body();
                let mut loop_env = env.clone();
                let is_static = loop {
                    if self.bt_of(&args[0], &loop_env) == Bt::D {
                        break false;
                    }
                    let mut body_env = loop_env.clone();
                    let saved = self.fresh;
                    let probe = self.block(body, &mut body_env, &unrolled, &head)?;
                    self.fresh = saved;
                    if breaks_in_residual(&probe) {
                        break false;
                    }
                    let joined = join_env(&loop_env, &body_env);
                    if joined == loop_env {
                        break true;
                    }
                    loop_env = joined;
                };
                let mut static_body = None;
                if is_static {
                    let saved = self.fresh;
                    let mut body_env = loop_env.clone();
                    let b = self.block(body, &mut body_env, &unrolled, &head)?;
                    if breaks_in_residual(&b) {
                        self.fresh = saved;
                    } else {
                        static_body = Some((b, body_env));
                    }
                }
                if let Some((b, body_env)) = static_body {
                    self.lift(&turned_dynamic(env, &loop_env), env, &mut pre);
                    let b = lift_at_merge(b, &body_env, &loop_env, &self.empties);
                    *env = join_env(&loop_env, &body_env);
                    node("BSWhile", vec![args[0].clone(), b])
                } else {
                    let w = self.all_writes(&args[1]);
                    let region = self.enter_region(&w, &head, env, &mut pre);
                    let bc = self.bexpr(&args[0], env);
                    let mut body_env = env.clone();
                    let b = self.block(body, &mut body_env, &region, &head)?;
                    leave_region(env, &w, &head);
                    node("BDWhile", vec![bc, b])
                }
            }
            "CRepeat" => {
                let var = text(&args[0]).unwrap_or("").to_string();
                let coll = self.anf(&args[1], env, ctl, &mut pre)?;
                let body = items(&args[2]);
                let head = loop_head(after, &HashSet::new(), body, Some(&var));
                if self.bt_of(&coll, env) == Bt::S && ctl.allows_var(&var) && !self.breaks_under_dynamic_control(body, env, ctl, &head, &var)? {
                    let unrolled = ctl.unrolled_body();
                    let mut loop_env = env.clone();
                    loop_env.insert(var.clone(), Bt::S);
                    let fresh = self.fresh;
                    let (b, body_env) = loop {
                        self.fresh = fresh;
                        let mut body_env = loop_env.clone();
                        let b = self.block(body, &mut body_env, &unrolled, &head)?;
                        let joined = join_env(&loop_env, &body_env);
                        if joined == loop_env {
                            break (b, body_env);
                        }
                        loop_env = joined;
                    };
                    let b = lift_at_merge(b, &body_env, &loop_env, &self.empties);
                    self.lift(&turned_dynamic(env, &loop_env), env, &mut pre);
                    *env = loop_env;
                    node("BSRepeat", vec![args[0].clone(), coll, b])
                } else {
                    let mut w = self.all_writes(&args[2]);
                    w.insert(var.clone());
                    let region = self.enter_region(&w, &head, env, &mut pre);
                    let bc = self.bexpr(&coll, env);
                    let mut body_env = env.clone();
                    body_env.insert(var.clone(), Bt::D);
                    let b = self.block(body, &mut body_env, &region, &head)?;
                    leave_region(env, &w, &head);
                    node("BDRepeat", vec![args[0].clone(), bc, b])
                }
            }
            "CRepeatRange" => {
                let var = text(&args[0]).unwrap_or("").to_string();
                let start = self.anf(&args[1], env, ctl, &mut pre)?;
                let end = self.anf(&args[2], env, ctl, &mut pre)?;
                let body = items(&args[3]);
                let head = loop_head(after, &HashSet::new(), body, Some(&var));
                if self.bt_of(&start, env) == Bt::S && self.bt_of(&end, env) == Bt::S && ctl.allows_var(&var) && !self.breaks_under_dynamic_control(body, env, ctl, &head, &var)? {
                    let unrolled = ctl.unrolled_body();
                    let mut loop_env = env.clone();
                    loop_env.insert(var.clone(), Bt::S);
                    let fresh = self.fresh;
                    let (b, body_env) = loop {
                        self.fresh = fresh;
                        let mut body_env = loop_env.clone();
                        let b = self.block(body, &mut body_env, &unrolled, &head)?;
                        let joined = join_env(&loop_env, &body_env);
                        if joined == loop_env {
                            break (b, body_env);
                        }
                        loop_env = joined;
                    };
                    let b = lift_at_merge(b, &body_env, &loop_env, &self.empties);
                    self.lift(&turned_dynamic(env, &loop_env), env, &mut pre);
                    *env = loop_env;
                    node("BSRepeatRange", vec![args[0].clone(), start, end, b])
                } else {
                    let mut w = self.all_writes(&args[3]);
                    w.insert(var.clone());
                    let region = self.enter_region(&w, &head, env, &mut pre);
                    let bs = self.bexpr(&start, env);
                    let be = self.bexpr(&end, env);
                    let mut body_env = env.clone();
                    body_env.insert(var.clone(), Bt::D);
                    let b = self.block(body, &mut body_env, &region, &head)?;
                    leave_region(env, &w, &head);
                    node("BDRepeatRange", vec![args[0].clone(), bs, be, b])
                }
            }
            "CInspect" => {
                let t = self.anf(&args[0], env, ctl, &mut pre)?;
                let bt = self.bt_of(&t, env);
                let w = self.all_writes(&args[1]);
                let region = match bt {
                    Bt::S => None,
                    Bt::D => Some(self.enter_region(&w, after, env, &mut pre)),
                };
                let arm_ctl = region.as_ref().unwrap_or(ctl);
                let mut arms = Vec::new();
                let mut ends = Vec::new();
                for arm in items(&args[1]) {
                    let mut arm_env = env.clone();
                    match ind(arm) {
                        Some(("CWhen", a)) => {
                            for b in items(&a[1]).iter().filter_map(text) {
                                arm_env.insert(b.to_string(), bt);
                            }
                            let body = self.block(items(&a[2]), &mut arm_env, arm_ctl, after)?;
                            arms.push(("BWhen", vec![a[0].clone(), a[1].clone()], body));
                        }
                        Some(("COtherwise", a)) => {
                            let body = self.block(items(&a[0]), &mut arm_env, arm_ctl, after)?;
                            arms.push(("BOtherwise", Vec::new(), body));
                        }
                        _ => return Err("mix_bta: an Inspect arm of an unknown form".to_string()),
                    }
                    ends.push(arm_env);
                }
                let merged = ends.iter().fold(env.clone(), |j, e| join_env(&j, e));
                let arms: Vec<CoreNode> = arms
                    .into_iter()
                    .zip(&ends)
                    .map(|((form, mut fields, body), end)| {
                        fields.push(if bt == Bt::S { lift_at_merge(body, end, &merged, &self.empties) } else { body });
                        node(form, fields)
                    })
                    .collect();
                match bt {
                    Bt::S => {
                        *env = merged;
                        node("BSInspect", vec![t, CoreNode::List(arms)])
                    }
                    Bt::D => {
                        leave_region(env, &w, after);
                        node("BDInspect", vec![self.bexpr(&t, env), CoreNode::List(arms)])
                    }
                }
            }
            other if EFFECT_STMTS.contains(&other) => {
                return Err(format!("mix_bta: the effect `{other}` is not annotated yet"));
            }
            _ => {
                let fields = self.shapes.get(ctor).cloned().unwrap_or_default();
                let mut new_args = Vec::with_capacity(args.len());
                for (i, a) in args.iter().enumerate() {
                    let ty = fields.get(i).map(|(_, t)| t.as_str()).unwrap_or("");
                    if ty.contains("CStmt") {
                        return Err(format!("mix_bta: `{ctor}` with a nested block is not annotated yet"));
                    }
                    new_args.push(self.anf_field(a, ty, env, ctl, &mut pre)?);
                }
                let stmt = node(ctor, new_args);
                let w = self.all_writes(&stmt);
                if self.bt_of(&stmt, env) == Bt::S
                    && w.iter().all(|v| env.get(v) != Some(&Bt::D) && !self.pinned.contains(v))
                    && ctl.allows(&w)
                {
                    node("BSBlock", vec![CoreNode::List(vec![stmt])])
                } else {
                    self.lift(&w, env, &mut pre);
                    let st = self.statics(&stmt, env);
                    for v in w {
                        env.insert(v, Bt::D);
                    }
                    node("BDStmt", vec![stmt, st])
                }
            }
        };
        pre.push(tail);
        Ok(pre)
    }

    /// Every variant's name and return binding time, in name order.
    fn returns(&self) -> Vec<(String, Bt)> {
        let mut r: Vec<(String, Bt)> = self.variants.values().map(|v| (v.name.clone(), v.ret)).collect();
        r.sort_by(|a, b| a.0.cmp(&b.0));
        r
    }

    /// Analyse every queued variant, in key order, each numbering its temporaries from the start (they are local to
    /// its residual function), so the two-level program does not depend on how a map orders its keys; repeat until no
    /// variant's return binding time changes.
    fn run(&mut self) -> Result<(), String> {
        loop {
            let mut changed = false;
            let mut done: HashSet<(String, Vec<Bt>)> = HashSet::new();
            loop {
                let mut pending: Vec<(String, Vec<Bt>)> =
                    self.variants.keys().filter(|k| !done.contains(*k)).cloned().collect();
                pending.sort();
                if pending.is_empty() {
                    break;
                }
                for key in pending {
                    done.insert(key.clone());
                    let (params, types, body) = {
                        let f = &self.funcs[&key.0];
                        (f.params.clone(), f.types.clone(), f.body.clone())
                    };
                    let mut env: Env = params.iter().cloned().zip(key.1.iter().copied()).collect();
                    let written_back: Vec<(&String, Bt)> = params
                        .iter()
                        .zip(&types)
                        .zip(&key.1)
                        .filter(|((_, t), _)| t.starts_with("mutable "))
                        .map(|((p, _), b)| (p, *b))
                        .collect();
                    let exit: HashSet<String> = written_back.iter().map(|(p, _)| (*p).clone()).collect();
                    self.pinned = written_back.iter().filter(|(_, b)| *b == Bt::D).map(|(p, _)| (*p).clone()).collect();
                    self.ret = Bt::S;
                    self.fresh = 0;
                    self.current = Some(key.0.clone());
                    self.empties = params
                        .iter()
                        .zip(&types)
                        .filter_map(|(p, t)| empty_construction(t).map(|e| (p.clone(), e)))
                        .collect();
                    let annotated = self.block(&body, &mut env, &Ctl::top(), &exit)?;
                    let ret = self.ret;
                    let v = self.variants.get_mut(&key).expect("queued variant");
                    if v.ret != ret {
                        v.ret = ret;
                        changed = true;
                    }
                    v.node = Some(annotated);
                }
            }
            self.queue.clear();
            if !changed {
                return Ok(());
            }
        }
    }
}

/// Prints [`CoreNode`]s as flat LOGOS statements: one `Let` per constructor or list, so no expression nests.
struct Printer<'a> {
    shapes: &'a HashMap<String, Vec<(String, String)>>,
    out: String,
    n: usize,
}

impl Printer<'_> {
    fn value(&mut self, node: &CoreNode, ty: &str) -> Result<String, String> {
        match node {
            CoreNode::Int(n) if *n < 0 => Ok(format!("(0 - {})", n.unsigned_abs())),
            CoreNode::Int(n) => Ok(n.to_string()),
            CoreNode::Bool(b) => Ok(b.to_string()),
            CoreNode::Float(f) => Ok(format!("{f:?}")),
            CoreNode::Text(s) => Ok(logicaffeine_language::lexer::text_literal(s)),
            CoreNode::Atom(a) => Ok(format!("({})", a.expression())),
            CoreNode::List(elems) => {
                let elem_ty = ty.strip_prefix("Seq of ").unwrap_or("Int").to_string();
                self.n += 1;
                let v = format!("mixN{}", self.n);
                let mut parts = Vec::with_capacity(elems.len());
                for e in elems {
                    parts.push(self.value(e, &elem_ty)?);
                }
                self.out.push_str(&format!("Let mutable {v} be a new Seq of {elem_ty}.\n"));
                for p in parts {
                    self.out.push_str(&format!("Push {p} to {v}.\n"));
                }
                Ok(v)
            }
            CoreNode::Ind(ctor, args) => {
                let fields = self
                    .shapes
                    .get(ctor)
                    .cloned()
                    .ok_or_else(|| format!("mix_bta: no catalog entry for `{ctor}`"))?;
                if fields.len() != args.len() {
                    return Err(format!("mix_bta: `{ctor}` has {} fields, {} given", fields.len(), args.len()));
                }
                let mut parts = Vec::with_capacity(args.len());
                for ((f, t), a) in fields.iter().zip(args) {
                    parts.push(format!("{f} {}", self.value(a, t)?));
                }
                self.n += 1;
                let v = format!("mixN{}", self.n);
                if parts.is_empty() {
                    self.out.push_str(&format!("Let {v} be a new {ctor}.\n"));
                } else {
                    self.out.push_str(&format!("Let {v} be a new {ctor} with {}.\n", parts.join(" and ")));
                }
                Ok(v)
            }
        }
    }
}

/// How many times each variable is written, counting every assignment site at any depth, every name an `Inspect`
/// arm binds, and every variable passed to a `mutable` parameter, which the call writes back.
fn write_counts(node: &CoreNode, funcs: &HashMap<String, Func>, out: &mut HashMap<String, usize>) {
    match node {
        CoreNode::Ind(c, args) => {
            if let Some((_, i)) = WRITE_TARGETS.iter().find(|(w, _)| w == c) {
                if let Some(n) = args.get(*i).and_then(text) {
                    *out.entry(n.to_string()).or_insert(0) += 1;
                }
            }
            if c == "CWhen" {
                for n in items(&args[1]).iter().filter_map(text) {
                    *out.entry(n.to_string()).or_insert(0) += 1;
                }
            }
            if c == "CCall" || c == "CCallS" {
                if let (Some(f), Some(a)) = (args.first().and_then(text).and_then(|f| funcs.get(f)), args.get(1)) {
                    for (t, arg) in f.types.iter().zip(items(a)) {
                        if let (true, Some(("CVar", va))) = (t.starts_with("mutable "), ind(arg)) {
                            if let Some(n) = va.first().and_then(text) {
                                *out.entry(n.to_string()).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
            args.iter().for_each(|a| write_counts(a, funcs, out));
        }
        CoreNode::List(v) => v.iter().for_each(|a| write_counts(a, funcs, out)),
        _ => {}
    }
}

/// The calls `node` makes, as `(callee, arguments, inside a closure)`, at any depth.
fn calls<'a>(node: &'a CoreNode, in_closure: bool, out: &mut Vec<(&'a str, &'a [CoreNode], bool)>) {
    match node {
        CoreNode::Ind(c, args) => {
            if c == "CCall" || c == "CCallS" {
                if let (Some(f), Some(a)) = (args.first().and_then(text), args.get(1)) {
                    out.push((f, items(a), in_closure));
                }
            }
            let inner = in_closure || c == "CClosure";
            args.iter().for_each(|a| calls(a, inner, out));
        }
        CoreNode::List(v) => v.iter().for_each(|a| calls(a, in_closure, out)),
        _ => {}
    }
}

/// For every function, which parameters are invariant: every call passes the value of one variable of main that holds
/// a single value for the whole run (an input main never writes, or a top-level `Let` main never writes again),
/// directly or threaded through invariant parameters no function writes. Every call then passes the same value, so a
/// specialization need not key on it.
fn invariant_params(funcs: &HashMap<String, Func>, main: &[CoreNode]) -> HashMap<String, Vec<bool>> {
    #[derive(Clone, PartialEq)]
    enum Root {
        Unknown,
        From(String),
        No,
    }
    let meet = |a: &Root, b: &Root| match (a, b) {
        (Root::Unknown, r) | (r, Root::Unknown) => r.clone(),
        (Root::From(x), Root::From(y)) if x == y => a.clone(),
        _ => Root::No,
    };
    let mut main_writes = HashMap::new();
    main.iter().for_each(|s| write_counts(s, funcs, &mut main_writes));
    let single = |x: &str| match main_writes.get(x).copied().unwrap_or(0) {
        0 => true,
        1 => main.iter().any(|s| matches!(ind(s), Some(("CLet", a)) if a.first().and_then(text) == Some(x))),
        _ => false,
    };
    let mut roots: HashMap<String, Vec<Root>> = HashMap::new();
    for (n, f) in funcs {
        let mut w = HashMap::new();
        f.body.iter().for_each(|s| write_counts(s, funcs, &mut w));
        let r = f
            .params
            .iter()
            .zip(&f.types)
            .map(|(p, t)| if w.contains_key(p) || t.starts_with("mutable ") { Root::No } else { Root::Unknown })
            .collect();
        roots.insert(n.clone(), r);
    }
    let mut sites: Vec<(Option<&str>, &str, &[CoreNode], bool)> = Vec::new();
    let mut main_calls = Vec::new();
    main.iter().for_each(|s| calls(s, false, &mut main_calls));
    sites.extend(main_calls.into_iter().map(|(g, a, c)| (None, g, a, c)));
    for (n, f) in funcs {
        let mut fc = Vec::new();
        f.body.iter().for_each(|s| calls(s, false, &mut fc));
        sites.extend(fc.into_iter().map(|(g, a, c)| (Some(n.as_str()), g, a, c)));
    }
    loop {
        let before = roots.clone();
        for (caller, g, args, in_closure) in &sites {
            if !roots.contains_key(*g) {
                continue;
            }
            for (i, a) in args.iter().enumerate() {
                let var = match ind(a) {
                    Some(("CVar", va)) if !in_closure => va.first().and_then(text),
                    _ => None,
                };
                let source = match (var, caller) {
                    (Some(x), None) if single(x) => Root::From(x.to_string()),
                    (Some(x), Some(f)) => match funcs[*f].params.iter().position(|p| p == x) {
                        Some(j) => before[*f][j].clone(),
                        None => Root::No,
                    },
                    _ => Root::No,
                };
                if let Some(slot) = roots.get_mut(*g).and_then(|r| r.get_mut(i)) {
                    *slot = meet(slot, &source);
                }
            }
        }
        if roots == before {
            break;
        }
    }
    roots.into_iter().map(|(n, r)| (n, r.iter().map(|x| matches!(x, Root::From(_))).collect())).collect()
}

/// A two-level program: its functions as Core IR, which `core_interp` runs for the static parts; its two-level
/// functions, the variants `mix` unfolds or specializes; and its two-level main block.
pub struct TwoLevelProgram {
    funcs: Vec<(String, CoreNode)>,
    bfuncs: Vec<(String, CoreNode)>,
    main: CoreNode,
    shapes: HashMap<String, Vec<(String, String)>>,
    enums: HashMap<String, String>,
}

impl TwoLevelProgram {
    /// LOGOS functions building the program as `mix` reads it: `mixBuildFuncs () -> Map of Text to CFunc`,
    /// `mixBuildBFuncs () -> Map of Text to BFunc` and `mixBuildMain () -> Seq of BStmt`. Every binding a builder makes
    /// dies with its frame.
    pub fn builders(&self) -> Result<String, String> {
        let indent = |body: &str| body.lines().map(|l| format!("    {l}\n")).collect::<String>();
        let mut out = String::new();
        for (builder, ty, value_ty, map, entries) in [
            ("mixBuildFuncs", "Map of Text to CFunc", "CFunc", "mixFuncs", &self.funcs),
            ("mixBuildBFuncs", "Map of Text to BFunc", "BFunc", "mixBFuncs", &self.bfuncs),
        ] {
            let mut printer = Printer { shapes: &self.shapes, out: String::new(), n: 0 };
            printer.out.push_str(&format!("Let mutable {map} be a new {ty}.\n"));
            for (name, f) in entries {
                let v = printer.value(f, value_ty)?;
                printer.out.push_str(&format!(
                    "Set item {} of {map} to {v}.\n",
                    logicaffeine_language::lexer::text_literal(name)
                ));
            }
            printer.out.push_str(&format!("Return {map}.\n"));
            out.push_str(&format!("## To {builder} () -> {ty}:\n{}\n", indent(&printer.out)));
        }
        let mut printer = Printer { shapes: &self.shapes, out: String::new(), n: 0 };
        let main = printer.value(&self.main, "Seq of BStmt")?;
        printer.out.push_str(&format!("Return {main}.\n"));
        out.push_str(&format!("## To mixBuildMain () -> Seq of BStmt:\n{}\n", indent(&printer.out)));
        Ok(out)
    }

    /// The program as the input of a program that specializes it: `mixMain`, `mixFuncs` and `mixBFuncs`, the names
    /// `mix`'s own run takes them by.
    pub fn store_entries(&self) -> Vec<(&'static str, StoreEntry)> {
        vec![
            ("mixMain", StoreEntry::Value(self.main.clone())),
            ("mixFuncs", StoreEntry::Map(self.funcs.clone())),
            ("mixBFuncs", StoreEntry::Map(self.bfuncs.clone())),
        ]
    }

    /// The native specializer's input, `MixIn(main, store, funcs, bfuncs)`, as wire bytes: this program as `mix`
    /// reads it, and `store` as `core_interp`'s values ([`store_builder`] builds the same values in LOGOS).
    pub fn mix_input_wire(&self, store: &[(&str, StoreEntry)]) -> Result<Vec<u8>, String> {
        use logicaffeine_data::wire::{write_inductive_header, write_str, write_uvarint, T_MAP, T_TEXT};
        let wire = Wire { shapes: &self.shapes, enums: &self.enums };
        let mut out = Vec::new();
        write_inductive_header(&mut out, "MixInput", "MixIn", 4);
        wire.value(&self.main, &mut out)?;
        wire.store(store, &mut out)?;
        for entries in [&self.funcs, &self.bfuncs] {
            out.push(T_MAP);
            write_uvarint(entries.len() as u64, &mut out);
            for (name, node) in entries {
                out.push(T_TEXT);
                write_str(name, &mut out);
                wire.value(node, &mut out)?;
            }
        }
        Ok(out)
    }
}

/// Writes [`CoreNode`]s in the wire form the generated engines decode: as the values of the catalog's types, or as
/// `core_interp`'s values of them.
struct Wire<'a> {
    shapes: &'a HashMap<String, Vec<(String, String)>>,
    enums: &'a HashMap<String, String>,
}

impl Wire<'_> {
    fn value(&self, node: &CoreNode, out: &mut Vec<u8>) -> Result<(), String> {
        use logicaffeine_data::wire::*;
        match node {
            CoreNode::Int(n) => {
                out.push(T_INT);
                write_uvarint(zigzag(*n), out);
            }
            CoreNode::Bool(b) => out.push(if *b { T_TRUE } else { T_FALSE }),
            CoreNode::Float(f) => {
                out.push(T_FLOAT);
                out.extend_from_slice(&f.to_le_bytes());
            }
            CoreNode::Text(s) => {
                out.push(T_TEXT);
                write_str(s, out);
            }
            CoreNode::Atom(a) => a.write_wire(out),
            CoreNode::List(items) => {
                out.push(T_LIST);
                write_uvarint(items.len() as u64, out);
                for it in items {
                    self.value(it, out)?;
                }
            }
            CoreNode::Ind(ctor, args) => {
                let ty = self.enums.get(ctor).ok_or_else(|| format!("mix_bta: no catalog entry for `{ctor}`"))?;
                write_inductive_header(out, ty, ctor, args.len() as u64);
                for a in args {
                    self.value(a, out)?;
                }
            }
        }
        Ok(())
    }

    fn cval_header(ctor: &str, out: &mut Vec<u8>) {
        logicaffeine_data::wire::write_inductive_header(out, "CVal", ctor, 1);
    }

    fn cval_text(s: &str, out: &mut Vec<u8>) {
        Self::cval_header("VText", out);
        out.push(logicaffeine_data::wire::T_TEXT);
        logicaffeine_data::wire::write_str(s, out);
    }

    /// `core_interp`'s value of `node`: as [`CValPrinter`] builds it, a constructor as `CNewVariant` evaluates it.
    fn cval(&self, node: &CoreNode, out: &mut Vec<u8>) -> Result<(), String> {
        use logicaffeine_data::wire::*;
        match node {
            CoreNode::Int(_) => Self::cval_header("VInt", out),
            CoreNode::Bool(_) => Self::cval_header("VBool", out),
            CoreNode::Float(_) => Self::cval_header("VFloat", out),
            CoreNode::Atom(a) => Self::cval_header(a.value_variant(), out),
            CoreNode::Text(s) => {
                Self::cval_text(s, out);
                return Ok(());
            }
            CoreNode::List(items) => {
                Self::cval_header("VSeq", out);
                out.push(T_LIST);
                write_uvarint(items.len() as u64, out);
                for it in items {
                    self.cval(it, out)?;
                }
                return Ok(());
            }
            CoreNode::Ind(ctor, args) => {
                let fields = self.shapes.get(ctor).ok_or_else(|| format!("mix_bta: no catalog entry for `{ctor}`"))?;
                if fields.len() != args.len() {
                    return Err(format!("mix_bta: `{ctor}` has {} fields, {} given", fields.len(), args.len()));
                }
                Self::cval_header("VMap", out);
                out.push(T_MAP);
                write_uvarint(args.len() as u64 + 2, out);
                out.push(T_TEXT);
                write_str("__tag", out);
                Self::cval_text(ctor, out);
                for ((f, _), a) in fields.iter().zip(args) {
                    out.push(T_TEXT);
                    write_str(f, out);
                    self.cval(a, out)?;
                }
                out.push(T_TEXT);
                write_str("__fnames__", out);
                Self::cval_header("VSeq", out);
                out.push(T_LIST);
                write_uvarint(fields.len() as u64, out);
                for (f, _) in fields {
                    Self::cval_text(f, out);
                }
                return Ok(());
            }
        }
        self.value(node, out)
    }

    /// A store, `Map of Text to CVal`: each name with `core_interp`'s value of its entry.
    fn store(&self, store: &[(&str, StoreEntry)], out: &mut Vec<u8>) -> Result<(), String> {
        use logicaffeine_data::wire::*;
        out.push(T_MAP);
        write_uvarint(store.len() as u64, out);
        for (key, entry) in store {
            out.push(T_TEXT);
            write_str(key, out);
            match entry {
                StoreEntry::Value(node) => self.cval(node, out)?,
                StoreEntry::Map(items) => self.cval_map(items, out)?,
            }
        }
        Ok(())
    }

    /// A map of texts as `core_interp` holds one: each entry under its key's exact text ([`text_key`]), holding the
    /// key and the value as a pair.
    fn cval_map(&self, items: &[(String, CoreNode)], out: &mut Vec<u8>) -> Result<(), String> {
        use logicaffeine_data::wire::*;
        Self::cval_header("VMap", out);
        out.push(T_MAP);
        write_uvarint(items.len() as u64, out);
        for (k, node) in items {
            out.push(T_TEXT);
            write_str(&text_key(k), out);
            Self::cval_header("VTuple", out);
            out.push(T_LIST);
            write_uvarint(2, out);
            Self::cval_text(k, out);
            self.cval(node, out)?;
        }
        Ok(())
    }
}

/// A residual engine's input, `ResidualIn(store)`, as wire bytes: `store` as `core_interp`'s values, which the
/// residual's main reads as `pStore` ([`store_builder`] builds the same values in LOGOS). `catalog` names every
/// constructor's fields.
pub fn residual_input_wire(store: &[(&str, StoreEntry)], catalog: &str) -> Result<Vec<u8>, String> {
    let shapes = catalog_shapes(catalog);
    let enums = catalog_enums(catalog);
    let wire = Wire { shapes: &shapes, enums: &enums };
    let mut out = Vec::new();
    logicaffeine_data::wire::write_inductive_header(&mut out, "ResidualInput", "ResidualIn", 1);
    wire.store(store, &mut out)?;
    Ok(out)
}

/// Constructor → the enum it belongs to, from a catalog written as [`catalog_shapes`] reads it.
fn catalog_enums(catalog: &str) -> HashMap<String, String> {
    let mut enums = HashMap::new();
    let mut current: Option<String> = None;
    for line in catalog.lines() {
        if let Some(rest) = line.strip_prefix("## A ").or_else(|| line.strip_prefix("## An ")) {
            current = line
                .trim_end()
                .ends_with("is one of:")
                .then(|| rest.split_whitespace().next().unwrap_or("").to_string());
            continue;
        }
        if line.starts_with("## ") {
            current = None;
            continue;
        }
        if let Some(ty) = &current {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("A ").or_else(|| t.strip_prefix("An ")) {
                let ctor: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !ctor.is_empty() {
                    enums.insert(ctor, ty.clone());
                }
            }
        }
    }
    enums
}

/// One static input of a specialization: a value, or a map from names to values (a program's functions).
pub enum StoreEntry {
    Value(CoreNode),
    Map(Vec<(String, CoreNode)>),
}

/// The text `core_interp` keys a map entry whose key is the text `key` under (its `coreMapKey`).
pub fn text_key(key: &str) -> String {
    format!("t{key}")
}

/// A LOGOS function `name () -> Map of Text to CVal` building `entries` as the values `core_interp` makes of them —
/// a constructor as `CNewVariant` evaluates it, a `VMap` of its `__tag`, its fields in order and their `__fnames__`;
/// a list as a `VSeq` — the static store `mix` specializes a program with. `catalog` names every constructor's
/// fields.
pub fn store_builder(name: &str, entries: &[(&str, StoreEntry)], catalog: &str) -> Result<String, String> {
    let shapes = catalog_shapes(catalog);
    let mut printer = CValPrinter { shapes: &shapes, out: String::new(), n: 0, names: HashMap::new() };
    printer.out.push_str("Let mutable mixStore be a new Map of Text to CVal.\n");
    for (key, entry) in entries {
        let v = match entry {
            StoreEntry::Value(node) => printer.value(node)?,
            StoreEntry::Map(items) => {
                printer.n += 1;
                let m = format!("mixM{}", printer.n);
                let mut parts = Vec::with_capacity(items.len());
                for (k, node) in items {
                    parts.push((k, printer.value(node)?));
                }
                printer.out.push_str(&format!("Let mutable {m} be a new Map of Text to CVal.\n"));
                for (k, v) in parts {
                    printer.n += 1;
                    let pair = format!("mixP{}", printer.n);
                    let key = logicaffeine_language::lexer::text_literal(k);
                    printer.out.push_str(&format!("Let mutable {pair} be a new Seq of CVal.\n"));
                    printer.out.push_str(&format!("Push (a new VText with value {key}) to {pair}.\n"));
                    printer.out.push_str(&format!("Push {v} to {pair}.\n"));
                    printer.out.push_str(&format!(
                        "Set item {} of {m} to a new VTuple with items {pair}.\n",
                        logicaffeine_language::lexer::text_literal(&text_key(k))
                    ));
                }
                printer.bind(format!("a new VMap with entries {m}"))
            }
        };
        printer.out.push_str(&format!(
            "Set item {} of mixStore to {v}.\n",
            logicaffeine_language::lexer::text_literal(key)
        ));
    }
    printer.out.push_str("Return mixStore.\n");
    let body: String = printer.out.lines().map(|l| format!("    {l}\n")).collect();
    Ok(format!("{MIX_VARIANT}\n## To {name} () -> Map of Text to CVal:\n{body}\n"))
}

/// The value `CNewVariant` evaluates to, built from its tag, field names and field values.
const MIX_VARIANT: &str = "## To mixVariant (vTag: Text) and (vNames: Seq of Text) and (vVals: Seq of CVal) -> CVal:
    Let mutable vMap be a new Map of Text to CVal.
    Set item \"__tag\" of vMap to a new VText with value vTag.
    Let mutable vNameVals be a new Seq of CVal.
    Let mutable vi be 1.
    While vi is at most (length of vNames):
        Let vName be item vi of vNames.
        Set item vName of vMap to item vi of vVals.
        Push a new VText with value vName to vNameVals.
        Set vi to vi + 1.
    Set item \"__fnames__\" of vMap to a new VSeq with items vNameVals.
    Return a new VMap with entries vMap.
";

/// Prints [`CoreNode`]s as LOGOS statements building `core_interp`'s values of them, one `Let` per constructor or list.
struct CValPrinter<'a> {
    shapes: &'a HashMap<String, Vec<(String, String)>>,
    out: String,
    n: usize,
    /// Constructor → the variable holding its field names, bound at its first use.
    names: HashMap<String, String>,
}

impl CValPrinter<'_> {
    fn bind(&mut self, expr: String) -> String {
        self.n += 1;
        let v = format!("mixC{}", self.n);
        self.out.push_str(&format!("Let {v} be {expr}.\n"));
        v
    }

    fn value(&mut self, node: &CoreNode) -> Result<String, String> {
        Ok(match node {
            CoreNode::Int(n) if *n < 0 => format!("(a new VInt with value (0 - {}))", n.unsigned_abs()),
            CoreNode::Int(n) => format!("(a new VInt with value {n})"),
            CoreNode::Bool(b) => format!("(a new VBool with value {b})"),
            CoreNode::Float(f) => format!("(a new VFloat with value {f:?})"),
            CoreNode::Text(s) => format!("(a new VText with value {})", logicaffeine_language::lexer::text_literal(s)),
            CoreNode::Atom(a) => format!("(a new {} with value ({}))", a.value_variant(), a.expression()),
            CoreNode::List(elems) => {
                let mut parts = Vec::with_capacity(elems.len());
                for e in elems {
                    parts.push(self.value(e)?);
                }
                self.n += 1;
                let v = format!("mixL{}", self.n);
                self.out.push_str(&format!("Let mutable {v} be a new Seq of CVal.\n"));
                for p in parts {
                    self.out.push_str(&format!("Push {p} to {v}.\n"));
                }
                self.bind(format!("a new VSeq with items {v}"))
            }
            CoreNode::Ind(ctor, args) => {
                let fields = self
                    .shapes
                    .get(ctor)
                    .cloned()
                    .ok_or_else(|| format!("mix_bta: no catalog entry for `{ctor}`"))?;
                if fields.len() != args.len() {
                    return Err(format!("mix_bta: `{ctor}` has {} fields, {} given", fields.len(), args.len()));
                }
                let mut parts = Vec::with_capacity(args.len());
                for a in args {
                    parts.push(self.value(a)?);
                }
                let names = match self.names.get(ctor) {
                    Some(n) => n.clone(),
                    None => {
                        let n = format!("mixF{}", self.names.len() + 1);
                        self.out.push_str(&format!("Let mutable {n} be a new Seq of Text.\n"));
                        for (f, _) in &fields {
                            self.out.push_str(&format!("Push {} to {n}.\n", logicaffeine_language::lexer::text_literal(f)));
                        }
                        self.names.insert(ctor.clone(), n.clone());
                        n
                    }
                };
                self.n += 1;
                let vals = format!("mixV{}", self.n);
                self.out.push_str(&format!("Let mutable {vals} be a new Seq of CVal.\n"));
                for p in parts {
                    self.out.push_str(&format!("Push {p} to {vals}.\n"));
                }
                self.bind(format!(
                    "mixVariant({}, {names}, {vals})",
                    logicaffeine_language::lexer::text_literal(ctor)
                ))
            }
        })
    }
}

/// The two-level program of `prog` (`CProg(funcs, main)`) ([`TwoLevelProgram`]). Main's free variables are its
/// inputs: those in `statics` are known while specializing, those in `dynamic` only when the residual runs. `catalog`
/// is the Core IR catalog followed by `mix`'s own.
pub fn annotate(prog: &CoreNode, catalog: &str, statics: &[&str], dynamic: &[&str]) -> Result<TwoLevelProgram, String> {
    let shapes = catalog_shapes(catalog);
    let (funcs, main) = match ind(prog) {
        Some(("CProg", a)) if a.len() == 2 => (items(&a[0]), items(&a[1])),
        _ => return Err("mix_bta: not a CProg".to_string()),
    };
    let run_reads: Option<HashSet<String>> = (!dynamic.is_empty()).then(|| {
        funcs
            .iter()
            .filter_map(|f| match ind(f) {
                Some(("CFuncDef", fa)) => fa.first().and_then(text).map(str::to_string),
                _ => None,
            })
            .collect()
    });
    let effectful = effectful_functions(funcs, run_reads.as_ref());
    let mut table = HashMap::new();
    for f in funcs {
        if let Some(("CFuncDef", fa)) = ind(f) {
            table.insert(
                fa.first().and_then(text).unwrap_or("").to_string(),
                Func {
                    params: items(&fa[1]).iter().filter_map(text).map(str::to_string).collect(),
                    types: items(&fa[2]).iter().filter_map(text).map(str::to_string).collect(),
                    ret: fa.get(3).and_then(text).unwrap_or("Nothing").to_string(),
                    body: std::rc::Rc::new(items(&fa[4]).to_vec()),
                },
            );
        }
    }
    let mut annotator = Annotator {
        funcs: table,
        effectful,
        run_reads,
        empties: HashMap::new(),
        shapes: &shapes,
        variants: HashMap::new(),
        queue: Vec::new(),
        fresh: 0,
        ret: Bt::S,
        pinned: HashSet::new(),
        current: None,
        reach: HashMap::new(),
        cycles: HashMap::new(),
    };
    for f in annotator.funcs.values() {
        reach_uses(&f.params, &f.body, &annotator.funcs, &shapes, &mut annotator.reach);
    }
    annotator.cycles = recursive_components(&annotator.funcs);
    let inputs = |_: ()| -> Env {
        statics
            .iter()
            .map(|s| (s.to_string(), Bt::S))
            .chain(dynamic.iter().map(|d| (d.to_string(), Bt::D)))
            .collect()
    };
    let mut main_env: Env = inputs(());
    let mut main_ann;
    loop {
        annotator.pinned.clear();
        annotator.current = None;
        annotator.empties.clear();
        main_ann = annotator.block(main, &mut main_env, &Ctl::top(), &HashSet::new())?;
        let before = annotator.returns();
        annotator.run()?;
        let after = annotator.returns();
        main_env = inputs(());
        annotator.fresh = 0;
        if before == after {
            break;
        }
    }
    let invariants = invariant_params(&annotator.funcs, main);
    let mut func_nodes = Vec::new();
    for f in funcs {
        if let Some(("CFuncDef", fa)) = ind(f) {
            func_nodes.push((fa.first().and_then(text).unwrap_or("").to_string(), f.clone()));
        }
    }
    let mut keys: Vec<&(String, Vec<Bt>)> = annotator.variants.keys().collect();
    keys.sort_by(|a, b| annotator.variants[*a].name.cmp(&annotator.variants[*b].name));
    let mut bfunc_nodes = Vec::with_capacity(keys.len());
    for key in keys {
        let v = &annotator.variants[key];
        let f = &annotator.funcs[&key.0];
        let body = v.node.clone().ok_or_else(|| format!("mix_bta: `{}` was never analysed", v.name))?;
        bfunc_nodes.push((
            v.name.clone(),
            node(
                "BFuncDef",
                vec![
                    CoreNode::Text(v.name.clone()),
                    CoreNode::List(f.params.iter().cloned().map(CoreNode::Text).collect()),
                    CoreNode::List(f.types.iter().cloned().map(CoreNode::Text).collect()),
                    CoreNode::Text(f.ret.clone()),
                    CoreNode::List(key.1.iter().map(|b| CoreNode::Bool(*b == Bt::S)).collect()),
                    CoreNode::List(f.types.iter().map(|t| CoreNode::Bool(t.starts_with("mutable "))).collect()),
                    CoreNode::List(
                        invariants
                            .get(&key.0)
                            .map(|v| v.iter().map(|b| CoreNode::Bool(*b)).collect())
                            .unwrap_or_default(),
                    ),
                    body,
                ],
            ),
        ));
    }
    Ok(TwoLevelProgram { funcs: func_nodes, bfuncs: bfunc_nodes, main: main_ann, shapes, enums: catalog_enums(catalog) })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/mix_bta/tests.rs"]
mod tests;
