//! Integer sequence slots own tagged integers once their header records that representation.
use super::*;

fn tagged_header(code: &mut Vec<u8>, hdr: u32) {
    local_get(code, hdr);
    i32_load(code, 12);
    i32_const(code, 0xff);
    code.push(0x71);
    i32_const(code, AllocClass::ExactIntegerHeader.code() as i32);
    code.push(0x46);
}

/// Promotion changes representation only; aliases still observe the same integer values.
pub(in super::super) fn promote(code: &mut Vec<u8>, ctx: &Ctx, _nr: u32, hdr: u32) -> R<()> {
    local_get(code, hdr);
    call(
        code,
        ctx.runtime
            .and_then(|r| r.index(RuntimeFn::ExactSeqPromote))
            .ok_or(WasmLowerError::Unsupported(
                "exact sequence promotion requires runtime",
            ))?,
    );
    Ok(())
}

pub(in super::super) fn emit_promote_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut bytes = vec![1, 2, I32]; // parameter 0 = header; locals 1 = index, 2 = address
    let code = &mut bytes;
    let (hdr, i, addr) = (0, 1, 2);
    tagged_header(code, hdr);
    code.extend([0x45, 0x04, 0x40]);
    i32_const(code, 0);
    local_set(code, i);
    code.extend([0x02, 0x40, 0x03, 0x40]);
    local_get(code, i);
    local_get(code, hdr);
    i32_load(code, 0);
    code.extend([0x4f, 0x0d]);
    leb_u32(code, 1);
    local_get(code, hdr);
    i32_load(code, 8);
    local_get(code, i);
    i32_const(code, 8);
    code.extend([0x6c, 0x6a]);
    local_tee(code, addr);
    local_get(code, addr);
    i64_load(code, 0);
    call(code, env.call(RuntimeFn::ExactTag));
    i64_store(code, 0);
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6a);
    local_set(code, i);
    code.push(0x0c);
    leb_u32(code, 0);
    code.extend([0x0b, 0x0b]);
    local_get(code, hdr);
    local_get(code, hdr);
    i32_load(code, 12);
    i32_const(code, !0xff);
    code.push(0x71);
    i32_const(code, AllocClass::ExactIntegerHeader.code() as i32);
    code.push(0x72);
    i32_store(code, 12);
    code.push(0x0b);
    code.push(0x0b);
    bytes
}

/// A borrowed tagged slot becomes a register's independent owned value.
pub(in super::super) fn take_tagged(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, dst: u16) -> R<()> {
    clone_top(code, ctx, scratch(plan, OPERAND_A))?;
    local_set(code, scratch(plan, RESULT));
    store_owned(
        code,
        plan,
        ctx,
        dst,
        scratch(plan, RESULT),
        FailCode::IntTooWide,
    )
}

fn operand(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, value: u16) -> R<()> {
    local_get(code, value as u32);
    if holds(plan, value) {
        clone_top(code, ctx, scratch(plan, OPERAND_A))?;
    } else {
        tag_top(code, ctx, scratch(plan, OPERAND_A))?;
    }
    local_set(code, scratch(plan, RESULT));
    Ok(())
}

fn address(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, collection: u16, index: u16) -> R<()> {
    if holds(plan, index) {
        local_get(code, index as u32);
        local_set(code, saved(plan, index));
        local_get(code, index as u32);
        narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
        local_set(code, index as u32);
    }
    emit_seq_elem_addr(code, ctx, &plan.kinds, collection, index)?;
    if holds(plan, index) {
        local_get(code, saved(plan, index));
        local_set(code, index as u32);
    }
    Ok(())
}

pub(in super::super) fn push(
    code: &mut Vec<u8>,
    plan: &Plan,
    ctx: &Ctx,
    hdr: u32,
    value: u16,
) -> R<()> {
    operand(code, plan, ctx, value)?;
    let push = ctx
        .runtime
        .and_then(|r| r.index(RuntimeFn::SeqPushSlot))
        .ok_or(WasmLowerError::Unsupported(
            "exact sequence requires owned sequence runtime",
        ))?;
    local_get(code, hdr);
    call(code, push);
    local_get(code, scratch(plan, RESULT));
    i64_store(code, 0);
    Ok(())
}

