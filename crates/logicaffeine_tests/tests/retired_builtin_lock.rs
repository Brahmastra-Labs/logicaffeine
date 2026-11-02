//! ★ LOCK — a builtin the language no longer has is refused, before the program runs, on every tier. ★
//!
//! A builtin leaves the language by retirement (`builtins::RETIRED_BUILTINS`): its `BuiltinId` stays,
//! so bincode numbers every id after it as before (`builtin_wire_id_lock`), no source spelling
//! resolves to it, and a call to its old name is refused in its own sentence, which names what a
//! program writes instead. The refusal is the front end's (`known_calls::refuse_retired_calls`), so
//! every tier says the same words whether or not the call is ever reached, and a program that
//! defines a function of that name calls its own. The id itself refuses too, wherever one arrives.

use logicaffeine_compile::analysis::PolicyRegistry;
use logicaffeine_compile::compile::{
    compile_to_rust, compile_to_wasm, tw_outcome, vm_outcome, RunOutcome,
};
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::builtins::{
    builtin_from_name, call_builtin, check_arity, retired_builtin, BuiltinId, RETIRED_BUILTINS,
};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::vm::{CompiledProgram, Constant, Op};
use logicaffeine_compile::Interner;
use logicaffeine_language::error::socratic_explanation;

/// The sentence a call to `moveChildren` is refused in.
const MOVE_CHILDREN_IS_GONE: &str = "`moveChildren` is gone: a caller's children are drawn where the \
    caller is, as a range of the retained tree's tape (`vdRangeOpen`/`vdRangeClose`), and placed where \
    the callee says `{children}` (`vdSlot`) — which an LGX component's view does for you.";

