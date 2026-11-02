//! Real numbers a program defines by a function — `the real number whose digits in base 3 are f`,
//! `the real number approximated by f` — on the interpreter's values.
//!
//! The number is base's [`ExactReal::defined`]: it reads its function's digits or approximations
//! only as a refinement needs them. Here the function is a closure value of the running program.
//! The engine running the program opens a [`RunScope`] the first time the program defines a real:
//! the scope registers the closures the program defines reals by, and holds the evaluator that calls
//! one in a context of its own — a child interpreter over the same program on the tree-walker, a
//! fresh machine over the same bytecode on the VM — so a digit read in the middle of an operation
//! changes nothing the program sees. Every engine alive on a thread — the program's, and each task's
//! — keeps a run of its own there, however the scheduler interleaves them; a defined real names its
//! run and its closure, and read after its run has ended, it refuses. The checker admits only a
//! function that reads nothing but its argument and does nothing but answer
//! ([`crate::analysis::defined_real`]), so where it runs makes no difference to what it answers.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use logicaffeine_base::exact_real::{not_a_rational_answer, Approximant, Definition, ExactReal};
use logicaffeine_base::Rational;

use crate::interpreter::RuntimeValue;

/// How a run calls one of its closures on its own: the closure and its argument to its answer.
pub(crate) type Evaluator<'a> = Rc<dyn Fn(&RuntimeValue, i64) -> Result<RuntimeValue, String> + 'a>;

/// One run's evaluator and the closures its program has defined reals by.
struct Run {
    evaluator: Evaluator<'static>,
    closures: Vec<RuntimeValue>,
}

thread_local! {
    /// The runs open on this thread, by generation.
    static RUNS: RefCell<HashMap<u64, Run>> = RefCell::new(HashMap::new());
    /// The generation the next run takes.
    static GENERATIONS: Cell<u64> = const { Cell::new(0) };
}

/// A run's place on its thread, open while the engine that opened it lives; dropping it closes the
/// run, and every real it defined refuses from then on.
pub(crate) struct RunScope<'a> {
    generation: u64,
    _program: std::marker::PhantomData<&'a ()>,
}

impl Drop for RunScope<'_> {
    fn drop(&mut self) {
        // The run leaves the table before it drops: its evaluator may hold an engine whose own runs
        // close as it goes.
        let closed = RUNS.try_with(|r| r.borrow_mut().remove(&self.generation)).ok().flatten();
        drop(closed);
    }
}

/// The evaluator as the thread's table holds it.
fn erased<'a>(evaluator: Evaluator<'a>) -> Evaluator<'static> {
    // SAFETY: the evaluator is stored only in this thread's `RUNS` table, under the generation of a
    // scope that borrows `'a` and removes it when it drops — within `'a` — and a clone taken from the
    // table by `Interpreted::call` is dropped before that call returns.
    unsafe { std::mem::transmute::<Evaluator<'a>, Evaluator<'static>>(evaluator) }
}

/// Open a run whose evaluator may borrow the running program for `'a`.
pub(crate) fn open_run<'a>(evaluator: Evaluator<'a>) -> RunScope<'a> {
    let evaluator = erased(evaluator);
    let generation = GENERATIONS.with(|g| {
        let next = g.get() + 1;
        g.set(next);
        next
    });
    RUNS.with(|r| {
        r.borrow_mut().insert(
            generation,
            Run {
                evaluator,
                closures: Vec::new(),
            },
        )
    });
    RunScope {
        generation,
        _program: std::marker::PhantomData,
    }
}

impl<'a> RunScope<'a> {
    /// Hand the run an evaluator that sees more of the program than the one it has — the
    /// tree-walker learns a closure's body only when it builds the closure.
    pub(crate) fn renew(&self, evaluator: Evaluator<'a>) {
        let evaluator = erased(evaluator);
        let replaced = RUNS.with(|r| {
            r.borrow_mut()
                .get_mut(&self.generation)
                .map(|run| std::mem::replace(&mut run.evaluator, evaluator))
        });
        drop(replaced);
    }

    /// `definedReal(kind, base, name, f)`: the real number `f` defines, in this run.
    pub(crate) fn define(&self, args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
        let (definition, name, f) = definition(args)?;
        let index = RUNS
            .with(|r| {
                r.borrow_mut().get_mut(&self.generation).map(|run| {
                    run.closures.push(f);
                    run.closures.len() - 1
                })
            })
            .ok_or_else(|| format!("{name} defines a real number after its program's run ended"))?;
        let source = Arc::new(Interpreted {
            name: name.clone(),
            generation: self.generation,
            index,
        });
        Ok(RuntimeValue::ExactReal(Rc::new(ExactReal::defined(definition, &name, source))))
    }
}

/// A closure of a run, by the run's generation and the closure's place among its closures.
struct Interpreted {
    name: String,
    generation: u64,
    index: usize,
}

impl Approximant for Interpreted {
    fn call(&self, n: u64) -> Result<Rational, String> {
        let found = RUNS.with(|r| {
            let runs = r.borrow();
            let run = runs.get(&self.generation)?;
            Some((run.closures.get(self.index)?.clone(), run.evaluator.clone()))
        });
        let Some((closure, evaluator)) = found else {
            return Err(format!(
                "{} is read after the run of the program that defined its real number",
                self.name
            ));
        };
        let arg = i64::try_from(n).map_err(|_| format!("{}({n}) is past every Int", self.name))?;
        let answer = evaluator(&closure, arg)?;
        answered_rational(&self.name, n, &answer)
    }
}

/// The rational a function's answer is; anything else is refused in a defined real's words.
pub(crate) fn answered_rational(name: &str, n: u64, v: &RuntimeValue) -> Result<Rational, String> {
    match v {
        RuntimeValue::Int(i) => Ok(Rational::from_i64(*i)),
        RuntimeValue::BigInt(b) => Ok(Rational::from_bigint((**b).clone())),
        RuntimeValue::Rational(r) => Ok((**r).clone()),
        other => Err(not_a_rational_answer(name, n, &other.type_name())),
    }
}

/// What `definedReal(kind, base, name, f)`'s arguments say: the definition, the function's name and
/// the closure.
fn definition(args: Vec<RuntimeValue>) -> Result<(Definition, String, RuntimeValue), String> {
    let shape = || "definedReal takes a kind, a base, a name and a function".to_string();
    let [kind, base, name, f]: [RuntimeValue; 4] = args.try_into().map_err(|_| shape())?;
    let (RuntimeValue::Text(kind), RuntimeValue::Int(base), RuntimeValue::Text(name)) = (&kind, &base, &name) else {
        return Err(shape());
    };
    if !matches!(f, RuntimeValue::Function(_)) {
        return Err(format!("{name} defines a real number as a function, got {}", f.type_name()));
    }
    Ok((Definition::named(kind, *base)?, name.to_string(), f))
}

/// `definedReal` called where no engine runs the program: refused, naming the function.
pub(crate) fn outside_a_run(args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
    let (_, name, _) = definition(args)?;
    Err(format!("{name} defines a real number outside a running program"))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/defined/tests.rs"]
mod tests;