pub(super) fn lower(
    pc: usize,
    plan: &Plan,
    ctx: &Ctx,
    blocks: &Blocks,
    k: usize,
    code: &mut Vec<u8>,
) -> R<Option<Flow>> {
    let sequence = |r: u16| plan.kinds.get(r as usize) == Some(Kind::SeqInt);
    match plan.ops[pc] {
        Op::NewList { dst, start, count } | Op::NewTuple { dst, start, count } if sequence(dst) => {
            let hdr = plan.num_regs + 20;
            emit_fixed_header(
                code,
                ctx,
                plan.num_regs,
                hdr,
                AllocClass::ExactIntegerHeader,
                u32::from(count),
            );
            for r in start..start + count {
                push(code, plan, ctx, hdr, r)?;
            }
            local_get(code, hdr);
            local_set(code, dst as u32);
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::RepeatSeq,
            args_start,
            ..
        } if sequence(dst) => {
            let (hdr, i, count) = (plan.num_regs + 20, plan.num_regs + 19, plan.num_regs + 15);
            local_get(code, (args_start + 1) as u32);
            if holds(plan, args_start + 1) {
                narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
            }
            local_set(code, count);
            emit_empty_header(
                code,
                ctx,
                plan.num_regs,
                hdr,
                AllocClass::ExactIntegerHeader,
            );
            i32_const(code, 0);
            local_set(code, i);
            code.extend([0x02, 0x40, 0x03, 0x40]);
            local_get(code, i);
            code.push(0xad);
            local_get(code, count);
            code.extend([0x59, 0x0d]);
            leb_u32(code, 1);
            push(code, plan, ctx, hdr, args_start)?;
            local_get(code, i);
            i32_const(code, 1);
            code.push(0x6a);
            local_set(code, i);
            code.push(0x0c);
            leb_u32(code, 0);
            code.extend([0x0b, 0x0b]);
            local_get(code, hdr);
            local_set(code, dst as u32);
        }
        Op::CallBuiltin {
            dst,
            builtin: BuiltinId::DomApplyWords,
            args_start,
            ..
        } if sequence(args_start) => {
            local_get(code, args_start as u32);
            raw_view(code, plan, ctx, args_start)?;
            let flow = on_words(pc, plan, ctx, blocks, k, code)?;
            if dst == args_start {
                code.push(0x1a);
            } else {
                local_set(code, args_start as u32);
            }
            return Ok(Some(flow));
        }
        Op::Add { dst, lhs, rhs } if sequence(lhs) && sequence(rhs) => {
            lower_seq_concat(code, &plan.kinds, ctx, plan.num_regs, dst, lhs, rhs)?;
        }
        Op::AddAssign { dst, src } if sequence(dst) && sequence(src) => {
            lower_seq_concat(code, &plan.kinds, ctx, plan.num_regs, dst, dst, src)?;
        }
        Op::ListPop { list, dst } if sequence(list) => {
            promote(code, ctx, plan.num_regs, list as u32)?;
            emit_cow(code, &plan.kinds, &plan.structs, ctx, plan.num_regs, list)?;
            local_get(code, dst as u32);
            local_set(code, saved(plan, dst));
            lower_list_pop(code, &plan.kinds, dst, list)?;
            local_get(code, dst as u32);
            local_set(code, scratch(plan, RESULT));
            local_get(code, saved(plan, dst));
            local_set(code, dst as u32);
            store_owned(
                code,
                plan,
                ctx,
                dst,
                scratch(plan, RESULT),
                FailCode::IntTooWide,
            )?;
        }
        Op::Contains {
            dst,
            collection,
            value,
        } if sequence(collection) => {
            promote(code, ctx, plan.num_regs, collection as u32)?;
            operand(code, plan, ctx, value)?;
            let i = plan.num_regs + 20;
            i64_const(code, 0);
            local_set(code, dst as u32);
            i32_const(code, 0);
            local_set(code, i);
            code.extend([0x02, 0x40, 0x03, 0x40]);
            local_get(code, i);
            local_get(code, collection as u32);
            i32_load(code, 0);
            code.extend([0x4f, 0x0d]);
            leb_u32(code, 1);
            local_get(code, collection as u32);
            i32_load(code, 8);
            local_get(code, i);
            i32_const(code, 8);
            code.extend([0x6c, 0x6a]);
            i64_load(code, 0);
            local_get(code, scratch(plan, RESULT));
            call(code, import(ctx, HostFn::ExactCmp)?);
            code.extend([0x45, 0x04, 0x40]);
            i64_const(code, 1);
            local_set(code, dst as u32);
            code.push(0x0c);
            leb_u32(code, 2);
            code.push(0x0b);
            local_get(code, i);
            i32_const(code, 1);
            code.push(0x6a);
            local_set(code, i);
            code.push(0x0c);
            leb_u32(code, 0);
            code.extend([0x0b, 0x0b]);
            drop_local(code, ctx, scratch(plan, RESULT))?;
        }
        Op::ListPushField { obj, field, src }
            if plan.kinds.get(src as usize) == Some(Kind::Int) =>
        {
            let cow = plan.cow_inserts.get(pc).copied().unwrap_or(true);
            lower_list_push_field(code, plan, &plan.kinds, ctx, cow, obj, field, src)?;
        }
        Op::ListPush { list, value } if sequence(list) => {
            promote(code, ctx, plan.num_regs, list as u32)?;
            emit_cow(code, &plan.kinds, &plan.structs, ctx, plan.num_regs, list)?;
            push(code, plan, ctx, list as u32, value)?;
        }
        Op::Index {
            dst,
            collection,
            index,
        }
        | Op::IndexUnchecked {
            dst,
            collection,
            index,
        } if sequence(collection) => {
            local_get(code, collection as u32);
            local_get(code, index as u32);
            if holds(plan, index) {
                narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
            }
            call(
                code,
                ctx.runtime
                    .and_then(|r| r.index(RuntimeFn::ExactSeqAt))
                    .ok_or(WasmLowerError::Unsupported(
                        "exact sequence read requires runtime",
                    ))?,
            );
            local_set(code, scratch(plan, RESULT));
            store_owned(
                code,
                plan,
                ctx,
                dst,
                scratch(plan, RESULT),
                FailCode::IntTooWide,
            )?;
        }
        Op::SetIndex {
            collection,
            index,
            value,
        }
        | Op::SetIndexUnchecked {
            collection,
            index,
            value,
        } if sequence(collection) => {
            if !plan.place.clear.get(pc).copied().unwrap_or(false) {
                promote(code, ctx, plan.num_regs, collection as u32)?;
                emit_cow(
                    code,
                    &plan.kinds,
                    &plan.structs,
                    ctx,
                    plan.num_regs,
                    collection,
                )?;
                // Validate the address before acquiring the replacement's ownership.
                address(code, plan, ctx, collection, index)?;
                local_get(code, value as u32);
                if holds(plan, value) {
                    clone_top(code, ctx, scratch(plan, OPERAND_A))?;
                } else {
                    tag_top(code, ctx, scratch(plan, OPERAND_A))?;
                }
                call(
                    code,
                    ctx.runtime
                        .and_then(|r| r.index(RuntimeFn::ExactSeqStore))
                        .ok_or(WasmLowerError::Unsupported(
                            "exact sequence store requires runtime",
                        ))?,
                );
            }
        }
        Op::IterPrepare { iterable }
            if sequence(iterable) && !plan.range_loops.contains_key(&iterable) =>
        {
            promote(code, ctx, plan.num_regs, iterable as u32)?;
            lower_iter_prepare(code, &plan.kinds, ctx, plan.num_regs, iterable)?;
        }
        Op::IterNext { dst, exit }
            if plan
                .iter_iterable
                .get(pc)
                .copied()
                .flatten()
                .is_some_and(|r| sequence(r as u16)) =>
        {
            lower_iter_next(code, plan, ctx, blocks, k, dst, exit, pc)?;
            return Ok(Some(Flow::Terminated));
        }
        Op::Show { src } if sequence(src) && !plan.structs.tuple_layouts.contains_key(&src) => {
            stringify(code, ctx, plan.num_regs, src as u32, plan.num_regs + 8)?;
            local_get(code, plan.num_regs + 8);
            call(code, import(ctx, HostFn::PrintText)?);
        }
        _ => return Ok(None),
    }
    Ok(Some(Flow::Straight))
}

