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

const INT: Option<Form> = Some(Form::Number(Number::Int));
const BIG: Option<Form> = Some(Form::Number(Number::BigInt));
const RATIONAL: Option<Form> = Some(Form::Number(Number::Rational));
const REAL: Option<Form> = Some(Form::Number(Number::Real));

/// The joins of `code` (one entry register per `entries`), each op definition's own form given by pc and each
/// entry's by register; `reals` the registers that hold a real wherever they are read.
fn join(
    code: &[Op],
    constants: &[Constant],
    entries: u16,
    forms: &[(usize, Option<Form>)],
    entry_forms: &[(u16, Option<Form>)],
    reals: &[u16],
) -> Joined {
    joined(
        code,
        constants,
        &Linear(code),
        entries,
        |site| match site {
            Site::Op(pc) => forms.iter().find(|(p, _)| *p == pc).and_then(|(_, f)| *f),
            Site::Entry(reg) => entry_forms.iter().find(|(r, _)| *r == reg).and_then(|(_, f)| *f),
        },
        |_, reg| reals.contains(&reg),
    )
}

/// `total = <first>` then `total = total + x` in a loop over a real `x` (register 1), `total` shown after; `read`
/// replaces the display with any other op over `total` (register 0).
fn accumulate(first: Op, read: Op) -> Vec<Op> {
    vec![
        first,
        Op::Lt { dst: 2, lhs: 3, rhs: 4 },
        Op::JumpIfFalse { cond: 2, target: 5 },
        Op::AddAssign { dst: 0, src: 1 },
        Op::Jump { target: 1 },
        read,
        Op::Halt,
    ]
}

#[test]
fn an_integer_constant_summed_with_reals_lands_as_a_real() {
    let code = accumulate(Op::LoadConst { dst: 0, idx: 0 }, Op::Show { src: 0 });
    let j = join(&code, &[Constant::Int(0)], 5, &[(0, INT), (3, REAL)], &[], &[1]);
    assert_eq!(j.lands, BTreeMap::from([(0, Form::Number(Number::Real))]));
    assert!(j.refused.is_empty(), "{j:?}");
}

#[test]
fn an_integer_constant_copied_in_lands_as_a_real() {
    let code = vec![
        Op::LoadConst { dst: 5, idx: 0 },
        Op::Move { dst: 0, src: 5 },
        Op::Lt { dst: 2, lhs: 3, rhs: 4 },
        Op::JumpIfFalse { cond: 2, target: 6 },
        Op::AddAssign { dst: 0, src: 1 },
        Op::Jump { target: 2 },
        Op::Show { src: 0 },
        Op::Halt,
    ];
    let j = join(&code, &[Constant::Int(0)], 5, &[(0, INT), (1, INT), (4, REAL)], &[], &[1]);
    assert_eq!(j.lands, BTreeMap::from([(1, Form::Number(Number::Real))]));
    assert!(j.refused.is_empty(), "{j:?}");
}

#[test]
fn an_integer_a_double_may_not_hold_is_refused_beside_a_real() {
    for c in [1i64 << 53, -(1i64 << 53), i64::MIN, i64::MAX] {
        let code = accumulate(Op::LoadConst { dst: 0, idx: 0 }, Op::Show { src: 0 });
        let j = join(&code, &[Constant::Int(c)], 5, &[(0, INT), (3, REAL)], &[], &[1]);
        assert_eq!(j.refused, vec![Site::Op(0)], "{c}");
        assert!(j.lands.is_empty(), "{c}");
    }
    let code = accumulate(Op::LoadConst { dst: 0, idx: 0 }, Op::Show { src: 0 });
    let j = join(&code, &[Constant::Int((1 << 53) - 1)], 5, &[(0, INT), (3, REAL)], &[], &[1]);
    assert!(j.refused.is_empty(), "every integer below 2^53 is exact in a double");
}

#[test]
fn an_integer_made_at_run_time_is_refused_beside_a_real() {
    let parse = Op::CallBuiltin {
        dst: 0,
        builtin: crate::semantics::builtins::BuiltinId::ParseInt,
        args_start: 6,
        arg_count: 1,
    };
    let code = accumulate(parse, Op::Show { src: 0 });
    let j = join(&code, &[], 7, &[(0, INT), (3, REAL)], &[], &[1]);
    assert_eq!(j.refused, vec![Site::Op(0)]);
}

#[test]
fn an_op_that_tells_an_integer_from_a_real_refuses_the_widening() {
    let telling = [
        Op::Div { dst: 9, lhs: 0, rhs: 3 },
        Op::Mod { dst: 9, lhs: 0, rhs: 3 },
        Op::FloorDiv { dst: 9, lhs: 0, rhs: 3 },
        Op::Pow { dst: 9, lhs: 0, rhs: 3 },
        Op::BitXor { dst: 9, lhs: 0, rhs: 3 },
        Op::Add { dst: 9, lhs: 0, rhs: 3 },
        Op::Index { dst: 9, collection: 3, index: 0 },
        Op::Return { src: 0 },
    ];
    for read in telling {
        let code = accumulate(Op::LoadConst { dst: 0, idx: 0 }, read);
        let j = join(&code, &[Constant::Int(0)], 5, &[(0, INT), (3, REAL)], &[], &[1]);
        assert_eq!(j.refused, vec![Site::Op(0)], "{read:?} reads the integer phase");
    }
}

