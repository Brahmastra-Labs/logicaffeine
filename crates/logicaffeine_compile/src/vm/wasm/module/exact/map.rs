//! Integer map slots own immutable counted cells, sharing the collector's handle protocol.
use super::*;

fn held(code: &mut Vec<u8>, map: u32, offset: u32) {
    let mask: i32 = AllocClass::ALL
        .iter()
        .filter(|class| {
            let (key, value) = class.held_handles();
            if offset == 0 {
                key
            } else {
                value
            }
        })
        .map(|class| 1i32 << class.code())
        .sum();
    i32_const(code, mask);
    local_get(code, map);
    i32_load(code, 12);
    i32_const(code, 0xff);
    code.extend([0x71, 0x76]);
    i32_const(code, 1);
    code.push(0x71);
}

/// Consume an owned tagged word and leave a counted immutable cell in the given local.
fn cell(code: &mut Vec<u8>, _plan: &Plan, ctx: &Ctx, cell: u32) {
    call(
        code,
        ctx.runtime
            .and_then(|r| r.index(RuntimeFn::ExactCell))
            .expect("exact maps carry the cell allocator"),
    );
    local_set(code, cell);
}

/// Consume a tagged integer into its counted one-slot cell, with the turn's initial hold.
pub(in super::super) fn emit_cell_helper(env: &RuntimeEnv) -> Vec<u8> {
    let mut c = vec![1, 2, I32]; // parameter 0 = tagged word; locals 1 = header, 2 = data
    i32_const(&mut c, AllocClass::ExactIntegerHeader.code() as i32);
    call(&mut c, env.call(RuntimeFn::NewHeader));
    local_set(&mut c, 1);
    i32_const(&mut c, 8);
    call(&mut c, env.call(RuntimeFn::Alloc));
    local_set(&mut c, 2);
    local_get(&mut c, 2);
    local_get(&mut c, 0);
    i64_store(&mut c, 0);
    for offset in [0, 4] {
        local_get(&mut c, 1);
        i32_const(&mut c, 1);
        i32_store(&mut c, offset);
    }
    local_get(&mut c, 1);
    local_get(&mut c, 2);
    i32_store(&mut c, 8);
    local_get(&mut c, 1);
    c.push(0x0b);
    c
}

fn cell_from_reg(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, reg: u16, out: u32) -> R<()> {
    local_get(code, reg as u32);
    if holds(plan, reg) {
        clone_top(code, ctx, scratch(plan, OPERAND_A))?;
    } else {
        tag_top(code, ctx, scratch(plan, OPERAND_A))?;
    }
    cell(code, plan, ctx, out);
    Ok(())
}

/// Representation changes preserve aliases: each header gains its own hold on each new cell.
fn promote(code: &mut Vec<u8>, _plan: &Plan, ctx: &Ctx, map: u32, offset: u32) -> R<()> {
    local_get(code, map);
    let helper = if offset == 0 {
        RuntimeFn::ExactMapPromoteKey
    } else {
        RuntimeFn::ExactMapPromoteValue
    };
    call(
        code,
        ctx.runtime
            .and_then(|r| r.index(helper))
            .ok_or(WasmLowerError::Unsupported(
                "exact map promotion requires runtime",
            ))?,
    );
    Ok(())
}

pub(in super::super) fn emit_promote_helper(env: &RuntimeEnv, offset: u32) -> Vec<u8> {
    let mut bytes = vec![1, 3, I32]; // map 0; index 1, entry 2, cell 3
    let code = &mut bytes;
    let (map, index, entry, boxed) = (0, 1, 2, 3);
    held(code, map, offset);
    code.extend([0x45, 0x04, 0x40]);
    i32_const(code, 0);
    local_set(code, index);
    code.extend([0x02, 0x40, 0x03, 0x40]);
    local_get(code, index);
    local_get(code, map);
    i32_load(code, 0);
    code.extend([0x4f, 0x0d]);
    leb_u32(code, 1);
    emit_map_entry_addr(code, map, index);
    local_tee(code, entry);
    i64_load(code, offset);
    call(code, env.call(RuntimeFn::ExactTag));
    call(code, env.call(RuntimeFn::ExactCell));
    local_set(code, boxed);
    local_get(code, entry);
    local_get(code, boxed);
    code.push(0xad);
    i64_store(code, offset);
    if env.retain_base.is_some() {
        local_get(code, boxed);
        call(code, env.call(RuntimeFn::Retain));
    }
    local_get(code, index);
    i32_const(code, 1);
    code.push(0x6a);
    local_set(code, index);
    code.push(0x0c);
    leb_u32(code, 0);
    code.extend([0x0b, 0x0b]);
    local_get(code, map);
    local_get(code, map);
    i32_load(code, 12);
    i32_const(code, if offset == 0 { 2 } else { 1 });
    code.push(0x6a);
    i32_store(code, 12);
    code.push(0x0b);
    code.push(0x0b);
    bytes
}