pub(in super::super) fn stringify(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    nr: u32,
    src: u32,
    out: u32,
) -> R<()> {
    // Nested displays keep their own accumulators in these locals. Preserve them
    // on the operand stack, and stabilize the source across text-append scratch.
    for offset in [8, 9, 10, 11, 13, 14, 17, 18, 19, 20, 21, 22, 23] {
        local_get(code, nr + offset);
    }
    local_get(code, src);
    local_set(code, nr + 20);
    let src = nr + 20;
    promote(code, ctx, nr, src)?;
    let (acc, i, part) = (nr + 17, nr + 18, nr + 19);
    lower_text_literal(code, ctx, nr, b"[");
    local_set(code, acc);
    i32_const(code, 0);
    local_set(code, i);
    code.extend([0x02, 0x40, 0x03, 0x40]);
    local_get(code, i);
    local_get(code, src);
    i32_load(code, 0);
    code.extend([0x4f, 0x0d]);
    leb_u32(code, 1);
    local_get(code, i);
    code.extend([0x04, 0x40]);
    lower_text_literal(code, ctx, nr, b", ");
    local_set(code, part);
    emit_text_append(code, ctx, nr, acc, part, acc);
    code.push(0x0b);
    local_get(code, src);
    i32_load(code, 8);
    local_get(code, i);
    i32_const(code, 8);
    code.extend([0x6c, 0x6a]);
    i64_load(code, 0);
    call(code, import(ctx, HostFn::ExactText)?);
    local_set(code, part);
    emit_text_append(code, ctx, nr, acc, part, acc);
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6a);
    local_set(code, i);
    code.push(0x0c);
    leb_u32(code, 0);
    code.extend([0x0b, 0x0b]);
    lower_text_literal(code, ctx, nr, b"]");
    local_set(code, part);
    emit_text_append(code, ctx, nr, acc, part, acc);
    local_get(code, acc);
    local_set(code, nr + 5);
    for offset in [8, 9, 10, 11, 13, 14, 17, 18, 19, 20, 21, 22, 23]
        .into_iter()
        .rev()
    {
        local_set(code, nr + offset);
    }
    local_get(code, nr + 5);
    local_set(code, out);
    Ok(())
}