/// What each tier answers for `src`: the tree-walker's and the VM's error, and the wasm and AOT-Rust
/// compile refusals, in the words a diagnostic shows.
fn refusals(src: &str) -> [(&'static str, Option<String>); 4] {
    let words = |e: logicaffeine_language::error::ParseError| {
        socratic_explanation(&e, &logicaffeine_language::Interner::new())
    };
    [
        ("tree-walker", tw_outcome(src).error),
        ("VM", vm_outcome(src).error),
        ("wasm", compile_to_wasm(src).map_err(words).err()),
        ("AOT-Rust", compile_to_rust(src).map_err(words).err()),
    ]
}

/// How many arguments `id` was called with.
fn arity(id: BuiltinId) -> usize {
    (0..=8)
        .find(|&n| check_arity(id, n).is_ok())
        .unwrap_or_else(|| panic!("{id:?} has an arity"))
}

#[test]
fn the_retirement_table_names_each_builtin_once() {
    assert!(
        RETIRED_BUILTINS.len() >= 2,
        "beginPaint and moveChildren are retired: {}",
        RETIRED_BUILTINS.len()
    );
    for (i, (id, name, why)) in RETIRED_BUILTINS.iter().enumerate() {
        assert!(
            !why.is_empty() && why.starts_with(&format!("`{name}` is gone: ")),
            "{name}: its sentence names it: {why}"
        );
        assert!(
            RETIRED_BUILTINS[i + 1..]
                .iter()
                .all(|(other, spelling, _)| other != id && spelling != name),
            "{name} is retired once"
        );
        assert!(
            BuiltinId::ALL.contains(id),
            "{name}'s id keeps its place in the catalogue"
        );
    }
}

#[test]
fn every_retired_builtin_is_refused_before_running_on_every_tier() {
    for (_, name, why) in RETIRED_BUILTINS {
        let src = format!("## Main\nLet d be createElement(\"div\").\nCall {name}.\nCall appendChild with body() and d.\n");
        for (tier, refusal) in refusals(&src) {
            assert_eq!(
                refusal.as_deref(),
                Some(*why),
                "the {tier} refuses `{name}` in its sentence"
            );
        }
        dom::reset();
        let _ = tw_outcome(&src);
        assert_eq!(
            dom::render(),
            "<body></body>",
            "`{name}` is refused before anything ran"
        );
    }
}

#[test]
fn a_program_may_call_its_own_function_named_like_any_retired_builtin() {
    // The refusal is of the retired builtin, not of the name.
    for (_, name, _) in RETIRED_BUILTINS {
        let src = format!("## To {name}():\n    Show \"own\".\n\n## Main\nCall {name}.\n");
        for (tier, run) in [
            ("tree-walker", tw_outcome as fn(&str) -> RunOutcome),
            ("VM", vm_outcome),
        ] {
            let out = run(&src);
            assert_eq!(
                (out.error.as_deref(), out.output.trim()),
                (None, "own"),
                "the {tier} calls the program's own `{name}`"
            );
        }
        assert!(
            compile_to_wasm(&src).is_ok(),
            "a program's own `{name}` compiles to wasm"
        );
    }
}

#[test]
fn no_spelling_resolves_to_a_retired_builtin() {
    for (_, name, why) in RETIRED_BUILTINS {
        assert_eq!(
            builtin_from_name(name),
            None,
            "no source spelling reaches the retired `{name}`"
        );
        assert_eq!(
            retired_builtin(name),
            Some(*why),
            "`{name}` is refused in its sentence"
        );
    }
    assert_eq!(
        retired_builtin("appendChild"),
        None,
        "a live builtin is not retired"
    );
}

#[test]
fn every_retired_id_is_refused_by_the_evaluator() {
    // The id keeps its wire position, so a program an earlier build serialized can still name it;
    // the evaluator refuses it in the same sentence instead of doing anything.
    for (id, name, why) in RETIRED_BUILTINS {
        dom::reset();
        let answer = call_builtin(*id, vec![RuntimeValue::Int(0); arity(*id)]);
        assert_eq!(answer.err().as_deref(), Some(*why), "`{name}`");
        assert_eq!(dom::op_log(), Vec::new(), "`{name}` writes nothing");
    }
}

#[test]
fn every_retired_id_is_refused_by_the_wasm_backend() {
    for (id, name, _) in RETIRED_BUILTINS {
        let args = arity(*id);
        let mut code: Vec<Op> = (0..args as u16)
            .map(|dst| Op::LoadConst { dst, idx: 0 })
            .collect();
        code.push(Op::CallBuiltin {
            dst: args as u16,
            builtin: *id,
            args_start: 0,
            arg_count: args as u16,
        });
        code.push(Op::Halt);
        let program = CompiledProgram {
            constants: vec![Constant::Int(0)],
            code,
            register_count: args + 1,
            ..Default::default()
        };
        let refused = logicaffeine_compile::vm::wasm::assemble_program(
            &program,
            &PolicyRegistry::default(),
            &Interner::new(),
        )
        .expect_err("a retired id does not lower");
        assert!(
            refused.to_string().contains(&format!("{name} is retired")),
            "`{name}`: {refused}"
        );
    }
}

#[test]
fn the_retired_child_move_is_refused_on_every_tier_pointing_at_the_tape_range() {
    // `moveChildren` reparented a holder's children for a `{children}` slot. A slot's children are
    // a range of the retained tree's tape, placed by the commit; a call that looked like it placed
    // children and did not would be worse than none.
    let src = "## Main\nLet a be createElement(\"div\").\nLet b be createElement(\"p\").\n\
               Call moveChildren with a and b.\nCall appendChild with body() and b.\n";
    for (tier, refusal) in refusals(src) {
        assert_eq!(
            refusal.as_deref(),
            Some(MOVE_CHILDREN_IS_GONE),
            "the {tier} refuses it in its sentence"
        );
    }
    dom::reset();
    let _ = tw_outcome(src);
    assert_eq!(
        dom::render(),
        "<body></body>",
        "refused before anything ran"
    );
}

#[test]
fn no_spelling_resolves_to_the_retired_child_move() {
    assert_eq!(
        builtin_from_name("moveChildren"),
        None,
        "no source spelling reaches the retired id"
    );
    assert_eq!(
        retired_builtin("moveChildren"),
        Some(MOVE_CHILDREN_IS_GONE),
        "its name is refused in its sentence"
    );
}

#[test]
fn the_retired_child_move_id_refuses_if_it_arrives() {
    dom::reset();
    let answer = call_builtin(
        BuiltinId::DomMoveChildren,
        vec![RuntimeValue::Int(0), RuntimeValue::Int(0)],
    );
    assert_eq!(answer.err().as_deref(), Some(MOVE_CHILDREN_IS_GONE));
    assert_eq!(dom::op_log(), Vec::new(), "and writes nothing");
}

#[test]
fn a_program_of_its_own_may_define_a_function_named_move_children() {
    let src = "## To moveChildren():\n    Show \"moving\".\n\n## Main\nCall moveChildren.\n";
    for (tier, run) in [
        ("tree-walker", tw_outcome as fn(&str) -> RunOutcome),
        ("VM", vm_outcome),
    ] {
        let out = run(src);
        assert_eq!(
            (out.error.as_deref(), out.output.trim()),
            (None, "moving"),
            "the {tier} calls the program's own function"
        );
    }
    assert!(compile_to_wasm(src).is_ok(), "and it compiles to wasm");
}