// Shared collections use the reference tier's signed-word scalar domain.
fn shared_scalar(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, map: u16, reg: u16) -> R<()> {
    if ctx.shared_maps && holds(plan, reg) {
        local_get(code, map as u32);
        i32_load(code, 12);
        i32_const(code, 0);
        code.extend([0x48, 0x04, 0x40]);
        local_get(code, reg as u32);
        narrow_top(code, ctx, scratch(plan, OPERAND_A), FailCode::IntTooWide)?;
        code.extend([0x1a, 0x0b]);
    }
    Ok(())
}

fn key(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, map: u16, key: u16) -> R<u32> {
    let out = plan.num_regs + 18;
    if plan.kinds.get(key as usize) == Some(Kind::Int) {
        shared_scalar(code, plan, ctx, map, key)?;
        promote(code, plan, ctx, map as u32, 0)?;
        cell_from_reg(code, plan, ctx, key, out)?;
    } else if plan.kinds.get(key as usize) == Some(Kind::Text) {
        local_get(code, key as u32);
        local_set(code, out);
    } else {
        return Err(WasmLowerError::Unsupported("map key must be Int or Text"));
    }
    Ok(out)
}

fn lookup(code: &mut Vec<u8>, plan: &Plan, ctx: &Ctx, map: u16, key: u16, required: bool) -> R<()> {
    let query = self::key(code, plan, ctx, map, key)?;
    local_get(code, map as u32);
    local_get(code, query);
    code.push(0xad);
    i32_const(code, 1);
    call(
        code,
        ctx.runtime
            .and_then(|r| {
                r.index(if required {
                    RuntimeFn::MapAt
                } else {
                    RuntimeFn::MapFind
                })
            })
            .ok_or(WasmLowerError::Unsupported(
                "exact map lookup requires the owned runtime",
            ))?,
    );
    Ok(())
}

