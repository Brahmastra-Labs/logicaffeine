use super::*;

/// Bytecode as one region: every pc runs, control falls through or jumps.
struct Linear<'a>(&'a [Op]);

impl Flow for Linear<'_> {
    fn pcs(&self) -> Vec<usize> {
        (0..self.0.len()).collect()
    }

    fn successors(&self, pc: usize) -> Vec<usize> {
        match self.0[pc] {
            Op::Jump { target } => vec![target as usize],
            Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => vec![pc + 1, target as usize],
            Op::Halt | Op::Return { .. } | Op::ReturnNothing => Vec::new(),
            _ => vec![pc + 1],
        }
    }

    fn written(&self, op: &Op, f: &mut dyn FnMut(u16)) {
        op.for_each_written_reg(f);
    }

    fn read(&self, op: &Op, f: &mut dyn FnMut(u16)) {
        op.for_each_read_reg(|_| 0, f);
    }
}

fn never(_: &Op, _: u16, _: u16) -> bool {
    false
}

/// The variable of the definition the op at `pc` makes.
fn var_at(vars: &Variables, pc: usize) -> usize {
    vars.variable(vars.definition(pc).expect("the op defines a register").1)
}

#[test]
fn a_straight_run_of_assignments_makes_one_variable_per_value() {
    let code = [
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Add { dst: 0, lhs: 0, rhs: 1 },
        Op::Show { src: 0 },
        Op::Halt,
    ];
    let vars = Variables::of(&code, &Linear(&code), 2, never);
    assert_ne!(var_at(&vars, 0), var_at(&vars, 1), "each read is reached by one definition");
    assert_eq!(vars.reaching(1, 0).collect::<Vec<_>>(), vec![vars.definition(0).unwrap().1]);
    assert_eq!(vars.reaching(2, 0).collect::<Vec<_>>(), vec![vars.definition(1).unwrap().1]);
}

#[test]
fn an_op_that_continues_what_it_reads_joins_its_definition_to_that_variable() {
    let code = [
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Add { dst: 0, lhs: 0, rhs: 1 },
        Op::Halt,
    ];
    let vars = Variables::of(&code, &Linear(&code), 2, |_, w, r| w == r);
    assert_eq!(var_at(&vars, 0), var_at(&vars, 1));
    let vars = Variables::of(&code, &Linear(&code), 2, |_, w, r| w != r);
    assert_ne!(var_at(&vars, 0), var_at(&vars, 1), "the predicate decides, not the shape");
}

#[test]
fn a_loop_joins_the_definition_before_it_to_the_one_inside_it() {
    let code = [
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Lt { dst: 2, lhs: 0, rhs: 1 },
        Op::JumpIfFalse { cond: 2, target: 5 },
        Op::AddAssign { dst: 0, src: 1 },
        Op::Jump { target: 1 },
        Op::Show { src: 0 },
        Op::Halt,
    ];
    let vars = Variables::of(&code, &Linear(&code), 2, never);
    assert_eq!(var_at(&vars, 0), var_at(&vars, 3));
    let reaching: Vec<usize> = vars.reaching(5, 0).collect();
    assert_eq!(reaching.len(), 2, "the exit reads both the value before the loop and the one made in it");
}

#[test]
fn a_branch_joins_the_values_its_arms_make() {
    let code = [
        Op::JumpIfFalse { cond: 9, target: 3 },
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Jump { target: 4 },
        Op::LoadConst { dst: 0, idx: 1 },
        Op::Show { src: 0 },
        Op::Halt,
    ];
    let vars = Variables::of(&code, &Linear(&code), 10, never);
    assert_eq!(var_at(&vars, 1), var_at(&vars, 3));
}

#[test]
fn a_register_reused_for_an_unrelated_value_is_two_variables() {
    let code = [
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Show { src: 0 },
        Op::LoadConst { dst: 0, idx: 1 },
        Op::Show { src: 0 },
        Op::Halt,
    ];
    let vars = Variables::of(&code, &Linear(&code), 1, never);
    assert_ne!(var_at(&vars, 0), var_at(&vars, 2));
}

#[test]
fn a_register_live_on_entry_is_a_definition_of_its_own() {
    let code = [Op::Show { src: 0 }, Op::LoadConst { dst: 0, idx: 0 }, Op::Show { src: 0 }, Op::Halt];
    let vars = Variables::of(&code, &Linear(&code), 1, never);
    let entry: Vec<usize> = vars.reaching(0, 0).collect();
    assert_eq!(entry.len(), 1);
    assert_eq!(vars.site(entry[0]), Site::Entry(0));
    assert_eq!(vars.register(entry[0]), 0);
    assert_ne!(vars.variable(entry[0]), var_at(&vars, 1));
}

#[test]
fn a_collection_changed_in_place_is_defined_by_the_change() {
    let push = Op::ListPush { list: 0, value: 1 };
    assert_eq!(defined(&push), Some(0));
    assert!(changes_in_place(&push));
    let store = Op::SetIndex { collection: 0, index: 1, value: 2 };
    assert_eq!(defined(&store), Some(0));
    assert!(changes_in_place(&store));
    assert!(!changes_in_place(&Op::AddAssign { dst: 0, src: 1 }), "a new number is a new value");
    let code = [Op::NewEmptyList { dst: 0 }, push, Op::Show { src: 0 }, Op::Halt];
    let vars = Variables::of(&code, &Linear(&code), 2, |op, w, r| w == r && changes_in_place(op));
    assert_eq!(var_at(&vars, 0), var_at(&vars, 1));
}

#[test]
fn an_op_that_defines_nothing_makes_no_definition() {
    let code = [Op::Show { src: 0 }, Op::Halt];
    let vars = Variables::of(&code, &Linear(&code), 1, never);
    assert_eq!(vars.definition(0), None);
    assert_eq!(vars.sites().count(), 1, "only the entry");
}