#[test]
fn an_order_test_or_a_sum_beside_a_real_cannot_tell() {
    for read in [
        Op::Lt { dst: 9, lhs: 0, rhs: 3 },
        Op::Eq { dst: 9, lhs: 3, rhs: 0 },
        Op::NotEq { dst: 9, lhs: 0, rhs: 3 },
        Op::Add { dst: 9, lhs: 1, rhs: 0 },
        Op::Mul { dst: 9, lhs: 0, rhs: 1 },
    ] {
        let code = accumulate(Op::LoadConst { dst: 0, idx: 0 }, read);
        let j = join(&code, &[Constant::Int(0)], 5, &[(0, INT), (3, REAL)], &[], &[1]);
        assert!(j.refused.is_empty(), "{read:?}: {j:?}");
    }
}

#[test]
fn a_copy_into_another_variable_widens_it_with_the_first() {
    let code = vec![
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Lt { dst: 2, lhs: 3, rhs: 4 },
        Op::JumpIfFalse { cond: 2, target: 5 },
        Op::AddAssign { dst: 0, src: 1 },
        Op::Jump { target: 1 },
        Op::Move { dst: 7, src: 0 },
        Op::Show { src: 7 },
        Op::Halt,
    ];
    let j = join(&code, &[Constant::Int(0)], 5, &[(0, INT), (3, REAL), (5, INT)], &[], &[1]);
    assert!(j.refused.is_empty(), "{j:?}");
    assert_eq!(
        j.lands,
        BTreeMap::from([(0, Form::Number(Number::Real)), (5, Form::Number(Number::Real))]),
        "the copy holds the widened value too"
    );
    let mut divided = code.clone();
    divided[6] = Op::Div { dst: 9, lhs: 7, rhs: 3 };
    let j = join(&divided, &[Constant::Int(0)], 5, &[(0, INT), (3, REAL), (5, INT)], &[], &[1]);
    assert_eq!(j.refused, vec![Site::Op(0)], "what the copy is read by counts");
}

#[test]
fn integers_and_rationals_join_as_rationals_wherever_they_meet() {
    let code = accumulate(
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Div { dst: 9, lhs: 0, rhs: 3 },
    );
    let j = join(&code, &[Constant::Int(i64::MAX)], 5, &[(0, INT), (3, RATIONAL)], &[], &[]);
    assert_eq!(j.lands, BTreeMap::from([(0, Form::Number(Number::Rational))]));
    assert!(j.refused.is_empty(), "a rational answers every op on a whole value as its integer");
    let j = join(&code, &[], 5, &[(0, BIG), (3, INT)], &[], &[]);
    assert_eq!(j.lands, BTreeMap::from([(3, Form::Number(Number::BigInt))]));
}

#[test]
fn an_exact_number_beside_a_real_has_no_machine_form() {
    for exact in [RATIONAL, BIG] {
        let code = accumulate(Op::LoadConst { dst: 0, idx: 0 }, Op::Show { src: 0 });
        let j = join(&code, &[Constant::Int(0)], 5, &[(0, exact), (3, REAL)], &[], &[1]);
        assert_eq!(j.refused, vec![Site::Op(0)], "{exact:?}");
        assert!(j.lands.is_empty());
    }
}

#[test]
fn a_straight_run_keeps_each_value_in_its_own_form() {
    let code = [
        Op::LoadConst { dst: 0, idx: 0 },
        Op::Div { dst: 9, lhs: 0, rhs: 3 },
        Op::AddAssign { dst: 0, src: 1 },
        Op::Show { src: 0 },
        Op::Halt,
    ];
    let j = join(&code, &[Constant::Int(5)], 4, &[(0, INT), (2, REAL)], &[], &[1]);
    assert_eq!(j, Joined::default(), "no read is reached by both");
}

#[test]
fn a_list_grown_in_place_with_a_rational_holds_rationals() {
    let code = [
        Op::NewList { dst: 0, start: 1, count: 1 },
        Op::ListPush { list: 0, value: 2 },
        Op::Show { src: 0 },
        Op::Halt,
    ];
    let j = join(
        &code,
        &[],
        3,
        &[(0, Some(Form::List(Number::Int))), (1, Some(Form::List(Number::Rational)))],
        &[],
        &[],
    );
    assert_eq!(j.lands, BTreeMap::from([(0, Form::List(Number::Rational))]));
}

#[test]
fn a_parameter_that_meets_a_wider_form_is_converted_on_entry() {
    let code = [
        Op::Lt { dst: 2, lhs: 3, rhs: 4 },
        Op::JumpIfFalse { cond: 2, target: 4 },
        Op::AddAssign { dst: 0, src: 1 },
        Op::Jump { target: 0 },
        Op::Show { src: 0 },
        Op::Halt,
    ];
    let j = join(&code, &[], 5, &[(2, RATIONAL)], &[(0, INT)], &[]);
    assert_eq!(j.entries, BTreeMap::from([(0, Form::Number(Number::Rational))]));
    let j = join(&code, &[], 5, &[(2, REAL)], &[(0, INT)], &[1]);
    assert_eq!(j.refused, vec![Site::Entry(0)], "an integer arriving from a caller is any integer");
}

#[test]
fn a_variable_with_a_value_that_is_no_number_is_left_to_its_backend() {
    let code = accumulate(Op::LoadConst { dst: 0, idx: 0 }, Op::Show { src: 0 });
    let j = join(&code, &[Constant::Int(0)], 5, &[(0, INT), (3, None)], &[], &[]);
    assert_eq!(j, Joined::default());
}