pub(super) fn lower(pc: usize, plan: &Plan, ctx: &Ctx, code: &mut Vec<u8>) -> R<Option<Flow>> {
    let map = |r: u16| plan.kinds.get(r as usize) == Some(Kind::Map);
    let int = |r: u16| plan.kinds.get(r as usize) == Some(Kind::Int);
    match plan.ops[pc] {
        Op::SetIndex {
            collection,
            index,
            value,
        }
        | Op::SetIndexUnchecked {
            collection,
            index,
            value,
        } if map(collection) => {
            if plan.place.clear.get(pc).copied().unwrap_or(false) {
                return Ok(Some(Flow::Straight));
            }
            emit_cow(
                code,
                &plan.kinds,
                &plan.structs,
                ctx,
                plan.num_regs,
                collection,
            )?;
            let query = key(code, plan, ctx, collection, index)?;
            let value_handle = int(value) || counted_kind(plan.kinds.get(value as usize));
            let boxed = plan.num_regs + 19;
            if int(value) {
                shared_scalar(code, plan, ctx, collection, value)?;
                promote(code, plan, ctx, collection as u32, 8)?;
                cell_from_reg(code, plan, ctx, value, boxed)?;
            }
            let entry = plan.num_regs + 17;
            if ctx.shared_maps {
                local_get(code, collection as u32);
                i32_load(code, 0);
                local_set(code, plan.num_regs + 23);
            }
            local_get(code, collection as u32);
            local_get(code, query);
            code.push(0xad);
            i32_const(code, AllocClass::of_map(true, value_handle).code() as i32);
            call(
                code,
                ctx.runtime.and_then(|r| r.index(RuntimeFn::MapPut)).ok_or(
                    WasmLowerError::Unsupported("exact map insert requires the owned runtime"),
                )?,
            );
            local_set(code, entry);
            if value_handle {
                emit_true_release(code, ctx, |c| {
                    local_get(c, entry);
                    i32_load(c, 8);
                });
            }
            local_get(code, entry);
            if int(value) {
                local_get(code, boxed);
                code.push(0xad);
                i64_store(code, 8);
                emit_true_retain_of(code, ctx, |c| local_get(c, boxed));
            } else {
                local_get(code, value as u32);
                map_value_store(plan.kinds.get(value as usize))?(code, 8);
                if counts_aliases(plan.kinds.get(value as usize))
                    && !plan.place.put_back.get(pc).copied().unwrap_or(false)
                {
                    emit_retain(code, ctx, value);
                }
                if counted_kind(plan.kinds.get(value as usize)) {
                    emit_true_retain(code, ctx, value);
                }
            }
            if ctx.shared_maps {
                local_get(code, collection as u32);
                i32_load(code, 12);
                i32_const(code, 0);
                code.push(0x48);
                local_get(code, collection as u32);
                i32_load(code, 0);
                local_get(code, plan.num_regs + 23);
                code.extend([0x4b, 0x71, 0x04, 0x40]);
                emit_sink_last_entry_to_sorted_place(
                    code,
                    plan.num_regs,
                    collection as u32,
                    query,
                    !int(index),
                    if int(index) {
                        Some(import(ctx, HostFn::ExactCmp)?)
                    } else {
                        None
                    },
                );
                code.push(0x0b);
            }
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
        } if map(collection) => {
            if int(dst) {
                promote(code, plan, ctx, collection as u32, 8)?;
            }
            lookup(code, plan, ctx, collection, index, true)?;
            if int(dst) {
                i32_load(code, 8);
                i32_load(code, 8);
                i64_load(code, 0);
                list::take_tagged(code, plan, ctx, dst)?;
            } else {
                map_value_load(plan.kinds.get(dst as usize))?(code, 8);
                local_set(code, dst as u32);
            }
        }
        Op::Contains {
            dst,
            collection,
            value,
        } if map(collection) => {
            lookup(code, plan, ctx, collection, value, false)?;
            i32_const(code, 0);
            code.extend([0x47, 0xad]);
            local_set(code, dst as u32);
        }
        Op::RemoveFrom { collection, value } if map(collection) => {
            emit_cow(
                code,
                &plan.kinds,
                &plan.structs,
                ctx,
                plan.num_regs,
                collection,
            )?;
            lookup(code, plan, ctx, collection, value, false)?;
            let entry = plan.num_regs + 17;
            local_tee(code, entry);
            code.extend([0x04, 0x40]);
            for offset in [0, 8] {
                held(code, collection as u32, offset);
                code.extend([0x04, 0x40]);
                emit_true_release(code, ctx, |c| {
                    local_get(c, entry);
                    i32_load(c, offset);
                });
                code.push(0x0b);
            }
            local_get(code, entry);
            local_get(code, entry);
            i32_const(code, 16);
            code.push(0x6a);
            local_get(code, collection as u32);
            i32_load(code, 8);
            local_get(code, collection as u32);
            i32_load(code, 0);
            i32_const(code, 16);
            code.extend([0x6c, 0x6a]);
            local_get(code, entry);
            code.push(0x6b);
            i32_const(code, 16);
            code.push(0x6b);
            memory_copy(code);
            local_get(code, collection as u32);
            local_get(code, collection as u32);
            i32_load(code, 0);
            i32_const(code, 1);
            code.push(0x6b);
            i32_store(code, 0);
            code.push(0x0b);
        }
        _ => return Ok(None),
    }
    Ok(Some(Flow::Straight))
}

/// Consume an entry address and format an integer slot in its actual stored representation.
pub(in super::super) fn integer_text(
    code: &mut Vec<u8>,
    ctx: &Ctx,
    nr: u32,
    map: u32,
    offset: u32,
    out: u32,
) -> R<()> {
    let entry = nr + 23;
    local_set(code, entry);
    held(code, map, offset);
    code.extend([0x04, 0x40]);
    local_get(code, entry);
    i32_load(code, offset);
    i32_load(code, 8);
    i64_load(code, 0);
    call(code, import(ctx, HostFn::ExactText)?);
    local_set(code, out);
    code.push(0x05);
    local_get(code, entry);
    i64_load(code, offset);
    local_set(code, nr + 15);
    emit_stringify(code, ctx, nr, nr + 15, Some(Kind::Int), out)?;
    code.push(0x0b);
    Ok(())
}