/// A fixed-word host ABI borrows a raw view; the language sequence keeps its exact storage.
fn raw_view(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, src: u16) -> R<()> {
    let (hdr, i) = (plan.num_regs + 20, plan.num_regs + 21);
    let source = src as u32;
    emit_empty_header(code, ctx, plan.num_regs, hdr, AllocClass::ScalarHeader);
    i32_const(code, 0);
    local_set(code, i);
    code.extend([0x02, 0x40, 0x03, 0x40]);
    local_get(code, i);
    local_get(code, source);
    i32_load(code, 0);
    code.extend([0x4f, 0x0d]);
    leb_u32(code, 1);
    local_get(code, hdr);
    call(
        code,
        ctx.runtime
            .and_then(|r| r.index(RuntimeFn::SeqPushSlot))
            .ok_or(WasmLowerError::Unsupported(
                "word view requires sequence runtime",
            ))?,
    );
    local_get(code, source);
    i32_load(code, 8);
    local_get(code, i);
    i32_const(code, 8);
    code.extend([0x6c, 0x6a]);
    i64_load(code, 0);
    local_set(code, plan.num_regs + 16);
    tagged_header(code, source);
    code.extend([0x04, I64]);
    local_get(code, plan.num_regs + 16);
    narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
    code.push(0x05);
    local_get(code, plan.num_regs + 16);
    code.push(0x0b);
    i64_store(code, 0);
    local_get(code, i);
    i32_const(code, 1);
    code.push(0x6a);
    local_set(code, i);
    code.push(0x0c);
    leb_u32(code, 0);
    code.extend([0x0b, 0x0b]);
    local_get(code, hdr);
    local_set(code, source);
    Ok(())
}

/// Consume a borrowed slot's bits and leave its integer value for a word-only boundary.
pub(in super::super) fn narrow_slot(code: &mut Vec<u8>, ctx: &Ctx, nr: u32, hdr: u32) -> R<()> {
    local_set(code, nr + 15);
    tagged_header(code, hdr);
    code.extend([0x04, I64]);
    local_get(code, nr + 15);
    narrow_top(code, ctx, nr + 16, FailCode::IntTooWide)?;
    code.push(0x05);
    local_get(code, nr + 15);
    code.push(0x0b);
    Ok(())
}

/// Borrow the sequence and index, returning an owned tagged integer after the same bounds check.
pub(in super::super) fn emit_at_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![0]; // parameters header=0, raw index=1
    local_get(&mut c, 0);
    call(&mut c, env.call(RuntimeFn::ExactSeqPromote));
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    call(&mut c, env.call(RuntimeFn::SeqSlot));
    i64_load(&mut c, 0);
    call(&mut c, env.call(RuntimeFn::ExactClone));
    c.push(0x0b);
    c
}

/// Consume the replacement into a checked slot and release the slot's previous integer.
pub(in super::super) fn emit_store_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![0]; // parameters address=0, owned tagged replacement=1
    local_get(&mut c, 0);
    i64_load(&mut c, 0);
    call(&mut c, env.call(RuntimeFn::ExactRelease));
    local_get(&mut c, 0);
    local_get(&mut c, 1);
    i64_store(&mut c, 0);
    c.push(0x0b);
    c
}
