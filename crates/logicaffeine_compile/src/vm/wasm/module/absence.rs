//! A global loan's absence travels separately from a valid zero or empty payload.
use super::*;

/// Only registers that can carry a global's missing value need a shadow write.
/// The bytecode already separates value lifetimes; Moves propagate that possibility.
fn prepare(plan: &mut Plan, constants: &[Constant], globals: &[bool]) {
    plan.absence_globals = globals.to_vec();
    plan.absence_regs = vec![false; plan.num_regs as usize];
    if !plan.absence {
        return;
    }
    for op in &plan.ops {
        match *op {
            Op::GlobalGet { dst, idx } if globals.get(idx as usize).copied().unwrap_or(false) => {
                plan.absence_regs[dst as usize] = true
            }
            Op::LoadConst { dst, idx }
                if matches!(constants.get(idx as usize), Some(Constant::Nothing)) =>
            {
                plan.absence_regs[dst as usize] = true;
            }
            _ => {}
        }
    }
    loop {
        let mut changed = false;
        for op in &plan.ops {
            if let Op::Move { dst, src } = *op {
                if plan.absence_regs[src as usize] && !plan.absence_regs[dst as usize] {
                    plan.absence_regs[dst as usize] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Only a clear or an absent value stored into a global can make that global missing.
/// Propagate stores and loads to a fixed point before deciding which shadow bits to emit.
pub(super) fn decide(
    main: &mut Plan,
    fns: &mut [Plan],
    constants: &[Constant],
    count: usize,
    linked: bool,
) -> bool {
    if linked {
        return false;
    }
    let mut globals = vec![false; count];
    loop {
        for plan in std::iter::once(&mut *main).chain(fns.iter_mut()) {
            plan.absence = true;
            prepare(plan, constants, &globals);
        }
        let mut changed = false;
        for plan in std::iter::once(&*main).chain(fns.iter()) {
            for (pc, op) in plan.ops.iter().enumerate() {
                if let Op::GlobalSet { idx, src } = *op {
                    if !globals[idx as usize]
                        && (plan.place.clear.get(pc).copied().unwrap_or(false)
                            || tracked(plan, src)
                            || plan.kinds.get(src as usize) == Some(Kind::Nothing))
                    {
                        globals[idx as usize] = true;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let enabled = globals.iter().any(|v| *v);
    for plan in std::iter::once(main).chain(fns.iter_mut()) {
        plan.absence = enabled;
        prepare(plan, constants, &globals);
    }
    enabled
}

fn tracked(plan: &Plan, r: u16) -> bool {
    plan.absence_regs.get(r as usize).copied().unwrap_or(false)
}

pub(super) fn extra_locals(plan: &Plan) -> u32 {
    if plan.absence {
        plan.num_regs
    } else {
        0
    }
}

fn local(plan: &Plan, r: u16) -> u32 {
    plan.num_regs + 25 + exact::extra_locals(plan) + u32::from(r)
}

fn absent(code: &mut Vec<u8>, plan: &Plan, r: u16) {
    match plan.kinds.get(r as usize) {
        Some(Kind::Nothing) => i32_const(code, 1),
        Some(Kind::Optional) => {
            local_get(code, u32::from(r));
            code.push(0x45);
        }
        _ if tracked(plan, r) => local_get(code, local(plan, r)),
        _ => i32_const(code, 0),
    }
}

pub(super) fn lower(
    pc: usize,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    code: &mut Vec<u8>,
) -> R<Flow> {
    if !plan.absence {
        return exact::lower(pc, plan, ctx, blocks, k, code);
    }
    let flow = match plan.ops[pc] {
        Op::Show { src } if tracked(plan, src) => {
            absent(code, plan, src);
            code.extend([0x04, 0x40]);
            code.push(0x10);
            leb_u32(
                code,
                (ctx.host_index)(HostFn::PrintNothing)
                    .ok_or(WasmLowerError::Unsupported("absence sink not imported"))?,
            );
            code.push(0x05);
            exact::lower(pc, plan, ctx, blocks, k, code)?;
            code.push(0x0b);
            Flow::Straight
        }
        Op::Eq { dst, lhs, rhs } | Op::NotEq { dst, lhs, rhs }
            if tracked(plan, lhs) || tracked(plan, rhs) =>
        {
            absent(code, plan, lhs);
            absent(code, plan, rhs);
            code.extend([0x72, 0x04, 0x40]);
            absent(code, plan, lhs);
            absent(code, plan, rhs);
            code.push(0x71);
            if matches!(plan.ops[pc], Op::NotEq { .. }) {
                code.push(0x45);
            }
            code.push(0xad);
            local_set(code, u32::from(dst));
            code.push(0x05);
            exact::lower(pc, plan, ctx, blocks, k, code)?;
            code.push(0x0b);
            Flow::Straight
        }
        _ => exact::lower(pc, plan, ctx, blocks, k, code)?,
    };
    if flow == Flow::Straight {
        match plan.ops[pc] {
            Op::Move { dst, src } if tracked(plan, dst) => {
                absent(code, plan, src);
                local_set(code, local(plan, dst));
            }
            Op::GlobalGet { dst, idx } if tracked(plan, dst) => {
                global_get(code, ctx.absence_base.unwrap() + u32::from(idx));
                local_set(code, local(plan, dst));
            }
            Op::GlobalSet { idx, src } if plan.absence_globals[idx as usize] => {
                if plan.place.clear.get(pc).copied().unwrap_or(false) {
                    i32_const(code, 1);
                } else {
                    absent(code, plan, src);
                }
                global_set(code, ctx.absence_base.unwrap() + u32::from(idx));
            }
            Op::LoadConst { dst, idx }
                if matches!(ctx.constants.get(idx as usize), Some(Constant::Nothing)) =>
            {
                i32_const(code, 1);
                local_set(code, local(plan, dst));
            }
            ref op => op.for_each_written_reg(|dst| {
                if tracked(plan, dst) {
                    i32_const(code, 0);
                    local_set(code, local(plan, dst));
                }
            }),
        }
    }
    Ok(flow)
}
