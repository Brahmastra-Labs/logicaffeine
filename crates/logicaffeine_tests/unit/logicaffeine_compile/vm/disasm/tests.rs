use super::*;

fn prog() -> CompiledProgram {
    CompiledProgram {
        constants: vec![
            Constant::Int(6),
            Constant::Int(7),
            Constant::Text("hi".into()),
        ],
        code: vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::Mul {
                dst: 2,
                lhs: 0,
                rhs: 1,
            },
            Op::JumpIfFalse { cond: 2, target: 7 },
            Op::Show { src: 2 },
            Op::Jump { target: 7 },
            Op::LoadConst { dst: 3, idx: 2 },
            Op::Halt,
        ],
        ..Default::default()
    }
}

#[test]
fn renders_common_ops_with_resolved_operands() {
    let d = disassemble(&prog());
    assert_eq!(d[0].text, "LoadConst R0 = 6");
    assert_eq!(d[1].text, "LoadConst R1 = 7");
    assert_eq!(d[2].text, "Mul R2 = R0 * R1");
    assert_eq!(d[3].text, "JumpIfFalse R2 -> 7");
    assert_eq!(d[4].text, "Show R2");
    assert_eq!(d[5].text, "Jump -> 7");
    assert_eq!(d[6].text, "LoadConst R3 = \"hi\"");
    assert_eq!(d[7].text, "Halt");
}

#[test]
fn extracts_jump_targets() {
    let d = disassemble(&prog());
    assert_eq!(d[3].targets, vec![7]); // JumpIfFalse
    assert_eq!(d[5].targets, vec![7]); // Jump
    assert!(d[2].targets.is_empty()); // straight-line Mul
}

#[test]
fn op_io_identifies_reads_and_writes() {
    assert_eq!(
        op_io(&Op::Add {
            dst: 2,
            lhs: 0,
            rhs: 1
        }),
        OpIo {
            writes: Some(2),
            reads: vec![0, 1]
        }
    );
    assert_eq!(
        op_io(&Op::LoadConst { dst: 0, idx: 0 }),
        OpIo {
            writes: Some(0),
            reads: vec![]
        }
    );
    assert_eq!(
        op_io(&Op::Show { src: 3 }),
        OpIo {
            writes: None,
            reads: vec![3]
        }
    );
    assert_eq!(
        op_io(&Op::Move { dst: 1, src: 0 }),
        OpIo {
            writes: Some(1),
            reads: vec![0]
        }
    );
}

#[test]
fn pc_indices_are_sequential() {
    let d = disassemble(&prog());
    for (i, line) in d.iter().enumerate() {
        assert_eq!(line.pc, i);
    }
}
